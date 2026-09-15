#!/usr/bin/env bash
# Run inside nix-shell shell.nix. All live checks use private storage and browser profiles.
set -euo pipefail
export CARGO_BUILD_JOBS="$(nproc)"

cargo test -p afterglow-memory
cargo test -p afterglow-storage-worker --lib byte_store
cargo test -p afterglow-paint-worker --lib
cargo test --manifest-path prototype/character-editor/paint/maipointo/Cargo.toml --features demo --lib
cargo test -p afterglow-shell --lib browser::
cargo test -p afterglow-shell --lib browser::canvas::tests::gpu_regions_keep_pixels_across_capture_resize_and_removal -- --ignored
bun test scripts/check-paint-stress.test.ts
(
  cd prototype/character-editor
  bun test src
  node node_modules/vue-tsc/bin/vue-tsc.js --noEmit
  bash paint/build-wasm.sh
  bun run build
)
cargo build -p afterglow-shell --release
bun scripts/build-web.ts
bun scripts/build-web.ts --check
bun scripts/test-native-paint-recovery.ts docs/benchmarks/native-paint-shell/final-recovery
bun scripts/test-native-paint-recovery.ts docs/benchmarks/native-paint-shell/final-raster --raster
bun scripts/test-native-paint-recovery.ts docs/benchmarks/native-paint-shell/stress-native --stress
bun scripts/test-web-paint-stress.ts docs/benchmarks/native-paint-shell/stress-web
bun scripts/check-paint-stress.ts \
  docs/benchmarks/native-paint-shell/stress-native/result.json \
  docs/benchmarks/native-paint-shell/stress-web/result.json \
  docs/benchmarks/native-paint-shell/stress-comparison.json
(
  cd book
  nix-shell -p mdbook mdbook-mermaid --run 'mdbook build'
)
git diff --check
