# Serving Assets

The native shell serves assets through the `afterglow://local/` custom scheme —
not over HTTP. One asset is **embedded** (`.index_html(...)`, ships in the
binary); everything else is **filesystem** (`.fs_root(...)`, streamed from disk).
Both paths stream via `AssetSource::read_at` and support `Range` requests — no
whole-file buffering. See [The Asset System](./asset-system.md) for the
streaming primitive.

## Why a scheme, not `file://` or HTTP

- **`file://` can't load Three.js.** Three.js is an ES module; a `file:` page
  can't do cross-file ES-module imports (CORS treats `file:` origins as
  unique). So you can't `import` Three.js from `file://`.
- **A localhost HTTP server works but funnels every asset through a TCP
  socket.** The custom scheme serves bytes straight from the binary or the
  filesystem, same-origin and secure-context, with no socket.

`afterglow://` is registered as standard + secure + CORS + fetch +
CSP-bypassing, so ES-module imports, WebGPU, fetch, and inline scripts all
work on it.

## Providing assets

```rust
AppBuilder::new()
    .index_html(include_bytes!("../assets/index.html"))  // the one embedded asset
    .fs_root("crates/afterglow-web/www")                  // everything else, streamed
    .run();
```

- **`.index_html(bytes)`** — embed `index.html` (the entry page) directly in
  the binary via `include_bytes!`. This is the **only** embedded asset.
- **`.fs_root(dir)`** — serve everything else from a filesystem directory at
  runtime, streamed via `pread` (no whole-file load). The directory is
  canonically confined (see below).
- If `.index_html()` is unset, `index.html` is also served from `fs_root`.

Why only one embedded asset? Large assets (Three.js, textures, wasm, models)
bloat the binary if embedded. The filesystem path streams them in whatever
chunk the caller asks for, so a multi-GB video or huge streaming world is
served from disk without loading it whole.

## Resolution order

Per request, the handler:

1. **Strip & decode.** Remove `afterglow://local` and any `?query`, then
   percent-decode the UTF-8 path.
2. **Embedded-first.** If the path matches the `.index_html()` entry, serve
   its bytes via `BytesSource` (200).
3. **FS fallback.** Else if `.fs_root(..)` is set, resolve via
   `afterglow_assets::AssetRoot::open_source` (canonically confined) and
   stream via `FsSource::read_at`; MIME from `afterglow_assets::guess_mime`.
4. **Else 404.** Missing, escaped, unreadable, or no FS root → `404 not found`
   as `text/plain`. Malformed request → `400`.

## Streaming & ranges

Both the embedded and FS paths stream through `AssetSource::read_at` — the
CEF handler's `read` serves chunks straight from disk (or the embedded
bytes), never buffering the whole file. The handler emits `Accept-Ranges:
bytes` and implements CEF's `skip()` for range support, so a client can
request partial content at arbitrary offsets (e.g. a video player scrubbing).

See [The Asset System](./asset-system.md) for the `AssetSource` trait,
`FsSource`/`BytesSource`, and the range parser.

## MIME types

`afterglow_assets::guess_mime(path)` guesses from the extension
(case-insensitive). Covered types:

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

Unknown or missing extensions fall back to `application/octet-stream`.

## Confinement (the security boundary)

The FS-fallback path is canonically confined — it:

- strips `?query` and percent-decodes the path as UTF-8;
- splits on both `/` and `\` (backslashes can't smuggle traversal);
- ignores `.` and empty segments;
- allows `..` that stays within `root`, rejects any `..` that would pop above it;
- rejects empty/root-only paths (no auto directory index);
- **requires the target to exist and canonicalize**, and **rejects symlinks (or
  any resolved path) that escape the canonical `root`**.

Confinement is enforced *canonically*, not lexically: both `root` and the joined
candidate are `canonicalize()`d, and the candidate must `starts_with` the
canonical root. A `../`-free path through a symlink to `/etc/passwd`
canonicalizes outside `root` and is rejected. Every failure mode collapses to a
404; the handler never reports which check failed.

## COOP/COEP

Every response carries:

```http
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
Cross-Origin-Resource-Policy: same-origin
```

These enable `SharedArrayBuffer` on `afterglow://` pages. Verify in the JS
console: `self.crossOriginIsolated === true`.

## Next

- [The Asset System](./asset-system.md) — the streaming `AssetSource` primitive,
  range parsing, and where the code lives.
- [Graphics & DevTools](./graphics.md) — the GPU flags and debugging.
- [The AppBuilder API](./app-builder.md) — the full builder reference.
