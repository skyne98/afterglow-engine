// CLI entry point for afterglow-pipeline.
//
// Usage:
//   afterglow-pipeline process input_dir/ output.big
//   afterglow-pipeline inspect assets.big

use std::path::PathBuf;
use std::io::Write;

use afterglow_pipeline::{BigWriter, parse_header, generate_mip_chain};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match cmd {
        "process" => process(&args[2..]),
        "inspect" => inspect(&args[2..]),
        "help" | "--help" | "-h" | _ => {
            eprintln!("afterglow-pipeline — offline asset processor");
            eprintln!();
            eprintln!("Usage:");
            eprintln!("  afterglow-pipeline process <input_dir> <output.big> [--texture-mips] [--mesh-lods N]");
            eprintln!("  afterglow-pipeline inspect <file.big>");
            eprintln!();
            eprintln!("Options:");
            eprintln!("  --texture-mips    Generate mip chains for textures (default: on)");
            eprintln!("  --mesh-lods N     Generate N LOD levels for meshes (default: 4)");
            eprintln!("  --mesh-optimize   Optimize vertex cache + fetch (default: on)");
            eprintln!("  --compress        Compress mesh chunks with meshopt encode (default: on)");
        }
    }
}

fn process(args: &[String]) {
    if args.len() < 2 {
        eprintln!("usage: afterglow-pipeline process <input_dir> <output.big>");
        std::process::exit(1);
    }
    let input_dir = PathBuf::from(&args[0]);
    let output_path = PathBuf::from(&args[1]);
    let mesh_lods: usize = args.iter()
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

    // Process textures (.png, .jpg — raw RGBA would be decoded here).
    // For now, we generate synthetic test textures since we don't have an image decoder.
    // In production, use the `image` crate to decode.
    for entry in std::fs::read_dir(&input_dir).unwrap().flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        match ext {
            "png" | "jpg" | "jpeg" => {
                eprintln!("[texture] {name} — generating mips...");
                // TODO: decode image to RGBA. For now, create a synthetic texture.
                let size = 256u32;
                let data = vec![200u8; (size * size * 4) as usize];
                let mips = generate_mip_chain(&data, size, size);
                let mip_count = mips.len();
                writer.add_texture(&name, mips);
                eprintln!("  → {mip_count} mip levels");
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
