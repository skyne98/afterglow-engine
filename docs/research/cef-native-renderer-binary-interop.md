# CEF native-worker ↔ renderer binary interop

**Investigated:** 2026-07-21  
**Target:** CEF/Chromium 149 (`cef` 149.3.0+149.0.6)  
**Status:** selected direction; transport primitive implemented, native-service
composition and live source-sorted provider wiring incomplete

> **Implementation audit (2026-07-22):** the accepted benchmark explicitly
> source-sorted page spans, but `BigAssetSession`'s live
> `BoundedBulkReadQueue` currently preserves scheduler/admission order and does
> not call the sorting helper. The browser bridge merges only ranges already
> adjacent in the supplied order. It also reads `FsSource` directly rather than
> forwarding service work through generated native clients, and CEF VT
> transcoding still defaults to `texture.wasm` Web Workers. These are gaps
> against the decision below; they do not revise it.

## Question

What is the fastest supported way to exchange binary data between Afterglow's
native Rust workers and JavaScript in CEF's renderer, preferably without a
copy?

## Conclusion

CEF has two separate solutions that do not compose into a cross-process
zero-copy JavaScript buffer under the official V8-sandboxed build:

- `CefSharedProcessMessageBuilder` transfers an OS shared-memory mapping between
  the browser and renderer processes without serializing the payload bytes.
- `CefV8BackingStore` allocates sandbox-compatible V8 memory on the renderer
  thread, permits a **renderer-process** background thread to populate it, and
  then transfers ownership to a JavaScript `ArrayBuffer` without copying.

A background Rust thread **inside the renderer process** can therefore produce a
known-size JavaScript result with a true one-way zero-copy handoff. A native
worker in CEF's **browser process** cannot access that V8 allocation. Its shared
process-message mapping must be copied once into V8-owned memory before
JavaScript can consume it.

## Evidence

