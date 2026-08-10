# Open-Source Web Character-Editor Building Blocks (exploration TODO)

Date: 2026-08-02
Status: **todo — explore**

Recorded decision to explore open-source web UI/engine building blocks for the
afterglow character editor. This is a backlog note, not a plan. Nothing is
integrated yet.

## Direction

The character editor needs three interactive layers, each best served by a
distinct open-source reference:

1. **3D texture painting** (Paint Tool SAI-like, on mesh) — real-time UV paint.
2. **3D sculpting** — brush deformation, dynamic topology, subdivision.
3. **Pen / tablet / touch input** — pressure + tilt in the browser.

Plus the BDO-style region-drag + explicit-controller interaction overlay (see
`docs/implementation/bdo-style-direct-character-editor-design.md`), which no
open-source editor implements and therefore must be built.

## Candidate references (all MIT)

| Need | Pick | Why |
|------|------|-----|
| Sculpt engine/UI | **SculptGL** (`stephomi/sculptgl`) | canonical algorithm + ZBrush-like UI; archived 2023 (MIT references) |
| Maintained sculpt base | **Microtome/sculpt_ng** | "Continuation of SculptGL", TypeScript, MIT, active 2024-2025 |
| Sculpt alt | marmelab/sculpt-3D | modern React+Three, MIT |
| **Painting engine (brush feel)** | **MyPaint / libmypaint** | best open-source brush engine (friction/dynamics); libmypaint is MIT; Krita also uses it |
| Painting tech ref (3D UV) | Aphene/texture-painter | real-time 3D UV paint (MIT) but user-rejected as vibe-coded |
| Paint tech ref (GPU rtarget) | manthrax/monkeypaint | GPU render-target painting core, MIT |
| Pen/touch | native Chromium PointerEvents | pressure + tilt in Chromium; no heavy lib |

## Notes / caveats

- **SculptGL** relation to Nomad: same author (Stéphane Ginier); Nomad is the
  production successor but closed-source/web-less. SculptGL is MIT + archived.
- Delivery layer (React, WebGL1, Three variant) is irrelevant to reference
  value — what matters is the **algorithm + approach + UI structure**.
- SculptGL distro is WebGL1 + glMatrix; algorithms port to WebGPU shader math
  in the Chromium shell.
- All key 3D references are MIT → commercial-clean with credit.

## Todo items

- [ ] Clone `stephomi/sculptgl` and `Microtome/sculpt_ng`; diff the brush
      engine, dynamic topology, subdivision, and UI structure to decide which
      to build on.
- [ ] Audit `libmypaint` brush-stroke algorithm (friction model + input
      mappings: pressure/velocity/direction/tilt) and the `.mybrush` brush
      definition format, as the painting-engine reference.
- [ ] Audit `Aphene/texture-painter` paint pipeline (UV raycast + texture
      writeback, layer stack, decals) for reuse vs rewrite.
- [ ] Audit `manthrax/monkeypaint` GPU render-target technique.
- [ ] Validate Chromium PointerEvents pressure/tilt path for pen + tablet in
      the native shell.
- [ ] Decide reuse vs rewrite against afterglow's KISS + allocation-hygiene
      rules (bounded pools, no per-frame allocation, O(1) hot paths).
- [ ] Scope a paint/sculpt layer integration into the character prototype.

Not promoted to engine code; remains a research/exploration item pending the
above audits.
