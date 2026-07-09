//! Direct resource serving via a CEF custom scheme (`afterglow://local/`).
//!
//! Resources (HTML/JS/WASM/...) are served from embedded bytes or a filesystem
//! root through CEF's internal request routing — no localhost HTTP server on
//! the resource path. The scheme is registered standard + secure + CORS +
//! fetch + CSP-bypassing, so ES-module imports, WebGPU, and inline scripts work.
//!
//! The JS<->Rust invoke bridge lives in [`crate::ipc`] (a tiny localhost HTTP
//! server for `POST /__invoke`, since CEF doesn't forward fetch bodies to
//! custom-scheme handlers). This module serves that bridge's
//! `/__afterglow_config.js` (which tells the page the invoke URL) alongside the
//! regular assets.

use cef::*;
use std::sync::{Arc, Mutex};

use crate::config::{CONFIG, SCHEME, SCHEME_DOMAIN};

/// `afterglow://local/foo/bar.js?x=1` -> `/foo/bar.js`
fn path_of(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let after_host = after_scheme.split_once('/').map(|(_, r)| r).unwrap_or("");
    let path = format!("/{after_host}");
    path.split_once('?').map(|(p, _)| p.to_string()).unwrap_or(path)
}

fn guess_mime(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).as_deref() {
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

/// Resolve a scheme path to (body, mime). `/__afterglow_config.js` is generated
/// dynamically (carries the invoke URL); embedded assets next; then a FS root.
fn resolve(path: &str) -> (Vec<u8>, String) {
    if path == "/__afterglow_bootstrap.js" {
        return (b"// afterglow bootstrap (placeholder)\n".to_vec(), "text/javascript".to_string());
    }
    let cfg = CONFIG.get().expect("afterglow-cef config not set");
    if let Some((_, mime, bytes)) = cfg.embedded.iter().find(|(p, _, _)| p == path) {
        return (bytes.to_vec(), mime.clone());
    }
    if let Some(root) = &cfg.fs_root {
        let rel = path.strip_prefix('/').unwrap_or(path);
        if let Ok(bytes) = std::fs::read(root.join(rel)) {
            return (bytes, guess_mime(path).to_string());
        }
    }
    (b"404 not found".to_vec(), "text/plain".to_string())
}

wrap_resource_handler! {
    struct AfterglowResource {
        body: Arc<Mutex<Option<Vec<u8>>>>,
        mime: Arc<Mutex<String>>,
        offset: Arc<Mutex<usize>>,
    }

    impl ResourceHandler {
        fn open(&self, request: Option<&mut Request>, handle_request: Option<&mut ::std::os::raw::c_int>, _callback: Option<&mut Callback>) -> ::std::os::raw::c_int {
            let (body, mime) = if let Some(req) = request {
                let url_uf = req.url();
                let url = CefString::from(&url_uf).to_string();
                resolve(&path_of(&url))
            } else {
                (b"bad request".to_vec(), "text/plain".to_string())
            };
            *self.body.lock().expect("body lock") = Some(body);
            *self.mime.lock().expect("mime lock") = mime;
            if let Some(hr) = handle_request { *hr = 1; }
            1
        }

        fn response_headers(&self, response: Option<&mut Response>, response_length: Option<&mut i64>, _redirect_url: Option<&mut CefString>) {
            let mime = self.mime.lock().expect("mime lock").clone();
            if let Some(r) = response {
                r.set_mime_type(Some(&CefString::from(mime.as_str())));
                r.set_status(200);
                // COOP/COEP headers: required for SharedArrayBuffer support.
                // On the web target, the server sets these; on the native
                // (CEF) target, we set them here so afterglow:// pages get
                // crossOriginIsolated = true.
                let key1 = CefString::from("Cross-Origin-Opener-Policy");
                let val1 = CefString::from("same-origin");
                r.set_header_by_name(Some(&key1), Some(&val1), 1);
                let key2 = CefString::from("Cross-Origin-Embedder-Policy");
                let val2 = CefString::from("require-corp");
                r.set_header_by_name(Some(&key2), Some(&val2), 1);
            }
            if let Some(rl) = response_length { *rl = -1; } // read until EOF
        }

        fn read(&self, data_out: *mut u8, bytes_to_read: ::std::os::raw::c_int, bytes_read: Option<&mut ::std::os::raw::c_int>, _callback: Option<&mut ResourceReadCallback>) -> ::std::os::raw::c_int {
            let guard = self.body.lock().expect("body lock");
            let Some(body) = guard.as_ref() else {
                if let Some(br) = bytes_read { *br = 0; }
                return 0;
            };
            let mut off = self.offset.lock().expect("offset lock");
            let remaining = &body[*off..];
            let n = std::cmp::min(bytes_to_read as usize, remaining.len());
            if n == 0 {
                if let Some(br) = bytes_read { *br = 0; }
                return 0;
            }
            unsafe { std::ptr::copy_nonoverlapping(remaining.as_ptr(), data_out, n); }
            *off += n;
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
            Some(AfterglowResource::new(
                Arc::new(Mutex::new(None)),
                Arc::new(Mutex::new(String::new())),
                Arc::new(Mutex::new(0)),
            ))
        }
    }
}

/// Register the `afterglow://` scheme handler factory. Call from the browser
/// process UI thread (on_context_initialized).
pub fn register_factory() {
    let mut factory = AfterglowSchemeFactory::new();
    let scheme = CefString::from(SCHEME);
    let domain = CefString::from(SCHEME_DOMAIN);
    register_scheme_handler_factory(Some(&scheme), Some(&domain), Some(&mut factory));
}
