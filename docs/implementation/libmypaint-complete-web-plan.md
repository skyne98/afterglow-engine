# Complete MyPaint-fidelity web paint plan

Status: active implementation
Date: 2026-08-02

Implementation update:

- the proxy Canvas2D dab path is removed from the WASM build
- the complete ISC libmypaint tiled surface is compiled
- the browser uses RGBA16 premultiplied tiles and fixed operation storage
- real surface color sampling and brush-level modes are active
- the display path applies EOTF and fixed dither in WASM
- bounded paint layers and groups, a clean-room implementation of the flat MyPaint blend modes, background color, configurable document size, input storage, bounded history, redo, and PNG export are active
- final group file-order parity, generational COW history, and long file-fixture validation remain open
- fixed pressure and tilt interpolation, `restore_color`, page-thread zoom, rotation, and mirror controls are now active
- signed-coordinate sparse tile lookup, bounded capacity errors, and up to 40 changed-tile history records are now active
- fixed eight-layer and four-group trees now support nesting, reparenting, order, visibility, opacity, modes, pass-through, and isolated rendering
- low-zoom box-filtered mip display tiles, high-zoom pixelization, pan, and frame outline controls are now active
- internal-pixel PNG export and bounded OpenRaster import/export are now active
- pointer inverse mapping now removes CSS translation before camera inverse mapping
- the WASM build uses `-O3` with WebAssembly SIMD128
- 16K x 16K documents use uniform background storage, a reduced display backing store, a 256-tile history profile, and fast reused-source mip generation
- view-only zoom changes skip tile redraw when the selected mip level stays the same
- display tile drawing uses an OffscreenCanvas Worker when the browser supports it
- display output uses an EOTF and dither lookup table
- the brush engine runs entirely in a Web Worker that also owns the display OffscreenCanvas and the motion queue, so the brush engine never blocks the render thread
- the worker fills the uniform background once and draws only mip tiles whose source regions contain paint
- the surface merges each atomic operation's dirty rectangles into one bounding box
- mip display skips unpainted source regions with a direct background fill and uses an exact box filter that keeps the background for painted regions
- final mip roots, generational COW roots, and full file-fidelity validation remain open

## 0. Requested execution scope

Implement every remaining parity item except long soak tests.
Keep the existing decisions: clean-room layer code, page-thread WASM, configurable MyPaint-default document state, 8 paint layers, 4 groups, and bounded fixed-capacity runtime storage.
Do not claim sealed-runtime stability without the excluded long-soak evidence.

### Execution phases

1. **Input and brush state**
   - Port pressure and tilt interpolation.
   - Add mouse, pen, release, and stroke-time fixtures.
   - Add MyPaint brush selection, `restore_color`, settings, and EOTF rules.

2. **Sparse document storage**
   - Replace the full rectangle with signed sparse tile coordinates.
   - Add fixed tile maps, tile handles, mip roots, and bounded failure results.
   - Preserve 64 x 64 RGBA16 premultiplied base tiles.

3. **Layer tree and compositor**
   - Add fixed paint and group records.
   - Add reorder, reparent, pass-through, isolated groups, group opacity, and background image nodes.
   - Keep the current 15-bit flat mode implementation as the leaf compositor.

4. **View and display**
   - Add model and display matrices for zoom, pan, rotation, and mirror.
   - Add mip selection, pixelize threshold, checkerboard transparency, frame display, and sparse redraw.
   - Keep all view operations outside document pixels.

5. **History and replay**
   - Replace the one-record full-copy history with 40 bounded copy-on-write roots.
   - Record brush hashes, settings, input events, layer commands, and document commands.
   - Add exact undo, redo, restart, and replay validation.

6. **File formats and export**
   - Add internal-pixel PNG export.
   - Add bounded OpenRaster import and export with layer, group, mode, opacity, and image state.
   - Reject unsupported data with visible, deterministic errors.

7. **Integration gates**
   - Split the TypeScript paint modules.
   - Add browser fixtures for every brush mode, layer mode, group mode, view mode, history command, and file command.
   - Run unit, native, WASM, browser, and 10-second performance checks.
   - Exclude 10-minute and longer soak runs from this scope.

The phase order is mandatory because input depends on document coordinates, groups depend on sparse layer roots, view output depends on mip data, and file/history state depends on stable document commands.

Related research:

- [`../research/libmypaint-python-gui-audit.md`](../research/libmypaint-python-gui-audit.md)
- [`../research/libmypaint-wasm/README.md`](../research/libmypaint-wasm/README.md)
- [`character-editor-prototype.md`](character-editor-prototype.md)

## 1. Goal

Replace the proxy Canvas2D dab path with a complete bounded web paint surface.
The result must support the MyPaint brush engine, real tile pixels, canvas sampling, brush modes, layers, display transforms, background images, undo, replay, and export.

The target is behavioral fidelity to the audited MyPaint Python GUI.
The target is not a copy of the MyPaint GTK user interface.

The feature set is one system with one pixel owner.
Do not add a second Canvas2D paint path for brushes that the new surface cannot process.

## 2. What "all at once" means

Implement the feature as one coordinated vertical change.
Use dependency order inside the change because the input, surface, compositor, and display parts depend on each other.

Do not land these as unrelated approximations:

- a Canvas2D dab renderer
- a separate smudge renderer
- a separate layer blend implementation
- a separate high-zoom pixel path

