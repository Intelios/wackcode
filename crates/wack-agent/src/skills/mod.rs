//! Skills: folders of instructions (a `SKILL.md` and whatever it refers to) the agent loads when
//! a task matches, or the user runs as `/name`. Ported from version 2, which took its rules from
//! Pi's loader and the Agent Skills format.
//!
//! - **Where from:** only the roots the engine hands over, in order (`~/.agents/skills` first,
//!   then other tools' folders the user switched on, then folders they added). A project's own
//!   skill folders are never read: a cloned repository can't put instructions in front of the
//!   agent beyond its `AGENTS.md`.
//! - **What is a skill:** a folder holding `SKILL.md` is one skill and is not searched further.
//!   Other folders are searched for more; a root's top-level `.md` files are skills too when
//!   they have a description. Dot entries and `node_modules` are skipped, symlinks followed.
//! - **Name clashes:** the first skill with a name wins, in root order. A switched-off skill
//!   drops out before names are compared, so the next one with its name loads instead. The same
//!   file reached twice (a symlink between two tools' folders) loads once. Losers are kept as
//!   [`Shadowed`] so Settings can say why they are "Not loaded".
//! - **How the model sees them:** [`index`] lists each skill's name, description and file in the
//!   system prompt, read once per run (so the prompt cache holds and a run never sees its skills
//!   change). The model loads one with the ordinary `read` tool; there is no skill tool. A skill
//!   marked `disable-model-invocation` is left out and runs only as `/name`.
//! - **`/name`:** [`expand`] wraps the body in a `<skill>` block, version 2's exact text.

pub mod frontmatter;

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use frontmatter::{Fields, Value};

pub const MAX_NAME_CHARS: usize = 64;
pub const MAX_DESCRIPTION_CHARS: usize = 1_024;
pub const MAX_HINT_CHARS: usize = 256;
pub const MAX_BODY_CHARS: usize = 200_000;
/// A skill file larger than this is not read: instructions are never this long.
const MAX_FILE_BYTES: u64 = 1_000_000;
/// How deep a root is searched, which also stops a symlink loop.
const MAX_DEPTH: usize = 8;
/// More than this from one root is a folder that is not really skills.
const MAX_PER_ROOT: usize = 500;
const SKILL_FILE: &str = "SKILL.md";

/// One folder skills load from, with the label Settings and the `/` menu show for it.
#[derive(Clone, Debug, PartialEq)]
pub struct Root {
    pub path: PathBuf,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// The `SKILL.md` (or top-level `.md`) file.
    pub file: PathBuf,
    /// The folder its relative references resolve against.
    pub base_dir: PathBuf,
    /// `disable-model-invocation`: runs only when the user types `/name`.
    pub manual: bool,
    /// `argument-hint`: what `/name` takes, as `<required>` or `[optional]`.
    pub hint: Option<String>,
    /// The label of the root it came from.
    pub source: String,
}

/// A skill that lost its name to an earlier one.
#[derive(Clone, Debug, PartialEq)]
pub struct Shadowed {
    pub skill: Skill,
    /// The label of the root the winner came from.
    pub by: String,
}

/// A file that looked like a skill but could not load, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub file: PathBuf,
    pub message: String,
}

/// What a chat loads from its roots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    pub skills: Vec<Skill>,
    pub shadowed: Vec<Shadowed>,
    pub problems: Vec<Problem>,
}

/// Every skill in one root, in a stable order, with the files that failed. Nothing is checked
/// against other roots here.
pub fn scan_root(root: &Root) -> (Vec<Skill>, Vec<Problem>) {
    let mut found = (Vec::new(), Vec::new());
    let mut visited = HashSet::new();
    // A root that is itself a single `.md` file (an import picks one) is one skill.
    if root.path.is_file() {
        if let Some(result) = load_file(&root.path, &root.label, true) {
            push(&mut found, result);
        }
        return found;
    }
    walk(&root.path, root, 0, true, &mut visited, &mut found);
    found
}

fn push(found: &mut (Vec<Skill>, Vec<Problem>), result: Result<Skill, Problem>) {
    match result {
        Ok(skill) => found.0.push(skill),
        Err(problem) => found.1.push(problem),
    }
}

