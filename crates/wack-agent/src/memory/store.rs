//! The memory note format, and every read and write of note files. The tools and Settings ›
//! Memory both go through this module, so the two can never disagree about a file.
//!
//! One note per `<type>_<slug>.md` file:
//!
//! ```text
//! ---
//! type: feedback
//! title: Run the tests after protocol changes
//! description: Protocol edits need cargo test
//! modified: 2026-09-28T10:12:00Z
//! ---
//!
//! Body markdown…
//! ```
//!
//! - A note's *name* is its filename without `.md`: the stable id `memory_recall` and
//!   `memory_forget` take. It is checked by [`is_note_name`] before it becomes a path, so a name
//!   can never lead out of the memory folder or into `.trash`.
//! - The index is always generated from the files (there is no MEMORY.md to maintain), so it can
//!   never drift from what is on disk.
//! - Files starting with a dot (staging files) and the `.trash` folder are never read.
//! - Writes stage a dot file and rename it into place, so a half-written note is never read.
//!
//! Ported from version 2's `worker/src/builtin/memory/store.ts` and `src-tauri/src/memory.rs`,
//! which disagreed in places; this one implementation keeps the worker's rules (and its 50,000
//! character body cap) and Settings' validation sentences.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

/// The four kinds of note, in display order: the index lists them grouped by kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    User,
    Feedback,
    Project,
    Reference,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::User, Kind::Feedback, Kind::Project, Kind::Reference];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::User => "user",
            Kind::Feedback => "feedback",
            Kind::Project => "project",
            Kind::Reference => "reference",
        }
    }

    pub fn parse(text: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.as_str() == text)
    }
}

pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_DESCRIPTION_CHARS: usize = 300;
pub const MAX_BODY_CHARS: usize = 50_000;
pub const MAX_NAME_CHARS: usize = 80;
/// Keeps a runaway project from quietly growing past what a system prompt should carry.
const INDEX_MAX_ENTRIES: usize = 200;
const INDEX_MAX_CHARS: usize = 25_000;
/// Where `memory_forget` moves notes: out of the index and Settings, but recoverable.
pub const TRASH_DIR: &str = ".trash";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// The filename without `.md`, like `feedback_run-tests`.
    pub name: String,
    pub kind: Kind,
    pub title: String,
    pub description: String,
    /// ISO 8601 (UTC), written at every save so a stale fact can be told from a current one.
    pub modified: Option<String>,
    pub body: String,
}

/// The ASCII letter an accented Latin letter is written with, so "déjà" slugs to "deja" rather
/// than "d-j". Covers Latin-1 and Latin Extended-A, where nearly every title's accents live.
fn fold(character: char) -> Option<&'static str> {
    Some(match character {
        'À'..='Å' | 'à'..='å' | 'Ā'..='ą' => "a",
        'Æ' | 'æ' => "ae",
        'Ç' | 'ç' | 'Ć'..='č' => "c",
        'Ď'..='đ' | 'Ð' | 'ð' => "d",
        'È'..='Ë' | 'è'..='ë' | 'Ē'..='ě' => "e",
        'Ĝ'..='ģ' => "g",
        'Ĥ'..='ħ' => "h",
        'Ì'..='Ï' | 'ì'..='ï' | 'Ĩ'..='ı' => "i",
        'Ĵ' | 'ĵ' => "j",
        'Ķ' | 'ķ' => "k",
        'Ĺ'..='ł' => "l",
        'Ñ' | 'ñ' | 'Ń'..='ŋ' => "n",
        'Ò'..='Ö' | 'Ø' | 'ò'..='ö' | 'ø' | 'Ō'..='ő' => "o",
        'Œ' | 'œ' => "oe",
        'Ŕ'..='ř' => "r",
        'Ś'..='š' => "s",
        'ß' => "ss",
        'Ţ'..='ŧ' => "t",
        'Ù'..='Ü' | 'ù'..='ü' | 'Ũ'..='ų' => "u",
        'Ŵ' | 'ŵ' => "w",
        'Ý' | 'ý' | 'ÿ' | 'Ŷ' | 'ŷ' | 'Ÿ' => "y",
        'Ź'..='ž' => "z",
        'Þ' | 'þ' => "th",
        _ => return None,
    })
}

