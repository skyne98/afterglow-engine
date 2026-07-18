# Dungeon resident POM height fields

The deployable payloads in `crates/afterglow-web/web/assets/dungeon-height/`
are lossless source-R16 data generated from the official 1K, 16-bit
displacement PNGs for the same ambientCG materials used by `dungeon.big`:

- `Rock064_Height.r16`
- `Ground103_Height.r16`
- `PavingStones150_Height.r16`

Source archives: `https://ambientcg.com/get?file=<name>_1K-PNG.zip`, member
`<name>_1K-PNG_Displacement.png`.

Regenerate each asset with:

```sh
cargo run -p afterglow-pipeline -- \
  height-r16 <name>_1K-PNG_Displacement.png <name>_Height.r16
```

The versioned file is `AGR16LE` + version byte `1`, little-endian `u32` width
and height, then exactly `width × height` little-endian normalized `u16`
samples. Runtime converts those exact levels into a single-channel
`RedFormat + FloatType` Three `DataTexture`, mapping to filterable WebGPU
`r32float`. Every u16 level remains distinct in float32 and browser image
decoding is never involved. Runtime fails closed without `float32-filterable`.

WebGPU exposes `r16unorm` as unfilterable, and Three r185 cannot produce the
required custom-WGSL non-filtering binding. `r32float` avoids a vendored Three
patch and avoids four manual bilinear reads for every POM height lookup.

ambientCG assets are CC0/public domain. The maps are intentionally resident: a
non-uniform POM march must not depend on asynchronous VT page residency. They
are physical-height inputs (white/exposed, black/recessed); AO is lighting
information and is not a valid substitute for geometric height.
