# Direct-Manipulation Character Creator UX: The Sims 4 and Black Desert Online

Date: 2026-08-02

## Research question

How do the mouse-driven character editors in *The Sims 4* and *Black Desert
Online* work, and which parts should Afterglow use?

## Result

The two editors use different direct-manipulation models:

- **The Sims 4** makes the character the primary control surface. A hover finds
  a data-driven hotspot. A drag changes two mapped modifiers. The active
  hotspot and its meaning can change with camera angle and detail level.
- **Black Desert Online (BDO)** uses a hybrid model. The user selects a visible
  body region and an explicit controller type. Direct drag gives a fast change,
  while three related control bars give an accurate change.

The Sims 4 has the lower initial UI load. BDO gives better control for a large
morph library. For Afterglow, the recommended default is a **BDO-style hybrid
with Sims-style feedback**:

1. Keep hotspot identity stable at all zoom levels.
2. Highlight the region below the cursor.
3. Select one explicit operation before drag.
4. Mirror each drag value in an accurate control bar.
5. Supply undo, redo, reset-part, symmetry, and limit feedback.
6. Keep the current complete slider list as the expert fallback.

This design also avoids one important part of an active EA patent claim. A
legal review is still necessary before a production release.

## Evidence and limits

The research used:

- The Maxis GDC 2015 presentation and its video transcript.
- The active Electronic Arts direct-manipulation patent.
- The current EA Create A Sim guide and its linked beginner video.
- The current official BDO customization guide and its UI images.
- A detailed 2015 BDO face-controller video for direct-use observations.
- Ben Shneiderman's original direct-manipulation paper.

The proprietary game clients were not installed for this research. The BDO
video shows an older editor that had no general undo button. The current
official BDO guide shows a later, part-grouped edit history with undo and redo.
The report identifies this difference where applicable.

## HCI basis

Shneiderman defines the important direct-manipulation properties as:

- Continuous representation of the object of interest.
- Physical actions instead of complex command syntax.
- Rapid, incremental, reversible operations.
- Immediate visible results.
- A layered learning path from a small novice set to expert functions.

These properties explain the appeal of both editors. The user does not first
translate “make the nose wider” into a slider name. The user points at the nose
and moves it. Immediate deformation closes the action-to-result loop.

Direct manipulation does not automatically make an interface good. Shneiderman
also identifies risks:

- An icon can be unclear.
- A visual model can suggest an operation that is not available.
- Too much visual information can cause more confusion.
- The displayed representation and its permitted operations need user tests.

The Sims 4 reduces these risks with local highlights, cursor changes, limits,
and progressive detail. BDO reduces them with explicit controller types,
visible control bars, reset operations, and persistent selection.

# The Sims 4

## Design goal

Maxis states two primary goals: accessible and fun, and deep customization. The
supporting goals were a minimalist UI, direct manipulation, smart
randomization, and fast response.

The central UX decision was to make the Sim the start of most operations. The
user clicks, pulls, and pushes the visible body instead of searching a large
slider tree.

## Interaction model

A typical operation is:

1. Move the cursor over the character.
2. The system identifies a hotspot below the cursor.
3. The hotspot receives a local highlight.
4. The cursor changes to show a move, rotate, or scale operation.
5. Press and hold the primary mouse button.
6. Drag in X, Y, or both directions.
7. The mapped modifiers change on the same frame.
8. Release the button to complete the operation.

The GDC nose example maps:

- Drag left or right to make the nose narrow or wide.
- Drag up or down to move the nose up or down.
- A diagonal drag changes both values at the same time.

The character remains visible during all changes. There is no modal dialog
between selection and result.

## Context changes the meaning

The same screen-space drag does not always have the same meaning. The hotspot
resource uses two context values.

### Camera-angle sectors

Maxis calls camera-angle sectors **pie wedges**. For example:

- A horizontal nose drag in front view changes width.
- A horizontal nose drag in profile view changes length.

This makes the drag agree with the visible silhouette. It also means that a
control can be unavailable until the user finds the applicable view.

### Detail levels

The system has top, macro, and micro levels:

- **Top:** the complete body, with head size, shoulders, hips, legs, and other
  large areas.
- **Macro:** the head and large face regions, such as eyes, nose, lips, and
  ears.
- **Micro:** small face details, such as pupils and eyelids.

