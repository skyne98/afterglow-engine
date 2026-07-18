#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
prototype="$root/prototype/steam-audio-wasm"
cache="$root/target/steam-audio-wasm-build"
upstream="$cache/steam-audio"
sdk="$cache/sdk"
version=v4.8.1

for command in em++ python3 cmake git curl unzip bun cargo rustc; do
  command -v "$command" >/dev/null || { echo "missing build command: $command" >&2; exit 1; }
done
em++ --version | head -1 | grep -q '4\.0\.23' || {
  echo 'Steam Audio benchmark requires pinned Emscripten 4.0.23; use toolchain.nix' >&2
  exit 1
}
rustc --version | grep -q '1\.99\.0-nightly (375b1431b 2026-07-10)' || {
  echo 'Steam Audio obvhs benchmark requires rustc 1.99.0-nightly commit 375b1431b' >&2
  exit 1
}

if [[ ! -d "$upstream/.git" ]]; then
  mkdir -p "$cache"
  git clone --depth 1 --branch "$version" https://github.com/ValveSoftware/steam-audio.git "$upstream"
fi

emroot=$(dirname "$(dirname "$(readlink -f "$(command -v emcc)")")")
mkdir -p "$cache/emsdk/upstream"
ln -sfn "$emroot/share/emscripten" "$cache/emsdk/upstream/emscripten"

# Steam Audio 4.8.1 pins dependencies whose old CMake minimum needs an
# explicit compatibility floor with current CMake.
getdeps="$upstream/core/build/get_dependencies.py"
if ! grep -q 'CMAKE_POLICY_VERSION_MINIMUM=3.5' "$getdeps"; then
  perl -0pi -e "s/cmake_args = \[\]/cmake_args = ['-DCMAKE_POLICY_VERSION_MINIMUM=3.5']/" "$getdeps"
fi

zlib_library="$upstream/core/deps/zlib/lib/wasm/release/libz.a"
pffft_library="$upstream/core/deps/pffft/lib/wasm/release/libpffft.a"
mysofa_library="$upstream/core/deps/mysofa/lib/wasm/release/libmysofa.a"
pushd "$upstream/core/build" >/dev/null
for dependency in zlib pffft mysofa; do
  library="$upstream/core/deps/$dependency/lib/wasm/release/lib${dependency}.a"
  [[ "$dependency" == zlib ]] && library="$zlib_library"
  if [[ ! -f "$library" ]]; then
    python3 get_dependencies.py --platform wasm --emsdk "$cache/emsdk" --dependency "$dependency"
  fi
done
popd >/dev/null

# Emscripten requires every object linked into a shared-memory module to have
# atomics and bulk-memory enabled. Keep Valve's ordinary dependency archives for
# the direct-only module and build a second, pthread-compatible set for dynamic
# reflections.
single_deps="$cache/single-thread-deps"
pthread_deps="$cache/pthread-deps"
mkdir -p "$single_deps" "$pthread_deps"
cp "$zlib_library" "$single_deps/libz.a"
cp "$pffft_library" "$single_deps/libpffft.a"
cp "$mysofa_library" "$single_deps/libmysofa.a"
if [[ ! -f "$pthread_deps/libz.a" || ! -f "$pthread_deps/libpffft.a" ||
      ! -f "$pthread_deps/libmysofa.a" ]]; then
  dependencies_json="$upstream/core/build/dependencies.json"
  cp "$dependencies_json" "$cache/dependencies.json.single-thread"
  restore_single_deps() {
    cp "$cache/dependencies.json.single-thread" "$dependencies_json"
    cp "$single_deps/libz.a" "$zlib_library"
    cp "$single_deps/libpffft.a" "$pffft_library"
    cp "$single_deps/libmysofa.a" "$mysofa_library"
  }
  trap restore_single_deps EXIT
  perl -0pi -e 's/-msimd128/-pthread -msimd128/g' "$dependencies_json"
  rm -rf "$upstream/core/deps-build/zlib" \
         "$upstream/core/deps-build/pffft" \
         "$upstream/core/deps-build/mysofa"
  rm -f "$zlib_library" "$pffft_library" "$mysofa_library"
  pushd "$upstream/core/build" >/dev/null
  for dependency in zlib pffft mysofa; do
    python3 get_dependencies.py --platform wasm --emsdk "$cache/emsdk" --dependency "$dependency"
  done
  popd >/dev/null
  cp "$zlib_library" "$pthread_deps/libz.a"
  cp "$pffft_library" "$pthread_deps/libpffft.a"
  cp "$mysofa_library" "$pthread_deps/libmysofa.a"
  restore_single_deps
  trap - EXIT
