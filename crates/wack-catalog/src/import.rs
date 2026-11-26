//! models.dev's feed into the slim [`Catalog`].
//!
//! The feed is `{ providers: { <id>: { name, api, models: { <id>: Model } } }, models: { "<lab>/<id>":
//! Model } }`, and a Model carries far more than WackCode needs. Only these are read: `name`,
//! `type`, `tool_call`, `status`, `canonical_model_id`, `limit.context`, `limit.output`,
//! `reasoning`, `reasoning_options`, `modalities.input` and `modalities.output`. Anything else
//! (API formats, packages, prices, request settings) is never looked at, so a change there can't
//! break a refresh.
//!
//! Each entry is read on its own: a value of the wrong kind is unknown, and an entry that isn't an
//! object is skipped and counted. A model is left out only on evidence that an agent can't use it:
//! a `type` other than chat, `tool_call: false`, or output with no text.

use std::collections::{BTreeSet, HashMap};

use serde_json::{Map, Value};

use crate::{Catalog, Level, Listing, Provider, SOURCE, VERSION};

/// The longest id or name kept; anything longer is not a model id.
const MAX_TEXT: usize = 256;
/// The largest token count believed.
const MAX_TOKENS: u64 = 1_000_000_000;

/// A feed read into a catalogue, and how many entries had to be skipped.
#[derive(Debug)]
pub struct Imported {
    pub catalog: Catalog,
    pub skipped: usize,
}

/// Reads models.dev's feed. It fails only when the body is not the feed at all, or holds no model
/// WackCode can use; each error is a sentence naming models.dev.
pub fn import(body: &[u8], captured: u64) -> Result<Imported, String> {
    let feed: Value =
        serde_json::from_slice(body).map_err(|_| "models.dev sent something that isn't its model list.".to_string())?;
    let Some(providers) = feed.get("providers").and_then(Value::as_object) else {
        return Err("models.dev's list has no providers in it, so it can't be read.".to_string());
    };
    let empty = Map::new();
    let makers = feed.get("models").and_then(Value::as_object).unwrap_or(&empty);
    let mut skipped = 0;

    let mut catalog = Catalog { version: VERSION, source: SOURCE.to_string(), captured, ..Catalog::default() };
    for (provider_id, provider) in providers {
        let Some(models) = provider.get("models").and_then(Value::as_object) else {
            skipped += 1;
            continue;
        };
        let Some(provider_id) = text(provider_id) else {
            skipped += 1;
            continue;
        };
        let mut listings = Vec::new();
        for (id, model) in models {
            match read(id, model) {
                Read::Listing(mut listing) => {
                    listing.canonical = model
                        .get("canonical_model_id")
                        .and_then(Value::as_str)
                        .filter(|canonical| makers.contains_key(*canonical))
                        .map(str::to_string)
                        .or_else(|| {
                            let own = format!("{provider_id}/{}", listing.id);
                            makers.contains_key(&own).then_some(own)
                        });
                    listings.push(listing);
                }
                Read::Unusable => {}
                Read::Malformed => skipped += 1,
            }
        }
        listings.sort_by(|a, b| a.id.cmp(&b.id));
        catalog.providers.push(Provider {
            id: provider_id.to_string(),
            name: provider.get("name").and_then(Value::as_str).and_then(text).unwrap_or(provider_id).to_string(),
            api: provider.get("api").and_then(Value::as_str).filter(|api| crate::hosts::endpoint_key(api).is_some()).map(str::to_string),
            models: listings,
        });
    }
    for (id, model) in makers {
        if !id.contains('/') {
            skipped += 1;
            continue;
        }
        match read(id, model) {
            Read::Listing(listing) => catalog.models.push(listing),
            Read::Unusable => {}
            Read::Malformed => skipped += 1,
        }
    }
    catalog.providers.sort_by(|a, b| a.id.cmp(&b.id));
    catalog.models.sort_by(|a, b| a.id.cmp(&b.id));
    borrow_efforts(&mut catalog);
    // A maker's record that no provider lists and nothing usable came of is still a model; only a
    // feed with nothing at all is refused.
    if catalog.listings().next().is_none() {
        return Err("models.dev's list had no models WackCode could read.".to_string());
    }
    Ok(Imported { catalog, skipped })
}

enum Read {
    Listing(Listing),
    /// A model an agent can't use: an embedding, image or speech model, or one without tools.
    Unusable,
    /// Not a model entry at all.
    Malformed,
}

