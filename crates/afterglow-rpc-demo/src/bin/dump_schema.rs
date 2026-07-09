//! Prints the RPC schema as JSON. The build system (xtask) runs this and feeds
//! the JSON to the TypeScript/Rust client generators.
//!
//!   cargo run -p afterglow-rpc-demo --bin dump-schema

use afterglow_rpc_demo::SCHEMA;

fn main() {
    println!("{}", serde_json::to_string_pretty(&SCHEMA).unwrap());
}