fi

if [[ ! -f "$sdk/libphonon.a" || ! -f "$sdk/phonon_version.h" ]]; then
  archive="$cache/steamaudio.zip"
  curl -L --fail --silent --show-error -o "$archive" \
    "https://github.com/ValveSoftware/steam-audio/releases/download/${version}/steamaudio_4.8.1.zip"
  mkdir -p "$sdk"
  unzip -j -o "$archive" \
    steamaudio/lib/wasm/libphonon.a \
    steamaudio/include/phonon.h \
    steamaudio/include/phonon_version.h \
    -d "$sdk" >/dev/null
fi

# Valve's released WASM archive constructs std::threads for real-time
# reflections, but it was compiled without atomics/pthreads. Rebuild the pinned
# source with Emscripten pthreads. Two workers are pre-created during bootstrap
# so gameplay never creates a thread or waits for browser worker startup.
threaded="$cache/steam-audio-threaded/src/core/libphonon.a"
if [[ ! -f "$threaded" ]]; then
  if [[ ! -x "$upstream/core/deps/flatbuffers/bin/linux-x64/flatc" ]]; then
    pushd "$upstream/core/build" >/dev/null
    python3 get_dependencies.py --platform wasm --emsdk "$cache/emsdk" --dependency flatbuffers
    popd >/dev/null
  fi
  # FlatBuffers 1.12's move assignment predates current Clang's stricter
  # deleted-copy diagnostics.
  perl -0pi -e 's/buf_ = other\.buf_;/buf_ = std::move(other.buf_);/g' \
    "$upstream/core/deps/flatbuffers/include/flatbuffers/flatbuffers.h"
  emcmake cmake -S "$upstream/core" -B "$cache/steam-audio-threaded" \
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
    -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=FALSE \
    -DCMAKE_C_FLAGS=-pthread -DCMAKE_CXX_FLAGS=-pthread \
    -DCMAKE_FIND_ROOT_PATH_MODE_INCLUDE=BOTH \
    -DCMAKE_FIND_ROOT_PATH_MODE_LIBRARY=BOTH \
    -DEMSCRIPTEN_SYSTEM_PROCESSOR=arm \
    -DSTEAMAUDIO_BUILD_TESTS=FALSE -DSTEAMAUDIO_BUILD_BENCHMARKS=FALSE \
    -DSTEAMAUDIO_BUILD_SAMPLES=FALSE -DSTEAMAUDIO_BUILD_ITESTS=FALSE \
    -DSTEAMAUDIO_BUILD_DOCS=FALSE
  cmake --build "$cache/steam-audio-threaded" --target phonon -j8
fi

single_tracer_target="$cache/obvhs-tracer-target-single"
CARGO_TARGET_DIR="$single_tracer_target" \
RUSTFLAGS='-C target-feature=+simd128' \
  cargo build --manifest-path "$prototype/obvhs-tracer/Cargo.toml" \
  --release --target wasm32-unknown-emscripten \
  -Zbuild-std=core,alloc,std,panic_abort
single_tracer_library="$single_tracer_target/wasm32-unknown-emscripten/release/libafterglow_obvhs_tracer.a"

pthread_tracer_target="$cache/obvhs-tracer-target-pthread"
CARGO_TARGET_DIR="$pthread_tracer_target" \
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals,+simd128' \
  cargo build --manifest-path "$prototype/obvhs-tracer/Cargo.toml" \
  --release --target wasm32-unknown-emscripten \
  -Zbuild-std=core,alloc,std,panic_abort
pthread_tracer_library="$pthread_tracer_target/wasm32-unknown-emscripten/release/libafterglow_obvhs_tracer.a"

pthread_flags=(
  -pthread
  -sPTHREAD_POOL_SIZE=2
  -sPTHREAD_POOL_SIZE_STRICT=2
)

