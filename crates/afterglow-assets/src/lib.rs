//! Shared asset-path/MIME helpers for afterglow-engine.
//!
//! Single owner of the security-sensitive resource logic the engine's two
//! resource servers need:
//!
//! - [`guess_mime`]: MIME type from a path extension.
//! - [`AssetRoot`]: a canonicalized root reused across requests.
//! - [`resolve`]: one-shot secure resolution beneath an asset root.
//! - [`AssetSource`](source::AssetSource): a positional, streaming byte source
//!   (`FsSource` reads from disk via `pread`; `BytesSource` wraps an embedded
//!   `&'static [u8]`). Both serving backends stream through this trait — no
//!   whole-file buffering.
//! - [`range::parse_range`]: single-range HTTP `Range` parsing shared by both
//!   backends.
//!
//! No third-party dependencies. The confinement logic (`resolve`) performs
//! canonicalization but no content reads; the streaming sources (`source`
//! module) own the reads. Every miss, escape, and unreadable path maps to
//! `None` so callers can answer a uniform 404 without leaking which check
//! failed.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

pub mod range;
pub mod source;

pub use source::{AssetSource, BytesSource};
#[cfg(not(target_arch = "wasm32"))]
pub use source::FsSource;

/// Guess a MIME type from a path's extension.
///
/// Extension matching is case-insensitive. Covers the asset types the engine
/// serves today: HTML (`.html`/`.htm`), JavaScript (`.js`/`.mjs`), CSS, JSON,
/// WASM, the common web image formats (PNG/JPEG/SVG), and WOFF2 fonts. Unknown
/// or missing extensions fall back to `application/octet-stream`.
pub fn guess_mime(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("html" | "htm") => "text/html",
        Some("js" | "mjs") => "text/javascript",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// A canonical asset root. Construct it once and reuse it for requests, avoiding
/// repeated root canonicalization while retaining symlink confinement.
#[derive(Debug, Clone)]
pub struct AssetRoot(PathBuf);

impl AssetRoot {
    /// Canonicalize an existing asset directory.
    pub fn new(root: impl AsRef<Path>) -> Option<Self> {
        root.as_ref().canonicalize().ok().map(Self)
    }

    /// Resolve a URL path beneath this root.
    pub fn resolve(&self, url_path: &str) -> Option<PathBuf> {
        resolve_canonical(&self.0, url_path)
    }

    /// Resolve and open a URL path as a streaming [`FsSource`] (native only).
    /// Returns `None` if the path is missing/escaped/unreadable — callers
    /// answer a uniform 404.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_source(&self, url_path: &str) -> Option<FsSource> {
        let path = self.resolve(url_path)?;
        FsSource::open(path)
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Percent-decode a URL path as UTF-8. Invalid escapes and invalid UTF-8 are
/// rejected instead of being interpreted inconsistently by different backends.
pub fn decode_url_path(path: &str) -> Option<Cow<'_, str>> {
    if !path.as_bytes().contains(&b'%') {
        return Some(Cow::Borrowed(path));
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = hex(*bytes.get(i + 1)?)?;
            let lo = hex(*bytes.get(i + 2)?)?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok().map(Cow::Owned)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Resolve a URL/scheme path beneath `root` into a canonical, confined path.
///
/// This convenience function canonicalizes `root` on each call. Request
/// handlers should construct and reuse [`AssetRoot`] instead.
///
/// This is the single security boundary for serving filesystem assets. It:
///
/// - strips a `?query` string, if present;
/// - splits on both `/` and `\` separators;
/// - ignores `.` and empty segments;
/// - allows `..` that stays within `root`, but rejects any `..` that would pop
///   above `root` (traversal escape);
/// - rejects empty or root-only paths (`""`, `/`, `/./`);
/// - requires the target to exist and canonicalize, so missing/unreadable
///   paths yield `None`;
/// - rejects symlinks (or any resolved path) that escape the canonical `root`.
///
/// Returns the canonical, in-root [`PathBuf`] on success, else `None`. Reading
/// the bytes is left to the caller; a successful result is guaranteed confined.
pub fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
    AssetRoot::new(root)?.resolve(url_path)
}

fn resolve_canonical(root: &Path, url_path: &str) -> Option<PathBuf> {
    let path = url_path.split_once('?').map(|(p, _)| p).unwrap_or(url_path);
    let decoded = decode_url_path(path)?;
    let rel = decoded.strip_prefix('/').unwrap_or(&decoded);
    let mut acc = PathBuf::new();
    for seg in rel.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                if !acc.pop() {
                    return None;
                }
            }
            other => acc.push(other),
        }
    }
    if acc.as_os_str().is_empty() {
        return None;
    }
    confine(root, &root.join(acc))
}