fn walk(dir: &Path, root: &Root, depth: usize, top: bool, visited: &mut HashSet<PathBuf>, found: &mut (Vec<Skill>, Vec<Problem>)) {
    if depth > MAX_DEPTH || found.0.len() >= MAX_PER_ROOT {
        return;
    }
    let Ok(real) = fs::canonicalize(dir) else { return };
    if !visited.insert(real) {
        return;
    }
    let skill_file = dir.join(SKILL_FILE);
    if skill_file.is_file() {
        if let Some(result) = load_file(&skill_file, &root.label, true) {
            push(found, result);
        }
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut entries: Vec<(String, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| (entry.file_name().to_string_lossy().into_owned(), entry.path()))
        .filter(|(name, _)| !name.starts_with('.') && name != "node_modules")
        .collect();
    entries.sort();
    for (name, path) in entries {
        // `metadata` follows symlinks; a broken one fails and is skipped.
        let Ok(meta) = fs::metadata(&path) else { continue };
        if meta.is_dir() {
            walk(&path, root, depth + 1, false, visited, found);
        } else if top
            && meta.is_file()
            && name.ends_with(".md")
            && let Some(result) = load_file(&path, &root.label, false)
        {
            push(found, result);
        }
    }
}

/// One file as a skill. `declared` is a `SKILL.md`: one that fails says why, while a loose `.md`
/// without a description is just not a skill (`None`).
fn load_file(file: &Path, source: &str, declared: bool) -> Option<Result<Skill, Problem>> {
    let problem = |message: String| Some(Err(Problem { file: file.to_path_buf(), message }));
    let too_big = fs::metadata(file).is_ok_and(|meta| meta.len() > MAX_FILE_BYTES);
    if too_big {
        return if declared { problem("It is larger than 1 MB.".into()) } else { None };
    }
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) if declared => return problem(format!("It could not be read: {error}")),
        Err(_) => return None,
    };
    let (yaml, _) = frontmatter::split(&text);
    let fields = match Fields::parse(yaml.as_deref()) {
        Ok(fields) => fields,
        Err(message) if declared => return problem(message),
        Err(_) => return None,
    };
    let description = fields.text("description").map(|text| text.trim().to_string()).unwrap_or_default();
    if description.is_empty() {
        return if declared { problem("It has no description, which the agent decides from.".into()) } else { None };
    }
    let base_dir = file.parent().unwrap_or(Path::new("/")).to_path_buf();
    let folder = base_dir.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let fallback = if declared {
        folder
    } else {
        file.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or(folder)
    };
    let name = fields.text("name").map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).unwrap_or(fallback);
    if name.is_empty() || name.contains(char::is_whitespace) {
        return problem("Its name has spaces in it, so it can't be typed as /name.".into());
    }
    Some(Ok(Skill {
        name,
        description: description.chars().take(MAX_DESCRIPTION_CHARS).collect(),
        file: file.to_path_buf(),
        base_dir,
        manual: fields.flag("disable-model-invocation"),
        hint: fields.hint("argument-hint").map(|hint| hint.chars().take(MAX_HINT_CHARS).collect()),
        source: source.to_string(),
    }))
}

/// What a chat loads: `roots` in order, minus the files in `disabled`, first name winning.
pub fn load(roots: &[Root], disabled: &HashSet<PathBuf>) -> Catalog {
    let mut catalog = Catalog::default();
    let mut seen = HashSet::new();
    for root in roots {
        let (skills, problems) = scan_root(root);
        catalog.problems.extend(problems);
        for skill in skills {
            if disabled.contains(&skill.file) {
                continue;
            }
            let real = fs::canonicalize(&skill.file).unwrap_or_else(|_| skill.file.clone());
            if seen.contains(&real) {
                continue;
            }
            if let Some(winner) = catalog.skills.iter().find(|winner| winner.name == skill.name) {
                let by = winner.source.clone();
                catalog.shadowed.push(Shadowed { skill, by });
                continue;
            }
            seen.insert(real);
            catalog.skills.push(skill);
        }
    }
    catalog
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// The system prompt's skills section (Pi's `formatSkillsForPrompt` plus version 2's one-line
/// rule); `None` when no skill is offered to the model.
pub fn index(catalog: &Catalog) -> Option<String> {
    let offered: Vec<&Skill> = catalog.skills.iter().filter(|skill| !skill.manual).collect();
    if offered.is_empty() {
        return None;
    }
    let mut lines = vec![
        "# Skills".to_string(),
        String::new(),
        "The following skills provide specialized instructions for specific tasks.".to_string(),
        "Use the read tool to load a skill's file when the task matches its description.".to_string(),
        "When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.".to_string(),
        "Read a matching skill's file first and announce that you are using it. The user's instructions beat the skill's.".to_string(),
        String::new(),
        "<available_skills>".to_string(),
    ];
    for skill in offered {
        lines.push("  <skill>".to_string());
        lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
        lines.push(format!("    <description>{}</description>", escape_xml(&skill.description)));
        lines.push(format!("    <location>{}</location>", escape_xml(&skill.file.display().to_string())));
        lines.push("  </skill>".to_string());
    }
    lines.push("</available_skills>".to_string());
    Some(lines.join("\n"))
}

/// The user message `/name args` sends: the skill's body (read now, without its frontmatter) in
/// a `<skill>` block, then the arguments.
pub fn expand(skill: &Skill, args: &str) -> Result<String, String> {
    let text = fs::read_to_string(&skill.file).map_err(|_| format!("The skill {} could not be read. It may have been moved or deleted.", skill.name))?;
    let (_, body) = frontmatter::split(&text);
    let mut out = format!(
        "<skill name=\"{}\" location=\"{}\">\nReferences are relative to {}.\n\n{}\n</skill>",
        skill.name,
        skill.file.display(),
        skill.base_dir.display(),
        body
    );
    let args = args.trim();
    if !args.is_empty() {
        out.push_str("\n\n");
        out.push_str(args);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// The editor's rules (Settings › Skills). Loading is lenient, as other tools' skills deserve;
// saving holds the user's own skills to the Agent Skills format.

pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the skill a name.".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("A skill name can be at most {MAX_NAME_CHARS} characters."));
    }
    if !name.chars().all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-') {
        return Err("Use only lowercase letters, numbers and hyphens in a skill name.".into());
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        return Err("A skill name can't start or end with a hyphen, or have two in a row.".into());
    }
    Ok(name.to_string())
}

