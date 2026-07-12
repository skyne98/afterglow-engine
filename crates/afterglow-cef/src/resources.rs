//! Direct resource serving via a CEF custom scheme (`afterglow://local/`).
//!
//! Resources (HTML/JS/WASM/...) are served from the one embedded asset
//! (`index.html`) or a filesystem root through CEF's internal request routing
//! — no localhost HTTP server on the resource path. The scheme is registered
//! standard + secure + CORS + fetch + CSP-bypassing, so ES-module imports,
//! WebGPU, and inline scripts work.
//!
//! Serving is **streaming**: both the embedded [`BytesSource`] and the
//! filesystem [`FsSource`] implement [`AssetSource::read_at`], so `read` serves
//! chunks straight from disk (no whole-file buffering) and `skip` handles
//! range requests by advancing the read offset.
//!
//! Path/MIME/status resolution is shared with the web dev server via
//! [`afterglow_assets`] (`guess_mime` + `resolve` + `parse_range`); this module
//! owns only the CEF-specific policy — embedded-first lookup, optional FS
//! fallback, and the 200/400/404 status mapping — plus the thin
//! `ResourceHandler` wrapper holding one mutex-protected response state
//! (source/mime/status/offset).

use afterglow_assets::{AssetRoot, BytesSource, decode_url_path, guess_mime};
use afterglow_assets::source::AssetSource;
use cef::*;
use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use crate::config::{CONFIG, SCHEME, SCHEME_DOMAIN};

/// Strip the scheme, host, and query while preserving URL encoding.
fn path_of(url: &str) -> Cow<'_, str> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = rest.split_once('/').map_or("", |(_, path)| path);
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    Cow::Owned(format!("/{path}"))
}

/// A boxed streaming source. `FsSource` (disk) or `BytesSource` (embedded
/// `index.html`). Errors (404/400) use `BytesSource` on static bytes.
type BoxedSource = Box<dyn AssetSource + Send + Sync>;

/// Outcome of resolving a scheme path: a streaming source + mime (200) or a
/// miss (404). Takes `embedded` and `fs_root` explicitly so tests can drive it
/// without the global [`CONFIG`].
enum Resolved {
    Found { source: BoxedSource, mime: String, etag: Option<String> },
    NotFound,
}

