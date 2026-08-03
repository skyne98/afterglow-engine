# Open-License Character Rig Research for an In-Game Character Editor

Date: 2026-08-02
Status: Research note for the character-rig prototype and the future in-game
character editor (hard requirement).

## TL;DR / recommendation

**MakeHuman + MPFB2 is the only foundation that satisfies all three
requirements — good base + full morph library + genitalia — with a license
that permits building your own in-game editor.** This corrects an earlier
draft caution in this repo that was based on stale (pre-1.2.0) license
information.

The correct, current facts (MakeHuman 1.2.0+, Sept 2020 license change):

- Base mesh, targets/proxies, morphs, textures, clothes = **CC0 data files**,
  not merely CC0-by-export.
- The official FAQ explicitly permits the in-game editor use case: "Take all
  mesh and target assets and build a character generator of your own, with no
  restriction on what license that character generator needs to have."
- Old AGPL headers on asset files are **stale**; the project treats them as
  bugs to report, not license claims.

## The one genuine gap: redistribution of the morph + genital *data*

Every alternative foundation fails on redistribution because the releasing
project grants CC0 only *via an export exception* or licenses its data under
AGPL/copyleft or academic-only terms:

| Foundation | Base | Morphs | Genitalia | Editor-redistributable? |
|-----------|------|--------|-----------|--------------------------|
| **MakeHuman / MPFB2** | CC0 | CC0 | ✓ CC0 male targets; CC-BY female proxy | **YES** |
| MB-Lab / CharMorph | AGPL | AGPL | refused / none | NO (AGPL) |
| SMPL / SMPL-X / STAR / GHUM / Anny | academic-only | academic | none | NO |
| Blender Studio Human Base Meshes | CC0 | none | none | partial (no morphs) |
| Blender Studio chars (Rigo/Storm/Rain) | CC-BY | CC-BY | none | CC-BY, but no mature |
| VRM / VRoid | per-model | per-model | none (mature discouraged) | per-model |
| Khronos glTF samples | CC0 | small | none | fixtures only |
| OpenGameArt / BodyParts3D / marketplaces | varies | rare | varies; SA issues | unreliable |

## Dive-by-dive evidence

### 1. MakeHuman current license (authoritative)
- `makehuman/LICENSE.md` (current): source code = AGPL; **bundled assets**
  ("base mesh and proxies", "targets and modifiers", "textures", "clothes",
  "poses and expressions") = **CC0 1.0 Universal**. Output (files, MPFB
  import, scripting, plugins) = "your data, yours to handle".
- `FAQ:Are MakeHuman files free?` explicitly allows building your own
  character generator with no license restriction; no attribution; commercial;
  sell the model.
- The historical `license_explanation.html` (which described base mesh/targets
  as AGPL and a narrow GUI-export CC0 exception) is **marked no longer valid**
  for MakeHuman 1.2.0+.
- Forum `t=18707` (dev discussion of the license rewrite): "AFTER: Targets,
  proxies and the base mesh are considered graphical assets, covered by CC0 …
  Targets are CC0 no matter how you got hold of them."

### 2. Male genitalia are official CC0 morph targets
- `makehuman/data/targets/genitals/` contains six official **penis morph
  targets**: `penis-circ-decr/incr`, `penis-length-decr/incr`,
  `penis-testicles-decr/incr`. These are blendshapes → runtime-morphable in an
  in-game editor, and part of the CC0 bundled data.
- Max Planck pip distribution of these targets is CC0 per the sweep.

### 3. Clean community genital proxies
- **PunkElvs Male/Female Proxy1** — **CC-BY**, simplified penis/vagina,
  "works with the male genital sliders", weighted. Good redistributable
  female-anatomy option (official data ships only male targets).
- **adult_male/female_genitalia (MHteam core)** community pages are still
  labeled **AGPL** ("anatomically correct", higher detail) — **avoid** for a
  redistributable editor. (These are topology proxies, not morphs, anyway.)

