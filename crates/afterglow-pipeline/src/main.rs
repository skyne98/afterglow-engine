// CLI entry point for afterglow-pipeline.
//
// Usage:
//   afterglow-pipeline process input_dir/ output.big
//   afterglow-pipeline inspect assets.big

use std::path::PathBuf;
use rayon::prelude::*;
use afterglow_pipeline::{
    BigWriter, TextureEncoding, VirtualTextureMipTailData, VirtualTexturePageData,
    pack_mask_channels, parse_header, stream_virtual_texture, virtual_mip_tail_first_mip,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match cmd {
        "process" => process(&args[2..]),
        "inspect" => inspect(&args[2..]),
        "pack-masks" => pack_masks(&args[2..]),
        "help" | "--help" | "-h" | _ => {
            eprintln!("afterglow-pipeline — offline asset processor");
            eprintln!();
            eprintln!("Usage:");
            eprintln!("  afterglow-pipeline process <input_dir> <output.big> [--texture-mips] [--mesh-lods N]");
            eprintln!("  afterglow-pipeline inspect <file.big>");
            eprintln!("  afterglow-pipeline pack-masks <red.png> <green.png> <output.png>");
            eprintln!();
            eprintln!("Options:");
            eprintln!("  --texture-mips    Generate mip chains for textures (default: on)");
            eprintln!("  --mesh-lods N     Generate N LOD levels for meshes (default: 4)");
            eprintln!("  --mesh-optimize   Optimize vertex cache + fetch (default: on)");
            eprintln!("  --compress        Compress mesh chunks with meshopt encode (default: on)");
        }
    }
}

fn pack_masks(args: &[String]) {
    if args.len() != 3 {
        eprintln!("usage: afterglow-pipeline pack-masks <red.png> <green.png> <output.png>");
        std::process::exit(1);
    }
    let red = image::open(&args[0]).unwrap_or_else(|error| panic!("failed to read {}: {error}", args[0])).into_rgba8();
    let green = image::open(&args[1]).unwrap_or_else(|error| panic!("failed to read {}: {error}", args[1])).into_rgba8();
    assert_eq!(red.dimensions(), green.dimensions(), "mask dimensions must match");
    let (width, height) = red.dimensions();
    let packed = pack_mask_channels(red.as_raw(), green.as_raw()).expect("validated mask dimensions");
    let output = image::RgbaImage::from_raw(width, height, packed).expect("packed mask byte length");
    output.save(&args[2]).unwrap_or_else(|error| panic!("failed to write {}: {error}", args[2]));
}

fn process(args: &[String]) {
    if args.len() < 2 {
        eprintln!("usage: afterglow-pipeline process <input_dir> <output.big>");
        std::process::exit(1);
    }
    let input_dir = PathBuf::from(&args[0]);
    let output_path = PathBuf::from(&args[1]);
    let _mesh_lods: usize = args.iter()
        .find(|a| a.starts_with("--mesh-lods="))
        .and_then(|a| a.split('=').nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);

    if !input_dir.exists() {
        eprintln!("error: input directory '{}' does not exist", input_dir.display());
        std::process::exit(1);
    }

    let mut writer = BigWriter::new();
    let mut asset_count = 0;

    // Universal VT: decode source images and emit independently seekable,
    // bordered virtual pages rather than conventional per-texture mip blobs.
    for entry in std::fs::read_dir(&input_dir).unwrap().flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        match ext {
            "png" | "jpg" | "jpeg" => {
                eprintln!("[virtual-texture] {name} — decoding and tiling...");
                let image = match image::open(&path) {
                    Ok(image) => image.into_rgba8(),
                    Err(error) => {
                        eprintln!("error: failed to decode {}: {error}", path.display());
                        std::process::exit(1);
                    }
                };
                let (width, height) = image.dimensions();
                let first_tail_mip = virtual_mip_tail_first_mip(width, height)
                    .expect("validated VT dimensions must produce a mip tail");
                let asset = writer.begin_virtual_texture(
                    &name, width, height, first_tail_mip, TextureEncoding::Basis,
                );
                let mut page_count = 0usize;
                let tail = stream_virtual_texture(image.into_raw(), width, height, |batch| {
                    page_count += batch.len();
                    let encoded: Vec<_> = batch.par_iter().map(|page| {
                        let data = afterglow_basis_encoder::encode_uastc_rgba(&page.data, 136, 136)
                            .map_err(|error| format!("failed to encode {name} mip {} ({},{}): {error}", page.mip, page.page_x, page.page_y))?;
                        Ok(VirtualTexturePageData {
                            mip: page.mip, page_x: page.page_x, page_y: page.page_y,
                            encoding: TextureEncoding::Basis, data,
                        })
                    }).collect::<Result<Vec<_>, String>>()?;
                    for page in encoded { writer.push_virtual_texture_page(asset, page); }
                    Ok(())
                }).unwrap_or_else(|error| panic!("failed to stream {name}: {error}"));
                let encoded_tail = afterglow_basis_encoder::encode_uastc_rgba(&tail.data, 136, 136)
                    .unwrap_or_else(|error| panic!("failed to encode {name} mip tail: {error}"));
                writer.finish_virtual_texture(asset, VirtualTextureMipTailData {
                    first_mip: tail.first_mip, encoding: TextureEncoding::Basis, data: encoded_tail,
                });
                eprintln!("  → {width}×{height}, {page_count} bordered pages");
                asset_count += 1;
            }
            _ => {
                // Skip unknown files.
            }
        }
    }

    // Write the .big file.
    let mut file = std::fs::File::create(&output_path).unwrap();
    writer.finish(&mut file).unwrap();
    eprintln!("\nDone: {} assets → {} ({})", asset_count, output_path.display(), file.metadata().unwrap().len());
}

fn inspect(args: &[String]) {
    if args.is_empty() {
        eprintln!("usage: afterglow-pipeline inspect <file.big>");
        std::process::exit(1);
    }
    let path = &args[0];
    let data = std::fs::read(path).unwrap();
    let (header, _) = match parse_header(&data) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    println!("{} (version {})", path, header.version);
    println!("{} assets:", header.assets.len());
    for asset in &header.assets {
        println!("  {} ({:?}): {} chunks", asset.name, asset.asset_type, asset.chunks.len());
        if let Some(vt) = &asset.virtual_texture {
            let pages: usize = vt.mips.iter().map(|mip| mip.page_sizes.len()).sum();
            println!("    VT {}×{} {:?}: {} mips, {} pages, tail={}",
                vt.width, vt.height, vt.encoding, vt.mips.len(), pages, vt.tail.is_some());
        }
        for chunk in &asset.chunks {
            let detail = match &chunk.meta {
                afterglow_pipeline::ChunkMeta::Texture { width, height } => {
                    format!("{width}×{height}")
                }
                afterglow_pipeline::ChunkMeta::Mesh { index_count, vertex_count, .. } => {
                    format!("{index_count} indices, {vertex_count} verts")
                }
                afterglow_pipeline::ChunkMeta::Raw => "raw".to_string(),
            };
            println!("    LOD{} MIP{} {:?} {} ({}→{} bytes at offset {})",
                chunk.lod_level, chunk.mip_level, chunk.compression, detail, chunk.uncompressed_size, chunk.compressed_size, chunk.offset);
        }
    }
}
