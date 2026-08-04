# Character Baking

Afterglow separates editable character source data from finished runtime data.
The current first slice implements the core fixed-workspace algorithms in
`afterglow-character`.

## Current algorithms

The crate supplies:

- Signed three-parent SurfaceWrap fitting.
- Sparse structural-target evaluation and incremental updates.
- Piecewise macro weights and precomputed macro products.
- Corrected top-four skin-weight transfer.
- Area-weighted normal rebuilding.

All output uses caller-owned arrays. The tested hot operations do not allocate.

## Intended data flow

```text
CharacterSourcePack + CharacterRecipe
  -> CharacterBakeWorker
  -> compact CharacterBakeRecord
  -> atomic ModelSystem publication
```

A finished character will keep fitted geometry, selected face targets, cooked
LODs, rig rest data, materials, colliders, and spring records. It will not keep
the complete structural target library or SurfaceWrap maps.

The worker, source pack, runtime publication, and public `CharacterSystem` are
not implemented yet. See
`docs/implementation/runtime-character-bake-plan.md` for the ordered gates.

## Correctness

The unit suite includes synthetic boundary cases and a CC0 MakeHuman `short04`
hair fixture. The fitter matches MPFB neutral and head-width output within
`3e-6` Blender units.

Humentity and `bevy_make_human` are the pinned permissive implementation
references. Their notices are in
`crates/afterglow-character/THIRD_PARTY_NOTICES.md`.
