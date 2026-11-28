//! Settings › Prompts: the built-in system prompts across WackCode, and the user's overrides.
//!
//! Each prompt can be read as shipped or replaced with up to [`MAX_PROMPT_CHARS`] characters of
//! the user's own text, saved in `state.json` under `prompts`. An absent override (`None`) leaves
//! the shipped default in place, and saving text identical to the default clears the override so a
//! prompt never reads as customised when it matches WackCode's word for word.
//!
//! Dynamic guides and data wrappers stay app-owned around every custom prompt: `# This chat`'s
//! tool list and untrusted-content rule, the `[WACKCODE PLAN MODE CONTRACT v1: …]` header lines,
//! Team's `[WACKCODE TEAM MODE v1]` line and the models it names, compaction's `<conversation>` and `<previous-summary>` framing, automatic titles'
//! `<message>` data tag, and step summaries' `<intent>` and `<steps>` tags.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use wack_agent::{
    PromptOverrides,
    agents::{prompt::CHILD_PROMPT, team::TEAM_PROMPT},
    compaction::SUMMARY_FORMAT,
    plan::prompt::{default_plan_prompt, default_ultra_plan_prompt},
    prompt::{CHAT_PROMPT, CODE_PROMPT, ULTRA_PROMPT},
};

use crate::{step_summary, store::Store, title};

/// Maximum length of a user-customised prompt, in Unicode scalar values (`chars().count()`).
pub const MAX_PROMPT_CHARS: usize = 20_000;

/// Which of WackCode's built-in prompts is being read or customised.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptId {
    Code,
    Chat,
    Plan,
    UltraPlan,
    Ultra,
    Compaction,
    Subagent,
    Team,
    Title,
    StepSummary,
}

/// How Settings › Prompts groups the prompts into cards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptGroup {
    Chats,
    Planning,
    Background,
}

impl PromptGroup {
    pub const ALL: [PromptGroup; 3] = [PromptGroup::Chats, PromptGroup::Planning, PromptGroup::Background];

    pub fn title(self) -> &'static str {
        match self {
            PromptGroup::Chats => "Chats",
            PromptGroup::Planning => "Planning & reasoning",
            PromptGroup::Background => "Background & helpers",
        }
    }

    pub fn prompts(self) -> &'static [PromptId] {
        match self {
            PromptGroup::Chats => &[PromptId::Code, PromptId::Chat],
            PromptGroup::Planning => &[PromptId::Plan, PromptId::UltraPlan, PromptId::Ultra],
            PromptGroup::Background => &[PromptId::Compaction, PromptId::Subagent, PromptId::Team, PromptId::Title, PromptId::StepSummary],
        }
    }
}

impl PromptId {
    pub const ALL: [PromptId; 10] = [
        PromptId::Code,
        PromptId::Chat,
        PromptId::Plan,
        PromptId::UltraPlan,
        PromptId::Ultra,
        PromptId::Compaction,
        PromptId::Subagent,
        PromptId::Team,
        PromptId::Title,
        PromptId::StepSummary,
    ];

