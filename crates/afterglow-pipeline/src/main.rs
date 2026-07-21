// CLI entry point for afterglow-pipeline.
//
// Usage:
//   afterglow-pipeline process input_dir/ output.big
//   afterglow-pipeline inspect assets.big

use afterglow_pipeline::{
    cook_static_gltf_lods, embed_external_gltf, encode_height_r16_image, extract_glb_images, pack_mask_channels,
    parse_header, stream_virtual_texture, strip_glb_images_for_virtual_texturing,
    virtual_mip_tail_first_mip, BigWriter, TextureEncoding, TextureFormat, VirtualTextureMipTailData,
    VirtualTexturePageData,
};
use afterglow_pipeline::resident::{blue_noise_resident_payload, load_displacement_r8};
use rayon::prelude::*;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match cmd {
        "process" => process(&args[2..]),
        "inspect" => inspect(&args[2..]),
        "pack-masks" => pack_masks(&args[2..]),
        "height-r16" => height_r16(&args[2..]),
        "resident-texture" => resident_texture(&args[2..]),
        "blue-noise" => blue_noise(&args[2..]),
        "static-lod" => static_lod(&args[2..]),
        "help" | "--help" | "-h" | _ => {
            eprintln!("afterglow-pipeline — offline asset processor");
            eprintln!();
            eprintln!("Usage:");
            eprintln!("  afterglow-pipeline process <input_dir> <output.big>");
            eprintln!("  afterglow-pipeline inspect <file.big>");
            eprintln!("  afterglow-pipeline pack-masks <red.png> <green.png> <output.png>");
            eprintln!("  afterglow-pipeline height-r16 <height.png> <output.r16>");
            eprintln!("  afterglow-pipeline resident-texture <input.png> <output.big> [--format r8|rgba8] [--name <name>]");
            eprintln!("  afterglow-pipeline blue-noise <size> <output.big> [--name <name>]");
            eprintln!("  afterglow-pipeline static-lod <model.gltf|glb> <output.big>");
            eprintln!();
            eprintln!();
            eprintln!("Use static-lod for offline static-mesh simplification.");
        }
    }
}

fn static_lod(args: &[String]) {
    if args.len() != 2 {
        eprintln!("usage: afterglow-pipeline static-lod <model.gltf|glb> <output.big>");
        std::process::exit(1);
    }
    let input = PathBuf::from(&args[0]);
    let lods = cook_static_gltf_lods(&input, &[1.0, 0.5, 0.25, 0.1], 0.02)
        .unwrap_or_else(|error| panic!("failed to cook {}: {error}", input.display()));
    let triangles: Vec<_> = lods.iter().map(|lod| lod.indices.len() / 3).collect();
    let name = input.file_stem().and_then(|name| name.to_str()).unwrap_or("static-mesh");
    let mut writer = BigWriter::new();
    writer.add_mesh(name, lods);
    let mut file = std::fs::File::create(&args[1])
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", args[1]));
    writer.finish(&mut file)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", args[1]));
    eprintln!("[static-lod] {} → {} ({triangles:?} triangles)", input.display(), args[1]);
}

fn height_r16(args: &[String]) {
    if args.len() != 2 {
        eprintln!("usage: afterglow-pipeline height-r16 <height.png> <output.r16>");
        std::process::exit(1);
    }
    let image =
        image::open(&args[0]).unwrap_or_else(|error| panic!("failed to read {}: {error}", args[0]));
    let (width, height, encoded) = encode_height_r16_image(image)
        .unwrap_or_else(|error| panic!("failed to encode {}: {error}", args[0]));
    std::fs::write(&args[1], encoded)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", args[1]));
    eprintln!(
        "[height-r16] {} → {} ({}×{}, r16unorm)",
        args[0], args[1], width, height
    );
}

