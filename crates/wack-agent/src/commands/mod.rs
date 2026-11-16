//! Slash commands: what `/name args` at the start of a message means. Ported from version 2
//! (`slash.ts`, `user-commands.ts`, `command-utils.ts`) with its tested rules.
//!
//! - **Three kinds,** in catalogue order: WackCode's own ([`BUILTINS`]), the user's templates
//!   (`<data>/v3/commands/*.md`, the file name being the command), then skills.
//! - **Names:** the first entry to claim a name wins it; a later one, or one that takes a
//!   built-in's name, is renamed `<kind>:<name>` (then `:<n>`), so a skill that loses `/pdf`
//!   still answers to `/skill:pdf`. A switched-off entry frees its name. Every skill also
//!   answers to the alias `skill:<name>`.
//! - **Expansion** happens in the engine, the same for the app and `wack`: a template's body with
//!   its `$1` / `$ARGUMENTS` placeholders filled ([`expand_template`]), a skill's `<skill>` block
//!   ([`crate::skills::expand`]), and `/init`'s prompt. The other built-ins act on the app and
//!   never reach the model. What the user typed is kept beside the expanded text as a
//!   [`CommandUse`], so the transcript shows `/pdf form.pdf` rather than the whole skill.
//! - **Only a leading `/name`** is a command. The `/` menu opens on a `/` after whitespace too,
//!   and picking moves the command to the front ([`pick`]).

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::skills::{self, Skill, frontmatter};

pub const MAX_NAME_CHARS: usize = 64;
pub const MAX_BODY_CHARS: usize = 200_000;
const MAX_DESCRIPTION_FALLBACK: usize = 60;

/// One of WackCode's own commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Builtin {
    pub name: &'static str,
    pub hint: Option<&'static str>,
    pub description: &'static str,
}

pub const BUILTINS: &[Builtin] = &[
    Builtin { name: "compact", hint: Some("[focus]"), description: "Summarise older conversation context" },
    Builtin { name: "init", hint: None, description: "Create or refine this project's AGENTS.md" },
    Builtin { name: "new", hint: None, description: "Open a new chat" },
    Builtin { name: "name", hint: Some("<name>"), description: "Rename this chat" },
    Builtin { name: "copy", hint: None, description: "Copy the latest reply" },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Builtin,
    Template,
    Skill,
}

impl Kind {
    /// The prefix a renamed entry gets, and the label the `/` menu shows.
    pub fn prefix(self) -> &'static str {
        match self {
            Kind::Builtin => "wackcode",
            Kind::Template => "command",
            Kind::Skill => "skill",
        }
    }
}

/// What the user typed to send a message, kept beside the expanded text the model gets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandUse {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub args: String,
    pub kind: Kind,
}

impl CommandUse {
    /// The line as typed, `/name args`.
    pub fn line(&self) -> String {
        if self.args.is_empty() { format!("/{}", self.name) } else { format!("/{} {}", self.name, self.args) }
    }
}

/// A user command: a Markdown file whose name is the command and whose body is the prompt.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    pub name: String,
    pub description: String,
    pub hint: Option<String>,
    pub body: String,
    pub file: PathBuf,
}

/// A command name the user can type: lowercase words joined by single hyphens.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_CHARS
        && name.split('-').all(|word| !word.is_empty() && word.chars().all(|character| character.is_ascii_lowercase() || character.is_ascii_digit()))
}

/// One template file. `None` when it is not a command (a bad name, no body, unreadable).
pub fn load_template(file: &Path) -> Option<Template> {
    let name = file.file_name()?.to_str()?.strip_suffix(".md")?.to_string();
    if !valid_name(&name) {
        return None;
    }
    let text = fs::read_to_string(file).ok()?;
    let (yaml, body) = frontmatter::split(&text);
    if body.is_empty() {
        return None;
    }
    let fields = frontmatter::Fields::parse(yaml.as_deref()).ok()?;
    let description = fields.text("description").map(|text| text.trim().to_string()).filter(|text| !text.is_empty()).unwrap_or_else(|| {
        let first = body.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim();
        if first.chars().count() > MAX_DESCRIPTION_FALLBACK {
            format!("{}…", first.chars().take(MAX_DESCRIPTION_FALLBACK).collect::<String>())
        } else {
            first.to_string()
        }
    });
    Some(Template { name, description, hint: fields.hint("argument-hint"), body, file: file.to_path_buf() })
}

