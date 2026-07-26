# afterglow-basis-encoder

`afterglow-basis-encoder` is an **offline-only** wrapper around the official
Basis Universal C++ encoder. It is isolated from `afterglow-texture`, which
remains the pure-Rust runtime/wasm transcoder.

## API

```rust
pub fn encode_uastc_rgba(
    data: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String>
```

The input must be tightly packed RGBA8. Output is a single-image, single-level
UASTC `.basis` payload. VT page borders and mip levels are generated before this
function is called; the encoder must not generate additional local mipmaps.

The crate is used only by `afterglow-pipeline`. It must not become a dependency
of game runtime, worker, CEF, or wasm crates.

`afterglow-pipeline process` performs:

1. PNG/JPEG decode to RGBA8.
2. Full-image filtered mip generation.
3. One-mip-at-a-time 128x128 page extraction with four-texel neighbor borders,
   retaining at most 64 pages per parallel encoding batch.
4. Packing 64x64 through 1x1 levels into one bordered mip-tail slot.
5. Independent UASTC encoding of every 136x136 page/tail slot.
6. Immediate disk spooling into seekable `.big` mip blocks indexed by compact
   page-size directories and tagged once per virtual texture with
   `TextureEncoding::Basis`. The layout originated in v5 and remains unchanged
   in current writer v6.

The current generic image cook box-filters every role in RGBA byte space and
clamps global-edge border texels. Linear-light albedo filtering, normal-vector
renormalization, and seam-correct repeat/mirrored-repeat borders are known open
quality requirements.

At runtime, `afterglow-texture` transcodes each requested UASTC page to BC7,
ASTC, or RGBA according to adapter support. Public web uses its WASM worker;
CEF must use the generated native worker, whose `EngineAssets` composition is
currently incomplete.
