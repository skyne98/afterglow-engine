#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
prototype="$root/prototype/steam-audio-wasm"
cache="$root/target/steam-audio-native-build"
sdk="$cache/sdk"
dist="$prototype/dist-native"
version=v4.8.1
archive="$cache/steamaudio.zip"

for command in c++ curl unzip; do
  command -v "$command" >/dev/null || { echo "missing build command: $command" >&2; exit 1; }
done

if [[ ! -f "$sdk/libphonon.so" || ! -f "$sdk/phonon_version.h" ]]; then
  mkdir -p "$sdk"
  curl -L --fail --silent --show-error -o "$archive" \
    "https://github.com/ValveSoftware/steam-audio/releases/download/${version}/steamaudio_4.8.1.zip"
  unzip -j -o "$archive" \
    steamaudio/lib/linux-x64/libphonon.so \
    steamaudio/include/phonon.h \
    steamaudio/include/phonon_version.h \
    -d "$sdk" >/dev/null
fi

mkdir -p "$dist"
c++ -std=c++20 -O3 -march=x86-64-v3 -pthread \
  -I "$sdk" \
  "$prototype/dynamic-benchmark.cpp" \
  "$prototype/native-dynamic-benchmark.cpp" \
  -L "$sdk" -lphonon \
  -Wl,-rpath,'$ORIGIN' \
  -o "$dist/native-dynamic-benchmark"
cp "$sdk/libphonon.so" "$dist/libphonon.so"
echo "built native Steam Audio benchmark in ${dist#$root/}"
