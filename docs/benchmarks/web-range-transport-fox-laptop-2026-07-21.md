# Web VT range transport — fox-laptop (2026-07-21)

> **Runtime-wiring note (updated 2026-07-26):** these measurements remain valid
> for the explicitly source-sorted diagnostic described below. The live
> `EngineAssets` provider intentionally preserves scheduler/admission order. The
> diagnostic is now `createSourceSortedPageReader()` over the same immutable
> `VtPageDirectory`. Do not cite 950.2 MiB/s as current gameplay-provider
> throughput until a live-provider gate reproduces it.

## Scope and method

This is a **single-client, same-host** browser transport gate. Caddy and CEF
both ran on fox-laptop for network profiles; the browser fetched `localhost`,
so Wi-Fi/WAN latency and bandwidth are absent. The native profile instead uses
CEF's `afterglow://local/` scheme with the same filesystem root. The diagnostic
page reads every compressed page of `Rock064_Color.png`: 5,461 pages, 127
row-coalesced ranges, and 96.85 MiB of useful source bytes. It uses 16
application-level in-flight reads and a fresh CEF profile per sample. Browser
`PerformanceResourceTiming.nextHopProtocol` proves H2/H3; the custom scheme has
no network protocol and therefore reports no resource protocol.

The machine used CEF/Chromium 149 and Caddy 2.10.2. Caddy served the generated
`www/` tree from `deploy/web/Caddyfile`; the H3 local-CA gate uses Chromium's
narrow SPKI certificate allowlist and `--origin-to-force-quic-on=localhost:9443`.
That forced-origin switch is necessary only for local custom-CA testing and is
not a production deployment mode.

Run the reproducible gate with:

```sh
nix-shell shell.nix --run "scripts/bench/bench-caddy-browser.sh h1-close"
nix-shell shell.nix --run "scripts/bench/bench-caddy-browser.sh h1"
nix-shell shell.nix --run "scripts/bench/bench-caddy-browser.sh h2"
nix-shell shell.nix --run "scripts/bench/bench-caddy-browser.sh h3"
```

## Row-coalesced result (three fresh browser samples)

| Profile | Negotiated transport | MiB/s samples | Median MiB/s |
|---|---|---:|---:|
| `afterglow` | CEF `afterglow://local/` scheme; exact 206 range reads | 441.2, 429.9, 443.3 | **441.2** |
| `h1-close` | dev server HTTP/1.1, `Connection: close` | 257.7, 257.9, 255.7 | **257.7** |
| `h1` | Caddy HTTPS persistent HTTP/1.1 | 123.7, 113.3, 121.0 | **121.0** |
| `h2` | Caddy HTTPS HTTP/2 | 208.4, 224.0, 201.4 | **208.4** |
| `h3` | Caddy HTTPS HTTP/3; resource timing was exactly `h3` | 80.0, 76.9, 78.1 | **78.1** |

The native custom scheme is the fastest browser-visible transport in this gate:
**2.12×** Caddy H2 and **5.65×** Caddy H3. The current handler reads `pread`
directly into CEF's supplied buffer (no temporary allocation/copy) and uses one
fixed 16-source cache across handlers, but it still includes CEF
browser→renderer delivery and V8 fetch/`arrayBuffer` work; it is not the raw
`pread` ceiling. Do not compare the `h1-close` number directly to Caddy as a protocol-only result: it
also changes server implementation and removes TLS. The valid Caddy comparison
is H2 versus H3: on this loopback CEF 149 workload, H3 is **2.67× slower** than
H2. Persistent H1 is also slower because Chromium limits parallel HTTP/1.1
sockets while H2 multiplexes the 16 application reads.

## Accepted CEF shared-message range bridge

The accepted native path bypasses the scheme URL loader, multipart parsing,
Mojo data-pipe streaming, and `fetch().arrayBuffer()`. TypeScript sorts the
5,461 independent page spans by source offset and groups them into bounded bulk
requests. The browser process merges adjacent spans into contiguous `pread`
calls and writes each result directly into a `CefSharedProcessMessageBuilder`.
A bounded renderer-local thread performs the one V8-sandbox-required copy into
`CefV8BackingStore`; the V8 task runner transfers it into an `ArrayBuffer`
without another payload copy. TypeScript returns page views over that buffer.

Capacities are fixed at 256 spans, 4 MiB per complete response, two responses,
and 8 MiB total in flight. Byte verification passed. The sorted contiguous-read
variant produced:

| Sample | Elapsed | Throughput |
|---:|---:|---:|
| 1 | 108.33 ms | 894.0 MiB/s |
| 2 | 101.92 ms | 950.2 MiB/s |
| 3 | 100.04 ms | 968.0 MiB/s |
| **Median** | **101.92 ms** | **950.2 MiB/s** |

