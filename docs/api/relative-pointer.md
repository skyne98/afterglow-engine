# Relative pointer input API

`crates/afterglow-web/web/src/engine/input/relative-pointer.ts` provides the engine's
lowest-latency browser-relative mouse path.

## `RelativePointerInput`

```ts
const input = new RelativePointerInput(canvas, (movementX, movementY) => {
  yaw -= movementX * sensitivity;
  pitch -= movementY * sensitivity;
});
```

Construction registers a primary `pointerdown` activation on the element and
feature-detects `pointerrawupdate`. Chromium/CEF receives this
event as soon as possible instead of waiting for the coalescible `pointermove`
path. Browsers without it use `mousemove`. Exactly one movement event type is
registered, preventing duplicate deltas. The passive movement callback checks
that its element owns pointer lock and forwards numeric `movementX/Y` directly;
the authored hot callback allocates nothing.

### `requestLock(): void`

The element calls this automatically for primary pointer gestures. It remains
public for other user-gesture controls. It requests Pointer Lock 2 with
`{ unadjustedMovement: true }`, bypassing operating-system mouse acceleration.
Rejection or a synchronous legacy-browser failure retries ordinary pointer lock.

### `getStatus(): Readonly<RelativePointerStatus>`

Returns one stable object:

- `eventType`: `pointerrawupdate` or fallback `mousemove`
- `locked`: whether the input element currently owns pointer lock
- `unadjustedMovement`: whether the current lock accepted raw movement

### `dispose(): void`

Removes the activation, movement, and `pointerlockchange` listeners.

## Latency boundary

This is the earliest standards-based event delivery available in Chromium. It
cannot bypass CEF/Chromium event dispatch, JavaScript task scheduling, WebGPU
submission, compositor scheduling, or display scanout. Windowed CEF avoids an
OSR texture-copy stage; vsync remains enabled by default because uncapped CEF
was empirically choppy. Claims about total input-to-present latency still
require hardware measurement rather than event timestamps alone.
