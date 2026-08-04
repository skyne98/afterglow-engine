use crate::CharacterBakeError;

/// One sparse position delta.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SparseDelta {
    pub vertex: u32,
    pub delta: [f32; 3],
}

/// One target with strictly ascending vertex indices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SparseTarget<'a> {
    pub deltas: &'a [SparseDelta],
}

fn finite3(value: [f32; 3]) -> bool {
    value[0].is_finite() && value[1].is_finite() && value[2].is_finite()
}

fn validate_target(
    target: SparseTarget<'_>,
    vertex_count: usize,
) -> Result<(), CharacterBakeError> {
    let mut previous = None;
    for delta in target.deltas {
        let vertex = delta.vertex as usize;
        if vertex >= vertex_count {
            return Err(CharacterBakeError::IndexOutOfRange);
        }
        if previous.is_some_and(|value| delta.vertex <= value) {
            return Err(CharacterBakeError::InvalidSparseTarget);
        }
        if !finite3(delta.delta) {
            return Err(CharacterBakeError::NonFiniteValue);
        }
        previous = Some(delta.vertex);
    }
    Ok(())
}

/// Evaluate a complete structural shape into caller-owned output positions.
pub fn evaluate_sparse_targets(
    neutral_positions: &[[f32; 3]],
    targets: &[SparseTarget<'_>],
    weights: &[f32],
    output_positions: &mut [[f32; 3]],
) -> Result<(), CharacterBakeError> {
    if neutral_positions.len() != output_positions.len() || targets.len() != weights.len() {
        return Err(CharacterBakeError::LengthMismatch);
    }
    for position in neutral_positions {
        if !finite3(*position) {
            return Err(CharacterBakeError::NonFiniteValue);
        }
    }
    for (target, weight) in targets.iter().copied().zip(weights.iter().copied()) {
        if !weight.is_finite() {
            return Err(CharacterBakeError::NonFiniteValue);
        }
        validate_target(target, neutral_positions.len())?;
        for delta in target.deltas {
            let neutral = neutral_positions[delta.vertex as usize];
            for (base, component) in neutral.iter().zip(delta.delta) {
                let value = *base + component * weight;
                if !value.is_finite() {
                    return Err(CharacterBakeError::NonFiniteValue);
                }
            }
        }
    }

    output_positions.copy_from_slice(neutral_positions);
    for (target, weight) in targets.iter().copied().zip(weights.iter().copied()) {
        if weight == 0.0 {
            continue;
        }
        for delta in target.deltas {
            let output = &mut output_positions[delta.vertex as usize];
            output[0] += delta.delta[0] * weight;
            output[1] += delta.delta[1] * weight;
            output[2] += delta.delta[2] * weight;
        }
    }
    if output_positions.iter().any(|position| !finite3(*position)) {
        return Err(CharacterBakeError::NonFiniteValue);
    }
    Ok(())
}

/// Change one sparse target contribution without rebuilding the neutral shape.
pub fn apply_sparse_target_delta(
    positions: &mut [[f32; 3]],
    target: SparseTarget<'_>,
    prior_weight: f32,
    next_weight: f32,
) -> Result<(), CharacterBakeError> {
    if !prior_weight.is_finite() || !next_weight.is_finite() {
        return Err(CharacterBakeError::NonFiniteValue);
    }
    validate_target(target, positions.len())?;
    let weight_delta = next_weight - prior_weight;
    if !weight_delta.is_finite() {
        return Err(CharacterBakeError::NonFiniteValue);
    }
    for delta in target.deltas {
        let position = positions[delta.vertex as usize];
        for (base, component) in position.iter().zip(delta.delta) {
            if !(*base + component * weight_delta).is_finite() {
                return Err(CharacterBakeError::NonFiniteValue);
            }
        }
    }
    for delta in target.deltas {
        let output = &mut positions[delta.vertex as usize];
        output[0] += delta.delta[0] * weight_delta;
        output[1] += delta.delta[1] * weight_delta;
        output[2] += delta.delta[2] * weight_delta;
    }
    Ok(())
}
