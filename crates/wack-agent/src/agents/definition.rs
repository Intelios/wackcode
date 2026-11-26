//! What a sub-agent is: a role with a name, a description the parent model chooses it by, its
//! own prompt, the tools it may use and which model it runs on.
//!
//! - **Where from:** the roots the engine names, in order (WackCode's own `agents` folder first,
//!   then other tools' folders the user switched on), then the built-ins compiled into this crate.
//!   The first role with a name wins, so a file named `explore.md` in the user's folder replaces
//!   the built-in Explore. A project's own agent folders are never read, as with skills.
//! - **A file:** Markdown with frontmatter (`name`, `description`, `tools`, `model`, `team`,
//!   `max_turns`); the body is the role's prompt. `model` is where it runs outside Team (the
//!   chat's model or the helper); `team` is where it runs while the chat's Team is on (`lead` or
//!   `crew`, the crew when it says nothing). Only a root's top-level `.md` files are roles.
//!   A file without a `name` takes its file name's.
//! - **No edits:** a role that may use neither `write` nor `edit` changes nothing in the project
//!   through its tools. Its shell is deliberately left unrestricted outside Plan mode: helpers that
//!   could not grep, build or run tests stalled workflows in versions 1 and 2. Plan mode lets only
//!   these roles run.
//! - **Other tools' files:** their tool names are mapped to ours where they have one (`Read`,
//!   `Glob`, `WebFetch`…); a name we don't know matches nothing. Their `model` is ignored.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use super::Tier;
use crate::skills::{
    Root,
    frontmatter::{Fields, split},
};

pub const MAX_NAME_CHARS: usize = 32;
pub const MAX_DESCRIPTION_CHARS: usize = 1_024;
pub const MAX_PROMPT_CHARS: usize = 60_000;
/// Model requests a role may make in one round when its file names no limit.
pub const DEFAULT_MAX_TURNS: u32 = 60;
pub const MAX_MAX_TURNS: u32 = 500;
const MAX_FILE_BYTES: u64 = 256_000;
const MAX_PER_ROOT: usize = 200;
pub const BUILTIN_LABEL: &str = "Built in";

const BUILTINS: [&str; 3] = [include_str!("builtin/explore.md"), include_str!("builtin/review.md"), include_str!("builtin/general.md")];

/// Which model a role runs on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModelPref {
    /// The chat's own model and effort.
    #[default]
    Chat,
    /// The helper model chosen in Settings › Sub-agents, or the chat's when none is.
    Helper,
}

impl ModelPref {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelPref::Chat => "chat",
            ModelPref::Helper => "helper",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentDef {
    pub name: String,
    pub description: String,
    /// The role's own prompt: the file's body.
    pub prompt: String,
    /// The tools it may use, by name; an entry ending in `*` matches by prefix. `None` is every
    /// tool a sub-agent can have.
    pub tools: Option<Vec<String>>,
    pub model: ModelPref,
    /// Where it runs while the chat's Team is on; `None` is the crew.
    pub team: Option<Tier>,
    pub max_turns: u32,
    /// The file it came from, or `None` for a built-in.
    pub file: Option<PathBuf>,
    /// The label of the root it came from.
    pub source: String,
}

impl AgentDef {
    /// Whether the role may use the tool named `name`.
    pub fn allows(&self, name: &str) -> bool {
        match &self.tools {
            None => true,
            Some(tools) => tools.iter().any(|tool| match tool.strip_suffix('*') {
                Some(prefix) => name.starts_with(prefix),
                None => tool == name,
            }),
        }
    }

    /// Whether the role can change files through its tools.
    pub fn edits(&self) -> bool {
        self.allows("write") || self.allows("edit")
    }

    pub fn builtin(&self) -> bool {
        self.file.is_none()
    }
}

/// A role that lost its name to an earlier one.
#[derive(Clone, Debug, PartialEq)]
pub struct Shadowed {
    pub agent: AgentDef,
    /// The label of the root the winner came from.
    pub by: String,
}

/// A file that looked like a role but could not load, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub file: PathBuf,
    pub message: String,
}

/// The roles a chat may use.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    pub agents: Vec<AgentDef>,
    pub shadowed: Vec<Shadowed>,
    pub problems: Vec<Problem>,
}

impl Catalog {
    pub fn get(&self, name: &str) -> Option<&AgentDef> {
        self.agents.iter().find(|agent| agent.name == name)
    }
}

/// The roles compiled into the app, as their files define them.
pub fn builtins() -> Vec<AgentDef> {
    BUILTINS
        .iter()
        .map(|text| parse(text, None, BUILTIN_LABEL, None).expect("a built-in role parses"))
        .collect()
}