pub fn validate_description(description: &str) -> Result<String, String> {
    let description = description.trim();
    if description.is_empty() {
        return Err("Describe what the skill does and when to use it: the agent decides from this alone.".into());
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!("A skill's description can be at most {MAX_DESCRIPTION_CHARS} characters."));
    }
    Ok(description.to_string())
}

pub fn validate_body(body: &str) -> Result<String, String> {
    if body.chars().count() > MAX_BODY_CHARS {
        return Err(format!("A skill's instructions can be at most {MAX_BODY_CHARS} characters."));
    }
    Ok(body.trim().to_string())
}

pub fn validate_hint(hint: &str) -> Result<String, String> {
    let hint = hint.trim();
    if hint.chars().count() > MAX_HINT_CHARS {
        return Err(format!("An argument hint can be at most {MAX_HINT_CHARS} characters."));
    }
    Ok(hint.to_string())
}

/// A skill as the editor saves it, keeping every frontmatter key of `original` the editor doesn't
/// own (`license`, `metadata`, `allowed-tools`, …) exactly as written.
pub fn compose(original: Option<&str>, name: &str, description: &str, manual: bool, hint: &str, body: &str) -> String {
    let hint = hint.trim();
    frontmatter::compose(
        original,
        &[
            ("name", Some(Value::Text(name))),
            ("description", Some(Value::Text(description))),
            ("disable-model-invocation", manual.then_some(Value::True)),
            ("argument-hint", (!hint.is_empty()).then_some(Value::Text(hint))),
        ],
        body,
    )
}

/// A skill file as the editor shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub name: String,
    pub description: String,
    pub manual: bool,
    pub hint: String,
    pub body: String,
}