### 4. MB-Lab and CharMorph are unsuitable
- MB-Lab data (meshes, JSON targets) = **AGPL**; the maintainer explicitly
  refuses mature content "out of the box" (issue #324); has censoring code.
- CharMorph code = GPLv3; its meshes/data come from ManuelbastioniLAB =
  **AGPL**. Only the separate Vitruvian base character is CC0; the morph
  database is not.

### 5. Blender Studio CC0 Human Base Meshes
- 17+ meshes, **CC0**, updated through v1.4.0 (2025), downloadable from
  `download.blender.org/demo/asset-bundles/human-base-meshes/`. They are
  **base/sculpting meshes only — no morph library, no genitalia, no rig.** Good
  clean body topology; you must author morphs + rig + mature content.

### 6. Blender Studio CC-BY full rigs
- Rigo / Storm / Rain / Snow (CloudRig + Rigify) are **CC-BY** full body + face
  shape-key rigs, film-quality and high-poly (your meshopt LOD handles that).
  No mature content. Good quality body+face foundation if you accept CC-BY and
  author mature content; not a full mature editor out of the box.

### 7. VRM / VRoid
- VRM is a format; each model carries its own license in VRM metadata. VRoid
  Hub restricts mature content and per-model licenses vary (some CC0 on
  OpenGameArt, many not). Not a uniform redistributable mature base.

### 8. Khronos glTF sample models
- CesiumMan / Fox / BrainStem / Monopod (skinning+anim) and Duck/Sonomotor
  (morphs) are mostly CC0 — ideal **validation fixtures** for the sealed
  skinning + morph + LOD path, but not an editor body base.

### 9. Parametric research body models — all disqualified
- SMPL / SMPL-X / STAR / GHUM: academic/research licenses, **redistribution
  not allowed**, commercial only via paid licensing (e.g. Meshcapade). No
  mature content. Anny (NAVER) is research-HMR-oriented. None usable for a
  redistributable in-game editor.

### 10. OpenGameArt / BodyParts3D / adult marketplaces
- OpenGameArt has CC0 basemeshes and stylized humanoids but no complete
  base+morphs+genitalia mature library.
- BodyParts3D (Anatomography) is share-alike (CC BY-SA) → incompatible with
  closed-source redistribution.
- Dedicated adult marketplaces (SmutBase, Turbosquid, CGTrader, Meshy) have
  individual items with mixed licenses and chain-of-title that is hard to
  certify for a foundation; Meshy "CC0" items are AI-generated with
  provenance/copyright uncertainty. Not recommended as a foundation.

### 11. Practical export mechanism (the editor's data path)
- MPFB2 treats targets as blendshapes ("a target is conceptually a shape key").
- Export flow: *bake shapekeys* then *delete helpers*, then export glTF.
- Issue #303 proposes the exact editor architecture: a *design version* (full
  MPFB rig, parametric) vs an *export version* (baked geometry + mesh-specific
  shape keys + glTF-compatible materials). MPFB2 exports glTF with shape keys.

## Concrete recommended stack for the in-game editor

1. **Body + morph library**: MPFB2 default human, bake the full target library
   (incl. the six CC0 penis targets) as glTF shape keys → runtime-editable, CC0.
2. **Skeleton**: Rigify-derived default MakeHuman rig (bones), exported in the
   same glTF.
3. **Wings + tails**: `bodyparts03` pack (CC-BY) with MPFB2 sub-rigs.
4. **Female genitalia**: PunkElvs Female Proxy1 (CC-BY), or author a vulva
   shape key in-house (CC0 or owned).
5. **(Optional) CC0 base topology** + authored morphs if a 100%-CC0-everything
   path is wanted; but the CC-BY pieces (wings/tails/female proxy) are
   redistributable with attribution, so the stack ships fine either way.

License ledger per file already tracked in
`assets/character-rig/SOURCE.md` (update here plus in `assets/`).

## Triple-check verification (2026-08-02, primary files, not search snippets)

All core claims re-verified directly against the authoritative files:

1. **MakeHuman bundled assets = CC0** — confirmed at 4 independent levels:
   - `makehuman/master/LICENSE.md` section C: base mesh, proxies, targets and
     modifiers, textures, clothes, poses/expressions = CC0 1.0 Universal.
   - The **shipped binary's** `makehuman/license.txt` (the file inside the
     released app) contains the identical section C.
   - `LICENSE.ASSETS.md` = the full CC0 1.0 legal text; `LICENSE.CODE.md` =
     AGPL for source only.
   - `license_headers/target.txt` template and the actual official
     `data/targets/genitals/penis-length-incr.target` file header both read:
     "This asset was explicitly released as CC0 in september 2020", copyright
     Data Collection AB, primary legal contact Data Collection AB.

2. **The in-game editor is explicitly permitted** — the official FAQ
   `are_makehuman_files_free.html`: "Take all mesh and target assets and build
   a character generator of your own, with no restriction on what license that
   character generator needs to have." 1.2.0 release notes confirm the license
   "changed to be more comprehensive and permissive".

