use crate::{CharacterBakeError, SurfaceBinding};

/// Four normalized skin influences for one vertex.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkinInfluences {
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

fn add_influence(
    joints: &mut [u16; 12],
    weights: &mut [f32; 12],
    count: &mut usize,
    joint: u16,
    weight: f32,
) {
    if let Some(index) = joints[..*count]
        .iter()
        .position(|candidate| *candidate == joint)
    {
        weights[index] += weight;
        return;
    }
    joints[*count] = joint;
    weights[*count] = weight;
    *count += 1;
}

fn better(weight: f32, joint: u16, other_weight: f32, other_joint: u16) -> bool {
    weight > other_weight || (weight == other_weight && joint < other_joint)
}

fn calculate_influences(
    driver_skin: &[SkinInfluences],
    binding: &SurfaceBinding,
) -> Result<SkinInfluences, CharacterBakeError> {
    let mut aggregate_joints = [0_u16; 12];
    let mut aggregate_weights = [0.0_f32; 12];
    let mut aggregate_count = 0_usize;

    for parent in 0..3 {
        let source = driver_skin
            .get(binding.driver_vertices[parent] as usize)
            .ok_or(CharacterBakeError::IndexOutOfRange)?;
        let map_weight = binding.weights[parent];
        if !map_weight.is_finite() {
            return Err(CharacterBakeError::NonFiniteValue);
        }
        for influence in 0..4 {
            let source_weight = source.weights[influence];
            if !source_weight.is_finite() || source_weight < 0.0 {
                return Err(CharacterBakeError::NonFiniteValue);
            }
            if source_weight == 0.0 {
                continue;
            }
            let contribution = source_weight * map_weight;
            if !contribution.is_finite() {
                return Err(CharacterBakeError::NonFiniteValue);
            }
            add_influence(
                &mut aggregate_joints,
                &mut aggregate_weights,
                &mut aggregate_count,
                source.joints[influence],
                contribution,
            );
        }
    }

    let mut selected = SkinInfluences::default();
    let mut selected_count = 0_usize;
    for aggregate in 0..aggregate_count {
        let weight = aggregate_weights[aggregate];
        if weight <= 0.0 {
            continue;
        }
        let joint = aggregate_joints[aggregate];
        let mut insert = selected_count.min(4);
        for index in 0..selected_count.min(4) {
            if better(
                weight,
                joint,
                selected.weights[index],
                selected.joints[index],
            ) {
                insert = index;
                break;
            }
        }
        if insert >= 4 {
            continue;
        }
        let last = selected_count.min(3);
        for index in (insert + 1..=last).rev() {
            selected.weights[index] = selected.weights[index - 1];
            selected.joints[index] = selected.joints[index - 1];
        }
        selected.weights[insert] = weight;
        selected.joints[insert] = joint;
        selected_count = (selected_count + 1).min(4);
    }

    let sum: f32 = selected.weights[..selected_count].iter().sum();
    if !sum.is_finite() || sum <= 0.0 {
        return Err(CharacterBakeError::MissingSkinInfluence);
    }
    for weight in &mut selected.weights[..selected_count] {
        *weight /= sum;
    }
    Ok(selected)
}

/// Transfer driver skin weights through SurfaceWrap bindings.
///
/// Signed mapping contributions are aggregated first. Non-positive final bone
/// weights are removed before deterministic top-four selection.
pub fn transfer_skin_weights(
    driver_skin: &[SkinInfluences],
    bindings: &[SurfaceBinding],
    output_skin: &mut [SkinInfluences],
) -> Result<(), CharacterBakeError> {
    if bindings.len() != output_skin.len() {
        return Err(CharacterBakeError::LengthMismatch);
    }
    for binding in bindings {
        let _ = calculate_influences(driver_skin, binding)?;
    }
    for (output, binding) in output_skin.iter_mut().zip(bindings) {
        *output = calculate_influences(driver_skin, binding)?;
    }
    Ok(())
}
