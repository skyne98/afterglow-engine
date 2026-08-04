use core::fmt;

/// A deterministic error from a character bake algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterBakeError {
    LengthMismatch,
    IndexOutOfRange,
    NonFiniteValue,
    InvalidScale,
    InvalidSparseTarget,
    InvalidMacro,
    InvalidTriangleList,
    MissingSkinInfluence,
}

impl fmt::Display for CharacterBakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::LengthMismatch => "character data lengths do not match",
            Self::IndexOutOfRange => "character data has an out-of-range index",
            Self::NonFiniteValue => "character data has a non-finite value",
            Self::InvalidScale => "surface scale data is invalid",
            Self::InvalidSparseTarget => "sparse target data is invalid",
            Self::InvalidMacro => "macro weight data is invalid",
            Self::InvalidTriangleList => "mesh indices are not a valid triangle list",
            Self::MissingSkinInfluence => "surface binding has no positive skin influence",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for CharacterBakeError {}