fn read(id: &str, model: &Value) -> Read {
    let (Some(id), Some(model)) = (text(id), model.as_object()) else {
        return Read::Malformed;
    };
    let kind = model.get("type").and_then(Value::as_str);
    let output_has_text =
        model.get("modalities").and_then(|modalities| modalities.get("output")).and_then(Value::as_array).is_none_or(|kinds| {
            kinds.iter().any(|kind| kind.as_str() == Some("text"))
        });
    if kind.is_some_and(|kind| kind != "chat") || model.get("tool_call").and_then(Value::as_bool) == Some(false) || !output_has_text {
        return Read::Unusable;
    }
    let limit = |key: &str| model.get("limit").and_then(|limit| limit.get(key)).and_then(tokens);
    let reasons = model.get("reasoning").and_then(Value::as_bool);
    Read::Listing(Listing {
        id: id.to_string(),
        name: model.get("name").and_then(Value::as_str).and_then(text).unwrap_or(id).to_string(),
        canonical: None,
        context: limit("context"),
        output: limit("output"),
        reasons,
        efforts: if reasons == Some(false) { None } else { efforts(model.get("reasoning_options")) },
        guessed: false,
        vision: model
            .get("modalities")
            .and_then(|modalities| modalities.get("input"))
            .and_then(Value::as_array)
            .map(|kinds| kinds.iter().filter_map(Value::as_str).any(|kind| kind == "image" || kind.starts_with("image/"))),
        deprecated: model.get("status").and_then(Value::as_str) == Some("deprecated"),
    })
}

/// Trimmed text that could be an id or a name.
fn text(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty() && value.len() <= MAX_TEXT).then_some(value)
}

/// A token count: a whole number from 1 up. Zero (models.dev's "unknown"), fractions, negatives
/// and text are all unknown.
fn tokens(value: &Value) -> Option<u64> {
    let tokens = value.as_u64().or_else(|| value.as_f64().filter(|n| n.fract() == 0.0 && *n >= 0.0).map(|n| n as u64))?;
    (1..=MAX_TOKENS).contains(&tokens).then_some(tokens)
}

/// The efforts a model's `reasoning_options` list. An `effort` option's values are levels (`none`
/// is Off, an unknown word is passed over); a `toggle` means it can also run without reasoning.
/// A budget alone, or nothing above Off, says nothing about efforts: unknown.
fn efforts(options: Option<&Value>) -> Option<Vec<Level>> {
    let mut levels = BTreeSet::new();
    for option in options?.as_array()? {
        match option.get("type").and_then(Value::as_str) {
            Some("effort") => {
                for value in option.get("values").and_then(Value::as_array).into_iter().flatten() {
                    if let Some(level) = value.as_str().and_then(level) {
                        levels.insert(level);
                    }
                }
            }
            Some("toggle") => {
                levels.insert(Level::Off);
            }
            _ => {}
        }
    }
    levels.iter().any(|level| *level != Level::Off).then(|| levels.into_iter().collect())
}

fn level(word: &str) -> Option<Level> {
    Some(match word.trim().to_ascii_lowercase().as_str() {
        "none" | "off" => Level::Off,
        "minimal" => Level::Minimal,
        "low" => Level::Low,
        "medium" => Level::Medium,
        "high" => Level::High,
        "xhigh" => Level::Xhigh,
        "max" => Level::Max,
        _ => return None,
    })
}

