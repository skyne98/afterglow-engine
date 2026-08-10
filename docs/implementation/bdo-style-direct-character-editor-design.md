# BDO-Style Direct-Manipulation Editor: Design

Date: 2026-08-02
Status: proposal (no engine or prototype code changed yet)

## 1. Goal

Add a direct-manipulation editing layer to the character-editor prototype in the
style of Black Desert Online (BDO), built on top of the existing slider panel
and body-zone picking.

The goal is **not** a free-form sculpt. It is a BDO-style direct-drag plus
explicit-controller hybrid:

- Direct drag gives a fast, visible change.
- Length/width/depth control bars give an accurate change.
- Drag and control bars stay synchronized on the same underlying value range.
- The existing complete slider panel remains the expert fallback.

The research basis is
`docs/research/direct-manipulation-character-creator-ux.md`, which the current
study refined against the official BDO guide and technical/modding sources.

## 2. Current prototype

Already present:

- Body-zone raycast picking with hover highlight and click-to-select overlay.
- Zone-to-category filtering of the slider panel (`filterMorphControls`).
- Bilateral left/right zone splitting.
- Continuous GPU morph application.
- Full slider panel with positive/negative control pairs.
- Hair-fit update triggered from structural morph changes.

Missing for BDO-style interaction:

- Explicit operation modes (Move / Rotate / Size).
- Length/width/depth control bars per mode.
- Direct drag that drives morph values (not just filtering).
- Drag-to-control-bar and control-bar-to-drag synchronization.
- Axis lock, pointer-record undo, and reset part/face/body.
- An authored hotspot map (see section 5, the main prerequisite).

## 3. Interaction model

One end-to-end operation:

1. Hover a body region -> local highlight.
2. Click the region -> lock selection, persistent highlight.
3. Choose an operation mode: Move, Rotate, or Size.
4. Drag on the region -> fast morph change; three control bars update.
5. Drag or edit the control bars -> accurate change; direct result stays live.
6. Rotate the character and re-check the profile.
7. Shift+drag -> one-axis lock (optional).
8. Release -> one bounded undo record.
9. Escape or cancel -> restore pointer-down values.
10. Reset Part / Reset Face / Reset Body where needed.

The drag path must never depend on frame-to-frame pointer deltas. All
displacement is accumulated from the pointer-down position and normalized by
viewport size and hotspot sensitivity.

## 4. Controller semantics

Three operation modes, each with three control bars. Axes are screen-relative
(as in BDO), not character-local.

| Mode | Bar group | Meaning |
|------|-----------|---------|
| Move | length, width, depth | translation of the region |
| Rotate | length, width, depth | rotation of the region |
| Size | length, width, depth | scaling of the region |

The three screen axes map to the screen-relative directions. Rotating the
character changes which local morph axes the bars drive, so the mapping is
recomputed from the camera basis at pointer-down.

## 5. Hotspot map and the no-authoring alternative

BDO-style direct manipulation requires a per-region mapping from gesture to
morph controls. Two methods can supply it.

### Method A: authored hotspot sidecar (baseline)

An artist writes a per-region table. This is the most reliable but the most
expensive to create and review. Each hotspot stores:

- Stable hotspot identifier and region/triangle set.
- Applicable operation modes (only show modes that have controls).
- For each mode axis: one or more (target name, sign, scale) entries.
- Sensitivity and permitted value range.
- Symmetry group (mirrored counterpart and sign).
- Reset group (which controls a Reset Part operation restores).

### Method B: computed hotspot map (no authoring)

The proxy already carries 689-691 morph targets, each a per-vertex delta field.
The map can be computed from that data.

1. **Vertex-signature clustering.** For each vertex, record how every morph
   moves it. Cluster vertices by spatial proximity AND shared morph response.
   Regions emerge from the data as "a contiguous patch mostly driven by a
   small set of morphs", not from an artist.
2. **Regional morph candidates.** For each cluster, keep the top-K morphs by
   mean displacement over its vertices. A drag works on this bounded candidate
   set resolved at pointer-down, never the full morph library.
