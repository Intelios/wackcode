//! A Markdown file's YAML frontmatter, for skills and command templates.
//!
//! - Reading parses the whole of YAML (`yaml-rust2`): skills that other tools wrote use folded
//!   descriptions, quoted scalars, lists and nested `metadata`, which memory's line reader would
//!   get wrong. Memory keeps that simpler reader for its own files.
//! - Writing never re-serialises. [`compose`] keeps the original YAML text and replaces only the
//!   top-level blocks of the keys the editor owns, so every other key, comment and the key order
//!   survive an edit exactly as written (version 2's `skills::compose`).

use yaml_rust2::{Yaml, YamlLoader};

use crate::memory::store::yaml_scalar;

/// A file split into its raw YAML (without the `---` fences) and its trimmed body. `None` for
/// the YAML when the file has no frontmatter; then the whole text is body.
pub fn split(text: &str) -> (Option<String>, String) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).replace("\r\n", "\n").replace('\r', "\n");
    if !text.starts_with("---") {
        return (None, text.trim().to_string());
    }
    let Some(end) = text[3..].find("\n---").map(|at| at + 3) else {
        return (None, text.trim().to_string());
    };
    let yaml = text.get(4..end).unwrap_or("").to_string();
    let rest = &text[end + 4..];
    // The closing fence's own line ends at the first newline after it.
    let body = rest.split_once('\n').map_or("", |(_, body)| body);
    (Some(yaml), body.trim().to_string())
}

/// The parsed frontmatter of one file. Missing or unparsable YAML reads as empty, and the caller
/// decides what a missing key means.
#[derive(Debug)]
pub struct Fields(Yaml);

impl Fields {
    /// `Err` carries the parser's message, for a skill file's "Not loaded" reason.
    pub fn parse(yaml: Option<&str>) -> Result<Self, String> {
        let Some(yaml) = yaml.filter(|yaml| !yaml.trim().is_empty()) else { return Ok(Self(Yaml::Null)) };
        let mut docs = YamlLoader::load_from_str(yaml).map_err(|error| format!("Its frontmatter is not valid YAML: {error}"))?;
        Ok(Self(if docs.is_empty() { Yaml::Null } else { docs.swap_remove(0) }))
    }

    fn get(&self, key: &str) -> &Yaml {
        match &self.0 {
            Yaml::Hash(_) => &self.0[key],
            _ => &Yaml::BadValue,
        }
    }

    /// A string value. Numbers read as their text, so `name: 2048` still names the skill.
    pub fn text(&self, key: &str) -> Option<String> {
        match self.get(key) {
            Yaml::String(value) | Yaml::Real(value) => Some(value.clone()),
            Yaml::Integer(value) => Some(value.to_string()),
            _ => None,
        }
    }

    pub fn flag(&self, key: &str) -> bool {
        matches!(self.get(key), Yaml::Boolean(true))
    }

    /// A list of words, written as a YAML list or as one comma-separated line (`tools: Read,
    /// Grep`, as other tools' agent files have it). `None` when the key is missing.
    pub fn list(&self, key: &str) -> Option<Vec<String>> {
        let items: Vec<String> = match self.get(key) {
            Yaml::Array(items) => items
                .iter()
                .filter_map(|item| match item {
                    Yaml::String(value) | Yaml::Real(value) => Some(value.clone()),
                    Yaml::Integer(value) => Some(value.to_string()),
                    _ => None,
                })
                .collect(),
            _ => self.text(key)?.split(',').map(str::to_string).collect(),
        };
        Some(items.into_iter().map(|item| item.trim().to_string()).filter(|item| !item.is_empty()).collect())
    }

    /// `argument-hint`: an unquoted `[files]` parses as a YAML list, and is rebuilt as the
    /// `"[files]"` the writer meant.
    pub fn hint(&self, key: &str) -> Option<String> {
        let hint = match self.get(key) {
            Yaml::Array(items) => {
                let words: Option<Vec<String>> = items
                    .iter()
                    .map(|item| match item {
                        Yaml::String(value) | Yaml::Real(value) => Some(value.clone()),
                        Yaml::Integer(value) => Some(value.to_string()),
                        _ => None,
                    })
                    .collect();
                format!("[{}]", words?.join(" "))
            }
            _ => self.text(key)?,
        };
        let hint = hint.trim();
        (!hint.is_empty()).then(|| hint.to_string())
    }
}

/// A top-level key starts at column 0; everything up to the next one (indented lines, list items,
/// block scalars, comments) belongs to it.
fn top_level_key(line: &str) -> Option<&str> {
    let first = line.chars().next()?;
    if first.is_whitespace() || first == '#' || first == '-' {
        return None;
    }
    let (key, rest) = if first == '"' || first == '\'' {
        let close = line[1..].find(first)? + 1;
        (&line[1..close], line[close + 1..].trim_start())
    } else {
        let colon = line.find(':')?;
        (line[..colon].trim_end(), &line[colon..])
    };
    let after = rest.strip_prefix(':')?;
    (after.is_empty() || after.starts_with([' ', '\t'])).then_some(key)
}

