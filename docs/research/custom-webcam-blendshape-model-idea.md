# Custom webcam→ARKit-blendshape model idea: iPhone TrueDepth teacher + webcam student

**Status:** idea — recorded for later decisions, not a plan
**Date:** 2026-08-10
**Scope:** train a private webcam-to-52-blendshape regression model for the
character editor and engine, using an iPhone TrueDepth capture as the
ground-truth teacher. This is a candidate quality path for webcam face
animation, not a committed design.

Related documents:

- [`../implementation/character-editor-prototype.md`](../implementation/character-editor-prototype.md)
- [`open-license-character-rig-in-game-editor.md`](open-license-character-rig-in-game-editor.md)
- [`direct-manipulation-character-creator-ux.md`](direct-manipulation-character-creator-ux.md)
- [`threejs-webgpu-stability.md`](threejs-webgpu-stability.md)

## The idea in one paragraph

Record the same face at the same time with an iPhone (TrueDepth camera, which
emits the true ARKit 52 blendshape coefficients per frame) and a normal webcam.
Synchronize the two streams frame-by-frame, augment the webcam frames to
emulate many webcam qualities (resolution, blur, noise, JPEG artifacts, color,
exposure, small viewpoint changes), and train a small regression model from
webcam image (or landmarks) to the 52 ARKit coefficients. The result is a
full-52 webcam tracker — including `tongueOut`, `cheekPuff`, `jawLaterals`,
and `mouthDimples` — that runs in the browser, with no iPhone required at
runtime. This is the same data-collection pattern used by the MDPI emotion
recognition study (custom iOS app + synchronized video capture), and it is the
only practical way to beat the released MediaPipe blendshape model, which
cannot signal those shapes from landmarks.

## Why this exists

The character editor drives a rig with all 52 ARKit morphs by exact name. The
current webcam tracker is MediaPipe Face Landmarker (tasks-vision, blendshape
model V2: MLP-Mixer on 146 FaceMesh landmarks). It is the best released
free/in-browser option, but seven shapes are effectively dead
(`jawForward`, `jawLeft`, `jawRight`, `mouthDimpleLeft`, `mouthDimpleRight`,
`cheekPuff`, `tongueOut`) — the model never moves them, because a
landmark-based regressor cannot see the tongue or cheek inflation.

No better released model exists. The Google GHUM V3 blendshape model (Sept
2023 paper) is unreleased; MediaPipe issue #5329 (the most-upvoted blendshape
issue) is open since 2024 with no response. NVIDIA Maxine AR SDK covers the
full set but requires an RTX GPU and a desktop/CUDA deployment. VTuber
community consensus is the same ladder: iPhone ARKit > NVIDIA RTX (Maxine) >
MediaPipe > OpenSeeFace. All free webcam solutions are MediaPipe underneath.

A private model trained on TrueDepth ground truth is the only open path to
full-52 quality on a plain webcam.

## Core method

### 1. Capture (the teacher)

Build a small iOS app (Swift + ARKit + video recording) that records, per
frame with timestamps:

- the TrueDepth front-camera RGB video (60 fps),
- the 52 ARKit blendshape coefficients (60 fps),
- head pose (rotation + translation).

The MDPI study (reference below) did exactly this with Swift + ARVideoKit.
Live Link Face streams but does not record synchronized webcam + coefficients,
so a custom app is required.

A second machine records the same face with a normal webcam (30–60 fps). Mount
the phone near the webcam; the phone blocks part of the view, so accept a
small viewpoint offset and cover it with augmentation.

### 2. Synchronization

- Clap or flash at session start to find the offset between streams.
- Interpolate the 60 fps ARKit labels onto the webcam frame timestamps.
- Verify with per-shape smoothness and a lip-sync sanity check.

### 3. Preprocessing and augmentation (the student domain)

Close the iPhone↔webcam gap by degrading the iPhone frames (or by training on
the real webcam frames directly):

