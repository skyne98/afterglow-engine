# Temporary native WebGPU feature patches

This project targets native GPUs only. It carries narrow upstream patches for
features already provided by its native wgpu backend but not yet represented
correctly through deno_webgpu's WebGPU-facing API.

## Core features and limits

Deno parses `GPURequestAdapterOptions.featureLevel`, but currently ignores it
and always requests wgpu's core-defaulting native adapter and core default
limits. This is documented directly in `adapter.rs`; compatibility mode is
tracked by [gfx-rs/wgpu#8124](https://github.com/gfx-rs/wgpu/issues/8124).

Until that is implemented, the returned adapter is a core adapter and must
advertise `core-features-and-limits`. The deno_webgpu patch:

- represents this standardized feature separately from wgpu's feature bitset;
- exposes it on the core-defaulting adapter;
- accepts it in `requiredFeatures` without passing a nonexistent bit to wgpu;
- exposes it on `GPUDevice.features` only when the application requested it.

This matters functionally: three.js correctly disables multisampling in WebGPU
compatibility mode. Omitting the feature made a core native device look like a
compatibility device and silently forced every renderer's sample count to zero.

## Subgroups

wgpu 29 already has native subgroup operations, built-ins, backend lowering,
and `naga::valid::Capabilities::SUBGROUP`, but does not yet expose that support
through the standard WebGPU `subgroups` feature surface:

- `wgpu-types` classifies `Features::SUBGROUP` as native-only;
- `deno_webgpu` filters native-only features from `GPUAdapter.features`;
- Naga recognizes `enable subgroups;` as intentionally unimplemented.

The patches expose an adapter's existing `Features::SUBGROUP` and map the WGSL
directive to Naga's existing capability. Unsupported adapters still omit the
feature, and Naga still rejects the directive unless the requested device has
the capability. Standards alignment is tracked by
[gfx-rs/wgpu#5555](https://github.com/gfx-rs/wgpu/issues/5555).

## Native surface presentation

The upstream canvas layer can configure and acquire a `ContextData::Surface`,
but did not expose a host presentation boundary and treated an acquired surface
texture like an ordinary destroyable headless texture. The local canvas patch:

- adds `GPUCanvasContext::present()` for an explicitly submitted native frame;
- calls wgpu-core `surface_present` and retires the current surface texture;
- discards, rather than destroys, an unpresented surface texture during resize
  or unconfigure;
- leaves headless `ContextData::Canvas` readback behavior unchanged.

`src/main.rs` creates the winit surface through the same wgpu-core `Global`
placed in deno_webgpu's `OpState`, so JavaScript rendering and presentation do
not use unrelated devices or a full-frame CPU round trip.

## Shared wgpu facade handles for Vello

Vello's GPU renderer consumes the safe `wgpu` facade while deno_webgpu exposes
wgpu-core IDs. The local wgpu patch adds explicitly borrowed constructors for
an embedding-owned `Global`, device, queue, and surface texture. Borrowed facade
handles suppress their normal core-ID drop calls; JavaScript remains the sole
owner. This lets Vello rasterize Blitz paint scenes and composite them on the
exact JavaScript device/queue without a second adapter, cross-device copy, CPU
pixel raster, or readback.

## Maintenance

Versions are recorded in `versions.env`. Recreate the complete source trees
from crates.io and reapply the maintained deltas with:

```bash
scripts/update_vendored_webgpu.sh
```

Patch files live in `vendor/patches/`, so incompatible upstream changes fail
before replacing the checked-in sources. No three.js example, addon, generated
shader, or module-loader source is modified.
