# Animation: Generation & Runtime Models — Survey (2026-08-03)

Survey of state-of-the-art (SOTA) animation generation and runtime models,
focused on game-oriented and lightweight runtime options. Intended to inform
the afterglow-engine character animation system.

Related: `docs/implementation/soma-to-makehuman-retargeting.md`.

## 1. Animation generation (offline / authoring)

### Text-to-motion (T2M)
| Model | Year/venue | Notes |
|---|---|---|
| **MARDM** | CVPR 2025 | "Rethinking Diffusion for Text-Driven Human Motion Generation". Fixes redundant representation in diffusion. Strong quality SOTA. |
| **HY-Motion 1.0** (Tencent) | 2025 | Flow-matching T2M. Scalable, recent SOTA quality. On Hugging Face. |
| **MoMask** | CVPR 2024 | Masked modeling ("masked motion diffusion"). High quality, discrete tokens. |
| **MotionLCM** | ECCV 2024 | Latent Consistency Model. **Real-time / few-step** controllable T2M; bridges generation to runtime. |
| MDM / MotionDiffuse / MLD / T2M-GPT / MotionGPT | 2022–23 | Established baselines (MDM diffusion; MLD latent diffusion; GPT-style). |
| **Kimodo** (NVIDIA) | 2026 | Motion diffusion on the **SOMA** skeleton; production-oriented; feeds our SOMA retarget pipeline. |
| ReAlign / Motion-Adapter | 2025 | Controllability / compound-action adapters. |

### Motion in-betweening / keyframe (ML only)
- **Motion In-Betweening for Densely Interacting Characters** (SIGGRAPH Asia 2025) — neural.
- **Motion In-Betweening via Two-Stage Transformers** (2022) — transformer-based, conventional ML baseline.
- CMDI (Conditional Motion Inbetweening) — conditional VAE, game-relevant (handles dead zones).
- **MotionBricks** (NVIDIA, SIGGRAPH 2026) — modular latent + smart primitives; see runtime.
- (Non-ML interpolation such as SILK / Adaptive Interpolation-Synthesis excluded.)

### Generation verdict
For content production, **flow-matching (HY-Motion)** and **masked/consistency
(MoMask, MotionLCM)** lead; **Kimodo** is the fit if we stay on SOMA. Output is
typically SMPL(-X), SMPL, or SOMA → must retarget to the game rig (recall the
SOMA retarget note).

## 2. Runtime / real-time game-oriented (lightweight)

### Tier A — lightweight neural kinematic (ML, cheap)
- **Learned Motion Matching** (Ubisoft LaForge) — small embedding NN + KNN.
  Lightweight runtime; the ML data-driven choice for games. (Traditional,
  non-ML Motion Matching excluded.)
- **DeepPhase / PFNN / MANN** (Starke, Holden) — small mode/phase-adaptive
  MLP networks for locomotion. Real-time, CPU-friendly. Great per-frame
  generative interpolation.
- **Neural State Machine (NSM)** — phase-based RNN for scene interactions.

### Tier B — physics-based control (SOTA lifelike, heavier)
- **ASE** (SIGGRAPH 2022) — reusable adversarial skill embeddings
  (nv-tlabs/ASE, PyTorch). Needs physics sim + RL training.
- **C·ASE** (SIGGRAPH Asia 2023) — conditional variant (text/IK-conditioned).
- **MaskedMimic** (SIGGRAPH 2024, xbpeng) — unified physics control via masked
  motion inpainting, release-conditioned. Very game-relevant; runs against a
  physics simulation at runtime.

### Tier C — real-time generation / orchestration
- **MotionLCM** — real-time few-step consistency T2M (online prompting).
- **MotionBricks** (NVIDIA, SIGGRAPH 2026) — real-time modular latent generative
  model + smart primitives in **Unreal Engine 5**; part of GR00T Whole-Body
  Control; 350k+ real-time motions. Bleeding-edge but UE5/NVIDIA-stack bound.

## 3. Recommendations for afterglow-engine

Constraints: web wasm/TS, sealed runtime, no runtime allocation, lightweight,
game-oriented.

- **Runtime core (recommended): Learned Motion Matching or DeepPhase/PFNN-class.**
  Data-driven NNs, tiny, CPU-friendly, no general allocation, robust, and it
  composes with an existing clip library. Best fit for a real-time editor and
  sealed gameplay loop.
