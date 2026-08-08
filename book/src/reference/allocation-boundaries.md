# Allocation Boundaries

Afterglow's frame-owned queues, caches, and typed-array operations are sealed.
Some boundaries necessarily allocate: browser fetch/readback, worker response
Promises, Basis output, image/model parsing, bounded runtime model revisions,
mutable RAM texture page regeneration, resident R8 displacement + blue-noise
fetch/texture creation, prewarmed VT material creation,
Three.js pipeline compilation, timestamp resolution,
diagnostics, and game-facing reactive callbacks.

These paths are explicitly classified in `engine-allocation-effects.json` and
bounded by task counts, bytes, loading phases, or diagnostic frequency. Pipeline
compilation belongs to warm-up; debug snapshots and timestamp resolution never
run every frame. Promise APIs adapt to fixed task slots rather than owning an
unbounded engine queue.

Heap deltas are profiling signals, not allocation counts. Atlas tests therefore
also require queues and pending bytes to return to zero, fixed high-water marks,
no post-seal pipelines, and no monotonic trend in longer soaks. See the canonical
boundary table in `docs/api/allocation-boundaries.md`.
