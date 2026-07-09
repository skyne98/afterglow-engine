//! JS <-> Rust bridge.
//!
//! - **Web -> Rust**: a tiny localhost HTTP server at `POST /__invoke` (real
//!   HTTP, so the POST body is forwarded — unlike custom-scheme fetch). Served
//!   on `127.0.0.1`, which Chromium treats as a trustworthy origin, so the
//!   secure `afterglow://` page can fetch it without mixed-content blocking.
//! - **Rust -> Web**: [`emit`] injects JS via `Frame::execute_java_script`,
//!   calling `window.afterglow.emit(event, data)` registered by [`INVOKE_JS`].
//!
//! The asset scheme handler serves `/__afterglow_config.js` so the page learns
//! the invoke URL without any `execute_java_script` timing races.

use cef::*;
use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};

use crate::config::CONFIG;

/// JS to include in the page: defines `window.afterglow.invoke` (web->rust) and
/// `window.afterglow.on/emit` (rust->web handler registry). The page must also
/// include `<script src="/__afterglow_config.js"></script>` first.
pub const INVOKE_JS: &str = r#"
window.afterglow = window.afterglow || {};
window.afterglow.invoke = async (method, params) => {
  const r = await fetch(window.__afterglow_invoke_url, {
    method: 'POST',
    headers: { 'content-type': 'text/plain' },
    body: JSON.stringify({ method, params: params ?? null }),
  });
  const j = await r.json();
  if (j.error) throw new Error(j.error);
  return j.result;
};
window.afterglow._handlers = {};
window.afterglow.on = (event, fn) => { window.afterglow._handlers[event] = fn; };
window.afterglow.emit = (event, data) => {
  const fn = window.afterglow._handlers[event];
  if (fn) { try { fn(data); } catch (e) { console.log('afterglow handler error: ' + e); } }
};
"#;

/// `/__afterglow_config.js` body: tells the page the invoke URL.
pub fn config_js() -> String {
    format!(
        "window.__afterglow_invoke_url = 'http://127.0.0.1:{}/__invoke';\n",
        ensure_invoke_server()
    )
}

static MAIN_BROWSER: Mutex<Option<Browser>> = Mutex::new(None);

/// Remember the main browser so [`emit`] can push to it. Called on the UI
/// thread in `on_after_created`.
pub fn set_main_browser(b: Browser) {
    *MAIN_BROWSER.lock().expect("main browser lock") = Some(b);
}

/// Rust -> Web: call `window.afterglow.emit(event, data)` in the page.
/// `json` must be a valid JSON value string (it is inlined into the script).
/// Safe to call from any thread (CEF marshals `execute_java_script`).
pub fn emit(event: &str, json: &str) {
    let b = MAIN_BROWSER.lock().expect("main browser lock").clone();
    let Some(b) = b else { return };
    let Some(frame) = b.main_frame() else { return };
    // event is a JS string literal; json is inlined verbatim.
    let js = format!("window.afterglow.emit({:?}, {});", event, json);
    frame.execute_java_script(
        Some(&CefString::from(js.as_str())),
        Some(&CefString::from("afterglow://emit")),
        0,
    );
}

// --- localhost invoke HTTP server ------------------------------------------

static INVOKE_PORT: OnceLock<u16> = OnceLock::new();

fn ensure_invoke_server() -> u16 {
    *INVOKE_PORT.get_or_init(start_invoke_server)
}

fn start_invoke_server() -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind invoke server");
    let port = listener.local_addr().expect("invoke addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = vec![0u8; 65536];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let first = req.lines().next().unwrap_or("");
            let mut parts = first.split_whitespace();
            let method = parts.next().unwrap_or("");
            let path = parts.next().unwrap_or("");

            // CORS preflight
            if method == "OPTIONS" {
                let _ = s.write_all(b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: content-type\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Private-Network: true\r\nContent-Length: 0\r\n\r\n");
                continue;
            }
            let body = req.split("\r\n\r\n").nth(1).unwrap_or("").as_bytes();
            let resp_bytes = if path == "/__invoke" && method == "POST" {
                let parsed: Value = serde_json::from_slice(body).unwrap_or_else(|_| json!({}));
                let m = parsed["method"].as_str().unwrap_or("").to_string();
                let p = parsed["params"].clone();
                let result = CONFIG.get().and_then(|c| c.invoke.as_ref())
                    .map(|h| h(&m, p))
                    .unwrap_or_else(|| json!({ "error": "no invoke handler registered" }));
                serde_json::to_vec(&json!({ "result": result })).unwrap_or_default()
            } else {
                b"{\"error\":\"not found\"}".to_vec()
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Private-Network: true\r\n\r\n",
                resp_bytes.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&resp_bytes);
        }
    });
    port
}
