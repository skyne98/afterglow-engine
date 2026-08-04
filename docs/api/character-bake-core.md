# Character bake core

`afterglow-character` supplies the fixed-workspace algorithms for compact
character baking. It does not yet supply source-pack parsing, RPC workers,
Three.js publication, or the public `CharacterSystem`.

## SurfaceWrap

```rust
fit_surface(driver_positions, bindings, scale, output_positions)
calculate_surface_scale(driver_positions, scale)
```

Each `SurfaceBinding` has three driver indices, three signed weights, and one
engine-space offset. `SurfaceScale` can define one source-distance reference for
each axis.

The fit uses this relation:

```text
output = sum(driver[parent] * weight) + offset * current_axis_scale
```

The evaluator does not normalize or clamp mapping weights. The offline cooker
must convert MakeHuman coordinates to engine coordinates before runtime use.

## Sparse structural targets

```rust
evaluate_sparse_targets(neutral, targets, weights, output)
apply_sparse_target_delta(positions, target, prior_weight, next_weight)
```

A `SparseTarget` contains strictly ascending unique vertex indices. The complete
evaluator copies the neutral positions and applies targets in input order.

The incremental operation subtracts the prior contribution and adds the new
contribution. Both operations use caller-owned arrays.

## Macro weights

```rust
resolve_piecewise_macro(value, segments, state_weights)
compose_macro_products(state_weights, factors, terms, target_weights)
```

`resolve_piecewise_macro` converts one control into linear endpoint-state
weights. `NO_MACRO_STATE` identifies an empty endpoint.

`compose_macro_products` evaluates a cooker-built table of state products. This
supports the sex, age, ethnicity, weight, muscle, height, proportion, cup, and
firmness target families without runtime string construction.

## Skin transfer

```rust
transfer_skin_weights(driver_skin, bindings, output_skin)
```

The operation interpolates driver weights through the same SurfaceWrap map. It
then performs these steps:

1. Aggregate duplicate bone IDs.
2. Remove non-positive final contributions.
3. Select the four largest weights.
4. Use the lower bone ID to resolve an equal-weight order.
5. Normalize only the selected influences.
6. Leave unused output entries at zero.

This corrects the repeated-padding behavior found in the Humentity reference.

## Normal rebuild

```rust
rebuild_area_weighted_normals(positions, indices, output_normals)
```

The operation accumulates triangle cross products and normalizes each vertex
result. It reports triangle, degenerate-triangle, and isolated-vertex counts.

## Allocation and failure

All algorithm output uses caller-owned slices. Unit tests use the
`afterglow-rpc` tracking allocator and prove that the accepted hot operations do
not allocate.

Errors use `CharacterBakeError`. Length, index, finite-value, scale, sparse
order, macro, triangle, and skin failures are deterministic.

## Validation

The crate has synthetic boundary tests and one real CC0 hair fixture. MPFB
2.0.17 generated neutral and `head-scale-horiz-incr` positions for 26 sampled
`short04` vertices.

The independent evaluator matches those values within `3e-6` Blender units.
The fixture includes negative and greater-than-one mapping weights.

Run:

```sh
nix-shell shell.nix --run "cargo test -p afterglow-character"
nix-shell shell.nix --run \
  "cargo clippy -p afterglow-character --lib --no-deps -- -D warnings"
```

## Provenance

Humentity and `bevy_make_human` are the co-primary N1 permissive references.
`crates/afterglow-character/THIRD_PARTY_NOTICES.md` records their pinned
revisions and MIT notices.
