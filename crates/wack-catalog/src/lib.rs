//! The model catalogue: what is known about models, to fill a model row in Settings › Providers
//! (its context window, max output, reasoning efforts and whether it reads pictures) instead of
//! typing them.
//!
//! - **Where it comes from.** [models.dev](https://models.dev)'s feed, slimmed by [`import`] to
//!   only those facts. A snapshot ships inside the app (`data/models.json`, read lazily by
//!   [`bundled`]), so it works offline and never breaks; `wack-core` keeps a newer copy when the
//!   user presses Refresh. `cargo catalog` regenerates the snapshot with the same importer.
//! - **Lenient on purpose.** Version 2 checked the whole feed at once and refused it for one odd
//!   value anywhere (and, from 2026-11-04, for an API format it had never heard of), so every
//!   refresh failed. Here a bad value is unknown, a bad entry is skipped and counted, and only a
//!   feed with nothing usable in it is refused. Nothing is read that is not used.
//! - **Facts, never destinations.** A provider's `api` address and the code-owned [`hosts`]
//!   hints only rank suggestions for the address the user typed. Nothing here is ever contacted
//!   except the feed itself, by [`download`], when the user asks.

use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

mod download;
mod hosts;
mod import;
mod search;

pub use download::{Limits, download};
pub use import::{Imported, import};
pub use search::{Match, SUGGESTIONS, Suggestion};

/// The feed, and the only address this crate ever contacts.
pub const SOURCE: &str = "https://models.dev/catalog.json";
/// The version of the slim format below. A copy with another version is not read.
pub const VERSION: u32 = 1;

/// The snapshot built into the app.
const BUNDLED: &str = include_str!("../data/models.json");

/// A reasoning effort a model accepts, low to high. `Off` is "can run without reasoning".
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

/// What the catalogue knows, in the slim form the app keeps.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    pub version: u32,
    pub source: String,
    /// When the feed was read, in milliseconds since 1970.
    pub captured: u64,
    /// Each host and the models it serves, under its own ids.
    pub providers: Vec<Provider>,
    /// The makers' own records ("anthropic/claude-opus-5-5"), for addresses no provider matches.
    pub models: Vec<Listing>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    /// The provider's API address as models.dev gives it: used only to match the user's address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    pub models: Vec<Listing>,
}

/// One model as one provider (or its maker) lists it. Every fact is optional: unknown stays
/// unknown, and filling a row leaves an unknown field as the user had it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Listing {
    pub id: String,
    pub name: String,
    /// The maker's record this listing is a host's copy of ("anthropic/claude-opus-5-5").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical: Option<String>,
    /// The context window, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
    /// The most it writes in one reply, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    /// It can reason at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasons: Option<bool>,
    /// The efforts it accepts, low to high. `None` when it reasons but nobody says how.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub efforts: Option<Vec<Level>>,
    /// The efforts are the maker's own, borrowed for a host that doesn't list its own: that host
    /// may refuse some.
    #[serde(default, skip_serializing_if = "is_false")]
    pub guessed: bool,
    /// It reads pictures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision: Option<bool>,
    /// Its provider marks it deprecated; it ranks below the others.
    #[serde(default, skip_serializing_if = "is_false")]
    pub deprecated: bool,
    /// When it came out ("2026-11-09"): among equally good matches, the newest comes first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Catalog {
    /// Reads a catalogue in the slim form. `None` for anything else, a different version included.
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str::<Catalog>(text).ok().filter(|catalog| catalog.version == VERSION)
    }

    /// Every model listing, providers' and makers'.
    pub fn listings(&self) -> impl Iterator<Item = &Listing> {
        self.providers.iter().flat_map(|provider| provider.models.iter()).chain(self.models.iter())
    }

    /// How many models it knows: the makers' records, plus listings no maker's record covers.
    pub fn model_count(&self) -> usize {
        self.models.len() + self.providers.iter().flat_map(|provider| &provider.models).filter(|listing| listing.canonical.is_none()).count()
    }

    /// How many makers' models this catalogue has that `older` lacked.
    pub fn count_new(&self, older: &Catalog) -> usize {
        let known: std::collections::HashSet<&str> = older.models.iter().map(|model| model.id.as_str()).collect();
        self.models.iter().filter(|model| !known.contains(model.id.as_str())).count()
    }

    /// The catalogue as the app stores it: one listing per line, so a snapshot's diff shows
    /// which models changed.
    pub fn to_json(&self) -> String {
        let line = |listing: &Listing| serde_json::to_string(listing).unwrap_or_default();
        let mut out = format!(
            "{{\"version\":{},\"source\":{},\"captured\":{},\n\"providers\":[\n",
            self.version,
            serde_json::to_string(&self.source).unwrap_or_default(),
            self.captured
        );
        for (index, provider) in self.providers.iter().enumerate() {
            let head = serde_json::json!({ "id": provider.id, "name": provider.name });
            let mut head = serde_json::to_string(&head).unwrap_or_default();
            head.pop();
            out.push_str(&head);
            if let Some(api) = &provider.api {
                out.push_str(&format!(",\"api\":{}", serde_json::to_string(api).unwrap_or_default()));
            }
            out.push_str(",\"models\":[\n");
            let models: Vec<String> = provider.models.iter().map(line).collect();
            out.push_str(&models.join(",\n"));
            out.push_str(if index + 1 == self.providers.len() { "\n]}\n" } else { "\n]},\n" });
        }
        out.push_str("],\n\"models\":[\n");
        let models: Vec<String> = self.models.iter().map(line).collect();
        out.push_str(&models.join(",\n"));
        out.push_str("\n]}\n");
        out
    }
}

