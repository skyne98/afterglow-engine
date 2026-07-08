#!/usr/bin/env bash
# Download the Three.js WebGPU build into resources/ (pinned version).
# These are build inputs for include_bytes! / include_str! in src/shared/simple_app.rs
# and are gitignored — re-run after a fresh clone or to bump the version.
set -euo pipefail
cd "$(dirname "$0")"

THREE_VERSION="${THREE_VERSION:-0.185.1}"   # bump here to upgrade Three.js
BASE="https://unpkg.com/three@${THREE_VERSION}/build"

echo "Downloading Three.js ${THREE_VERSION} WebGPU build into resources/ ..."
curl -fsSL -o three.core.js   "${BASE}/three.core.js"
curl -fsSL -o three.webgpu.js "${BASE}/three.webgpu.js"
echo "Done. three.core.js + three.webgpu.js -> $(pwd)"