/// Lowercase letters, digits and single hyphens, at most 64 characters; `note` when nothing is
/// left.
pub fn slugify(text: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for character in text.chars() {
        let lower = character.to_ascii_lowercase();
        let piece: Option<&str> = if lower.is_ascii_alphanumeric() { None } else { fold(character) };
        if lower.is_ascii_alphanumeric() || piece.is_some() {
            if dash && !slug.is_empty() {
                slug.push('-');
            }
            dash = false;
            match piece {
                Some(piece) => slug.push_str(piece),
                None => slug.push(lower),
            }
        } else if !('\u{300}'..='\u{36f}').contains(&character) {
            dash = true;
        }
    }
    slug.truncate(64);
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() { "note".into() } else { slug.to_string() }
}

pub fn note_name(kind: Kind, title: &str) -> String {
    format!("{}_{}", kind.as_str(), slugify(title))
}

/// Names a note file may carry: `^[a-z0-9][a-z0-9_-]{0,79}$`. Every name that becomes a path is
/// checked here first, since a `/`, `..` or leading dot would land outside the folder or in the
/// trash.
pub fn is_note_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_NAME_CHARS
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes.iter().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-' || *byte == b'_')
}

/// A frontmatter value: plain when YAML would read it back unchanged, otherwise a JSON string
/// (whose escapes YAML also accepts).
pub fn yaml_scalar(value: &str) -> String {
    let plain = value.chars().next().is_some_and(|first| first.is_ascii_alphabetic())
        && !value.ends_with(' ')
        && !value.ends_with(':')
        && !value.contains(": ")
        && !value.contains(" #")
        && !value.chars().any(|character| character.is_control())
        && !["true", "false", "yes", "no", "on", "off", "null", "y", "n"].contains(&value.to_lowercase().as_str());
    if plain { value.to_string() } else { serde_json::Value::String(value.to_string()).to_string() }
}

fn parse_scalar(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        return match serde_json::from_str::<String>(trimmed) {
            Ok(parsed) => parsed,
            Err(_) => trimmed[1..trimmed.len() - 1].to_string(),
        };
    }
    trimmed.to_string()
}

/// `---`, `key: value` lines, `---`, then the body. Text with no frontmatter is all body.
pub fn split_frontmatter(text: &str) -> (Vec<(String, String)>, String) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).replace("\r\n", "\n").replace('\r', "\n");
    let mut fields = Vec::new();
    if !text.starts_with("---") {
        return (fields, text.trim().to_string());
    }
    let Some(end) = text[3..].find("\n---").map(|at| at + 3) else {
        return (fields, text.trim().to_string());
    };
    for line in text.get(4..end).unwrap_or("").split('\n') {
        let Some((key, value)) = line.split_once(':') else { continue };
        let valid = key.chars().next().is_some_and(|first| first.is_ascii_alphabetic())
            && key.chars().all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-');
        if valid {
            fields.push((key.to_string(), parse_scalar(value)));
        }
    }
    (fields, text[end + 4..].trim().to_string())
}

/// One note's file text.
pub fn compose(kind: Kind, title: &str, description: &str, modified: Option<&str>, body: &str) -> String {
    let mut out = format!(
        "---\ntype: {}\ntitle: {}\ndescription: {}\n",
        kind.as_str(),
        yaml_scalar(title),
        yaml_scalar(description)
    );
    if let Some(modified) = modified {
        out.push_str(&format!("modified: {modified}\n"));
    }
    out.push_str("---\n\n");
    out.push_str(body.trim());
    out.push('\n');
    out
}

/// One file as a note. No frontmatter, no title or an unknown type means it is not a note; no
/// type at all means `project`.
pub fn parse(name: &str, text: &str) -> Option<Note> {
    let (fields, body) = split_frontmatter(text);
    let get = |key: &str| fields.iter().find(|(candidate, _)| candidate == key).map(|(_, value)| value.trim().to_string());
    let title = get("title").filter(|title| !title.is_empty())?;
    let kind = Kind::parse(&get("type").unwrap_or_else(|| "project".into()))?;
    Some(Note {
        name: name.to_string(),
        kind,
        title,
        description: get("description").unwrap_or_default(),
        modified: get("modified").filter(|modified| !modified.is_empty()),
        body,
    })
}

