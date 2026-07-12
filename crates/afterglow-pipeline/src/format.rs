// .big streaming container format — Command & Conquer: Generals inspired.
//
// 1. SEQUENTIAL ULTRA FRIENDLY — reading front-to-back gives progressive quality.
// 2. FULLY PARTIAL SEEKABLE — index has absolute offsets for every chunk.
// 3. COMPRESSED BY DEFAULT — all chunk data is meshopt-compressed.

use std::io::{self, Write};

pub const MAGIC: &[u8; 4] = b"BIG1";
pub const VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AssetType { Texture, Mesh, VirtualTexture }

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Compression { Meshopt, None }

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
    Texture { width: u32, height: u32 },
    Mesh { index_count: u32, vertex_count: u32, position_stride: u32, uv_stride: u32 },
    /// A virtual texture page — 128×128 texels (+ 4px border per side).
    /// Stored as a chunk in the .big file, seekable by offset.
    VirtualTexturePage {
        mip: u8,
        page_x: u32,
        page_y: u32,
    },
    Raw,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AssetEntry {
    pub name: String,
    pub asset_type: AssetType,
    pub chunks: Vec<ChunkInfo>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BigHeader {
    pub version: u32,
    pub data_offset: u64,
    pub assets: Vec<AssetEntry>,
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

fn compress_chunk(data: &[u8], compression: Compression) -> Vec<u8> {
    match compression {
        Compression::Meshopt => {
            let padded_len = (data.len() + 3) & !3;
            let mut padded = data.to_vec();
            padded.resize(padded_len, 0);
            afterglow_meshopt::safe::encode_vertex_buffer(&padded, 4)
        }
        Compression::None => data.to_vec(),
    }
}

// --- writer ---

pub struct BigWriter {
    assets: Vec<AssetEntry>,
    chunks: Vec<(usize, usize, Vec<u8>, Compression)>,
}

impl BigWriter {
    pub fn new() -> Self {
        Self { assets: Vec::new(), chunks: Vec::new() }
    }

    pub fn add_texture(&mut self, name: &str, mips: Vec<(u32, u32, Vec<u8>)>) {
        let asset_idx = self.assets.len();
        let mut chunks_meta = Vec::new();
        for (mip_level, (width, height, data)) in mips.into_iter().enumerate() {
            let uncompressed_size = data.len() as u64;
            self.chunks.push((asset_idx, mip_level, data, Compression::Meshopt));
            chunks_meta.push(ChunkInfo {
                offset: 0, compressed_size: 0, uncompressed_size,
                lod_level: 0, mip_level: mip_level as u8,
                compression: Compression::Meshopt,
                meta: ChunkMeta::Texture { width, height },
            });
        }
        self.assets.push(AssetEntry { name: name.to_string(), asset_type: AssetType::Texture, chunks: chunks_meta });
    }

    /// Add a virtual texture as a set of page chunks.
    /// Each page is 128×128 texels (+ 4px border per side = 136×136 slot).
    /// Pages are stored as individual seekable chunks in the .big file.
    pub fn add_virtual_texture(&mut self, name: &str, virtual_size: u32, pages: Vec<VirtualTexturePageData>) {
        let asset_idx = self.assets.len();
        let mut chunks_meta = Vec::new();
        for page in &pages {
            let uncompressed_size = page.data.len() as u64;
            self.chunks.push((asset_idx, chunks_meta.len(), page.data.clone(), Compression::None));
            chunks_meta.push(ChunkInfo {
                offset: 0, compressed_size: 0, uncompressed_size,
                lod_level: 0, mip_level: page.mip,
                compression: Compression::None,
                meta: ChunkMeta::VirtualTexturePage {
                    mip: page.mip,
                    page_x: page.page_x,
                    page_y: page.page_y,
                },
            });
        }
        self.assets.push(AssetEntry {
            name: name.to_string(),
            asset_type: AssetType::VirtualTexture,
            chunks: chunks_meta,
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
            self.chunks.push((asset_idx, lod_level, data, Compression::Meshopt));
            chunks_meta.push(ChunkInfo {
                offset: 0, compressed_size: 0, uncompressed_size,
                lod_level: lod_level as u8, mip_level: 0,
                compression: Compression::Meshopt,
                meta: ChunkMeta::Mesh {
                    index_count: lod.indices.len() as u32, vertex_count,
                    position_stride: lod.position_stride, uv_stride: lod.uv_stride,
                },
            });
        }
        self.assets.push(AssetEntry { name: name.to_string(), asset_type: AssetType::Mesh, chunks: chunks_meta });
    }

    pub fn finish(mut self, writer: &mut impl Write) -> io::Result<()> {
        let mut header = BigHeader { version: VERSION, data_offset: 0, assets: self.assets.clone() };
        let order = header.streaming_order();

        // Solve header size + chunk offsets. Iterate 3 times — always enough
        // for postcard varints (offset changes shift data_offset by ≤2 bytes,
        // which rarely changes varint encoding, and 3 passes converges).
        let mut header_bytes = Vec::new();
        for _ in 0..3 {
            header_bytes = postcard::to_allocvec(&header).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            let data_offset = 4 + 4 + 8 + header_bytes.len() as u64;
            let mut offset = data_offset;
            for &(ai, ci) in &order {
                let (_, _, data, comp) = self.chunks.iter().find(|(a, c, _, _)| *a == ai && *c == ci).unwrap();
                let compressed = compress_chunk(data, *comp);
                let chunk = &mut header.assets[ai].chunks[ci];
                chunk.offset = offset;
                chunk.compressed_size = compressed.len() as u64;
                chunk.compression = *comp;
                offset += chunk.compressed_size;
            }
            header.data_offset = data_offset;
        }
        // Final serialization with stable offsets.
        header_bytes = postcard::to_allocvec(&header).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        writer.write_all(MAGIC)?;
        writer.write_all(&VERSION.to_le_bytes())?;
        writer.write_all(&header.data_offset.to_le_bytes())?;
        writer.write_all(&header_bytes)?;
        for &(ai, ci) in &order {
            let (_, _, data, comp) = self.chunks.iter().find(|(a, c, _, _)| *a == ai && *c == ci).unwrap();
            writer.write_all(&compress_chunk(data, *comp))?;
        }
        Ok(())
    }
}

pub struct VirtualTexturePageData {
    pub mip: u8,
    pub page_x: u32,
    pub page_y: u32,
    /// Raw page data: SLOT_SIZE × SLOT_SIZE × 4 bytes (RGBA8, with border).
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
    if data.len() < 16 { return Err("file too small".into()); }
    if &data[0..4] != MAGIC { return Err(format!("bad magic")); }
    let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if version != VERSION { return Err(format!("version {version} != {VERSION}")); }
    let data_offset = u64::from_le_bytes(data[8..16].try_into().unwrap()) as usize;
    let header: BigHeader = postcard::from_bytes(&data[16..data_offset]).map_err(|e| format!("decode: {e}"))?;
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
            let decoded = afterglow_meshopt::safe::decode_vertex_buffer(compressed, padded_count, 4);
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
        writer.add_virtual_texture("terrain", 4096, vec![
            VirtualTexturePageData { mip: 0, page_x: 0, page_y: 0, data: page_data.clone() },
            VirtualTexturePageData { mip: 1, page_x: 0, page_y: 0, data: vec![0xCD; 136 * 136 * 4] },
        ]);
        let mut buf = Vec::new();
        writer.finish(&mut buf).unwrap();
        let (header, _) = parse_header(&buf).unwrap();
        assert_eq!(header.assets.len(), 1);
        assert_eq!(header.assets[0].asset_type, AssetType::VirtualTexture);
        assert_eq!(header.assets[0].chunks.len(), 2);
        // Verify page metadata
        match &header.assets[0].chunks[0].meta {
            ChunkMeta::VirtualTexturePage { mip, page_x, page_y } => {
                assert_eq!(*mip, 0);
                assert_eq!(*page_x, 0);
                assert_eq!(*page_y, 0);
            }
            _ => panic!("expected VirtualTexturePage"),
        }
        // Verify data roundtrip (Compression::None for VT pages)
        let c0 = &header.assets[0].chunks[0];
        assert_eq!(c0.compression, Compression::None);
        let decoded = read_chunk_decompressed(&buf, c0);
        assert_eq!(decoded, page_data);
    }

    #[test]
    fn roundtrip_compressed_texture() {
        let mut writer = BigWriter::new();
        let tex_data = vec![0xAB; 4 * 4 * 4];
        writer.add_texture("sky.png", vec![
            (4, 4, tex_data.clone()),
            (2, 2, vec![0xCD; 2 * 2 * 4]),
            (1, 1, vec![0xEF; 4]),
        ]);
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
    fn roundtrip_compressed_mesh() {
        let mut writer = BigWriter::new();
        writer.add_mesh("sphere", vec![
            MeshLodData {
                indices: vec![0, 1, 2, 0, 2, 3],
                positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0],
                uvs: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
                position_stride: 12, uv_stride: 8,
            },
        ]);
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
        writer.add_texture("a.png", vec![(4, 4, vec![0; 64]), (2, 2, vec![0; 16]), (1, 1, vec![0; 4])]);
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
            assert_eq!(chunk.offset, expected_offset, "chunk should be at expected offset");
            expected_offset += chunk.compressed_size;
        }
    }
}
