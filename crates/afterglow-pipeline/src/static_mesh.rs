use crate::{generate_lod_chain, MeshLodData};
use std::path::Path;

/// Cook one static triangle primitive into independently streamable LOD records.
/// Skins and morph targets are rejected rather than silently simplified.
pub fn cook_static_gltf_lods(
    path: &Path,
    lod_ratios: &[f32],
    target_error: f32,
) -> Result<Vec<MeshLodData>, String> {
    let gltf = gltf::Gltf::open(path).map_err(|error| error.to_string())?;
    if gltf.skins().next().is_some() {
        return Err("static LOD cook rejects skinned glTF assets".into());
    }
    let primitives: Vec<_> = gltf.meshes().flat_map(|mesh| mesh.primitives()).collect();
    if primitives.len() != 1 {
        return Err(format!("static LOD cook requires exactly one primitive, found {}", primitives.len()));
    }
    let primitive = &primitives[0];
    if primitive.mode() != gltf::mesh::Mode::Triangles {
        return Err("static LOD cook requires triangle-list topology".into());
    }
    if primitive.morph_targets().next().is_some() {
        return Err("static LOD cook rejects morph targets".into());
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut buffers = Vec::new();
    for buffer in gltf.buffers() {
        let data = match buffer.source() {
            gltf::buffer::Source::Bin => gltf.blob.as_ref()
                .ok_or_else(|| "GLB buffer has no BIN chunk".to_string())?.clone(),
            gltf::buffer::Source::Uri(uri) => {
                if uri.starts_with("data:") { return Err("static LOD cook does not accept data-URI buffers".into()); }
                std::fs::read(parent.join(uri)).map_err(|error| format!("failed to read buffer {uri}: {error}"))?
            }
        };
        buffers.push(data);
    }
    let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));
    let positions: Vec<f32> = reader.read_positions()
        .ok_or_else(|| "static LOD primitive has no POSITION".to_string())?
        .flatten().collect();
    let vertex_count = positions.len() / 3;
    let uvs: Vec<f32> = match reader.read_tex_coords(0) {
        Some(values) => values.into_f32().flatten().collect(),
        None => vec![0.0; vertex_count * 2],
    };
    let indices: Vec<u32> = match reader.read_indices() {
        Some(values) => values.into_u32().collect(),
        None => (0..u32::try_from(vertex_count).map_err(|_| "vertex count exceeds u32")?).collect(),
    };
    if indices.len() % 3 != 0 || indices.is_empty() {
        return Err("static LOD primitive has no complete triangles".into());
    }
    let cooked = generate_lod_chain(
        &indices, &positions, &uvs, 12, 8, lod_ratios, target_error,
    );
    Ok(cooked.into_iter().map(|lod| MeshLodData {
        indices: lod.indices,
        positions: lod.positions,
        uvs: lod.uvs,
        position_stride: lod.position_stride,
        uv_stride: lod.uv_stride,
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooks_cc0_avocado_into_descending_lods() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/lod-demo/Avocado.gltf");
        let lods = cook_static_gltf_lods(&source, &[1.0, 0.5, 0.25, 0.1], 0.02)
            .expect("Avocado static LOD cook");
        let triangles: Vec<_> = lods.iter().map(|lod| lod.indices.len() / 3).collect();
        assert_eq!(triangles, vec![682, 341, 170, 104]);
        assert!(lods.iter().all(|lod| lod.positions.len() == 1_218));
        assert!(lods.iter().all(|lod| lod.uvs.len() == 812));
    }

    #[test]
    fn missing_input_is_reported() {
        let error = cook_static_gltf_lods(Path::new("does-not-exist.gltf"), &[1.0], 0.01)
            .err().expect("missing input must fail");
        assert!(!error.is_empty());
    }
}
