#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
PROFILE=${RUNTIME_GOLDEN_PROFILE:-nvidia-rtx3090}
ACTUAL_DIR=${1:-/tmp/runs}
GOLDEN_DIR="$ROOT/e2e/runtime_goldens/$PROFILE"
MANIFEST="$ROOT/e2e/runtime_goldens/$PROFILE.sha256"

if [[ ! -d "$GOLDEN_DIR" || ! -f "$MANIFEST" ]]; then
  echo "unknown runtime golden profile: $PROFILE" >&2
  exit 2
fi

failed=0
while read -r expected name; do
  actual="$ACTUAL_DIR/$name"
  if [[ ! -f "$actual" ]]; then
    printf 'MISSING\t%s\n' "$name"
    failed=1
    continue
  fi
  observed=$(sha256sum "$actual" | cut -d' ' -f1)
  if [[ "$observed" == "$expected" ]]; then
    printf 'PASS\t%s\n' "$name"
  else
    printf 'FAIL\t%s\texpected=%s\tobserved=%s\n' "$name" "$expected" "$observed"
    failed=1
  fi
done < "$MANIFEST"

exit "$failed"
