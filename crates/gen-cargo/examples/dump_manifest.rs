//! Quick smoke-runner — point at a Cargo workspace, print the typed
//! manifest as JSON. Useful for hand-validating parses against real
//! pleme-io repos.

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: dump_manifest <path-to-cargo-workspace>");
        std::process::exit(2);
    }
    let path = PathBuf::from(&args[1]);
    let manifest = match gen_cargo::parse(&path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("parse failed: {e}");
            std::process::exit(1);
        }
    };
    let json = serde_json::to_string_pretty(&manifest).unwrap();
    println!("{json}");
}
