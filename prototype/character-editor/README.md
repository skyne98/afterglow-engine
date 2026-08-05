# Afterglow — Character Editor Prototype

A minimal **Three.js (WebGL) + Bun** character editor built on the MakeHuman /
MPFB2 character system, using **genital proxy topology + offline morph transfer**.

It loads a **single self-contained skinned mesh per sex** (the proxy body, which
carries the genitals) that also owns the **body-morph library as native glTF
morph targets**. No refit runs at runtime — moving a slider just sets a GPU
`morphTargetInfluence`.

Features:
- **Male body / Female body** swap (penis via the male proxy, vulva via the
  female proxy); the two now have distinct male/female silhouettes.
- **Ethnicity**: Caucasian / Asian / African (drives transferred race morphs).
- **423 male knobs** backed by 691 morph targets and **422 female knobs** backed
  by 689 morph targets. This includes all body, expression, and speech controls.
- **Face animation**: 52 ARKit expression units, 21 Microsoft visemes, and 14
  Meta visemes, with eyes, teeth, and tongue geometry.
- **Face preview library**: 17 expression compositions and all 35 speech shapes.
- **Hair selection**: None, CC0 `short04`, or CC0 `ponytail01`.
- **Live hair fitting**: a 733-vertex compact body driver first updates the
  PunkElvs scalp. Both styles then fit to that visible proxy surface with an
  8 mm minimum scalp clearance. Expressions and visemes do not change the hair
  rest shape.
- **Smooth shading** + **Wireframe** rendering toggles.
- **Clickable body zones**: hover shows a generated color paint; click filters
  the panel to controls for that zone. Left/right areas stay separate.
- Randomize / reset, bone focus / list, body-part toggle, load any glTF.

For larger, rounder breasts, click the breast zone and move **Macro → Cup
size** toward +1. Add **Macro → Breast firmness** toward +1 for a rounder shape.

## Face controls

1. Select **Focus face** for a close view.
2. Select an **Expression** preview, such as Happy, Angry, or Surprised.
3. Select a **Speech shape** to combine a viseme with that expression.
4. Select **Reset face** to clear expression and speech values.
5. Use the `expression-*`, `speech-meta`, and `speech-microsoft` slider groups
   for direct control of each morph.

The expression previews are useful compositions of ARKit units. They are not
an official ARKit emotion standard. Expression previews clear only expression
units. Speech previews clear only speech units, so one of each can be active.

## Quickstart

```sh
bun install
bunx vite          # dev server
```

Open the printed URL (here it runs on 5175). The committed
`public/character_male.glb` + `public/character_female.glb` are already baked,
so it works immediately.

## Regenerate (Blender + MPFB2, headless)

```sh
bun run gen:character
```

This runs `scripts/gen-proxy-transfer.py` for male and female. Each run:
1. builds an MPFB human + `game_engine` skeleton,
2. applies the **PunkElvs genital proxy** (CC-BY; alternate pelvis topology),
3. captures all applicable `target.json` targets plus 62 asymmetry targets,
4. captures age, muscle, weight, proportions, height, cup size, and firmness,
5. captures all 87 non-empty CC0 expression and speech targets,
6. replaces Caucasian to capture Asian and African bodies,
7. adds the eyes, teeth, and tongue,
8. creates smooth proxy-native morph targets and control sidecars,
9. extracts `short04` and `ponytail01` from the local CC0 system pack,
10. binds both styles and their rig weights to the PunkElvs scalp,
11. exports both styles plus an exact scalp cap and two-stage SurfaceWrap
    sidecar.

Requires (one-time): Blender 5.x on `PATH` and the MPFB extension installed at
`~/.config/blender/5.2/extensions/user_default/mpfb`. PunkElvs proxies and the
CC0 functional face packs live in `assets/character-rig/downloads/`.

Validate the generated GLBs and the TypeScript source:

```sh
bun run test
```

## Bake-core Rust prototype

`bake-core-rs/` tests the difficult source-to-bake algorithms separately from
the engine. It is not an engine crate, workspace member, worker, or public API.

It contains fixed-array SurfaceWrap fitting, sparse-target evaluation, macro
products, corrected skin transfer, and normal rebuilding. Its tests include
caller-owned-memory checks and MPFB parity for the CC0 `short04` hair asset.

Run it with:

```sh
cargo test --manifest-path prototype/character-editor/bake-core-rs/Cargo.toml
```

## Why proxy + offline transfer

MakeHuman genitals exist only on **proxy topology** (an alternate pelvis mesh).
A proxy is a tight body **cover** over the morphable base; it follows base
morphs only via a ("refit") operation. Doing that refit live per slider drag is
slow, so we do it **offline once** and bake each morph's proxy-vertex delta into
the proxy mesh itself. Result: the runtime editor is a trivial static glTF
morph mesh — genitals and body morphs together, no refit, no base mesh.

## Asset & license notes

- Body + morph targets: **CC0** (MakeHuman bundled assets).
- **PunkElvs male/female genital proxies: CC-BY 4.0** (keep attribution).
- MPFB add-on: GPLv3 (tool only, not shipped).
- Full ledger: `assets/character-rig/SOURCE.md`.
- License research: `docs/research/open-license-character-rig-in-game-editor.md`.
- Direct-manipulation UX research:
  `docs/research/direct-manipulation-character-creator-ux.md`.
- Runtime MakeHuman hair research:
  `docs/research/makehuman-hair-runtime-dynamic-character-system.md`.

## Status / next steps

- [x] Genitals on both sexes (penis / vulva) via proxy topology.
- [x] Body-morph library transferred onto proxy (no runtime refit).
- [x] All canonical and asymmetry body controls with paired −1…+1 knobs.
- [x] Sex, ethnicity, age, muscle, weight, proportions, and height controls.
- [x] Female cup-size and breast-firmness controls.
- [x] Morph-derived color zones with hover highlighting and click filtering.
- [x] Add all non-empty CC0 ARKit expression and Microsoft/Meta speech shapes.
- [x] Add eyes, teeth, and tongue geometry for face animation.
- [x] Add expression and complete viseme preview libraries.
- [x] Research The Sims 4 and BDO direct-manipulation editors.
- [x] Audit MakeHuman hair fitting, rigging, and runtime physics needs.
- [x] Prototype fixed-workspace SurfaceWrap and related bake algorithms.
- [x] Add live-fitted short and ponytail hair selection with a scalp cap.
- [ ] Prototype authored SpringChain hair.
- [ ] Select a patent-review path before direct body manipulation.
- [ ] Add an audio or phoneme timeline that drives Meta visemes.
- [ ] Wings + tails (CC-BY) via MPFB sub-rigs.
- [ ] Real materials/textures (currently a flat skin material).
- [ ] Persist character presets; pose presets via bone transforms.
