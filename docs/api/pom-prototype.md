# POM prototype API

`prototype/pom/` is a WebGPU-only visual/performance prototype, served by the
`afterglow-cef` `pom_bench` example. It is not a runtime engine subsystem.

## Controls

| Input | Effect |
|---|---|
| `1` / `2` / `3` | normal map / one-tap parallax / adaptive POM |
| `4` | adaptive POM with optional light-direction contact shadows |
| `S` | toggle contact shadow strength between `0` and `0.72` |
| Arrow left/right | change POM head-on layer count (4–64) |
| Arrow up/down | change height scale (0.005–0.2) |
| `V` | hard grazing-angle inspection view |
| `F` | freeze orbit rotation |
| `B` | start the 300-frame rAF benchmark |

## Automation hooks

The page deliberately exposes direct hooks because synthetic CDP keyboard
events are not reliable CEF input:

- `window.pomSetMode(index)` selects mode `0..3`.
- `window.pomSetShadowStrength(value)` clamps contact-shadow strength to
  `0..1`.
- `window.pomFreeze(boolean)` freezes/unfreezes orbit rotation.

## Shadow model

Mode 4 reuses the canonical POM hit UV, then runs a fixed two-sample,
explicit-LOD tangent-space ray toward the `DirectionalLight`. The height map
uses physical height: brick `1`, mortar approximately `0.16`. A higher terrain
sample above the rising light ray blocks it. A `0.035` bias prevents
self-shadow acne. `ContactShadowLightingModel` applies the visibility only to
Three's direct diffuse and direct specular terms at the configured strength;
ambient/indirect illumination and normal-map angle response are unchanged. It
is a bounded contact-shadow enhancement, not long-range soft shadowing.

`textureSampleLevel` is used in both POM and shadow loops because WGSL forbids
implicit-derivative texture sampling in non-uniform control flow.
