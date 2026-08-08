# Character Rig Prototype — Asset Sources & Licenses

Prototype rig assets for the afterglow-engine character system. All assets are
from the MakeHuman / MPFB2 community. They load in **Blender 5.2 + MPFB2**,
rig through **Rigify**, and export to **glTF** (skinned mesh + shape keys).

## License summary

| Component | License | Ship in closed-source? |
|-----------|---------|------------------------|
| Base human body (MPFB2 default) | CC0 | Yes, no attribution |
| MakeHuman morph targets incl. penis targets | CC0 | Yes |
| `faceunits01`, `visemes01`, `visemes02` | CC0 | Yes |
| `makehuman_system_assets` (skins, hair, eyes, clothes) | CC0 | Yes |
| Wings + tails (bodyparts03) | CC-BY | Yes, keep attribution |
| Horns (bodyparts01) | CC0 | Yes |
| PunkElvs Male/Female Proxy1 (genitalia) | CC-BY | Yes, keep attribution |
| Community AGPL genital proxies (raw) | AGPL3 | No — do not redistribute |
| MPFB2 addon code | GPLv3 | Not linked; tool only |

## Downloaded files (`downloads/`)

| File | Size | License | Contents |
|------|------|---------|----------|
| `mpfb2_v2.0.17.zip` | 43M | GPLv3 (addon) | MPFB2 Blender add-on (from `makehumancommunity/mpfb2` tag v2.0.17) |
| `makehuman_system_assets_cc0.zip` | 268M | CC0 | system pack: skins, eyes, hair, clothes |
| `bodyparts01_cc0.zip` | 812K | CC0 | horns (faun, minotaur, quetzalcoatl) |
| `bodyparts03_cc-by.zip` | 38M | CC-BY | 11 wings/tails items |
| `functional/` | 2.4M | CC0 | 52 ARKit units, 22 Microsoft labels, 15 Meta labels |
| `proxies/punkelvs_male` (`elvs_maleproxy1.proxy/.obj/.mhw`) | 3M | CC-BY | male proxy: simplified penis, "works with the male genital sliders" |
| `proxies/punkelvs_female` (`elvs_femaleproxy1.proxy/.obj/.mhw`) | 2.6M | CC-BY | female proxy: simplified vulva |
| `genitalia/` (see ledger below) | mixed | see below | community proxies |

## Standard hair audit

`makehuman_system_assets_cc0.zip` contains ten CC0 card-mesh styles:
`afro01`, `bob01`, `bob02`, `braid01`, `long01`, `ponytail01`, `short01`,
`short02`, `short03`, and `short04`. Together they have 27,117 vertices and
18,943 faces.

Each hair vertex has an `MHCLO` three-parent surface-wrap record. The ten styles
use only 1,601 unique `hm08` body or helper vertices. Thus, they can refit to a
runtime body shape if the engine keeps a compact deformed `hm08` driver.

The local ZIP has no hair sub-rig or custom-weight sidecars. The upstream CC0
`makehuman-assets` repository adds `ponytail01.mpfbskel` and `ponytail01.mhw`:
one root plus five ponytail bones. See
`docs/research/makehuman-hair-runtime-dynamic-character-system.md`.

The character prototype extracts all ten styles from this ZIP. It commits their
CC0 diffuse textures beside the generated GLBs. It keeps authored helper-cage
bindings and composes body-bound records with the PunkElvs proxy.

Downloaded `genitalia/` ledger (each page verified 2026-08-02):

| File | License | Verdict |
|------|---------|---------|
| `adult_male_genitalia.proxy` | AGPL | **do not redistribute** |
| `adult_female_genitalia.proxy` | AGPL | **do not redistribute** |
| `female_generic_with_simplified_genitals.proxy` | CC0 | OK |
| (re-download instead) `punkelvs_male_proxy1` / `punkelvs_female_proxy1` | CC-BY | OK, keep attribution |


## Wings + tails available (bodyparts03, CC-BY)