The WASM surface owns all document pixels.
The browser only displays exported tiles and sends input and document commands.

The current proxy callback must be removed after the new surface passes the single-layer gate.
Keep no runtime fallback that changes brush behavior.

## 3. Reference behavior

Use these source files as behavior references:

| Behavior | Reference |
|---|---|
| raw input, pressure cleanup, interpolation | MyPaint `gui/freehand.py` |
| abrupt stroke start | MyPaint `gui/mode.py` and `lib/command.py` |
| stroke recording | MyPaint `lib/stroke.py` |
| brush selection and overrides | MyPaint `gui/brushmanager.py` and `gui/brushmodifier.py` |
| brush color and linear flag | MyPaint `lib/brush.py`, `lib/eotf.py`, `lib/helpers.py` |
| tiled pixels and mipmaps | MyPaint `lib/tiledsurface.py` |
| layer tree and render operations | MyPaint `lib/layer/tree.py`, `lib/layer/rendering.py` |
| display transforms and redraw | MyPaint `gui/tileddrawwidget.py` |
| 16-bit conversion | MyPaint `lib/pixbufsurface.py`, `lib/pixops.cpp` |
| brush mask and brush modes | vendored libmypaint `mypaint-tiled-surface.c`, `brushmodes.c` |
| layer blend mode names | MyPaint `lib/modes.py` |

The vendored libmypaint tree is at commit `d5a88fbe6649d5ec776bc42ec8c1f4bb29d7fd7f`.
The audit uses MyPaint source commit `35aa9d33cd3deba6cafea6d8fc901b5a1d161ceb`.

## 4. Decisions that must be made first

These decisions affect memory, file format, and legal scope.
Do not turn the recommended values into permanent product policy without approval.

### 4.1 Layer blend code license

The vendored libmypaint code has an ISC license.
MyPaint Python files such as `lib/pixops.cpp`, `lib/blending.hpp`, and `lib/compositing.hpp` have GPL terms.

Recommended path:

- use the ISC libmypaint C code already vendored
- write new layer and display code from the W3C compositing rules and published color formulas
- use MyPaint behavior as a test reference
- do not copy GPL MyPaint implementation code into the Afterglow source tree

Alternative path:

- approve GPL source use for this prototype and record the distribution limit

This legal choice blocks implementation of the layer compositor.

### 4.2 Paint host

Recommended default: run the paint WASM module on the page thread.

Reason:

- brush calls are synchronous
- `get_color` reads the active surface during a brush call
- dirty tile publication is immediate
- the prototype does not yet need a worker transport

Alternative: move the complete paint document to a Worker.
That path needs a bounded message protocol, input latency tests, and a display tile queue.
It must not use a second payload transport in the engine.

### 4.3 Document size and capacities

MyPaint does not create a fixed canvas by default.
Its new document starts with frame `[0, 0, 0, 0]`, frame display disabled, and one blank paint layer.
The visible viewport is not the document boundary.

The web document must therefore expose configurable frame state:

```text
frame_enabled: boolean
frame_x: signed integer
frame_y: signed integer
frame_width: positive integer
frame_height: positive integer
```

Recommended default:

- `frame_enabled = false`
- `frame = [0, 0, 0, 0]`
- one blank paint layer
- sparse signed tile coordinates
- a fixed tile-pool limit instead of a fixed document rectangle

When the frame is disabled, the user can paint at any signed tile coordinate until a declared tile capacity is full.
When the frame is enabled, the frame limits view and export, but it does not delete pixels outside the frame.

Recommended development capacities:

| Capacity | Default |
|---|---:|
| tile size | 64 x 64 pixels |
| paint layers | 8 |
| groups | 4 |
| base tile pool | document tile count (on demand) |
| mip tile pool | 1024 tiles |
| history records | 40 |
| operation records per atomic update | 8192 |
| input records | 4096 |
| dirty output tile records | 2048 |
| symmetry lines | 64 |
| render cache entries | 16384 |

One RGBA16 tile uses `64 * 64 * 4 * 2 = 32768` bytes.
A tile is allocated on demand the first time it is touched, so memory scales with use and the WASM growth ceiling (set to the 4 GiB wasm32 maximum) is the only bound.

The original MyPaint undo default is 40 records and its render-cache default is 16384 entries.
Keep those defaults unless a measured web memory gate selects a profile override.

The document must report capacity values in the UI.
When a capacity is full, the document must stop the current transaction and report an error.
It must not silently delete a tile, event, layer, or history record.

### 4.4 Initial background

Recommended default: transparent document with a checkerboard display.
Add an optional opaque background layer or background image.

Alternative: create an opaque background layer at document creation.
This matches a common MyPaint workflow but changes alpha export behavior.

### 4.5 File format

Recommended default:

- native project file with versioned JSON metadata and compressed tile data
- OpenRaster import and export for interoperability
- PNG export for the merged image

Alternative: OpenRaster only.
OpenRaster does not cover all Afterglow brush event and capacity metadata without a private extension.

### 4.6 Color sampling randomness

The MyPaint tiled surface uses random sampling for large `get_color` regions.
Recommended default: keep the upstream sampling rule and add a document seed for reproducible browser tests.

Alternative: use a fixed deterministic sample pattern.
This improves repeatability but changes large smudge behavior from the reference.

## 5. Target architecture

