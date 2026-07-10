//! Prints the RPC schema as JSON.
//!
//! ```sh
//! cargo run -p afterglow-rpc-demo --bin dump-schema
//! ```
//!
//! The schema static is generated directly by the `#[rpc]` macro; no transport
//! type is needed.

use afterglow_rpc_demo::PHYSICS_SCHEMA;

fn main() {
    println!("{}", serde_json::to_string_pretty(PHYSICS_SCHEMA).unwrap());
}