/// Every note in `root`, by name. Dot files and `.trash` never load; a missing folder is empty.
pub fn scan(root: &Path) -> Vec<Note> {
    let Ok(entries) = fs::read_dir(root) else { return Vec::new() };
    let mut notes: Vec<Note> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            let name = file.strip_suffix(".md").filter(|_| !file.starts_with('.'))?;
            parse(name, &fs::read_to_string(entry.path()).ok()?)
        })
        .collect();
    notes.sort_by(|a, b| a.name.cmp(&b.name));
    notes
}

/// The section the system prompt carries: one line per note, grouped by kind then title, which
/// the model scans to decide what to recall. Bodies never enter it. `None` with no notes.
pub fn index(notes: &[Note]) -> Option<String> {
    if notes.is_empty() {
        return None;
    }
    let mut sorted: Vec<&Note> = notes.iter().collect();
    sorted.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase())));
    let mut lines: Vec<String> = sorted
        .iter()
        .map(|note| {
            let described = if note.description.is_empty() { String::new() } else { format!(" — {}", note.description) };
            format!("- [{}] {}{} (name: {})", note.kind.as_str(), note.title, described, note.name)
        })
        .collect();
    let joined = |lines: &[String]| lines.iter().map(|line| line.chars().count()).sum::<usize>() + lines.len().saturating_sub(1);
    while !lines.is_empty() && (lines.len() > INDEX_MAX_ENTRIES || joined(&lines) > INDEX_MAX_CHARS) {
        lines.pop();
    }
    let hidden = sorted.len() - lines.len();
    let mut out = vec![
        "## Project memory".to_string(),
        String::new(),
        "Notes saved from earlier conversations in this project, one line each. When one is relevant to the current request, read its full text with memory_recall; otherwise leave it alone.".to_string(),
        String::new(),
    ];
    out.extend(lines);
    if hidden > 0 {
        out.push(String::new());
        out.push(format!(
            "[… {hidden} more notes are not listed; recall them by name if you know it, and prune stale notes with memory_forget.]"
        ));
    }
    out.push(String::new());
    out.push("Keep memory small and current: memory_save to add or update a note, memory_forget to drop one that no longer holds.".to_string());
    Some(out.join("\n"))
}

/// A name no file holds yet: `base`, then `base-2`, `base-3`, … after a collision.
pub fn available_name(root: &Path, base: &str) -> String {
    let held = |name: &str| fs::symlink_metadata(root.join(format!("{name}.md"))).is_ok();
    if !held(base) {
        return base.to_string();
    }
    (2..).map(|suffix| format!("{base}-{suffix}")).find(|candidate| !held(candidate)).unwrap()
}

/// Now, as ISO 8601 in UTC, to the second.
pub fn now_iso() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(0) as libc::time_t;
    // SAFETY: `gmtime_r` only writes into the `tm` it is handed, which lives on this stack.
    let time = unsafe {
        let mut time: libc::tm = std::mem::zeroed();
        libc::gmtime_r(&now, &mut time);
        time
    };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        time.tm_year + 1900,
        time.tm_mon + 1,
        time.tm_mday,
        time.tm_hour,
        time.tm_min,
        time.tm_sec
    )
}

