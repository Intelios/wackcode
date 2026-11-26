//! Team: the chat's model leads, and a crew model beside it does most of the reading and typing.
//!
//! - **On is a crew.** The engine resolves the chat's crew into [`super::AgentsSetup::crew`];
//!   while it is set, roles run on the tier their file names for Team (`team: lead|crew`, crew by
//!   default), and the `agent` call's `model` overrides it.
//! - **The contract is a note**, tagged `team:on` or `team:off`, never part of the system prompt:
//!   the instructions and tool list are byte-identical with Team on or off, so switching never
//!   breaks the prompt cache. It is sent whenever the latest team note the model can still see
//!   differs from what is in force (a new crew, an edited prompt, a compaction that dropped it),
//!   and `team:off` only while the model can still see `team:on`. The tag must not start with
//!   `contract:`, which is Plan's: Plan re-sends its own contract whenever the newest
//!   `contract:` note isn't its own.
//! - **Workers are briefed.** A fresh child of a Team chat starts with [`briefing`]: the user's
//!   request, the plan or todo list, who else is working on which files, and its own files. It is
//!   the first message, never the child's instructions, so siblings on the crew share a cached
//!   prefix. Its words are the app's; the user's request inside it is quoted, not obeyed blindly.

use crate::{
    agent::Feature,
    session::{Note, Session},
};


/// The first line of every Team note: app-owned, whatever the user's prompt says.
pub const TEAM_MARKER: &str = "[WACKCODE TEAM MODE v1]";
const NOTE_PREFIX: &str = "team:";
const ON_TAG: &str = "team:on";
const OFF_TAG: &str = "team:off";

/// The longest piece of the user's request a briefing quotes.
const MAX_REQUEST_CHARS: usize = 2_000;
/// The longest piece of an approved plan a briefing quotes.
const MAX_PLAN_CHARS: usize = 6_000;

/// The Team contract's body, which Settings › Prompts can replace.
pub const TEAM_PROMPT: &str = "The user turned Team on: they want you to plan, delegate and review, with a crew doing most of the reading and typing. You're judged on the result, not on how much you hand over.

# How to lead
1. Size it first. A question, a change with one concern, a fix in a file or two: do it yourself. Briefing and reviewing a crew costs more than it saves there. Team pays off for reading across much of the codebase, or for work that splits into separable pieces.
2. Understand before you split. Send `explore` to read widely. Read the few files that decide the design yourself: you can't write a good brief for code you haven't seen. Never hand over understanding (\"based on what you find, fix it\").
3. Decide the shape yourself. Keep the plan in the todo list. Settle interfaces, shared types and names before anyone builds on them: write that core yourself, or give it to a `general` with `model: \"lead\"` when it is hard.
4. Brief like a spec. Each brief gives the goal and why it matters, what you already know or ruled out, the `files` it owns, the patterns and constraints to follow, how to check the work, and the report you want back. Workers also get a short briefing from the app (the user's request, your todo list, who owns which files), but they never see this conversation: everything else goes in the brief.
5. Split by file. Agents working at the same time get separate files, and none of yours. Start independent pieces together in one reply; dependent pieces one after another.
6. Review every change. Hand `review` the files or the diff and what the change is for, never your own verdict. For a small change, read the diff yourself. A report is the worker's claim; its \"Files it changed\" section is what the app saw.
7. Send work back with `agent_message`, saying exactly what to change and why. After two rounds on the same problem, fix it yourself or re-scope it.
8. Finish yourself. Run the project's build and tests over the whole change, then tell the user what changed, who did which part (a line), what you verified, and anything left.

While a Plan mode contract is in force it wins: use the crew to explore and review, not to change files.";

const OFF_TEXT: &str = "Team is off: the user switched it off. Work as you would without it. The rules for sub-agents in your instructions apply as written, and the agents you start run on the models their roles name.";

/// Whether Team is on, and with which models; the note it sends follows.
#[derive(Clone, Default)]
pub struct TeamFeature {
    /// The lead's model and the crew's, while Team is on.
    on: Option<(String, String)>,
    /// Settings › Prompts' replacement for [`TEAM_PROMPT`].
    prompt: Option<String>,
}

impl TeamFeature {
    /// Team off.
    pub fn off() -> Self {
        Self::default()
    }

    /// Team on: the chat's model, `lead`, leads a crew on `crew`.
    pub fn on(mut self, lead: &str, crew: &str) -> Self {
        self.on = Some((lead.to_string(), crew.to_string()));
        self
    }

    pub fn with_prompt(mut self, prompt: Option<String>) -> Self {
        self.prompt = prompt;
        self
    }

    pub fn is_on(&self) -> bool {
        self.on.is_some()
    }

    /// The contract the model is told while Team is on.
    pub fn contract(&self) -> Option<String> {
        let (lead, crew) = self.on.as_ref()?;
        let body = self.prompt.as_deref().unwrap_or(TEAM_PROMPT);
        Some(format!(
            "{TEAM_MARKER}\nTeam is on. You lead, on {lead}; your crew runs on {crew}. Each role runs on the crew or on your model as its file says (review runs on yours), and the `agent` call's `model` overrides it.\n\n{body}"
        ))
    }
}

impl Feature for TeamFeature {
    fn notes(&self, session: &Session) -> Vec<Note> {
        let latest = session.latest_note_with_text(NOTE_PREFIX);
        let (tag, text) = match self.contract() {
            Some(contract) => (ON_TAG, contract),
            // Off needs saying only while the model can still see that it was on.
            None if latest.is_some_and(|(tag, _)| tag == ON_TAG) => (OFF_TAG, format!("{TEAM_MARKER}\n{OFF_TEXT}")),
            None => return Vec::new(),
        };
        match latest {
            Some((seen, said)) if seen == tag && said == text => Vec::new(),
            _ => vec![Note { tag: tag.to_string(), text }],
        }
    }
}

/// What a fresh worker on a Team is told before its brief.
pub(crate) struct Briefing<'a> {
    /// The user's latest message to the lead.
    pub request: Option<&'a str>,
    /// The plan the user approved for this run, if it is one.
    pub plan: Option<String>,
    /// The lead's todo list, a line a task.
    pub todo: Vec<String>,
    /// The other agents working now, or starting beside it.
    pub siblings: Vec<Sibling>,
    /// The files this worker owns.
    pub files: &'a [String],
}

/// Another agent at work on the team, as a briefing names it.
pub(crate) struct Sibling {
    /// Who it is: its id, role and task, or that it starts in the same reply.
    pub who: String,
    pub files: Vec<String>,
    /// The lead's call that started it, so one is never named twice.
    pub call_id: String,
}

/// The worker's first message: the app's briefing on the team, then the lead's brief.
pub(crate) fn briefing(team: Briefing<'_>, brief: &str) -> String {
    let mut parts = vec![
        "You're working on a team. The lead (the agent that sent you) planned this work and will review what you change, and other agents may be working beside you. This briefing is from WackCode; your task follows it.".to_string(),
    ];
    if let Some(request) = team.request.map(str::trim).filter(|text| !text.is_empty()) {
        parts.push(format!("## What the user asked the lead\n{}", cut(request, MAX_REQUEST_CHARS)));
    }
    if let Some(plan) = team.plan.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
        parts.push(format!("## The plan the user approved\n{}", cut(plan, MAX_PLAN_CHARS)));
    }
    if !team.todo.is_empty() {
        parts.push(format!("## The lead's todo list\n{}", team.todo.join("\n")));
    }
    if !team.siblings.is_empty() {
        let lines: Vec<String> = team
            .siblings
            .iter()
            .map(|sibling| {
                let files = if sibling.files.is_empty() { String::new() } else { format!(": {}", sibling.files.join(", ")) };
                format!("- {}{files}", sibling.who)
            })
            .collect();
        parts.push(format!("## Working beside you\n{}", lines.join("\n")));
    }
    if team.files.is_empty() {
        parts.push("## Your files\nThe lead named none. Change only what your task needs, and nothing another agent above owns.".to_string());
    } else {
        parts.push(format!(
            "## Your files\n{}\nStay within them. If your task needs a change in a file another agent owns, don't make it: say so in your report.",
            team.files.join(", ")
        ));
    }
    parts.push(format!("## Your task\n{brief}"));
    parts.join("\n\n")
}