/// Every template in `dir`, by name. A missing folder has none.
pub fn load_templates(dir: &Path) -> Vec<Template> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut templates: Vec<Template> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .filter(|entry| fs::metadata(entry.path()).is_ok_and(|meta| meta.is_file()))
        .filter_map(|entry| load_template(&entry.path()))
        .collect();
    templates.sort_by(|a, b| a.name.cmp(&b.name));
    templates
}

/// A template file as the editor saves it, keeping any other frontmatter key as written.
pub fn compose_template(original: Option<&str>, description: &str, hint: &str, body: &str) -> String {
    use frontmatter::Value;
    let (description, hint) = (description.trim(), hint.trim());
    frontmatter::compose(
        original,
        &[
            ("description", (!description.is_empty()).then_some(Value::Text(description))),
            ("argument-hint", (!hint.is_empty()).then_some(Value::Text(hint))),
        ],
        body,
    )
}

/// Arguments split on whitespace, with `'` or `"` quoting a run that keeps its spaces. An empty
/// quoted argument (`""`) keeps its position.
pub fn split_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut quote: Option<char> = None;
    for character in input.chars() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => current.push(character),
            None if character == '\'' || character == '"' => {
                quote = Some(character);
                has_token = true;
            }
            None if character.is_whitespace() => {
                if has_token {
                    args.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            None => {
                current.push(character);
                has_token = true;
            }
        }
    }
    if has_token {
        args.push(current);
    }
    args
}

