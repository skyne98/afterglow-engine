//! Minimal COOP/COEP dev server for `afterglow-web`'s `www/` directory.
//!
//! Used by the `coep_server` example. Path/MIME resolution and canonical
//! confinement are delegated to [`afterglow_assets`]; this module owns only the
//! HTTP policy (request-line parsing, GET-only routing, response assembly),
//! kept in pure functions so it can be tested without a socket.

use std::path::Path;

use afterglow_assets::{guess_mime, resolve};

/// Parsed HTTP request line: `(method, raw_path)` where `raw_path` keeps its
/// query string (the handler strips it). `None` if malformed.
fn parse_request_line(request: &str) -> Option<(&str, &str)> {
    let mut parts = request.lines().next()?.split_whitespace();
    Some((parts.next()?, parts.next()?))
}

/// A resolved HTTP response. `#[doc(hidden)]` because only [`handle_request`]
/// returns it and the `coep_server` example reads its fields.
#[doc(hidden)]
pub struct Response {
    pub status: u16,
    pub reason: &'static str,
    pub mime: &'static str,
    pub body: Vec<u8>,
}

/// Build a `text/plain` error response (compact: avoids `struct_lit_width`
/// expansion).
fn plain(status: u16, reason: &'static str, body: &'static [u8]) -> Response {
    Response {
        status,
        reason,
        mime: "text/plain",
        body: body.to_vec(),
    }
}

/// Handle one raw HTTP request against `root`.
///
/// GET-only: non-GET -> 405, malformed -> 400. `/` serves `worker-test.html`.
/// Traversal attempts and missing files both yield 404 (no leak about which).
/// Existing files are canonically confined (symlinks cannot escape); resolution
/// and confinement are delegated to [`afterglow_assets::resolve`].
#[doc(hidden)]
pub fn handle_request(root: &Path, request: &str) -> Response {
    let Some((method, raw)) = parse_request_line(request) else {
        return plain(400, "Bad Request", b"bad request");
    };
    if method != "GET" {
        return plain(405, "Method Not Allowed", b"method not allowed");
    }
    let path = raw.split_once('?').map(|(p, _)| p).unwrap_or(raw);
    let path = if path == "/" {
        "/worker-test.html"
    } else {
        path
    };
    match resolve(root, path).and_then(|p| std::fs::read(p).ok()) {
        Some(body) => Response {
            status: 200,
            reason: "OK",
            mime: guess_mime(path),
            body,
        },
        None => plain(404, "Not Found", b"not found"),
    }
}

/// COOP/COEP/CORP headers (enough for `SharedArrayBuffer`). `#[doc(hidden)]`
/// for the `coep_server` example.
#[doc(hidden)]
pub const CROSS_ORIGIN_HEADERS: &[(&str, &str)] = &[
    ("Cross-Origin-Opener-Policy", "same-origin"),
    ("Cross-Origin-Embedder-Policy", "require-corp"),
    ("Cross-Origin-Resource-Policy", "same-origin"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parse_request_line_basic() {
        assert_eq!(
            parse_request_line("GET /foo.html HTTP/1.1\r\nHost: x"),
            Some(("GET", "/foo.html"))
        );
        assert_eq!(
            parse_request_line("GET /x?y=1 HTTP/1.1"),
            Some(("GET", "/x?y=1"))
        );
        assert_eq!(parse_request_line(""), None);
        assert_eq!(parse_request_line("GET"), None);
    }

    #[test]
    fn handle_request_get_404_405_400() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("www");
        let r = handle_request(&root, "GET /bench.html HTTP/1.1\r\n");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/html");
        assert!(!r.body.is_empty());
        // GET-only: HEAD and POST are 405
        assert_eq!(
            handle_request(&root, "HEAD /bench.html HTTP/1.1\r\n").status,
            405
        );
        assert_eq!(
            handle_request(&root, "POST /bench.html HTTP/1.1\r\n").status,
            405
        );
        // missing -> 404; malformed -> 400
        assert_eq!(
            handle_request(&root, "GET /nope.html HTTP/1.1\r\n").status,
            404
        );
        assert_eq!(handle_request(&root, "garbage").status, 400);
    }

    #[test]
    fn query_string_still_serves_file() {
        // A query on an existing file must be stripped and the file served with
        // the right MIME (regression for query handling at the handler level).
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("www");
        let r = handle_request(&root, "GET /bench.html?cachebust=42 HTTP/1.1\r\n");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/html");
        assert!(!r.body.is_empty());
    }

    #[test]
    fn root_serves_worker_test() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("www");
        let r = handle_request(&root, "GET / HTTP/1.1\r\n");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/html");
        assert!(!r.body.is_empty());
    }

    #[test]
    fn traversal_cannot_reach_cargo_toml() {
        // The confirmed prior exploit: GET /../../../Cargo.toml returned 200.
        // The manifest dir is the crate root; Cargo.toml is one level up.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("www");
        let r = handle_request(&root, "GET /../../../Cargo.toml HTTP/1.1\r\n");
        assert_eq!(r.status, 404);
        assert!(!r.body.contains(&b'[')); // not TOML content
    }
}