    /// The stable CLI and automation name.
    pub fn slug(self) -> &'static str {
        match self {
            PromptId::Code => "code",
            PromptId::Chat => "chat",
            PromptId::Plan => "plan",
            PromptId::UltraPlan => "ultra-plan",
            PromptId::Ultra => "ultra",
            PromptId::Compaction => "compaction",
            PromptId::Subagent => "subagent",
            PromptId::Team => "team",
            PromptId::Title => "title",
            PromptId::StepSummary => "step-summary",
        }
    }

    /// Parses a prompt name from `wack prompts <name>`, accepting plain aliases.
    pub fn from_slug(text: &str) -> Option<Self> {
        Some(match text.trim().to_ascii_lowercase().as_str() {
            "code" | "coding" | "default" | "system" => PromptId::Code,
            "chat" | "chat-mode" => PromptId::Chat,
            "plan" | "plan-mode" => PromptId::Plan,
            "ultra-plan" | "ultraplan" => PromptId::UltraPlan,
            "ultra" => PromptId::Ultra,
            "compaction" | "compact" | "summary" => PromptId::Compaction,
            "subagent" | "sub-agent" | "subagents" | "sub-agents" | "child" => PromptId::Subagent,
            "team" | "lead" | "crew" | "orchestrator" => PromptId::Team,
            "title" | "titles" | "auto-title" => PromptId::Title,
            "step-summary" | "step-summaries" | "steps" => PromptId::StepSummary,
            _ => return None,
        })
    }

    pub fn title(self) -> &'static str {
        match self {
            PromptId::Code => "Default coding prompt",
            PromptId::Chat => "Chat mode",
            PromptId::Plan => "Plan mode",
            PromptId::UltraPlan => "Ultra Plan",
            PromptId::Ultra => "Ultra",
            PromptId::Compaction => "Compaction",
            PromptId::Subagent => "Sub-agent core",
            PromptId::Team => "Team lead",
            PromptId::Title => "Automatic titles",
            PromptId::StepSummary => "Step summaries",
        }
    }

    pub fn group(self) -> PromptGroup {
        match self {
            PromptId::Code | PromptId::Chat => PromptGroup::Chats,
            PromptId::Plan | PromptId::UltraPlan | PromptId::Ultra => PromptGroup::Planning,
            PromptId::Compaction | PromptId::Subagent | PromptId::Team | PromptId::Title | PromptId::StepSummary => PromptGroup::Background,
        }
    }

    /// Whether this prompt belongs to Ultra's fixed prism look rather than the theme's accent.
    pub fn is_ultra(self) -> bool {
        matches!(self, PromptId::UltraPlan | PromptId::Ultra)
    }

    pub fn description(self) -> &'static str {
        match self {
            PromptId::Code => "The core instructions every coding chat opens with: how to work, act with care, verify changes and communicate. Project instructions, memory and skills follow it.",
            PromptId::Chat => "The persona every chat in the Chat area opens with, replacing the coding prompt entirely.",
            PromptId::Plan => "The contract steering a coding chat while Plan mode is on: explore read-only, ask, and submit a decision-ready plan.",
            PromptId::UltraPlan => "The contract steering a coding chat while Ultra Plan is on: interview one question at a time until every branch is settled.",
            PromptId::Ultra => "Added to the end of a chat’s system prompt while its reasoning effort is set to Ultra, in both Code and Chat.",
            PromptId::Compaction => "The summary headings and rules sent when an older part of a conversation is compacted.",
            PromptId::Subagent => "The shared core instructions every fresh sub-agent starts with before its own role’s prompt.",
            PromptId::Team => "How a chat’s model leads while its Team is on: when to delegate, how to brief and review the crew, and how to finish.",
            PromptId::Title => "The system prompt that names a new chat in 3 to 7 words from its opening message.",
            PromptId::StepSummary => "The system prompt that sums up a finished group of the agent’s steps in one short line for the transcript.",
        }
    }

    /// Explains what WackCode still adds around this prompt when customised.
    pub fn wrapper_note(self) -> &'static str {
        match self {
            PromptId::Code => "WackCode adds # This chat after this text from the tools switched on: today’s date, Git status, commit attribution (when Settings › Git is on), the untrusted-content rule, and the tool list, followed by AGENTS.md / CLAUDE.md, memory and skills.",
            PromptId::Chat => "WackCode adds a short # This chat guide after this text from the tools switched on: today’s date, how the private scratchpad works, and the untrusted-content rule.",
            PromptId::Plan => "WackCode adds one [WACKCODE PLAN MODE CONTRACT v1: PLAN] marker line above this text to recognise the contract. Active planning chats pick up edits from their next message.",
            PromptId::UltraPlan => "WackCode adds one [WACKCODE PLAN MODE CONTRACT v1: ULTRAPLAN] marker line above this text to recognise the contract. Active planning chats pick up edits from their next message.",
            PromptId::Ultra => "Ultra also asks the provider for the model’s highest reasoning level; this text is the extra instruction appended at the end of the system prompt.",
            PromptId::Compaction => "WackCode keeps the one-line summariser system role, wraps the transcript in <conversation> and any earlier summary in <previous-summary>, and carries the user’s quoted messages and read/changed file lists in code.",
            PromptId::Subagent => "Prepended before # Your role and the role’s own instructions (Settings › Sub-agents) on fresh sub-agents. Sub-agents started from a copy of the conversation keep their parent’s prompt so the prompt cache holds.",
            PromptId::Team => "WackCode adds one [WACKCODE TEAM MODE v1] marker line above this text, then a line naming the lead’s and the crew’s models. Chats with Team on pick up edits from their next message; the system prompt never changes.",
            PromptId::Title => "WackCode wraps the first 8,000 characters of the opening message inside <message>…</message> as source data and strips quotes, markdown and emoji from the answer.",
            PromptId::StepSummary => "WackCode wraps what the agent meant to do in <intent>…</intent> and its steps, cut to 6,000 characters, in <steps>…</steps> as source data, and refuses answers longer than one short line.",
        }
    }

    /// WackCode's shipped built-in text for this prompt.
    pub fn default_text(self) -> Cow<'static, str> {
        match self {
            PromptId::Code => Cow::Borrowed(CODE_PROMPT),
            PromptId::Chat => Cow::Borrowed(CHAT_PROMPT),
            PromptId::Plan => Cow::Owned(default_plan_prompt()),
            PromptId::UltraPlan => Cow::Owned(default_ultra_plan_prompt()),
            PromptId::Ultra => Cow::Borrowed(ULTRA_PROMPT),
            PromptId::Compaction => Cow::Borrowed(SUMMARY_FORMAT),
            PromptId::Subagent => Cow::Borrowed(CHILD_PROMPT),
            PromptId::Team => Cow::Borrowed(TEAM_PROMPT),
            PromptId::Title => Cow::Borrowed(title::SYSTEM),
            PromptId::StepSummary => Cow::Borrowed(step_summary::SYSTEM),
        }
    }
}