```text
PointerEvent / pen input
        |
        v
Fixed input queue and pressure/tilt cleanup
        |
        v
Brush state and MyPaint NG Brush
        |
        v
Active Layer -> Fixed MyPaint Tiled Surface
        |
        +--> fixed 15-bit premultiplied tile store
        +--> fixed operation queue
        +--> real get_color sampling
        +--> dirty tile and dirty rectangle records
        |
        v
Layer tree compositor
        |
        +--> layer mode and opacity
        +--> group push/pop isolation
        +--> per-layer mipmaps
        |
        v
Display tile conversion
        |
        +--> un-premultiply
        +--> EOTF conversion
        +--> fixed dithering
        +--> RGBA8 tile
        |
        v
Browser display canvas and viewport transform
```

The C/WASM side owns:

- brush state
- active paint surface
- layer pixel tiles
- tile handles and tile reference counts
- operation records
- mipmap records
- layer composition scratch tiles
- dirty records
- history tile roots
- color conversion scratch buffers

The TypeScript side owns:

- DOM controls
- browser pointer event capture
- fixed input arrays
- view matrix state
- brush catalog metadata
- layer tree controls
- project file and OpenRaster slow paths
- display canvas objects

The browser must not read or write internal pixels through Canvas2D drawing operations.
It may read an exported RGBA8 tile from WASM memory and place it in the display canvas.

## 6. WASM source set

Extend `paint/build-wasm.sh` to compile the complete ISC libmypaint surface set:

```text
mypaint.c
mypaint-brush.c
mypaint-mapping.c
mypaint-brush-settings.c
helpers.c
rng-double.c
brushmodes.c
mypaint-rectangle.c
mypaint-matrix.c
mypaint-symmetry.c
mypaint-surface.c
mypaint-tiled-surface.c
```

Add the fixed Afterglow surface files:

```text
paint/surface/afterglow-surface.c
paint/surface/afterglow-surface.h
paint/surface/afterglow-tile-store.c
paint/surface/afterglow-tile-store.h
paint/surface/afterglow-operation-queue.c
paint/surface/afterglow-operation-queue.h
paint/surface/afterglow-layer.c
paint/surface/afterglow-layer.h
paint/surface/afterglow-compositor.c
paint/surface/afterglow-compositor.h
paint/surface/afterglow-pixops.c
paint/surface/afterglow-pixops.h
paint/main.c
```

Do not compile the stock `fifo.c`, `operationqueue.c`, or `tilemap.c` for the sealed surface.
The stock queue allocates operation nodes and grows a tile map during painting.
That violates the fixed-capacity requirement.

Keep the stock `mypaint-fixed-tiled-surface.c` as a native test reference.
Do not use it as the browser surface because it has no dirty-tile export, no tile ownership policy, and no bounded operation queue.

## 7. Fixed tile store

### 7.1 Tile format

Use the libmypaint tile format without conversion during painting:

```text
channel order: R, G, B, A
channel type: unsigned 16-bit
full value: 32768
alpha form: premultiplied
empty pixel: 0, 0, 0, 0
```

Keep all internal values in the range `[0, 32768]`.
Clamp RGB to alpha after every blend operation.

### 7.2 Tile map

The document uses signed sparse tile coordinates because the default frame is disabled.
Use a fixed open-addressed table for logical tile entries.
The table key contains the layer ID, mip level, signed tile X, and signed tile Y.

Use a power-of-two table with a declared maximum load factor.
Use linear probing with a fixed probe limit.
Return `PAINT_CAPACITY` when the table cannot admit a new key.
Do not grow the table during a stroke.

Each logical tile entry stores:

```text
u16 layer_id
u16 level
i32 tile_x
i32 tile_y
u16 handle
u16 generation
u8 present
u8 dirty_base
u8 dirty_mipmap
u8 reserved
```

Handle zero means an empty tile.
The tile pool stores the RGBA16 data in fixed slabs.

A tile is allocated on the first non-zero write.
A tile is released only when no layer root, history root, or mipmap entry refers to it.

If the frame is enabled, reject writes outside the frame only when the product chooses a clipped-paint mode.
The recommended mode keeps out-of-frame pixels and clips only display and export.

### 7.3 Copy-on-write history

Use tile handles instead of full pixel copies for history.
Each history root stores the logical handle map for all paint layers.

At stroke start:

1. capture the current document metadata root
2. increment tile references for the root
3. clear the redo stack
4. start a transaction record

Before a write:

1. if the tile has one reference, write it in place
2. if the tile has more than one reference, allocate a new tile
3. copy the old tile into the new tile
4. replace the current layer handle
5. release the current root reference

At stroke end, store the new root in the undo ring.
Undo swaps the current root with the previous root.
Redo swaps it with the next root.

If the tile pool cannot allocate a copy, abort the transaction and restore the captured root.

### 7.4 Tile requests

Implement `MyPaintTileRequestStartFunction` and `MyPaintTileRequestEndFunction` in `afterglow-surface.c`.

Rules:

- read-only requests return the current tile or a shared zero tile
- write requests allocate a tile only when the operation needs a write
- out-of-range coordinates return the zero tile and discard writes
- every request has a matching end call
- no callback creates a JavaScript object
- no callback allocates after document warm-up

The shared zero tile must never receive a write.
Use a separate scratch tile for out-of-range write requests.

## 8. Fixed operation queue

The upstream `mypaint-tiled-surface.c` is the correct mask and brush-mode reference.
Its queue must use fixed storage for the browser build.

Implement these changes:

1. Preallocate `8192` `OperationDataDrawDab` records.
2. Preallocate a link index for every operation record.
3. Preallocate a per-tile FIFO head and tail.
4. Preallocate a dirty tile list.
5. Preallocate a dirty bitset for the document tile range.
6. Replace `malloc(sizeof(OperationDataDrawDab))` with pool acquire.
7. Replace `free(op)` with pool release.
8. Return `PAINT_CAPACITY` when the operation pool is full.
9. Return `PAINT_CAPACITY` when a dirty tile list is full.
10. Keep operation order for each tile.

The operation record must contain the upstream values:

```text
x, y, radius
color_r, color_g, color_b, color_a
opaque, hardness, softness
aspect_ratio, angle
normal, lock_alpha, colorize
posterize, posterize_num, paint
```

Do not process a tile before all earlier operations for that tile are queued.
This preserves stroke order across tile boundaries.

### Atomic update

Expose these operations:

```text
paint_begin_atomic()
paint_flush(max_tiles, max_operations)
paint_end_atomic()
```

`paint_flush` may stop at a tile or operation boundary.
It must preserve queue order and return the remaining counts.

`get_color` must synchronously process the affected active-layer tiles before sampling them.
This rule matches the upstream tiled surface.

## 9. Brush and active surface

Rewrite `paint/main.c` around a document object.
Keep the existing brush JSON parser and the successful abrupt-start sequence.

Recommended public exports:

```text
paint_document_new(width, height, capacities_ptr)
paint_document_destroy()
paint_set_active_layer(layer_id)
paint_set_eotf(value)

paint_load_brush(json_ptr)
paint_new_brush()
paint_set_brush_base_value(name_ptr, value)
paint_get_brush_base_value(name_ptr)
paint_set_brush_mapping_n(name_ptr, input_ptr, count)
paint_set_brush_mapping_point(name_ptr, input_ptr, index, x, y)
paint_set_brush_color_rgb(r, g, b)
paint_get_brush_color_rgb(out_ptr)

paint_begin_stroke(x, y, xtilt, ytilt, viewzoom, viewrotation, barrel_rotation)
paint_stroke_to(x, y, pressure, xtilt, ytilt, dtime,
                viewzoom, viewrotation, barrel_rotation, linear)
paint_end_stroke()

paint_begin_atomic()
paint_flush(max_tiles, max_operations)
paint_end_atomic()

paint_set_symmetry(active, center_x, center_y, angle, type, lines)
paint_pick_color(x, y, radius, paint, out_ptr)
paint_clear_layer(layer_id)

paint_dirty_tile_count()
paint_dirty_tile_info(index, out_ptr)
paint_render_tile(layer_id, mip_level, tile_x, tile_y, output_ptr)
paint_render_display_tile(mip_level, tile_x, tile_y, output_ptr)
paint_release_dirty_tiles()

paint_history_undo()
paint_history_redo()
paint_history_can_undo()
paint_history_can_redo()
paint_error_code()
paint_error_text(out_ptr, capacity)
```

The exact pointer ABI must be documented in a generated header and TypeScript wrapper.
Do not use `ccall` string lookup in pointer and frame hot paths.
Resolve function pointers once during initialization.

### Brush color

Port the MyPaint color path without using the HTML color as a draw override.

1. Convert the HTML sRGB color to normalized RGB.
2. Apply the selected EOTF to RGB.
3. Convert RGB to HSV.
4. Set `color_h`, `color_s`, and `color_v` in the brush.
5. Let the brush engine produce the dab RGB values.

The reference transform is `rgb_to_hsv(r ** eotf, g ** eotf, b ** eotf)`.
Use the same transform for `restore_color` and brush selection.

`paint_get_brush_color_rgb` must read the brush settings for the color picker UI.

### Brush selection state

Extend the imported manifest with:

- source package version
- source relative path
- group
- order value
- parent brush name
- preview path
- brush file hash

Implement the selected-brush state from `BrushModifier`:

- keep an unmodified copy of the selected brush state
- preserve the active color according to `restore_color`
- identify dedicated erasers from `eraser`
- clear lock-alpha for dedicated erasers
- preserve the previous lock-alpha state for normal brushes
- update the UI mode state from `lock_alpha`, `colorize`, and `eraser`

The brush catalog can remain a browser UI feature.
It must not change surface behavior.

## 10. Input pipeline

Create `src/paint/paint-input.ts`.
It must use preallocated typed arrays.
Do not create an event object for every pointer event.

### Raw event record

Use one fixed record per event:

```text
u32 time_ms
f32 x_display
f32 y_display
f32 pressure
f32 xtilt
f32 ytilt
f32 viewzoom
f32 viewrotation
f32 barrel_rotation
u8 pressure_valid
u8 tilt_valid
u8 view_valid
u8 pointer_type
```

The browser event handler only:

1. reads primitive event fields
2. transforms display coordinates to model coordinates
3. clamps known finite values
4. writes one queue record
5. updates a queue counter

### Queue rules

Match `gui/freehand.py`:

- start only on primary button contact
- use fake mouse pressure `0.5` by default
- use fake mouse barrel rotation `0.5` by default
- send a zero-pressure release event
- correct backwards timestamps to the previous timestamp
- distribute same-time events across the next positive interval
- keep the device last-good pressure and tilt values during contact
- reject NaN and infinity
- clamp pressure to `[0, 1]`
- clamp tilt to `[-1, 1]`
- apply mirror to X tilt and barrel rotation
- apply barrel rotation offset and optional tilt correction