/// Writes `text` to `file` through a dot-file in the same folder, so a reader never sees half.
fn write_atomic(file: &Path, text: &str) -> Result<(), String> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let folder = file.parent().ok_or("A memory needs a folder.")?;
    fs::create_dir_all(folder).map_err(|error| format!("Could not create the memory folder: {error}"))?;
    let stem = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let staging = folder.join(format!(
        ".{stem}.{}-{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&staging, text).map_err(|error| format!("Could not save the memory: {error}"))?;
    fs::rename(&staging, file).map_err(|error| {
        let _ = fs::remove_file(&staging);
        format!("Could not save the memory: {error}")
    })
}

/// Writes one note, stamping it with the time now. Returns the file.
pub fn write(root: &Path, name: &str, kind: Kind, title: &str, description: &str, body: &str) -> Result<PathBuf, String> {
    if !is_note_name(name) {
        return Err(NAME_RULE.into());
    }
    let file = root.join(format!("{name}.md"));
    write_atomic(&file, &compose(kind, title, description, Some(&now_iso()), body))?;
    Ok(file)
}

/// Moves a note into the folder's `.trash`, numbered if one of the same name is there already.
pub fn trash(root: &Path, name: &str) -> Result<(), String> {
    let folder = root.join(TRASH_DIR);
    fs::create_dir_all(&folder).map_err(|error| format!("Could not create the memory trash: {error}"))?;
    let mut target = folder.join(format!("{name}.md"));
    let mut suffix = 2;
    while fs::symlink_metadata(&target).is_ok() {
        target = folder.join(format!("{name}-{suffix}.md"));
        suffix += 1;
    }
    fs::rename(root.join(format!("{name}.md")), target).map_err(|error| format!("Could not forget the memory: {error}"))
}

pub const NAME_RULE: &str = "Use only lowercase letters, numbers, hyphens and underscores, like feedback_run-tests.";

/// A note as Settings' editor hands it over, before it is checked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub name: String,
    pub kind: Option<Kind>,
    pub title: String,
    pub description: String,
    pub body: String,
}

/// What is wrong with a draft, as the sentence Settings shows; `None` when it can be saved.
pub fn draft_issue(draft: &Draft) -> Option<String> {
    let name = draft.name.trim();
    Some(if name.is_empty() {
        "Give the memory a name.".into()
    } else if name.chars().count() > MAX_NAME_CHARS {
        format!("A memory name can be at most {MAX_NAME_CHARS} characters.")
    } else if !is_note_name(name) {
        NAME_RULE.into()
    } else if draft.kind.is_none() {
        "The memory type must be one of user, feedback, project, reference.".into()
    } else if draft.title.trim().is_empty() {
        "Give the memory a title.".into()
    } else if draft.title.trim().chars().count() > MAX_TITLE_CHARS {
        format!("A memory title can be at most {MAX_TITLE_CHARS} characters.")
    } else if draft.description.trim().chars().count() > MAX_DESCRIPTION_CHARS {
        format!("A memory description can be at most {MAX_DESCRIPTION_CHARS} characters.")
    } else if draft.body.trim().is_empty() {
        "The memory needs something to remember.".into()
    } else if draft.body.trim().chars().count() > MAX_BODY_CHARS {
        format!("A memory can be at most {MAX_BODY_CHARS} characters.")
    } else {
        return None;
    })
}

/// Saves a draft from Settings into `root`. `previous` is the note being edited, if any: saving
/// under a new name renames its file. Creating over an existing name is refused.
pub fn save_draft(root: &Path, previous: Option<&str>, draft: &Draft) -> Result<String, String> {
    if let Some(issue) = draft_issue(draft) {
        return Err(issue);
    }
    let name = draft.name.trim();
    let held = fs::symlink_metadata(root.join(format!("{name}.md"))).is_ok();
    if held && previous != Some(name) {
        return Err(format!("A memory named {name} already exists."));
    }
    if let Some(previous) = previous.filter(|previous| *previous != name) {
        if !is_note_name(previous) {
            return Err(NAME_RULE.into());
        }
        let old = root.join(format!("{previous}.md"));
        fs::rename(&old, root.join(format!("{name}.md")))
            .map_err(|error| format!("The memory's file could not be renamed: {error}"))?;
    }
    let kind = draft.kind.expect("checked by draft_issue");
    write(root, name, kind, draft.title.trim(), draft.description.trim(), draft.body.trim())?;
    Ok(name.to_string())
}

