# LinkeDOM + Blitz Atomic Integration Contract

The complete repair and atomic-cutover work plan is in
[`BLITZ_LINKEDOM_COMPLETE_FIX_PLAN.md`](BLITZ_LINKEDOM_COMPLETE_FIX_PLAN.md).

## Delivery model

The integration is one atomic implementation followed by one validation pass. There is no project feature flag, runtime switch, legacy fallback, example-specific behavior, or client/addon modification.

## Architecture

- LinkeDOM remains the sole JavaScript-facing DOM and owns node identity, structure, attributes, text, properties, selectors, listeners, and event dispatch.
- Blitz is authoritative for CSS cascade, computed style, layout, hit testing, scrolling/focus render state, paint order, and final page composition.
- LinkeDOM nodes receive out-of-band IDs held in a `WeakMap`. Structured node records (including text and namespaces) are reconciled by ID into one long-lived Blitz document whenever synchronous layout is forced. No transport metadata enters HTML attributes or selectors. MutationObserver records mark the tree dirty; correctness does not depend on interpreting individual records.
- Blitz-derived metrics and native/default actions flow back through normal LinkeDOM APIs and events. Pointer, compatibility-mouse, wheel, keyboard, focus, form-control, and scroll actions dispatch JavaScript events and respect `preventDefault()` before committing cancelable defaults.
- `elementFromPoint()` and `elementsFromPoint()` use Blitz paint-order hit tests. ResizeObserver and IntersectionObserver are microtask-delivered from resolved Blitz geometry, while matchMedia delegates parsing and evaluation to the document's Stylo device.

## Implementation

1. Add exact pinned Blitz/AnyRender dependencies and a reproducible Python-enabled build environment for Stylo.
2. Add native browser modules for protocol, document reconciliation, styles/resources, layout, interaction, and composition.
3. Add a JS bridge that assigns private IDs, snapshots the tree, tracks mutation epochs, flushes before layout reads, binds canvases, and exposes standard browser geometry/computed-style APIs.
4. Load linked and embedded CSS through the existing deterministic resource system before module code observes layout; include dynamic CSS/images/fonts in readiness accounting.
5. Replace all hand-written rectangle, client-size, CSS-unit, computed-style, and canvas-order approximations with Blitz results.
6. Preserve canvas backing size separately from CSS size. At capture, reconcile the final post-clean-page tree, read GPU/Canvas2D RGBA, inject it into matching Blitz nodes, and paint one complete opaque 800x500 viewport using Blitz Paint and AnyRender Vello CPU.
7. Support backgrounds, alpha, clipping, transforms, opacity, stacking contexts, DOM content above/below canvas, multiple canvases, and correct fixed viewport output.
8. Route Blitz hit testing, focus, scroll, form defaults, pointer coordinates, media state, ResizeObserver, and IntersectionObserver through LinkeDOM-visible state/events.
9. Delete the legacy layout/compositor in the same cutover. Bridge inconsistencies fail loudly.

## Validation

After the complete cutover builds:

- Test DOM reconciliation for insertion/removal/moves/clones/fragments/innerHTML/text/attributes/styles/namespaces/detachment/reflected state.
- Compare browser geometry for block/inline/flex/grid/percentages/viewport units/positioning/transforms/overflow/scrolling/replaced elements/fractional values.
- Verify opaque canvas byte identity and transparent/multi-layer/multi-canvas page composition.
- Re-run the known DOM failures first, then all 182 tests serially on Lavapipe and selected NVIDIA checks.
- Acceptance requires no regression among the existing 140 passes, fixed 800x500 output, deterministic repeat hashes, no client changes, no special cases, and complete removal of the legacy layout/compositor.
