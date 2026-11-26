//! The model catalogue as the app keeps it (`wack-catalog`): the snapshot built in, or the copy
//! the user last refreshed (`v3/model-catalog.json`), whichever was read from models.dev later.
//!
//! - **Refresh** is the only time models.dev is contacted, and only when the user presses it
//!   (docs/security.md). It writes the new copy whole and renames it into place, so a failure at
//!   any step leaves the copy in use as it was; the error says what happened and which copy is
//!   still in use. A copy that can't be read later is passed over quietly for the built-in one.
//! - **Filling** a model row ([`fill`]) gives only what is known: an unknown fact leaves the
//!   row's own value alone.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

pub use wack_catalog::{Catalog, Level, Limits, Listing, Match, Provider, SOURCE, SUGGESTIONS, Suggestion};

use crate::{
    clock::{MONTHS, local},
    store::{DEFAULT_EFFORTS, Reasoning},
};

/// The refreshed copy's name in the data folder.
pub const FILE: &str = "model-catalog.json";

/// The note under a row that reasons when nobody lists its efforts.
pub const UNLISTED_EFFORTS: &str = "The catalogue doesn't list this model's efforts. Tick the ones its endpoint accepts.";
/// The note under a row whose efforts are its maker's, not its host's.
pub const GUESSED_EFFORTS: &str = "These efforts are the maker's; this endpoint may not accept them all.";

/// The catalogue to use: the refreshed copy at `path` when it reads and is newer than the
/// built-in snapshot, else the snapshot.
pub fn load(path: &Path) -> Arc<Catalog> {
    let bundled = wack_catalog::bundled();
    match fs::read_to_string(path).ok().and_then(|text| Catalog::parse(&text)) {
        Some(copy) if copy.captured > bundled.captured => Arc::new(copy),
        _ => bundled,
    }
}

/// Whether `catalog` is the snapshot built into the app.
pub fn is_built_in(catalog: &Catalog) -> bool {
    catalog.captured == wack_catalog::bundled().captured
}

/// A refresh that worked.
#[derive(Debug)]
pub struct Refreshed {
    pub catalog: Arc<Catalog>,
    /// How many models it knows.
    pub models: usize,
    /// How many of the makers' models are new since the copy it replaced.
    pub new: usize,
}

/// Reads models.dev's list again and keeps it at `path`. `current` is the copy in use, kept if
/// anything fails. The error is the user's sentence.
pub async fn refresh(path: PathBuf, current: Arc<Catalog>) -> Result<Refreshed, String> {
    refresh_from(SOURCE, Limits::default(), path, current).await
}

/// [`refresh`] from another address, for tests.
pub async fn refresh_from(url: &str, limits: Limits, path: PathBuf, current: Arc<Catalog>) -> Result<Refreshed, String> {
    let kept = |error: String| format!("{error} The catalogue from {} is still in use.", date(current.captured));
    let body = wack_catalog::download(url, limits).await.map_err(kept)?;
    let captured = SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.as_millis() as u64).unwrap_or(0);
    let catalog = wack_catalog::import(&body, captured).map_err(kept)?.catalog;
    let temporary = path.with_extension("json.tmp");
    let failed = |error: std::io::Error| kept(format!("Could not save the new catalogue: {error}."));
    fs::write(&temporary, catalog.to_json()).map_err(failed)?;
    fs::rename(&temporary, &path).map_err(failed)?;
    Ok(Refreshed { new: catalog.count_new(&current), models: catalog.model_count(), catalog: Arc::new(catalog) })
}

/// A catalogue's date as Settings shows it: "26 Nov 2026".
pub fn date(captured: u64) -> String {
    let day = local(captured);
    format!("{} {} {}", day.day, MONTHS[(day.month as usize).clamp(1, 12) - 1], day.year)
}

/// What filling a model row from a listing changes. `None` leaves the row's own value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fill {
    pub context: Option<u64>,
    pub output: Option<u64>,
    pub vision: Option<bool>,
    /// The efforts to tick: empty for a model that doesn't reason.
    pub efforts: Option<Vec<Reasoning>>,
    /// Something the user should check, shown under the row.
    pub note: Option<&'static str>,
}

/// What a listing says about a model row. A model that reasons with no efforts listed starts at
/// the default efforts, with a note asking the user to check them.
pub fn fill(listing: &Listing) -> Fill {
    let listed = listing.efforts.as_ref().map(|levels| levels.iter().map(|level| reasoning(*level)).collect::<Vec<_>>());
    let (efforts, note) = match (listing.reasons, listed) {
        (Some(false), _) => (Some(Vec::new()), None),
        (_, Some(efforts)) => (Some(efforts), listing.guessed.then_some(GUESSED_EFFORTS)),
        (Some(true), None) => (Some(DEFAULT_EFFORTS.to_vec()), Some(UNLISTED_EFFORTS)),
        (None, None) => (None, None),
    };
    Fill { context: listing.context, output: listing.output, vision: listing.vision, efforts, note }
}