/// The snapshot built into the app, read the first time it is asked for. A snapshot that fails
/// to read gives an empty catalogue rather than a crash; a test keeps that from shipping.
pub fn bundled() -> Arc<Catalog> {
    static BUNDLE: OnceLock<Arc<Catalog>> = OnceLock::new();
    BUNDLE.get_or_init(|| Arc::new(Catalog::parse(BUNDLED).unwrap_or_default())).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_snapshot_reads_and_is_sane() {
        let catalog = Catalog::parse(BUNDLED).expect("data/models.json must parse: run `cargo catalog`");
        assert_eq!(catalog.version, VERSION);
        assert_eq!(catalog.source, SOURCE);
        assert!(catalog.captured > 1_700_000_000_000, "captured is in milliseconds");
        assert!(catalog.models.len() > 300, "{} makers' models", catalog.models.len());
        assert!(catalog.providers.len() > 50, "{} providers", catalog.providers.len());
        for listing in catalog.listings() {
            assert!(!listing.id.trim().is_empty() && !listing.name.trim().is_empty());
            for limit in [listing.context, listing.output].into_iter().flatten() {
                assert!((1..=1_000_000_000).contains(&limit), "{}: {limit}", listing.id);
            }
            if let Some(efforts) = &listing.efforts {
                assert!(efforts.iter().any(|level| *level != Level::Off), "{}: efforts above Off", listing.id);
                assert!(efforts.windows(2).all(|pair| pair[0] < pair[1]), "{}: sorted", listing.id);
            }
        }
        assert!(bundled().providers.iter().any(|provider| provider.id == "anthropic"));
    }

    #[test]
    fn the_stored_form_reads_back_exactly() {
        let catalog = Catalog {
            version: VERSION,
            source: SOURCE.into(),
            captured: 1_764_000_000_000,
            providers: vec![
                Provider {
                    id: "a".into(),
                    name: "A \"quoted\"".into(),
                    api: Some("https://a.example/v1".into()),
                    models: vec![
                        Listing { id: "m1".into(), name: "M1".into(), context: Some(200_000), ..Listing::default() },
                        Listing {
                            id: "m2".into(),
                            name: "M2".into(),
                            reasons: Some(true),
                            efforts: Some(vec![Level::Low, Level::High]),
                            guessed: true,
                            vision: Some(false),
                            deprecated: true,
                            ..Listing::default()
                        },
                    ],
                },
                Provider { id: "b".into(), name: "B".into(), api: None, models: vec![] },
            ],
            models: vec![Listing { id: "lab/m1".into(), name: "M1".into(), ..Listing::default() }],
        };
        let text = catalog.to_json();
        assert!(text.lines().count() > 6, "one listing per line");
        assert_eq!(Catalog::parse(&text), Some(catalog.clone()));
        let other = Catalog { version: 2, ..catalog };
        assert_eq!(Catalog::parse(&serde_json::to_string(&other).unwrap()), None, "another version is not read");
    }

    #[test]
    fn new_models_are_counted_by_the_makers_ids() {
        let model = |id: &str| Listing { id: id.into(), name: id.into(), ..Listing::default() };
        let old = Catalog { models: vec![model("a/1"), model("a/2")], ..Catalog::default() };
        let new = Catalog { models: vec![model("a/1"), model("a/2"), model("b/3")], ..Catalog::default() };
        assert_eq!(new.count_new(&old), 1);
        assert_eq!(old.count_new(&new), 0);
    }
}
