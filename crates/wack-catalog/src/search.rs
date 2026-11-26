//! Finding models: [`Catalog::search`] for the suggestions under a model id field as the user
//! types, and [`Catalog::lookup`] for the ids a provider lists (Fetch models).
//!
//! When the address matches a provider, its listings come first, with the ids that address
//! accepts and its own limits. Otherwise (a server on this Mac, an unknown gateway) the makers'
//! records answer, under their bare ids, with the maker's efforts marked as borrowed.

use crate::{Catalog, Listing};

/// How many suggestions show at once.
pub const SUGGESTIONS: usize = 8;

/// A model the catalogue offers for a row.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    /// What the model id field holds once it is picked.
    pub id: String,
    pub listing: Listing,
    /// Where the facts come from: the provider's name, or "models.dev" for a maker's record.
    pub source: String,
    pub matched: Match,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Match {
    /// The provider at this address lists it under this id.
    Provider,
    /// The maker's own record.
    Maker,
    /// The maker's record, found by an id written another way (a date, a tag, dots for dashes).
    Normalised,
}

/// Lowercase letters-and-digits runs: "Claude Opus 5.5" is `claude opus 5 5`.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()).map(str::to_string).collect()
}

/// How well `listing` answers the query, best first, as version 2 ranked: an exact id, an exact
/// name, a name that starts with it, an id that starts with it, then every word a prefix of one of
/// the model's words, then every word inside one. `None` when it doesn't match at all.
fn score(listing: &Listing, query: &[String]) -> Option<u8> {
    let needle = query.join(" ");
    let bare = |id: &str| id.rsplit('/').next().unwrap_or(id).to_string();
    let mut ids = vec![listing.id.clone(), bare(&listing.id)];
    if let Some(canonical) = &listing.canonical {
        ids.push(canonical.clone());
        ids.push(bare(canonical));
    }
    let ids: Vec<String> = ids.iter().map(|id| words(id).join(" ")).collect();
    let name = words(&listing.name).join(" ");
    let all: Vec<String> = ids.iter().chain(std::iter::once(&name)).flat_map(|text| text.split(' ').map(str::to_string)).collect();
    if ids.contains(&needle) {
        Some(0)
    } else if name == needle {
        Some(1)
    } else if name.starts_with(&needle) {
        Some(2)
    } else if ids.iter().any(|id| id.starts_with(&needle)) {
        Some(3)
    } else if query.iter().all(|part| all.iter().any(|word| word.starts_with(part.as_str()))) {
        Some(4)
    } else if query.iter().all(|part| all.iter().any(|word| word.contains(part.as_str()))) {
        Some(5)
    } else {
        None
    }
}

/// An id written the way different hosts disagree about it: lowercase, no vendor or `models/`
/// prefix, no `:tag`, no date or `-latest` suffix, and dots as dashes.
fn normalise(id: &str) -> String {
    let mut id = id.trim().to_lowercase();
    if let Some(bare) = id.rsplit('/').next() {
        id = bare.to_string();
    }
    if let Some((before, _)) = id.split_once(':') {
        id = before.to_string();
    }
    if let Some((before, _)) = id.split_once('@') {
        id = before.to_string();
    }
    id = id.replace(['.', '_'], "-");
    if let Some(stripped) = id.strip_suffix("-latest") {
        id = stripped.to_string();
    }
    // -20250514, or -2025-05-14.
    let digits = |text: &str| !text.is_empty() && text.chars().all(|c| c.is_ascii_digit());
    let parts: Vec<&str> = id.split('-').collect();
    let keep = match parts.as_slice() {
        [rest @ .., year, month, day] if !rest.is_empty() && year.len() == 4 && digits(year) && month.len() == 2 && digits(month) && day.len() == 2 && digits(day) => rest.len(),
        [rest @ .., date] if !rest.is_empty() && date.len() == 8 && digits(date) => rest.len(),
        _ => parts.len(),
    };
    parts[..keep].join("-")
}