pub fn read_document(file: &Path) -> Result<Document, String> {
    let text = fs::read_to_string(file).map_err(|error| format!("Could not read the skill: {error}"))?;
    let (yaml, body) = frontmatter::split(&text);
    let fields = Fields::parse(yaml.as_deref())?;
    let folder = file.parent().and_then(|dir| dir.file_name()).map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Document {
        name: fields.text("name").unwrap_or(folder),
        description: fields.text("description").map(|text| text.trim().to_string()).unwrap_or_default(),
        manual: fields.flag("disable-model-invocation"),
        hint: fields.hint("argument-hint").unwrap_or_default(),
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn skill_text(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\nDo the {name} thing.\n")
    }

    fn root(path: &Path, label: &str) -> Root {
        Root { path: path.to_path_buf(), label: label.to_string() }
    }

    #[test]
    fn finds_skill_folders_and_loose_files() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write(&base.join("pdf/SKILL.md"), &skill_text("pdf", "PDF forms"));
        // A skill folder is not searched further.
        write(&base.join("pdf/nested/SKILL.md"), &skill_text("hidden", "Never loads"));
        write(&base.join("group/deep/SKILL.md"), "---\ndescription: Named by its folder\n---\nBody");
        write(&base.join("loose.md"), "---\ndescription: A loose skill\n---\nBody");
        write(&base.join("notes.md"), "No frontmatter, so not a skill.");
        write(&base.join("group/stray.md"), "---\ndescription: Only top-level files count\n---\n");
        write(&base.join(".hidden/SKILL.md"), &skill_text("dot", "Skipped"));
        write(&base.join("node_modules/x/SKILL.md"), &skill_text("dep", "Skipped"));
        write(&base.join("broken/SKILL.md"), "---\nname: broken\n---\nNo description.");

        let (skills, problems) = scan_root(&root(base, "Your skills"));
        let names: Vec<&str> = skills.iter().map(|skill| skill.name.as_str()).collect();
        assert_eq!(names, ["deep", "loose", "pdf"]);
        assert_eq!(skills[2].base_dir, base.join("pdf"));
        assert_eq!(skills[2].source, "Your skills");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].file.ends_with("broken/SKILL.md"));
    }

    #[test]
    fn first_root_wins_and_switching_off_lets_the_next_load() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        write(&a.join("pdf/SKILL.md"), &skill_text("pdf", "Mine"));
        write(&b.join("pdf/SKILL.md"), &skill_text("pdf", "Theirs"));
        let roots = [root(&a, "Your skills"), root(&b, "Claude Code")];

        let catalog = load(&roots, &HashSet::new());
        assert_eq!(catalog.skills.len(), 1);
        assert_eq!(catalog.skills[0].description, "Mine");
        assert_eq!(catalog.shadowed.len(), 1);
        assert_eq!(catalog.shadowed[0].by, "Your skills");

        let disabled = HashSet::from([a.join("pdf/SKILL.md")]);
        let catalog = load(&roots, &disabled);
        assert_eq!(catalog.skills[0].description, "Theirs");
        assert!(catalog.shadowed.is_empty());
    }

    #[test]
    fn the_same_file_through_a_symlink_loads_once() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        write(&a.join("pdf/SKILL.md"), &skill_text("pdf", "Mine"));
        fs::create_dir_all(&b).unwrap();
        std::os::unix::fs::symlink(a.join("pdf"), b.join("pdf")).unwrap();
        // A loop back to the root must not hang the walk.
        std::os::unix::fs::symlink(&b, b.join("again")).unwrap();
        let catalog = load(&[root(&a, "A"), root(&b, "B")], &HashSet::new());
        assert_eq!(catalog.skills.len(), 1);
        assert!(catalog.shadowed.is_empty());
    }

    #[test]
    fn the_index_lists_only_skills_the_model_may_choose() {
        let skill = |name: &str, manual: bool| Skill {
            name: name.into(),
            description: "Use <this> & that".into(),
            file: PathBuf::from(format!("/s/{name}/SKILL.md")),
            base_dir: PathBuf::from(format!("/s/{name}")),
            manual,
            hint: None,
            source: "Your skills".into(),
        };
        assert_eq!(index(&Catalog::default()), None);
        assert_eq!(index(&Catalog { skills: vec![skill("x", true)], ..Default::default() }), None);
        let text = index(&Catalog { skills: vec![skill("pdf", false), skill("manual", true)], ..Default::default() }).unwrap();
        assert!(text.contains("<name>pdf</name>"));
        assert!(text.contains("Use &lt;this&gt; &amp; that"));
        assert!(text.contains("<location>/s/pdf/SKILL.md</location>"));
        assert!(!text.contains("manual"));
    }

    #[test]
    fn expanding_wraps_the_body_and_appends_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("pdf/SKILL.md");
        write(&file, "---\nname: pdf\ndescription: d\nargument-hint: [file]\n---\n\nFill it in.\n");
        let (skills, _) = scan_root(&root(dir.path(), "Your skills"));
        assert_eq!(skills[0].hint.as_deref(), Some("[file]"));
        let base = dir.path().join("pdf");
        assert_eq!(
            expand(&skills[0], "  form.pdf ").unwrap(),
            format!("<skill name=\"pdf\" location=\"{}\">\nReferences are relative to {}.\n\nFill it in.\n</skill>\n\nform.pdf", file.display(), base.display())
        );
        assert!(expand(&skills[0], "").unwrap().ends_with("</skill>"));
        fs::remove_file(&file).unwrap();
        assert!(expand(&skills[0], "").is_err());
    }

    #[test]
    fn the_editor_holds_the_format() {
        assert_eq!(validate_name(" pdf-forms ").unwrap(), "pdf-forms");
        assert!(validate_name("").is_err());
        assert!(validate_name("PDF").is_err());
        assert!(validate_name("-pdf").is_err());
        assert!(validate_name("pdf--forms").is_err());
        assert!(validate_name(&"a".repeat(65)).is_err());
        assert!(validate_description(" ").is_err());
        assert!(validate_description(&"a".repeat(1025)).is_err());
        assert!(validate_hint(&"a".repeat(257)).is_err());
    }

    #[test]
    fn a_saved_skill_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("pdf/SKILL.md");
        write(&file, &compose(None, "pdf", "Fill: forms", true, "[file]", "Body"));
        let document = read_document(&file).unwrap();
        assert_eq!(document, Document { name: "pdf".into(), description: "Fill: forms".into(), manual: true, hint: "[file]".into(), body: "Body".into() });
    }
}
