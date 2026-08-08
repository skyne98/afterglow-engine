# Demo Input and Diagnostic Builds

Visual game bundles use `BoundedKeyboardInput` for fixed action state and
`TextHud` for game-owned text. Renderer construction, page teardown, global
error capture, readiness, and reverse shutdown belong to `EngineRuntime`.

Production bundles contain no `window.__afterglow*` globals, frame waiters, or
scenario/capture controls. Separate `diagnostic-*.html` pages load diagnostic
entrypoints built from the same game modules. Only those artifacts install the
versioned `globalThis.__afterglowDiagnosticV1` protocol.

The protocol waits for strict `GameReady`, snapshots adapter/readiness/dimension
state, reports fatal diagnostics and post-seal pipeline violations, and performs
idempotent shutdown. Artifact contracts and the deletion ledger prevent the old
per-demo bootstrap and automation surfaces from returning.
