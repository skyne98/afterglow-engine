# libmypaint NG → WebAssembly

Date: 2026-08-02
Status: working build with tiled WASM surface

The original proxy recipe in this directory is historical.
The active recipe is `prototype/character-editor/paint/build-wasm.sh`.
The active module compiles the ISC tiled surface, fixed operation queue, background, layers, and RGBA8 display conversion.

Compiled the latest **NG** (v2.0 `master`) libmypaint brush engine to
WebAssembly, following the approach of `eliot-akira/brushlib-wasm` but for the
next-generation (v2) core.

## Result

The active `brushlib.wasm` is about 140 KB plus `brushlib.js`, built with
Emscripten 6.0.2. The module loads in the browser, creates a tiled surface,
loads all 196 stock brushes, paints RGBA16 tiles, and exports RGBA8 display tiles.

## What "NG" means

libmypaint `master` is now **2.0.0** (`libmypaint_api_major=2`). Compared to
eliot's vendored 1.3.0, the NG core:
- requires **json-c** (parses `brushsettings.json`; 30 `json_object_*` calls in
  `mypaint-brush.c`);
- uses **GLib types** only via `mypaint-glib-compat.h` when built with
  `MYPAINT_CONFIG_USE_GLIB 0` — the wasm clean path;
- changed `draw_dab` signature (adds softness, posterize, posterize_num, paint)
  and `stroke_to` (adds viewzoom, viewrotation, barrel_rotation, linear).

## Build recipe (files in this dir)

- `build.sh` — historical proxy emcc command.
- `config.h` — sets `MYPAINT_CONFIG_USE_GLIB 0` (use glib-compat, no real GLib).
- `proxy-surface.c/.h` — historical callback bridge.
- `main.c` — eliot-style exported API:
  `init`, `new_brush`, `load_brush`, `begin_stroke`, `reset_brush`,
  `set_brush_base_value`, `set_brush_mapping_n`, `set_brush_mapping_point`,
  `stroke_to` (NG args).
- `prototype/character-editor/scripts/import-mypaint-brushes.ts` — imports the
  196 default `.myb` files and their `_prev.png` previews into the demo.

## Dependencies

- **json-c** must be cross-compiled to wasm first (static-AR). The recipe
  references `/tmp/libmypaint-wasm/json-c/build/libjson-c.a`.
- Two generated headers from `generate.py` (run from the repo):
  `python3 generate.py mypaint-brush-settings-gen.h brushsettings-gen.h`.

## MyPaint Python reference

The GUI input behavior follows MyPaint's Python source at commit
`35aa9d33cd3deba6cafea6d8fc901b5a1d161ceb`:

- `gui/freehand.py` starts input on button 1, uses fake mouse pressure `0.5`,
  queues motion only while the button is down, and sends pressure `0.0` on release.
- `lib/stroke.py` calls `brush.new_stroke()` before each recorded stroke.
- `lib/command.py` uses `brush.reset()` for an abrupt new segment, then primes
  the brush at the contact point with zero pressure. The demo does the same for
  each pointer-down so separate clicks cannot connect.
- `gui/brushmodifier.py` identifies dedicated erasers from the `eraser` brush
  setting. The NG C core sends this as an alpha target in `draw_dab`, and the
  active tiled surface processes it in premultiplied RGBA16.

## Notes

- The NG core's only real-GLib refs are 3 commented-out `g_print` calls; the
  GLib-compat shim supplies the rest. So wasm is natural.
- The active surface stores 64 x 64 RGBA16 premultiplied tiles and exports dirty
  RGBA8 tiles. Canvas2D no longer paints dabs.
- Alternative: `reearth/hokusai` (Apache-2.0) is a pure-Rust, wasm-ready,
  `.myb`-compatible port — a stronger fit for afterglow's Rust tooling. This
  emcc build is the mechanical proof and a C-level reference.
