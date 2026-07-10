//! Shared asset-path/MIME helpers for afterglow-engine.
//!
//! Single owner of the two pieces of security-sensitive resource logic that
//! were previously duplicated by the CEF resource handler (`afterglow-cef`)
//! and the web dev server (`afterglow-web`):
//!
//! - [`guess_mime`]: MIME type from a path extension.
//! - [`resolve`]: secure resolution of a URL/scheme path beneath an asset root
//!   into a canonical, confined [`PathBuf`].
//!
//! Deliberately tiny: no third-party dependencies, no HTTP types, no
//! percent-decoder, and no file-content reads; resolution performs
//! canonicalization — callers read the returned path themselves. Every miss,
//! escape, and unreadable path maps to `None` so
//! callers can answer a uniform 404 without leaking which check failed.

use std::path::{Path, PathBuf};

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

/// Resolve a URL/scheme path beneath `root` into a canonical, confined path.
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
    let path = url_path.split_once('?').map(|(p, _)| p).unwrap_or(url_path);
    let rel = path.strip_prefix('/').unwrap_or(path);
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

/// Canonically confine `candidate` within `root`: both must canonicalize and
/// `candidate` must stay within `root`, so symlinks cannot escape. Missing or
/// unreadable paths fail canonicalize -> `None` (no content leak).
fn confine(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let root_c = root.canonicalize().ok()?;
    let cand_c = candidate.canonicalize().ok()?;
    cand_c.starts_with(root_c).then_some(cand_c)
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
