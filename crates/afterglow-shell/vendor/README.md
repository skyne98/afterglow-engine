# Vendored dependencies

Dependencies are vendored only when this native runtime needs a narrow upstream
patch or a reproducible browser-environment component.

## Rust WebGPU crates

`deno_webgpu/` and `naga/` are complete crates.io source releases selected by
`Cargo.toml` through `[patch.crates-io]`. Their versions are in `versions.env`.
The maintained local deltas are standalone patches under `patches/`.

Refresh the current releases exactly:

```bash
scripts/update_vendored_webgpu.sh
```

Try newer compatible releases:

```bash
scripts/update_vendored_webgpu.sh <deno_webgpu-version> <naga-version>
```

The command downloads into a temporary directory, applies both patches, and
only then replaces the checked-in trees. If upstream changed the relevant code
or implemented the corresponding native feature support, patching fails without
touching the current vendor directories. Inspect upstream, adjust or remove the corresponding
patch, rerun the script, update the normal dependency constraint in
`Cargo.toml` when required, and run:

```bash
cargo check -p afterglow-shell --example browser_test
```

Do not run an unqualified `cargo update`; it needlessly updates unrelated
locked dependencies. Cargo will refresh the patched package entries when their
source versions or dependency constraints change.

Do not edit generated three.js WGSL to compensate for dependency behavior. Any
compatibility change belongs in these native dependencies or the host runtime.

## Blitz

The pinned Blitz monorepo lives at workspace path
`vendor/afterglow-shell-blitz/`. It is outside this crate directory so its own
workspace inheritance remains isolated from the Afterglow Cargo workspace. The
workspace root's `[patch.crates-io]` table selects the required Blitz, paint,
traits, Stylo/Taffy, and debug-timer crates.

Run its focused browser-query regressions with:

```bash
cargo test --manifest-path vendor/afterglow-shell-blitz/Cargo.toml \
  -p blitz-html --test browser_queries --locked
```

## LinkeDOM

`linkedom/` is the browser-environment DOM implementation used by the native V8
host. Its license and upstream release files are kept in its directory.
