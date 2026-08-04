//! Fixed-workspace algorithms for character source evaluation and baking.
//!
//! The geometric fitting semantics use Humentity and `bevy_make_human` as the
//! co-primary permissive references. See `THIRD_PARTY_NOTICES.md`.

mod error;
mod macro_weights;
mod morph;
mod normals;
mod skin;
mod surface_wrap;

pub use error::CharacterBakeError;
pub use macro_weights::{
    MacroProductTerm, MacroSegment, NO_MACRO_STATE, compose_macro_products, resolve_piecewise_macro,
};
pub use morph::{SparseDelta, SparseTarget, apply_sparse_target_delta, evaluate_sparse_targets};
pub use normals::{NormalBuildStats, rebuild_area_weighted_normals};
pub use skin::{SkinInfluences, transfer_skin_weights};
pub use surface_wrap::{
    AxisScale, SurfaceBinding, SurfaceScale, calculate_surface_scale, fit_surface,
};

#[cfg(test)]
mod tests;