/// The YAML's lines minus the blocks of `owned` keys.
fn other_keys(yaml: &str, owned: &[&str]) -> Vec<String> {
    let mut kept = Vec::new();
    let mut skipping = false;
    for line in yaml.lines() {
        if let Some(key) = top_level_key(line) {
            skipping = owned.contains(&key);
        }
        if !skipping {
            kept.push(line.to_string());
        }
    }
    while kept.last().is_some_and(|line| line.trim().is_empty()) {
        kept.pop();
    }
    kept
}

/// A file with `fields` first (a `None` value writes no key), then every other key of
/// `original`'s frontmatter as it was written, then the body.
pub fn compose(original: Option<&str>, fields: &[(&str, Option<Value<'_>>)], body: &str) -> String {
    let owned: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
    let kept = original.and_then(|text| split(text).0).map(|yaml| other_keys(&yaml, &owned)).unwrap_or_default();
    let mut out = String::from("---\n");
    for (key, value) in fields {
        match value {
            Some(Value::Text(text)) => out.push_str(&format!("{key}: {}\n", yaml_scalar(text))),
            Some(Value::True) => out.push_str(&format!("{key}: true\n")),
            Some(Value::Raw(yaml)) => out.push_str(&format!("{key}: {yaml}\n")),
            None => {}
        }
    }
    for line in kept {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("---\n");
    let body = body.trim();
    // With nothing to say above the body, the file is the body alone.
    if out == "---\n---\n" {
        return format!("{body}\n");
    }
    if !body.is_empty() {
        out.push('\n');
        out.push_str(body);
        out.push('\n');
    }
    out
}

/// One value [`compose`] writes.
pub enum Value<'a> {
    Text(&'a str),
    True,
    /// YAML written as it is: a number, or a flow list the caller built.
    Raw(&'a str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_fences_bom_and_crlf() {
        let (yaml, body) = split("\u{feff}---\r\nname: pdf\r\n---\r\n\r\nUse it.\r\n");
        assert_eq!(yaml.as_deref(), Some("name: pdf"));
        assert_eq!(body, "Use it.");
        assert_eq!(split("No frontmatter."), (None, "No frontmatter.".to_string()));
        assert_eq!(split("---\nname: x\nunclosed"), (None, "---\nname: x\nunclosed".to_string()));
    }

    #[test]
    fn reads_the_yaml_other_tools_write() {
        let (yaml, _) = split(
            "---\nname: pdf\ndescription: >\n  Fill and merge\n  PDF forms.\nlicense: MIT\nmetadata:\n  author: someone\nallowed-tools: [Read, Bash]\ndisable-model-invocation: true\nargument-hint: [files]\n---\nBody",
        );
        let fields = Fields::parse(yaml.as_deref()).unwrap();
        assert_eq!(fields.text("name").as_deref(), Some("pdf"));
        assert_eq!(fields.text("description").as_deref(), Some("Fill and merge PDF forms.\n"));
        assert!(fields.flag("disable-model-invocation"));
        assert_eq!(fields.hint("argument-hint").as_deref(), Some("[files]"));
        assert_eq!(fields.text("missing"), None);
        assert!(!fields.flag("missing"));
    }

    #[test]
    fn quoted_hints_and_bad_yaml() {
        let fields = Fields::parse(Some("argument-hint: \"<file> [more]\"")).unwrap();
        assert_eq!(fields.hint("argument-hint").as_deref(), Some("<file> [more]"));
        assert!(Fields::parse(Some("name: [unclosed")).is_err());
        assert_eq!(Fields::parse(Some("just a string")).unwrap().text("name"), None);
    }

    #[test]
    fn no_keys_means_no_frontmatter() {
        assert_eq!(compose(None, &[("description", None)], "Sort the imports in $1."), "Sort the imports in $1.\n");
    }

    #[test]
    fn compose_keeps_every_key_it_does_not_own() {
        let original = "---\nname: old\nlicense: MIT\n# a comment\ndescription: |\n  Old\n  text\nmetadata:\n  author: someone\nargument-hint: x\n---\nOld body\n";
        let text = compose(
            Some(original),
            &[
                ("name", Some(Value::Text("pdf"))),
                ("description", Some(Value::Text("Fill: forms"))),
                ("disable-model-invocation", Some(Value::True)),
                ("argument-hint", None),
            ],
            "  New body  ",
        );
        assert_eq!(
            text,
            "---\nname: pdf\ndescription: \"Fill: forms\"\ndisable-model-invocation: true\nlicense: MIT\n# a comment\nmetadata:\n  author: someone\n---\n\nNew body\n"
        );
        // What it wrote reads back.
        let (yaml, body) = split(&text);
        let fields = Fields::parse(yaml.as_deref()).unwrap();
        assert_eq!(fields.text("description").as_deref(), Some("Fill: forms"));
        assert_eq!(fields.hint("argument-hint"), None);
        assert_eq!(body, "New body");
    }
}
