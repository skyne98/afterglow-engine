# Live native paint capture

The collector recorded ten seconds from the existing editor without changes to the drawing or window.

- File: `capture-b262c7820a0ffdf3babef474bbacdefc-1.dgtl`.
- Size: 122,178 bytes.
- Host records: 2,866.
- Runtime turns: 1,432 starts and 1,431 ends.
- Input, paint samples, paint publications, frame, presentation, and RPC records: zero.
- Dropped records, overwritten records, and sequence gaps: zero.

The runtime continued to execute during this interval.
No stroke occurred in the captured interval, so this capture cannot distinguish an idle editor from a disabled paint adapter.
It does not identify the error that stopped the earlier drawing or measure large-brush latency.
The capacity and error-handling findings in `../../../implementation/native-large-canvas-paint-plan.md` are separate source findings.

## Subsequent editor failure

The same editor process (3359498) subsequently exited with code 101.
`editor-exit.log` retains the output from that launch, including this error:

```text
thread 'main' (3359498) panicked at crates/afterglow-shell/src/main.rs:2891:9:
native JavaScript event loop failed: Error: Paint history capture allocation failed.
    at Vt.accept (file:///home/fox/Project/afterglow-engine/prototype/character-editor/dist/assets/paint-demo-9RIqobmK.js:1:26319)
    at Vt.drain (file:///home/fox/Project/afterglow-engine/prototype/character-editor/dist/assets/paint-demo-9RIqobmK.js:1:27628)
```

This log confirms a history-capture failure and an uncaught JavaScript error at the native event-loop boundary.
It does not supply the failed tile count or prove that all earlier delays had the same cause.
The running binary did not contain the new SQLite store or crash-recovery integration.
No recovery of the unsaved drawing is verified.