/// User-customised built-in prompts (`v3/state.json`). Every field is `None` while the shipped
/// default is in force.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ultra_plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ultra: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_summary: Option<String>,
}

impl PromptConfig {
    pub fn get(&self, id: PromptId) -> Option<&str> {
        match id {
            PromptId::Code => self.code.as_deref(),
            PromptId::Chat => self.chat.as_deref(),
            PromptId::Plan => self.plan.as_deref(),
            PromptId::UltraPlan => self.ultra_plan.as_deref(),
            PromptId::Ultra => self.ultra.as_deref(),
            PromptId::Compaction => self.compaction.as_deref(),
            PromptId::Subagent => self.subagent.as_deref(),
            PromptId::Team => self.team.as_deref(),
            PromptId::Title => self.title.as_deref(),
            PromptId::StepSummary => self.step_summary.as_deref(),
        }
    }

    fn slot_mut(&mut self, id: PromptId) -> &mut Option<String> {
        match id {
            PromptId::Code => &mut self.code,
            PromptId::Chat => &mut self.chat,
            PromptId::Plan => &mut self.plan,
            PromptId::UltraPlan => &mut self.ultra_plan,
            PromptId::Ultra => &mut self.ultra,
            PromptId::Compaction => &mut self.compaction,
            PromptId::Subagent => &mut self.subagent,
            PromptId::Team => &mut self.team,
            PromptId::Title => &mut self.title,
            PromptId::StepSummary => &mut self.step_summary,
        }
    }

    /// The text in force for `id`: the user's override, or WackCode's built-in text.
    pub fn effective(&self, id: PromptId) -> Cow<'_, str> {
        match self.get(id) {
            Some(custom) => Cow::Borrowed(custom),
            None => id.default_text(),
        }
    }

    pub fn is_customised(&self, id: PromptId) -> bool {
        self.get(id).is_some()
    }

    pub fn customised_count(&self) -> usize {
        PromptId::ALL.into_iter().filter(|&id| self.is_customised(id)).count()
    }

    pub fn is_empty(&self) -> bool {
        self.customised_count() == 0
    }

    /// The overrides `wack-agent` reads during a run.
    pub fn agent_overrides(&self) -> PromptOverrides {
        PromptOverrides {
            code: self.code.clone(),
            chat: self.chat.clone(),
            plan: self.plan.clone(),
            ultra_plan: self.ultra_plan.clone(),
            ultra: self.ultra.clone(),
            compaction: self.compaction.clone(),
            subagent: self.subagent.clone(),
            team: self.team.clone(),
        }
    }
}

/// Validates a draft override for `id`: trims outer whitespace, refuses blank text or text past
/// [`MAX_PROMPT_CHARS`], and folds text identical to the built-in default back to `None`.
pub fn normalize_prompt(id: PromptId, value: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Write the prompt, or restore the built-in text.".to_string());
    }
    let count = trimmed.chars().count();
    if count > MAX_PROMPT_CHARS {
        return Err(format!("Keep the {} prompt under 20,000 characters.", id.title()));
    }
    if trimmed == id.default_text().trim() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
}

impl Store {
    pub fn prompts(&self) -> &PromptConfig {
        self.prompts_state()
    }

    /// Saves or restores one prompt (`None` restores the built-in default). Applies from each
    /// chat's next message.
    pub fn set_prompt(&mut self, id: PromptId, text: Option<&str>) -> Result<(), String> {
        let normalized = normalize_prompt(id, text)?;
        let slot = self.prompts_state_mut().slot_mut(id);
        if *slot == normalized {
            return Ok(());
        }
        *slot = normalized;
        self.save_state()
    }

