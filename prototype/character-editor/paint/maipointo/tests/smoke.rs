//! End-to-end smoke test: a default-brush stroke on the fixed surface.

use maipointo::brush::Brush;
use maipointo::settings::SettingId;
use maipointo::surface::FixedTiledSurface;

#[test]
fn default_brush_stroke_renders() {
    let mut surface = FixedTiledSurface::new(256, 256);
    let mut brush = Brush::new();
    brush.from_defaults();
    // Paint something visible (default color is white on a white canvas).
    brush.set_base_value(SettingId::ColorV, 0.35);

    // Warm-up event, then a stroke across the canvas.
    brush.stroke_to(&mut surface, 64.0, 128.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, false);
    let mut painted = 0u64;
    for i in 0..80 {
        let t = i as f32 / 79.0;
        let _ = brush.stroke_to(
            &mut surface,
            64.0 + t * 128.0,
            128.0 + (t * std::f32::consts::PI * 2.0).sin() * 30.0,
            0.6,
            0.0,
            0.0,
            0.012,
            1.0,
            0.0,
            0.0,
            false,
        );
    }
    surface.flush_all();
    for &w in surface.tile_bytes() {
        if w != 0xFFFF {
            painted += 1;
        }
    }
    assert!(painted > 100, "stroke painted nothing (words={painted})");
}