mkdir -p "$prototype/dist"
em++ "$prototype/benchmark.cpp" \
  -I "$sdk" \
  -I "$prototype/obvhs-tracer/include" \
  "$single_tracer_library" \
  "$sdk/libphonon.a" \
  "$single_deps/libmysofa.a" \
  "$single_deps/libpffft.a" \
  "$single_deps/libz.a" \
  -O3 -msimd128 \
  -sMODULARIZE=1 -sEXPORT_ES6=1 -sENVIRONMENT=worker \
  -sALLOW_MEMORY_GROWTH=0 -sINITIAL_MEMORY=67108864 -sNO_EXIT_RUNTIME=1 \
  -sEXPORTED_FUNCTIONS='["_sa_init","_sa_set_occluded","_sa_run_direct","_sa_run_direct_batch","_sa_get_occlusion","_sa_get_transmission_low","_sa_get_transmission_mid","_sa_get_transmission_high","_sa_get_tracer_nodes","_sa_get_tracer_build_ms","_sa_shutdown"]' \
  -o "$prototype/dist/steam-audio.js"

em++ "$prototype/dynamic-benchmark.cpp" \
  -I "$sdk" \
  -I "$prototype/obvhs-tracer/include" \
  "$pthread_tracer_library" \
  "$threaded" \
  "$pthread_deps/libmysofa.a" \
  "$pthread_deps/libpffft.a" \
  "$pthread_deps/libz.a" \
  -O3 -msimd128 "${pthread_flags[@]}" \
  -sMODULARIZE=1 -sEXPORT_ES6=1 -sENVIRONMENT=worker \
  -sALLOW_MEMORY_GROWTH=0 -sINITIAL_MEMORY=268435456 -sNO_EXIT_RUNTIME=1 \
  -sEXPORTED_FUNCTIONS='["_dyn_init","_dyn_update","_dyn_run_reflections","_dyn_run_audio","_dyn_run_binaural","_dyn_get_reverb_low","_dyn_get_reverb_mid","_dyn_get_reverb_high","_dyn_get_ir_valid","_dyn_get_output_energy","_dyn_get_tracer_nodes","_dyn_get_tracer_build_ms","_dyn_get_tracer_owned_bytes","_dyn_get_simulation_threads","_dyn_get_tracer_lanes","_dyn_shutdown"]' \
  -o "$prototype/dist/dynamic-steam-audio.js"

# The full-resolution Bistro stress test has a separate large fixed memory. It
# is not loaded by the normal prototype and never grows after worker bootstrap.
em++ "$prototype/bistro-benchmark.cpp" \
  -I "$sdk" \
  -I "$prototype/obvhs-tracer/include" \
  "$pthread_tracer_library" \
  "$threaded" \
  "$pthread_deps/libmysofa.a" \
  "$pthread_deps/libpffft.a" \
  "$pthread_deps/libz.a" \
  -O3 -msimd128 "${pthread_flags[@]}" \
  -sMODULARIZE=1 -sEXPORT_ES6=1 -sENVIRONMENT=worker \
  -sALLOW_MEMORY_GROWTH=0 -sINITIAL_MEMORY=1610612736 -sNO_EXIT_RUNTIME=1 \
  -sEXPORTED_RUNTIME_METHODS='["HEAPU8"]' \
  -sEXPORTED_FUNCTIONS='["_malloc","_free","_bistro_init","_bistro_run_reflections","_bistro_get_vertices","_bistro_get_triangles","_bistro_get_tracer_nodes","_bistro_get_tracer_build_ms","_bistro_get_tracer_owned_bytes","_bistro_get_reverb_low","_bistro_get_ir_valid","_bistro_get_simulation_threads","_bistro_get_tracer_lanes","_bistro_shutdown"]' \
  -o "$prototype/dist/bistro-steam-audio.js"

bun build "$prototype/src/worker.ts" --outdir "$prototype/dist" --target browser
bun build "$prototype/src/main.ts" --outdir "$prototype/dist" --target browser
bun build "$prototype/src/dynamic-worker.ts" --outdir "$prototype/dist" --target browser
bun build "$prototype/src/dynamic-main.ts" --outdir "$prototype/dist" --target browser
bun build "$prototype/src/bistro-worker.ts" --outdir "$prototype/dist" --target browser
bun build "$prototype/src/bistro-main.ts" --outdir "$prototype/dist" --target browser
cp "$prototype/index.html" "$prototype/dist/index.html"
cp "$prototype/dynamic.html" "$prototype/dist/dynamic.html"
cp "$prototype/bistro.html" "$prototype/dist/bistro.html"
echo "built Steam Audio WASM prototype in ${prototype#$root/}/dist"