/// The single embedded asset entry: `(path, mime, bytes)`. Today only
/// `index.html` is embedded; the general `.asset(...)` mechanism is gone.
fn resolve_scheme_path(
    path: &str,
    embedded: &[(String, String, &'static [u8])],
    fs_root: Option<&AssetRoot>,
) -> Resolved {
    let Some(decoded) = decode_url_path(path) else {
        return Resolved::NotFound;
    };
    // Embedded-first (the one embedded asset: index.html).
    if let Some((_, mime, bytes)) = embedded.iter().find(|(p, _, _)| p == decoded.as_ref()) {
        let src = BytesSource(bytes);
        return Resolved::Found {
            source: Box::new(src),
            mime: mime.clone(),
            etag: None,
        };
    }
    // FS fallback: stream from disk via pread (no whole-file load).
    if let Some(root) = fs_root
        && let Some(src) = root.open_source(path)
    {
        let etag = src.etag();
        return Resolved::Found {
            source: Box::new(src),
            mime: guess_mime(&decoded).to_string(),
            etag,
        };
    }
    Resolved::NotFound
}

/// Per-response state behind a single lock. Reset on every `open` so a reused
/// handler can never leak a prior response.
struct ResponseState {
    source: Option<BoxedSource>,
    mime: String,
    status: i32,
    offset: u64,
    len: u64,
    etag: Option<String>,
}

impl Default for ResponseState {
    fn default() -> Self {
        Self {
            source: None,
            mime: String::new(),
            status: 0,
            offset: 0,
            len: 0,
            etag: None,
        }
    }
}

wrap_resource_handler! {
    struct AfterglowResource {
        state: Arc<Mutex<ResponseState>>,
    }

    impl ResourceHandler {
        fn open(&self, request: Option<&mut Request>, handle_request: Option<&mut ::std::os::raw::c_int>, _callback: Option<&mut Callback>) -> ::std::os::raw::c_int {
            let (source, mime, status, len, etag) = match request {
                Some(req) => {
                    let cfg = CONFIG.get().expect("afterglow-cef config not set");
                    let url = CefString::from(&req.url()).to_string();
                    match resolve_scheme_path(&path_of(&url), &cfg.embedded, cfg.asset_root.as_ref()) {
                        Resolved::Found { source, mime, etag } => {
                            let len = source.len();
                            (Some(source), mime, 200, len, etag)
                        }
                        Resolved::NotFound => (
                            Some(Box::new(BytesSource(&b"404 not found"[..])) as BoxedSource),
                            "text/plain".to_string(),
                            404,
                            13,
                            None,
                        ),
                    }
                }
                None => (
                    Some(Box::new(BytesSource(&b"bad request"[..])) as BoxedSource),
                    "text/plain".to_string(),
                    400,
                    11,
                    None,
                ),
            };
            let mut st = self.state.lock().expect("state lock");
            st.source = source;
            st.mime = mime;
            st.status = status;
            st.offset = 0;
            st.len = len;
            st.etag = etag;
            if let Some(hr) = handle_request { *hr = 1; }
            1
        }

        fn response_headers(&self, response: Option<&mut Response>, response_length: Option<&mut i64>, _redirect_url: Option<&mut CefString>) {
            let st = self.state.lock().expect("state lock");
            if let Some(r) = response {
                r.set_mime_type(Some(&CefString::from(st.mime.as_str())));
                r.set_status(st.status);
                // COOP/COEP: required for SharedArrayBuffer on afterglow:// pages
                // (the web target sets these on its HTTP server).
                for (k, v) in [
                    ("Cross-Origin-Opener-Policy", "same-origin"),
                    ("Cross-Origin-Embedder-Policy", "require-corp"),
                    ("Cross-Origin-Resource-Policy", "same-origin"),
                    // Range support: tell Chromium we accept byte-range skips.
                    ("Accept-Ranges", "bytes"),
                ] {
                    r.set_header_by_name(Some(&CefString::from(k)), Some(&CefString::from(v)), 1);
                }
                if let Some(etag) = &st.etag {
                    r.set_header_by_name(
                        Some(&CefString::from("ETag")),
                        Some(&CefString::from(etag.as_str())),
                        1,
                    );
                }
            }
            if let Some(rl) = response_length { *rl = st.len as i64; }
        }

        fn skip(&self, bytes_to_skip: i64, bytes_skipped: Option<&mut i64>, _callback: Option<&mut ResourceSkipCallback>) -> ::std::os::raw::c_int {
            if bytes_to_skip <= 0 {
                if let Some(bs) = bytes_skipped { *bs = 0; }
                return 0;
            }
            let mut st = self.state.lock().expect("state lock");
            let skip = (bytes_to_skip as u64).min(st.len.saturating_sub(st.offset));
            st.offset += skip;
            if let Some(bs) = bytes_skipped { *bs = skip as i64; }
            1
        }

        fn read(&self, data_out: *mut u8, bytes_to_read: ::std::os::raw::c_int, bytes_read: Option<&mut ::std::os::raw::c_int>, _callback: Option<&mut ResourceReadCallback>) -> ::std::os::raw::c_int {
            // `read_at` is a fast pread (disk) or memcpy (embedded); we hold
            // the state lock for the duration. Per-request handler, one read
            // at a time, so this doesn't serialize across requests.
            let mut st = self.state.lock().expect("state lock");
            let Some(source) = st.source.as_ref() else {
                if let Some(br) = bytes_read { *br = 0; }
                return 0;
            };
            if bytes_to_read <= 0 || st.offset >= st.len {
                if let Some(br) = bytes_read { *br = 0; }
                return 0;
            }
            let want = (bytes_to_read as usize).min((st.len - st.offset) as usize);
            let off = st.offset;
            let mut buf = vec![0u8; want];
            let n = match source.read_at(off, &mut buf) {
                Ok(n) => n,
                Err(_) => {
                    if let Some(br) = bytes_read { *br = 0; }
                    return 0;
                }
            };
            if n == 0 {
                if let Some(br) = bytes_read { *br = 0; }
                return 0;
            }
            // SAFETY: `data_out` holds `bytes_to_read` bytes (CEF contract);
            // `n <= want <= bytes_to_read`.
            unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), data_out, n); }
            st.offset += n as u64;
            if let Some(br) = bytes_read { *br = n as i32; }
            1
        }

        fn cancel(&self) {}
    }
}

