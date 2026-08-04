use crate::CharacterBakeError;

/// Counts from one area-weighted normal rebuild.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NormalBuildStats {
    pub triangles: u32,
    pub degenerate_triangles: u32,
    pub isolated_vertices: u32,
}

fn finite3(value: [f32; 3]) -> bool {
    value[0].is_finite() && value[1].is_finite() && value[2].is_finite()
}

/// Rebuild area-weighted vertex normals into a caller-owned array.
pub fn rebuild_area_weighted_normals(
    positions: &[[f32; 3]],
    indices: &[u32],
    output_normals: &mut [[f32; 3]],
) -> Result<NormalBuildStats, CharacterBakeError> {
    if positions.len() != output_normals.len() {
        return Err(CharacterBakeError::LengthMismatch);
    }
    if indices.is_empty() || !indices.len().is_multiple_of(3) {
        return Err(CharacterBakeError::InvalidTriangleList);
    }
    for position in positions {
        if !finite3(*position) {
            return Err(CharacterBakeError::NonFiniteValue);
        }
    }
    for index in indices {
        if *index as usize >= positions.len() {
            return Err(CharacterBakeError::IndexOutOfRange);
        }
    }

    output_normals.fill([0.0; 3]);
    let mut stats = NormalBuildStats {
        triangles: (indices.len() / 3) as u32,
        ..NormalBuildStats::default()
    };
    for triangle in indices.as_chunks::<3>().0 {
        let a = positions[triangle[0] as usize];
        let b = positions[triangle[1] as usize];
        let c = positions[triangle[2] as usize];
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let normal = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        let length_squared = normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2];
        if !length_squared.is_finite() {
            return Err(CharacterBakeError::NonFiniteValue);
        }
        if length_squared <= f32::EPSILON {
            stats.degenerate_triangles += 1;
            continue;
        }
        for index in triangle {
            let output = &mut output_normals[*index as usize];
            output[0] += normal[0];
            output[1] += normal[1];
            output[2] += normal[2];
        }
    }
    for normal in output_normals {
        let length_squared = normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2];
        if length_squared <= f32::EPSILON {
            *normal = [0.0; 3];
            stats.isolated_vertices += 1;
            continue;
        }
        let inverse_length = length_squared.sqrt().recip();
        normal[0] *= inverse_length;
        normal[1] *= inverse_length;
        normal[2] *= inverse_length;
    }
    Ok(stats)
}
