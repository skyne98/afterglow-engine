//! Simple HTTP server with COOP/COEP headers for SharedArrayBuffer support.
//!
//! ```sh
//! cargo run --example coep_server
//! ```
//! Then open http://localhost:8787

use std::io::{Read, Write};
use std::net::TcpListener;

fn main() {
    let listener = TcpListener::bind("127.0.0.1:8787").unwrap();
    eprintln!("COOP/COEP server running on http://localhost:8787");
    eprintln!("Serving from crates/afterglow-web/www/");

    let www_dir = std::path::Path::new("crates/afterglow-web/www");

    for stream in listener.incoming() {
        let mut stream = stream.unwrap();
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap();
        let request = String::from_utf8_lossy(&buf[..n]);
        let path = request.lines().next().unwrap_or("");
        // Parse: GET /path HTTP/1.1
        let path = path.split_whitespace().nth(1).unwrap_or("/");

        let file_path = if path == "/" {
            www_dir.join("index.html")
        } else {
            www_dir.join(path.trim_start_matches('/'))
        };

        let (content_type, body) = if file_path.exists() {
            let body = std::fs::read(&file_path).unwrap();
            let ct = match file_path.extension().and_then(|e| e.to_str()) {
                Some("html") => "text/html",
                Some("js") => "application/javascript",
                Some("wasm") => "application/wasm",
                _ => "application/octet-stream",
            };
            (ct, body)
        } else {
            ("text/plain", b"Not Found".to_vec())
        };

        let status = if file_path.exists() { "200 OK" } else { "404 Not Found" };

        let response = format!(
            "HTTP/1.1 {status}\r\n\
             Content-Type: {content_type}\r\n\
             Content-Length: {len}\r\n\
             Cross-Origin-Opener-Policy: same-origin\r\n\
             Cross-Origin-Embedder-Policy: require-corp\r\n\
             Cross-Origin-Resource-Policy: same-origin\r\n\
             \r\n",
            len = body.len(),
        );

        stream.write_all(response.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
    }
}
