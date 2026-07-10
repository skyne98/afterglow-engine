# `afterglow-assets` API — shared asset-path/MIME helpers

> Status: working; API checked against the 2026-07-10 source.

## Purpose

`afterglow-assets` is the single owner of the two pieces of security-sensitive
resource logic the engine's two resource servers need:

- [`guess_mime`]: MIME type from a path extension.
- [`resolve`]: secure resolution of a URL/scheme path beneath an asset root
  into a canonical, confined `PathBuf`.

It is deliberately tiny: **no third-party dependencies, no HTTP types, no
percent-decoder, and no file-content reads; resolution performs canonicalization** — callers read the returned path themselves.
Both resource backends consume it:

- [`afterglow-cef`](cef-shell.md) serves embedded bytes and/or FS files through
  the `afterglow://local/` custom scheme;
- the `afterglow-web` dev server (`crates/afterglow-web/src/dev_server.rs`, used
  by the `coep_server` example) serves files over plain HTTP — see
  [`web-shared-memory.md`](web-shared-memory.md).

Every miss, escape, and unreadable path maps to `None` so callers answer a
uniform 404 without leaking which check failed.

## `guess_mime`

```rust
pub fn guess_mime(path: &str) -> &'static str
```

Guess a MIME type from a path's extension. Extension matching is
**case-insensitive**. Covers the asset types the engine serves today:

| Extension(s) | MIME |
|---|---|
| `html`, `htm` | `text/html` |
| `js`, `mjs` | `text/javascript` |
| `css` | `text/css` |
| `json` | `application/json` |
| `wasm` | `application/wasm` |
| `png` | `image/png` |
| `jpg`, `jpeg` | `image/jpeg` |
| `svg` | `image/svg+xml` |
| `woff2` | `font/woff2` |

Unknown or missing extensions fall back to `application/octet-stream`. The
extension is the substring after the last `.`; a path with no dot has no
extension and also falls back to `application/octet-stream`.

## `resolve`

```rust
pub fn resolve(root: &std::path::Path, url_path: &str) -> Option<std::path::PathBuf>
```

Resolve a URL/scheme path beneath `root` into a canonical, confined `PathBuf`.
This is the single security boundary for serving filesystem assets. It:

- **strips a `?query` string**, if present (the path before `?` is used);
- **splits on both `/` and `\`** separators, so Windows-style backslashes and
  mixed separators cannot smuggle traversal;
- **ignores `.` and empty segments**;
- **allows `..` that stays within `root`**, but rejects any `..` that would pop
  above `root` (traversal escape → `None`);
- **rejects empty or root-only paths** (`""`, `/`, `/./`, `?x=1`) — the engine
  never auto-serves a directory index;
- **requires the target to exist and canonicalize**, so missing/unreadable
  paths yield `None`;
- **rejects symlinks (or any resolved path) that escape the canonical `root`**.

Returns the canonical, in-root `PathBuf` on success, else `None`. A successful
result is guaranteed confined; reading the bytes is left to the caller.

### Why canonicalization, not lexical checks

Confinement is enforced *canonically*, not lexically: both `root` and the
joined candidate are `canonicalize()`d, and the candidate must `starts_with`
the canonical root. This is what defeats symlinks — a `../`-free path that
points through a symlink to `/etc/passwd` canonicalizes outside `root` and is
rejected. The cost is that **the file must exist** to be resolved
(`canonicalize` fails on missing paths), which is the desired 404 behavior
anyway.

## Caller-owned reads and 404 policy

`resolve` does no I/O beyond `canonicalize`. Callers own the actual byte read:

```rust
use afterglow_assets::{guess_mime, resolve};
use std::path::Path;

let root = Path::new("/srv/assets");
match resolve(root, "/index.html?v=2") {
    Some(path) => {
        let mime = guess_mime("/index.html");
        let body = std::fs::read(&path)?; // caller decides error policy
        // serve `body` as `mime`
    }
    None => /* answer 404 — do not distinguish miss/escape/unreadable */,
}
```

Every failure mode (traversal escape, root-only path, missing file, unreadable
path, symlink escape) collapses to `None`. Callers should answer a uniform 404
for `None` and **never** report which check failed, to avoid leaking filesystem
layout. Both backends follow this: CEF returns `404 not found` as `text/plain`;
the web dev server returns `404 Not Found`.

## Cross-links

- [`cef-shell.md`](cef-shell.md) — the `afterglow://local/` resource handler that
  calls `resolve` / `guess_mime` for the FS-fallback path.
- [`web-shared-memory.md`](web-shared-memory.md) — the `coep_server` dev server
  shares the same resolution path and COOP/COEP requirements.