/// A template's body with its placeholders filled from `input`, by Pi's rules: `$1`…,
/// `$ARGUMENTS` and `$@` (every argument), `${N:-default}`, `${@:start}`, `${@:start:count}`.
/// Text an argument brings in is never expanded again. A body without placeholders ignores the
/// arguments, as version 2's did.
pub fn expand_template(body: &str, input: &str) -> String {
    let args = split_args(input);
    let all = args.join(" ");
    let arg = |index: usize| index.checked_sub(1).and_then(|index| args.get(index)).cloned().unwrap_or_default();
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some((value, used)) = placeholder(after, &args, &all, &arg) {
            out.push_str(&value);
            rest = &after[used..];
        } else {
            out.push('$');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// The placeholder at the start of `text` (just after a `$`): its value and how many bytes it
/// spans, or `None` when `$` starts nothing.
fn placeholder(text: &str, args: &[String], all: &str, arg: &dyn Fn(usize) -> String) -> Option<(String, usize)> {
    let digits = |text: &str| text.bytes().take_while(u8::is_ascii_digit).count();
    if let Some(inner) = text.strip_prefix('{') {
        let close = inner.find('}')?;
        let body = &inner[..close];
        let used = close + 2;
        // ${N:-default}, ${@:-default}, ${ARGUMENTS:-default}
        if let Some((target, fallback)) = body.split_once(":-") {
            let value = match target {
                "@" | "ARGUMENTS" => all.to_string(),
                _ if !target.is_empty() && digits(target) == target.len() => arg(target.parse().ok()?),
                _ => return None,
            };
            return Some((if value.is_empty() { fallback.to_string() } else { value }, used));
        }
        // ${@:start} and ${@:start:count}
        let slice = body.strip_prefix("@:")?;
        let (start, count) = match slice.split_once(':') {
            Some((start, count)) => (start, Some(count)),
            None => (slice, None),
        };
        if start.is_empty() || digits(start) != start.len() || count.is_some_and(|count| count.is_empty() || digits(count) != count.len()) {
            return None;
        }
        let start = start.parse::<usize>().ok()?.saturating_sub(1).min(args.len());
        let end = match count {
            Some(count) => start.saturating_add(count.parse().ok()?).min(args.len()),
            None => args.len(),
        };
        return Some((args[start..end].join(" "), used));
    }
    if text.starts_with("ARGUMENTS") {
        return Some((all.to_string(), "ARGUMENTS".len()));
    }
    if text.starts_with('@') {
        return Some((all.to_string(), 1));
    }
    let count = digits(text);
    (count > 0).then(|| (arg(text[..count].parse().unwrap_or(0)), count))
}

/// `/init`'s prompt for a project at `cwd` (version 2's, less Pi's override file).
pub fn init_prompt(cwd: &Path) -> Result<String, String> {
    let agents = cwd.join("AGENTS.md");
    let existing = match fs::symlink_metadata(&agents) {
        Ok(meta) if !meta.is_file() => return Err("AGENTS.md must be a regular file, not a folder or symlink.".into()),
        Ok(_) => true,
        Err(_) => false,
    };
    let claude = cwd.join("CLAUDE.md").is_file();
    let mut prompt = String::from(
        "Initialize project instructions in ./AGENTS.md at this workspace root. This is a focused documentation task: change no other files.\n\nInspect the repository before writing. Read the existing AGENTS.md if present, ",
    );
    if claude {
        prompt.push_str("and read CLAUDE.md because WackCode will prefer the new AGENTS.md over it, ");
    }
    prompt.push_str("then consult only the relevant README, manifests, CI configuration, and source files needed to verify useful guidance. Include project-specific setup and test commands, non-obvious conventions or constraints, and short pointers to deeper documentation. Verify every factual claim against the repository. Do not copy large sections of existing docs, list the whole tree, include secrets, or add generic coding advice. Aim for well under 100 lines; a small project may need only a few bullets. Omit sections with no useful verified guidance.\n\n");
    prompt.push_str(if existing {
        "Conservatively refine the existing AGENTS.md: preserve useful intentional instructions, correct unsupported or stale claims, and trim clear redundancy. If it is already sufficient, leave it unchanged."
    } else {
        "Create AGENTS.md only if there is defensible project-specific guidance. If there is none, leave it absent and explain why."
    });
    prompt.push(' ');
    if claude {
        prompt.push_str("Carry over applicable guidance from CLAUDE.md without editing that file. ");
    }
    prompt.push_str("Do not delete AGENTS.md. When finished, briefly state whether you created, updated, or left it unchanged and why.");
    Ok(prompt)
}

// ---------------------------------------------------------------------------------------------
// The catalogue

/// What a command does once picked.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Builtin(Builtin),
    Template(Template),
    Skill(Skill),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    /// What the user types after `/`.
    pub name: String,
    /// Other names it answers to (`skill:<name>`).
    pub aliases: Vec<String>,
    pub kind: Kind,
    pub hint: Option<String>,
    pub description: String,
    /// Where it comes from: "WackCode", "Your commands", or a skill folder's label.
    pub source: String,
    /// Its stable identity for Settings' switches: `builtin:<name>`, `template:<file>`,
    /// `skill:<file>`.
    pub key: String,
    pub action: Action,
}

pub fn builtin_key(name: &str) -> String {
    format!("builtin:{name}")
}

pub fn file_key(kind: Kind, file: &Path) -> String {
    format!("{}:{}", if kind == Kind::Template { "template" } else { "skill" }, file.display())
}

/// Every command offered: built-ins, templates, then skills, minus the keys in `off`, with names
/// resolved. Skills marked as off should already be gone from `skills` (switching one off lets
/// another with its name load), but their keys are honoured here too.
pub fn catalog(templates: &[Template], skills: &[Skill], off: &HashSet<String>) -> Vec<Command> {
    let mut entries: Vec<(String, Command)> = Vec::new();
    for builtin in BUILTINS {
        entries.push((
            builtin.name.to_string(),
            Command {
                name: String::new(),
                aliases: Vec::new(),
                kind: Kind::Builtin,
                hint: builtin.hint.map(str::to_string),
                description: builtin.description.to_string(),
                source: "WackCode".into(),
                key: builtin_key(builtin.name),
                action: Action::Builtin(*builtin),
            },
        ));
    }
    for template in templates {
        entries.push((
            template.name.clone(),
            Command {
                name: String::new(),
                aliases: Vec::new(),
                kind: Kind::Template,
                hint: template.hint.clone(),
                description: template.description.clone(),
                source: "Your commands".into(),
                key: file_key(Kind::Template, &template.file),
                action: Action::Template(template.clone()),
            },
        ));
    }
    for skill in skills {
        entries.push((
            skill.name.clone(),
            Command {
                name: String::new(),
                aliases: vec![format!("skill:{}", skill.name)],
                kind: Kind::Skill,
                hint: skill.hint.clone(),
                description: skill.description.clone(),
                source: skill.source.clone(),
                key: file_key(Kind::Skill, &skill.file),
                action: Action::Skill(skill.clone()),
            },
        ));
    }
    // Built-ins keep their names even when switched off, so nothing else takes `/new`.
    let mut used: HashSet<String> = BUILTINS.iter().map(|builtin| builtin.name.to_string()).collect();
    let mut commands = Vec::new();
    for (candidate, mut command) in entries {
        if off.contains(&command.key) {
            continue;
        }
        let mut name = candidate.clone();
        if command.kind != Kind::Builtin && used.contains(&name) {
            let base = format!("{}:{candidate}", command.kind.prefix());
            name = base.clone();
            let mut suffix = 2;
            while used.contains(&name) {
                name = format!("{base}:{suffix}");
                suffix += 1;
            }
        }
        used.insert(name.clone());
        command.aliases.retain(|alias| *alias != name);
        command.name = name;
        commands.push(command);
    }
    commands
}

/// The command a leading `/name` names: by name first, then by alias.
pub fn find<'a>(commands: &'a [Command], name: &str) -> Option<&'a Command> {
    commands.iter().find(|command| command.name == name).or_else(|| commands.iter().find(|command| command.aliases.iter().any(|alias| alias == name)))
}