/// Every role in `roots` and the built-ins, first name winning. Roles named in `off` are left out
/// after names are compared, so switching off a customised built-in does not bring back the
/// original.
pub fn load(roots: &[Root], off: &HashSet<String>) -> Catalog {
    let mut catalog = Catalog::default();
    let mut found = Vec::new();
    for root in roots {
        let (agents, problems) = scan_root(root);
        found.extend(agents);
        catalog.problems.extend(problems);
    }
    found.extend(builtins());
    let mut winners: Vec<AgentDef> = Vec::new();
    for agent in found {
        match winners.iter().find(|winner| winner.name == agent.name) {
            Some(winner) => catalog.shadowed.push(Shadowed { by: winner.source.clone(), agent }),
            None => winners.push(agent),
        }
    }
    catalog.agents = winners.into_iter().filter(|agent| !off.contains(&agent.name)).collect();
    catalog
}

/// Every role in one root's top level, in name order, with the files that failed.
pub fn scan_root(root: &Root) -> (Vec<AgentDef>, Vec<Problem>) {
    let mut agents = Vec::new();
    let mut problems = Vec::new();
    let Ok(entries) = fs::read_dir(&root.path) else { return (agents, problems) };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .filter(|path| !path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.')))
        .collect();
    files.sort();
    for file in files.into_iter().take(MAX_PER_ROOT) {
        match read(&file, &root.label) {
            Ok(agent) => agents.push(agent),
            Err(message) => problems.push(Problem { file, message }),
        }
    }
    (agents, problems)
}

fn read(file: &Path, source: &str) -> Result<AgentDef, String> {
    let size = fs::metadata(file).map_err(|error| format!("It could not be read: {error}"))?.len();
    if size > MAX_FILE_BYTES {
        return Err("It is too large to be an agent's instructions.".to_string());
    }
    let text = fs::read_to_string(file).map_err(|error| format!("It could not be read: {error}"))?;
    let stem = file.file_stem().map(|stem| stem.to_string_lossy().into_owned());
    parse(&text, Some(file), source, stem.as_deref())
}

/// One role from a file's text. `fallback_name` is used when the frontmatter has no `name`.
pub fn parse(text: &str, file: Option<&Path>, source: &str, fallback_name: Option<&str>) -> Result<AgentDef, String> {
    let (yaml, body) = split(text);
    let fields = Fields::parse(yaml.as_deref())?;
    let name = fields.text("name").map(|name| name.trim().to_string()).or(fallback_name.map(str::to_string)).unwrap_or_default();
    check_name(&name)?;
    let description = fields.text("description").map(|text| text.trim().to_string()).unwrap_or_default();
    if description.is_empty() {
        return Err("It has no description, which is how the agent chooses it.".to_string());
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!("Its description is longer than {MAX_DESCRIPTION_CHARS} characters."));
    }
    if body.is_empty() {
        return Err("It has no instructions below its frontmatter.".to_string());
    }
    if body.chars().count() > MAX_PROMPT_CHARS {
        return Err(format!("Its instructions are longer than {MAX_PROMPT_CHARS} characters."));
    }
    let tools = fields.list("tools").map(|tools| {
        let mut seen = HashSet::new();
        tools.iter().map(|tool| our_tool_name(tool)).filter(|tool| seen.insert(tool.clone())).collect()
    });
    let model = match fields.text("model").map(|model| model.trim().to_lowercase()).as_deref() {
        Some("helper") => ModelPref::Helper,
        _ => ModelPref::Chat,
    };
    let team = match fields.text("team") {
        None => None,
        Some(text) => Some(Tier::parse(&text).ok_or("Its team must be lead or crew.")?),
    };
    let max_turns = match fields.text("max_turns") {
        None => DEFAULT_MAX_TURNS,
        Some(text) => match text.trim().parse::<u32>() {
            Ok(turns) if (1..=MAX_MAX_TURNS).contains(&turns) => turns,
            _ => return Err(format!("Its max_turns must be a whole number from 1 to {MAX_MAX_TURNS}.")),
        },
    };
    Ok(AgentDef {
        name,
        description,
        prompt: body,
        tools,
        model,
        team,
        max_turns,
        file: file.map(Path::to_path_buf),
        source: source.to_string(),
    })
}

/// `Err` says why a name can't be a role's.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("It has no name.".to_string());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("Its name is longer than {MAX_NAME_CHARS} characters."));
    }
    let mut chars = name.chars();
    let starts = chars.next().is_some_and(|first| first.is_ascii_lowercase());
    if !starts || !chars.all(|rest| rest.is_ascii_lowercase() || rest.is_ascii_digit() || rest == '-') {
        return Err(format!("Its name \"{name}\" must be lowercase letters, digits and hyphens, starting with a letter."));
    }
    Ok(())
}

