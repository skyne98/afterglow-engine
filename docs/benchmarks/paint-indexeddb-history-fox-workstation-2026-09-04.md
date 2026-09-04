# IndexedDB paint-history validation — 2026-09-04

## Configuration

The test used these items:

- AMD Ryzen 9 9950X3D with 32 logical CPUs
- Chromium 146 with cross-origin isolation
- The character-editor Vite development server
- A 16,384 x 16,384 document
- A 512 MiB paint-memory selection
- The Classic brush with radius 60
- One operation with three full-width rows and 387 input samples

CDP sent the input messages directly to the paint worker. The operation changed
1,770 RGBA16 tiles. Its exact before-images used 55.31 MiB in IndexedDB. Thus,
the operation was larger than the 32 MiB internal Rust capture limit.

## Exact undo and redo

Before undo, the quarter-height display row had one painted run from pixel 23
through pixel 3,967. It had 3,945 painted pixels. The three sampled pixels had
RGBA values `(78, 205, 196, 255)`.

Undo changed all 1,770 history records from side 0 to side 1. The same display
row then had zero painted pixels. Redo changed all records back to side 0. The
painted run, painted-pixel count, and all three sampled RGBA values were equal
to the values before undo.

The worker had no engine error. It kept 1,770 resident tiles with a 4,096-tile
current limit and an 8,448-tile selected maximum.

## Queue-capacity check

A separate 64 x 64 document received 257 completed operations. IndexedDB kept
256 operations with identifiers 2 through 257 and removed identifier 1. All 256
retained operations completed undo and redo. The final history side values were
1 after all undo commands and 0 after all redo commands.
