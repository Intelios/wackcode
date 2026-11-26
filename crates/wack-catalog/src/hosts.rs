//! Which of the catalogue's providers the user's base URL belongs to, so suggestions come with
//! that host's own ids and limits first. Addresses here are compared, never contacted.

use crate::{Catalog, Provider};

/// Addresses for providers whose feed entry gives none (their SDK knows it instead). Kept in code
/// so an app update improves matching without a Refresh; a wrong one only ranks suggestions worse.
const HINTS: &[(&str, &str)] = &[
    ("anthropic", "https://api.anthropic.com"),
    ("openai", "https://api.openai.com/v1"),
    ("google", "https://generativelanguage.googleapis.com/v1beta"),
    ("mistral", "https://api.mistral.ai/v1"),
    ("xai", "https://api.x.ai/v1"),
    ("groq", "https://api.groq.com/openai/v1"),
    ("cerebras", "https://api.cerebras.ai/v1"),
    ("togetherai", "https://api.together.xyz/v1"),
    ("deepinfra", "https://api.deepinfra.com/v1/openai"),
    ("venice", "https://api.venice.ai/api/v1"),
    ("perplexity", "https://api.perplexity.ai"),
    ("cohere", "https://api.cohere.ai/compatibility/v1"),
    ("vercel", "https://ai-gateway.vercel.sh/v1"),
];

/// An address as its host (with any port) and path, for comparing: lowercase, without the
/// request paths WackCode adds itself, and `localhost` as `127.0.0.1`. `None` for anything that
/// isn't an http(s) address, a template like `${ACCOUNT}` included.
pub(crate) fn endpoint_key(address: &str) -> Option<(String, String)> {
    let address = address.trim();
    if address.is_empty() || address.contains("${") {
        return None;
    }
    let address = if address.contains("://") { address.to_string() } else { format!("https://{address}") };
    let url = url::Url::parse(&address).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let host = if host == "localhost" || host == "[::1]" { "127.0.0.1".to_string() } else { host };
    let host = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    let mut path = url.path().trim_end_matches('/').to_ascii_lowercase();
    for suffix in ["/chat/completions", "/responses", "/v1/messages", "/messages"] {
        if let Some(stripped) = path.strip_suffix(suffix) {
            path = stripped.to_string();
            break;
        }
    }
    Some((host, path))
}

/// How well a provider's address fits the user's: the same path, one inside the other, or only
/// the same host. `None` for another host.
fn fit(user: &(String, String), provider: &(String, String)) -> Option<u8> {
    if user.0 != provider.0 {
        return None;
    }
    let (a, b) = (&user.1, &provider.1);
    Some(if a == b {
        0
    } else if a.starts_with(b.as_str()) || b.starts_with(a.as_str()) {
        1
    } else {
        2
    })
}

impl Catalog {
    /// The providers at the user's address, best fit only (several when they share it equally).
    pub fn providers_for(&self, base_url: &str) -> Vec<&Provider> {
        let Some(user) = endpoint_key(base_url) else {
            return Vec::new();
        };
        let scored: Vec<(u8, &Provider)> = self
            .providers
            .iter()
            .filter_map(|provider| {
                let hint = HINTS.iter().find(|(id, _)| *id == provider.id).map(|(_, address)| *address);
                let best = [provider.api.as_deref(), hint]
                    .into_iter()
                    .flatten()
                    .filter_map(endpoint_key)
                    .filter_map(|key| fit(&user, &key))
                    .min()?;
                Some((best, provider))
            })
            .collect();
        let Some(best) = scored.iter().map(|(fit, _)| *fit).min() else {
            return Vec::new();
        };
        scored.into_iter().filter(|(fit, _)| *fit == best).map(|(_, provider)| provider).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str, api: Option<&str>) -> Provider {
        Provider { id: id.into(), name: id.into(), api: api.map(str::to_string), models: vec![] }
    }

    fn catalog() -> Catalog {
        Catalog {
            providers: vec![
                provider("anthropic", None),
                provider("openrouter", Some("https://openrouter.ai/api/v1")),
                provider("lmstudio", Some("http://127.0.0.1:1234/v1")),
                provider("zen", Some("https://opencode.ai/zen/v1")),
                provider("zen-go", Some("https://opencode.ai/zen/go/v1")),
                provider("templated", Some("https://${ACCOUNT}.example/v1")),
            ],
            ..Catalog::default()
        }
    }

    fn ids(catalog: &Catalog, address: &str) -> Vec<String> {
        catalog.providers_for(address).into_iter().map(|provider| provider.id.clone()).collect()
    }

    #[test]
    fn addresses_compare_without_what_the_app_adds() {
        assert_eq!(endpoint_key("https://API.openai.com/v1/"), Some(("api.openai.com".into(), "/v1".into())));
        assert_eq!(endpoint_key("http://localhost:1234/v1/chat/completions"), Some(("127.0.0.1:1234".into(), "/v1".into())));
        assert_eq!(endpoint_key("api.anthropic.com/v1/messages"), Some(("api.anthropic.com".into(), "".into())));
        assert_eq!(endpoint_key("https://${X}.example"), None);
        assert_eq!(endpoint_key("ftp://example.com"), None);
        assert_eq!(endpoint_key(""), None);
    }

    #[test]
    fn a_hint_matches_a_provider_with_no_address_of_its_own() {
        let catalog = catalog();
        assert_eq!(ids(&catalog, "https://api.anthropic.com"), ["anthropic"]);
        assert_eq!(ids(&catalog, "https://api.anthropic.com/v1"), ["anthropic"]);
        assert_eq!(ids(&catalog, "https://openrouter.ai/api/v1"), ["openrouter"]);
        assert_eq!(ids(&catalog, "http://localhost:1234/v1"), ["lmstudio"]);
        assert!(ids(&catalog, "http://localhost:11434/v1").is_empty(), "another port is another server");
        assert!(ids(&catalog, "https://my-proxy.example/v1").is_empty());
    }

    #[test]
    fn the_closest_path_wins_on_a_shared_host() {
        let catalog = catalog();
        assert_eq!(ids(&catalog, "https://opencode.ai/zen/go/v1"), ["zen-go"]);
        assert_eq!(ids(&catalog, "https://opencode.ai/zen/v1"), ["zen"]);
        assert_eq!(ids(&catalog, "https://opencode.ai/zen"), ["zen", "zen-go"], "both hold the path; neither is closer");
    }
}