/// A message's leading `/name` and the rest as its arguments. `None` when it does not start with
/// one. A second `/` in the word makes it a path (`/tmp/x`), not a command.
pub fn parse_line(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix('/')?;
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let name = &rest[..end];
    if name.is_empty() || name.contains('/') {
        return None;
    }
    Some((name, rest[end..].trim()))
}

/// The commands the `/` menu shows for `query` (what follows the `/`): names that start with it
/// first, then names or aliases that contain it, each group in catalogue order.
pub fn matching<'a>(commands: &'a [Command], query: &str) -> Vec<&'a Command> {
    let query = query.to_lowercase();
    let starts = |command: &&Command| command.name.to_lowercase().starts_with(&query);
    let contains = |command: &&Command| {
        command.name.to_lowercase().contains(&query) || command.aliases.iter().any(|alias| alias.to_lowercase().contains(&query))
    };
    let mut found: Vec<&Command> = commands.iter().filter(starts).collect();
    found.extend(commands.iter().filter(|command| !starts(command) && contains(command)));
    found
}

/// The `/word` the caret is in, for the `/` menu: its byte range in `text`. Only a `/` at the
/// start of the text or after whitespace, with the caret in the word, counts. A second `/` makes
/// it a path or URL, which never opens the menu.
pub fn active_slash(text: &str, caret: usize) -> Option<std::ops::Range<usize>> {
    let before = text.get(..caret)?;
    let start = before.rfind(|character: char| character.is_whitespace()).map_or(0, |at| at + before[at..].chars().next().map_or(1, char::len_utf8));
    let word = &before[start..];
    if !word.starts_with('/') || word[1..].contains('/') {
        return None;
    }
    let end = caret + text[caret..].find(char::is_whitespace).unwrap_or(text.len() - caret);
    if text[start + 1..end].contains('/') {
        return None;
    }
    Some(start..end)
}

/// The text after picking `name` for the `/word` at `range`: the command moves to the front, and
/// what was around the word becomes its arguments. Returns the text and where the caret goes.
pub fn pick(text: &str, range: std::ops::Range<usize>, name: &str) -> (String, usize) {
    let before = text[..range.start].trim_end();
    let after = text[range.end..].trim_start();
    let head = format!("/{name} ");
    let mut out = head.clone();
    out.push_str(before);
    let caret = out.len();
    if !before.is_empty() && !after.is_empty() {
        out.push(' ');
    }
    out.push_str(after);
    // With nothing before the word, the caret goes where the arguments start.
    (out, if before.is_empty() { head.len() } else { caret })
}

