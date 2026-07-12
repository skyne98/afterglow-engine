//! Minimal COOP/COEP dev server for `afterglow-web`'s `www/` directory.
//!
//! Used by the `coep_server` example. Path/MIME resolution and canonical
//! confinement are delegated to [`afterglow_assets`]; this module owns only the
//! HTTP policy (request-line parsing, GET-only routing, response assembly),
//! kept in pure functions so it can be tested without a socket.
//!
//! **Streaming + ranges:** responses carry an [`AssetSource`] and serve bytes
//! via [`AssetSource::read_at`] — no whole-file buffering. A `Range` header
//! produces a `206 Partial Content` with `Content-Range`; otherwise `200` full
//! (also streamed).

use afterglow_assets::range::{self, RangeSpec};
use afterglow_assets::source::AssetSource;
use afterglow_assets::{AssetRoot, BytesSource, guess_mime};

/// Parsed HTTP request line: `(method, raw_path)` where `raw_path` keeps its
/// query string (the handler strips it). `None` if malformed.
fn parse_request_line(request: &str) -> Option<(&str, &str)> {
    let mut parts = request.lines().next()?.split_whitespace();
    Some((parts.next()?, parts.next()?))
}

/// Extract a header value from a raw HTTP request (case-insensitive name).
/// Returns the first matching header's value, trimmed.
fn header_value<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    let name_lower = name.to_ascii_lowercase();
    for line in request.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().to_ascii_lowercase() == name_lower {
                return Some(v.trim());
            }
        }
    }
    None
}

/// A resolved HTTP response. Carries a streaming [`AssetSource`] + a byte range
/// (start, len) into it, plus headers. `#[doc(hidden)]` because only
/// [`handle_request`] returns it and the `coep_server` example reads its fields.
#[doc(hidden)]
pub struct Response {
    pub status: u16,
    pub reason: &'static str,
    pub mime: &'static str,
    /// The streaming source. `None` only for error responses (which use a
    /// static `BytesSource`).
    pub source: Box<dyn AssetSource + Send + Sync>,
    /// First byte to read from `source`.
    pub start: u64,
    /// Number of bytes to serve.
    pub len: u64,
    /// `Some` for a 206 partial response (the `Content-Range` value); `None`
    /// for 200/404/etc.
    pub content_range: Option<String>,
    /// Optional weak ETag for caching.
    pub etag: Option<String>,
}

impl Response {
    /// The total number of body bytes this response will stream.
    pub fn body_len(&self) -> u64 {
        self.len
    }
}

/// Build a `text/plain` error response from static bytes.
fn plain(status: u16, reason: &'static str, body: &'static [u8]) -> Response {
    let len = body.len() as u64;
    Response {
        status,
        reason,
        mime: "text/plain",
        source: Box::new(BytesSource(body)),
        start: 0,
        len,
        content_range: None,
        etag: None,
    }
}

/// Handle one raw HTTP request against `root`.
///
/// GET-only: non-GET -> 405, malformed -> 400. `/` serves `worker-test.html`.
/// Traversal attempts and missing files both yield 404 (no leak about which).
/// Existing files are canonically confined (symlinks cannot escape); resolution
/// and confinement are delegated to [`afterglow_assets::resolve`]. A `Range`
/// header produces a `206 Partial Content` with `Content-Range`.
#[doc(hidden)]
pub fn handle_request(root: &AssetRoot, request: &str) -> Response {
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
    let Some(src) = root.open_source(path) else {
        return plain(404, "Not Found", b"not found");
    };
    let total = src.len();
    let etag = src.etag();
    let range_header = header_value(request, "Range");
    let spec = range::parse_range(range_header, total);
    match spec {
        RangeSpec::Range { start, end } => {
            let len = end - start + 1;
            Response {
                status: 206,
                reason: "Partial Content",
                mime: guess_mime(path),
                source: Box::new(src),
                start,
                len,
                content_range: Some(range::content_range(start, end, total)),
                etag,
            }
        }
        RangeSpec::Full => Response {
            status: 200,
            reason: "OK",
            mime: guess_mime(path),
            source: Box::new(src),
            start: 0,
            len: total,
            content_range: None,
            etag,
        },
        RangeSpec::Unsatisfiable => plain(416, "Range Not Satisfiable", b"range not satisfiable"),
    }
}

/// COOP/COEP/CORP + `Accept-Ranges` headers. `#[doc(hidden)]` for the
/// `coep_server` example.
#[doc(hidden)]
pub const CROSS_ORIGIN_HEADERS: &[(&str, &str)] = &[
    ("Cross-Origin-Opener-Policy", "same-origin"),
    ("Cross-Origin-Embedder-Policy", "require-corp"),
    ("Cross-Origin-Resource-Policy", "same-origin"),
    ("Accept-Ranges", "bytes"),
];