3. **Genitalia ledger (each page re-polled 2026-08-02):**
   - Official penis targets (`penis-circ/length/testicles` incr/decr) = **CC0**
     (file header).
   - `female_generic_with_simplified_genitals` proxy = **CC0**.
   - `punkelvs_male_proxy1` / `punkelvs_female_proxy1` = **CC-BY** ("works with
     the male genital sliders").
   - `adult_male_genitalia` / `adult_female_genitalia` (MHteam community core)
     = **AGPL** — do not redistribute; prefer the CC0/CC-BY items above.

4. **MB-Lab / CharMorph morph data = AGPL** — CharMorph `license.txt`: "all the
   meshes and data ... are released under GNU AGPL3" (from ManuelbastioniLAB).
   MB-Lab maintainer refuses mature content "out of the box".

## Open questions / decisions for the user
- Female genitalia source: accept CC-BY proxy vs author a CC0 vulva shape key.
- Whether to treat the six official penis targets as sufficient male anatomy,
  or also bake a CC-BY/CC0 proxy.
- Verify in-engine that MPFB2's baked shape-key set (hundreds, incl. genitals)
  exports cleanly to glTF at target vertex count.

## Addendum: genitals require proxy topology + offline morph transfer (2026-08-02)

Built and verified in `prototype/character-editor/`. Two important corrections
to the earlier analysis:

1. **Genitals exist only on proxy topology.** The MakeHuman base (hm08) has no
   genital geometry; a vulva/penis appears only via a **proxy** (alternate
   pelvis mesh). The 6 official penis targets shape an existing penis on a
   male proxy; they produce nothing on the smooth base.

2. **Proxies DO support morphs (my earlier "they don't" was wrong).** Per the
   official FAQ "Model sliders stops working when using a proxy", sliders keep
   working — the basemesh morphs, the **proxy covers and hides it**, and you
   **refit** the proxy to the updated base (or auto-refit). A proxy is a tight
   body **cover** over the morphable base, sharing the same skeleton. Confirmed
   headlessly: setting a base morph then re-running `fit_clothes_to_human`
   moves the proxy.

3. **Runtime refit is avoided by offline morph transfer.** Because a proxy's
   topology differs from the base, base targets can't be applied to it
   directly. Instead, at **bake time**, for each base morph: set it, refit the
   proxy, and record the proxy-vertex delta as a **proxy-native glTF morph
   target**. Morphs are linear, so the precomputed deltas reproduce any
   combination at runtime — with genitals and **no runtime refit**. The editor
   only blends `morphTargetInfluences`.

Result: two self-contained skinned bodies, `character_male.glb` (691 morphs,
including 6 penis targets) and `character_female.glb` (689 morphs, vulva), built
from the **PunkElvs CC-BY 4.0** proxies and CC0 face helpers. PunkElvs proxy file
headers confirm CC BY 4.0.

Open in this addendum: female vulva currently has no shape sliders. The editor
includes all canonical controls, all 62 asymmetry targets, and all body macro endpoints.
Full process is in `docs/implementation/character-editor-prototype.md`.

## Correction: MPFB has expression and speech targets (2026-08-02)

The earlier runtime assessment was wrong because it checked only the installed
MPFB data directory. The functional face packs were not installed there.
MPFB 2.0.15 added a **Face operations** panel and direct support for:

- `visemes01`: 22 Microsoft-style speech visemes.
- `visemes02`: 15 Meta/Oculus-style speech visemes.
- `faceunits01`: 52 ARKit-style expression units.

All three functional packs are CC0. The official MPFB pages are:

- <https://static.makehumancommunity.org/mpfb/releases/release_2015.html>
- <https://static.makehumancommunity.org/mpfb/docs/lipsync/index.html>
- <https://static.makehumancommunity.org/mpfb/docs/exporting/export_copy.html>
- <https://static.makehumancommunity.org/assets/assetpacks/index.html>

The export-copy documentation says 54 ARKit units, but the current
`faceunits01.zip`, its pack manifest, and MPFB 2.0.17 `ARKIT_FACEUNITS` list
contain **52**. This matches the standard ARKit set. MPFB can load all face
shape keys onto the basemesh and propagate them to a proxy with
`FaceService.interpolate_targets`, using the MHCLO vertex correspondence.
Thus, the current genital proxy architecture can support these targets.

The committed GLBs now contain all 52 ARKit units, 21 non-empty Microsoft
visemes, and 14 non-empty Meta visemes. Each pack also has one empty silence
file, which is a label and not a morph. The generator appends the CC0 base eyes,
teeth, and tongue because the PunkElvs proxy has no tongue geometry. MPFB's Lip
Sync integration uses `visemes02`; the separate Lip Sync addon generates
animation from audio or phoneme data.
