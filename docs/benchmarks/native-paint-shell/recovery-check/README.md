# Native recovery input check

The release shell passed three process launches on the NVIDIA GeForce RTX 3090.
The test used a new temporary `AFTERGLOW_STORAGE_ROOT` and removed it afterward.
It did not open user recovery data.

1. The seed process created a 512 × 384 document, two layers, one group, and a completed stroke.
2. The runner sent `SIGKILL` after the completed-stroke result.
3. A new process displayed the recovery prompt and accepted a Restore click through XWayland and `xdotool`.
4. Restore retained the dimensions, layer/group order, raster hash, and 249 colored pixels on the center row.
5. Undo removed the stroke. Redo restored the same raster hash and composition.
6. The runner sent `SIGKILL` again.
7. A third process accepted a Discard click and created an empty 2048 × 2048 document with one layer.
8. Undo and redo did not restore the discarded drawing.

Both recovery clicks used the single visible window owned by the test process.
The probe checked native document readiness and button hit-testing before each click.
No script-generated click activated the recovery controls.

## Results

- Seed and Restore raster hash (32-bit FNV-1a): `2063070052`.
- Seed and Restore layer order: `Group 1`, `Layer 2`, `Layer 1`.
- Discard raster hash: `3172769221`. Center-row colored pixels: `0`.
- Process resident high-water values: 511,796 KiB (seed), 458,116 KiB (Restore), 538,580 KiB (Discard).
- Raw results: `result.json`, `seed.log`, `restore.log`, and `discard.log`.

The raster hash is not a byte-for-byte cross-process comparison.
The memory values include the shell and other native services.
They are not proof of the paint RAM ceiling or a long-session plateau.
This check does not measure 16K drawing, GPU upload time, pen input, or termination during an incomplete UI stroke.
Lower-level SQLite tests cover termination before and after root publication.

## Run

Build the release shell and editor first. Then run from the repository root:

```sh
nix-shell shell.nix --run 'bun scripts/test-native-paint-recovery.ts'
```

The runner creates private temporary storage for all three processes.
The runner needs an available XWayland display and `xdotool`.