fn resident_texture(args: &[String]) {
    // afterglow-pipeline resident-texture <input.png|.r16> [<input2>...] <output.big> [--format r8|rgba8] [--name <name>]
    // Multiple inputs each become a named resident asset in one container.
    if args.len() < 2 {
        eprintln!("usage: afterglow-pipeline resident-texture <input.png|.r16> [<input2>...] <output.big> [--format r8|rgba8] [--name <name>]");
        std::process::exit(1);
    }
    let mut format = TextureFormat::R8;
    let mut name: Option<String> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                i += 1;
                format = match args.get(i).map(|s| s.as_str()) {
                    Some("r8") => TextureFormat::R8,
                    Some("rgba8") => TextureFormat::Rgba8,
                    other => {
                        eprintln!("--format expects r8|rgba8, got {:?}", other);
                        std::process::exit(1);
                    }
                };
            }
            "--name" => {
                i += 1;
                name = args.get(i).cloned();
            }
            _ => { /* positional — handled below */ }
        }
        i += 1;
    }
    // Positional inputs are all args except the last (output) and the flags.
    let mut positionals: Vec<usize> = Vec::new();
    let mut j = 0usize;
    while j < args.len() {
        match args[j].as_str() {
            "--format" | "--name" => { j += 2; continue; }
            _ => { positionals.push(j); j += 1; }
        }
    }
    if positionals.len() < 2 {
        eprintln!("resident-texture needs at least one input and an output.big");
        std::process::exit(1);
    }
    let (input_idxs, output_idx) = positionals.split_at(positionals.len() - 1);
    let output = PathBuf::from(&args[*output_idx.last().unwrap()]);
    let single_name = name.clone();
    let mut writer = BigWriter::new();
    for &idx in input_idxs {
        let input = PathBuf::from(&args[idx]);
        let asset_name = if input_idxs.len() == 1 {
            single_name.clone().unwrap_or_else(|| {
                input
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("resident-texture")
                    .to_string()
            })
        } else {
            input
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("resident-texture")
                .to_string()
        };
        let (width, height, bytes) = match format {
            TextureFormat::R8 => load_displacement_r8(&input)
                .unwrap_or_else(|e| panic!("failed to cook R8 {}: {e}", input.display())),
            TextureFormat::Rgba8 => {
                let img = image::open(&input)
                    .unwrap_or_else(|e| panic!("failed to read {}: {e}", input.display()))
                    .into_rgba8();
                let (w, h) = img.dimensions();
                (w, h, img.into_raw())
            }
        };
        writer.add_resident_texture(&asset_name, width, height, format, bytes);
        eprintln!(
            "[resident-texture] {} → {asset_name} ({width}×{height}, {format:?})",
            input.display()
        );
    }
    let mut file = std::fs::File::create(&output)
        .unwrap_or_else(|e| panic!("failed to create {}: {e}", output.display()));
    writer
        .finish(&mut file)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", output.display()));
    eprintln!("[resident-texture] wrote {} ({} asset(s))", output.display(), input_idxs.len());
}

fn blue_noise(args: &[String]) {
    // afterglow-pipeline blue-noise <size> <output.big> [--name <name>]
    if args.len() < 2 {
        eprintln!("usage: afterglow-pipeline blue-noise <size> <output.big> [--name <name>]");
        std::process::exit(1);
    }
    let size: u32 = args[0]
        .parse()
        .unwrap_or_else(|e| panic!("blue-noise size must be a power-of-two u32: {e}"));
    let output = PathBuf::from(&args[1]);
    let mut name = "blue-noise".to_string();
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => {
                i += 1;
                name = args.get(i).cloned().unwrap_or(name);
            }
            other => {
                eprintln!("unknown blue-noise flag: {other}");
                std::process::exit(1);
            }
        }
        i += 1;
    }
    let (w, h, fmt, bytes) = blue_noise_resident_payload(size);
    let mut writer = BigWriter::new();
    writer.add_resident_texture(&name, w, h, fmt, bytes);
    let mut file = std::fs::File::create(&output)
        .unwrap_or_else(|e| panic!("failed to create {}: {e}", output.display()));
    writer
        .finish(&mut file)
        .unwrap_or_else(|e| panic!("failed to write {}: {e}", output.display()));
    eprintln!("[blue-noise] {size}×{size} → {} ({name})", output.display());
}

fn pack_masks(args: &[String]) {
    if args.len() != 3 {
        eprintln!("usage: afterglow-pipeline pack-masks <red.png> <green.png> <output.png>");
        std::process::exit(1);
    }
    let red = image::open(&args[0])
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", args[0]))
        .into_rgba8();
    let green = image::open(&args[1])
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", args[1]))
        .into_rgba8();
    assert_eq!(
        red.dimensions(),
        green.dimensions(),
        "mask dimensions must match"
    );
    let (width, height) = red.dimensions();
    let packed =
        pack_mask_channels(red.as_raw(), green.as_raw()).expect("validated mask dimensions");
    let output =
        image::RgbaImage::from_raw(width, height, packed).expect("packed mask byte length");
    output
        .save(&args[2])
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", args[2]));
}

