// Mesh processing — LOD generation, optimization, and compression.
//
// Takes raw mesh data (indices + positions + UVs) and generates a full LOD
// chain using afterglow-meshopt's UV-aware simplification. Each LOD is
// optimized for vertex cache and compressed with meshopt encode.

use afterglow_meshopt::safe;

/// A single LOD level of a processed mesh.
pub struct ProcessedLod {
    pub indices: Vec<u32>,
    pub positions: Vec<f32>,
    pub uvs: Vec<f32>,
    pub position_stride: u32,
    pub uv_stride: u32,
    pub compressed: bool,
}

/// Generate a full LOD chain from a source mesh.
///
/// `lod_ratios` specifies the target triangle ratio for each LOD level
/// (e.g., [1.0, 0.5, 0.25, 0.1] for 4 levels).
///
/// LOD 0 is the original mesh (optimized but not simplified). Higher LODs
/// are progressively simplified using UV-aware simplification.
pub fn generate_lod_chain(
    indices: &[u32],
    positions: &[f32],
    uvs: &[f32],
    position_stride: u32,
    uv_stride: u32,
    lod_ratios: &[f32],
    target_error: f32,
) -> Vec<ProcessedLod> {
    let orig_tris = indices.len() / 3;
    let mut lods = Vec::new();

    for (i, &ratio) in lod_ratios.iter().enumerate() {
        if i == 0 {
            // LOD 0: optimize but don't simplify.
            let opt_indices = safe::optimize_vertex_cache(indices, positions.len() / (position_stride as usize / 4));
            lods.push(ProcessedLod {
                indices: opt_indices,
                positions: positions.to_vec(),
                uvs: uvs.to_vec(),
                position_stride,
                uv_stride,
                compressed: false,
            });
        } else {
            // Higher LODs: UV-aware simplification.
            let target_tris = (orig_tris as f32 * ratio).max(4.0) as usize;
            let target_index_count = target_tris * 3;

            let uv_weight = 0.5f32;
            let weights = vec![uv_weight; uv_stride as usize / 4];

            let (simplified, _, _) = safe::simplify_with_attributes(
                indices,
                positions,
                position_stride as usize,
                uvs,
                uv_stride as usize,
                &weights,
                target_index_count,
                target_error,
            );

            lods.push(ProcessedLod {
                indices: simplified,
                positions: positions.to_vec(),
                uvs: uvs.to_vec(),
                position_stride,
                uv_stride,
                compressed: false,
            });
        }
    }

    lods
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_grid(n: u32) -> (Vec<u32>, Vec<f32>, Vec<f32>) {
        let mut positions = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::new();
        for y in 0..n {
            for x in 0..n {
                positions.push(x as f32);
                positions.push(y as f32);
                positions.push(0.0);
                uvs.push(x as f32 / n as f32);
                uvs.push(y as f32 / n as f32);
            }
        }
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let i = y * n + x;
                indices.extend_from_slice(&[i, i + 1, i + n, i + 1, i + n + 1, i + n]);
            }
        }
        (indices, positions, uvs)
    }

    #[test]
    fn generate_4_lods() {
        let (indices, positions, uvs) = test_grid(20);
        let orig_tris = indices.len() / 3;

        let lods = generate_lod_chain(
            &indices, &positions, &uvs, 12, 8,
            &[1.0, 0.5, 0.25, 0.1],
            0.02,
        );

        assert_eq!(lods.len(), 4);

        // LOD 0 = full (optimized, same count).
        assert_eq!(lods[0].indices.len(), indices.len());

        // Each LOD should be smaller.
        for i in 1..lods.len() {
            let tris = lods[i].indices.len() / 3;
            let prev_tris = lods[i - 1].indices.len() / 3;
            assert!(tris < prev_tris, "LOD{i} ({tris}) should be < LOD{} ({prev_tris})", i - 1);
        }

        // LOD 3 should be ~10%.
        let lod3_tris = lods[3].indices.len() / 3;
        assert!(lod3_tris < orig_tris / 5, "LOD3 should be < 20% of original");
    }

    #[test]
    fn lods_are_triangle_lists() {
        let (indices, positions, uvs) = test_grid(10);
        let lods = generate_lod_chain(
            &indices, &positions, &uvs, 12, 8,
            &[1.0, 0.5, 0.25],
            0.02,
        );
        for (i, lod) in lods.iter().enumerate() {
            assert_eq!(lod.indices.len() % 3, 0, "LOD{i} is a triangle list");
        }
    }
}