Queue overflow policy:

- keep the first event and the newest event for the active segment
- discard only intermediate motion records
- increment an overflow counter
- never discard a press or release record

### Pressure and tilt interpolation

Port the four-point interpolation state:

```text
pt0_prev, pt0, pt1, pt1_next
null-axis records before the next defined record
```

For a missing-axis event between two defined events, use the four-point cubic function:

```text
spline_4p(t, pt0_prev, pt0, pt1, pt1_next)
```

Use `pt0` and `pt1` when no outer point exists.
Drop leading events without a known axis value.

For a zero-to-positive pressure transition:

- clear the old interpolation history
- keep the transition event
- do not interpolate from a released position

For a positive-to-zero transition:

- duplicate the zero-pressure control point for the tail
- emit the tail events
- clear interpolation history

Add the current MyPaint `_TEST_DATA` as a TypeScript and Python-compatible fixture.

### Stroke timing

The first accepted event after a begin must not use the old event time.
The first real event follows the existing abrupt-start prime.

Use:

```text
dtime = (event_time - last_handled_time) / 1000
```

Do not use `performance.now()` as the only stroke time source.
The event timestamp preserves device timing and makes replay deterministic.

## 11. Brush surface behavior

Compile the ISC `mypaint-tiled-surface.c` and `brushmodes.c` behavior into the new surface.
Keep these rules:

- 64 x 64 tile masks
- 15-bit mask values
- premultiplied RGBA16 pixels
- radius fringe of one pixel
- radius below three pixel anti-alias path
- aspect ratio lower bound of one
- angle in degrees
- linear segment hardness and softness
- run-length encoded mask
- normal source-over
- target-alpha eraser and smudge path
- lock-alpha
- colorize
- posterize
- additive and spectral paint
- real surface color sampling

The current Canvas2D radial gradient must not remain in the output path.

## 12. Surface color sampling

Use the existing tiled-surface `get_color` algorithm after its tile callbacks use the fixed tile store.

Required behavior:

1. flush active-layer operations for affected tiles
2. create the fixed dab mask
3. sample every pixel for radius up to two
4. use interval `radius * 7` for larger regions
5. add the random sample rate `1 / (7 * radius)`
6. accumulate alpha-weighted color
7. use the paint factor to select RGB or spectral accumulation
8. un-premultiply the returned RGB
9. return transparent black for an empty region

Do not return a fixed gray value.

Test cases:

- pick an empty surface
- pick one opaque primary color
- pick two overlapping colors
- smudge across a color edge
- watercolor over transparent and opaque pixels
- large-radius sampling across tile boundaries
- sampling while queued operations exist

## 13. Layer model

Create a bounded layer tree.
Use fixed records with these fields:

```text
u16 layer_id
u16 parent_id
u16 first_child
u16 next_sibling
u16 first_tile_map
u8 kind
u8 visible
u8 isolated
u8 pass_through
f32 opacity
u16 mode
u16 revision
```

Layer kinds:

- background
- paint
- group

Recommended limits:

- 8 paint layers
- 4 groups
- one background layer
- one active paint layer
- no cycles

Create one blank paint layer at document creation.
Use `CombineSpectralWGM` as the default layer mode, as in MyPaint 2.x.
Keep the background visible by default when a valid stock background is available.

Each paint layer has its own fixed tile handle map.
Only the active paint layer receives brush operations.

Layer commands:

```text
layer_create(kind, parent_id)
layer_delete(layer_id)
layer_reparent(layer_id, parent_id, index)
layer_set_visible(layer_id, visible)
layer_set_opacity(layer_id, opacity)
layer_set_mode(layer_id, mode)
layer_set_isolated(layer_id, isolated)
layer_set_active(layer_id)
```

Reject deletion of the last paint layer.
Reject a parent that would create a cycle.
Invalidate the composite cache after every tree or layer property change.

## 14. Layer compositor

Implement the compositor as a separate C module.
Use premultiplied RGBA16 input and output.

### Render program

Build a flat render program from the tree:

```text
BLIT background
COMPOSITE paint layer
PUSH isolated group
COMPOSITE child
POP group with mode and opacity
```

The output order must match `lib/layer/rendering.py`.

A pass-through group writes its child output directly into the parent backdrop.
An isolated group starts with transparent pixels and composites one result into its parent at `POP`.

### Standard blend modes

Implement these modes:

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

Use W3C Compositing and Blending Level 1 as the clean implementation reference.
Use the document EOTF and premultiplied-alpha rules consistently.

For separable modes, use these straight-color functions:

```text
normal       Cs
multiply     Cb * Cs
screen       1 - (1 - Cb) * (1 - Cs)
overlay      2 * Cb * Cs when Cb <= 0.5, otherwise 1 - 2 * (1 - Cb) * (1 - Cs)
darken       min(Cb, Cs)
lighten      max(Cb, Cs)
difference   abs(Cb - Cs)
exclusion    Cb + Cs - 2 * Cb * Cs
```

Use the published definitions for dodge, burn, hard light, and soft light.

For hue, saturation, color, and luminosity, use the W3C `Lum`, `SetLum`, `Sat`, `SetSat`, and `ClipColor` operations.
Use the same luma coefficients for all four nonseparable modes.

For Porter-Duff modes, operate on premultiplied source and backdrop alpha.
Clamp every result to the 15-bit range.

