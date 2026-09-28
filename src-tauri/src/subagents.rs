//! Sub-agents: the settings behind the worker's `subagent` built-in, and what a worker receives.
//!
//! The shipped roles (Scout, Reviewer, Worker) are adapted from the sample agents in Pi's
//! `examples/extensions/subagent` (MIT, © Mario Zechner). Their definitions live only here and
//! are refreshed into `wackcode.json` on every load, so an app update reaches existing users;
//! the user owns just whether each one is on and which model it uses.
//!
//! The feature is off by default. While it is off a worker receives nothing at all — no roster
//! and no credentials — and the tool stays out of the model's active set.

use crate::models::{ProviderKind, ProviderRecord, SubagentConfig, SubagentModel, SubagentRecord, SubagentTrigger, SubagentWatchTarget};
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

/// The tools a sub-agent may be given: Pi's own, plus the built-in `web_fetch` (which the worker
/// withholds while the user has switched Web Fetch off). Package tools never reach a child.
pub const CHILD_TOOLS: &[&str] = &["read", "grep", "find", "ls", "bash", "edit", "write", "web_fetch"];
/// Everything a read-only agent may use; its bash is further limited by the Plan-mode policy.
const READ_ONLY_TOOLS: &[&str] = &["read", "grep", "find", "ls", "bash", "web_fetch"];
pub const MAX_CONCURRENCY: u32 = 8;
/// Children one `subagent` call may carry; mirrors `MAX_PARALLEL_TASKS` in the worker.
const MAX_PARALLEL_TASKS: u32 = 8;
/// Longer than any tool call id a provider mints.
const MAX_TOOL_CALL_ID_CHARS: usize = 256;
const MAX_AGENTS: usize = 32;
const MAX_NAME_CHARS: usize = 32;
const MAX_DESCRIPTION_CHARS: usize = 400;
const MAX_PROMPT_CHARS: usize = 20_000;

struct BuiltinAgent {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    prompt: &'static str,
    tools: &'static [&'static str],
    read_only: bool,
}

const BUILTINS: &[BuiltinAgent] = &[
    BuiltinAgent {
        id: "builtin:scout",
        name: "scout",
        description: "Fast read-only codebase reconnaissance. Returns compressed findings (files, line ranges, key code, architecture) that another agent can act on without re-reading everything. Can also read public web pages by URL, such as a library's docs or an issue, but can't search the web.",
        prompt: include_str!("subagents/scout.md"),
        tools: READ_ONLY_TOOLS,
        read_only: true,
    },
    BuiltinAgent {
        id: "builtin:reviewer",
        name: "reviewer",
        description: "Read-only code review of the current changes or the files named in the task. Reports bugs, risks and missed edge cases with file:line references, and never edits.",
        prompt: include_str!("subagents/reviewer.md"),
        tools: READ_ONLY_TOOLS,
        read_only: true,
    },
    BuiltinAgent {
        id: "builtin:worker",
        name: "worker",
        description: "Implements a well-scoped task in its own context window, editing files as needed. Reports what changed and how it was verified.",
        prompt: include_str!("subagents/worker.md"),
        tools: CHILD_TOOLS,
        read_only: false,
    },
];

fn builtin(id: &str) -> Option<&'static BuiltinAgent> {
    BUILTINS.iter().find(|agent| agent.id == id)
}

/// Bring built-ins up to the shipped definitions, keeping the user's switch and model; keep
/// custom agents as they are, after the built-ins. Returns whether anything changed.
pub fn normalize(config: &mut SubagentConfig) -> bool {
    let before = config.clone();
    let mut agents: Vec<SubagentRecord> = BUILTINS.iter().map(|shipped| {
        let saved = config.agents.iter().find(|agent| agent.id == shipped.id);
        SubagentRecord {
            id: shipped.id.to_string(),
            builtin: true,
            enabled: saved.map_or(true, |agent| agent.enabled),
            name: shipped.name.to_string(),
            description: shipped.description.to_string(),
            prompt: shipped.prompt.trim().to_string(),
            tools: shipped.tools.iter().map(|tool| tool.to_string()).collect(),
            read_only: shipped.read_only,
            model: saved.and_then(|agent| agent.model.clone()),
        }
    }).collect();
    agents.extend(config.agents.iter().filter(|agent| builtin(&agent.id).is_none()).map(|agent| {
        SubagentRecord { builtin: false, ..agent.clone() }
    }));
    config.agents = agents;
    config.max_concurrency = config.max_concurrency.clamp(1, MAX_CONCURRENCY);
    *config != before
}

