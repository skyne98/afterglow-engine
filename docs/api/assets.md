# `afterglow-assets` API — confinement, MIME, and streaming byte sources

> Status: working; API checked against the 2026-07-22 source.

## Purpose

`afterglow-assets` owns the security-sensitive resource primitives shared by
the engine's two serving backends:

- [`guess_mime`]: MIME type from a path extension.
- [`AssetRoot`]: a canonicalized root reused across requests.
- [`resolve`]: one-shot secure resolution beneath an asset root.
- [`AssetSource`], [`FsSource`], and [`BytesSource`]: positional streaming bytes.
- the bounded single-range and multipart-range parsers.

The confinement module remains dependency-free and performs no file-content
reads. The crate as a whole does read content: native `FsSource` owns cached,
positional `pread`, while `BytesSource` wraps embedded bytes. It contains no HTTP
framework types; the shell and the web server adapt these primitives to their
own request APIs. URL paths are consistently percent-decoded as UTF-8.
Both resource backends consume it:

- the `afterglow-web` dev server (`crates/afterglow-web/src/dev_server.rs`, used
  by the `coep_server` example) serves files over plain HTTP — see
  [`web-shared-memory.md`](web-shared-memory.md).
- the native `afterglow-shell` host will consume `FsSource`/`BytesSource`
  through an equivalent asset-root loader once that gate lands (see
  `docs/implementation/shell-promotion-plan.md`).

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

## `AssetRoot` and `resolve`

```rust
pub struct AssetRoot(/* canonical path */);
pub fn AssetRoot::new(root: impl AsRef<Path>) -> Option<AssetRoot>;
pub fn AssetRoot::resolve(&self, url_path: &str) -> Option<PathBuf>;
```

Request handlers should construct `AssetRoot` once. The free `resolve` function
is a convenience API that canonicalizes the root on each call.


```rust
pub fn resolve(root: &std::path::Path, url_path: &str) -> Option<std::path::PathBuf>
```

Resolve a URL/scheme path beneath `root` into a canonical, confined `PathBuf`.
This is the single security boundary for serving filesystem assets. It:

- **strips a `?query` string** and percent-decodes the path as UTF-8;
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

## Resolution, streaming reads, and 404 policy

`resolve` does no I/O beyond `canonicalize`. Callers may open the returned path
themselves, but serving code should normally use `AssetRoot::open_source()` or
the fixed-capacity `AssetSourceCache` documented in
[`asset-system.md`](asset-system.md):

```rust
use afterglow_assets::{guess_mime, resolve};
use std::path::Path;

let root = afterglow_assets::AssetRoot::new("/srv/assets").unwrap();
match root.resolve("/index.html?v=2") {
    Some(path) => {
        let mime = guess_mime("/index.html");
        // Low-level confinement example. Serving adapters normally stream this
        // path through FsSource instead of buffering it with std::fs::read.
        let body = std::fs::read(&path)?;
        // serve `body` as `mime`
    }
    None => /* answer 404 — do not distinguish miss/escape/unreadable */,
}
```

Every failure mode (traversal escape, root-only path, missing file, unreadable
path, symlink escape) collapses to `None`. Callers should answer a uniform 404
for `None` and **never** report which check failed, to avoid leaking filesystem
layout. Both backends follow this: the web dev server returns `404 Not Found`.

## Cross-links

- [`web-shared-memory.md`](web-shared-memory.md) — the `coep_server` dev server
  shares the same resolution path and COOP/COEP requirements.
