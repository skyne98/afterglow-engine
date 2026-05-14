# Streaming Persistence And Interest Profile

Date: 2026-05-13

Benchmark command:

```bash
cargo bench -p afterglow-engine --bench persistence_streaming
```

Synthetic load:

- 10,000 players
- 1,024 chunks
- 64 persistent entities per chunk
- 65,536 total replicated/persistent entities
- 9 visible chunks per player
- 128 chunks captured/applied in one streaming step

Latest local result:

```text
streaming_interest players=10000 chunks=1024 entities=65536 visible_chunks=9 player_chunk_update=859us chunk_ref_fanout_snapshot=9.812ms chunk_owned_fanout_snapshot=14.676ms batch_filter_snapshot_for_all_players=1533.057ms legacy_filter_snapshot_for_100_players=355.839ms
streaming_persistence chunks_total=1024 entities_per_chunk=64 streaming_chunks=128 captured_entities=8192 capture_chunks=26.142ms apply_chunks=6.730ms
```

## Findings

Player chunk visibility churn is cheap. Updating 10,000 players' visible chunk
sets is about 1 ms in this synthetic case.

The old per-player interest path is not viable for large fanout. Filtering 100
players one by one over a 65,536-entity snapshot costs about 356 ms, which
extrapolates to a 10,000-player full-snapshot path in the tens of seconds. Do
not use `InterestMap::filter_snapshot()` in a loop for server-wide snapshot
fanout.

The batch owned interest path is much better, but still not something to run as
a full snapshot fanout every tick. `InterestMap::filter_snapshots()` builds the
chunk-to-player fanout once and filters all 10,000 players in about 1.5 seconds
for this full-snapshot case. That is acceptable for rare debugging/recovery,
not normal movement.

The chunk fanout path is the server-scale shape. `snapshot_chunk_ref_fanout()`
builds shared chunk payload routing in about 9.8 ms without cloning per-player
entity snapshots. `snapshot_chunk_fanout()` owns/clones one snapshot per visible
chunk and costs about 14.7 ms, still far below per-player snapshots. Network
packing should prefer the borrowed fanout and serialize one payload per chunk,
then send that payload to the chunk's interested players.

Persistence capture used to be dominated by repeated stable-registry
maintenance and full-world scans. Batch capture dropped the 128-chunk unload
case from about 494 ms to about 26 ms. It remains mostly serial because Bevy
`World` access and component capture are not safely parallelized at this layer.

Persistence apply used to repeat stable-registry maintenance per chunk. Batch
apply dropped the 128-chunk load case from about 76 ms to about 6.7 ms.

## Design Implications

The normal multiplayer path for 10,000 moving players must be chunk-event and
delta based:

- update each player's visible chunk set
- compute chunk enter/leave
- send shared chunk baselines for newly entered chunks
- send dirty per-tick entity deltas only for changed entities
- batch fanout by chunk, not by player
- retain last-known chunk membership for removed entities until their removal
  delta is routed
- clear retained removed-entity chunk membership automatically after batch
  delta/fanout routing by default; disable the cleanup knob only when the server
  needs explicit ack-driven retention
- deduplicate player recipient lists before fanout so duplicate inputs cannot
  multiply packet work

Full per-player snapshots are only for reconnect, debugging, or rare recovery.
The engine should eventually cache serialized chunk baselines and changed
entity payloads so 100 players entering the same chunk share the same packed
bytes instead of cloning or serializing the same entity data 100 times.

Rayon is useful only on pure-data stages with deterministic output ordering.
The current fanout code keeps a serial fast path for normal chunk counts and a
Rayon path for very large entity counts. An attempted Rayon pass inside
persistence capture/apply was slower because the expensive parts touch Bevy
`World` or are too small to amortize thread scheduling.

The persistence API now has streaming-sized batch paths:

- `capture_chunk_deltas(world, chunks)` for unload/save
- `apply_chunk_deltas(world, deltas)` for load/restore

Batch apply validates chunk IDs, entity IDs, duplicate entity records, and
restore/delete conflicts before mutating the world. Corrupt overlapping chunk
saves must fail loudly instead of silently moving entities between chunks.

Single-chunk wrappers remain for small tests and tools.
