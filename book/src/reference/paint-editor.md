# Paint editor surface

The character-editor paint demo uses the NG libmypaint brush engine in WebAssembly.
Brush dabs write 64 x 64 RGBA16 premultiplied tiles.

The browser reads dirty RGBA8 tiles for display.
The display path applies EOTF `2.2` and fixed dither.
The WASM build uses `-O3` and WebAssembly SIMD128.
Supported browsers draw display tiles on a Worker through `OffscreenCanvas`.

The demo supports configurable document dimensions up to 16K x 16K, sparse on-demand tile storage, eight paint layers, four nested groups, layer reparenting and order, group pass-through and isolation, all flat MyPaint layer modes, background color, brush erasing, real brush surface sampling, pressure and tilt interpolation, brush color restore, up to 40 tile history records, zoom, pan, quarter-turn rotation, mirror view, low-zoom mip display, fast view-only zoom changes, internal-pixel PNG export, and OpenRaster import and export. The brush engine runs in a Web Worker, so drawing and view changes never block the editor and no pointer samples are lost. The undo snapshot pool grows on demand. A HUD shows the input queue state, including queued samples and input sample rate.

The full API is in `docs/api/libmypaint-paint-surface.md` in the repository.