/// Our name for a tool another agent tool's file names; our own names pass through.
fn our_tool_name(tool: &str) -> String {
    let mapped = match tool.to_lowercase().as_str() {
        "read" => "read",
        "write" => "write",
        "edit" | "multiedit" => "edit",
        "bash" => "bash",
        "grep" => "grep",
        "glob" => "find",
        "ls" => "ls",
        "webfetch" => "web_fetch",
        "websearch" => "web_search",
        _ => return tool.to_string(),
    };
    mapped.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_ins_parse_and_only_general_edits() {
        let builtins = builtins();
        let names: Vec<&str> = builtins.iter().map(|agent| agent.name.as_str()).collect();
        assert_eq!(names, ["explore", "review", "general"]);
        let edits: Vec<bool> = builtins.iter().map(AgentDef::edits).collect();
        assert_eq!(edits, [false, false, true]);
        // The no-edit roles still get the shell, and read-only server tools by prefix.
        let explore = &builtins[0];
        assert!(explore.allows("bash") && explore.allows("mcp__github__list_issues"));
        assert_eq!(explore.model, ModelPref::Helper);
        assert_eq!(builtins[2].model, ModelPref::Chat);
        // In a Team, review runs on the lead's model and the others on the crew.
        let teams: Vec<Option<Tier>> = builtins.iter().map(|agent| agent.team).collect();
        assert_eq!(teams, [None, Some(Tier::Lead), None]);
        assert!(builtins.iter().all(AgentDef::builtin));
    }

    #[test]
    fn a_file_reads_its_fields_and_other_tools_names_are_mapped() {
        let agent = parse(
            "---\ndescription: Checks docs\ntools: Read, Glob, WebFetch, Agent\nmodel: haiku\nmax_turns: 12\n---\nYou check docs.",
            Some(Path::new("/x/doc-checker.md")),
            "Claude Code",
            Some("doc-checker"),
        )
        .unwrap();
        assert_eq!(agent.name, "doc-checker");
        assert_eq!(agent.tools.as_deref(), Some(&["read".to_string(), "find".to_string(), "web_fetch".to_string(), "Agent".to_string()][..]));
        assert_eq!(agent.model, ModelPref::Chat, "another tool's model name is ignored");
        assert_eq!(agent.max_turns, 12);
        assert!(!agent.edits());
        assert!(!agent.allows("agent"), "a name we don't know matches nothing");

        let all = parse("---\nname: doer\ndescription: Does\n---\nDo.", None, "x", None).unwrap();
        assert!(all.tools.is_none() && all.edits() && all.max_turns == DEFAULT_MAX_TURNS);
    }

    #[test]
    fn bad_files_say_why() {
        let problem = |text: &str| parse(text, None, "x", Some("ok-name")).unwrap_err();
        assert!(problem("---\nname: Bad Name\ndescription: d\n---\nbody").contains("lowercase"));
        assert!(problem("---\nname: fine\n---\nbody").contains("no description"));
        assert!(problem("---\ndescription: d\n---\n").contains("no instructions"));
        assert!(problem("---\ndescription: d\nmax_turns: 0\n---\nbody").contains("max_turns"));
        assert!(problem("---\ndescription: d\nteam: boss\n---\nbody").contains("lead or crew"));
        assert!(problem("---\ndescription: [unclosed\n---\nbody").contains("YAML"));
    }

    #[test]
    fn a_users_file_replaces_a_built_in_and_switching_it_off_leaves_neither() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(folder.path().join("explore.md"), "---\ndescription: My explorer\n---\nLook around.").unwrap();
        fs::write(folder.path().join("broken.md"), "---\nname: broken\n---\nNo description.").unwrap();
        fs::write(folder.path().join("notes.txt"), "not a role").unwrap();
        let roots = [Root { path: folder.path().to_path_buf(), label: "Your agents".into() }];

        let catalog = load(&roots, &HashSet::new());
        let explore = catalog.get("explore").unwrap();
        assert_eq!(explore.description, "My explorer");
        assert_eq!(explore.source, "Your agents");
        assert_eq!(catalog.shadowed.len(), 1);
        assert_eq!(catalog.shadowed[0].by, "Your agents");
        assert!(catalog.shadowed[0].agent.builtin());
        assert_eq!(catalog.problems.len(), 1);
        assert_eq!(catalog.agents.len(), 3);

        let catalog = load(&roots, &HashSet::from(["explore".to_string()]));
        assert!(catalog.get("explore").is_none());
        assert_eq!(catalog.agents.len(), 2);
    }
}