fn reasoning(level: Level) -> Reasoning {
    match level {
        Level::Off => Reasoning::Off,
        Level::Minimal => Reasoning::Minimal,
        Level::Low => Reasoning::Low,
        Level::Medium => Reasoning::Medium,
        Level::High => Reasoning::High,
        Level::Xhigh => Reasoning::Xhigh,
        Level::Max => Reasoning::Max,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    const FEED: &str = r#"{"providers":{"p":{"name":"P","models":{"m":{"name":"M","tool_call":true,"limit":{"context":1000,"output":10}}}}},
        "models":{"lab/new":{"name":"New","tool_call":true}}}"#;

    /// Answers one connection with `reply`.
    async fn serve(reply: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 4096];
            let _ = socket.read(&mut buffer).await;
            let _ = socket.write_all(reply.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
        address
    }

    fn ok(body: &str) -> String {
        format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
    }

    fn limits() -> Limits {
        Limits { timeout: Duration::from_secs(5), cap: 1024 * 1024 }
    }

    fn copy(captured: u64) -> Catalog {
        Catalog { version: wack_catalog::VERSION, source: SOURCE.into(), captured, ..Catalog::default() }
    }

    #[test]
    fn the_newer_copy_is_used_and_a_damaged_one_passed_over() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join(FILE);
        let built_in = wack_catalog::bundled();
        assert_eq!(load(&path).captured, built_in.captured, "no copy yet");
        fs::write(&path, copy(built_in.captured + 1).to_json()).unwrap();
        assert_eq!(load(&path).captured, built_in.captured + 1);
        assert!(!is_built_in(&load(&path)));
        fs::write(&path, copy(1).to_json()).unwrap();
        assert!(is_built_in(&load(&path)), "an older copy than the app's own");
        fs::write(&path, "{ damaged").unwrap();
        assert!(is_built_in(&load(&path)));
        fs::write(&path, copy(u64::MAX).to_json().replacen("\"version\":1", "\"version\":2", 1)).unwrap();
        assert!(is_built_in(&load(&path)), "a copy in a format this app doesn't know");
    }

    #[tokio::test]
    async fn a_refresh_writes_the_new_copy_and_counts_whats_new() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join(FILE);
        let address = serve(ok(FEED)).await;
        let refreshed = refresh_from(&address, limits(), path.clone(), Arc::new(copy(1))).await.unwrap();
        assert_eq!((refreshed.models, refreshed.new), (2, 1));
        let saved = Catalog::parse(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved, *refreshed.catalog);
        assert!(!path.with_extension("json.tmp").exists());
        assert_eq!(load(&path).captured, saved.captured, "a refreshed copy is newer than the app's own");
    }

    #[tokio::test]
    async fn a_failed_refresh_keeps_the_copy_and_says_which() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join(FILE);
        fs::write(&path, "the old copy").unwrap();
        let current = Arc::new(copy(1_764_115_200_000));
        let still = format!(" The catalogue from {} is still in use.", date(current.captured));
        for (reply, said) in [
            ("HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n".to_string(), "models.dev answered with HTTP 503."),
            ("HTTP/1.1 301 Moved\r\nlocation: https://x.invalid/\r\ncontent-length: 0\r\n\r\n".to_string(), "somewhere else"),
            (ok("<html>not it</html>"), "isn't its model list"),
            (ok(r#"{"providers":{}}"#), "no models WackCode could read"),
            ("HTTP/1.1 200 OK\r\ncontent-length: 9999999\r\n\r\n".to_string(), "larger than"),
        ] {
            let address = serve(reply).await;
            let error = refresh_from(&address, limits(), path.clone(), current.clone()).await.unwrap_err();
            assert!(error.contains(said) && error.ends_with(&still), "{error}");
            assert_eq!(fs::read_to_string(&path).unwrap(), "the old copy");
        }
    }

    #[test]
    fn dates_read_as_settings_shows_them() {
        let text = date(1_764_115_200_000);
        assert!(text.ends_with(" Nov 2025"), "{text}");
        assert!(text.split(' ').next().unwrap().parse::<u32>().is_ok());
    }

    #[test]
    fn filling_gives_only_what_is_known() {
        let listing = |reasons, efforts: Option<Vec<Level>>, guessed| Listing {
            id: "m".into(),
            name: "M".into(),
            context: Some(200_000),
            reasons,
            efforts,
            guessed,
            ..Listing::default()
        };
        let plain = fill(&listing(Some(false), None, false));
        assert_eq!((plain.context, plain.output, plain.vision), (Some(200_000), None, None));
        assert_eq!((plain.efforts, plain.note), (Some(vec![]), None), "a model that doesn't reason turns Reasoning off");
        let listed = fill(&listing(Some(true), Some(vec![Level::Off, Level::High, Level::Max]), false));
        assert_eq!(listed.efforts, Some(vec![Reasoning::Off, Reasoning::High, Reasoning::Max]));
        assert_eq!(listed.note, None);
        let borrowed = fill(&listing(Some(true), Some(vec![Level::Low]), true));
        assert_eq!(borrowed.note, Some(GUESSED_EFFORTS));
        let unlisted = fill(&listing(Some(true), None, false));
        assert_eq!((unlisted.efforts, unlisted.note), (Some(DEFAULT_EFFORTS.to_vec()), Some(UNLISTED_EFFORTS)));
        let unknown = fill(&listing(None, None, false));
        assert_eq!((unknown.efforts, unknown.note), (None, None), "nothing said, nothing changed");
    }
}
