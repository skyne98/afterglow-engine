# Native shell input: Chromium comparison

## Scope

This source inspection uses Chromium commit `d4dcd6494fe4efe1ec5d6494b8dd51350d2c4a2e`.
It examines input dispatch and text controls, not all Chromium code.
The native shell does not yet have evidence of equivalent input quality.

## Source references

All paths below are under `third_party/blink/renderer/` at the specified commit.
The source URL prefix is:

`https://chromium.googlesource.com/chromium/src/+/d4dcd6494fe4efe1ec5d6494b8dd51350d2c4a2e/third_party/blink/renderer/`

- `core/input/keyboard_event_manager.cc:330-410`: dispatch the key event first. Cancellation stops subsequent processing. Recheck the focus after handlers.
- `core/input/pointer_event_manager.cc:1190-1284`: cancellation of primary pointerdown suppresses compatibility mouse events, not the separate click decision. Release capture before the click dispatch.
- `core/input/mouse_event_manager.cc:327-374`: capture controls the click target. Otherwise use the common ancestor of the press and release targets. Secondary buttons produce `auxclick`.
- `core/editing/selection_controller.cc:1140-1220`: selection starts only after permitted mouse input. Single, double, and triple clicks have different selection actions. Drag selection retains its start position.
- `core/html/forms/text_control_element.cc:391-448`: select-all restores the cached selection. A user value change produces `change` on focus loss, not on each key.
- `core/editing/editor.cc:340-412`: edit actions dispatch cancellable `beforeinput`. Handlers can remove the target or frame, so the editor checks them again before mutation.

## Initial shell defects

- The JavaScript bridge sends keyboard events but has no text-editing default action.
- Native pointer state changes only hover and active state. It does not move the text caret or extend text selection.
- The existing Blitz/Parley text editor already supplies caret geometry, word movement, deletion, and text selection.
- Pointerdown cancellation currently suppresses click as well as compatibility mouse events.
- The bridge supplies synthetic single-event coalescing, not retained OS samples.
- Winit keyboard dispatch does not connect IME composition to text controls.

## Menu failure

LinkeDOM nodes initially returned `[object Object]` from `Object.prototype.toString`.
Vue therefore put nodes inside reactive proxies. A proxy had a different native ID from its original node.
Menu startup then failed with `native node 1563 is not connected`. Proxy-based tree queries also failed during Escape handling.
The shell now gives DOM interface prototypes their type tags. A regression uses Vue and checks that reactive conversion retains node identity.
This correction applies to all DOM consumers, not only the paint menu.

Two additional defects prevented correct menu geometry:
- LinkeDOM style-property assignments changed the attribute storage directly, without mutation records. Native style therefore retained the initial offscreen transform.
- Blitz client rectangles omitted CSS transforms. Floating menus use a translated parent, so their reported rectangles stayed at the origin.

The local style patch uses the attribute setter. Client geometry now includes the transform chain without changes to layout offsets.

A native regression also reproduced a select popup with zero stacking bounds on its initial layout.
The stacking data used geometry from before layout, so the hit test selected the canvas below the popup.
The repair updates stacking positions and transformed bounds after layout, within the existing transform traversal.
The regression covers the initial hit and a subsequent translated popup.

## Accepted input policy

The user selected these policies for the remaining input work:

- Native application scripts may read and write the text clipboard without a user action.
- Text undo retains at most 512 edits per control and 32 MiB per document. Capacity removes the oldest history, not current text.
- Pen input must support Linux (X11 and Wayland), Windows, and macOS. A prototype must establish the backend before integration.
- The user has a tablet and physical latency equipment. Device models and access instructions are still necessary for physical checks.

Clipboard errors must not delete selected text. Platform support needs separate evidence for each platform.

## Acceptance checks

The repair must reuse the existing native text editor and keep application event cancellation effective.
Checks must cover displayed text and selection, not only JavaScript property values.

- Text and numeric fields: caret placement, drag selection, replacement, deletion, arrows, Home/End, and select-all.
- Focus and events: Tab, Shift+Tab, readonly/disabled controls, `beforeinput`, `input`, and commit-time `change`.
- Menus: pointer and keyboard opening, item activation, focus restoration, Escape, and outside clicks.
- Pointer dispatch: capture, cancellation, button masks, release outside the original target, and focus loss.
- Additional quality gates: clipboard, undo/redo, IME, Unicode, touch, pen pressure/tilt, OS event timestamps, sample retention, and measured input-to-pixel latency.

Passing a short paint probe does not establish these additional quality gates.
Artist input quality needs physical mouse and tablet measurements in addition to unit tests.