/// One note by name, for the editor.
pub fn read(root: &Path, name: &str) -> Option<Note> {
    if !is_note_name(name) {
        return None;
    }
    parse(name, &fs::read_to_string(root.join(format!("{name}.md"))).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(name: &str, kind: Kind, title: &str, description: &str) -> Note {
        Note {
            name: name.into(),
            kind,
            title: title.into(),
            description: description.into(),
            modified: Some("2026-09-28T10:12:00Z".into()),
            body: "Any change to protocol.rs needs `cargo test` before review.".into(),
        }
    }

    #[test]
    fn scalars_are_plain_only_when_safe() {
        assert_eq!(yaml_scalar("Run the tests"), "Run the tests");
        assert_eq!(yaml_scalar("say: hi"), "\"say: hi\"");
        assert_eq!(yaml_scalar("A note #1"), "\"A note #1\"");
        assert_eq!(yaml_scalar("trailing "), "\"trailing \"");
        assert_eq!(yaml_scalar("true"), "\"true\"");
        assert_eq!(yaml_scalar(""), "\"\"");
        assert_eq!(yaml_scalar("123abc"), "\"123abc\"");
        assert_eq!(yaml_scalar("abc123"), "abc123");
    }

    #[test]
    fn titles_slug_into_names() {
        assert_eq!(note_name(Kind::Feedback, "Run worker tests after protocol changes!"), "feedback_run-worker-tests-after-protocol-changes");
        assert_eq!(note_name(Kind::Project, "  Ünicode — déjà vu  "), "project_unicode-deja-vu");
        assert_eq!(note_name(Kind::User, "???"), "user_note");
        assert_eq!(slugify("Fix: the/imaginary `bug` #42"), "fix-the-imaginary-bug-42");
        assert_eq!(slugify("————"), "note");
        assert_eq!(slugify(&"a".repeat(100)).len(), 64);
    }

    #[test]
    fn names_are_plain_filenames() {
        assert!(is_note_name("feedback_run-worker-tests"));
        assert!(is_note_name("project_shipping-checklist-2"));
        for bad in ["", "Upper", "-leading", "_leading", "has space", "a/b", "../escape", "..", ".hidden", "é-note"] {
            assert!(!is_note_name(bad), "{bad}");
        }
        assert!(!is_note_name(&"a".repeat(81)));
    }

    #[test]
    fn a_note_round_trips() {
        let original = note("feedback_run-tests", Kind::Feedback, "Run tests: always", "Protocol edits need cargo test");
        let text = compose(original.kind, &original.title, &original.description, original.modified.as_deref(), &original.body);
        assert_eq!(parse(&original.name, &text), Some(original));
    }

    #[test]
    fn frontmatter_parses_quoted_values() {
        let text = "---\r\ntype: user\r\ntitle: \"Prefers: dark UI, no emojis \\u00e9\"\r\ndescription: Tells it how to answer\r\n---\r\nBody text";
        let parsed = parse("user_prefers", text).unwrap();
        assert_eq!(parsed.title, "Prefers: dark UI, no emojis é");
        assert_eq!(parsed.body, "Body text");
    }

    #[test]
    fn files_without_a_title_or_with_an_unknown_type_are_not_notes() {
        assert!(parse("x", "Just a body, no frontmatter.").is_none());
        assert!(parse("x", "---\nbody only, no title\n---\nnothing").is_none());
        assert!(parse("x", "---\ntype: diary\ntitle: Not a memory type\n---\nbody").is_none());
        assert_eq!(parse("x", "---\ntitle: Untyped\n---\nbody").unwrap().kind, Kind::Project);
    }

    #[test]
    fn the_index_lists_one_line_per_note_by_kind_then_title() {
        let index = index(&[
            note("project_shipping", Kind::Project, "Shipping checklist", "Before a release"),
            note("user_prefers", Kind::User, "Prefers terse answers", ""),
        ])
        .unwrap();
        let lines: Vec<&str> = index.lines().filter(|line| line.starts_with("- ")).collect();
        assert_eq!(lines, [
            "- [user] Prefers terse answers (name: user_prefers)",
            "- [project] Shipping checklist — Before a release (name: project_shipping)",
        ]);
        assert!(index.starts_with("## Project memory\n\nNotes saved from earlier conversations in this project"));
        assert!(index.ends_with("memory_forget to drop one that no longer holds."));
        assert!(!index.contains("cargo test"), "bodies never enter the index");
    }

    #[test]
    fn the_index_is_none_when_empty_and_capped_when_large() {
        assert!(index(&[]).is_none());
        let many: Vec<Note> = (0..300)
            .map(|at| note(&format!("project_note-{at}"), Kind::Project, &format!("Note {at} {}", "x".repeat(90)), "d"))
            .collect();
        let index = index(&many).unwrap();
        assert!(index.contains("more notes are not listed"));
        assert!(index.chars().count() < 30_000);
    }

    #[test]
    fn scanning_skips_dot_files_trash_and_non_notes() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        write(root, "feedback_run-tests", Kind::Feedback, "Run tests", "", "Body").unwrap();
        fs::write(root.join(".feedback_x.1-1.tmp"), compose(Kind::User, "Staging", "", None, "b")).unwrap();
        fs::write(root.join(".hidden.md"), compose(Kind::User, "Hidden", "", None, "b")).unwrap();
        fs::write(root.join("notes.txt"), "not markdown").unwrap();
        fs::write(root.join("plain.md"), "no frontmatter").unwrap();
        fs::write(root.join("origin.json"), "{}").unwrap();
        trash_fixture(root);
        let names: Vec<String> = scan(root).into_iter().map(|note| note.name).collect();
        assert_eq!(names, ["feedback_run-tests"]);
        assert!(scan(&root.join("missing")).is_empty());
    }

    fn trash_fixture(root: &Path) {
        fs::create_dir_all(root.join(TRASH_DIR)).unwrap();
        fs::write(root.join(TRASH_DIR).join("user_old.md"), compose(Kind::User, "Old", "", None, "b")).unwrap();
    }

    #[test]
    fn names_collide_into_numbered_ones_and_trash_keeps_both() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        assert_eq!(available_name(root, "user_x"), "user_x");
        write(root, "user_x", Kind::User, "X", "", "one").unwrap();
        assert_eq!(available_name(root, "user_x"), "user_x-2");
        write(root, "user_x-2", Kind::User, "X", "", "two").unwrap();
        assert_eq!(available_name(root, "user_x"), "user_x-3");

        trash(root, "user_x").unwrap();
        write(root, "user_x", Kind::User, "X", "", "again").unwrap();
        trash(root, "user_x").unwrap();
        assert!(root.join(".trash/user_x.md").exists());
        assert!(root.join(".trash/user_x-2.md").exists());
        assert_eq!(scan(root).len(), 1);
    }

    #[test]
    fn writes_refuse_bad_names() {
        let folder = tempfile::tempdir().unwrap();
        for bad in ["../escape", "a/b", ".hidden", "Has Space", "UPPER"] {
            assert_eq!(write(folder.path(), bad, Kind::User, "T", "", "b").unwrap_err(), NAME_RULE);
        }
        assert!(fs::read_dir(folder.path()).unwrap().next().is_none());
    }

    #[test]
    fn timestamps_are_iso_utc() {
        let now = now_iso();
        assert_eq!(now.len(), 20);
        assert!(now.ends_with('Z') && now.as_bytes()[10] == b'T');
    }

    #[test]
    fn drafts_are_checked_created_and_renamed() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        let mut draft = Draft { name: "user_tabs".into(), kind: Some(Kind::User), title: "Tabs".into(), description: "".into(), body: "Prefers tabs".into() };
        assert_eq!(draft_issue(&Draft::default()).unwrap(), "Give the memory a name.");
        assert_eq!(draft_issue(&Draft { body: " ".into(), ..draft.clone() }).unwrap(), "The memory needs something to remember.");
        assert_eq!(draft_issue(&Draft { name: "Bad Name".into(), ..draft.clone() }).unwrap(), NAME_RULE);

        assert_eq!(save_draft(root, None, &draft).unwrap(), "user_tabs");
        assert_eq!(save_draft(root, None, &draft).unwrap_err(), "A memory named user_tabs already exists.");
        draft.body = "Prefers tabs, width 4".into();
        save_draft(root, Some("user_tabs"), &draft).unwrap();
        assert_eq!(read(root, "user_tabs").unwrap().body, "Prefers tabs, width 4");

        draft.name = "user_indentation".into();
        save_draft(root, Some("user_tabs"), &draft).unwrap();
        assert!(read(root, "user_tabs").is_none());
        assert_eq!(read(root, "user_indentation").unwrap().title, "Tabs");
        assert_eq!(scan(root).len(), 1);
    }
}
