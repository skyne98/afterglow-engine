# MyPaint Python GUI and pixel pipeline audit

Date: 2026-08-09

Reference: MyPaint source commit `35aa9d33cd3deba6cafea6d8fc901b5a1d161ceb`.
The source is the reference for input, brush state, pixel storage, blending, and display behavior.

## Result

The complete implementation plan is [`../implementation/libmypaint-complete-web-plan.md`](../implementation/libmypaint-complete-web-plan.md).

The current demo proves the NG brush engine, `.myb` loading, preview loading, pointer input, and Canvas2D dab output.
It does not yet reproduce the MyPaint pixel surface or the MyPaint display path.

The largest gap is the surface callback. The current proxy sends each `draw_dab` call to a Canvas2D radial gradient.
MyPaint sends the call to a 64 x 64 tiled 16-bit premultiplied RGBA surface.

## 1. Python input path

MyPaint uses `gui/freehand.py` as an input capture and cleanup layer.

1. GTK receives button, motion, pressure, tilt, view, and barrel-rotation data.
2. Button 1 starts an input stroke.
3. A mouse uses fake pressure, normally `0.5`.
4. A release sends a zero-pressure event to make a clean tail.
5. Motion data enters a queue before brush processing.
6. Equal-time events receive interpolated times.
7. Missing pressure and tilt values use `PressureAndTiltInterpolator`.
8. Zero-to-positive and positive-to-zero transitions clear interpolation history.
9. The processor clamps pressure and tilt before `stroke_to`.
10. A device change resets the brush when needed.

The current demo now follows the abrupt-start and release behavior.
It does not yet have the raw-event queue, equal-time cleanup, axis interpolation, last-good-axis recovery, or device-change handling.

The Python path passes these values to the engine:

- model-space X and Y
- pressure in `[0, 1]`
- X and Y tilt in `[-1, 1]`
- view zoom
- view rotation
- barrel rotation
- event time delta
- linear-color flag

The current demo passes fixed view zoom `1`, view rotation `0`, barrel rotation `0`, and linear flag `0`.

## 2. Stroke state

`gui/mode.py` and `lib/command.py` create a `Brushwork` command for each active segment.
The first segment starts abruptly.

An abrupt segment performs this sequence before the real sample:

1. Call `brush.reset()`.
2. Call `stroke_to` at the contact point.
3. Use pressure `0.0`.
4. Use `dtime` `10.0`.
5. Then send the real pressure sample.

This prime is necessary because brush settings such as `slow_tracking` use the prior virtual cursor state.
The current demo now implements this sequence in `begin_stroke`.

`lib/stroke.py` records the complete nine-value input event for replay.
It also stores the brush JSON and the brush state at stroke start.
The current demo does not record events, state, undo data, or replay data.

## 3. Brush preset behavior

`gui/brushmanager.py` manages more than a list of files.
It loads stock and user brush paths, parses `order.conf`, manages groups, history, favorites, device mappings, and brush packs.
The default stock brush is `Dieterle/Fan#1`.
The default eraser is `deevad/kneaded_eraser_large`.

`gui/brushmodifier.py` loads a selected brush into one working brush instance.
It also:

- preserves the active color based on `restore_color`
- preserves or clears lock-alpha mode
- identifies dedicated erasers from the `eraser` setting
- updates normal, eraser, lock-alpha, and colorize mode state
- keeps a copy of the unmodified selected brush

The current demo includes the 196 default brushes and previews.
It does not implement `order.conf`, history, favorites, user brush paths, device mappings, brush packs, `restore_color`, or mode state.

The current demo also uses the HTML color for every dab.
It does not use the brush engine RGB output from the selected `.myb` file.
This is a visible difference for fixed-color brushes and color dynamics.

## 4. Brush settings and color

`lib/brush.py` wraps the C brush and sends all settings to the engine.
The Python wrapper applies the current EOTF to HSV values when needed.
The selected brush can contain:

- base values
- input mappings
- string fields
- color dynamics
- smudge settings
- paint mode
- alpha and blend settings

The active demo loads the JSON through `mypaint_brush_from_string`.
That preserves the engine settings and mappings.
The HTML sliders override radius, hardness, and opacity.
The HTML color updates the engine HSV settings after conversion to linear RGB.

## 5. Brush dab mask

The NG C tiled surface uses `mypaint-tiled-surface.c`.
For each dab it:

1. clamps radius-related values and mode values
2. rejects radius below `0.1` pixels
3. rejects zero hardness, zero opacity, and full softness
4. expands the dab by one pixel for the fringe
5. finds all affected 64 x 64 tiles
6. queues one operation per affected tile
7. renders a mask for each tile
8. processes the mask with run-length encoding
9. updates dirty bounding boxes
10. flushes the queued tile work at the atomic boundary

The mask uses two linear opacity segments.
It uses `rr = distance squared / radius squared`.
It has a special anti-alias path for radius below `3` pixels.
It clamps aspect ratio to at least `1` and applies angle in degrees.

The current radial gradient is only an approximation.
It differs in hardness, softness, small-radius anti-aliasing, pixel-center rules, fringe size, aspect handling, and edge rounding.

## 6. Pixel storage

MyPaint uses tiles of `64 x 64` pixels.
Each tile stores four unsigned 16-bit channels.
The channels use a 15-bit range with `32768` as the full value.
The stored colors are premultiplied by alpha.
Therefore stored RGB values cannot exceed stored alpha.

Empty tiles are not allocated.
Read-only snapshots use copy-on-write tiles.
Mipmap tiles are marked dirty after a write and rebuilt from four child tiles.

The current demo uses a browser Canvas2D surface.
The browser surface uses implementation-defined 8-bit premultiplied storage and does not expose the MyPaint tile data.
It has no fixed tile pool, no MyPaint mipmaps, no copy-on-write snapshot, and no exact 16-bit path.

## 7. Dab pixel processing

The MyPaint tiled surface applies the mask to premultiplied pixels.
Normal mode uses source-over equations:

- `out_alpha = source_alpha + (1 - source_alpha) * destination_alpha`
- `out_color = source_color + (1 - source_alpha) * destination_color`

The surface supports two normal paint paths:

- additive normal blending
- spectral WGM pigment blending

The surface also supports smudge and eraser behavior.
The `color_a` argument is a target alpha.
A normal dab uses target alpha `1`.
An eraser uses target alpha `0`.
Smudge uses the sampled target alpha.

The current demo uses Canvas2D `source-over` for normal dabs and `destination-out` for eraser dabs.
It does not implement exact premultiplied 16-bit equations or partial-alpha smudge and eraser behavior.

## 8. Surface color sampling

MyPaint `get_color` samples the real surface under the dab.
It first flushes queued dab operations for affected tiles.
It creates a mask and samples the masked pixels.
Small brushes sample every pixel.
Large brushes use a guaranteed interval plus random samples to bound work.

The sample returns RGB and alpha.
Paint mode can combine RGB samples with the ten-channel spectral WGM path.

This callback supplies the input for smudge, watercolor, and related brushes.

The active demo uses the libmypaint tiled-surface sampler.
Therefore smudge, watercolor, pigment mixing, and other canvas-dependent brushes use the sampled tile pixels.
The remaining difference is the bounded fixed operation queue, which preserves dab order but has a finite capacity.

## 9. Brush-level modes

The NG surface supports these brush-level operations:

- normal
- pigment paint
- lock alpha
- colorize
- posterize
- eraser target alpha
- smudge color and alpha

At the time of this audit, the proxy demo only approximated normal and eraser behavior.
The active WASM brush path now uses the vendored ISC tiled surface, which supplies lock alpha, colorize, posterize, paint factor, smudge, and brush-generated color.

## 10. Layer compositing

MyPaint has a layer tree.
A render operation list can contain:

- background blit
- layer composite
- isolated group push
- isolated group pop

Each layer has visibility, opacity, and a combine mode.
Groups can use pass-through or isolated composition.

The available layer modes include:

- normal
- pigment
- multiply
- screen
- overlay
- darken
- lighten
- color dodge
- color burn
- hard light
- soft light
- difference
- exclusion
- hue
- saturation
- color
- luminosity
- plus
- destination-in
- destination-out
- source-atop
- destination-atop

At the time of this audit, the proxy demo had one flat Canvas2D target.
The active WASM demo now has bounded paint layers, opacity, the 22 flat layer modes, background composition, and 15-bit premultiplied output.
Group isolation and pass-through remain open.

## 11. Color space and alpha conversion

