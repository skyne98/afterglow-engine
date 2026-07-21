// .big streaming container format — Command & Conquer: Generals inspired.
//
// 1. SEQUENTIAL ULTRA FRIENDLY — reading front-to-back gives progressive quality.
// 2. FULLY PARTIAL SEEKABLE — index has absolute offsets for every chunk.
// 3. COMPRESSED BY DEFAULT — all chunk data is meshopt-compressed.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub const MAGIC: &[u8; 4] = b"BIG1";
/// Current `.big` container version written by this crate.
///
/// v6 adds an explicit `TextureFormat` to `ChunkMeta::Texture`. v5 files are
/// still readable (they never contain `AssetType::Texture` chunks, so the
/// `ChunkMeta::Texture` encoding is unambiguous); the parser accepts both.
pub const VERSION: u32 = 6;
/// Oldest readable version. v5 files predate resident `Texture` assets.
pub const MIN_READABLE_VERSION: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AssetType {
    Texture,
    Mesh,
    VirtualTexture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Compression {
    Meshopt,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TextureEncoding {
    RawRgba8,
    Basis,
}

/// Texel format of a resident (non-virtual) `AssetType::Texture` asset.
///
/// Virtual textures always use `TextureEncoding` (RGBA8/Basis) for their
/// paged PBR channels; resident textures are single-mip, always-resident
/// byte streams interpreted by this format (e.g. an R8 height field for POM).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TextureFormat {
    /// 4 bytes per texel, RGBA channel order.
    Rgba8,
    /// 1 byte per texel, single-channel unorm (e.g. 8-bit displacement).
    R8,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChunkInfo {
    pub offset: u64,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub lod_level: u8,
    pub mip_level: u8,
    pub compression: Compression,
    pub meta: ChunkMeta,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ChunkMeta {
    Texture {
        width: u32,
        height: u32,
        format: TextureFormat,
    },
    Mesh {
        index_count: u32,
        vertex_count: u32,
        position_stride: u32,
        uv_stride: u32,
    },
    Raw,
}

/// One contiguous row-major mip block. Per-page sizes are enough to reconstruct
/// direct absolute offsets while avoiding a full ChunkInfo per VT page.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VirtualTextureMipDirectory {
    pub mip: u8,
    pub pages_x: u32,
    pub pages_y: u32,
    pub offset: u64,
    pub page_sizes: Vec<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VirtualTextureTailDirectory {
    pub first_mip: u8,
    pub offset: u64,
    pub size: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VirtualTextureDirectory {
    pub width: u32,
    pub height: u32,
    pub encoding: TextureEncoding,
    pub mips: Vec<VirtualTextureMipDirectory>,
    pub tail: Option<VirtualTextureTailDirectory>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AssetEntry {
    pub name: String,
    pub asset_type: AssetType,
    pub chunks: Vec<ChunkInfo>,
    pub virtual_texture: Option<VirtualTextureDirectory>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BigHeader {
    pub version: u32,
    pub data_offset: u64,
    pub assets: Vec<AssetEntry>,
}

impl TextureFormat {
    pub fn bytes_per_texel(self) -> usize {
        match self {
            TextureFormat::Rgba8 => 4,
            TextureFormat::R8 => 1,
        }
    }
}

impl BigHeader {
    pub fn streaming_order(&self) -> Vec<(usize, usize)> {
        let mut entries: Vec<(usize, usize, u32)> = Vec::new();
        for (ai, asset) in self.assets.iter().enumerate() {
            for (ci, chunk) in asset.chunks.iter().enumerate() {
                entries.push((ai, ci, chunk.lod_level as u32 + chunk.mip_level as u32));
            }
        }
        entries.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
        entries.iter().map(|&(a, c, _)| (a, c)).collect()
    }
    pub fn find(&self, name: &str) -> Option<usize> {
        self.assets.iter().position(|a| a.name == name)
    }
}

// --- helper ---

fn compress_chunk(mut data: Vec<u8>, compression: Compression) -> Vec<u8> {
    match compression {
        Compression::Meshopt => {
            let padded_len = (data.len() + 3) & !3;
            data.resize(padded_len, 0);
            afterglow_meshopt::safe::encode_vertex_buffer(&data, 4)
        }
        Compression::None => data,
    }
}

// --- writer ---

#[derive(Clone, Copy)]
enum PendingChunkKind {
    Regular { chunk: usize },
    VirtualTexturePage { mip: usize, page: usize },
    VirtualTextureTail,
}

struct PendingChunk {
    asset: usize,
    spool_offset: u64,
    size: u32,
    compression: Compression,
    stream_level: u8,
    order_in_level: usize,
    kind: PendingChunkKind,
}

pub struct BigWriter {
    assets: Vec<AssetEntry>,
    chunks: Vec<PendingChunk>,
    spool: Option<std::fs::File>,
    spool_path: PathBuf,
}

impl BigWriter {
    pub fn new() -> Self {
        static NEXT_SPOOL: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_SPOOL.fetch_add(1, Ordering::Relaxed);
        let spool_path =
            std::env::temp_dir().join(format!("afterglow-big-{}-{id}.spool", std::process::id(),));
        let spool = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&spool_path)
            .unwrap_or_else(|error| {
                panic!(
                    "failed to create BIG spool {}: {error}",
                    spool_path.display()
                )
            });
        Self {
            assets: Vec::new(),
            chunks: Vec::new(),
            spool: Some(spool),
            spool_path,
        }
    }

    fn push_chunk(
        &mut self,
        asset: usize,
        data: Vec<u8>,
        compression: Compression,
        stream_level: u8,
        order_in_level: usize,
        kind: PendingChunkKind,
    ) {
        let data = compress_chunk(data, compression);
        let size = u32::try_from(data.len()).expect("BIG chunks must fit u32");
        let spool = self.spool.as_mut().unwrap();
        let spool_offset = spool.seek(SeekFrom::End(0)).expect("seek BIG spool");
        spool.write_all(&data).expect("write BIG spool");
        self.chunks.push(PendingChunk {
            asset,
            spool_offset,
            size,
            compression,
            stream_level,
            order_in_level,
            kind,
        });
    }

    pub fn add_texture(&mut self, name: &str, mips: Vec<(u32, u32, Vec<u8>)>) {
        let asset_idx = self.assets.len();
        let mut chunks_meta = Vec::new();
        for (mip_level, (width, height, data)) in mips.into_iter().enumerate() {
            let uncompressed_size = data.len() as u64;
            self.push_chunk(
                asset_idx,
                data,
                Compression::Meshopt,
                mip_level as u8,
                mip_level,
                PendingChunkKind::Regular { chunk: mip_level },
            );
            chunks_meta.push(ChunkInfo {
                offset: 0,
                compressed_size: 0,
                uncompressed_size,
                lod_level: 0,
                mip_level: mip_level as u8,
                compression: Compression::Meshopt,
                meta: ChunkMeta::Texture {
                    width,
                    height,
                    format: TextureFormat::Rgba8,
                },
            });
        }
        self.assets.push(AssetEntry {
            name: name.to_string(),
            asset_type: AssetType::Texture,
            chunks: chunks_meta,
            virtual_texture: None,
        });
    }

    /// Add a single-mip resident (non-virtual) texture, e.g. an R8 height field.
    ///
    /// Resident textures are always-resident byte streams sampled directly at
    /// runtime (no page table, no mip tail). `bytes.len()` must equal
    /// `width * height * format.bytes_per_texel()`.
    pub fn add_resident_texture(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        format: TextureFormat,
        bytes: Vec<u8>,
    ) {
        let expected = (width as u64)
            .checked_mul(height as u64)
            .and_then(|texels| texels.checked_mul(format.bytes_per_texel() as u64))
            .expect("resident texture dimensions overflow");
        assert_eq!(
            bytes.len() as u64,
            expected,
            "resident texture byte length must match width*height*bpp"
        );
        let asset_idx = self.assets.len();
        let uncompressed_size = bytes.len() as u64;
        self.push_chunk(
            asset_idx,
            bytes,
            Compression::None,
            0,
            0,
            PendingChunkKind::Regular { chunk: 0 },
        );
        self.assets.push(AssetEntry {
            name: name.to_string(),
            asset_type: AssetType::Texture,
            chunks: vec![ChunkInfo {
                offset: 0,
                compressed_size: 0,
                uncompressed_size,
                lod_level: 0,
                mip_level: 0,
                compression: Compression::None,
                meta: ChunkMeta::Texture {
                    width,
                    height,
                    format,
                },
            }],
            virtual_texture: None,
        });
    }

    /// Begin a VT whose regular mip range is known from its packed tail.
    /// Pages may then be encoded and admitted in bounded batches.
    pub fn begin_virtual_texture(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        first_tail_mip: u8,
        encoding: TextureEncoding,
    ) -> usize {
        let asset = self.assets.len();
        let mut mips = Vec::with_capacity(first_tail_mip as usize);
        for mip in 0..first_tail_mip {
            let span = 128u64 << mip;
            let pages_x = ((width as u64 + span - 1) / span).max(1) as u32;
            let pages_y = ((height as u64 + span - 1) / span).max(1) as u32;
            mips.push(VirtualTextureMipDirectory {
                mip,
                pages_x,
                pages_y,
                offset: 0,
                page_sizes: vec![0; (pages_x * pages_y) as usize],
            });
        }
        self.assets.push(AssetEntry {
            name: name.to_owned(),
            asset_type: AssetType::VirtualTexture,
            chunks: Vec::new(),
            virtual_texture: Some(VirtualTextureDirectory {
                width,
                height,
                encoding,
                mips,
                tail: Some(VirtualTextureTailDirectory {
                    first_mip: first_tail_mip,
                    offset: 0,
                    size: 0,
                }),
            }),
        });
        asset
    }

    pub fn push_virtual_texture_page(&mut self, asset: usize, page: VirtualTexturePageData) {
        let directory = self
            .assets
            .get_mut(asset)
            .and_then(|entry| entry.virtual_texture.as_mut())
            .expect("invalid virtual texture asset");
        assert_eq!(
            page.encoding, directory.encoding,
            "one VT must use one encoding"
        );
        let mip_index = page.mip as usize;
        let mip = directory
            .mips
            .get_mut(mip_index)
            .expect("VT page mip is in the packed tail");
        let page_index = (page.page_y * mip.pages_x + page.page_x) as usize;
        assert!(
            page.page_x < mip.pages_x && page.page_y < mip.pages_y,
            "VT page outside directory"
        );
        assert_eq!(mip.page_sizes[page_index], 0, "duplicate VT page");
        mip.page_sizes[page_index] = u32::MAX;
        self.push_chunk(
            asset,
            page.data,
            Compression::None,
            page.mip,
            page_index,
            PendingChunkKind::VirtualTexturePage {
                mip: mip_index,
                page: page_index,
            },
        );
    }

    pub fn finish_virtual_texture(&mut self, asset: usize, tail: VirtualTextureMipTailData) {
        let directory = self
            .assets
            .get_mut(asset)
            .and_then(|entry| entry.virtual_texture.as_mut())
            .expect("invalid virtual texture asset");
        assert_eq!(
            tail.encoding, directory.encoding,
            "VT tail encoding must match pages"
        );
        assert_eq!(directory.tail.as_ref().unwrap().first_mip, tail.first_mip);
        assert!(
            directory
                .mips
                .iter()
                .all(|mip| mip.page_sizes.iter().all(|size| *size != 0)),
            "VT directories require every row-major page"
        );
        self.push_chunk(
            asset,
            tail.data,
            Compression::None,
            tail.first_mip,
            0,
            PendingChunkKind::VirtualTextureTail,
        );
    }

    /// Add a virtual texture as a set of page chunks.
    /// Each page is 128×128 texels (+ 4px border per side = 136×136 slot).
    pub fn add_virtual_texture(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        pages: Vec<VirtualTexturePageData>,
    ) {
        self.add_virtual_texture_with_tail(name, width, height, pages, None);
    }

    pub fn add_virtual_texture_with_tail(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        pages: Vec<VirtualTexturePageData>,
        tail: Option<VirtualTextureMipTailData>,
    ) {
        let asset_idx = self.assets.len();
        let encoding = pages
            .first()
            .map(|page| page.encoding)
            .or_else(|| tail.as_ref().map(|tail| tail.encoding))
            .expect("virtual texture requires a page or mip tail");
        let max_mip = pages.iter().map(|page| page.mip).max();
        let mut mips = Vec::new();
        if let Some(max_mip) = max_mip {
            for mip in 0..=max_mip {
                if !pages.iter().any(|page| page.mip == mip) {
                    continue;
                }
                let span = 128u64 << mip;
                let pages_x = ((width as u64 + span - 1) / span).max(1) as u32;
                let pages_y = ((height as u64 + span - 1) / span).max(1) as u32;
                mips.push(VirtualTextureMipDirectory {
                    mip,
                    pages_x,
                    pages_y,
                    offset: 0,
                    page_sizes: vec![0; (pages_x * pages_y) as usize],
                });
            }
        }
        for page in pages {
            assert_eq!(page.encoding, encoding, "one VT must use one encoding");
            let mip_index = mips
                .iter()
                .position(|directory| directory.mip == page.mip)
                .expect("VT mip directory missing");
            let directory = &mut mips[mip_index];
            assert!(
                page.page_x < directory.pages_x && page.page_y < directory.pages_y,
                "VT page coordinates outside directory"
            );
            let page_index = (page.page_y * directory.pages_x + page.page_x) as usize;
            assert_eq!(directory.page_sizes[page_index], 0, "duplicate VT page");
            directory.page_sizes[page_index] = u32::MAX;
            self.push_chunk(
                asset_idx,
                page.data,
                Compression::None,
                page.mip,
                page_index,
                PendingChunkKind::VirtualTexturePage {
                    mip: mip_index,
                    page: page_index,
                },
            );
        }
        assert!(
            mips.iter()
                .all(|mip| mip.page_sizes.iter().all(|size| *size != 0)),
            "VT directories require every row-major page"
        );
        let tail_directory = tail.map(|tail| {
            assert_eq!(tail.encoding, encoding, "VT tail encoding must match pages");
            self.push_chunk(
                asset_idx,
                tail.data,
                Compression::None,
                tail.first_mip,
                0,
                PendingChunkKind::VirtualTextureTail,
            );
            VirtualTextureTailDirectory {
                first_mip: tail.first_mip,
                offset: 0,
                size: 0,
            }
        });
        self.assets.push(AssetEntry {
            name: name.to_string(),
            asset_type: AssetType::VirtualTexture,
            chunks: Vec::new(),
            virtual_texture: Some(VirtualTextureDirectory {
                width,
                height,
                encoding,
                mips,
                tail: tail_directory,
            }),
        });
    }

    /// Pack a self-contained model payload for runtime parsing and mesh-worker
    /// optimization. Container compression is intentionally `None`: arbitrary
    /// GLB bytes are not a meshopt vertex stream.
    pub fn add_raw_mesh(&mut self, name: &str, data: Vec<u8>) {
        let asset = self.assets.len();
        let size = data.len() as u64;
        self.push_chunk(
            asset,
            data,
            Compression::None,
            0,
            0,
            PendingChunkKind::Regular { chunk: 0 },
        );
        self.assets.push(AssetEntry {
            name: name.to_owned(),
            asset_type: AssetType::Mesh,
            chunks: vec![ChunkInfo {
                offset: 0,
                compressed_size: 0,
                uncompressed_size: size,
                lod_level: 0,
                mip_level: 0,
                compression: Compression::None,
                meta: ChunkMeta::Raw,
            }],
            virtual_texture: None,
        });
    }

    pub fn add_mesh(&mut self, name: &str, lods: Vec<MeshLodData>) {
        let asset_idx = self.assets.len();
        let mut chunks_meta = Vec::new();
        for (lod_level, lod) in lods.into_iter().enumerate() {
            let mut data = Vec::new();
            data.extend_from_slice(&(lod.indices.len() as u32).to_le_bytes());
            data.extend_from_slice(bytemuck::cast_slice(&lod.indices));
            data.extend_from_slice(bytemuck::cast_slice(&lod.positions));
            if !lod.uvs.is_empty() {
                data.extend_from_slice(bytemuck::cast_slice(&lod.uvs));
            }
            let uncompressed_size = data.len() as u64;
            let vertex_count = (lod.positions.len() / (lod.position_stride as usize / 4)) as u32;
            self.push_chunk(
                asset_idx,
                data,
                Compression::Meshopt,
                lod_level as u8,
                lod_level,
                PendingChunkKind::Regular { chunk: lod_level },
            );
            chunks_meta.push(ChunkInfo {
                offset: 0,
                compressed_size: 0,
                uncompressed_size,
                lod_level: lod_level as u8,
                mip_level: 0,
                compression: Compression::Meshopt,
                meta: ChunkMeta::Mesh {
                    index_count: lod.indices.len() as u32,
                    vertex_count,
                    position_stride: lod.position_stride,
                    uv_stride: lod.uv_stride,
                },
            });
        }
        self.assets.push(AssetEntry {
            name: name.to_string(),
            asset_type: AssetType::Mesh,
            chunks: chunks_meta,
            virtual_texture: None,
        });
    }

    pub fn finish(mut self, writer: &mut impl Write) -> io::Result<()> {
        let mut header = BigHeader {
            version: VERSION,
            data_offset: 0,
            assets: std::mem::take(&mut self.assets),
        };
        let mut chunks = std::mem::take(&mut self.chunks);
        chunks.sort_by(|left, right| {
            right
                .stream_level
                .cmp(&left.stream_level)
                .then(left.asset.cmp(&right.asset))
                .then(left.order_in_level.cmp(&right.order_in_level))
        });

        for chunk in &chunks {
            match chunk.kind {
                PendingChunkKind::Regular { chunk: chunk_index } => {
                    let metadata = &mut header.assets[chunk.asset].chunks[chunk_index];
                    metadata.compressed_size = chunk.size as u64;
                    metadata.compression = chunk.compression;
                }
                PendingChunkKind::VirtualTexturePage { mip, page } => {
                    header.assets[chunk.asset]
                        .virtual_texture
                        .as_mut()
                        .unwrap()
                        .mips[mip]
                        .page_sizes[page] = chunk.size;
                }
                PendingChunkKind::VirtualTextureTail => {
                    header.assets[chunk.asset]
                        .virtual_texture
                        .as_mut()
                        .unwrap()
                        .tail
                        .as_mut()
                        .unwrap()
                        .size = chunk.size;
                }
            }
        }

        // Solve the postcard varint fixed point for header size and absolute
        // block offsets. Only one offset per VT mip is serialized.
        let mut previous_data_offset = u64::MAX;
        for _ in 0..16 {
            let bytes = postcard::to_allocvec(&header)
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
            let data_offset = 16 + bytes.len() as u64;
            let mut offset = data_offset;
            for chunk in &chunks {
                match chunk.kind {
                    PendingChunkKind::Regular { chunk: index } => {
                        header.assets[chunk.asset].chunks[index].offset = offset
                    }
                    PendingChunkKind::VirtualTexturePage { mip, page: 0 } => {
                        header.assets[chunk.asset]
                            .virtual_texture
                            .as_mut()
                            .unwrap()
                            .mips[mip]
                            .offset = offset;
                    }
                    PendingChunkKind::VirtualTexturePage { .. } => {}
                    PendingChunkKind::VirtualTextureTail => {
                        header.assets[chunk.asset]
                            .virtual_texture
                            .as_mut()
                            .unwrap()
                            .tail
                            .as_mut()
                            .unwrap()
                            .offset = offset;
                    }
                }
                offset += chunk.size as u64;
            }
            header.data_offset = data_offset;
            if data_offset == previous_data_offset {
                break;
            }
            previous_data_offset = data_offset;
        }
        let header_bytes = postcard::to_allocvec(&header)
            .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
        if header.data_offset != 16 + header_bytes.len() as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "container header offsets did not converge",
            ));
        }

        writer.write_all(MAGIC)?;
        writer.write_all(&VERSION.to_le_bytes())?;
        writer.write_all(&header.data_offset.to_le_bytes())?;
        writer.write_all(&header_bytes)?;
        let mut spool = self.spool.take().unwrap();
        let mut scratch = [0u8; 64 * 1024];
        for chunk in chunks {
            spool.seek(SeekFrom::Start(chunk.spool_offset))?;
            let mut remaining = chunk.size as usize;
            while remaining != 0 {
                let count = remaining.min(scratch.len());
                spool.read_exact(&mut scratch[..count])?;
                writer.write_all(&scratch[..count])?;
                remaining -= count;
            }
        }
        drop(spool);
        std::fs::remove_file(&self.spool_path)?;
        self.spool_path = PathBuf::new();
        Ok(())
    }
}

impl Drop for BigWriter {
    fn drop(&mut self) {
        self.spool.take();
        if !self.spool_path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.spool_path);
        }
    }
}

