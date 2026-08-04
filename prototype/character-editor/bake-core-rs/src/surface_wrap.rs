use crate::CharacterBakeError;

/// Two driver vertices and a source distance define one offset scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisScale {
    pub minimum_vertex: u32,
    pub maximum_vertex: u32,
    pub source_distance: f32,
}

/// Optional scale data for each engine-space axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceScale {
    pub x: Option<AxisScale>,
    pub y: Option<AxisScale>,
    pub z: Option<AxisScale>,
}

/// One output vertex bound to three driver vertices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceBinding {
    pub driver_vertices: [u32; 3],
    pub weights: [f32; 3],
    pub offset: [f32; 3],
}

impl SurfaceBinding {
    /// Make one exact driver-vertex binding.
    pub const fn exact(driver_vertex: u32) -> Self {
        Self {
            driver_vertices: [driver_vertex; 3],
            weights: [1.0, 0.0, 0.0],
            offset: [0.0; 3],
        }
    }
}

fn finite3(value: [f32; 3]) -> bool {
    value[0].is_finite() && value[1].is_finite() && value[2].is_finite()
}

fn axis_scale(
    driver_positions: &[[f32; 3]],
    reference: Option<AxisScale>,
    axis: usize,
) -> Result<f32, CharacterBakeError> {
    let Some(reference) = reference else {
        return Ok(1.0);
    };
    if !reference.source_distance.is_finite() || reference.source_distance <= 0.0 {
        return Err(CharacterBakeError::InvalidScale);
    }
    let minimum = driver_positions
        .get(reference.minimum_vertex as usize)
        .ok_or(CharacterBakeError::IndexOutOfRange)?;
    let maximum = driver_positions
        .get(reference.maximum_vertex as usize)
        .ok_or(CharacterBakeError::IndexOutOfRange)?;
    if !finite3(*minimum) || !finite3(*maximum) {
        return Err(CharacterBakeError::NonFiniteValue);
    }
    let scale = (maximum[axis] - minimum[axis]).abs() / reference.source_distance;
    if !scale.is_finite() {
        return Err(CharacterBakeError::InvalidScale);
    }
    Ok(scale)
}

/// Calculate the three offset scales from the current driver shape.
pub fn calculate_surface_scale(
    driver_positions: &[[f32; 3]],
    scale: SurfaceScale,
) -> Result<[f32; 3], CharacterBakeError> {
    Ok([
        axis_scale(driver_positions, scale.x, 0)?,
        axis_scale(driver_positions, scale.y, 1)?,
        axis_scale(driver_positions, scale.z, 2)?,
    ])
}

fn calculate_position(
    binding: &SurfaceBinding,
    driver_positions: &[[f32; 3]],
    scales: [f32; 3],
) -> Result<[f32; 3], CharacterBakeError> {
    if !finite3(binding.weights) || !finite3(binding.offset) {
        return Err(CharacterBakeError::NonFiniteValue);
    }
    let mut position = [0.0_f32; 3];
    for parent in 0..3 {
        let driver = driver_positions
            .get(binding.driver_vertices[parent] as usize)
            .ok_or(CharacterBakeError::IndexOutOfRange)?;
        if !finite3(*driver) {
            return Err(CharacterBakeError::NonFiniteValue);
        }
        let weight = binding.weights[parent];
        position[0] += driver[0] * weight;
        position[1] += driver[1] * weight;
        position[2] += driver[2] * weight;
    }
    position[0] += binding.offset[0] * scales[0];
    position[1] += binding.offset[1] * scales[1];
    position[2] += binding.offset[2] * scales[2];
    if !finite3(position) {
        return Err(CharacterBakeError::NonFiniteValue);
    }
    Ok(position)
}

/// Fit all output positions to the current driver shape.
///
/// The caller converts source coordinates before this operation. The function
/// preserves signed mapping weights and does not normalize or clamp them.
pub fn fit_surface(
    driver_positions: &[[f32; 3]],
    bindings: &[SurfaceBinding],
    scale: SurfaceScale,
    output_positions: &mut [[f32; 3]],
) -> Result<(), CharacterBakeError> {
    if output_positions.len() != bindings.len() {
        return Err(CharacterBakeError::LengthMismatch);
    }
    let scales = calculate_surface_scale(driver_positions, scale)?;
    for binding in bindings {
        let _ = calculate_position(binding, driver_positions, scales)?;
    }
    for (output, binding) in output_positions.iter_mut().zip(bindings) {
        *output = calculate_position(binding, driver_positions, scales)?;
    }
    Ok(())
}
