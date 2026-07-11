# `latency-tool` CLI

> Status: working; API checked against the 2026-07-11 source.

CDP-based diagnostic tool for the `afterglow-cef` DevTools endpoint. It attaches
to the first page target through the browser-level websocket; CEF Views pages
do not necessarily appear in `/json/list`.

```text
latency-tool [host:port]                 # measure; default 127.0.0.1:9222
latency-tool eval '<expression>' [host:port]
latency-tool nav <url> [host:port]
```

- **measure** records Chromium tracing events, dispatches twelve synthetic mouse
  bursts, and reports input-event-to-next-`SkiaRenderer::SwapBuffers` latency
  plus present cadence. CDP input bypasses the OS input stack, so this is a
  reproducible lower bound rather than physical-device latency.
- **eval** uses `Runtime.evaluate` with `awaitPromise` and `returnByValue`.
- **nav** enables Page/Network domains, navigates, and prints loading events for
  2.5 seconds.

The implementation uses one `Cdp` session abstraction for connection, target
attachment, IDs, and browser/session commands. Trace sample extraction is a
pure tested function. The tool has no library API.
