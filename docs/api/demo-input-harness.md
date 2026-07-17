# Bounded demo input and automation

Public browser barrels:

- `engine/input-api.ts`
- `engine/dev-harness-api.ts`

These are reusable ownership mechanisms for visual entrypoints; they do not
encode game controls or renderer policy.

## `BoundedKeyboardInput`

`BoundedKeyboardInput` owns keydown, keyup, and blur listener registration. It
maps a closed `DemoInputAction` enum into fixed `Uint8Array` down/pressed tables.
`isDown`, `consumePressed`, and `clear` are allocation-free. Repeats do not
create extra transitions, blur clears state, and `dispose()` removes every
listener. `programmatic` disables new keydown input for deterministic tests.

## Automation and diagnostics

`BootstrapGuard(capacity)` stores cleanup callbacks in fixed slots and rolls a
partially completed asynchronous bootstrap back in reverse order. `release()`
commits ownership to the installed runtime/page-shutdown path.

`FrameStepHarness(capacity)` stores frame targets and promise resolvers in fixed
slots. Exhaustion throws instead of growing; polling compacts in place without
`splice`. Promise construction/resolution is explicitly diagnostic and never a
gameplay mechanism.

`BrowserErrorCapture` owns global error/rejection listeners and records into the
runtime's bounded `EngineDiagnostics`. `snapshot()` allocates only when an
external diagnostic client requests details. `TextHud` owns diagnostic DOM text
writes. `publishDevHarness()` installs a test surface without visual entrypoints
touching global engine namespaces. `PageShutdown` owns the one-shot page teardown
listener and removes it on explicit disposal.