pub struct VirtualTextureMipTailData {
    pub first_mip: u8,
    pub encoding: TextureEncoding,
    pub data: Vec<u8>,
}

pub struct VirtualTexturePageData {
    pub mip: u8,
    pub page_x: u32,
    pub page_y: u32,
    pub encoding: TextureEncoding,
    /// Encoded page payload. RawRgba8 is SLOT_SIZE × SLOT_SIZE × 4 bytes.
    pub data: Vec<u8>,
}

pub struct MeshLodData {
    pub indices: Vec<u32>,
    pub positions: Vec<f32>,
    pub uvs: Vec<f32>,
    pub position_stride: u32,
    pub uv_stride: u32,
}

// --- reader ---

pub fn parse_header(data: &[u8]) -> Result<(BigHeader, usize), String> {
    if data.len() < 16 {
        return Err("file too small".into());
    }
    if &data[0..4] != MAGIC {
        return Err(format!("bad magic"));
    }
    let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if !(MIN_READABLE_VERSION..=VERSION).contains(&version) {
        return Err(format!("version {version} not in [{MIN_READABLE_VERSION},{VERSION}]"));
    }
    let data_offset = u64::from_le_bytes(data[8..16].try_into().unwrap()) as usize;
    let header: BigHeader =
        postcard::from_bytes(&data[16..data_offset]).map_err(|e| format!("decode: {e}"))?;
    Ok((header, data_offset))
}

