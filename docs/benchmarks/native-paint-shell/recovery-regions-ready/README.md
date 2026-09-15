# Recovery after native region publication

The release shell passed three process launches on the RTX 3090 with the region-publication code.
The runner used private temporary SQLite storage and physical pointer clicks on its own window.
It killed each completed process with SIGKILL before the next launch.

| Phase | Dimensions | Raster hash | Colored center-row pixels | Layer order |
|---|---|---:|---:|---|
| Seed | 512 × 384 | 2063070052 | 249 | Group 1, Layer 2, Layer 1 |
| Restore | 512 × 384 | 2063070052 | 249 | Group 1, Layer 2, Layer 1 |
| Discard | 2048 × 2048 | 3172769221 | 0 | Layer 1 |

Restore also passed the probe checks for undo and redo.
Discard returned an empty document without history.
`result.json` contains the results and process memory.
The phase logs contain native startup, adapter information, and probe results.
The complete run, mdBook build, and diff check passed in task `b5d40b023`.

The previous run stopped before a click because the pointer was outside its owned window.
The runner now waits for both startup and prompt markers, then activates only its owned window.
It still verifies the pointer window before every click.
The successful run did not bypass that check.

Whole-process VmHWM was 498,152 KiB for Seed, 453,708 KiB for Restore, and 538,492 KiB for Discard.
These are not paint allocation limits or evidence of a sustained memory bound.
The raster hash and center-row checks do not prove byte-for-byte equality across processes.
This 512 × 384 recovery check does not establish 16K performance, mid-stroke recovery, or native/web parity.

```sh
nix-shell shell.nix --run 'bun scripts/test-native-paint-recovery.ts docs/benchmarks/native-paint-shell/recovery-regions-ready'
```

The runner removes its private storage after the check. It never opens user recovery data.
