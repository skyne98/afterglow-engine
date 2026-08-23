#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
MP=paint/vendor/libmypaint

cc -O2 -std=c11 $(pkg-config --cflags json-c) \
  -Ipaint -I"$MP" \
  paint/mypaint-brush-cooperative.test.c \
  paint/mypaint-brush-cooperative.c \
  "$MP/mypaint.c" \
  "$MP/mypaint-mapping.c" \
  "$MP/mypaint-brush-settings.c" \
  "$MP/helpers.c" \
  "$MP/rng-double.c" \
  "$MP/brushmodes.c" \
  paint/fixed-operationqueue.c \
  "$MP/mypaint-rectangle.c" \
  "$MP/mypaint-matrix.c" \
  "$MP/mypaint-symmetry.c" \
  "$MP/mypaint-surface.c" \
  "$MP/mypaint-tiled-surface.c" \
  paint/web-surface.c \
  $(pkg-config --libs json-c) -lm -pthread \
  -o /tmp/mypaint-cooperative-test
/tmp/mypaint-cooperative-test
rm -f /tmp/mypaint-cooperative-test