- downsample to webcam resolution,
- Gaussian blur / defocus,
- sensor noise,
- JPEG compression artifacts,
- color / white-balance / exposure shifts,
- small random affine and viewpoint transforms,
- brightness and contrast variation.

### 4. Training

Two model shapes:

- **Landmark → 52 (MLP):** cheapest, most data-efficient, but capped by
  landmark quality — inherits the tongue/cheek blindness.
- **Image → 52 (small CNN):** can see the tongue and cheek pixels, so it is
  the only variant that can reach the full 52. Recommended target.

Train with the same 52-name vocabulary the rig already uses, so deployment is
a name-matched write to `morphTargetInfluences` (the face-tracker module
already does this). Export to ONNX and run with `onnxruntime-web` in the same
browser path.

### 5. Evaluation

- Held-out subjects (never in training), per-shape error vs the ARKit
  teacher, and a head-to-head against MediaPipe on the same webcam footage.
- Deployment gate: the model must beat MediaPipe on the dead shapes without
  regressing the live ones.

## Alternative: synthetic training with the character rig

Google's GHUM paper generated 2M synthetic (landmarks → known blendshape
weights) pairs by rendering a parametric face model with random coefficients
and projecting to 2D. The same trick is possible with this character rig:
render the character with known 52 weights, track landmarks on the render,
train a regressor. No iPhone needed, unlimited data. Risk: the rendered
character's landmark distribution may not transfer to real human faces.

## Risks and open questions

1. **Single-face overfitting.** Data from one person fits that face shape and
   idiosyncratic expressions. Real quality needs 5–10+ subjects × an
   expression protocol (each shape in isolation, combinations, natural speech,
   fast/slow motion) × varied lighting and backgrounds. This is the main cost.
2. **Viewpoint mismatch.** The phone must sit near the webcam; a small angle
   offset is inevitable. Mitigate with augmentation and multiple sessions.
3. **Sync accuracy.** Frame-accurate label interpolation is the fiddly
   engineering part.
4. **Tongue and cheek are idiosyncratic.** Even with TrueDepth labels, these
   shapes vary strongly between people; the model must see many examples.
5. **Legal/collection.** Recording faces of subjects requires consent
   documentation if the data or model leaves the project.

## Decision points (before implementation)

- Model shape: landmark MLP (cheap, capped) vs image CNN (full-52 target).
- Capture breadth: single-user demo vs multi-subject dataset.
- Where training runs: rented cloud GPU (the project has a vast.ai workflow)
  vs local NVIDIA hardware.
- Deployment format: ONNX + onnxruntime-web vs TensorFlow.js.

## Recommended first step

Validate the pipeline before weeks of data collection: capture 2–3 subjects,
20–30 minutes each, train a small landmark→52 model, and compare against
MediaPipe on a held-out person. This proves capture, sync, training, and
deployment end-to-end at low cost. Only then invest in the full dataset and
the image-CNN variant.

## Sources

- MediaPipe Face Landmarker blendshape model card V2 (Apache 2.0, MLP-Mixer,
  146 landmarks): `storage.googleapis.com/mediapipe-assets/Model Card
  Blendshape V2.pdf`
- MediaPipe issue #5329 — GHUM V3 not implemented (open, 12 comments, most
  upvoted blendshape issue): `github.com/google-ai-edge/mediapipe/issues/5329`
- Blendshapes GHUM paper (Google, Sept 2023) — synthetic 2M pair training
  approach: `arxiv.org/abs/2309.05782`
- MDPI study — custom iOS app with ARKit + ARVideoKit for synchronized
  52-coefficient video capture: "Real-Time Emotion Recognition Performance of
  Mobile Devices Using Apple's ARKit" (Sensors 2026, 26(3):1060)
- VTuber community quality consensus: r/vtubertech threads on webcam vs iPhone
  vs NVIDIA RTX ARKit tracking
- HuggingFace/GitHub survey (2026-08-10): no released model beats MediaPipe's
  blendshape model; all alternatives are MediaPipe-based or landmark heuristics