pub fn read_chunk<'a>(data: &'a [u8], chunk: &ChunkInfo) -> &'a [u8] {
    &data[chunk.offset as usize..(chunk.offset + chunk.compressed_size) as usize]
}

pub fn read_chunk_decompressed(data: &[u8], chunk: &ChunkInfo) -> Vec<u8> {
    let compressed = read_chunk(data, chunk);
    match chunk.compression {
        Compression::Meshopt => {
            let padded_count = (chunk.uncompressed_size as usize + 3) / 4;
            let decoded =
                afterglow_meshopt::safe::decode_vertex_buffer(compressed, padded_count, 4);
            decoded[..chunk.uncompressed_size as usize].to_vec()
        }
        Compression::None => compressed.to_vec(),
    }
}

// --- tests ---

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_virtual_texture_pages() {
        let mut writer = BigWriter::new();
        let page_data = vec![0xAB; 136 * 136 * 4]; // SLOT_SIZE × SLOT_SIZE × 4
        writer.add_virtual_texture_with_tail(
            "terrain",
            128,
            128,
            vec![
                VirtualTexturePageData {
                    mip: 0,
                    page_x: 0,
                    page_y: 0,
                    encoding: TextureEncoding::RawRgba8,
                    data: page_data.clone(),
                },
                VirtualTexturePageData {
                    mip: 1,
                    page_x: 0,
                    page_y: 0,
                    encoding: TextureEncoding::RawRgba8,
                    data: vec![0xCD; 136 * 136 * 4],
                },
            ],
            Some(VirtualTextureMipTailData {
                first_mip: 6,
                encoding: TextureEncoding::RawRgba8,
                data: vec![0xEF; 136 * 136 * 4],
            }),
        );
        let mut buf = Vec::new();
        writer.finish(&mut buf).unwrap();
        let (header, _) = parse_header(&buf).unwrap();
        assert_eq!(header.assets.len(), 1);
        assert_eq!(header.assets[0].asset_type, AssetType::VirtualTexture);
        assert!(header.assets[0].chunks.is_empty());
        let directory = header.assets[0].virtual_texture.as_ref().unwrap();
        assert_eq!((directory.width, directory.height), (128, 128));
        assert_eq!(directory.encoding, TextureEncoding::RawRgba8);
        assert_eq!(directory.mips.len(), 2);
        let mip0 = &directory.mips[0];
        assert_eq!((mip0.mip, mip0.pages_x, mip0.pages_y), (0, 1, 1));
        let decoded =
            &buf[mip0.offset as usize..mip0.offset as usize + mip0.page_sizes[0] as usize];
        assert_eq!(decoded, page_data);
        let tail = directory.tail.as_ref().unwrap();
        assert_eq!(tail.first_mip, 6);
        assert_eq!(
            &buf[tail.offset as usize..tail.offset as usize + tail.size as usize],
            vec![0xEF; 136 * 136 * 4]
        );
    }

    #[test]
    fn compact_vt_directory_does_not_serialize_per_page_metadata() {
        let mut writer = BigWriter::new();
        let pages = (0..32)
            .flat_map(|y| {
                (0..32).map(move |x| VirtualTexturePageData {
                    mip: 0,
                    page_x: x,
                    page_y: y,
                    encoding: TextureEncoding::Basis,
                    data: vec![x as u8],
                })
            })
            .collect();
        writer.add_virtual_texture("large", 4096, 4096, pages);
        let spool_path = writer.spool_path.clone();
        assert!(spool_path.exists());
        let mut bytes = Vec::new();
        writer.finish(&mut bytes).unwrap();
        assert!(!spool_path.exists());
        let (header, data_offset) = parse_header(&bytes).unwrap();
        let directory = header.assets[0].virtual_texture.as_ref().unwrap();
        assert_eq!(directory.mips[0].page_sizes.len(), 1024);
        assert!(data_offset < 1200, "compact header was {data_offset} bytes");
    }

    #[test]
    fn roundtrip_compressed_texture() {
        let mut writer = BigWriter::new();
        let tex_data = vec![0xAB; 4 * 4 * 4];
        writer.add_texture(
            "sky.png",
            vec![
                (4, 4, tex_data.clone()),
                (2, 2, vec![0xCD; 2 * 2 * 4]),
                (1, 1, vec![0xEF; 4]),
            ],
        );
        let mut buf = Vec::new();
        writer.finish(&mut buf).unwrap();
        let (header, _) = parse_header(&buf).unwrap();
        assert_eq!(header.assets.len(), 1);
        assert_eq!(header.assets[0].chunks.len(), 3);
        for chunk in &header.assets[0].chunks {
            assert_eq!(chunk.compression, Compression::Meshopt);
            assert!(chunk.compressed_size > 0);
        }
        let c0 = &header.assets[0].chunks[0];
        let decoded = read_chunk_decompressed(&buf, c0);
        assert_eq!(decoded, tex_data);
    }

    #[test]
    fn roundtrip_resident_r8_texture() {
        let mut writer = BigWriter::new();
        let height: Vec<u8> = (0..(8 * 8)).map(|i| (i * 2) as u8).collect();
        writer.add_resident_texture("Rock_Height", 8, 8, TextureFormat::R8, height.clone());
        let mut buf = Vec::new();
        writer.finish(&mut buf).unwrap();
        let (header, _) = parse_header(&buf).unwrap();
        assert_eq!(header.version, VERSION);
        assert_eq!(header.assets.len(), 1);
        assert_eq!(header.assets[0].asset_type, AssetType::Texture);
        assert_eq!(header.assets[0].chunks.len(), 1);
        let chunk = &header.assets[0].chunks[0];
        match &chunk.meta {
            ChunkMeta::Texture { width, height, format } => {
                assert_eq!((*width, *height), (8, 8));
                assert_eq!(*format, TextureFormat::R8);
            }
            other => panic!("unexpected chunk meta {other:?}"),
        }
        assert_eq!(chunk.compression, Compression::None);
        assert_eq!(chunk.uncompressed_size, 64);
        let decoded = read_chunk_decompressed(&buf, chunk);
        assert_eq!(decoded, height);
    }

    #[test]
    fn resident_texture_rejects_byte_count_mismatch() {
        let mut writer = BigWriter::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            writer.add_resident_texture("bad", 4, 4, TextureFormat::R8, vec![0; 15]);
        }));
        assert!(result.is_err(), "mismatched byte length must panic");
    }

    #[test]
    fn roundtrip_compressed_mesh() {
        let mut writer = BigWriter::new();
        writer.add_mesh(
            "sphere",
            vec![MeshLodData {
                indices: vec![0, 1, 2, 0, 2, 3],
                positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0],
                uvs: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
                position_stride: 12,
                uv_stride: 8,
            }],
        );
        let mut buf = Vec::new();
        writer.finish(&mut buf).unwrap();
        let (header, _) = parse_header(&buf).unwrap();
        let chunk = &header.assets[0].chunks[0];
        let decoded = read_chunk_decompressed(&buf, chunk);
        let count = u32::from_le_bytes(decoded[0..4].try_into().unwrap());
        assert_eq!(count, 6);
    }

    #[test]
    fn streaming_order_lowest_first() {
        let mut writer = BigWriter::new();
        writer.add_texture(
            "a.png",
            vec![(4, 4, vec![0; 64]), (2, 2, vec![0; 16]), (1, 1, vec![0; 4])],
        );
        writer.add_texture("b.png", vec![(4, 4, vec![0; 64]), (1, 1, vec![0; 4])]);
        let mut buf = Vec::new();
        writer.finish(&mut buf).unwrap();
        let (header, _) = parse_header(&buf).unwrap();
        let order = header.streaming_order();
        let (a0, c0) = order[0];
        let (a1, c1) = order[1];
        assert_eq!(header.assets[a0].chunks[c0].mip_level, 2);
        assert_eq!(header.assets[a1].chunks[c1].mip_level, 1);
    }

    #[test]
    fn sequential_read_no_gaps() {
        let mut writer = BigWriter::new();
        writer.add_texture("a.png", vec![(2, 2, vec![0xAA; 16]), (1, 1, vec![0xBB; 4])]);
        writer.add_texture("b.png", vec![(2, 2, vec![0xCC; 16]), (1, 1, vec![0xDD; 4])]);
        let mut buf = Vec::new();
        writer.finish(&mut buf).unwrap();
        let (header, _) = parse_header(&buf).unwrap();
        let order = header.streaming_order();
        assert_eq!(order.len(), 4);
        // Chunks are contiguous starting at data_offset.
        let mut expected_offset = header.data_offset;
        for &(ai, ci) in &order {
            let chunk = &header.assets[ai].chunks[ci];
            assert_eq!(
                chunk.offset, expected_offset,
                "chunk should be at expected offset"
            );
            expected_offset += chunk.compressed_size;
        }
    }
}
