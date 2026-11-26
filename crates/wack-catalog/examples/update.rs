//! `cargo catalog`: regenerates the snapshot built into the app (`data/models.json`) from
//! models.dev, with the same importer a Refresh uses. `cargo catalog --input catalog.json` reads
//! a feed downloaded earlier instead. Review the diff, then run `cargo test -p wack-catalog`.
//! Normal builds never download anything.

use std::time::{SystemTime, UNIX_EPOCH};

use wack_catalog::{Imported, Limits, SOURCE, download, import};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let body = match args.as_slice() {
        [] => download(SOURCE, Limits::default()).await?,
        [flag, path] if flag == "--input" => std::fs::read(path).map_err(|error| format!("Could not read {path}: {error}"))?,
        _ => return Err("Usage: cargo catalog [--input catalog.json]".to_string()),
    };
    let captured = SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.as_millis() as u64).unwrap_or(0);
    let Imported { catalog, skipped } = import(&body, captured)?;
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/models.json");
    let text = catalog.to_json();
    std::fs::write(path, &text).map_err(|error| format!("Could not write {path}: {error}"))?;
    let listings = catalog.providers.iter().map(|provider| provider.models.len()).sum::<usize>();
    println!(
        "Read {:.1} MB from models.dev: {} providers, {} listings, {} makers' models, {skipped} entries skipped.",
        body.len() as f64 / 1_000_000.0,
        catalog.providers.len(),
        listings,
        catalog.models.len(),
    );
    println!("Wrote {path} ({:.1} MB). The download cap is {} MB.", text.len() as f64 / 1_000_000.0, Limits::default().cap / (1024 * 1024));
    Ok(())
}