fn valid_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters.next().is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-')
        && name.chars().count() <= MAX_NAME_CHARS
}

/// Why `choice` can't be used on `providers`, if it can't. Connectivity is checked separately:
/// a signed-out subscription is still a valid choice to save.
fn model_problem(agent: &str, choice: &SubagentModel, providers: &[ProviderRecord]) -> Option<String> {
    let Some(provider) = providers.iter().find(|provider| provider.id == choice.provider_id) else {
        return Some(format!("{agent}'s connection no longer exists. Pick another model for it in Settings → Sub-agents."));
    };
    let Some(model) = provider.models.iter().find(|model| model.id == choice.model_id) else {
        return Some(format!("{agent}'s model ({} / {}) is no longer configured. Pick another model for it in Settings → Sub-agents.", provider.name, choice.model_id));
    };
    if model.context_window.is_none() || model.max_tokens.is_none() {
        return Some(format!("Confirm the context and output limits of {} in Settings before {agent} can use it.", model.name));
    }
    if !model.thinking_levels.iter().any(|level| *level == choice.thinking_level) {
        return Some(format!("{} doesn't support {} reasoning. Pick another level for {agent}.", model.name, choice.thinking_level));
    }
    None
}

/// Check a whole configuration from Settings. Built-in definitions sent by the client are
/// ignored — only their switch and model are taken — and custom agents are cleaned up.
pub fn validate(input: &SubagentConfig, providers: &[ProviderRecord]) -> Result<SubagentConfig, String> {
    if input.max_concurrency < 1 || input.max_concurrency > MAX_CONCURRENCY {
        return Err(format!("Sub-agents can run between 1 and {MAX_CONCURRENCY} at once."));
    }
    if input.agents.len() > MAX_AGENTS {
        return Err(format!("You can have at most {MAX_AGENTS} sub-agents."));
    }
    let mut agents = Vec::with_capacity(input.agents.len());
    for agent in &input.agents {
        if builtin(&agent.id).is_some() {
            agents.push(agent.clone());
            continue;
        }
        let name = agent.name.trim().to_string();
        if !valid_name(&name) {
            return Err(format!("\"{name}\" can't be a sub-agent name. Use up to {MAX_NAME_CHARS} lowercase letters, digits and hyphens, starting with a letter."));
        }
        let description = agent.description.trim().to_string();
        if description.is_empty() {
            return Err(format!("Describe what {name} is for, so the agent knows when to use it."));
        }
        if description.chars().count() > MAX_DESCRIPTION_CHARS {
            return Err(format!("Keep {name}'s description under {MAX_DESCRIPTION_CHARS} characters."));
        }
        let prompt = agent.prompt.trim().to_string();
        if prompt.is_empty() {
            return Err(format!("Give {name} instructions."));
        }
        if prompt.chars().count() > MAX_PROMPT_CHARS {
            return Err(format!("Keep {name}'s instructions under {MAX_PROMPT_CHARS} characters."));
        }
        let mut tools: Vec<String> = Vec::new();
        for tool in &agent.tools {
            if !CHILD_TOOLS.contains(&tool.as_str()) {
                return Err(format!("{tool} isn't a tool sub-agents can use."));
            }
            if agent.read_only && !READ_ONLY_TOOLS.contains(&tool.as_str()) {
                return Err(format!("{name} is read-only, so it can't have {tool}. Allow it to edit files first."));
            }
            if !tools.contains(tool) { tools.push(tool.clone()); }
        }
        tools.sort_by_key(|tool| CHILD_TOOLS.iter().position(|known| known == tool));
        let id = if agent.id.trim().is_empty() { Uuid::new_v4().to_string() } else { agent.id.trim().to_string() };
        agents.push(SubagentRecord {
            id, builtin: false, enabled: agent.enabled, name, description, prompt, tools,
            read_only: agent.read_only, model: agent.model.clone(),
        });
    }

    let mut config = SubagentConfig {
        enabled: input.enabled,
        trigger: input.trigger,
        max_concurrency: input.max_concurrency,
        agents,
    };
    normalize(&mut config);

    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for agent in &config.agents {
        if !ids.insert(agent.id.as_str()) {
            return Err("Two sub-agents share an id. Reload Settings and try again.".into());
        }
        if !names.insert(agent.name.as_str()) {
            return Err(format!("There is already a sub-agent called {}. Choose another name.", agent.name));
        }
        if let Some(choice) = &agent.model {
            if let Some(problem) = model_problem(&agent.name, choice, providers) { return Err(problem); }
        }
    }
    Ok(config)
}

