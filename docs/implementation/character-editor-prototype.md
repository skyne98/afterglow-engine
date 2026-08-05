# Character Editor Prototype — Implementation & Pipeline Notes

Status: working prototype (2026-08-02)
Location: `prototype/character-editor/`
Related: `assets/character-rig/` (assets + licenses),
`docs/research/open-license-character-rig-in-game-editor.md` (license research),
`docs/research/direct-manipulation-character-creator-ux.md` (editor UX),
`docs/research/makehuman-hair-runtime-dynamic-character-system.md` (runtime hair),
and `docs/implementation/runtime-character-bake-plan.md` (production bake path).

This note records the **full process**: the environment setup, each pipeline
iteration, the errors and the corrections, and the final implementation.

---

## 1. Goal

A simple Three.js (WebGL) + Bun prototype character editor built on the
MakeHuman / MPFB2 character system, with a hard requirement of an **in-game
character editor** (base + morphs + skeleton) plus **genitals**.

## 2. Final architecture (result of all iterations)

- **Two self-contained skinned glTF bodies per sex**: `character_male.glb` and
  `character_female.glb`.
- Each body is a **genital proxy mesh** (PunkElvs, CC-BY) that:
  - carries the genitals (penis on male, vulva on female) as base geometry,
  - owns the **body-morph library as native glTF morph targets**
    (604 male including 6 penis targets / 602 female) transferred onto the proxy topology,
  - owns 52 ARKit expression units, 21 Microsoft speech visemes, and 14 Meta
    speech visemes,
  - includes the base eyes, teeth, and tongue for facial animation,
  - is skinned to the `game_engine` skeleton.
- **No runtime refit**: the editor just blends `morphTargetInfluences` on the
  GPU. The expensive refit runs **once at bake time** (offline).

## 3. Environment setup

- Blender **5.2.0 LTS** (installed on PATH), Bun 1.3.13.
- MPFB2 v2.0.17 installed as a **Blender extension** (not a legacy addon):
  - user extensions repo: `~/.config/blender/5.2/extensions/user_default/mpfb`
  - enabled module name: `bl_ext.user_default.mpfb` (not `mpfb`).
  - MPFB requires Blender ≥ 4.2, has no runtime python deps.
- Editorial: the demo runs via `bunx vite` (port 5175; 5173/5174 were in use
  by other servers).

## 4. Iterations (process) and what each taught us

### 4.1 First generation: flat basemesh export (rejected)
- `create_human()` → `add_builtin_rig(game_engine)` → export glTF.
- **Bug:** exported the **raw** MPFB base which includes the
  **HelperGeometry/JointCubes** double shells → user sees a "skirt" / "multiple
  bodies in one" (measured ~14% exactly-coincident vertices, concentrated in
  groin/chest/feet).
- **Fix:** `ExportService.create_character_copy` +
  `bake_modifiers_remove_helpers(bake_masks=False, bake_subdiv=False,
  remove_helpers=True)` strips the helpers **without** baking away the morphs.
  Verified: base 21,833 → 14,517 verts, coincident 2,987 → ~1,300 (UV seams).

### 4.2 Morph-index bug (critical, user-visible)
- MPFB exports the 8 gender/ethnicity macro keys (`$md-*`) at indexes 0-7, then
  body morphs from index 8.
- My slider list filtered out `$md` keys, which shrank the array, but writes
  used the filtered index → **every morph was off by 8** (penis sliders moved
  the chest).
- **Fix:** keep the FULL sidecar (`character.morphs.json`) so list index == glTF
  morph-target index; only skip `$md` for display. Verified 1:1: `penis-length-
  incr` is glTF index 119 on both the slider title and the sidecar.

### 4.3 Gender as toggled shape keys (wrong — the "dress" + "penis on female")
- I assumed gender = flipping the 8 macro keys. This was **wrong**. MPFB
  composes gender as an **interpolated macro target stack**
  (`_interpolate_macro_components` + `calculate_target_stack_from_macro_info_dict`);
  it is a 0-1 value, and for pure sex the macro keys get per-baselayer weights.
