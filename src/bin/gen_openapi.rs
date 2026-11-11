//! Writes the OpenAPI document of the API, built from the routes, to compare against the one
//! committed in `openapi.json`: CI checks the committed one is current and that the change from
//! master's breaks no released client.
//!
//! Usage: `cargo run --bin gen_openapi [out_file]` (default `openapi.json`).

use std::{env, fs};

fn main() -> anyhow::Result<()> {
    let out = env::args().nth(1).unwrap_or_else(|| "openapi.json".to_string());
    let api = hachimi_world_server::web::routes::openapi();
    fs::write(&out, serde_json::to_string_pretty(&api)? + "\n")?;
    println!("Wrote {out}");
    Ok(())
}