impl Catalog {
    /// Up to `limit` models for what the user has typed into a model id field, best first. The
    /// matched provider's listings come first; the makers' records answer when it has none.
    pub fn search(&self, query: &str, base_url: &str, limit: usize) -> Vec<Suggestion> {
        let query = words(query);
        if query.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<(u8, bool, Suggestion)> = Vec::new();
        for provider in self.providers_for(base_url) {
            for listing in &provider.models {
                if let Some(score) = score(listing, &query) {
                    let suggestion =
                        Suggestion { id: listing.id.clone(), listing: listing.clone(), source: provider.name.clone(), matched: Match::Provider };
                    found.push((score, listing.deprecated, suggestion));
                }
            }
        }
        if found.is_empty() {
            for model in &self.models {
                if let Some(score) = score(model, &query) {
                    let id = model.id.rsplit('/').next().unwrap_or(&model.id).to_string();
                    let suggestion = Suggestion { id, listing: model.clone(), source: "models.dev".to_string(), matched: Match::Maker };
                    found.push((score, model.deprecated, suggestion));
                }
            }
        }
        found.sort_by(|a, b| {
            (a.0, a.1, &a.2.listing.name, &a.2.id).cmp(&(b.0, b.1, &b.2.listing.name, &b.2.id))
        });
        found.dedup_by(|a, b| a.2.id == b.2.id);
        found.into_iter().take(limit).map(|(_, _, suggestion)| suggestion).collect()
    }

