#!/usr/bin/env bash
# Vendor the three.js source (build + addons + reference screenshots) so the
# browser_test module loader can read from disk (fast, no HTTP-in-Rust).
# One-time download (~360MB tarball); extracts only the dirs we need.
set -e
DEST=${1:-/tmp/threejs}
URL="https://github.com/mrdoob/three.js/archive/refs/heads/master.tar.gz"
echo "downloading three.js tarball..."
curl -sL --retry 3 -o /tmp/threejs.tar.gz "$URL"
echo "extracting build + examples/jsm + examples/screenshots -> $DEST"
mkdir -p "$DEST"
tar xzf /tmp/threejs.tar.gz -C "$DEST" --strip-components=1 \
  three.js-master/build \
  three.js-master/examples/jsm \
  three.js-master/examples/screenshots \
  three.js-master/examples/webgpu_clipping.html
echo "vendored: $(ls $DEST/build/three.webgpu.js $DEST/examples/jsm $DEST/examples/screenshots/webgpu_clipping.jpg | wc -l) items"
