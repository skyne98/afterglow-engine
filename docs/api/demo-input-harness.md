# Bounded demo input and diagnostic builds

## Product-side input and HUD

`web/src/engine/input/index.ts` exports `BoundedKeyboardInput`, which owns its
listeners and maps the closed `DemoInputAction` enum into fixed `Uint8Array`
down/pressed tables. `isDown`, `consumePressed`, and `clear` allocate nothing;
`dispose()` removes all listeners.

`web/src/engine/diagnostics/text-hud.ts` exports the small `TextHud` DOM writer.
It contains no readiness, capture, lifecycle, or global automation policy.

Lifecycle and fatal error ownership belong to `EngineRuntime`. Visual demos no
longer construct `BootstrapGuard`, `PageShutdown`, `BrowserErrorCapture`, or
frame-waiter objects.

## Separate diagnostic artifacts

Production visual bundles contain no `window.__afterglow*` globals, frame
waiters, scenario registries, or screenshot controls. The artifact manifest
builds five separate diagnostic entrypoints under `www/diagnostic/` and five
`diagnostic-*.html` pages.

Those entrypoints install one typed protocol from
`engine/diagnostics/visual-protocol.ts`:

```ts
interface VisualDiagnosticProtocol {
  readonly version: 1;
  snapshot(): VisualDiagnosticSnapshot;
  waitForGameReady(timeoutMs?: number): Promise<VisualDiagnosticSnapshot>;
  shutdown(): Promise<void>;
}
```

Only diagnostic bundles publish `globalThis.__afterglowDiagnosticV1`.
`snapshot()` reports strict readiness, logical/physical canvas and surface
sizes, feedback size, adapter identity, fatal diagnostics, frame ID, and
post-seal pipeline violations. `waitForGameReady()` rejects on fatal/shutdown or
timeout; it never accepts a console phrase or first presentation as readiness.

`check-web-contracts.ts`, the deletion ledger, and generated-bundle checks keep
the protocol out of production artifacts.