    /// What the catalogue knows about `id`, as the provider at `base_url` listed it: its own
    /// listing, else the maker's record under that id, its bare id, or (only when exactly one
    /// fits) the id written another way. `None` rather than a guess between several.
    pub fn lookup(&self, id: &str, base_url: &str) -> Option<Suggestion> {
        let id = id.trim();
        if id.is_empty() {
            return None;
        }
        let providers = self.providers_for(base_url);
        let own = |listing: &Listing, provider: &str| Suggestion {
            id: id.to_string(),
            listing: listing.clone(),
            source: provider.to_string(),
            matched: Match::Provider,
        };
        let maker = |listing: &Listing, matched: Match| Suggestion {
            id: id.to_string(),
            listing: listing.clone(),
            source: "models.dev".to_string(),
            matched,
        };
        for exact in [true, false] {
            for provider in &providers {
                let found = provider.models.iter().find(|listing| {
                    if exact { listing.id == id } else { listing.id.eq_ignore_ascii_case(id) }
                });
                if let Some(listing) = found {
                    return Some(own(listing, &provider.name));
                }
            }
        }
        if let Some(model) = self.models.iter().find(|model| model.id.eq_ignore_ascii_case(id)) {
            return Some(maker(model, Match::Maker));
        }
        let bare: Vec<&Listing> =
            self.models.iter().filter(|model| model.id.rsplit('/').next().is_some_and(|bare| bare.eq_ignore_ascii_case(id))).collect();
        if let [model] = bare.as_slice() {
            return Some(maker(model, Match::Maker));
        }
        let wanted = normalise(id);
        for provider in &providers {
            let close: Vec<&Listing> = provider.models.iter().filter(|listing| normalise(&listing.id) == wanted).collect();
            if let [listing] = close.as_slice() {
                return Some(own(listing, &provider.name));
            }
        }
        let close: Vec<&Listing> = self.models.iter().filter(|model| normalise(&model.id) == wanted).collect();
        match close.as_slice() {
            [model] => Some(maker(model, Match::Normalised)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Level, Provider};

    fn listing(id: &str, name: &str) -> Listing {
        Listing { id: id.into(), name: name.into(), ..Listing::default() }
    }

    fn catalog() -> Catalog {
        Catalog {
            providers: vec![
                Provider {
                    id: "openrouter".into(),
                    name: "OpenRouter".into(),
                    api: Some("https://openrouter.ai/api/v1".into()),
                    models: vec![
                        Listing { canonical: Some("anthropic/claude-opus-5-5".into()), context: Some(1_000_000), ..listing("anthropic/claude-opus-5.5", "Anthropic: Claude Opus 5.5") },
                        Listing { canonical: Some("anthropic/claude-opus-5".into()), deprecated: true, ..listing("anthropic/claude-opus-5", "Anthropic: Claude Opus 5") },
                        listing("anthropic/claude-sonnet-5.5", "Anthropic: Claude Sonnet 5.5"),
                        listing("openai/gpt-5.6-sol", "OpenAI: GPT-5.6 Sol"),
                    ],
                },
                Provider {
                    id: "anthropic".into(),
                    name: "Anthropic".into(),
                    api: None,
                    models: vec![Listing { efforts: Some(vec![Level::Low, Level::Max]), ..listing("claude-opus-5-5", "Claude Opus 5.5") }],
                },
            ],
            models: vec![
                Listing { efforts: Some(vec![Level::Low, Level::Max]), guessed: true, ..listing("anthropic/claude-opus-5-5", "Claude Opus 5.5") },
                listing("anthropic/claude-opus-5", "Claude Opus 5"),
                listing("qwen/qwen3-coder", "Qwen3 Coder"),
                listing("qwen/qwen3-coder-plus", "Qwen3 Coder Plus"),
                listing("zai/glm-5.3", "GLM-5.3"),
                listing("other/glm-5.3", "GLM-5.3 (other)"),
            ],
            ..Catalog::default()
        }
    }

    fn ids(found: Vec<Suggestion>) -> Vec<String> {
        found.into_iter().map(|suggestion| suggestion.id).collect()
    }

    #[test]
    fn a_matched_provider_answers_with_its_own_ids_best_first() {
        let catalog = catalog();
        let found = catalog.search("opus", "https://openrouter.ai/api/v1", SUGGESTIONS);
        assert_eq!(ids(found.clone()), ["anthropic/claude-opus-5.5", "anthropic/claude-opus-5"], "deprecated ranks after");
        assert_eq!((found[0].source.as_str(), found[0].matched), ("OpenRouter", Match::Provider));
        assert_eq!(found[0].listing.context, Some(1_000_000));
        assert_eq!(ids(catalog.search("claude opus 5.5", "https://openrouter.ai/api/v1", 8))[0], "anthropic/claude-opus-5.5");
        assert_eq!(ids(catalog.search("gpt-5.6-sol", "https://openrouter.ai/api/v1", 8)), ["openai/gpt-5.6-sol"], "an exact bare id");
        assert_eq!(ids(catalog.search("son", "https://openrouter.ai/api/v1", 8)), ["anthropic/claude-sonnet-5.5"]);
    }

    #[test]
    fn elsewhere_the_makers_records_answer_under_bare_ids() {
        let catalog = catalog();
        let found = catalog.search("qwen3 coder", "http://localhost:11434/v1", SUGGESTIONS);
        assert_eq!(ids(found.clone()), ["qwen3-coder", "qwen3-coder-plus"], "an exact name first");
        assert_eq!((found[0].source.as_str(), found[0].matched), ("models.dev", Match::Maker));
        assert!(catalog.search("", "https://openrouter.ai/api/v1", 8).is_empty());
        assert!(catalog.search("  - ", "", 8).is_empty());
        assert_eq!(catalog.search("o", "", 2).len(), 2, "the limit holds");
    }

    #[test]
    fn a_provider_without_the_model_falls_back_to_the_makers() {
        let found = catalog().search("qwen", "https://openrouter.ai/api/v1", 8);
        assert_eq!(ids(found), ["qwen3-coder", "qwen3-coder-plus"]);
    }

    #[test]
    fn lookups_prefer_the_providers_own_listing_then_the_makers() {
        let catalog = catalog();
        let at = |id: &str, address: &str| catalog.lookup(id, address).map(|found| (found.listing.id, found.matched));
        assert_eq!(at("claude-opus-5-5", "https://api.anthropic.com"), Some(("claude-opus-5-5".into(), Match::Provider)));
        assert_eq!(at("ANTHROPIC/claude-opus-5.5", "https://openrouter.ai/api/v1"), Some(("anthropic/claude-opus-5.5".into(), Match::Provider)));
        assert_eq!(at("anthropic/claude-opus-5-5", "http://localhost:8080"), Some(("anthropic/claude-opus-5-5".into(), Match::Maker)));
        assert_eq!(at("qwen3-coder", "http://localhost:8080"), Some(("qwen/qwen3-coder".into(), Match::Maker)));
        assert_eq!(at("claude-opus-5.5-20261101", "http://localhost:8080"), Some(("anthropic/claude-opus-5-5".into(), Match::Normalised)));
        assert_eq!(at("qwen3-coder:30b", "http://localhost:8080"), Some(("qwen/qwen3-coder".into(), Match::Normalised)));
        assert_eq!(at("glm-5.3", "http://localhost:8080"), None, "two makers share it: no guess");
        assert_eq!(at("unheard-of", "http://localhost:8080"), None);
        let found = catalog.lookup("qwen3-coder:30b", "").unwrap();
        assert_eq!(found.id, "qwen3-coder:30b", "the provider's own id is kept");
    }

    #[test]
    fn ids_normalise_the_ways_hosts_disagree() {
        assert_eq!(normalise("anthropic/Claude-Opus-4-5-20251101"), "claude-opus-4-5");
        assert_eq!(normalise("models/gemini-3.0-pro-latest"), "gemini-3-0-pro");
        assert_eq!(normalise("gpt-5.6-2026-10-01"), "gpt-5-6");
        assert_eq!(normalise("qwen3-coder:30b"), "qwen3-coder");
        assert_eq!(normalise("claude-opus-4@20250514"), "claude-opus-4");
        assert_eq!(normalise("model-2026"), "model-2026", "a year alone is part of the name");
    }
}