/// Stream a [`Response`]'s body to a writer in chunks via
/// [`AssetSource::read_at`]. Used by the `coep_server` example. Writes must not
/// panic on `BrokenPipe` (a disconnecting client is normal) — the caller
/// should ignore write errors.
#[doc(hidden)]
pub fn stream_body<W: std::io::Write>(
    resp: &Response,
    out: &mut W,
    chunk: &mut [u8],
) -> std::io::Result<u64> {
    let mut written = 0u64;
    let mut off = resp.start;
    while written < resp.len {
        let want = (resp.len - written).min(chunk.len() as u64) as usize;
        let n = resp.source.read_at(off, &mut chunk[..want])?;
        if n == 0 {
            break;
        }
        out.write_all(&chunk[..n])?;
        off += n as u64;
        written += n as u64;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn www_root() -> AssetRoot {
        AssetRoot::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("www")).unwrap()
    }

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
        let root = www_root();
        let r = handle_request(&root, "GET /worker-test.html HTTP/1.1\r\n");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/html");
        assert!(r.body_len() > 0);
        // GET-only: HEAD and POST are 405
        assert_eq!(
            handle_request(&root, "HEAD /worker-test.html HTTP/1.1\r\n").status,
            405
        );
        assert_eq!(
            handle_request(&root, "POST /worker-test.html HTTP/1.1\r\n").status,
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
        let root = www_root();
        let r = handle_request(&root, "GET /worker-test.html?cachebust=42 HTTP/1.1\r\n");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/html");
        assert!(r.body_len() > 0);
    }

    #[test]
    fn root_serves_worker_test() {
        let root = www_root();
        let r = handle_request(&root, "GET / HTTP/1.1\r\n");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/html");
        assert!(r.body_len() > 0);
    }

    #[test]
    fn traversal_cannot_reach_cargo_toml() {
        let root = www_root();
        let r = handle_request(&root, "GET /../../../Cargo.toml HTTP/1.1\r\n");
        assert_eq!(r.status, 404);
    }

    #[test]
    fn range_request_returns_206() {
        let root = www_root();
        let r = handle_request(
            &root,
            "GET /worker-test.html HTTP/1.1\r\nRange: bytes=0-99\r\n",
        );
        assert_eq!(r.status, 206);
        assert_eq!(r.reason, "Partial Content");
        assert_eq!(r.len, 100);
        assert_eq!(r.start, 0);
        assert!(r.content_range.as_ref().unwrap().starts_with("bytes 0-99/"));
    }

    #[test]
    fn range_suffix_request() {
        let root = www_root();
        let r = handle_request(
            &root,
            "GET /worker-test.html HTTP/1.1\r\nRange: bytes=-100\r\n",
        );
        assert_eq!(r.status, 206);
        assert_eq!(r.len, 100);
        assert!(r.start > 0, "suffix should start near the end");
    }

    #[test]
    fn range_open_end_request() {
        let root = www_root();
        let r = handle_request(
            &root,
            "GET /worker-test.html HTTP/1.1\r\nRange: bytes=10-\r\n",
        );
        assert_eq!(r.status, 206);
        assert_eq!(r.start, 10);
    }

    #[test]
    fn unsatisfiable_range_returns_416() {
        let root = www_root();
        let r = handle_request(
            &root,
            "GET /worker-test.html HTTP/1.1\r\nRange: bytes=9999999-\r\n",
        );
        assert_eq!(r.status, 416);
    }

    #[test]
    fn multi_range_falls_back_to_full_200() {
        let root = www_root();
        let r = handle_request(
            &root,
            "GET /worker-test.html HTTP/1.1\r\nRange: bytes=0-99,200-299\r\n",
        );
        assert_eq!(r.status, 200);
        assert!(r.content_range.is_none());
    }

    #[test]
    fn stream_body_streams_full_file() {
        let root = www_root();
        let resp = handle_request(&root, "GET /worker-test.html HTTP/1.1\r\n");
        let mut out = Vec::new();
        let mut chunk = [0u8; 64];
        let written = stream_body(&resp, &mut out, &mut chunk).unwrap();
        assert_eq!(written, resp.len);
        assert!(out.starts_with(b"<!DOCTYPE"));
    }

    #[test]
    fn stream_body_streams_range() {
        let root = www_root();
        let resp = handle_request(
            &root,
            "GET /worker-test.html HTTP/1.1\r\nRange: bytes=0-15\r\n",
        );
        let mut out = Vec::new();
        let mut chunk = [0u8; 64];
        let written = stream_body(&resp, &mut out, &mut chunk).unwrap();
        assert_eq!(written, 16);
        assert_eq!(out.len(), 16);
    }

    #[test]
    fn header_value_case_insensitive() {
        let req = "GET /x HTTP/1.1\r\nRange: bytes=0-99\r\nHOST: x\r\n";
        assert_eq!(header_value(req, "range"), Some("bytes=0-99"));
        assert_eq!(header_value(req, "HOST"), Some("x"));
        assert_eq!(header_value(req, "missing"), None);
    }
}