fn cut(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let kept: String = text.chars().take(limit).collect();
    format!("{kept}… [cut]")
}

/// A copy of the conversation is not the lead, though it read the contract.
pub(crate) const COPY_LINE: &str = "The Team contract earlier in this conversation is for the agent that started you, not for you: do this task yourself.";

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> (tempfile::TempDir, Session) {
        let folder = tempfile::tempdir().unwrap();
        let session = Session::create(&folder.path().join("chat.jsonl"), folder.path()).unwrap();
        (folder, session)
    }

    fn send(session: &mut Session, feature: &TeamFeature) -> Vec<Note> {
        let notes = feature.notes(session);
        for note in &notes {
            session.push_note(note.clone()).unwrap();
        }
        notes
    }

    #[test]
    fn the_contract_is_sent_once_resent_when_it_changes_and_off_follows_on() {
        let (_folder, mut session) = session();
        assert!(send(&mut session, &TeamFeature::off()).is_empty(), "a chat that never had Team hears nothing");

        let on = TeamFeature::off().on("big", "small");
        let first = send(&mut session, &on);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].tag, "team:on");
        assert!(first[0].text.starts_with(TEAM_MARKER));
        assert!(first[0].text.contains("You lead, on big; your crew runs on small."));
        assert!(send(&mut session, &on).is_empty(), "the same contract isn't repeated");

        let edited = on.clone().with_prompt(Some("Lead briskly.".into()));
        let again = send(&mut session, &edited);
        assert!(again[0].text.ends_with("Lead briskly."));

        let off = send(&mut session, &TeamFeature::off());
        assert_eq!(off[0].tag, "team:off");
        assert!(send(&mut session, &TeamFeature::off()).is_empty());
        // Plan's notes are its own: nothing here starts with its prefix.
        assert!(!session.ever_noted("contract:"));
    }

    #[test]
    fn a_briefing_names_the_request_the_plan_the_siblings_and_the_workers_files() {
        let sibling = Sibling { who: "a2 (general · Add the parser)".into(), files: vec!["src/parse.rs".into()], call_id: "p".into() };
        let files = vec!["src/retry.rs".to_string()];
        let text = briefing(
            Briefing { request: Some("Honour Retry-After."), plan: None, todo: vec!["[in_progress] #1 Parse the header".into()], siblings: vec![sibling], files: &files },
            "Make the backoff read the header.",
        );
        assert!(text.contains("## What the user asked the lead\nHonour Retry-After."));
        assert!(text.contains("## The lead's todo list\n[in_progress] #1 Parse the header"));
        assert!(text.contains("- a2 (general · Add the parser): src/parse.rs"));
        assert!(text.contains("## Your files\nsrc/retry.rs\nStay within them."));
        assert!(text.ends_with("## Your task\nMake the backoff read the header."));
        assert_eq!(cut("abcdef", 3), "abc… [cut]");
    }
}