wrap_scheme_handler_factory! {
    struct AfterglowSchemeFactory;

    impl SchemeHandlerFactory {
        fn create(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _scheme_name: Option<&CefString>, _request: Option<&mut Request>) -> Option<ResourceHandler> {
            Some(AfterglowResource::new(Arc::new(Mutex::new(ResponseState::default()))))
        }
    }
}

/// Register the `afterglow://` scheme handler factory. Call from the browser
/// process UI thread (on_context_initialized).
pub(crate) fn register_factory() {
    let mut factory = AfterglowSchemeFactory::new();
    let scheme = CefString::from(SCHEME);
    let domain = CefString::from(SCHEME_DOMAIN);
    register_scheme_handler_factory(Some(&scheme), Some(&domain), Some(&mut factory));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_of_strips_scheme_host_and_query() {
        assert_eq!(
            path_of("afterglow://local/index.html").as_ref(),
            "/index.html"
        );
        assert_eq!(
            path_of("afterglow://local/a/b.js?x=1&y=2").as_ref(),
            "/a/b.js"
        );
        assert_eq!(path_of("afterglow://local/a%20b.js").as_ref(), "/a%20b.js");
        assert_eq!(path_of("afterglow://local/").as_ref(), "/");
        assert_eq!(path_of("not-a-url").as_ref(), "/");
    }

    #[test]
    fn resolve_embedded_first_then_fs_miss() {
        let emb: Vec<(String, String, &'static [u8])> = vec![(
            "/index.html".into(),
            "text/html".into(),
            &b"<html></html>"[..],
        )];
        match resolve_scheme_path("/index.html", &emb, None) {
            Resolved::Found { source, mime, .. } => {
                assert_eq!(source.len(), 13);
                assert_eq!(mime, "text/html");
                let mut buf = vec![0u8; 13];
                assert_eq!(source.read_at(0, &mut buf).unwrap(), 13);
                assert_eq!(&buf, b"<html></html>");
            }
            Resolved::NotFound => panic!("embedded asset should resolve"),
        }
        assert!(matches!(
            resolve_scheme_path("/missing.js", &emb, None),
            Resolved::NotFound
        ));
    }

    #[test]
    fn resolve_serves_fs_file_streaming() {
        // An existing workspace path (this crate's own dir) as the FS root.
        // Exercises the afterglow_assets delegation: a real hit streams from
        // disk via FsSource (not a whole-file read); traversal is rejected.
        let root = AssetRoot::new(env!("CARGO_MANIFEST_DIR")).unwrap();
        match resolve_scheme_path("/Cargo.toml", &[], Some(&root)) {
            Resolved::Found { source, mime, etag } => {
                assert_eq!(source.len() > 0, true);
                assert_eq!(mime, "application/octet-stream");
                assert!(etag.is_some(), "FsSource provides an mtime ETag");
                // Stream the first bytes via read_at.
                let mut buf = [0u8; 8];
                let n = source.read_at(0, &mut buf).unwrap();
                assert!(n <= 8);
                assert!(buf[..n].starts_with(b"["));
            }
            Resolved::NotFound => panic!("Cargo.toml should resolve from crate root"),
        }
        // traversal above root (workspace Cargo.toml is one level up) -> 404
        assert!(matches!(
            resolve_scheme_path("/../Cargo.toml", &[], Some(&root)),
            Resolved::NotFound
        ));
        // missing file -> 404
        assert!(matches!(
            resolve_scheme_path("/does-not-exist.bin", &[], Some(&root)),
            Resolved::NotFound
        ));
    }

    #[test]
    fn root_url_normalizes_leading_slash() {
        assert_eq!(
            crate::root_url("/index.html"),
            "afterglow://local/index.html"
        );
        assert_eq!(
            crate::root_url("index.html"),
            "afterglow://local/index.html"
        );
    }
}