CEF PR [#4079](https://github.com/chromiumembedded/cef/pull/4079), merged in
February 2026 and present from API 146, explicitly defines this sequence:

1. create `CefV8BackingStore` on a thread with a valid V8 isolate;
2. pass it to a renderer-local background thread and populate `Data()`;
3. return it to the V8 thread;
4. call `CreateArrayBufferFromBackingStore()` as a zero-copy ownership transfer.

The pointer must not be retained after conversion. The operation is a one-shot
handoff, not a persistent shared ring.

CEF's generic message router uses shared-memory process messages above 16 KiB.
Current source then selects:

```cpp
#ifdef CEF_V8_ENABLE_SANDBOX
  CefV8Value::CreateArrayBufferWithCopy(...);
#else
  CefV8Value::CreateArrayBuffer(shared_region_pointer, ...);
#endif
```

`CreateArrayBuffer(void*, ...)` always returns null with the V8 sandbox enabled.
`CefSettings.no_sandbox` controls Chromium's OS sandbox and does not disable the
compile-time V8 sandbox.

CEF's JavaScript integration guide confirms that Blink/V8 execute in the
renderer process, V8 operations belong on `TID_RENDERER`, and browser↔renderer
bindings should be asynchronous. V8's own sandbox design explains why arbitrary
external backing memory is restricted and reports about 1% or less overhead on
typical workloads; disabling it merely to remove one memcpy is not justified
without measurement.

## Supported options

### Browser-process workers: one-copy bridge

This preserves Afterglow's current native target boundary:

```text
TypeScript Promise
  ↕ compact asynchronous CEF request
renderer bridge
  ↕ CefSharedProcessMessage shared mapping
browser dispatcher
  ↕ afterglow-rpc::RingBuffer
native OS worker
```

For a large response, the browser producer writes directly into a bounded
shared-process-message region. The selected asset bridge allocates a
`CefV8BackingStore` on the renderer thread, performs the one sandbox-required
copy on one renderer-local bounded background thread, posts completion to the
V8 task runner, and transfers that store into an ArrayBuffer without a second
copy. TypeScript exposes zero-copy typed-array views over that V8 buffer. The
copy thread starts only after renderer context creation and has a two-job queue
matching the bridge's two admitted responses.

The stock `CefMessageRouter` is a useful correctness reference but does not
provide Afterglow's fixed range count, response-byte limit, in-flight limit,
source ordering, or typed range API.

### Renderer-process producer: zero-copy output

```text
renderer/V8 thread: allocate known-size backing store
renderer-local Rust thread: pread/decode directly into Data()
renderer/V8 thread: consume store into ArrayBuffer and resolve
```

This is the best supported zero-copy route. It changes service ownership,
renderer restart/failure behavior, future OS-sandbox access, and the mandatory
project rule that native services are spawned from browser-process
`AppBuilder::on_ready`. It therefore requires an explicit user decision before
implementation. It is most plausible for known-size renderer-consumed bulk
output, not audio-device, Steam, or other native-only services.

### V8 sandbox disabled: cross-process zero-copy

A custom CEF build with `v8_enable_sandbox=false` allows the received shared
mapping to be wrapped directly with external `CreateArrayBuffer`. CEF's router
already has this conditional path. If the producer also writes directly into
the shared-message builder, the one-shot browser-worker → JavaScript handoff can
avoid payload copies.

This is not the recommended default: it requires maintaining custom CEF builds,
removes a material exploit mitigation, still allocates one-shot shared regions
and GC-owned ArrayBuffers, and has no evidence that the removed memcpy is the
current dominant cost.

### Missing CEF capability

WebView2 exposes `CreateSharedBuffer` + `PostSharedBufferToScript`, presenting an
OS shared mapping to JavaScript as an `ArrayBuffer` with explicit access and
release semantics. CEF has no equivalent public API. Implementing one while
retaining the V8 sandbox would require a CEF/Chromium patch or successful
upstream feature request.

## Direction and lifetime constraints

| Direction | Best official CEF 149 behavior |
|---|---|
| Renderer-local native thread → JS | Zero-copy one-shot `V8BackingStore` ownership transfer |
| Browser-process native worker → JS | Shared-memory IPC plus one final copy into V8 |
| JS → browser-process native worker | At least one copy from V8 bytes into IPC/shared transport |
| Persistent JS/native mutable ring | Not exposed by CEF public APIs |

`GetArrayBufferData()` is not a substitute for a persistent channel. V8 handles
are renderer-thread-bound, ordinary `ArrayBuffer` provides no atomic JavaScript
synchronization, and CEF does not document asynchronous background use of that
pointer as safe. `SharedArrayBuffer` works among JavaScript agents but CEF does
not expose its backing store to host native workers.

## Decision (2026-07-21)

One renderer-side payload copy is acceptable. Keep every engine service in the
browser process as a generated native client plus real OS worker; do not move
services into the renderer and do not disable the V8 sandbox. All JS-visible
native services will use one generic multiplexed bridge rather than bespoke
per-service channels:

1. renderer → browser requests and browser → renderer responses use bounded CEF
   process messages; shared-memory messages carry bulk payloads;
2. browser dispatcher → native worker and worker → dispatcher remain generated
   `afterglow-rpc::RingBuffer` traffic;
3. one renderer-local bounded thread copies each bulk shared mapping into a
   `CefV8BackingStore`; the V8 task runner converts it without another copy,
   resolves the typed Promise, and TypeScript exposes range views without
   copying payload bytes;
4. native-only paths such as audio-worker → device and worker → worker never
   cross the CEF bridge.

This is an explicit, narrow exception to the prior rule that RingBuffer was the
only website↔worker payload mechanism: CEF IPC is the unavoidable adapter across
the browser/renderer process boundary; RingBuffer remains the sole service
transport beyond that adapter.

The selected design uses 256 ranges maximum, 4 MiB maximum complete responses,
and two / 8 MiB maximum in flight. Its measured diagnostic sorts independent
page spans by source offset before dispatch, allowing the browser bridge to
merge adjacent spans into contiguous `pread` calls, and restores caller order
without copying payload bytes. The transport bounds and bridge are implemented;
the live BIG page provider has not yet wired this ordering step.

The first strict 1 GiB/s prototype gate was not met. The sorted contiguous-read
variant initially measured **905.5, 920.0, and 978.3 MiB/s**. The final bounded
copy-thread build measured **894.0, 950.2, and 968.0 MiB/s**, a **950.2 MiB/s
median** and about 2.15× the 441.2 MiB/s scheme/fetch baseline. On 2026-07-21 the
user explicitly changed admission to **900 MiB/s median** and accepted this
bounded design. Byte verification returned the expected `BIG1` magic; the CEF
adapter was AMD/RDNA2 with no fallback or GPU-process failure. Twenty maximum
responses spaced at both the urgent 1 ms and quality 100 ms production batch
cadences caused zero intervals above 17 ms and zero frames below 55 FPS. An 8
MiB × one-in-flight variant measured only 573.7 MiB/s and was rejected.

## Sources

- CEF V8 API: <https://github.com/chromiumembedded/cef/blob/master/include/cef_v8.h>
- CEF PR #4079: <https://github.com/chromiumembedded/cef/pull/4079>
- CEF V8 sandbox issue #3332: <https://github.com/chromiumembedded/cef/issues/3332>
- CEF message router: <https://github.com/chromiumembedded/cef/blob/master/libcef_dll/wrapper/cef_message_router.cc>
- CEF shared-message builder: <https://github.com/chromiumembedded/cef/blob/master/include/cef_shared_process_message_builder.h>
- CEF JavaScript integration guide: <https://chromiumembedded.github.io/cef/javascript_integration.html>
- CEF General Usage/IPC guide: <https://chromiumembedded.github.io/cef/general_usage.html>
- V8 sandbox design blog: <https://v8.dev/blog/sandbox>
- WebView2 SharedBuffer specification: <https://github.com/MicrosoftEdge/WebView2Feedback/blob/main/specs/SharedBuffer.md>
- WebView2 practitioner guide: <https://www.nutrient.io/blog/sharing-buffers-from-uwp-to-webview2/>
