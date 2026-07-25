#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
VENDOR_ROOT=${VENDOR_ROOT:-"$ROOT/vendor"}
VERSIONS_FILE="$ROOT/vendor/versions.env"
PATCH_ROOT="$ROOT/vendor/patches"

# shellcheck source=/dev/null
source "$VERSIONS_FILE"
DENO_VERSION=${1:-$DENO_WEBGPU_VERSION}
NAGA_VERSION_REQUESTED=${2:-$NAGA_VERSION}

for command in curl tar patch; do
  command -v "$command" >/dev/null || {
    echo "error: required command not found: $command" >&2
    exit 1
  }
done

work=$(mktemp -d "${TMPDIR:-/tmp}/three-native-webgpu-vendor.XXXXXX")
trap 'rm -rf "$work"' EXIT

stage_crate() {
  local crate=$1
  local version=$2
  local patch_file=$3
  local destination=$4
  local archive="$work/$crate-$version.crate"
  local stage="$work/$crate"

  echo "[vendor] downloading $crate $version"
  curl --fail --location --silent --show-error \
    --user-agent "afterglow-shell-vendor/1.0" \
    "https://crates.io/api/v1/crates/$crate/$version/download" \
    --output "$archive"

  mkdir -p "$stage"
  tar -xzf "$archive" -C "$stage" --strip-components=1

  echo "[vendor] applying $(basename "$patch_file")"
  if ! patch --batch --forward --directory="$stage" -p1 < "$patch_file"; then
    echo "error: $crate $version no longer accepts the local patch" >&2
    echo "       Existing vendored sources were not changed." >&2
    echo "       Check whether upstream implemented the feature, then update" >&2
    echo "       or remove $patch_file." >&2
    exit 1
  fi

  printf '%s\t%s\n' "$stage" "$destination" >> "$work/install-plan"
}

stage_crate \
  deno_webgpu "$DENO_VERSION" \
  "$PATCH_ROOT/deno_webgpu-native-features.patch" \
  "$VENDOR_ROOT/deno_webgpu"
stage_crate \
  naga "$NAGA_VERSION_REQUESTED" \
  "$PATCH_ROOT/naga-subgroups.patch" \
  "$VENDOR_ROOT/naga"

# Both downloads and patches succeeded. Replace each tree only now.
while IFS=$'\t' read -r stage destination; do
  rm -rf "$destination"
  mv "$stage" "$destination"
done < "$work/install-plan"

cat > "$VENDOR_ROOT/versions.env" <<EOF
# Versions of locally patched Rust crates. Used by
# scripts/update_vendored_webgpu.sh when no explicit versions are supplied.
DENO_WEBGPU_VERSION=$DENO_VERSION
NAGA_VERSION=$NAGA_VERSION_REQUESTED
EOF

cat <<EOF
[vendor] installed deno_webgpu $DENO_VERSION and naga $NAGA_VERSION_REQUESTED
[vendor] next: cargo check -p afterglow-shell --example browser_test
EOF