- **Authoring/generation (offline): flow-matching (HY-Motion) or masked
  (MoMask), or **Kimodo**/**SOMA** output** — baked to the `game_engine` rig via
  the SOMA retarget pipeline. No runtime cost (pre-cooked clips).
- **Physics-based (ASE/C·ASE/MaskedMimic):** highest lifelike quality but
  needs a physics sim, RL training, and more compute. Candidate for the native
  shell later, **not** the lightweight web/editor target now.
- **MotionBricks/MotionLCM:** forward-looking but bound to UE5/NVIDIA stack or
  heavier; revisit only if we adopt that stack.

## 4. Skeleton note
Neural models emit on SMPL(-X)/SMPL/SOMA. Everything must be retargeted to the
game rig. Our SOMA retarget pipeline (map-once → bake → O(1) runtime apply)
extends to SMPL(-X) with a similar baked map.

## 5. License filter (verified via GitHub/HF API, 2026-08-03)
Licensing decides what can ship in a commercial game.

### Permissive (MIT / Apache-2.0) — keep
| Model | License | Type |
|---|---|---|
| **MARDM** | MIT | generation |
| **MoMask** | MIT | generation |
| **MDM** | MIT | generation |
| **MLD** (latent diffusion) | MIT | generation |
| **T2M-GPT** | Apache-2.0 | generation |
| **MotionGPT** | MIT | generation |
| **Kimodo** (code) | Apache-2.0 | generation/tool |

### Conditional / restricted — note
| Model | License | Type |
|---|---|---|
| **Kimodo** (model weights) | NVIDIA Open Model License | generation — weights NOT permissive; code is Apache-2.0 |

### Excluded — non-permissive, no license, or no code
- **Non-commercial (explicit):** MotionLCM (Tsinghua/SH-AI), HY-Motion (Tencent,
  "other"), ASE (NVIDIA License), C·ASE (NVIDIA License).
- **No license file (default all-rights -> not permissive):** DeepPhase,
  AI4Animation (PFNN/MANN), NSM, MaskedMimic, MotionBricks.
- **No usable official code:** Learned Motion Matching (Ubisoft, paper only),
  Two-Stage Transformers in-betweening, Densely Interacting Characters, CMDI
  (repo gone), ReAlign, Motion-Adapter.

### License verdict
**Every runtime animation model in the survey lacks a permissive license.** The
only permissive options are offline generation models (MARDM, MoMask, MDM, MLD,
T2M-GPT, MotionGPT) plus Kimodo's *code* (but not its weights). A commercial
game must therefore either: license the runtime model commercially, generate
content offline under a permissive model and bake it, or use only
Apache/MIT-licensed pieces (which excludes all listed runtime models today).

### Date filter (2025+)
Applying a 2025-or-newer date filter to the permissive list leaves only:
- **MARDM** (CVPR 2025, MIT) — diffusion text-to-motion.
- **Kimodo** (2026, code Apache-2.0; weights NVIDIA OML) — SOMA motion diffusion.

Dropped for age: MoMask (2024), MDM (2022), MLD (2022), T2M-GPT (2023),
MotionGPT (2023) — all permissive but older than 2025.

### Final surviving (permissive + 2025+):
- **MARDM** (CVPR 2025, MIT) — generation, the practical content source.
  Text-to-motion; no strong keyframe in-betweening mode.
- **Anytop** (SIGGRAPH 2025, MIT) — skeleton-agnostic motion diffusion; generates
  motion for **arbitrary skeletons** from skeletal structure alone (good fit for
  the `game_engine` rig). Generation, not lightweight runtime. **Supports keyframe
  in-betweening via inpainting/editing (code released May 2025).**
- **Kimodo** — generation on SOMA (code Apache; weights not permissive).
  **Purpose-built keyframe in-betweening**: full-body constraints at start/end of
  a clip, full-body keyframes at arbitrary frames, sparse joint
  positions/rotations, end-effector targets, 2D waypoints/paths.

### Permissive runtime (pre-2025 — outside the 2025+ filter)
The permissive **runtime** physics controllers are all older than 2025:
- **KinPoly / UHC** (BSD-3-Clause, 2021/2022) — Universal Humanoid Controller;
  the genuinely permissive runtime physics controller.
- **DeepMimic** (MIT, 2018) — physics RL character control.

### Permissive runtime models (corrected — 2026-08-03)
Earlier claim "no permissive runtime models exist" was WRONG — the search was
too narrow (game locomotion/control diffusers only). Permissive runtime options
verified (Apache-2.0 / MIT / BSD-3):
| Model | License | Type |
|---|---|---|
| **MediaPipe** (Google) | Apache-2.0 | real-time full-body pose/3D → avatar |
| **MoveNet / BlazePose** (`tfjs-models`) | Apache-2.0 | real-time 2D/3D pose |
| **mmpose** (OpenMMLab) | Apache-2.0 | real-time 2D pose |
| **pose-animator** (Google) | Apache-2.0 | pose → 2D vector avatar |
| **avatar-animator** | Apache-2.0 | real-time 2D avatar |
| **RealisMotion** (ICML 2026) | Apache-2.0 | runtime motion control |
| **motion-anything** (nexu) | Apache-2.0 | chat/agentic motion engine |
| **DeepMimic** | MIT | physics RL character controller |
| **KinPoly / UHC** | BSD-3-Clause | physics humanoid controller |
| **Ozz Animation** (C++) | MIT | generic runtime skeleton animation |

Non-permissive among new finds (do NOT use): DigiHuman (GPL-3.0),
HY-Motion-1.0 repo (no SPDX; Tencent NC), CondMDI / Wan-Move / MotionStream
(no license).

Refined statement: the genuinely sparse category is **permissive + 2025+ +
lightweight locomotion/generative runtime NNs**. But permissive runtime
options (tracking/driving + physics controllers) definitely exist.

### Deep search (2026-08-03) — believable runtime blending
Requirement: smooth blending among canned clips, physics-correct, context-aware
(crouch under a block, hug a wall), during gameplay. The systems that do this
are **Motion Matching and its learned/environment-aware variants**, which are
**permissive (MIT) and current**:
| Source | License | Fit |
|---|---|---|
| **Environment-aware Motion Matching (EMM)** | MIT, SIGGRAPH Asia 2025 | adapts to environment/obstacles/agents — the wall-hug/crouch case |
| **orangeduck/Motion-Matching** | MIT (code); dataset CC-BY-NC-ND | canonical Motion + Learned Motion Matching, training code, **wasm web demo** |
| **JLPM22/MotionMatching** | MIT | Unity MM core package (EMM builds on it) |
| **godot-motion-matching** | MIT | MM for Godot |
| **bevy_motion_matching** | Apache-2.0 | MM for Bevy/Rust |
| **CARL** (SIGGRAPH 2020) | MIT | physics-based RL character control |

Reframe: the earlier "non-ML" exclusion removed exactly the right tools here.
Motion Matching (regular + learned + environment-aware) is the permissive,
industry-standard runtime core for believable gameplay blending. For the
web/wasm target, orangeduck/Motion-Matching (MIT) is the most deployable
(working emscripten demo). EMM satisfies permissive + 2025+.

### Corrected verdict
Permissive **and** 2025+ is satisfied only by **generation** models (MARDM,
Anytop, Kimodo-code). There is **no lightweight permissive runtime animation
model from 2025+**. The permissive runtime physics options (KinPoly/UHC BSD-3,
DeepMimic MIT) exist but predate 2025; relaxed permissive non-commercial adds
AnimationGPT (MIT 2023, game combat AIGC tool) and MoMask (2024).

## Key sources
- MARDM (CVPR 2025); HY-Motion 1.0 (Tencent); MoMask (CVPR 2024); MotionLCM
  (ECCV 2024); Kimodo (NVIDIA).
- Motion Matching: UE5 docs; GPU-based MM for Crowds (SIGGRAPH Asia 2020).
- Learned Motion Matching: Holden, Ubisoft LaForge.
- DeepPhase / PFNN / MANN: Starke/Holden (work, Austria Center / AI4Animation).
- ASE (nv-tlabs), C·ASE (SIGGRAPH Asia 2023), MaskedMimic (xbpeng).
- MotionBricks (NVIDIA, SIGGRAPH 2026).