- culturalibre_cl_devil_tail
- culturalibre_tigra_tail
- elvs_cat_tail_as_clothes_1
- elvs_childrens_costume_wings1
- elvs_dual_fanned_wings_collaberation
- elvs_fluffy_tail_as_clothes_1
- elvs_static_wings1_dragonfly
- elvs_static_wings2_dragon1
- elvs_static_wings3_butterfly1
- elvs_static_wings4_bone_wings1
- elvs_static_wings5_fairy_moth1

Horns (bodyparts01, CC0): culturalibre_faun_horns, culturalibre_minotaur_horns,
freezychan_lucoa_quetzalcoatl_horns.

## Source URLs

- MPFB2 add-on: https://github.com/makehumancommunity/mpfb2 (downloads.html)
- Asset packs: https://static.makehumancommunity.org/assets/assetpacks/
  - system: `makehuman_system_assets_cc0.zip`
  - horns: `bodyparts01_cc0.zip`
  - wings/tails: `bodyparts03_cc-by.zip`
  - ARKit units: https://static.makehumancommunity.org/assets/assetpacks/faceunits01.html
  - Microsoft visemes: https://static.makehumancommunity.org/assets/assetpacks/visemes01.html
  - Meta visemes: https://static.makehumancommunity.org/assets/assetpacks/visemes02.html
- Genitalia proxies: http://www.makehumancommunity.org/proxies.html

## Genitalia — CORRECTED license status (2026-08-02)

Research update: the MakeHuman 1.2.0+ (Sept 2020) license change released ALL
bundled assets (base mesh, targets/proxies, morphs, textures, clothes) as CC0
as **data files**, not merely by export. The official FAQ explicitly permits
building your own character generator with no license restriction. See
`docs/research/open-license-character-rig-in-game-editor.md`.

For genitalia specifically:
- **Male (official, CC0):** the official MakeHuman data ships six **penis morph
  targets** (`data/targets/genitals/`: penis-circ/length/testicles, incr/decr)
  — real blendshapes, runtime-morphable, part of the CC0 sweep.
- **False alarm on AGPL:** old AGPL headers on asset files are **stale** (the
  project says to report them as bugs). The `adult_male/female_genitalia`
  **community proxy pages** are still individually labeled AGPL — avoid those
  high-detail topology proxies for a redistributable editor.
- **Clean female (CC-BY):** PunkElvs Male/Female Proxy1 — simplified penis/
  vulva, weighted, "works with the male genital sliders". Female anatomy
  option that is redistributable with attribution.
- CAUTION: nothing AGPL from MakeHuman/MB-Lab/CharMorph may be redistributed in
  a self-authored editor without AGPL contagion.

## Wings/tails/horns license
- `bodyparts01` horns = CC0.
- `bodyparts03` wings/tails = CC-BY (attribution). These load in MPFB2 as
  sub-rigged clothes (bone-driven wings/tails).
- `genitalia/*.proxy` downloaded from the community proxies page = AGPL-labeled
  community core; do NOT redistribute these raw files in the editor. Prefer the
  CC0 official penis targets and CC-BY PunkElvs proxies instead.

## Next steps

1. Add one wing and one tail with an MPFB sub-rig.
2. Add an audio or phoneme timeline for the Meta visemes.
3. Cook the selected character format through `afterglow-pipeline`.

## Chosen genital path (2026-08-02)

PunkElvs **male + female proxies** (CC-BY 4.0, confirmed in `.proxy` headers)
were downloaded to `downloads/proxies/punkelvs_{male,female}/` and are used as
the character topology for the prototype editor. Genitals (penis / vulva) exist
only on this proxy topology. The body-morph library is transferred onto the
proxy at bake time (offline) so the editor needs no runtime refit — see
`docs/implementation/character-editor-prototype.md`.

The community `adult_male_genitalia.proxy` / `adult_female_genitalia.proxy`
(AGPL-labeled) were deleted and are NOT used.