3. **Direction-matching drag.** At pointer-down, on the selected region, pick
   the morph whose displacement best matches the drag direction in screen
   space (maximize the dot product between the camera-space drag delta and the
   per-morph motion). The gesture moves the character in the direction the
   user drags, choosing whichever morph produces that motion. This makes the
   drag view-relative and automatic, with no authored width/length/depth
   labels.
4. **Ambiguity detector.** Compute per-region a confidence score for how
   cleanly one dominant morph or axis explains the motion. Low-confidence
   regions (overlapping nose, mouth, eyes) automatically fall back to category
   filter plus slider instead of a wrong direct-drag. This replaces authored
   review with a machine-checkable gate and flags only the hard regions, which
   an artist may optionally spot-check.

Method B removes the manual authored sidecar for most regions. The critical
regions are handled by automatic fallback rather than by hand-writing a table.
The remaining optional manual step is spot-checking the flagged low-confidence
regions, not authoring a full map.

### Overlapping-region solutions

Where several morphs share a region (nose width, nose length, nostril flare,
and so on), the direction match may be ambiguous. These solutions reduce or
resolve it:

1. **Offline hard partition + overlap ledger.** Assign each vertex to the one
   morph with dominant displacement; keep a secondary ledger of other morphs
   that touch it. Drag drives only the winner. Overlap is removed at the data
   level.
2. **Orthogonalize the candidate set (PCA).** Per region, take the principal
   axes of its candidate morphs and drive orthogonal, non-overlapping control
   directions (one per PCA axis). Overlap is removed in control space.
3. **Committed gesture (first-touch lock).** Commit to one morph on the first
   dominant drag direction and hold that mapping for the whole drag. It never
   flips mid-drag.
4. **Deterministic priority tie-break.** Order morphs by a fixed priority
   (category order) and resolve ties by that order. Fully reproducible.
5. **Hysteresis on best-morph selection.** Keep a persistent per-region bound
   morph across the session; switch only when a challenger wins by a clear
   margin. Removes flicker between edits and camera moves.
6. **Mode-filtered candidate sets.** Each operation mode sees only its
   relevant subset (Move = translation, Rotate = rotational, Size =
   symmetric/scale). Each mode is far less ambiguous.
7. **Bounded regularized fit.** Solve a small least-squares fit over the
   candidates with a tiny regularization term that penalizes flips and large
   total adjustment. Natural multi-axis movement; minimal-adjustment and
   stable.
8. **Landmark-axis decomposition.** Drive the three bars from the region's
   geometric bounding box (own height/width/depth), so each axis maps to a
   canonical region axis that is stable and view-derived.
9. **Depth-axis resolution via camera orbit.** Only the depth axis is truly
   ambiguous; the two on-screen axes are usually unambiguous. Render the depth
   bar from the current profile and prompt an orbit for the confusing axis.
10. **Tiny geometric sub-zone masks (optional).** For the worst human-critical
    patches, add a small authored sub-zone (a few dozen records) bound to one
    morph. High reliability where it matters; reintroduces a little authored
    data.

**Recommended combination.** Use 2 (orthogonalize) + 3 (committed gesture) +
6 (mode-filtered candidates) as the base, with 9 (depth via orbit) for the
residual depth ambiguity and 5 (hysteresis) for stability. Reserve 10 only if
the flagged worst regions still feel wrong after real-user testing.

## 6. Axis mapping

Each hotspot axis entry maps a screen-relative drag delta to one or more morph
targets.

- Delta along axis A -> morph index I with sign s and scale g.
- Applied delta = s * g * (pointer displacement along A).
- A diagonal drag can affect two axes at once.
- Shift -> lock to the dominant axis (the axis with the largest |delta|).
- Each target value is clamped to [min, max].

The mapping table is resolved once at pointer-down. No morph-name lookup happens
during the drag.

## 7. Synchronization (single source of truth)

Morph values are the single source of truth. Both input paths write to the same
value:

- Direct drag writes the accumulated value range.
- Control bars write the same value range.
- The slider panel writes the same value range.

The UI is a view of that shared value. All three controls update from the same
source, so drag, bar, and slider can never disagree.

## 8. Undo and recovery

