#!/usr/bin/env bash
# Fetch the rusty_v8 (v8) prebuilt static lib that deno_core 0.408 needs.
# deno_core 0.408 depends on v8 = "149.4.0" with the `simdutf` feature, so we
# need the *simdutf* variant -- a superset archive containing both the v8__
# and simdutf__ symbols. Do NOT declare a direct `v8` dep in the crate, or
# Cargo resolves a 2nd v8 version and one archive can't satisfy both.
URL=https://github.com/denoland/rusty_v8/releases/download/v149.4.0/librusty_v8_simdutf_release_x86_64-unknown-linux-gnu.a.gz
DEST=/tmp/v149_simdutf.a.gz
# Resumable, follow redirects, retry hard (GitHub can truncate).
curl -L -C - --retry 20 --retry-delay 2 --retry-all-errors -o "$DEST" "$URL" 2>&1 | tail -3
echo "final size: $(stat -c%s "$DEST" 2>/dev/null)"
gzip -t "$DEST" 2>&1 && echo "gzip OK" || echo "gzip CORRUPT"
gzip -dc "$DEST" > /tmp/v149.a
echo "decompressed -> /tmp/v149.a ($(stat -c%s /tmp/v149.a) bytes)"
echo "use: export RUSTY_V8_ARCHIVE=/tmp/v149.a"