The GDC presentation says that zoom changes the active region channel. The
current EA-linked guide also shows double-click entry into detail-edit mode.
Thus, the shipped UX uses both camera distance and an explicit detail-edit
transition as progressive disclosure.

## Hotspot data model

The Sims 4 system is data-driven. Each hotspot stores:

- Region identifier.
- X-axis and Y-axis modifiers.
- Top, macro, or micro detail level.
- Highlight texture.
- Applicable camera pie wedges.
- Cursor type: move, rotate, or scale.

A hotspot modifier can use a blend shape, bone pose, or deformation map. The
interaction layer does not need to know which deformation type is below it.
The body system had 18 direct touch points in the GDC implementation.

## Picking

Maxis uses a color region map that shares the character UV layout. Each
pickable region has a unique color value. A render-based pick reads the value
below the cursor.

The GDC sample encodes:

- Top region identifier in one RGB channel.
- Macro region identifier in a second channel.
- Micro region identifier in a third channel.
- Body type in alpha.

The result supplies a region identifier. Region, detail mode, and view sector
then select the hotspot resource.

This is more reliable than selecting a morph from geometric distance alone. An
artist controls the complete selectable area, including small features and
regions with overlapping visual influence.

## Feedback

The Sims 4 uses several related feedback signals:

- A local highlight shows the active region.
- A directional cursor shows the permitted movement.
- Deformation updates continuously during drag.
- Drag resistance increases near a limit.
- The highlight becomes red at the limit.
- Pointer sensitivity is scaled in relation to the area of the modifier.

The limit behavior is important. A hard clamp without resistance makes the
model stop while the cursor continues to move. Resistance gives the user a
physical indication before the hard limit.

## Symmetry

Maxis used vertical body symmetry to reduce the region-map data size. This does
not mean that all edits are permanently mirrored. The editor still supplies
local face adjustment where applicable.

## Presets, randomization, and recovery

The editor starts from face archetypes and then adds modifiers. This gives a
fast coarse-to-fine path.

Smart randomization uses tags such as age, gender, outfit type, archetype,
style, and color palette. It selects a coherent group, not an independent
random value for each option. If no complete match exists, it removes lower
priority tags until it finds one.

The current PC editor supports `Ctrl+Z` and `Ctrl+Y` for undo and redo. This is
a necessary part of the direct-manipulation model because experimentation must
be low risk.

## Performance

Maxis identifies fast response as a core UX property. Its implementation:

- Preloads part instances by age, gender, and type.
- Loads heavier resources on demand.
- Preloads high-reuse deformation maps.
- Applies deformation maps on the CPU.
- Uses compressed textures and run-length encoding.

The important UX rule is not the old CPU implementation. The important rule is
that pointer movement must change the visible body without a delayed apply
step.

## Why it feels intuitive

- The model is both the object and the main control.
- The result is visible during the gesture.
- X and Y movement agree with the visible view.
- Detail appears gradually instead of in one large panel.
- The user can make and reverse small experiments quickly.
- Presets give a useful starting state.
- Highlights and cursor changes give local instruction.

## Weak points

- Hidden hotspots are not fully discoverable. The current EA beginner guide
  specifically explains missed controls such as shoe size and posture.
- Angle-dependent controls can make a function appear missing.
- Zoom-dependent region changes can surprise a user when the cursor stays over
  the same visual area.
- There is no always-visible numeric value for accurate reproduction.
- A large direct region can hide which internal modifiers a diagonal drag
  changes.
- Small overlapping face regions need careful authoring and extensive tests.
- A pure direct model is less suitable for hundreds of named expert controls.

# Black Desert Online

## Editor structure

BDO places the character in the center and uses an accordion panel for major
systems:

- Hair.
- Face.
- Body.
- Voice.
- Makeup, tattoos, wrinkles, eye details, and related appearance data.

The shape tools use direct region selection. The selected region remains
visible, and the left panel shows the applicable controller.

## Region feedback

In the official face-shape image:

- All adjustable face regions have visible boundaries.
- The selected forehead region uses a bright cyan fill.
- The panel shows the three controller types and their control bars.
- A checkbox can hide or show adjustable parts.

The body image uses the same pattern. The selected abdomen region is cyan, and
the shape controller is visible at the same time.

This is less minimal than The Sims 4. It has better operation visibility.

## Controller model

BDO separates operations into explicit controller types:

| Controller | Purpose | Accurate controls |
|---|---|---|
| Move | Change feature position | Three directional control bars |
| Rotate | Change feature orientation | Three rotational control bars |
| Size | Change feature dimensions | Three dimensional control bars |

The official guide calls the accurate controls length, width, and depth. It
also says that these directions are based on the screen view, not the
character's local direction.

A typical operation is:

1. Open Face Shape or Body Shape.
2. Select a highlighted body region.
3. Select Move, Rotate, or Size.
4. Drag the selected region for a fast visible change.
5. Use the three control bars for an accurate change.
6. Rotate the character and check the result from another view.
7. Reset the selected part if necessary.

The 2015 face-controller video shows that direct drag updates the control bars.
It recommends the control bars for more accurate work. It also recommends
frequent profile checks.

## Symmetry and resets

The face controller supplies:

- A **Symmetrical** checkbox.
- **Reset Part**.
- **Reset All**.

Symmetry is an explicit state rather than an assumed permanent rule. This is a
good match for an editor that has separate left and right controls.

The older 2015 video had no general undo and therefore depended heavily on
Reset Part. The current official guide documents a newer edit history that
saves changes by part and supplies undo and redo.

## Camera and inspection

The current official UI image gives these camera controls:

- Right mouse drag: rotate character.
- Mouse wheel: zoom.
- Middle mouse drag: move camera.

Additional inspection tools include:

- Front-view or gaze lock, so the face and eyes do not follow the cursor.
- UI hide.
- Screenshot and screenshot-folder access.
- Weather or background selection.
- Character clothing and action selection.
- A temporary clear-hair option in the older face workflow.

These are not decorative extras. A realistic face can look different with a
new light, pose, camera view, hair silhouette, or outfit. BDO makes inspection
part of the editor instead of an external test.

## Coarse-to-fine path

BDO also starts from a preset:

1. Select a base face or hair type.
2. Change local shape regions.
3. Add skin, eye, makeup, wrinkle, and material details.
4. Check the result in different views and conditions.

Hair uses the same hybrid idea. The user selects a hairstyle, drags adjustable
hair sections, and can also change length and curl through control bars.
Some hairstyles have smaller permitted ranges.

## Persistence and sharing

BDO has stronger long-session support than The Sims 4:

- Part-grouped edit history.
- Up to ten temporary saves.
- Save and load files.
- Restore default.
- Apply a popular preset.
- Beauty Album sharing.
- Screenshots.

This matters because a high-detail editor can take a long time. A user needs
checkpoints, comparison, and reuse, not only one linear undo stack.

## Why it feels intuitive

- Regions and region boundaries are visible.
- The selected region has persistent feedback.
- The operation mode is explicit.
- Direct drag and accurate controls remain synchronized.
- Symmetry is visible and controllable.
- Reset Part is close to the active operation.
- Camera, light, clothing, and hair inspection are integrated.

## Weak points

- Region selection, controller selection, and axis adjustment add more steps.
- The full panel and all-region overlay add visual load.
- Screen-relative length, width, and depth can become unclear after rotation.
- Direct drag is less accurate than the control bars.
- Users must inspect the profile frequently to prevent bad depth.
- Detailed subdivision into many face pieces can be difficult for a novice.
- Limits vary by class, base face, hair type, and sex.
- The old workflow's lack of undo made experimentation risky. The current edit
  history corrects this issue.

# Direct comparison

| Property | The Sims 4 | Black Desert Online |
|---|---|---|
| Primary model | Character is the control | Character plus explicit controller |
| Hover | One local hotspot highlight | Optional full adjustable-region overlay |
| Selection | Hotspot from region, view, and level | Explicit region selection |
| Drag meaning | Data-driven X and Y modifiers | Selected Move, Rotate, or Size controller |
| Fine adjustment | No primary numeric slider UI | Three synchronized control bars |
| View dependence | Pie wedge selects different mapping | Screen-relative control directions |
| Zoom dependence | Top, macro, and micro hotspots | Zoom is mainly inspection |
| Symmetry | Strong default symmetry | Explicit symmetry checkbox |
| Limit feedback | Resistance and red highlight | Bar limits and constrained ranges |
| Recovery | Undo and redo | Undo, redo, reset part, reset all |
| Presets | Archetypes and coherent randomization | Base types and popular shared presets |
| Long-session support | Gallery and households | Temporary saves, files, album, history |
| Initial learning | Lower | Higher |
| Expert accuracy | Lower | Higher |
| Visual UI load | Lower | Higher |