### Pigment layer mode

Use the WGM ten-channel conversion already present in the ISC libmypaint helpers.
Keep this code in an ISC-derived module or write a clean implementation from the published formula.
Test pigment separately because floating-point rounding can differ between native and WASM.

### Opacity

Apply layer opacity to source alpha before the blend mode.
Do not apply layer opacity twice at group pop.

## 15. Mipmaps and dirty state

Keep base paint tiles at mip level zero.
Create derived mip levels on demand.

For each base tile write:

1. mark the layer base tile dirty
2. mark parent mip tile `(tx / 2, ty / 2)` dirty
3. continue to the maximum mip level
4. invalidate composite tiles at each affected level

Generate a mip tile from four parent tiles.
Average premultiplied RGBA16 values.
Treat absent tiles as zero.

A mip tile must be generated only after all four required parent tiles are available.
Use the fixed tile pool and a fixed mip dirty queue.

The composite cache key is:

```text
(document_revision, layer_tree_revision, mip_level, tile_x, tile_y)
```

A cache hit returns the same RGBA16 result until one source revision changes.

## 16. Display conversion

Add `afterglow-pixops.c` for the screen conversion.
The source is an internal RGBA16 premultiplied tile.
The destination is a fixed RGBA8 tile.

For every pixel:

1. read premultiplied `r`, `g`, `b`, and `a`
2. un-premultiply RGB with rounded division when `a != 0`
3. use zero RGB when `a == 0`
4. add the fixed dither value
5. apply `pow(channel, 1 / eotf)`
6. convert RGB to `[0, 255]`
7. convert alpha to `[0, 255]` without EOTF
8. clamp to byte range

Use EOTF `2.2` by default.
Support EOTF `1.0` for linear display tests.

Use a fixed `64 x 64 x 4` dither table.
Keep the table in the WASM module and include its version in the golden test metadata.

Do not let Canvas2D perform the internal blend or color conversion.

## 17. View and display canvas

Create `src/paint/paint-view.ts`.
Keep document pixels in model coordinates.
Map them to the visible viewport with a matrix:

```text
model -> mirror -> rotate -> scale -> translate -> display
```

Required controls:

- pan
- zoom
- rotation
- horizontal mirror
- reset view
- pixelize mode
- alpha checkerboard toggle

Use the inverse matrix for pointer input.

### Display surfaces

Use two browser canvases:

1. document display canvas for RGBA8 tile placement
2. visible viewport canvas for transformed display

Create one reusable `ImageData` object for a 64 x 64 tile.
Do not create an `ImageData` object for every dirty tile.

Use `putImageData` to place tiles in document coordinates.
Use `drawImage` with the view transform for the viewport.

Set image smoothing off when `zoom > 2.8`.
Use the selected mip level for lower zoom values.

### Dirty redraw

After `paint_flush`:

1. read dirty tile records
2. request composited RGBA8 tiles
3. place them into the document display canvas
4. clear the dirty list
5. request one viewport repaint

Merge adjacent dirty rectangles for status and diagnostics.
Do not scan every document pixel on every frame.

### Alpha checkerboard

Draw the checkerboard behind the document display canvas.
Use 16-pixel cells.
Use colors `(0.45, 0.45, 0.45)` and `(0.50, 0.50, 0.50)` in display space.
Do not write checker pixels into the document tile store.

### Background image

Treat a chosen background image as an opaque background layer.
On import:

1. decode the image in a slow path
2. flatten its alpha onto the selected background color
3. repeat it until it fits the tile grid when the repeat result is within the size limit
4. otherwise scale it to a tile-grid size
5. convert it to premultiplied RGBA16 with the selected EOTF
6. store it as background tiles

This follows the audited MyPaint background behavior.

## 18. Undo, replay, and export

### Undo and redo

Use the COW tile roots from section 7.
Each history record also stores:

- brush asset hash
- brush override values
- active layer ID
- input event range
- document revision before and after
- dirty tile count

The event stream uses fixed typed arrays.
Do not store a JSON string for every pointer event.

### Stroke replay

Replay must:

1. restore the saved document root
2. load the saved brush asset hash
3. apply saved settings and overrides
4. apply the saved EOTF
5. submit the saved nine-value events
6. compare the result with the saved post-stroke root

If the brush asset hash is unavailable, reject replay.
Do not replay with a different brush silently.

### Native project file

Use a versioned container with:

```text
manifest.json
layers.json
brushes.json
history.json
strokes.bin
pixels/layer-<id>/tile-<x>-<y>.rgba16
```

Write only present base tiles.
Write tile handles as file-local records, not runtime handles.
Sort all file names and JSON keys.
Write to a temporary file and publish only a complete file.

### OpenRaster

Support these entries:

```text
mimetype
stack.xml
mergedimage.png
Thumbnails/thumbnail.png
data/layer-*.png
```

Map layer visibility, opacity, mode, and order to OpenRaster fields.
Store Afterglow brush events and capacities in a private `data/afterglow.json` entry.
Reject unsupported modes with a visible import warning.

### PNG export

Render the merged output at mip level zero.
Convert internal RGBA16 to RGBA8 with the selected EOTF.
Encode PNG in a slow path.
Do not read back browser Canvas2D pixels as the source export.

## 19. TypeScript module split

Replace the current large `src/paint-demo.ts` with thin demo composition and these modules:

```text
src/paint/paint-wasm.ts       WASM load and typed ABI
src/paint/paint-document.ts   document and layer commands
src/paint/paint-input.ts      fixed event queue and interpolation
src/paint/paint-brushes.ts    catalog, selection, overrides
src/paint/paint-view.ts       matrix, viewport, tile display
src/paint/paint-history.ts    undo, redo, replay state
src/paint/paint-io.ts         project, OpenRaster, PNG slow paths
src/paint/paint-ui.ts         controls and diagnostics
src/paint-demo.ts             page composition only
```

The hot path may use typed arrays and fixed indexes.
Do not use `Map`, `Set`, array transforms, object literals, or new typed-array views in the pointer and flush paths.
DOM creation for the brush catalog remains a bootstrap slow path.

## 20. Build changes

Update `paint/build-wasm.sh`:

1. add all ISC libmypaint surface sources
2. add the fixed Afterglow surface sources
3. remove proxy surface sources from the production module
4. export the new document and tile functions
5. keep `addFunction` only for test or legacy compatibility if needed
6. add a debug build with assertions and capacity checks
7. add a release build with the same ABI
8. record the source commit and compiler version in generated metadata

Use the full CPU count for json-c and native test builds.
Run:

```sh
cd prototype/character-editor
nix-shell -p emscripten --run 'bash paint/build-wasm.sh'
bunx tsc --noEmit
bun run build
```

Do not edit generated `src/wasm/brushlib.js` by hand.

## 21. Test plan

### 21.1 C unit tests

Add native tests for:

- tile allocation and release
- out-of-range tile requests
- premultiplied alpha invariants
- COW tile roots
- operation queue order
- operation and dirty-list capacity errors
- mask values at center, edge, and outside
- radius below three anti-alias behavior
- aspect and angle
- every brush-level mode
- surface color sampling
- mipmap generation across tile borders
- EOTF conversion
- fixed dithering
- every layer mode
- group isolation and pass-through
- Porter-Duff operators
- symmetry transforms

Run the tests against the same C files used by WASM.

### 21.2 Python reference tests

Create a reference runner from the audited MyPaint Python surface.
For each fixture, record:

- source commit
- libmypaint commit
- EOTF
- brush JSON hash
- event records
- initial tile data
- final RGBA16 tile bytes
- dirty rectangles

Fixtures:

1. one opaque normal dab
2. two overlapping dabs
3. small radius anti-alias
4. hard, soft, and partial hardness
5. rotated aspect dab
6. dedicated eraser
7. lock-alpha
8. colorize
9. posterize
10. watercolor and smudge
11. spectral paint
12. repeated strokes at a tile edge
13. each symmetry type

Compare the WASM raw RGBA16 tiles with the reference.
Use exact equality for integer paths produced by the same C implementation.
Use a declared per-channel tolerance for floating spectral paths.

### 21.3 Input tests

Port the `PressureAndTiltInterpolator` doctest data.
Assert:

- leading undefined values are dropped
- missing values are filled
- equal timestamps receive positive intervals
- zero-to-positive clears old history
- positive-to-zero emits a tail and clears history
- finite clamping matches the reference
- mouse fake pressure is `0.5`
- release cannot connect to the next click

### 21.4 TypeScript tests

Add tests for:

- model and display matrix inversion
- mirror and rotation
- dirty tile placement
- layer tree cycle rejection
- deterministic layer ordering
- brush color EOTF transform
- `restore_color`
- project serialization order
- OpenRaster mode mapping
- history undo and redo state
- replay hash rejection

### 21.5 Browser tests

Use the real page and real pointer input.
Test:

- all 196 brush buttons and previews
- one stroke, release, move, second stroke
- pen pressure and tilt fixture
- eraser and partial eraser
- smudge over two colors
- lock-alpha and colorize
- posterize and pigment brush
- layer creation, reorder, hide, opacity, and mode
- isolated and pass-through groups
- zoom, pan, rotation, and mirror
- pixelize threshold
- checkerboard transparency
- background image tiling
- undo, redo, reload, and PNG export

Do not use synthetic `PointerEvent` objects as the only input test.

## 22. Performance and failure gates

The paint document is an editor slow path, but it must remain usable at 60 Hz.
Use these initial gates:

- no JS allocation in pointer capture, interpolation, tile request, or flush paths after warm-up
- no unbounded queue or tile map
- no queue overflow during 10 seconds of 240 input events per second
- no visible frame below 55 FPS during a 10-second normal stroke on the reference laptop
- no visible frame below 55 FPS during a 10-second smudge stroke at radius 64
- one dirty tile conversion and upload must stay below 1 ms median
- capacity errors must leave the previous committed document root unchanged
- short paint and pan runs must show bounded queue depth and tile-pool usage
- browser heap samples must return to the warm-up range after undo and export
- long soak tests are excluded from this implementation scope

Record:

- brush events per second
- operations queued and processed
- dirty tiles
- tile pool high water
- mip pool high water
- history pool high water
- `get_color` sample count
- flush time
- conversion time
- display upload time
- queue overflows
- capacity failures
- replay mismatches

## 23. Implementation order and gates

### Gate A: source and legal lock

1. Record the ISC libmypaint files used.
2. Record the GPL files that cannot be copied.
3. Select clean-room layer mode implementation or obtain explicit license approval.
4. Select capacity profile and file format.

No code implementation starts before this gate.

### Gate B: exact single-layer surface