fn add_virtual_image(writer: &mut BigWriter, name: &str, image: image::RgbaImage) {
    eprintln!("[virtual-texture] {name} — tiling and UASTC encoding...");
    let (width, height) = image.dimensions();
    let first_tail_mip = virtual_mip_tail_first_mip(width, height)
        .expect("validated VT dimensions must produce a mip tail");
    let asset =
        writer.begin_virtual_texture(name, width, height, first_tail_mip, TextureEncoding::Basis);
    let mut page_count = 0usize;
    let tail = stream_virtual_texture(image.into_raw(), width, height, |batch| {
        page_count += batch.len();
        let encoded: Vec<_> = batch
            .par_iter()
            .map(|page| {
                let data = afterglow_basis_encoder::encode_uastc_rgba(&page.data, 136, 136)
                    .map_err(|error| {
                        format!(
                            "failed to encode {name} mip {} ({},{}): {error}",
                            page.mip, page.page_x, page.page_y
                        )
                    })?;
                Ok(VirtualTexturePageData {
                    mip: page.mip,
                    page_x: page.page_x,
                    page_y: page.page_y,
                    encoding: TextureEncoding::Basis,
                    data,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        for page in encoded {
            writer.push_virtual_texture_page(asset, page);
        }
        Ok(())
    })
    .unwrap_or_else(|error| panic!("failed to stream {name}: {error}"));
    let encoded_tail = afterglow_basis_encoder::encode_uastc_rgba(&tail.data, 136, 136)
        .unwrap_or_else(|error| panic!("failed to encode {name} mip tail: {error}"));
    writer.finish_virtual_texture(
        asset,
        VirtualTextureMipTailData {
            first_mip: tail.first_mip,
            encoding: TextureEncoding::Basis,
            data: encoded_tail,
        },
    );
    eprintln!("  → {width}×{height}, {page_count} bordered pages");
}

fn process(args: &[String]) {
    if args.len() < 2 {
        eprintln!("usage: afterglow-pipeline process <input_dir> <output.big>");
        std::process::exit(1);
    }
    let input_dir = PathBuf::from(&args[0]);
    let output_path = PathBuf::from(&args[1]);
    if !input_dir.exists() {
        eprintln!(
            "error: input directory '{}' does not exist",
            input_dir.display()
        );
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
                let image = image::open(&path)
                    .unwrap_or_else(|error| panic!("failed to decode {}: {error}", path.display()))
                    .into_rgba8();
                add_virtual_image(&mut writer, &name, image);
                asset_count += 1;
            }
            "glb" | "gltf" => {
                let bytes = if ext == "glb" {
                    std::fs::read(&path).unwrap_or_else(|error| {
                        panic!("failed to read {}: {error}", path.display())
                    })
                } else {
                    embed_external_gltf(&path).unwrap_or_else(|error| {
                        panic!("failed to embed {}: {error}", path.display())
                    })
                };
                let images = extract_glb_images(&bytes).unwrap_or_else(|error| {
                    panic!("failed to inspect {}: {error}", path.display())
                });
                let model_name = if ext == "glb" {
                    name.clone()
                } else {
                    format!("{}.glb", path.file_stem().unwrap().to_string_lossy())
                };
                let runtime_glb =
                    strip_glb_images_for_virtual_texturing(&bytes).unwrap_or_else(|error| {
                        panic!(
                            "failed to strip runtime images from {}: {error}",
                            path.display()
                        )
                    });
                eprintln!(
                    "[model] {model_name} — packing image-free GLB for runtime mesh optimization"
                );
                writer.add_raw_mesh(&model_name, runtime_glb);
                asset_count += 1;
                for embedded in images {
                    let vt_name = format!("{model_name}#image-{}", embedded.index);
                    let image = image::load_from_memory(&embedded.bytes)
                        .unwrap_or_else(|error| {
                            panic!(
                                "failed to decode {vt_name} ({}): {error}",
                                embedded.mime_type
                            )
                        })
                        .into_rgba8();
                    add_virtual_image(&mut writer, &vt_name, image);
                    asset_count += 1;
                }
            }
            _ => {
                // Skip unknown files.
            }
        }
    }

    // Write the .big file.
    let mut file = std::fs::File::create(&output_path).unwrap();
    writer.finish(&mut file).unwrap();
    eprintln!(
        "\nDone: {} assets → {} ({})",
        asset_count,
        output_path.display(),
        file.metadata().unwrap().len()
    );
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
        println!(
            "  {} ({:?}): {} chunks",
            asset.name,
            asset.asset_type,
            asset.chunks.len()
        );
        if let Some(vt) = &asset.virtual_texture {
            let pages: usize = vt.mips.iter().map(|mip| mip.page_sizes.len()).sum();
            println!(
                "    VT {}×{} {:?}: {} mips, {} pages, tail={}",
                vt.width,
                vt.height,
                vt.encoding,
                vt.mips.len(),
                pages,
                vt.tail.is_some()
            );
        }
        for chunk in &asset.chunks {
            let detail = match &chunk.meta {
                afterglow_pipeline::ChunkMeta::Texture { width, height, format } => {
                    format!("{width}×{height} {format:?}")
                }
                afterglow_pipeline::ChunkMeta::Mesh {
                    index_count,
                    vertex_count,
                    ..
                } => {
                    format!("{index_count} indices, {vertex_count} verts")
                }
                afterglow_pipeline::ChunkMeta::Raw => "raw".to_string(),
            };
            println!(
                "    LOD{} MIP{} {:?} {} ({}→{} bytes at offset {})",
                chunk.lod_level,
                chunk.mip_level,
                chunk.compression,
                detail,
                chunk.uncompressed_size,
                chunk.compressed_size,
                chunk.offset
            );
        }
    }
}