/// Enabled agents whose own model lives on `provider_id`, for refusing to delete it.
pub fn agents_using(config: &SubagentConfig, provider_id: &str) -> Vec<String> {
    config.agents.iter()
        .filter(|agent| agent.model.as_ref().is_some_and(|model| model.provider_id == provider_id))
        .map(|agent| agent.name.clone())
        .collect()
}

/// How a worker reaches one connection: an API key or a Pi auth file. `None` when signed out.
pub struct ProviderCredential {
    pub api_key: Option<String>,
    pub auth_path: Option<String>,
}

/// The worker's `subagents` value: `null` while the feature is off. Otherwise the enabled
/// agents, and every connection their own models use with its credential — sent only over the
/// worker's stdin. A worker whose chat already uses one of them reuses its own runtime.
pub fn runtime_payload(
    config: &SubagentConfig,
    providers: &[ProviderRecord],
    credential: impl Fn(&ProviderRecord) -> Option<ProviderCredential>,
) -> Value {
    if !config.enabled { return Value::Null; }
    let mut names = HashSet::new();
    let mut agents = Vec::new();
    let mut connections: Vec<Value> = Vec::new();
    let mut seen_connections = HashSet::new();
    for agent in config.agents.iter().filter(|agent| agent.enabled) {
        if !names.insert(agent.name.clone()) { continue; }
        let mut entry = json!({
            "name": agent.name,
            "description": agent.description,
            "prompt": agent.prompt,
            "tools": agent.tools,
            "readOnly": agent.read_only,
        });
        if let Some(choice) = &agent.model {
            entry["model"] = json!({
                "providerId": choice.provider_id,
                "modelId": choice.model_id,
                "thinkingLevel": choice.thinking_level,
            });
            let provider = providers.iter().find(|provider| provider.id == choice.provider_id);
            let unavailable = model_problem(&agent.name, choice, providers).or_else(|| {
                let provider = provider?;
                if !provider.enabled {
                    return Some(format!("{}'s model is on {}, which is turned off. Enable it in Settings → Providers, or pick another model in Settings → Sub-agents.", agent.name, provider.name));
                }
                if seen_connections.contains(&provider.id) { return None; }
                match credential(provider) {
                    Some(found) => {
                        seen_connections.insert(provider.id.clone());
                        connections.push(json!({
                            "provider": crate::worker::worker_provider_json(provider),
                            "apiKey": found.api_key,
                            "authPath": found.auth_path,
                        }));
                        None
                    }
                    None => Some(if provider.kind == ProviderKind::Subscription {
                        format!("{}'s model is on {}, which is signed out. Sign in again, or pick another model in Settings → Sub-agents.", agent.name, provider.name)
                    } else {
                        format!("{}'s model is on {}, which has no API key. Add one, or pick another model in Settings → Sub-agents.", agent.name, provider.name)
                    }),
                }
            });
            if let Some(reason) = unavailable { entry["unavailable"] = json!(reason); }
        }
        agents.push(entry);
    }
    json!({
        "trigger": match config.trigger { SubagentTrigger::OnRequest => "on_request", SubagentTrigger::Auto => "auto" },
        "maxConcurrency": config.max_concurrency.clamp(1, MAX_CONCURRENCY),
        "agents": agents,
        "providers": connections,
    })
}