- Setting all 4 ethnic-female layers to 1 simultaneously **doubled the surface**
  ("dress" artifact) and never reshaped the pelvis female ("why does the woman
  have a penis").
- Empirically: `gender=0` → female, `gender=1` → male (the sample naming in
  script 01 was misleading; `macro.json` governs).

### 4.4 Baked male/female bodies (intermediate; helpers+genitals issues)
- Baked the composed gender into the base as a static body. Male/female
  geometry really differed (bbox), but the **base hm08 topology still carried a
  forward pubic bump** (~0.29 at y≈0.85-0.95) and there are **no genitals at
  all** on the smooth base. Provides confirmation that genitals need **proxy
  topology**.

### 4.5 Proxy misuse (wrong conclusion — corrected)
- I applied the **PunkElvs proxies** and loaded base/penis targets **directly
  onto the proxy mesh**. Every target "touched all 16k proxy verts" → I wrongly
  concluded "proxies don't support morphs".
- **Correction via MakeHuman docs:** FAQ "Model sliders stops working when using
  a proxy" says sliders **do** work — the basemesh morphs, the **proxy covers
  it**, and you **refit** the proxy (or enable auto-refit). So a proxy is a
  tight body **cover** over the morphable base sharing the same skeleton.
- Confirmed empirically: setting a base morph then re-running
  `ClothesService.fit_clothes_to_human` moves the proxy (breast → 304 proxy
  verts; nose → 121).

### 4.6 Offline morph-transfer (final, correct)
- A proxy's topology differs from the base, so base targets can't be applied to
  it directly. But we can bake the *refit result*: for each base morph, set it,
  refit the proxy, and record the proxy-vertex delta as a **proxy-native morph
  target**.
- Direct target deltas combine linearly at runtime with genitals present and
  no runtime refit. Macro combinations use endpoint-delta approximations.
- **Shape-key gotcha:** `shape_key_add` did not auto-create a `Basis`; the
  first added key became the base, and the glTF exporter treated it as Basis.
- **Refit overwrite gotcha:** each later refit changed the active proxy shape
  key and erased its captured coordinates. The exported deltas were only
  approximately 10⁻⁹ units. The generator now captures all refits in compact
  arrays before it creates any proxy shape key. Exported deltas are now
  0.000826-1.102110 units.

## 5. Final implementation

### `bake-core-rs/` (isolated Rust prototype)

This directory tests fixed-array SurfaceWrap fitting, sparse-target evaluation,
macro products, corrected skin transfer, and normal rebuilding. It is not an
engine crate, workspace member, worker, public API, or accepted runtime design.

The 14 tests include no-allocation checks and an MPFB golden fixture for 26
sampled CC0 `short04` hair vertices. The neutral and head-width fit error limit
is `3e-6` Blender units.

### Hair selection and live fit

The prototype offers None, CC0 `short04`, and CC0 `ponytail01`. The generated
character GLB contains both body-rig-skinned card meshes and one dark scalp cap.
Only the selected style and the cap are visible.

A 733-vertex compact `hm08` driver fits the exact PunkElvs scalp. Each hair
style then fits to that proxy scalp. The sidecar contains 201 structural target
streams. It excludes all expression and viseme targets, because transient face
animation does not change hair rest shape.

The generator binds each hair vertex to its nearest proxy-scalp triangle. It
also supplies 8 mm minimum clearance for vertices within 30 mm of the scalp.
The generator transfers rig weights from the same proxy triangle.

The runtime applies sparse driver changes incrementally. It first evaluates the
PunkElvs scalp and then evaluates the selected hair from that surface. It also
converts Blender coordinates to glTF Y-up coordinates, rebuilds normals, and
updates the selected geometry. Validation compares neutral fits and verifies
that `head-scale-horiz-incr` produces the same proxy-scalp positions as the body
morph. The prior direct-hm08 browser times do not apply to this two-stage fit.

The first clipping correction used `helper-hair` as a cap. That was incorrect,
because `helper-hair` is a large fitting cage that looked like a second
hairstyle. The final cap uses exact PunkElvs scalp faces.

The base-body scalp and PunkElvs proxy have different topology. Thus, a cap from
the base body can leave top and rear holes. The final generator duplicates the
exact PunkElvs scalp faces and their MHCLO bindings. A per-corner mask removes
only those same body faces. The duplicate replaces them with skin color for None
and hair color for either selected style.

`ponytail01` currently uses interpolated proxy-scalp rig weights. It has no SpringChain
or imported ponytail sub-rig in this prototype.

### `scripts/gen-proxy-transfer.py` (Blender headless)
Per sex:
1. `HumanService.create_human()` + `add_builtin_rig(game_engine)`.
2. Apply the PunkElvs `.proxy` via `Mhclo().load` + `load_mesh` +
   `fit_clothes_to_human` + `set_scalings` + `set_up_rigging`.
3. Capture all applicable `target.json` targets and all 62 asymmetry targets.
4. Capture 52 ARKit units and 35 non-empty speech visemes from the CC0 packs.
5. Capture the age, muscle, weight, proportions, height, cup, and firmness ends.
6. Replace the Caucasian macro with each ethnicity macro and capture the fit.
7. Append the base eyes, teeth, and tongue with vertex colors and rig weights.
8. Restore Caucasian, make all polygons smooth, and create `Basis` plus targets.
9. Bind both hairstyles to the PunkElvs scalp and transfer its rig weights.
10. Cook the compact hm08 driver, the two-stage bindings, structural deltas, and
    exact proxy-scalp cap.
11. Export the mesh, rig, morph-name, logical-control, and hair-fit sidecars.

The exporter does not include normal morphs. Their sparse data caused unchanged
surfaces to receive incorrect black shading in Three.js. Smooth base normals
remain active for all position morphs.

Verified output:
| Body | vertices | triangles | morphs | genitals |
|------|---------:|----------:|-------:|----------|
| male | 18,187 | 33,392 | 691 | penis |
| female | 17,515 | 32,424 | 689 | vulva |

Smooth polygons reduce the prior flat export from approximately 63,000 split
face-corner vertices to approximately 17,000 shared vertices.

### `scripts/gen-makehuman.ts` (bun wrapper)
Runs `gen-proxy-transfer.py` for both sexes with `SEX`, `PROXY_ROOT`,
`FACE_TARGET_ROOT`, and `CHAR_OUT`. It writes temporary files and only publishes a complete set.
`--python-exit-code 1` makes Blender generation fail closed.

### `src/main.ts` (Three.js WebGL editor)
- Loads `character_female.glb` by default; **Male body / Female body** buttons
  load the other sex's self-contained mesh.
- 423 male and 422 female knobs drive 691/689 target indexes. Paired targets
  use one −1…+1 knob. One-sided targets use 0…1. Bilateral controls use
  separate left and right knobs.
- The face controls include all 52 ARKit units and all 35 non-empty speech
  visemes. The two silence labels have no displacement and are not morphs.
- Direct controls reproduce MPFB targets exactly. Macro endpoints are linear
  deltas from the 0.5 baseline, so simultaneous macro values are an approximation
  of MPFB's nonlinear combined macro stack.
- The Face preview panel supplies 17 expression compositions and all 35
  non-empty speech shapes. Expression and speech previews can combine.
- Randomize / reset, bone focus / list, body-part toggle, load-any-glTF.
- Vertex colors distinguish the skin, eyes, teeth, and tongue.

## 6. Commands

```sh
cd prototype/character-editor
bun install
bun run gen:character       # regenerate both proxy bodies (Blender headless)
bun run test                # validate GLBs and TypeScript
bunx vite                    # dev server (prints port; e.g. 5175)

# From the repository root:
cargo test --manifest-path prototype/character-editor/bake-core-rs/Cargo.toml
```

One-time prerequisites: Blender 5.x on PATH; MPFB extension installed to
`~/.config/blender/5.2/extensions/user_default/mpfb` (unzip the download,
copy `src/mpfb` there); PunkElvs proxies in
`assets/character-rig/downloads/proxies/punkelvs_<sex>/`.

## 7. Licenses (see research note for full detail)

- MakeHuman base + morph targets: **CC0** (assets released CC0 as data since
  1.2.0, Sept 2020; verified at 4 levels incl. shipped `license.txt` and the
  actual `.target` headers). The in-game editor use is explicitly permitted by
  the official FAQ.
- **PunkElvs male/female proxies: CC-BY 4.0** (attribution; confirmed in both
  `.proxy` headers).
- MPFB add-on source: GPLv3 (tool only, not shipped; no AGPL contagion).
- Avoid: MB-Lab/CharMorph morph data (AGPL); SMPL/GHUM (no redistribution);
  community `adult_*_genitalia` proxies (AGPL-labeled).
- Wings/tails pack `bodyparts03` = CC-BY; horns `bodyparts01` = CC0.

## 8. Open items / next steps

- Select the patent-review path before direct body manipulation. The recommended
  default is fixed hotspots plus explicit operations and synchronized controls.
- Implement the fixed source-to-bake path in
  `runtime-character-bake-plan.md`. Use Humentity and `bevy_make_human` as the
  co-primary N1 permissive implementation references.
- Add authored SpringChain data to moving long hair. Short hair stays rigid to
  the main body rig.
- Add an audio or phoneme timeline that drives the Meta viseme set.
- Wings + tails (CC-BY) via MPFB sub-rigs.
- Real textures/materials (currently flat skin material).
- Female vulva morphs (currently static base geometry; no vulva shape sliders).
- Persist presets; bone-pose presets.
- Browser screenshot QA after each bake.

## Update (v2, same session) — sex differentiation, ethnicity, rendering

- **Male/female previously looked identical** (only genitals differed) because
  the sex-proportion macro was not baked and the PunkElvs proxies share one
  body envelope. Fix: the generator now bakes the **sex macro** (gender=1 male,
  0 female) + a default caucasian ethnicity into the base before the proxy
  transfer, so the refit proxy conforms to a genuinely male vs female
  silhouette. Verified: male bbox 1.136×1.747×0.475 vs female 0.980×1.606×0.388.
- **Ethnicity:** the editor has a **Caucasian/Asian/African** selector. The
  generator replaces the Caucasian macro values before each race capture. It
  does not add a complete race target on top of the Caucasian target. Male has
  691 morphs, and female has 689 morphs.
- **Rendering:** added **Smooth shading** and **Wireframe** checkboxes
  (set `material.flatShading` and `material.wireframe` on all skinned meshes).
  Verified working in the browser.

## Update (v3) — transfer and surface corrections

- The generator captures all proxy fits before shape-key creation. This prevents
  later refits from erasing each new target.
- Ethnicity capture replaces the Caucasian macro. It never combines two complete
  ethnicity macros.
- Smooth polygons reduce exported body vertices from 63,008/64,944 to
  16,926/17,598 before the 506 face-helper vertices are appended.
- Position-only morph export prevents incorrect black normal shading.
- `validate-character-glb.ts` checks all 691/689 targets, all 423/422 controls,
  CC0 face manifests, control coverage, body bounds, displacement ranges, and
  finite values.
- Browser screenshots verified smooth Caucasian, Asian, and African bodies.
  They also verified the breast and penis length sliders.

## Update (v4) — clickable color zones

- The browser derives a zone paint from direct morph-target displacement.
- Each target uses normalized displacement. The paint ignores measurement
  targets and values below 8% of that target's maximum displacement.
- Each triangle receives one primary anatomy category. Bilateral categories
  receive separate left and right zones.
- Hover creates a skinned, morph-aware color overlay for the applicable zone.
- Hover input uses a position-only hit mesh at 20 Hz. It does not raycast the
  689-target skinned mesh on each pointer event. A 500-event browser burst took
  1.5 ms to queue.
- Click keeps the overlay and filters the panel to related direct, macro, and
  asymmetry controls. **All controls** removes the filter.
- Six unit tests cover triangle category selection and control filtering.
- Browser checks identified head, breast, hip, and right-leg zones. A breast
  click showed 10 related controls, including Cup size and Breast firmness.

## Update (v5) — expressions, visemes, and mouth geometry

- The repository now contains the official CC0 `faceunits01`, `visemes01`, and
  `visemes02` functional packs. Their manifests and 89 source targets are part
  of generation input.
- `sil_00` and `viseme_sil` are empty files. The generator correctly treats
  them as silence labels, not morph targets. The runtime includes the remaining
  87 face targets.
- The PunkElvs proxy has no tongue geometry. The first transfer therefore
  failed closed because `tongueOut` had zero proxy displacement.
- The corrected generator appends 506 base-mesh vertices for both eyes, both
  teeth sets, and the tongue. It copies rig weights and captures these vertices
  for every morph index.
- Vertex colors supply skin, eye, tooth, and tongue colors without textures.
- Browser checks loaded 422 female controls and 423 male controls. A combined
  `jawOpen`, `tongueOut`, and Meta `CH` test visibly opened the mouth and moved
  the red tongue.

## Update (v6) — face preview library

- The Face preview panel has 17 named expression compositions, all 14 Meta
  visemes, and all 21 Microsoft visemes.
- Expression selection clears only ARKit expression values. Speech selection
  clears only speech values. Thus, one expression and one speech shape can be
  previewed together.
- **Focus face** moves the current camera toward the head bone. **Reset face**
  clears both sets without changing the body controls.
- The expression compositions are useful ARKit-unit combinations. They are not
  an official ARKit emotion standard.
- Body randomization does not randomize face targets. This prevents all 87 face
  morphs from becoming active at the same time.
- Three preset tests check unique names, all 35 speech shapes, available target
  names, bounded weights, and neutral reset entries.
- Browser checks verified Happy plus Meta PP, preset retention after a body
  swap, face focus, and face reset.
