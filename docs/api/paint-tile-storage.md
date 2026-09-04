# Paint cold tile storage

Status: prototype implementation in `prototype/character-editor`.

## Owner

`src/paint-tile-store.ts` owns IndexedDB access in the paint worker. Rust does
not wait for IndexedDB. Rust keeps the synchronous brush working set in the
runtime resident tile cache.

The database name is `afterglow-paint`. The `tiles` object store uses this
ordered key:

```text
[document_id, layer, tile_x, tile_y]
```

Each value is one raw RGBA16 tile. The value has 32,768 bytes. The worker
stores the WebAssembly byte representation without channel conversion.

The page gives a random document ID at page load and for each new document.
The first worker open clears old records from this prototype database. A
new-document reset also deletes records for the previous document ID. A
document reset therefore cannot read tiles from another document.

## Rust boundary

The demo ABI has these cold-tile operations:

- `paint_init_with_tile_limits`
- `paint_get_resident_tile_count`
- `paint_get_resident_tile_limit`
- `paint_get_maximum_resident_tile_limit`
- `paint_set_resident_tile_limit`
- `paint_get_layer_used_tile_count`
- `paint_get_layer_used_tile_info`
- `paint_get_layer_used_tile_is_storage_dirty`
- `paint_get_layer_tile_ptr`
- `paint_remove_layer_tile`
- `paint_write_layer_rgba16_tile`

The ABI also supplies the resident count, current limit, maximum limit, and
the storage-dirty state of each used tile. The host can increase the current
limit up to the fixed maximum for that document.

The store also removes cold records for a cleared layer and shifts records
when a layer is deleted. The worker copies modified tile bytes before it
removes a resident tile. It removes the tile only after the IndexedDB write
completes. A failed write leaves the tile resident and rejects the stroke.

## Page rule

The worker starts with 4,096 resident tiles. Near each limit, it increases the
limit by 2,048 tiles (64 MiB). It does not evict a tile until the runtime
limit reaches the selected maximum and enters its 512-tile allocation headroom.

The page operation does these steps:

1. Protect the recent input path plus a 1,024-pixel border.
2. Predict up to 100 ms and 512 pixels in the movement direction.
3. At the maximum, select far-behind tiles until one 2,048-tile block is free.
4. Write only modified selected tiles in transactions of at most 64 tiles (2 MiB).
5. Remove all selected tiles after the write completes.
6. Restore cold tiles in the protected and predicted path.
7. Start the brush only after all restore work completes.

A restored tile has a clean storage state. A later eviction does not write it
again unless paint, import, undo, or redo changed it. This prevents repeat
writes when the brush moves between old regions. Each page request has a
generation. A document reset or a newer request stops stale tile installation
after each IndexedDB wait.

The compound key orders X before Y. A rectangular read therefore uses one
exact key range for each X column. It does not scan unrelated Y values in the
columns between the rectangle edges.

The same database has a separate exact history store. One operation keeps one
before-image for each changed tile. The worker writes groups of 64 tiles (2 MiB)
during the operation. Undo and redo first write an opposite-side image for all
operation tiles. They then restore groups of 64 tiles. The cursor changes only
after all groups are restored.

The history queue keeps as many as 256 completed operations. It removes the
oldest operation when the count or IndexedDB quota blocks a new write. If the
current operation alone cannot fit, the worker restores its before-images. It
does not complete an operation without undo data.

The protected region has a limit of 1,536 source tiles. This gives the brush a
bounded working set. If the predicted corridor is too large, the worker uses
the current-point region. A later input region that cannot keep the protected
brush region ends the stroke.

An absent cold record means that the tile has no paint and stays absent from
Rust. An IndexedDB error reports `Paint storage is full.` and drops the stroke
without a wasm trap. Rust still reports error code `1` if a brush operation
reaches its resident limit during a stroke.

## Tests and checks

TypeScript tests cover key order, exact column bounds, 2 MiB write batches,
the 256-operation FIFO history queue, and memory-limit calculations. Rust tests cover shared runtime limits, metadata
growth, storage-dirty changes, removal, restoration, and the resident hash.
Browser checks must cover a 16K document with
more than 4,096 logical painted tiles and must confirm:

- no `RuntimeError: unreachable`,
- no `Paint tile allocation failed.` message,
- no RefCell borrow error,
- input queue completion after each page operation,
- full-width draw-over strokes after eviction,
- an operation above the internal 32 MiB history limit,
- smudge plus undo and redo after restoration,
- active-layer clear without loss of other-layer cold records.

The 2026-09-04 browser gate stored, undid, and redid one exact 1,770-tile
operation. It also kept operations 2 through 257 after a 257-operation test.
See `docs/benchmarks/paint-indexeddb-history-fox-workstation-2026-09-04.md`.
