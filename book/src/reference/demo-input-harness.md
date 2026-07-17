# Demo Input and Automation

Canonical visual demos use bounded ownership helpers instead of open-ended key,
waiter, error, and listener collections:

- `BootstrapGuard` provides fixed-capacity reverse rollback for partial startup.
- `BoundedKeyboardInput` maps fixed actions into preallocated down/pressed bits.
- `FrameStepHarness` provides fixed-capacity out-of-band frame waits.
- `BrowserErrorCapture` routes browser failures into bounded engine diagnostics.
- `TextHud` isolates diagnostic DOM writes from frame orchestration.
- `publishDevHarness` exposes automation without a global engine bridge.
- `PageShutdown` owns page-teardown listener cleanup.

These utilities are for demos and diagnostics. Game input policy remains game
code, while the engine primitives make capacity and lifecycle explicit.