/// A model that reasons with no efforts listed borrows its maker's: from the maker's record, or
/// from the maker's own provider listing ("anthropic/x" from provider `anthropic`, model `x`),
/// marked `guessed`. Makers' records never list efforts, so they always borrow.
fn borrow_efforts(catalog: &mut Catalog) {
    let mut own: HashMap<String, Vec<Level>> = HashMap::new();
    for provider in &catalog.providers {
        for listing in &provider.models {
            if let Some(efforts) = &listing.efforts {
                own.insert(format!("{}/{}", provider.id, listing.id), efforts.clone());
            }
        }
    }
    for model in &mut catalog.models {
        if model.reasons == Some(true) && model.efforts.is_none() && let Some(efforts) = own.get(&model.id) {
            model.efforts = Some(efforts.clone());
            model.guessed = true;
        }
    }
    let makers: HashMap<String, Vec<Level>> =
        catalog.models.iter().filter_map(|model| Some((model.id.clone(), model.efforts.clone()?))).collect();
    for provider in &mut catalog.providers {
        for listing in &mut provider.models {
            if listing.reasons != Some(true) || listing.efforts.is_some() {
                continue;
            }
            if let Some(efforts) = listing.canonical.as_ref().and_then(|canonical| makers.get(canonical)) {
                listing.efforts = Some(efforts.clone());
                listing.guessed = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A feed with what version 2 choked on: Mistral's own API format and package, odd values,
    /// entries that aren't models, and efforts of every kind.
    fn feed() -> Value {
        json!({
            "providers": {
                "mistral": {
                    "name": "Mistral", "npm": "@ai-sdk/mistral-conversations-next", "api": "https://api.mistral.ai/v1",
                    "models": {
                        "devstral-3": {
                            "name": "Devstral 3", "tool_call": true, "reasoning": false, "shape": "mistral-conversations",
                            "provider": { "npm": "@ai-sdk/mistral", "api": "${MISTRAL_BASE}" },
                            "modalities": { "input": ["text", "image/png"], "output": ["text"] },
                            "limit": { "context": 256000, "output": 32768.0 }
                        }
                    }
                },
                "anthropic": {
                    "name": "Anthropic",
                    "models": {
                        "claude-opus-5-5": {
                            "name": "Claude Opus 5.5", "tool_call": true, "reasoning": true,
                            "reasoning_options": [{ "type": "effort", "values": ["low", "medium", "high", "xhigh", "max", "turbo"] }],
                            "modalities": { "input": ["text", "image", "pdf"], "output": ["text"] },
                            "limit": { "context": 1000000, "output": 128000 },
                            "canonical_model_id": "anthropic/claude-opus-5-5"
                        },
                        "claude-old": { "name": "Claude Old", "tool_call": true, "reasoning": true, "status": "deprecated",
                            "reasoning_options": [{ "type": "toggle" }, { "type": "effort", "values": ["high"] }],
                            "limit": { "context": "200k", "output": -1 } }
                    }
                },
                "gateway": {
                    "name": "Gateway", "api": "https://gateway.example/v1",
                    "models": {
                        "anthropic/claude-opus-5.5": { "name": "Opus via gateway", "tool_call": true, "reasoning": true,
                            "reasoning_options": [{ "type": "budget_tokens", "min": 1024 }],
                            "canonical_model_id": "anthropic/claude-opus-5-5", "limit": { "context": 0, "output": 64000 } },
                        "toggle-only": { "name": "Toggle", "tool_call": true, "reasoning": true, "reasoning_options": [{ "type": "toggle" }] },
                        "yes-reasoning": { "name": "Yes", "tool_call": true, "reasoning": "yes", "reasoning_options": [{ "type": "effort", "values": [null, 3] }] },
                        "embedder": { "name": "Embed", "type": "embedding" },
                        "no-tools": { "name": "No tools", "tool_call": false },
                        "painter": { "name": "Painter", "tool_call": true, "modalities": { "input": ["text"], "output": ["image"] } },
                        "not-a-model": "oops",
                        "": { "name": "blank id" }
                    }
                },
                "broken": { "name": "Broken", "models": [1, 2, 3] },
                "templated": { "name": "Templated", "api": "https://${ACCOUNT}.example/v1", "models": {} }
            },
            "models": {
                "anthropic/claude-opus-5-5": { "name": "Claude Opus 5.5", "tool_call": true, "reasoning": true,
                    "modalities": { "input": ["text", "image"], "output": ["text"] }, "limit": { "context": 1000000, "output": 128000 } },
                "no-slash": { "name": "Bad" },
                "openai/whisper": { "name": "Whisper", "type": "transcription" }
            }
        })
    }

    fn imported() -> Imported {
        import(feed().to_string().as_bytes(), 1_764_000_000_000).unwrap()
    }

    fn listing<'a>(catalog: &'a Catalog, provider: &str, id: &str) -> &'a Listing {
        let provider = catalog.providers.iter().find(|candidate| candidate.id == provider).unwrap();
        provider.models.iter().find(|listing| listing.id == id).unwrap()
    }

    #[test]
    fn mistrals_own_format_and_odd_values_never_sink_the_feed() {
        let Imported { catalog, skipped } = imported();
        let devstral = listing(&catalog, "mistral", "devstral-3");
        assert_eq!((devstral.context, devstral.output), (Some(256_000), Some(32_768)), "a whole float is a number");
        assert_eq!((devstral.reasons, devstral.efforts.clone()), (Some(false), None));
        assert_eq!(devstral.vision, Some(true), "image/png is a picture");
        // "not-a-model", the blank id, the broken provider and "no-slash" were skipped; the
        // unusable models were left out without counting as damage.
        assert_eq!(skipped, 4);
        assert_eq!(catalog.version, VERSION);
        assert_eq!(catalog.captured, 1_764_000_000_000);
    }

    #[test]
    fn only_evidence_leaves_a_model_out() {
        let Imported { catalog, .. } = imported();
        let gateway = catalog.providers.iter().find(|provider| provider.id == "gateway").unwrap();
        let ids: Vec<&str> = gateway.models.iter().map(|listing| listing.id.as_str()).collect();
        assert_eq!(ids, ["anthropic/claude-opus-5.5", "toggle-only", "yes-reasoning"]);
        assert!(catalog.models.iter().all(|model| model.id != "openai/whisper"));
        let yes = listing(&catalog, "gateway", "yes-reasoning");
        assert_eq!((yes.reasons, yes.efforts.clone(), yes.vision), (None, None, None), "wrong kinds are unknown");
    }

    #[test]
    fn efforts_map_to_levels_and_unknown_words_are_passed_over() {
        let Imported { catalog, .. } = imported();
        let opus = listing(&catalog, "anthropic", "claude-opus-5-5");
        assert_eq!(opus.efforts, Some(vec![Level::Low, Level::Medium, Level::High, Level::Xhigh, Level::Max]));
        assert!(!opus.guessed);
        let old = listing(&catalog, "anthropic", "claude-old");
        assert_eq!(old.efforts, Some(vec![Level::Off, Level::High]), "a toggle adds Off");
        assert!(old.deprecated);
        assert_eq!((old.context, old.output), (None, None), "text and negatives are unknown");
        assert_eq!(listing(&catalog, "gateway", "toggle-only").efforts, None, "a toggle alone says nothing about efforts");
    }

    #[test]
    fn a_host_without_efforts_borrows_its_makers_marked_guessed() {
        let Imported { catalog, .. } = imported();
        let via = listing(&catalog, "gateway", "anthropic/claude-opus-5.5");
        assert_eq!(via.canonical.as_deref(), Some("anthropic/claude-opus-5-5"));
        assert_eq!(via.efforts, Some(vec![Level::Low, Level::Medium, Level::High, Level::Xhigh, Level::Max]));
        assert!(via.guessed);
        assert_eq!((via.context, via.output), (None, Some(64_000)), "zero is unknown, never borrowed");
        let maker = catalog.models.iter().find(|model| model.id == "anthropic/claude-opus-5-5").unwrap();
        assert!(maker.guessed && maker.efforts.is_some(), "a maker's record borrows from its own provider");
        assert_eq!(listing(&catalog, "anthropic", "claude-opus-5-5").canonical.as_deref(), Some("anthropic/claude-opus-5-5"));
    }

    #[test]
    fn templated_addresses_are_not_kept() {
        let Imported { catalog, .. } = imported();
        let api = |id: &str| catalog.providers.iter().find(|provider| provider.id == id).unwrap().api.clone();
        assert_eq!(api("templated"), None);
        assert_eq!(api("gateway").as_deref(), Some("https://gateway.example/v1"));
    }

    #[test]
    fn only_a_feed_with_nothing_usable_is_refused() {
        assert_eq!(import(b"<html>", 0).unwrap_err(), "models.dev sent something that isn't its model list.");
        assert!(import(b"{\"models\":{}}", 0).unwrap_err().contains("no providers"));
        let empty = json!({ "providers": { "x": { "models": { "e": { "type": "embedding" } } } } });
        assert_eq!(import(empty.to_string().as_bytes(), 0).unwrap_err(), "models.dev's list had no models WackCode could read.");
        let no_makers = json!({ "providers": { "x": { "models": { "m": { "tool_call": true } } } } });
        assert!(import(no_makers.to_string().as_bytes(), 0).is_ok(), "the makers' records are optional");
    }

    #[test]
    fn nothing_but_facts_survive() {
        let text = imported().catalog.to_json();
        for leak in ["${", "npm", "shape", "mistral-conversations", "cost", "budget"] {
            assert!(!text.contains(leak), "{leak} leaked into the catalogue");
        }
    }
}
