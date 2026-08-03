# SOMA → MakeHuman Retargeting — Note (2026-08-03)

Goal: retarget NVIDIA SOMA motion (Kimodo output) onto the MakeHuman / MPFB2
`game_engine` rig in the character editor prototype, in real time.

Status: decided architecture. Implementation not started.

## Context

- `prototype/character-editor/` — Three.js (WebGL) character editor. Bodies are
  MPFB2 `game_engine`-rigged glTF meshes (54 skin joints, fully articulated
  hands).
- NVIDIA motion generation (Kimodo) trains/emits on the **SOMA skeleton**:
  internal 30-joint `somaskel30`, exported 77-joint `somaskel77`, T-pose rest.
  Kimodo exports NPZ (`local_rot_mats`, `root_positions`).
- SOMA and `game_engine` are near-identical in topology (both have full hand
  chains, matched spine/limb counts) but use **different rest-pose local axes**
  and slightly different proportions.

## Decided approach (cleanest for a fixed pair)

Map **once**, bake a small static asset, apply in O(1) per frame at runtime.
The engine never re-maps.

### Baked asset (one-time, per skeleton change)
`docs/api`-adjacent / asset `soma_to_game_engine.json`:
- `root_scale` — target/source height ratio + foot offset.
- Per-joint entries: `{ source, target, offset }` (basis matrices).
- `skipped` — bones with no correspondence (held at rest).
- Constant source/target rest-pose world orientations, precomputed once.

### Runtime apply (per frame)
Recommended: **world-frame (FK-based) Level-2 orientation matching**, exact and
robust (no per-bone hand-tuned offsets):
1. Source FK over SOMA `local_rot_mats` → source world orientations `S_w`.
2. For each mapped joint: desired target world
   `D = T0_w · (S0_w⁻¹ · S_w)` (baked constant rest poses).
3. Decompose to target local via target FK → `bone.quaternion`.
4. Scale root translation by `root_scale`; optional ankle/ground pass.

Cost ≈ ~54 FK matrix transforms/frame — trivial for real-time. Non-allocating
with preallocated buffers (fits sealed-runtime rules).

Alternative (cheaper, no FK): pure per-bone basis `target_local = S_j · O_j`.
Fastest, but only exact when rest frames align structurally; the FK version is
safer and not meaningfully slower for 54 joints.

## Why not ML / neural retargeting
For a **fixed** skeleton pair the mapping is a constant function with an exact
closed form (lossless, deterministic, no training data, no error). ML adds error
and non-determinism for zero gain. ML would only help to *auto-discover the
correspondence map* from rest geometry (one-time), not to transfer motion.

## Mapping reference — SOMA → `game_engine`
Bone map (both sexes share one rig; `l`/`r` mirror, apply to both sides):

| SOMA | `game_engine` |
|---|---|
| Hips (root) | `pelvis` (root translation feeds `Root`) |
| Spine1 / Spine2 / Chest | `spine_01` / `spine_02` / `spine_03` |
| Neck1 | `neck_01` (Neck2: no match) |
| Head | `head` |
| XShoulder / XArm / XForeArm / XHand | `clavicle_x` / `upperarm_x` / `lowerarm_x` / `hand_x` |
| XHandThumb1/2/3 | `thumb_01_x` / `thumb_02_x` / `thumb_03_x` |
| XHand{Index,Middle,Ring,Pinky}1/2/3 | `{...}_01/02/03_x` |
| XLeg / XShin / XFoot | `thigh_x` / `calf_x` / `foot_x` |
| XToeBase | `ball_x` |
| XToeEnd | skipped |

Unmapped (hold at rest): `Neck2`, `Jaw`, `LeftEye`, `RightEye`, `HeadEnd`,
`XHand{...}4`, `XHand...End`, `XToeEnd`, `neutral_bone`.

Coverage: 52 of 54 game joints get a rotation source (+ `Root` gets root
translation). `somaskel30` omits finger detail, so hands stay relaxed if driven
from the 30-joint output.

## Primary source of truth
Skeleton definitions: `nv-tlabs/kimodo` — `kimodo/skeleton/definitions.py`
(`SOMASkeleton77`, `SOMASkeleton30`, `SMPLXSkeleton22`, `G1Skeleton34`).

## Tools considered
- **Auto-Rig Pro** (SuperHive, Blender) — installed at
  `~/.config/blender/5.1/extensions/superhivemarket_com/auto_rig_pro/`. Non-
  manual Remap ("Guess" + side-aware fuzzy bone mapping), saves `.bmap`
  presets. Version split: MPFB bake on Blender 5.2, ARP on 5.1. No SOMA or
  `game_engine` preset bundled; a custom `.bmap` would be authored once. This
  is the practical automatic tool but is not more correct than the analytic
  Level-2 solve for the fixed pair.
- NVIDIA/soma-retargeter — Level-3 IK, but currently outputs Unitree G1 DOF
  (not a game skeleton).
- Ozz Animation (C++ library) — high quality, but a native dependency;
  unnecessary for the near-1:1 pair.
- UEngine IK Rig / Unity Mecanim Humanoid — gold-standard/automatic but
  engine-bound.
- bvh_retarget.py (MakeHuman) — name-based Level-1 only, target is MHX rig not
  `game_engine`, no SOMA source definition. Not directly usable.

## Open implementation work
- Bake script `scripts/bake-soma-map.ts` (compute basis/rest orientations from
  the two GLBs).
- `SomaRetarget` engine module (preallocated, non-allocating, drives bones).
- Editor UI: load Kimodo NPZ/BVH → apply → scrub/pose live.