MyPaint internal tiles remain in a high-precision premultiplied representation.
Screen output converts tiles to 8-bit RGBA or RGB.
The conversion un-premultiplies RGB, applies the configured EOTF, adds fixed dithering noise, and writes 8-bit output.

The default EOTF is `2.2`.
The displayed color is not a direct 8-bit copy of the internal tile.

At the time of this audit, the proxy demo let Canvas2D perform browser color and alpha operations.
The active WASM demo controls EOTF, premultiplied conversion, and fixed dithering in its display path.

## 12. Display and canvas view

`gui/tileddrawwidget.py` renders the document through a Cairo and GdkPixbuf path.
It performs these steps:

1. Convert model coordinates to display coordinates.
2. Apply translation, scale, rotation, and mirror.
3. Select a mipmap level from the view scale.
4. Render only affected tiles when possible.
5. Cull tiles outside a sparse clip region.
6. Composite the layer tree into an 8-bit pixbuf surface.
7. Apply the display filter to 8-bit output.
8. Use Cairo to display the pixbuf.
9. Use nearest filtering above the pixelize threshold `2.8`.
10. Defer high-quality mipmap rendering during scroll and restore it after a short timeout.

The renderer has a tile cache for cacheable render specifications.
It queues partial redraws and merges overlapping dirty rectangles.

Transparent views use a 16-pixel checkerboard.
The checker colors are approximately `(0.45, 0.45, 0.45)` and `(0.50, 0.50, 0.50)`.

The current demo uses one fixed Canvas2D bitmap with a CSS background.
It has no document transform, mipmap selection, dirty tile redraw, display filter, cache, pixelize threshold, or alpha checkerboard.

## 13. Undo, selection, and persistence

MyPaint snapshots tile dictionaries with copy-on-write behavior.
A finished stroke stores brush JSON, brush state, and the nine-value event stream.
A strokemapped layer stores stroke shapes and brush JSON for later stroke selection.
OpenRaster saves layer PNG data and a compressed strokemap.

The current demo has no undo, stroke replay, stroke selection, layer persistence, or pixel export path.

## 14. Current gap priority

### P0

1. Replace the proxy-only Canvas2D path with the NG tiled surface.
2. Give the tiled surface a bounded tile store and a dirty-tile export path.
3. Make `get_color` sample the real tile store.
4. Keep the 16-bit premultiplied pixel path through dab processing.

### P1

1. Add the Python input queue model.
2. Add equal-time event handling and pressure and tilt interpolation.
3. Pass view zoom, view rotation, barrel rotation, and the linear-color flag.
4. Use brush RGB output or update brush HSV settings from the color control.
5. Add exact brush-level lock-alpha, colorize, posterize, paint, smudge, and partial eraser behavior.

### P2

1. Add a bounded layer tree and layer opacity.
2. Add the layer combine modes.
3. Add mipmaps, dirty-tile redraw, display filtering, and checkerboard alpha display.
4. Add snapshots, undo, stroke replay, and export.

## 15. Recommended implementation shape

Do not add more Canvas2D dab approximations.
They cannot reproduce smudge, pigment, partial erasing, exact mask processing, or layer modes.

Compile the NG tiled surface and operation queue into the wasm module.
Give it a fixed-capacity 64 x 64 tile store in wasm memory.
Expose only these browser operations:

- begin stroke
- submit input sample
- finish atomic update
- read dirty tile records
- read color sample
- clear surface

The browser can upload dirty 8-bit tile rectangles to Canvas2D for display.
The conversion must follow the MyPaint EOTF and premultiplied-alpha rules.

Use MyPaint Python output as the acceptance reference.
For fixed input events and a fixed `.myb` file, compare dirty tile pixels and final PNG output against a Python reference run.

## Sources

- `gui/freehand.py`
- `gui/mode.py`
- `lib/command.py`
- `lib/stroke.py`
- `lib/brush.py`
- `gui/brushmanager.py`
- `gui/brushmodifier.py`
- `lib/tiledsurface.py`
- `lib/layer/data.py`
- `lib/layer/tree.py`
- `lib/layer/rendering.py`
- `gui/tileddrawwidget.py`
- `lib/pixbufsurface.py`
- `lib/pixops.cpp`
- `lib/blending.hpp`
- `lib/modes.py`
- `lib/eotf.py`
- `libmypaint/mypaint-tiled-surface.c`
- `libmypaint/brushmodes.c`