This is about **2.15×** the 441.2 MiB/s scheme/fetch baseline. The original
1 GiB/s gate was not met; the user explicitly revised admission to **900 MiB/s
median** on 2026-07-21 and accepted this result. The final run verified `BIG1`
bytes and an AMD/RDNA2 non-fallback WebGPU adapter with no GPU-process or WebGPU
failure log lines.

Frame admission used twenty maximum-size responses at both production batch
cadences. At 1 ms urgent spacing, 180 sampled frames had p99/max 16.68 ms,
0 above 17 ms, and 0 below 55 FPS. At 100 ms quality spacing, all 300 frames had
the same result. The unconstrained throughput diagnostic intentionally chains
all 25 responses without a task/timer yield and can starve rAF; it is a
bandwidth gate, not production scheduling policy. The engine's non-resettable
1 ms/100 ms batching windows provide the validated event-loop yield.

A direct renderer-thread copy produced similar throughput. Both variants can
starve rAF when the diagnostic deliberately removes every event-loop yield; the
bounded copy thread was retained because it keeps each 4 MiB memcpy off the V8
thread and passed the production-cadence gates. An 8 MiB × one-in-flight variant
fell to 573.7 MiB/s and was rejected.

## Rejected mmap + CEF native-stream prototype

A follow-up prototype mapped `dungeon.big` read-only and created a
`cef_stream_reader_create_for_data` view for each selected `206` range. It was
built from source and measured on **fox-laptop** with the same release CEF
binary and hardware-WebGPU validation (`amd` / `rdna-2`). Correctness probes
completed for one, four, and sixteen parallel 1 MiB ranges. The representative
96.85 MiB / 127-range / 16-in-flight gate reached only **341.4 MiB/s**
(283.69 ms), versus the proven pre-mmap **441.2 MiB/s** median: a **22.6%
regression** and far below the predeclared **1 GiB/s** acceptance threshold.
The 4 MiB × 2 diagnostic reached only 117.2 MiB/s; row ranges at concurrency 2
reached 114.0 MiB/s.

The prototype was rejected and deleted. `afterglow-cef` therefore retains the
simpler cached-`pread` handler. This confirms that substituting CEF's byte stream
reader does not bypass browser→renderer Mojo transfer or renderer
`fetch().arrayBuffer()` materialization and is not a viable route to NVMe-rate
browser-visible reads on the laptop.

## H3 aggregation and UDP-buffer probes

The H3 slowdown is not Caddy's bulk transfer ceiling. One 96.85 MiB same-host
`curl` range transfer measured 438.8 MiB/s over H2 and 544.3 MiB/s over H3.
The browser bottleneck is many independently fetched ranges: browser H3 was
roughly flat across 1, 4, 8, 16, and 32 application-level in-flight reads
(76–93 MiB/s).

A diagnostic cross-row contiguous aggregation sweep at 16 application reads
improved H3 by reducing fetch count, but it is **not runtime policy**:

| Maximum range | Fetches | H3 MiB/s |
|---:|---:|---:|
| row-only | 127 | 79.8 |
| 1 MiB | 103 | 79.6 |
| 2 MiB | 54 | 91.4 |
| 4 MiB | 30 | 100.1 |
| 8 MiB | 18 | 113.4 |
| 16 MiB | 12 | 126.6 |

Temporarily raising `net.core.rmem_max` and `net.core.wmem_max` from 4 MiB to
7.5 MiB changed H3 insignificantly (median 79.3 MiB/s); values were restored
after the test.

A single native-scheme diagnostic with the same 4 MiB × 2 bounded shape
completed 30 ranges at 476.8 MiB/s, only 8% above its row-coalesced result. This
confirms that CEF's browser→renderer response delivery, rather than source open
or per-row request setup, is the dominant remaining native-scheme cost.

A constrained web probe found a promising sealed-mode candidate: **4 MiB maximum
range × 2 in-flight = 8 MiB hard raw-response budget**. It completed 30 ranges
and measured 342.6 MiB/s H2 and 122.9 MiB/s H3. Raising that same 4 MiB shape
to 4/16 requests reduced H2 to 314.4/287.4 MiB/s and H3 to 120.3/100.1 MiB/s,
so more concurrency is not a win here. This is a measurement, not selected
runtime policy.

## Decision

The bounded shared-process-message bridge is accepted for CEF `.big` range
batches. The admitted diagnostic source-sorts independent spans, adjacent native
spans collapse into contiguous `pread`, and page indices restore original
caller order. The bridge is bounded to 4 MiB × two in flight and passed the
900 MiB/s median transport gate. Production admission additionally requires
selecting source ordering as live `EngineAssets` policy and rerunning this gate
through the live provider; that policy change remains unselected.

Public web remains standards-only: offer H3 as a compatibility/robustness
transport with HTTP/2 fallback, but do not claim H3 makes this same-host
high-throughput workload faster. H2 is the fastest compatible Caddy path on
fox-laptop. Web bulk requests retain the same 256-span, 4 MiB complete-response,
and two/8 MiB in-flight bounds through standard multipart ranges.
