# Low-latency mouse input

Use `RelativePointerInput` for first-person or free-look camera controls:

```ts
const input = new RelativePointerInput(renderer.domElement, (dx, dy) => {
  yaw -= dx * sensitivity;
  pitch -= dy * sensitivity;
});
```

A primary press on the element requests pointer lock automatically. The engine chooses Chromium's `pointerrawupdate` event when available. Unlike a
normal coalescible pointer-move path, raw updates are dispatched as soon as the
browser can deliver them. It falls back to `mousemove` on other browsers.
Exactly one event type is active, so movement cannot be counted twice.

`requestLock()` first requests `{ unadjustedMovement: true }`. This disables OS
mouse acceleration and supplies raw relative deltas. If the platform rejects
that option, the engine retries ordinary pointer lock. `requestLock()` remains
available when another user-gesture control should initiate locking.

The movement callback is passive and performs no engine-authored allocation.
`getStatus()` exposes the selected event, lock state, and whether unadjusted
movement was accepted. Call `dispose()` when destroying the input owner.

This is the lowest-latency standards-based Chromium input path, not a guarantee
of zero end-to-end latency. JavaScript scheduling, WebGPU submission, the
compositor, vsync, and scanout remain. afterglow uses windowed CEF to avoid an
additional off-screen-rendering texture copy.
