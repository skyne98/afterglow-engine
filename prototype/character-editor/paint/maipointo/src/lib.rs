//! Maipointo (マイペイント) — from-scratch Rust reimplementation of the
//! libmypaint (brushlib) brush engine for the afterglow paint demo.
//!
//! Port status, in dependency order:
//! 1. [`mask`] — dab opacity mask + LRE encoding (`mypaint-tiled-surface.c`)
//! 2. [`brushmodes`] — pixel blend modes (`brushmodes.c`, non-spectral set)
//! 3. [`mapping`] — input→setting curves (`mypaint-mapping.c`)
//! 4. [`rngdouble`] — Knuth lagged-Fibonacci brush RNG (`rng-double.c`)
//! 5. [`settings`] — settings/input tables (generated from
//!    `brushsettings.json` at build time)
//! 6. TODO: `mypaint-brush.c` stroke state machine (`stroke_to`, dabs, smudge)
//! 7. TODO: spectral `paint` modes (fastapprox `fastpow` port) — NOTE: this
//!    NG version defaults `paint_mode` to 1.0, so this is required for
//!    default-brush parity, not an optional extra
//! 8. TODO: `.myb` JSON brushes, wasm bindings

pub mod brush;
pub mod compositor;
pub mod brushmodes;
pub mod helpers;
pub mod mapping;
pub mod mask;
pub mod random;
pub mod rngdouble;
pub mod surface;
pub mod symmetry;
#[cfg(feature = "demo")]
pub mod web_surface;
#[cfg(feature = "demo")]
pub mod app;
#[cfg(feature = "demo")]
pub mod demo_capi;
pub mod settings;
pub mod capi_json;

/// C ABI for the demo's Emscripten wasm module (wasm builds only; the
/// native test builds have no libmypaint surface symbols to link against).

pub use brushmodes::ColorSums;
pub use mask::{clamp, render_dab_mask, TILE_SIZE};

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
