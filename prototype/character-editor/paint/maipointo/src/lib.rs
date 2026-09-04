#![feature(allocator_api)]

//! Maipointo (マイペイント) — from-scratch Rust reimplementation of the
//! libmypaint (brushlib) brush engine for the afterglow paint demo.
//!
//! Port status, in dependency order:
//! 1. [`mask`] — dab opacity mask + LRE encoding (`mypaint-tiled-surface.c`)
//! 2. [`brushmodes`] — pixel blend modes (`brushmodes.c`, non-spectral set +
//!    the spectral `paint` modes with the fastapprox `fastpow` port)
//! 3. [`mapping`] — input→setting curves (`mypaint-mapping.c`)
//! 4. [`rngdouble`] — Knuth lagged-Fibonacci brush RNG (`rng-double.c`)
//! 5. [`settings`] — settings/input tables (generated from
//!    `brushsettings.json` at build time)
//! 6. [`brush`] — `mypaint-brush.c` stroke state machine (`stroke_to`,
//!    dabs, smudge) + the cooperative budgeted driver
//! 7. [`web_surface`]/[`app`]/[`demo_capi`] — demo surface, layer/group app,
//!    and the wasm C ABI (feature `demo`)
//! 8. TODO: nothing from the engine core; `.myb` JSON loading is in
//!    [`capi_json`]

#[cfg(feature = "demo")]
pub mod app;
pub mod brush;
pub mod brushmodes;
pub mod capi_json;
pub mod compositor;
#[cfg(feature = "demo")]
pub mod demo_capi;
pub mod helpers;
pub mod mapping;
pub mod mask;
pub mod random;
pub mod rngdouble;
pub mod settings;
pub mod surface;
pub mod symmetry;
#[cfg(feature = "demo")]
pub mod web_surface;

/// C ABI for the demo's Emscripten wasm module (wasm builds only; the
/// native test builds have no libmypaint surface symbols to link against).
pub use brushmodes::ColorSums;
pub use mask::{TILE_SIZE, clamp, render_dab_mask};

/// One 64×64 RGBA tile of premultiplied fix15 `u16` pixels (row-major).
#[derive(Clone)]
pub struct Tile {
    pub data: Vec<u16>,
}

impl Tile {
    pub fn new() -> Self {
        Self {
            data: vec![0; TILE_SIZE * TILE_SIZE * 4],
        }
    }

    pub fn pixel(&self, x: usize, y: usize) -> brushmodes::Pixel {
        let o = (y * TILE_SIZE + x) * 4;
        [
            self.data[o],
            self.data[o + 1],
            self.data[o + 2],
            self.data[o + 3],
        ]
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, px: brushmodes::Pixel) {
        let o = (y * TILE_SIZE + x) * 4;
        self.data[o..o + 4].copy_from_slice(&px);
    }
}
