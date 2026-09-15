LinkeDOM 0.18.0

Bundled as a self-contained ES module for the native V8 runtime.
Upstream: https://github.com/WebReflection/linkedom
License: ISC (see LICENSE).

Local event-dispatch patch:
- Capture, target, and bubble phases, including capture for non-bubbling events.
- Listener identity includes the capture flag. Removal, once, passive, and abort behavior use that identity.
- Listener callbacks receive the current target as `this`.
- Cancellation obeys `cancelable` and passive state.
- Exceptions do not stop subsequent listeners. Nested dispatch of the same event is rejected.

Local style patch: property assignments and removals use the attribute setter, so mutation observers receive style changes.
The shell also installs DOM interface type tags before application modules execute. This preserves native node identity in reactive libraries.

Regression command: `bun crates/afterglow-shell/tests/dom_setup_api_test.mjs`.
Shadow-DOM retargeting and full browser conformance remain outside this patch.
