//! Bounded COOP/COEP development server for `afterglow-web/www`.
//!
//! ```sh
//! cargo run -p xtask serve
//! ```

use afterglow_assets::AssetRoot;
use afterglow_web::dev_server::DevAssetServer;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

fn main() {
    let www = Path::new("crates/afterglow-web/www");
    let root = AssetRoot::new(www)
        .unwrap_or_else(|| panic!("invalid asset root {}", www.display()));
    let address: SocketAddr = "127.0.0.1:8787".parse().expect("fixed dev address");
    let mut server = DevAssetServer::start(root, address, 4, 16).expect("start bounded dev server");
    let stop = server.stop_token();
    let signal = stop.clone();
    ctrlc::set_handler(move || signal.store(true, Ordering::Release))
        .expect("install Ctrl-C handler");
    eprintln!(
        "COOP/COEP server running on http://{} with 4 workers × 16 queued connections",
        server.address()
    );
    while !stop.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(50));
    }
    server.shutdown();
    let stats = server.stats();
    eprintln!(
        "server stopped: accepted={} rejected={} completed={}",
        stats.accepted, stats.rejected, stats.completed
    );
}
