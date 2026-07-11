//! Direct resource serving via a CEF custom scheme (`afterglow://local/`).
//!
//! Resources (HTML/JS/WASM/...) are served from embedded bytes or a filesystem
//! root through CEF's internal request routing — no localhost HTTP server on
//! the resource path. The scheme is registered standard + secure + CORS +
//! fetch + CSP-bypassing, so ES-module imports, WebGPU, and inline scripts work.
//!
//! Path/MIME/status resolution is shared with the web dev server via
//! [`afterglow_assets`] (`guess_mime` + `resolve`); this module owns only the
//! CEF-specific policy — embedded-first lookup, optional FS fallback, and the
//! 200/400/404 status mapping — plus the thin `ResourceHandler` wrapper holding
//! one mutex-protected response state (body/mime/status/offset).

use afterglow_assets::{AssetRoot, decode_url_path, guess_mime};
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

/// Outcome of resolving a scheme path: a hit (body + mime, 200) or a miss (404).
enum Resolved {
    Found {
        body: Cow<'static, [u8]>,
        mime: String,
    },
    NotFound,
}

/// Resolve a scheme path to a response. Embedded assets take precedence, then a
/// configured FS root delegated to [`afterglow_assets::resolve`] (lexically +
/// canonically confined, so traversal escapes are rejected). Missing/unreadable/
/// escaped paths yield `NotFound` (real 404, no content leak). Takes `embedded`
/// and `fs_root` explicitly so tests can drive it without the global [`CONFIG`].
fn resolve_scheme_path(
    path: &str,
    embedded: &[(String, String, &'static [u8])],
    fs_root: Option<&AssetRoot>,
) -> Resolved {
    let Some(decoded) = decode_url_path(path) else {
        return Resolved::NotFound;
    };
    if let Some((_, mime, bytes)) = embedded.iter().find(|(p, _, _)| p == decoded.as_ref()) {
        return Resolved::Found {
            body: Cow::Borrowed(bytes),
            mime: mime.clone(),
        };
    }
    if let Some(root) = fs_root
        && let Some(confined) = root.resolve(path)
        && let Ok(body) = std::fs::read(&confined)
    {
        return Resolved::Found {
            body: Cow::Owned(body),
            mime: guess_mime(&decoded).to_string(),
        };
    }
    Resolved::NotFound
}

/// Per-response state behind a single lock. Reset on every `open` so a reused
/// handler can never leak a prior response.
#[derive(Default)]
struct ResponseState {
    body: Cow<'static, [u8]>,
    mime: String,
    status: i32,
    offset: usize,
}

wrap_resource_handler! {
    struct AfterglowResource {
        state: Arc<Mutex<ResponseState>>,
    }

    impl ResourceHandler {
        fn open(&self, request: Option<&mut Request>, handle_request: Option<&mut ::std::os::raw::c_int>, _callback: Option<&mut Callback>) -> ::std::os::raw::c_int {
            let (body, mime, status) = match request {
                Some(req) => {
                    let cfg = CONFIG.get().expect("afterglow-cef config not set");
                    let url = CefString::from(&req.url()).to_string();
                    match resolve_scheme_path(&path_of(&url), &cfg.embedded, cfg.asset_root.as_ref()) {
                        Resolved::Found { body, mime } => (body, mime, 200),
                        Resolved::NotFound => (Cow::Borrowed(&b"404 not found"[..]), "text/plain".to_string(), 404),
                    }
                }
                None => (Cow::Borrowed(&b"bad request"[..]), "text/plain".to_string(), 400),
            };
            let mut st = self.state.lock().expect("state lock");
            st.body = body;
            st.mime = mime;
            st.status = status;
            st.offset = 0;
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
                ] {
                    r.set_header_by_name(Some(&CefString::from(k)), Some(&CefString::from(v)), 1);
                }
            }
            if let Some(rl) = response_length { *rl = st.body.len() as i64; }
        }

        fn read(&self, data_out: *mut u8, bytes_to_read: ::std::os::raw::c_int, bytes_read: Option<&mut ::std::os::raw::c_int>, _callback: Option<&mut ResourceReadCallback>) -> ::std::os::raw::c_int {
            let mut st = self.state.lock().expect("state lock");
            if bytes_to_read <= 0 || st.offset >= st.body.len() {
                if let Some(br) = bytes_read { *br = 0; }
                return 0;
            }
            let n = std::cmp::min(bytes_to_read as usize, st.body.len() - st.offset);
            let off = st.offset;
            st.offset += n;
            // SAFETY: `off < body.len()` (guarded); `data_out` holds `bytes_to_read`
            // bytes (CEF contract); `n <= bytes_to_read` and `n <= body.len()-off`.
            unsafe { std::ptr::copy_nonoverlapping(st.body.as_ptr().add(off), data_out, n); }
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
            Resolved::Found { body, mime } => {
                assert_eq!(body.as_ref(), b"<html></html>");
                assert_eq!(mime, "text/html");
            }
            Resolved::NotFound => panic!("embedded asset should resolve"),
        }
        assert!(matches!(
            resolve_scheme_path("/missing.js", &emb, None),
            Resolved::NotFound
        ));

        let encoded = vec![("/a b.js".into(), "text/javascript".into(), &b"x"[..])];
        assert!(matches!(
            resolve_scheme_path("/a%20b.js", &encoded, None),
            Resolved::Found { .. }
        ));
    }

    #[test]
    fn resolve_serves_fs_file_and_rejects_traversal() {
        // An existing workspace path (this crate's own dir) as the FS root —
        // no temp fixture. Exercises the afterglow_assets delegation:
        // a real hit serves bytes with the correct MIME, and traversal above
        // the root plus missing files both yield NotFound (404, no leak).
        let root = AssetRoot::new(env!("CARGO_MANIFEST_DIR")).unwrap();
        // real file -> served; `.toml` is unmapped -> application/octet-stream
        match resolve_scheme_path("/Cargo.toml", &[], Some(&root)) {
            Resolved::Found { body, mime } => {
                assert!(body.starts_with(b"[package]"));
                assert_eq!(mime, "application/octet-stream");
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
