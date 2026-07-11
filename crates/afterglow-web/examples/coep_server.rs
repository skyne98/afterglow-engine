//! Minimal COOP/COEP dev server for `afterglow-web`'s `www/` directory.
//!
//! ```sh
//! cargo run -p afterglow-web --example coep_server
//! ```
//! Then open http://localhost:8787
//!
//! Each connection is served on its own thread with a read timeout, so an idle
//! or early-disconnecting client cannot block the server. Writes never panic on
//! `BrokenPipe` (a disconnecting client is normal). Path resolution rejects
//! traversal and canonically confines existing files (symlinks cannot escape).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::time::Duration;

use afterglow_assets::AssetRoot;
use afterglow_web::dev_server::{CROSS_ORIGIN_HEADERS, handle_request};

fn main() {
    let www_dir = Path::new("crates/afterglow-web/www");
    if !www_dir.exists() {
        eprintln!(
            "www dir not found at {} (run from the workspace root)",
            www_dir.display()
        );
        std::process::exit(1);
    }
    let root = AssetRoot::new(www_dir).expect("canonicalize www directory");
    let listener = TcpListener::bind("127.0.0.1:8787").expect("bind 127.0.0.1:8787");
    eprintln!("COOP/COEP server running on http://localhost:8787");
    eprintln!("Serving from {}", www_dir.display());

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let root = root.clone();
        // Per-connection thread + read timeout: an idle client cannot block the
        // accept loop or any other connection.
        std::thread::spawn(move || {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = [0u8; 8192];
            // Read the request; tolerate a clean EOF / timeout / early close.
            let n = match stream.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            let request = String::from_utf8_lossy(&buf[..n]);
            let resp = handle_request(&root, &request);

            let mut head = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n",
                resp.status,
                resp.reason,
                resp.mime,
                resp.body.len()
            );
            for (k, v) in CROSS_ORIGIN_HEADERS {
                head.push_str(&format!("{k}: {v}\r\n"));
            }
            head.push_str("Connection: close\r\n\r\n");

            // Writes must not panic on BrokenPipe (client gone).
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&resp.body);
            let _ = stream.flush();
        });
    }
}
