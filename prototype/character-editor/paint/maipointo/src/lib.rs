//! Maipointo (マイペイント) — from-scratch Rust reimplementation of the
//! libmypaint (brushlib) brush engine for the afterglow paint demo.
//!
//! Port status, in dependency order:
//! 1. [`mask`] — dab opacity mask + LRE encoding (`mypaint-tiled-surface.c`)
//! 2. [`brushmodes`] — pixel blend modes (`brushmodes.c`, non-spectral set)
//! 3. TODO: `mypaint-mapping.c` (input → setting curves)
//! 4. TODO: `mypaint-brush.c` stroke state machine (`stroke_to`, dabs)
//! 5. TODO: spectral `paint` modes (fastapprox `fastpow` port)
//! 6. TODO: `.myb` JSON brushes, wasm bindings, parity oracle vs vendored C

pub mod brushmodes;
pub mod mask;

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