1. Add the full ISC tiled surface sources.
2. Add fixed tile storage.
3. Add fixed operation queue.
4. Remove the proxy callback.
5. Export raw RGBA16 tiles.
6. Pass native and WASM single-layer goldens.

### Gate C: brush and input parity

1. Add input queue.
2. Add pressure and tilt interpolation.
3. Add real color settings and EOTF.
4. Add brush selection overrides.
5. Pass all brush-level mode and input tests.

### Gate D: layer and compositor parity

1. Add fixed layer tree.
2. Add layer tile roots.
3. Add all modes.
4. Add group isolation and pass-through.
5. Add background image layer.
6. Pass layer image goldens.

### Gate E: display parity

1. Add mipmaps.
2. Add RGBA16 to RGBA8 conversion.
3. Add dithering and EOTF.
4. Add dirty tile upload.
5. Add zoom, rotation, mirror, pixelize threshold, and checkerboard.
6. Pass display fixtures.

### Gate F: history and file parity

1. Add COW undo roots.
2. Add event replay.
3. Add native project save/load.
4. Add OpenRaster import/export.
5. Add PNG export.
6. Pass restart, undo, redo, and replay tests.

### Gate G: complete browser slice

1. Split TypeScript modules.
2. Add layer and brush controls.
3. Add view controls.
4. Add diagnostics and capacity display.
5. Run the browser and short performance gates. Exclude long soak gates.
6. Update `docs/api/`, the mdBook, and the research record.

## 24. Files to add or change

### WASM and C

- `prototype/character-editor/paint/build-wasm.sh`
- `prototype/character-editor/paint/main.c`
- `prototype/character-editor/paint/surface/afterglow-surface.c`
- `prototype/character-editor/paint/surface/afterglow-surface.h`
- `prototype/character-editor/paint/surface/afterglow-tile-store.c`
- `prototype/character-editor/paint/surface/afterglow-tile-store.h`
- `prototype/character-editor/paint/surface/afterglow-operation-queue.c`
- `prototype/character-editor/paint/surface/afterglow-operation-queue.h`
- `prototype/character-editor/paint/surface/afterglow-layer.c`
- `prototype/character-editor/paint/surface/afterglow-layer.h`
- `prototype/character-editor/paint/surface/afterglow-compositor.c`
- `prototype/character-editor/paint/surface/afterglow-compositor.h`
- `prototype/character-editor/paint/surface/afterglow-pixops.c`
- `prototype/character-editor/paint/surface/afterglow-pixops.h`

### TypeScript

- `prototype/character-editor/src/paint/paint-wasm.ts`
- `prototype/character-editor/src/paint/paint-document.ts`
- `prototype/character-editor/src/paint/paint-input.ts`
- `prototype/character-editor/src/paint/paint-brushes.ts`
- `prototype/character-editor/src/paint/paint-view.ts`
- `prototype/character-editor/src/paint/paint-history.ts`
- `prototype/character-editor/src/paint/paint-io.ts`
- `prototype/character-editor/src/paint/paint-ui.ts`
- `prototype/character-editor/src/paint-demo.ts`
- `prototype/character-editor/paint/paint-demo.html`
- `prototype/character-editor/scripts/import-mypaint-brushes.ts`

### Tests and docs

- `prototype/character-editor/paint/tests/`
- `prototype/character-editor/src/paint/*.test.ts`
- `prototype/character-editor/scripts/generate-mypaint-goldens.py`
- `docs/api/libmypaint-paint-surface.md`
- `book/src/reference/paint-editor.md`
- `book/src/SUMMARY.md`
- `docs/research/libmypaint-python-gui-audit.md`

## 25. Completion definition

The work is complete only when all of these statements are true:

1. The browser never sends brush dabs to Canvas2D.
2. Every brush reads and writes the fixed RGBA16 tile surface.
3. `get_color` reads real active-layer pixels.
4. Brush-level modes match the libmypaint surface.
5. Layer modes and group behavior pass image fixtures.
6. The default layer mode is the selected documented mode.
7. Mipmaps, EOTF, premultiplication, and dithering have tests.
8. View transforms do not change model pixels.
9. Background images stay outside the transparent document pixels.
10. Undo and redo restore exact tile roots.
11. Replay rejects missing brush hashes.
12. OpenRaster and PNG export use internal pixels.
13. Capacity overflow leaves the last committed root unchanged.
14. The 196 stock brushes load and paint.
15. Short browser performance checks pass with bounded memory and queue metrics.
16. The API note and mdBook chapter match the final ABI.
17. Long soak evidence remains a separate release gate.

## 26. Research sources

- [libmypaint source](https://github.com/mypaint/libmypaint)
- [libmypaint ISC license](https://github.com/mypaint/libmypaint/blob/master/COPYING)
- [MyPaint source](https://github.com/mypaint/mypaint)
- [MyPaint freehand input](https://github.com/mypaint/mypaint/blob/master/gui/freehand.py)
- [MyPaint tiled surface](https://github.com/mypaint/mypaint/blob/master/lib/tiledsurface.py)
- [MyPaint layer render operations](https://github.com/mypaint/mypaint/blob/master/lib/layer/rendering.py)
- [MyPaint layer modes](https://github.com/mypaint/mypaint/blob/master/lib/modes.py)
- [OpenRaster](https://www.openraster.org/)
- [W3C Compositing and Blending Level 1](https://www.w3.org/TR/compositing-1/)
