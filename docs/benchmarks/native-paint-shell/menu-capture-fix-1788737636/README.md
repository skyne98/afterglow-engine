# Native menu input during capture

## Cause and change

The capture probe previously held module evaluation open with top-level awaits.
The shell rejects OS input before native readiness.
Thus, the diagnostic window could show the editor without accepting menu clicks.
Direct browser-bridge checks did not detect this condition.

`paint-capture-probe.ts` now starts an asynchronous function without holding module evaluation open.
It waits for native document loading before measurement.
The native capture driver rejects measurement before the host reports input readiness.
No production menu handler or native input guard changed.

## Checks

- `menus-active/host.log`: the host enters `Running` before the capture workload starts.
  XTest mouse clicks then open File, Edit, and View.
  Each menu has `aria-expanded="true"` and a menu element.
  OS Escape closes each menu. The driver checks the owned window before each click.
- `capture/`: the corrected page completes a native WebSocket capture with the existing pixel, source, sequence, and loss checks.
- The two module-evaluation regression tests pass for native and web targets.
- All 86 editor tests pass, with 3,434 assertions.
- TypeScript, Vite, mdBook, and the diff whitespace check pass.

## Limits

The OS check uses XWayland/XTest, not physical mouse hardware or native Wayland input injection.
It checks DOM menu state, not screenshot pixels or input-to-pixel latency.
The earlier `menus/host.log` does not prove an active capture workload because its profiling service was disabled.
Use `menus-active/host.log` for that check.
The full `paint-native-probe.html` still holds startup open for its automatic checks.
Its separate composition-undo failure remains open in `../menu-regression-1788736764/host.log`.
These functional checks do not complete the presentation pacing or soak gates.
