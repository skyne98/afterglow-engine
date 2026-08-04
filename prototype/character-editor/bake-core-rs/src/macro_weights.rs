use crate::CharacterBakeError;

/// A missing low or high state in one macro segment.
pub const NO_MACRO_STATE: u16 = u16::MAX;

/// One linear part of a piecewise macro control.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MacroSegment {
    pub lowest: f32,
    pub highest: f32,
    pub low_state: u16,
    pub high_state: u16,
}

/// One target weight made from a product of resolved macro states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacroProductTerm {
    pub target: u32,
    pub first_factor: u32,
    pub factor_count: u16,
}

fn validate_state(state: u16, output_count: usize) -> Result<(), CharacterBakeError> {
    if state != NO_MACRO_STATE && state as usize >= output_count {
        return Err(CharacterBakeError::IndexOutOfRange);
    }
    Ok(())
}

/// Resolve one piecewise control into fixed state weights.
pub fn resolve_piecewise_macro(
    value: f32,
    segments: &[MacroSegment],
    state_weights: &mut [f32],
) -> Result<(), CharacterBakeError> {
    if !value.is_finite() || segments.is_empty() {
        return Err(CharacterBakeError::InvalidMacro);
    }
    let mut prior_highest = None;
    for segment in segments {
        if !segment.lowest.is_finite()
            || !segment.highest.is_finite()
            || segment.highest <= segment.lowest
            || prior_highest.is_some_and(|highest| segment.lowest < highest)
        {
            return Err(CharacterBakeError::InvalidMacro);
        }
        validate_state(segment.low_state, state_weights.len())?;
        validate_state(segment.high_state, state_weights.len())?;
        prior_highest = Some(segment.highest);
    }
    let segment = segments
        .iter()
        .find(|segment| value >= segment.lowest && value <= segment.highest)
        .ok_or(CharacterBakeError::InvalidMacro)?;
    let interpolation = (value - segment.lowest) / (segment.highest - segment.lowest);
    state_weights.fill(0.0);
    if segment.low_state != NO_MACRO_STATE {
        state_weights[segment.low_state as usize] += 1.0 - interpolation;
    }
    if segment.high_state != NO_MACRO_STATE {
        state_weights[segment.high_state as usize] += interpolation;
    }
    Ok(())
}

/// Compose precomputed macro target terms from resolved state weights.
///
/// `factors` stores state indices. Each term references one contiguous factor
/// range and adds their product to its target.
pub fn compose_macro_products(
    state_weights: &[f32],
    factors: &[u16],
    terms: &[MacroProductTerm],
    target_weights: &mut [f32],
) -> Result<(), CharacterBakeError> {
    if state_weights.iter().any(|weight| !weight.is_finite()) {
        return Err(CharacterBakeError::NonFiniteValue);
    }
    for term in terms {
        let first = term.first_factor as usize;
        let end = first
            .checked_add(term.factor_count as usize)
            .ok_or(CharacterBakeError::InvalidMacro)?;
        if term.factor_count == 0
            || end > factors.len()
            || term.target as usize >= target_weights.len()
        {
            return Err(CharacterBakeError::IndexOutOfRange);
        }
        for state in &factors[first..end] {
            if *state as usize >= state_weights.len() {
                return Err(CharacterBakeError::IndexOutOfRange);
            }
        }
    }

    target_weights.fill(0.0);
    for term in terms {
        let first = term.first_factor as usize;
        let end = first + term.factor_count as usize;
        let mut product = 1.0_f32;
        for state in &factors[first..end] {
            product *= state_weights[*state as usize];
        }
        if !product.is_finite() {
            return Err(CharacterBakeError::NonFiniteValue);
        }
        target_weights[term.target as usize] += product;
    }
    if target_weights.iter().any(|weight| !weight.is_finite()) {
        return Err(CharacterBakeError::NonFiniteValue);
    }
    Ok(())
}
