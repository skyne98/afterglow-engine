# Virtual Texturing

Afterglow's texture path uses a shared physical atlas instead of allocating one
GPU texture per asset. Textures are divided into 128x128 payload pages with a
four-texel border. The shader translates virtual UVs through a page table and
falls back to a resident coarser mip while detailed pages stream in.

This bounds texture memory and lets the runtime prioritize only visible pages.
The atlas format is selected for the GPU: BC7 on supported desktop adapters,
ASTC where available, and RGBA as the compatibility fallback.

Enable the path by installing a `VirtualTextureStore` on `AssetStore`; subsequent
`loadTexture()` calls delegate to it. After Three.js initializes the textures,
call `attachRenderer()` so updates become small GPU subregion writes. Each frame,
poll the store and submit the previous globally identified feedback results with
`processFeedback()`. `VirtualTextureFeedbackPass` provides the reduced-resolution
`RG32Uint` render target and asynchronous readback. Camera and frame-time data
enable prediction and adaptive LOD.

The `.big` asset format stores pages independently by texture name, mip, and
page coordinates, so the serving layer can range-load individual pages. The
offline pipeline decodes square power-of-two PNG/JPEG sources, filters their
mips, and emits 128x128 payloads with four-texel neighbor borders. Levels from
64x64 through 1x1 are packed into one permanently resident mip-tail slot rather
than wasting one physical slot per tiny level. Every complete slot is encoded
independently as UASTC Basis offline, then transcoded on demand to BC7, ASTC, or
RGBA by the runtime texture worker.

## Demo

The `vt-demo` CEF example exercises the WebGPU page-table shader, physical
atlas, borders, mip fallback, and incremental residency. Its deterministic
Perlin-style fBm source is 262,144x262,144 texels—256 GiB as ordinary RGBA8—but
only requested 128x128 pages are ever generated:

```sh
nix-shell shell.nix --run "cargo build --example vt-demo -p afterglow-cef"
nix-shell shell.nix --run "./target/debug/examples/vt-demo --ozone-platform=x11"
```

Run the automated real-GPU regression with `DISPLAY=:0 ./scripts/test-vt-gpu.sh`.
It validates feedback rendering/readback and compressed atlas subregion uploads
on the active adapter. On fox-laptop, stable rendering, panning, and continuous
streaming held 59.97 FPS with zero drops across each 600-frame measurement;
extreme per-frame teleport/cache-thrash tests dropped one frame per 600.

Use **WASD** to pan, the mouse wheel to zoom, **P** for a one-texel view,
**O** for the full boundary overview, and **R** to reset. **Show atlas** opens a
live corner view of the fixed physical cache. **Slow VT + mip debug** throttles
residency to one page every 20 frames and colors the selected mip levels so
streaming and fallback are visible. The demo currently simulates requests from
camera coverage on the CPU; GPU feedback rendering/readback is the remaining
integration step.
