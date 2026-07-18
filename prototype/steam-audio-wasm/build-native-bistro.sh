#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
prototype="$root/prototype/steam-audio-wasm"
source_dir="$root/target/bistro-source"
native_cache="$root/target/steam-audio-native-build/sdk"
dist="$prototype/dist-native"
archive="${BISTRO_ZIP:-$source_dir/Bistro_v5_2.zip}"
fbx="${BISTRO_FBX:-$source_dir/BistroInterior.fbx}"
geometry="$source_dir/BistroInterior.acoustic.bin"

for command in c++ curl unzip; do
  command -v "$command" >/dev/null || { echo "missing build command: $command" >&2; exit 1; }
done

"$prototype/build-native.sh"

if [[ ! -f "$fbx" ]]; then
  mkdir -p "$source_dir"
  if [[ ! -f "$archive" ]]; then
    echo "downloading official CC-BY 4.0 Amazon Lumberyard Bistro v5.2 archive" >&2
    curl -L --fail --show-error -o "$archive" https://developer.nvidia.com/bistro
  fi
  unzip -j -o "$archive" \
    Bistro_v5_2/BistroInterior.fbx \
    Bistro_v5_2/LICENSE.txt \
    Bistro_v5_2/README.txt \
    -d "$source_dir" >/dev/null
fi

c++ -std=c++20 -O3 "$prototype/cook-bistro-acoustic.cpp" \
  -lassimp -o "$dist/cook-bistro-acoustic"
"$dist/cook-bistro-acoustic" "$fbx" "$geometry"

c++ -std=c++20 -O3 -march=x86-64-v3 -pthread \
  -I "$native_cache" \
  "$prototype/native-bistro-geometry-benchmark.cpp" \
  -L "$native_cache" -lphonon \
  -Wl,-rpath,'$ORIGIN' \
  -o "$dist/native-bistro-geometry-benchmark"

echo "built Bistro benchmark; geometry: ${geometry#$root/}"