The important finding is that BDO is not only a cursor-drag system. It is a
**direct-drag plus explicit-control system**. This hybrid is why it can expose
more detailed control without making every drag ambiguous.

# Recommendation for Afterglow

## Recommended interaction

Use this sequence:

1. Start from a body or face preset.
2. Hover a stable authored hotspot.
3. Show a local highlight and a directional cursor.
4. Click to lock the hotspot.
5. Show a small operation control near the panel or pointer.
6. Drag for a fast change.
7. Mirror the result in the existing accurate control list.
8. Use `Shift` for low sensitivity or an explicit one-axis lock.
9. Release to create one undo record.
10. Use Reset Part, Reset Face, or Reset All when necessary.

The camera must not change hotspot identity. Zoom can make a region easier to
see, but it must not silently select a different region. An explicit
**Body / Face / Detail** mode can change available controls if necessary.

## Recommended operation modes

The exact BDO labels do not fit every MakeHuman morph. Use modes that describe
the available target data:

- **Shape:** width, height, and depth pairs.
- **Position:** up/down, left/right, and forward/back pairs.
- **Angle:** tilt, rotation, slant, and orientation pairs.
- **Detail:** local one-sided targets that do not form a spatial pair.

Only show a mode when the selected hotspot has applicable controls. Do not show
an inactive mode.

## Symmetry

Use symmetry by default for structural face and body changes. Supply a visible
symmetry switch. When symmetry is off, the user can select left and right
regions separately. Existing bilateral morph controls already support this.

Expression and speech previews should remain separate from structural editing.
An expression changes the inspection state, while a structural drag changes
the saved body. Supply a clear **Neutral preview** operation before structural
face work.

## Feedback

Use all of these signals together:

- Local hover highlight.
- Persistent selected-region highlight.
- Cursor that shows the active axes.
- Continuous deformation in the next rendered frame.
- Current values in the control panel.
- Resistance in the last 10% of a permitted range.
- Amber highlight near a limit.
- Red highlight at a hard limit.
- A small axis-lock indication.

Do not move the system cursor more slowly. Instead, scale the deformation
result. Pointer position must remain under operating-system control.

## Camera

Recommended desktop controls:

- Primary drag on a region: edit.
- Secondary drag: rotate character.
- Mouse wheel: zoom.
- Middle drag: pan.
- Double click a region: frame that region.
- `F`: frame selected region.
- A front/profile/rear snap control for accurate checks.

During an edit drag, capture the pointer and disable camera orbit until release
or cancel. `Escape` must restore the values from pointer-down.

## Accurate controls and recovery

Keep the complete current morph panel. It must show the same values that direct
drag changes. This gives:

- Direct manipulation for discovery and speed.
- Control bars for accuracy.
- Named controls for expert access.
- A way to identify what a drag changed.

Add:

- `Ctrl+Z` and `Ctrl+Y`.
- Reset selected operation.
- Reset selected region.
- Reset face.
- Reset body.
- Temporary snapshots with visual thumbnails.
- Before/after press-and-hold comparison.

One pointer-down through pointer-up sequence must be one undo transaction.
Pointer-move events must not each create a history item.

## Data required by the current prototype

The current prototype already has:

- A static hit mesh.
- Morph-name to glTF-index sidecars.
- Logical positive/negative control pairs.
- Separate left and right controls.
- Morph-derived category zones.
- Continuous GPU morph updates.

It does not yet have an accurate direct-manipulation map. The current zone map
finds broad categories from morph displacement. It cannot state which X and Y
gesture must change which exact controls.

Add a generated and reviewed hotspot sidecar with:

- Stable hotspot identifier.
- Triangle or region identifier.
- Label and highlight data.
- Applicable operation modes.
- X, Y, and optional wheel control mappings.
- Target names and signs.
- Sensitivity and permitted value range.
- Symmetry group.
- Reset group.

Generate an initial map from morph displacement, but require an authored review.
Nose, mouth, eye, finger, genital, and overlapping face regions are too
important for an automatic winner-only map.

## Pointer implementation

For each drag:

1. Record control values and pointer position at pointer-down.
2. Lock the hotspot and operation mapping.
3. Use accumulated displacement from the start, not frame-to-frame deltas.
4. Normalize displacement by viewport size and hotspot sensitivity.
5. Apply axis lock before the target mapping.
6. Clamp values and apply end resistance.
7. Update at most once per rendered frame.
8. On cancel, restore the recorded values.
9. On release, append one bounded undo record.

This method prevents event-rate dependence and accumulated numeric drift.

## Performance and allocation

The drag path must:

- Allocate no objects, arrays, closures, or strings.
- Use preallocated pointer and control records.
- Do no morph-name lookup after pointer-down.
- Change only mapped target indexes.
- Coalesce pointer movement into one frame update.
- Keep hover work at a fixed rate.
- Keep region selection independent of the number of prior edits.

The direct drag does not need a CPU proxy refit. It changes the same baked glTF
morph influences that the slider UI changes.

## Usability acceptance tests

Test with persons who did not use the current editor.

Minimum tasks:

1. Make the nose wider.
2. Move the nose down.
3. Increase nose depth.
4. Make only the left eye larger.
5. Make both shoulders wider.
6. Undo one complete drag.
7. Reset only the nose.
8. Find an accurate numeric or control-bar value.
9. Check the face in profile and return to front view.
10. Save and restore a temporary version.

Record:

- Time to first successful drag.
- Time for each task.
- Incorrect-region selections.
- Accidental camera movement.
- Undo and reset use.
- Number of instructions requested.
- Final value error for an accuracy task.
- User preference between direct drag, hybrid, and sliders.

Acceptance gates:

- A first-time user finds one body drag without instructions.
- The visible model changes within one displayed frame.
- Every drag is reversible.
- Zoom does not change hotspot identity.
- Camera movement never starts during a selected edit drag.
- The same start state and pointer displacement give the same result.
- All values stay finite and inside validated body bounds.

# Patent and product decision

Google Patents lists **US 10,275,947 B2, “Modifying a simulated character by
direct manipulation,”** as active, with an anticipated expiration date of
2035-03-31. Electronic Arts is the current assignee.

Independent claim 1 includes this combination:

- Identify current zoom level.
- Identify cursor location on a video-game character image.
- Select between portions associated with different zoom levels.
- Modify the selected portion in at least two directions from cursor movement.

Independent claim 15 also includes viewing-angle-based action selection. The
description covers region maps, highlights, cursor types, limit feedback,
view-angle sectors, zoom levels, and X/Y drag modifiers.

This report is not legal advice. Before production implementation, the user
must select one path:

1. **Recommended:** request patent review and implement the fixed-hotspot,
   explicit-operation hybrid described above.
2. Request a license or clearance for an exact Sims-style implementation.
3. Keep slider-only structural editing until the patent expires or counsel
   approves another design.

A keyword search found no directly applicable Pearl Abyss patent for the BDO
controller. This is not a freedom-to-operate search and is not legal clearance.

# Sources

Primary sources:

- Sri Nair, Maxis/EA, **Innovations in The Sims 4 Character Creator**, GDC 2015:
  <https://media.gdcvault.com/gdc2015/presentations/Nair_Sri_InnovationsInSims4CharacterCreator_GDC15.pdf>
- GDC Vault session page:
  <https://www.gdcvault.com/play/1022085/Innovations-in-The-Sims-4>
- GDC video transcript source:
  <https://www.youtube.com/watch?v=wt_4wJJNCIE>
- Electronic Arts patent US 10,275,947 B2:
  <https://patents.google.com/patent/US10275947B2/en>
- EA Create A Sim new-player page:
  <https://www.ea.com/games/the-sims/the-sims-4/new-player-hub/create-a-sim>
- EA-linked current beginner video:
  <https://www.youtube.com/watch?v=H46ryQJ0mIw>
- Black Desert official Adventurer's Guide, **Customization**:
  <https://www.naeu.playblackdesert.com/en-US/Wiki?wikiNo=5>
- Ben Shneiderman, **Direct Manipulation: A Step Beyond Programming
  Languages**, 1983:
  <https://www.cs.umd.edu/users/ben/papers/Shneiderman1983Direct.pdf>

Secondary practical source:

- **Black Desert Online: Intro to Face Sliders**, 2015:
  <https://www.youtube.com/watch?v=4a8AteZXbo0>

The secondary video is used only for observed operation order, direct-drag to
control-bar synchronization, profile checking, and the historical absence of a
general undo button.