/// What sending a message that names `command` sends to the model. `None` for a built-in that
/// acts on the app instead (everything but `/init`).
pub fn expand(command: &Command, args: &str, cwd: &Path) -> Option<Result<String, String>> {
    match &command.action {
        Action::Template(template) => Some(Ok(expand_template(&template.body, args))),
        Action::Skill(skill) => Some(skills::expand(skill, args)),
        Action::Builtin(builtin) if builtin.name == "init" => Some(init_prompt(cwd)),
        Action::Builtin(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(name: &str) -> Template {
        Template { name: name.into(), description: String::new(), hint: None, body: "b".into(), file: PathBuf::from(format!("/c/{name}.md")) }
    }

    fn skill(name: &str, source: &str) -> Skill {
        Skill {
            name: name.into(),
            description: "d".into(),
            file: PathBuf::from(format!("/{source}/{name}/SKILL.md")),
            base_dir: PathBuf::from(format!("/{source}/{name}")),
            manual: false,
            hint: None,
            source: source.into(),
        }
    }

    fn names(commands: &[Command]) -> Vec<&str> {
        commands.iter().map(|command| command.name.as_str()).collect()
    }

    #[test]
    fn the_first_claim_wins_and_later_ones_are_renamed() {
        let commands = catalog(&[template("review"), template("new")], &[skill("review", "a"), skill("pdf", "a"), skill("copy", "a")], &HashSet::new());
        assert_eq!(names(&commands)[5..], ["review", "command:new", "skill:review", "pdf", "skill:copy"]);
        let pdf = find(&commands, "pdf").unwrap();
        assert_eq!(pdf.aliases, ["skill:pdf"]);
        assert_eq!(find(&commands, "skill:pdf").unwrap().name, "pdf");
        // A renamed skill has no alias left that equals its name.
        assert!(find(&commands, "skill:review").unwrap().aliases.is_empty());
    }

    #[test]
    fn a_renamed_name_that_is_taken_gets_a_number() {
        let mut taken = template("pdf");
        taken.name = "skill:pdf".into();
        let commands = catalog(&[template("pdf"), taken], &[skill("pdf", "a")], &HashSet::new());
        assert_eq!(names(&commands)[5..], ["pdf", "skill:pdf", "skill:pdf:2"]);
    }

    #[test]
    fn switching_one_off_frees_its_name() {
        let off = HashSet::from([file_key(Kind::Template, Path::new("/c/review.md")), builtin_key("copy")]);
        let commands = catalog(&[template("review")], &[skill("review", "a"), skill("copy", "a")], &off);
        assert_eq!(names(&commands), ["compact", "init", "new", "name", "review", "skill:copy"]);
    }

    #[test]
    fn only_a_leading_word_is_a_command() {
        assert_eq!(parse_line("/pdf form.pdf  more "), Some(("pdf", "form.pdf  more")));
        assert_eq!(parse_line("/new"), Some(("new", "")));
        assert_eq!(parse_line("/tmp/x is a path"), None);
        assert_eq!(parse_line("say /pdf"), None);
        assert_eq!(parse_line("/ alone"), None);
    }

    #[test]
    fn prefix_matches_come_first() {
        let commands = catalog(&[template("review-code")], &[skill("pdf", "a")], &HashSet::new());
        let found: Vec<&str> = matching(&commands, "co").iter().map(|command| command.name.as_str()).collect();
        assert_eq!(found, ["compact", "copy", "review-code"]);
        let found: Vec<&str> = matching(&commands, "skill:").iter().map(|command| command.name.as_str()).collect();
        assert_eq!(found, ["pdf"]);
        assert_eq!(matching(&commands, "").len(), commands.len());
    }

    #[test]
    fn the_menu_opens_on_a_slash_word_under_the_caret() {
        assert_eq!(active_slash("/", 1), Some(0..1));
        assert_eq!(active_slash("/pd", 3), Some(0..3));
        assert_eq!(active_slash("fix this /pd", 12), Some(9..12));
        assert_eq!(active_slash("first\n/pd", 9), Some(6..9));
        // The caret mid-word still covers the whole word.
        assert_eq!(active_slash("/pdf now", 2), Some(0..4));
        assert_eq!(active_slash("a/pd", 4), None);
        assert_eq!(active_slash("/tmp/x", 6), None);
        assert_eq!(active_slash("/tmp/x", 2), None);
        assert_eq!(active_slash("https://x", 9), None);
        assert_eq!(active_slash("//x", 3), None);
        assert_eq!(active_slash("/pdf now", 8), None);
        assert_eq!(active_slash("é /p", "é /p".len()), Some(3..5));
    }

    #[test]
    fn picking_moves_the_command_to_the_front() {
        assert_eq!(pick("/pd", 0..3, "pdf"), ("/pdf ".to_string(), 5));
        assert_eq!(pick("fill this /pd", 10..13, "pdf"), ("/pdf fill this".to_string(), 14));
        assert_eq!(pick("fill /pd form.pdf", 5..8, "pdf"), ("/pdf fill form.pdf".to_string(), 9));
        assert_eq!(pick("/pd form.pdf", 0..3, "pdf"), ("/pdf form.pdf".to_string(), 5));
    }

    #[test]
    fn templates_keep_pis_argument_rules() {
        assert_eq!(expand_template("$1 | $2 | $ARGUMENTS | ${3:-fallback} | ${@:2}", "\"two words\" next"), "two words | next | two words next | fallback | next");
        assert_eq!(expand_template("$1", "'$ARGUMENTS'"), "$ARGUMENTS");
        assert_eq!(expand_template("${1:-first} | $2 | ${3:-third} | ${@:2}", "\"\" next '' last"), "first | next | third | next  last");
        assert_eq!(expand_template("${@:1:2} and $@", "a b c"), "a b and a b c");
        assert_eq!(expand_template("cost $ and ${x} and $", "a"), "cost $ and ${x} and $");
        assert_eq!(expand_template("No placeholders.", "ignored"), "No placeholders.");
        assert_eq!(expand_template("$9", "a"), "");
    }

    #[test]
    fn template_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("review.md"), "---\nargument-hint: <file>\n---\nReview $1 carefully, and say what you would change in it before changing anything.").unwrap();
        fs::write(dir.path().join("Bad Name.md"), "Body").unwrap();
        fs::write(dir.path().join("empty.md"), "---\ndescription: x\n---\n").unwrap();
        fs::write(dir.path().join("notes.txt"), "Body").unwrap();
        let templates = load_templates(dir.path());
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "review");
        assert_eq!(templates[0].hint.as_deref(), Some("<file>"));
        assert_eq!(templates[0].description, "Review $1 carefully, and say what you would change in it bef…");
        let text = compose_template(None, "Review a file", "<file>", "Review $1.");
        fs::write(dir.path().join("review.md"), text).unwrap();
        assert_eq!(load_templates(dir.path())[0].description, "Review a file");
    }

    #[test]
    fn init_reads_what_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let prompt = init_prompt(dir.path()).unwrap();
        assert!(prompt.contains("Create AGENTS.md only if"));
        assert!(!prompt.contains("CLAUDE.md"));
        fs::write(dir.path().join("AGENTS.md"), "# Rules").unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "# Rules").unwrap();
        let prompt = init_prompt(dir.path()).unwrap();
        assert!(prompt.contains("Conservatively refine"));
        assert!(prompt.contains("Carry over applicable guidance from CLAUDE.md"));
    }

    #[test]
    fn a_command_use_reads_as_typed() {
        assert_eq!(CommandUse { name: "pdf".into(), args: "a b".into(), kind: Kind::Skill }.line(), "/pdf a b");
        assert_eq!(CommandUse { name: "init".into(), args: String::new(), kind: Kind::Builtin }.line(), "/init");
    }
}