/// Canonically confine `candidate` within an already-canonical `root`.
fn confine(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let cand_c = candidate.canonicalize().ok()?;
    cand_c.starts_with(root).then_some(cand_c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// A scratch directory removed on drop. Unique per call so parallel tests
    /// never collide. No third-party `tempfile` dependency, matching the
    /// pattern already used in `afterglow-cef` and `afterglow-web`.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Scratch {
            use std::sync::atomic::{AtomicU64, Ordering};
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("ag-assets-{}-{}", std::process::id(), n));
            let _ = std::fs::remove_dir_all(&dir); // idempotent if a prior run leaked
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl std::ops::Deref for Scratch {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn guess_mime_common_types() {
        assert_eq!(guess_mime("x.html"), "text/html");
        assert_eq!(guess_mime("x.htm"), "text/html");
        assert_eq!(guess_mime("a/b/c.js"), "text/javascript");
        assert_eq!(guess_mime("mod.mjs"), "text/javascript");
        assert_eq!(guess_mime("s.css"), "text/css");
        assert_eq!(guess_mime("d.json"), "application/json");
        assert_eq!(guess_mime("mod.wasm"), "application/wasm");
        assert_eq!(guess_mime("i.png"), "image/png");
        assert_eq!(guess_mime("i.jpg"), "image/jpeg");
        assert_eq!(guess_mime("i.jpeg"), "image/jpeg");
        assert_eq!(guess_mime("i.svg"), "image/svg+xml");
        assert_eq!(guess_mime("f.woff2"), "font/woff2");
        assert_eq!(guess_mime("noext"), "application/octet-stream");
        // case-insensitive extensions
        assert_eq!(guess_mime("UPPER.PNG"), "image/png");
        assert_eq!(guess_mime("Mixed.WaSm"), "application/wasm");
    }

    #[test]
    fn decode_url_path_validates_escapes_and_utf8() {
        assert_eq!(decode_url_path("a%20b.js").as_deref(), Some("a b.js"));
        assert_eq!(decode_url_path("caf%C3%A9").as_deref(), Some("café"));
        assert!(decode_url_path("bad%2").is_none());
        assert!(decode_url_path("bad%zz").is_none());
        assert!(decode_url_path("%ff").is_none());
    }

    #[test]
    fn resolve_percent_encoded_file() {
        let s = Scratch::new();
        let root = s.join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a b.js"), b"hi").unwrap();
        let want = root.canonicalize().unwrap().join("a b.js");
        assert_eq!(resolve(&root, "/a%20b.js").as_deref(), Some(want.as_path()));
    }

    #[test]
    fn resolve_normal_nested_file() {
        let s = Scratch::new();
        let root = s.join("root");
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("index.html"), b"<html></html>").unwrap();
        std::fs::write(root.join("assets/app.js"), b"console.log(1)").unwrap();

        let want = root.canonicalize().unwrap().join("index.html");
        assert_eq!(
            resolve(&root, "/index.html").as_deref(),
            Some(want.as_path())
        );

        let want = root.canonicalize().unwrap().join("assets/app.js");
        assert_eq!(
            resolve(&root, "/assets/app.js").as_deref(),
            Some(want.as_path())
        );
    }

    #[test]
    fn resolve_strips_query_string() {
        let s = Scratch::new();
        let root = s.join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), b"hi").unwrap();

        let want = root.canonicalize().unwrap().join("index.html");
        assert_eq!(
            resolve(&root, "/index.html?x=1&y=2").as_deref(),
            Some(want.as_path())
        );
        assert_eq!(
            resolve(&root, "index.html?v=2").as_deref(),
            Some(want.as_path())
        );
    }

    #[test]
    fn resolve_both_separator_styles() {
        let s = Scratch::new();
        let root = s.join("root");
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("assets/app.js"), b"hi").unwrap();

        let want = root.canonicalize().unwrap().join("assets/app.js");
        assert_eq!(
            resolve(&root, "/assets/app.js").as_deref(),
            Some(want.as_path())
        );
        assert_eq!(
            resolve(&root, "/assets\\app.js").as_deref(),
            Some(want.as_path())
        );
        assert_eq!(
            resolve(&root, "assets\\app.js").as_deref(),
            Some(want.as_path())
        );
        assert_eq!(
            resolve(&root, "\\assets\\app.js").as_deref(),
            Some(want.as_path())
        );
    }

    #[test]
    fn resolve_allows_in_root_parent() {
        let s = Scratch::new();
        let root = s.join("root");
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("real.html"), b"hi").unwrap();

        let want = root.canonicalize().unwrap().join("real.html");
        assert_eq!(
            resolve(&root, "/assets/../real.html").as_deref(),
            Some(want.as_path())
        );
        assert_eq!(
            resolve(&root, "/./real.html").as_deref(),
            Some(want.as_path())
        );
    }

    #[test]
    fn resolve_rejects_above_root_traversal() {
        let s = Scratch::new();
        let root = s.join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("dummy.html"), b"hi").unwrap();

        assert_eq!(resolve(&root, "/.."), None);
        assert_eq!(resolve(&root, "/../Cargo.toml"), None);
        assert_eq!(resolve(&root, "/../../../etc/passwd"), None);
        assert_eq!(resolve(&root, "/a/../../etc/passwd"), None);
        assert_eq!(resolve(&root, "/..\\..\\etc"), None);
        assert_eq!(resolve(&root, "/assets/../../../etc"), None);
    }

    #[test]
    fn resolve_rejects_missing_file() {
        let s = Scratch::new();
        let root = s.join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("real.html"), b"hi").unwrap();

        assert_eq!(resolve(&root, "/nope.html"), None);
        assert_eq!(resolve(&root, "/missing/deep.js"), None);
    }

    #[test]
    fn resolve_rejects_root_only_and_empty() {
        let s = Scratch::new();
        let root = s.join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("real.html"), b"hi").unwrap();

        assert_eq!(resolve(&root, "/"), None);
        assert_eq!(resolve(&root, ""), None);
        assert_eq!(resolve(&root, "/."), None);
        assert_eq!(resolve(&root, "/./"), None);
        assert_eq!(resolve(&root, "?x=1"), None);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let s = Scratch::new();
        let root = s.join("root");
        let outside = s.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"SECRET").unwrap();
        symlink(outside.join("secret.txt"), root.join("link.js")).unwrap();
        std::fs::write(root.join("real.html"), b"hi").unwrap();

        // symlink escapes root -> canonically confined out -> None
        assert_eq!(resolve(&root, "/link.js"), None);
        // a real in-root file still resolves
        let want = root.canonicalize().unwrap().join("real.html");
        assert_eq!(
            resolve(&root, "/real.html").as_deref(),
            Some(want.as_path())
        );
    }
}