/// A side-panel watch can only name a child a `subagent` call could have: a tool call id as a
/// provider mints one, and a position within one call's limit.
pub fn validate_watch_target(target: &SubagentWatchTarget) -> Result<(), String> {
    let id = &target.tool_call_id;
    if id.is_empty() || id.len() > MAX_TOOL_CALL_ID_CHARS || id.chars().any(char::is_control) || target.index >= MAX_PARALLEL_TASKS {
        return Err("That sub-agent is not in this chat.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ModelRecord;
    use std::collections::BTreeMap;

    fn provider(id: &str, kind: ProviderKind) -> ProviderRecord {
        ProviderRecord {
            id: id.into(), name: format!("Provider {id}"), kind, base_url: "https://example.test/v1".into(),
            api_format: "openai-completions".into(),
            models: vec![ModelRecord {
                id: "small".into(), name: "Small".into(), context_window: Some(128_000), max_tokens: Some(8_192),
                reasoning: true, thinking_levels: vec!["off".into(), "low".into()], thinking_level_map: BTreeMap::new(), vision: false,
            }],
            created_at: "now".into(), updated_at: "now".into(), has_api_key: true, connected: true,
            enabled: true,
        }
    }

    fn custom(name: &str) -> SubagentRecord {
        SubagentRecord {
            id: String::new(), builtin: false, enabled: true, name: name.into(),
            description: "Checks docs".into(), prompt: "Read the docs.".into(),
            tools: vec!["read".into(), "grep".into()], read_only: true, model: None,
        }
    }

    fn choice(provider_id: &str, level: &str) -> Option<SubagentModel> {
        Some(SubagentModel { provider_id: provider_id.into(), model_id: "small".into(), thinking_level: level.into() })
    }

    #[test]
    fn normalize_adds_and_refreshes_builtins_but_keeps_the_users_choices() {
        let mut config = SubagentConfig::default();
        assert!(normalize(&mut config));
        assert_eq!(config.agents.iter().map(|agent| agent.name.as_str()).collect::<Vec<_>>(), ["scout", "reviewer", "worker"]);
        assert!(config.agents.iter().all(|agent| agent.enabled && agent.builtin));
        assert!(!normalize(&mut config), "normalizing twice changes nothing");

        config.agents[0].enabled = false;
        config.agents[0].model = choice("p", "low");
        config.agents[0].prompt = "tampered".into();
        config.agents[0].tools = vec!["write".into()];
        config.agents.push(custom("docs"));
        normalize(&mut config);
        let scout = &config.agents[0];
        assert!(!scout.enabled);
        assert_eq!(scout.model, choice("p", "low"));
        assert!(scout.prompt.starts_with("You are Scout"));
        assert!(!scout.tools.contains(&"write".to_string()));
        assert_eq!(config.agents.last().unwrap().name, "docs");
    }

    #[test]
    fn every_builtin_agent_can_read_the_web() {
        let mut config = SubagentConfig::default();
        normalize(&mut config);
        for agent in &config.agents {
            assert!(agent.tools.contains(&"web_fetch".to_string()), "{} lacks web_fetch", agent.name);
        }
        assert!(config.agents[0].description.contains("web pages"));
    }

    #[test]
    fn old_metadata_without_subagents_loads_switched_off() {
        let data: crate::models::AppData = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert!(!data.subagents.enabled);
        assert_eq!(data.subagents.max_concurrency, 4);
        assert_eq!(data.subagents.trigger, SubagentTrigger::OnRequest);
    }

    #[test]
    fn validate_cleans_custom_agents_and_ignores_client_builtin_definitions() {
        let mut input = SubagentConfig { enabled: true, ..SubagentConfig::default() };
        normalize(&mut input);
        input.agents[2].prompt = "do anything".into();
        let mut docs = custom("  docs  ");
        docs.tools = vec!["grep".into(), "read".into(), "grep".into()];
        input.agents.push(docs);
        let config = validate(&input, &[]).unwrap();
        assert!(config.agents[2].prompt.starts_with("You are Worker"));
        let docs = config.agents.last().unwrap();
        assert_eq!(docs.name, "docs");
        assert!(!docs.id.is_empty());
        assert_eq!(docs.tools, ["read", "grep"]);

        // A read-only custom agent may read the web; its tools keep CHILD_TOOLS order.
        let mut input = SubagentConfig { enabled: true, ..SubagentConfig::default() };
        normalize(&mut input);
        let mut researcher = custom("researcher");
        researcher.tools = vec!["web_fetch".into(), "read".into()];
        input.agents.push(researcher);
        let config = validate(&input, &[]).unwrap();
        assert_eq!(config.agents.last().unwrap().tools, ["read", "web_fetch"]);
    }

    #[test]
    fn validate_refuses_bad_agents() {
        let base = || { let mut config = SubagentConfig::default(); normalize(&mut config); config };
        let refuse = |agent: SubagentRecord| {
            let mut config = base();
            config.agents.push(agent);
            validate(&config, &[provider("p", ProviderKind::Custom)]).unwrap_err()
        };
        assert!(refuse(custom("Docs")).contains("can't be a sub-agent name"));
        assert!(refuse(custom("scout")).contains("already a sub-agent called scout"));
        assert!(refuse(SubagentRecord { description: " ".into(), ..custom("docs") }).contains("Describe"));
        assert!(refuse(SubagentRecord { prompt: String::new(), ..custom("docs") }).contains("instructions"));
        assert!(refuse(SubagentRecord { tools: vec!["powershell".into()], ..custom("docs") }).contains("isn't a tool"));
        assert!(refuse(SubagentRecord { tools: vec!["edit".into()], ..custom("docs") }).contains("read-only"));
        assert!(refuse(SubagentRecord { model: choice("missing", "low"), ..custom("docs") }).contains("no longer exists"));
        assert!(refuse(SubagentRecord { model: choice("p", "max"), ..custom("docs") }).contains("doesn't support max"));
        let mut config = base();
        config.max_concurrency = 9;
        assert!(validate(&config, &[]).is_err());
        assert!(validate(&SubagentConfig { max_concurrency: 0, ..base() }, &[]).is_err());
    }

    #[test]
    fn a_disabled_feature_sends_nothing() {
        let mut config = SubagentConfig::default();
        normalize(&mut config);
        config.agents[0].model = choice("p", "low");
        let payload = runtime_payload(&config, &[provider("p", ProviderKind::Custom)], |_| panic!("no credential may be read"));
        assert!(payload.is_null());
    }

    #[test]
    fn the_payload_carries_enabled_agents_and_only_the_connections_they_use() {
        let mut config = SubagentConfig { enabled: true, trigger: SubagentTrigger::Auto, ..SubagentConfig::default() };
        normalize(&mut config);
        config.agents[0].model = choice("p", "low");
        config.agents[1].enabled = false;
        config.agents[2].model = choice("signed-out", "off");
        let providers = [provider("p", ProviderKind::Custom), provider("unused", ProviderKind::Custom), provider("signed-out", ProviderKind::Subscription)];
        let payload = runtime_payload(&config, &providers, |provider| match provider.id.as_str() {
            "p" => Some(ProviderCredential { api_key: Some("sk-test-key".into()), auth_path: None }),
            _ => None,
        });
        assert_eq!(payload["trigger"], "auto");
        let names: Vec<&str> = payload["agents"].as_array().unwrap().iter().map(|agent| agent["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["scout", "worker"]);
        assert_eq!(payload["providers"].as_array().unwrap().len(), 1);
        assert_eq!(payload["providers"][0]["provider"]["id"], "p");
        assert_eq!(payload["providers"][0]["apiKey"], "sk-test-key");
        assert!(payload["agents"][0].get("unavailable").is_none());
        assert!(payload["agents"][1]["unavailable"].as_str().unwrap().contains("signed out"));
    }

    #[test]
    fn a_turned_off_connection_marks_its_agent_unavailable_and_shares_no_credential() {
        let mut config = SubagentConfig { enabled: true, trigger: SubagentTrigger::Auto, ..SubagentConfig::default() };
        normalize(&mut config);
        config.agents[0].model = choice("p", "low");
        let mut off = provider("p", ProviderKind::Custom);
        off.enabled = false;
        let payload = runtime_payload(&config, &[off], |_| panic!("a turned-off connection must not yield a credential"));
        assert!(payload["providers"].as_array().unwrap().is_empty());
        assert!(payload["agents"][0]["unavailable"].as_str().unwrap().contains("turned off"));
    }

    #[test]
    fn deleting_a_connection_finds_the_agents_that_use_it() {
        let mut config = SubagentConfig::default();
        normalize(&mut config);
        config.agents[1].model = choice("p", "low");
        assert_eq!(agents_using(&config, "p"), ["reviewer"]);
        assert!(agents_using(&config, "other").is_empty());
    }

    #[test]
    fn a_watch_names_only_a_child_a_call_could_have() {
        let target = |id: &str, index: u32| SubagentWatchTarget { tool_call_id: id.into(), index };
        assert!(validate_watch_target(&target("call_abc123", 0)).is_ok());
        assert!(validate_watch_target(&target("call_x|fc_y", 7)).is_ok());
        assert!(validate_watch_target(&target("call_abc123", 8)).is_err());
        assert!(validate_watch_target(&target("", 0)).is_err());
        assert!(validate_watch_target(&target("line\nbreak", 0)).is_err());
        assert!(validate_watch_target(&target(&"x".repeat(257), 0)).is_err());
    }
}
