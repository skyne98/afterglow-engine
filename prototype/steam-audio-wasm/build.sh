#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
prototype="$root/prototype/steam-audio-wasm"
cache="$root/target/steam-audio-wasm-build"
upstream="$cache/steam-audio"
sdk="$cache/sdk"
version=v4.8.1

for command in em++ python3 cmake git curl unzip bun; do
  command -v "$command" >/dev/null || { echo "missing build command: $command" >&2; exit 1; }
done

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

pushd "$upstream/core/build" >/dev/null
for dependency in zlib pffft mysofa; do
  library="$upstream/core/deps/$dependency/lib/wasm/release/lib${dependency}.a"
  [[ "$dependency" == zlib ]] && library="$upstream/core/deps/zlib/lib/wasm/release/libz.a"
  if [[ ! -f "$library" ]]; then
    python3 get_dependencies.py --platform wasm --emsdk "$cache/emsdk" --dependency "$dependency"
  fi
done
popd >/dev/null

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

mkdir -p "$prototype/dist"
em++ "$prototype/benchmark.cpp" \
  -I "$sdk" \
  "$sdk/libphonon.a" \
  "$upstream/core/deps/mysofa/lib/wasm/release/libmysofa.a" \
  "$upstream/core/deps/pffft/lib/wasm/release/libpffft.a" \
  "$upstream/core/deps/zlib/lib/wasm/release/libz.a" \
  -O3 -msimd128 \
  -sMODULARIZE=1 -sEXPORT_ES6=1 -sENVIRONMENT=worker \
  -sALLOW_MEMORY_GROWTH=0 -sINITIAL_MEMORY=67108864 -sNO_EXIT_RUNTIME=1 \
  -sEXPORTED_FUNCTIONS='["_sa_init","_sa_set_occluded","_sa_run_direct","_sa_run_direct_batch","_sa_get_occlusion","_sa_get_transmission_low","_sa_get_transmission_mid","_sa_get_transmission_high","_sa_shutdown"]' \
  -o "$prototype/dist/steam-audio.js"

bun build "$prototype/src/worker.ts" --outdir "$prototype/dist" --target browser
bun build "$prototype/src/main.ts" --outdir "$prototype/dist" --target browser
cp "$prototype/index.html" "$prototype/dist/index.html"
echo "built Steam Audio WASM prototype in ${prototype#$root/}/dist"