    /// Restores every customised prompt back to WackCode's built-in text.
    pub fn reset_all_prompts(&mut self) -> Result<(), String> {
        if self.prompts_state().is_empty() {
            return Ok(());
        }
        *self.prompts_state_mut() = PromptConfig::default();
        self.save_state()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_prompt_has_non_empty_built_in_text_and_a_round_tripping_slug() {
        let listed: Vec<PromptId> = PromptGroup::ALL.into_iter().flat_map(|group| group.prompts().iter().copied()).collect();
        assert_eq!(listed.as_slice(), &PromptId::ALL, "groups cover every prompt in order");
        for id in PromptId::ALL {
            let default = id.default_text();
            assert!(!default.trim().is_empty(), "{id:?}");
            assert!(default.chars().count() <= MAX_PROMPT_CHARS, "{id:?} exceeds MAX_PROMPT_CHARS");
            assert_eq!(PromptId::from_slug(id.slug()), Some(id));
            assert!(!id.title().is_empty() && !id.description().is_empty() && !id.wrapper_note().is_empty());
        }
        assert_eq!(PromptId::from_slug("ultraplan"), Some(PromptId::UltraPlan));
        assert_eq!(PromptId::from_slug("sub-agents"), Some(PromptId::Subagent));
        assert_eq!(PromptId::from_slug("titles"), Some(PromptId::Title));
        assert_eq!(PromptId::from_slug("unknown"), None);
    }

    #[test]
    fn built_in_texts_match_the_runtime_constants_byte_for_byte() {
        assert_eq!(PromptId::Code.default_text(), CODE_PROMPT);
        assert_eq!(PromptId::Chat.default_text(), CHAT_PROMPT);
        assert_eq!(PromptId::Plan.default_text(), default_plan_prompt());
        assert_eq!(PromptId::UltraPlan.default_text(), default_ultra_plan_prompt());
        assert_eq!(PromptId::Ultra.default_text(), ULTRA_PROMPT);
        assert_eq!(PromptId::Compaction.default_text(), SUMMARY_FORMAT);
        assert_eq!(PromptId::Subagent.default_text(), CHILD_PROMPT);
        assert_eq!(PromptId::Title.default_text(), title::SYSTEM);
        assert_eq!(PromptId::StepSummary.default_text(), step_summary::SYSTEM);
    }

    #[test]
    fn validation_trims_rejects_blank_and_overlong_and_folds_matching_defaults_to_none() {
        assert_eq!(normalize_prompt(PromptId::Chat, None).unwrap(), None);
        assert_eq!(normalize_prompt(PromptId::Chat, Some("  Be concise.  ")).unwrap().as_deref(), Some("Be concise."));
        assert!(normalize_prompt(PromptId::Chat, Some("   \n\t  ")).unwrap_err().contains("restore the built-in"));
        let built_in = format!("  {}\n", PromptId::Chat.default_text());
        assert_eq!(normalize_prompt(PromptId::Chat, Some(&built_in)).unwrap(), None, "saving the built-in text clears the override");

        let exact_max = "é".repeat(MAX_PROMPT_CHARS);
        assert_eq!(normalize_prompt(PromptId::Ultra, Some(&exact_max)).unwrap().unwrap().chars().count(), MAX_PROMPT_CHARS);
        let over_max = "é".repeat(MAX_PROMPT_CHARS + 1);
        assert!(normalize_prompt(PromptId::Ultra, Some(&over_max)).unwrap_err().contains("20,000 characters"));
    }

    #[test]
    fn store_persists_overrides_omits_empty_config_and_resets_all() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        assert!(store.prompts().is_empty());

        store.set_prompt(PromptId::Code, Some("Custom coding core.")).unwrap();
        store.set_prompt(PromptId::UltraPlan, Some("Custom interview rules.")).unwrap();
        assert_eq!(store.prompts().customised_count(), 2);

        let reopened = Store::open(folder.path()).unwrap();
        assert_eq!(reopened.prompts().get(PromptId::Code), Some("Custom coding core."));
        assert_eq!(reopened.prompts().get(PromptId::UltraPlan), Some("Custom interview rules."));
        assert_eq!(reopened.prompts().get(PromptId::Chat), None);
        assert_eq!(reopened.prompts().effective(PromptId::Chat), CHAT_PROMPT);

        let mut reopened = reopened;
        reopened.set_prompt(PromptId::Code, None).unwrap();
        assert_eq!(reopened.prompts().customised_count(), 1);
        reopened.reset_all_prompts().unwrap();
        assert!(reopened.prompts().is_empty());
        let raw = std::fs::read_to_string(folder.path().join("state.json")).unwrap();
        assert!(!raw.contains("\"prompts\""), "an all-default PromptConfig is left out of state.json: {raw}");
    }
}
