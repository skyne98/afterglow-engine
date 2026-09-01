# Paint cold tile storage

Status: prototype implementation in `prototype/character-editor`.

## Owner

`src/paint-tile-store.ts` owns IndexedDB access in the paint worker. Rust does
not wait for IndexedDB. Rust keeps the synchronous brush working set in the
fixed resident tile cache.

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

- `paint_get_layer_used_tile_count`
- `paint_get_layer_used_tile_info`
- `paint_get_layer_tile_ptr`
- `paint_remove_layer_tile`
- `paint_write_layer_rgba16_tile`

The store also removes cold records for a cleared layer and shifts records
when a layer is deleted. The worker copies tile bytes before it removes a resident tile. It removes the
tile only after the IndexedDB write completes. A failed write leaves the tile
resident and rejects the stroke start.

## Page rule

The worker starts a page operation before `_begin_stroke` when resident use
reaches 3,584 tiles or when cold records exist for the document.

The page operation does these steps:

1. Protect the current input path plus a 1,024-pixel border.
2. Write non-protected resident tiles to IndexedDB.
3. Remove saved tiles from Rust until 2,048 or fewer remain.
4. Restore cold tiles in the protected path.
5. Start the brush only after all restore work completes.

During an open stroke, the worker restores the next bounded input region before
it processes that input. It writes older tiles before it evicts them and keeps
the current brush region protected.

The protected region has a limit of 1,536 source tiles. This gives the brush a
fixed working set and keeps all allocations bounded. A brush path that needs a
larger working set reports a storage error and does not start. A later input
region that exceeds the resident limit ends the stroke without removing the
protected brush region.

An absent cold record means that the tile has no paint and stays absent from
Rust. An IndexedDB error reports `Paint storage is full.` and drops the stroke
without a wasm trap. Rust still reports error code `1` if a brush operation
reaches its resident limit during a stroke.

## Tests and checks

`paintTileKey` has a TypeScript unit test. The Rust tile surface tests remove,
restore, and hash resident tiles. Browser checks must cover a 16K document with
more than 4,096 logical painted tiles and must confirm:

- no `RuntimeError: unreachable`,
- no `Paint tile allocation failed.` message,
- no RefCell borrow error,
- input queue completion after each page operation,
- full-width draw-over strokes after eviction,
- smudge plus undo and redo after restoration,
- active-layer clear without loss of other-layer cold records.