- `Ctrl+Z` and `Ctrl+Y` for undo and redo.
- Each drag (pointer-down to release) appends one bounded undo record.
- A record stores the touched morph values at pointer-down only (compact).
- A fixed-capacity stack replaces itself deterministically (no unbounded growth).
- Reset Part: restores the controls in the selected hotspot's reset group.
- Reset Face / Reset Body: restore face or body groups.
- Reset All: existing global reset.

Records must be allocation-free after warm-up (see section 10).

## 9. Symmetry and limits

- Symmetry is an explicit checkbox.
- When on, a structural drag also applies to the mirrored hotspot with the
  correct sign. The prototype already has separate left/right controls, so the
  symmetry group maps to the known counterpart.
- Expression and speech previews stay separate from structural editing. Provide
  a Neutral preview operation before structural face work.
- Feedback near a limit: resistance in the last 10% of the range, amber
  highlight near a limit, red highlight at a hard limit.

## 10. Performance and allocation

The drag path must follow the prototype memory rules:

- Allocate no objects, arrays, closures, or strings per frame.
- Reuse preallocated pointer and control records.
- Do no morph-name or hotspot lookup after pointer-down.
- Change only mapped target indexes.
- Coalesce pointer movement into at most one update per rendered frame.
- Keep hover work at a fixed rate.
- Keep region selection independent of the number of prior edits.
- Use a fixed-capacity undo stack with deterministic replacement.

Direct drag does NOT need a CPU proxy refit. It drives the same baked glTF morph
influences that the slider panel drives.

## 11. Files

Proposed new prototype files (all under `prototype/character-editor/`):

- `scripts/gen-hotspots.ts` - offline computed hotspot map from morph data
  (clustering + direction table + ambiguity scores). No authored JSON.
- `src/hotspots.ts` - computed map schema, load, and readable tables.
- `src/direct-drag.ts` - drag path, direction-matching, axis lock, pointer
  records, ambiguity fallback.
- `src/controller-bars.ts` - Move/Rotate/Size UI and the three bars.
- `src/edit-history.ts` - fixed-capacity undo/redo.
- `src/controller-bars.test.ts`, `src/direct-drag.test.ts`,
  `src/edit-history.test.ts` - unit and regression tests.

## 12. Implementation order

1. **[done] Computed hotspot map** (clustering + control-pairing + ambiguity
   gate). Implemented in `scripts/gen-hotspots.ts`; writes
   `public/character_{sex}.hotspots.json`; validated by
   `src/hotspots.test.ts`. No authored sidecar. Paired incr/decr morphs are
   reconciled as one control, and the ambiguity detector flags genuinely
   overlapping regions (eyes, ears, cheeks, chin) while leaving single-control
   regions clean.
   The merge threshold is 0.95, which produces **51 hotspots on the male body
   and 60 on the female** (33/34 single-control clean regions, 9/12 flagged
   overlapping, the rest mid-confidence).
2. **Controller bars** UI bound to the same morph indexes (readable, no drag).
3. **Direct drag** with direction-matching, axis lock, and pointer records.
4. **Synchronization** proof: drag, bars, and sliders read one source.
5. **Undo/recovery**: Ctrl+Z/Y, reset part/face/body.
6. **Symmetry and limit feedback**.
7. **Allocation hygiene** lint pass over the new hot paths.
8. **Acceptance** and regression tests; update book/docs if public API changes.

## 13. Acceptance

- Hover and click select the same zones as today.
- Direct drag changes the mapped morphs on the same frame.
- Drag, control bars, and sliders never disagree.
- Shift locks to one axis.
- Undo/redo restores exact prior values.
- Reset Part restores only the selected group.
- The ambiguity detector flags the overlapping nose/mouth/eye regions and they
  fall back to category filter plus slider (no authored map).
- No per-frame allocation in the drag, hover, or undo paths (lint + test).
- Structural morphs still trigger the hair-fit update.
- All existing prototype tests still pass.
- A fixed hotspot identity survives camera rotation and zoom.

## 14. Out of scope

- Free-form sculpt / arbitrary topology editing (BDO is not that either).
- SpringChain or long-hair secondary motion.
- Promotion to engine code; this remains prototype-scoped.
- Changing the MPFB macro composition or bake format.
