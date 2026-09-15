# Field codec prototype measurement

## Scope

The native benchmark measures field encoding and decoding, not application latency or capture overhead.
The active capture format does not use this prototype.

- CPU: AMD Ryzen 9 9950X3D, 32 logical CPUs.
- Target: `x86_64-unknown-linux-gnu`, release profile.
- Compiler: Rust `1.100.0-nightly`, commit `2e2b193f8ada105f27608b7be81c293e0d7292cb`.
- Method: 10,000 encoding warm-up calls, then seven samples of 200,000 calls per operation.
- Each sample reuses the same values and output buffer.
- Bootstrap allocation is outside the measurement.
- The benchmark checks decoded values after each sample.

The sample times are means per call, not individual-call percentiles.
The process has no fixed CPU affinity.
There is no browser, ARM, memory-pressure, or sustained application measurement.

## Fixed-slot baseline

[`native.jsonl`](native.jsonl) contains the original samples.
[`fixed-slots.rs`](fixed-slots.rs) preserves the measured codec source.
[`fixed-slots.hex`](fixed-slots.hex) preserves its 90-byte cross-language fixture.
The current codec no longer uses this padded layout.

| Case | Encoded bytes | Median encode ns | Median decode ns |
| --- | ---: | ---: | ---: |
| Two `u32` fields | 10 | 5.745 | 18.048 |
| Generational reference | 37 | 5.707 | 11.060 |
| Empty text, capacity 64 | 69 | 5.530 | 34.733 |
| Seven-byte text, capacity 64 | 69 | 5.732 | 34.695 |
| Full text, capacity 64 | 69 | 5.697 | 16.892 |
| Empty text, capacity 1,024 | 1,029 | 6.905 | 404.993 |
| Redacted text, capacity 1,024 | 1,029 | 6.259 | 392.817 |

The empty and redacted 1 KiB fields still transferred 1,029 bytes.
Their decoder also checked the unused zero bytes.
This result supports removal of padding before capture integration.
It does not justify a new application-wide capacity or timing limit.

## Compact prototype

The replacement uses one state byte per field.
Redacted fields have no payload.
Present text uses a four-byte length and its actual UTF-8 bytes, without padding.
Registration still calculates a bounded maximum size.
The encoder returns the actual written prefix length.

The compact regression checks and matching benchmark passed.
[`compact.jsonl`](compact.jsonl) contains the new samples.
[`comparison.json`](comparison.json) contains medians from both runs.
The two runs used separate processes with the same compiler, target, and sample counts.

| Case | Encoded bytes | Median encode ns | Median decode ns |
| --- | ---: | ---: | ---: |
| Two `u32` fields | 10 | 6.125 | 19.210 |
| Generational reference | 37 | 5.146 | 11.043 |
| Empty text, capacity 64 | 5 | 5.159 | 13.011 |
| Seven-byte text, capacity 64 | 12 | 5.167 | 15.354 |
| Full text, capacity 64 | 69 | 5.190 | 17.085 |
| Empty text, capacity 1,024 | 5 | 5.160 | 13.021 |
| Redacted text, capacity 1,024 | 1 | 2.860 | 6.874 |

The compact format removes unused payload bytes.
It does not improve every measured operation: the two-`u32` case was slightly slower in this run.
Seven samples from one process per implementation do not establish a sustained timing bound.

All 41 Rust tests and 35 TypeScript tests passed.
The Rust allocator check completed 100,003 encodes and 100,003 decodes without allocation.
Strict scoped Clippy, TypeScript checking, and the allocation lint also passed.

The same example measures the replacement:

```sh
nix-shell shell.nix --run 'CARGO_BUILD_JOBS=$(nproc) cargo run --release -p afterglow-telemetry --example bench_fields'
```

Recalculate the comparison with `bun docs/benchmarks/telemetry-field-codec/analyze.ts`.
The retained baseline is evidence, not a second runtime implementation.
