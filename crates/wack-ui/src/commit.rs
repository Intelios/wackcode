//! Commit milestone rows: `git commit` calls lifted out of the work log and drawn as milestones.
//!
//! Ported from version 2's commit milestone logic (`isGitCommit`, `commitMessageSubject`,
//! `parseCommit`, `CommitWords`). A commit is the one command that lands something on a branch,
//! so it breaks the work group around it and stands on its own in the transcript, as one line
//! like the lines about the work around it: the commit's mark, then what it did.
//! - While running: the mark breathes, `"Committing"` shines, and the subject is read from the
//!   first `-m` (falling back to the command itself when `-m` isn't used, as in
//!   `--amend --no-edit`).
//! - Once git prints its summary line (`[branch hash] subject` and optional stats): the
//!   abbreviated hash in the accent's mono, git's own subject line, and file / `±` stats. If a
//!   later chained command failed (such as `git push`), it says `"failed"` in danger's colour.
//! - If it failed without landing a commit: the mark and `"Couldn't commit"` in danger's colour,
//!   with no hash or redundant `"failed"`.
//! - Clicking opens the command and git's output in a sheet under the line.
//!
//! Shell parsing here is presentation only — it decides what the transcript draws, never what may
//! run — so anything it does not model exactly (command substitution, unknown pre-subcommand flags,
//! non-git chained commands other than `cd`) stays an ordinary command row inside the work log.

use gpui::{ElementId, SharedString, div, prelude::*, px};
use serde_json::Value;

use crate::{
    conversation::{Step, ToolStep},
    icon::Icon,
    motion,
    tags::Tag,
    theme::Theme,
    transcript::Handlers,
    typography::{Type, Typography},
    work::{DETAIL_LINES, background_note, first_line, plural, tail},
};

/// A commit milestone row in the transcript.
#[derive(Clone, Debug, PartialEq)]
pub struct CommitRow {
    /// The tool call's id; toggles the expanded command and output.
    pub key: SharedString,
    /// The trimmed shell command, shown when expanded.
    pub command: SharedString,
    /// The commit message's subject line, or the command's first line when no message is known.
    pub subject: SharedString,
    /// True when `subject` is the raw command rather than a commit message (drawn in mono).
    pub subject_is_command: bool,
    pub state: CommitState,
    pub files: Option<u32>,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    /// Git's output (tailed), when non-empty and not an internal background-move notice.
    pub output: Option<SharedString>,
    pub open: bool,
    /// In a live turn: the mark lands when the row arrives.
    pub live: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitState {
    /// Still executing in a live turn.
    Running,
    /// Moved to the background while running (e.g. a slow hook).
    Background(SharedString),
    /// Ended without a result in a settled turn (stopped / interrupted).
    Stopped,
    /// Git printed a commit summary line; `failed` is true when a later part of the command failed.
    Landed { hash: SharedString, failed: bool },
    /// Succeeded without printing a summary line (e.g. `git commit -q`).
    Quiet,
    /// Failed and no commit landed.
    Failed,
}

impl CommitRow {
    /// What this commit is doing while it runs ("Committing Fix upload"); `None` once settled.
    pub fn activity(&self) -> Option<String> {
        if !matches!(self.state, CommitState::Running) {
            return None;
        }
        Some(if self.subject.is_empty() {
            "Committing".to_string()
        } else {
            format!("Committing {}", self.subject)
        })
    }
}

/// Git's own summary of a commit it made, parsed from `git commit` output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedCommit {
    /// The abbreviated hash exactly as git printed it.
    pub hash: String,
    /// The commit's subject line; may be empty for a message with no subject.
    pub subject: String,
    /// How much the commit changed, when git printed its stats line.
    pub files: Option<u32>,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

/// Whether `step` is a `bash` call that makes a Git commit and should break out of a work group.
pub fn is_commit_step(step: &Step) -> bool {
    matches!(step, Step::Tool(tool) if is_commit_tool(tool))
}

/// Whether `tool` is a `bash` call recognised as a Git commit.
pub fn is_commit_tool(tool: &ToolStep) -> bool {
    tool.name == "bash"
        && tool
            .input
            .as_ref()
            .and_then(|input| input.get("command"))
            .and_then(Value::as_str)
            .is_some_and(is_git_commit)
}

/// Builds a [`CommitRow`] from a `bash` tool step whose command passed [`is_git_commit`].
pub fn from_tool(tool: &ToolStep, live: bool, open: bool) -> CommitRow {
    let command = tool
        .input
        .as_ref()
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let result = tool.result.as_ref();
    let ok = result.filter(|result| !result.is_error).map(|result| result.text.as_str());
    let parsed = result.and_then(|result| parse_commit(&result.text));
    let bg = ok.and_then(background_note);

    let state = match (result, &parsed, bg) {
        (None, _, _) if live => CommitState::Running,
        (None, _, _) => CommitState::Stopped,
        (_, _, Some(how)) => CommitState::Background(how.into()),
        (Some(result), Some(commit), None) => CommitState::Landed {
            hash: commit.hash.clone().into(),
            failed: result.is_error,
        },
        (Some(result), None, None) if result.is_error => CommitState::Failed,
        (Some(_), None, None) => CommitState::Quiet,
    };

    let (subject, subject_is_command) = match &parsed {
        Some(commit) => (commit.subject.clone(), false),
        None => match commit_message_subject(command) {
            Some(subject) => (subject, false),
            None => (first_line(command), true),
        },
    };

    let output = if bg.is_some() {
        None
    } else {
        result
            .map(|result| result.text.trim_end())
            .filter(|text| !text.is_empty() && *text != "(No output.)")
            .map(|text| tail(text, DETAIL_LINES).into())
    };

    CommitRow {
        key: tool.id.clone().into(),
        command: command.trim().to_string().into(),
        subject: subject.into(),
        subject_is_command,
        state,
        files: parsed.as_ref().and_then(|commit| commit.files),
        additions: parsed.as_ref().and_then(|commit| commit.additions),
        deletions: parsed.as_ref().and_then(|commit| commit.deletions),
        output,
        open,
        live,
    }
}

/// A shell command split into its simple commands (at `|`, `&&`, `||`, `;` and newlines), each a
/// list of words with quoting removed. `None` for anything not modelled exactly: command
/// substitution, subshells, backgrounding, input redirection and heredocs, and any output
/// redirection except discarding it (`2>/dev/null`, `2>&1`).
fn split_shell(command: &str) -> Option<Vec<Vec<String>>> {
    let chars: Vec<char> = command.chars().collect();
    let len = chars.len();
    let mut commands: Vec<Vec<String>> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    // Just after `|`, `&&` or `||`: another command must follow, possibly on the next line.
    let mut after_operator = false;

    let append = |word: &mut String, in_word: &mut bool, after_operator: &mut bool, ch: char| {
        word.push(ch);
        *in_word = true;
        *after_operator = false;
    };
    let end_word = |words: &mut Vec<String>, word: &mut String, in_word: &mut bool| {
        if *in_word {
            words.push(std::mem::take(word));
            *in_word = false;
        }
    };
    let end_command = |commands: &mut Vec<Vec<String>>, words: &mut Vec<String>, word: &mut String, in_word: &mut bool| -> bool {
        end_word(words, word, in_word);
        if words.is_empty() {
            return false;
        }
        commands.push(std::mem::take(words));
        true
    };

    let mut index = 0;
    while index < len {
        let ch = chars[index];
        let next = chars.get(index + 1).copied();
        if ch == '\'' {
            let mut cursor = index + 1;
            while cursor < len && chars[cursor] != '\'' {
                word.push(chars[cursor]);
                cursor += 1;
            }
            if cursor >= len {
                return None;
            }
            in_word = true;
            after_operator = false;
            index = cursor + 1;
        } else if ch == '"' {
            let mut cursor = index + 1;
            while cursor < len && chars[cursor] != '"' {
                let inner = chars[cursor];
                if inner == '`' || (inner == '$' && chars.get(cursor + 1) == Some(&'(')) {
                    return None;
                }
                if inner == '\\' && cursor + 1 < len {
                    let escaped = chars[cursor + 1];
                    if matches!(escaped, '"' | '\\' | '$' | '`') {
                        word.push(escaped);
                    } else {
                        word.push('\\');
                        word.push(escaped);
                    }
                    cursor += 2;
                } else {
                    word.push(inner);
                    cursor += 1;
                }
            }
            if cursor >= len {
                return None;
            }
            in_word = true;
            after_operator = false;
            index = cursor + 1;
        } else if ch == '\\' {
            // A backslash-newline continues the line; any other escaped character is literal.
            if next != Some('\n') {
                if let Some(escaped) = next {
                    append(&mut word, &mut in_word, &mut after_operator, escaped);
                } else {
                    in_word = true;
                    after_operator = false;
                }
            }
            index += 2;
        } else if ch == ' ' || ch == '\t' || ch == '\r' {
            end_word(&mut words, &mut word, &mut in_word);
            index += 1;
        } else if ch == '#' && !in_word {
            while index < len && chars[index] != '\n' {
                index += 1;
            }
        } else if ch == '\n' {
            if !after_operator {
                end_command(&mut commands, &mut words, &mut word, &mut in_word);
            }
            index += 1;
        } else if ch == ';' {
            if after_operator {
                return None;
            }
            end_command(&mut commands, &mut words, &mut word, &mut in_word);
            index += 1;
        } else if (ch == '|' && next == Some('|')) || (ch == '&' && next == Some('&')) {
            if !end_command(&mut commands, &mut words, &mut word, &mut in_word) {
                return None;
            }
            after_operator = true;
            index += 2;
        } else if ch == '|' {
            if next == Some('&') || !end_command(&mut commands, &mut words, &mut word, &mut in_word) {
                return None;
            }
            after_operator = true;
            index += 1;
        } else if ch == '>' || (ch == '&' && next == Some('>')) {
            // Only discarding output is allowed. A single-digit fd belongs to the redirection.
            if ch == '>' && in_word && word.len() == 1 && word.as_bytes()[0].is_ascii_digit() {
                word.clear();
                in_word = false;
            }
            let mut cursor = index + if ch == '&' { 2 } else { 1 };
            if chars.get(cursor) == Some(&'>') {
                return None;
            }
            if chars.get(cursor) == Some(&'&') && matches!(chars.get(cursor + 1), Some('0' | '1' | '2')) {
                cursor += 2;
            } else {
                while matches!(chars.get(cursor), Some(' ' | '\t')) {
                    cursor += 1;
                }
                const DEV_NULL: [char; 9] = ['/', 'd', 'e', 'v', '/', 'n', 'u', 'l', 'l'];
                if chars.get(cursor..cursor + DEV_NULL.len()) != Some(&DEV_NULL) {
                    return None;
                }
                cursor += DEV_NULL.len();
            }
            if let Some(&after) = chars.get(cursor)
                && !after.is_whitespace()
                && !matches!(after, ';' | '|' | '&')
            {
                return None;
            }
            end_word(&mut words, &mut word, &mut in_word);
            index = cursor;
        } else if matches!(ch, '<' | '&' | '(' | ')' | '`') || (ch == '$' && next == Some('(')) {
            return None;
        } else {
            append(&mut word, &mut in_word, &mut after_operator, ch);
            index += 1;
        }
    }
    if after_operator {
        return None;
    }
    end_command(&mut commands, &mut words, &mut word, &mut in_word);
    Some(commands)
}

/// A `git` invocation as the subcommand it runs, past git's own `-C` / `-c` leading options, and
/// the index where the subcommand's own words begin. `None` for anything that isn't `git …`.
fn git_invocation(words: &[String]) -> Option<(&str, usize)> {
    if words.first().map(String::as_str) != Some("git") {
        return None;
    }
    let mut index = 1;
    while let Some(word) = words.get(index) {
        if word == "-C" || word == "-c" {
            index += 2;
            continue;
        }
        if word.starts_with("-C") || word.starts_with("-c") {
            index += 1;
            continue;
        }
        return Some((word.as_str(), index + 1));
    }
    None
}

/// A shell command that makes a Git commit: every simple command in it is `git` (a `cd` on the
/// way is fine) and one of them commits — `git commit …`, not `--dry-run`. Anything not modelled
/// exactly (command substitution, env assignments, non-git commands) stays an ordinary command.
pub fn is_git_commit(command: &str) -> bool {
    let Some(commands) = split_shell(command) else {
        return false;
    };
    if commands.is_empty() {
        return false;
    }
    let mut commits = false;
    for words in &commands {
        if words.first().map(String::as_str) == Some("cd") {
            continue;
        }
        let Some((subcommand, next)) = git_invocation(words) else {
            return false;
        };
        if subcommand == "commit" && !words[next..].iter().any(|word| word == "--dry-run") {
            commits = true;
        }
    }
    commits
}

/// The subject line of the message a commit command gives itself: the first `-m` / `--message`
/// after `commit` — only the first, since a later one is the body where the co-author trailer
/// rides. Shown until git's own summary lands; `None` when the command says nothing.
pub fn commit_message_subject(command: &str) -> Option<String> {
    for words in split_shell(command)? {
        let Some((subcommand, next)) = git_invocation(&words) else {
            continue;
        };
        if subcommand != "commit" {
            continue;
        }
        for (offset, word) in words[next..].iter().enumerate() {
            if word == "--" {
                break;
            }
            if word == "-m" || word == "--message" || is_short_message_cluster(word) {
                return message_subject(words.get(next + offset + 1).map(String::as_str).unwrap_or(""));
            }
            if let Some(rest) = word.strip_prefix("--message=") {
                return message_subject(rest);
            }
        }
    }
    None
}

/// Short option cluster ending in `m`, such as `-am` or `-nm`.
fn is_short_message_cluster(word: &str) -> bool {
    word.starts_with('-')
        && !word.starts_with("--")
        && word.len() > 2
        && word.ends_with('m')
        && word[1..].bytes().all(|byte| byte.is_ascii_alphabetic())
}

fn message_subject(message: &str) -> Option<String> {
    let subject = message.lines().next().unwrap_or("").trim();
    (!subject.is_empty()).then(|| subject.to_string())
}

/// What `git commit` printed about the commit it made. The ref may carry spaces (`master
/// (root-commit)`, `detached HEAD`) and the subject may carry brackets of its own, so the hash
/// is the 7–40 lowercase hex word the first closing bracket follows. The LAST summary line wins:
/// a command that commits twice names the newer commit, and its stats are read from the 1–2 lines
/// right after it (past the `Date:` line an amend prints).
pub fn parse_commit(text: &str) -> Option<ParsedCommit> {
    let lines: Vec<&str> = text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).collect();
    for index in (0..lines.len()).rev() {
        let Some((hash, subject)) = parse_commit_line(lines[index]) else {
            continue;
        };
        let mut parsed = ParsedCommit { hash: hash.to_string(), subject: subject.to_string(), files: None, additions: None, deletions: None };
        let end = (index + 3).min(lines.len());
        for &follow in &lines[index + 1..end] {
            if let Some((files, additions, deletions)) = parse_stats_line(follow) {
                parsed.files = Some(files);
                parsed.additions = additions;
                parsed.deletions = deletions;
                break;
            }
        }
        return Some(parsed);
    }
    None
}

/// Matches `^\[([^\]]+) ([0-9a-f]{7,40})\] (.*)$`.
fn parse_commit_line(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix('[')?;
    let close = rest.find(']')?;
    let inside = &rest[..close];
    let after = rest[close + 1..].strip_prefix(' ')?;
    let (branch, hash) = inside.rsplit_once(' ')?;
    if branch.is_empty() || !(7..=40).contains(&hash.len()) || !hash.bytes().all(is_lower_hex) {
        return None;
    }
    Some((hash, after))
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

/// Matches `^ (\d+) files? changed(?:, (\d+) insertions?\(\+\))?(?:, (\d+) deletions?\(-\))?$`.
fn parse_stats_line(line: &str) -> Option<(u32, Option<u32>, Option<u32>)> {
    let rest = line.strip_prefix(' ')?;
    let mut parts = rest.split(", ");
    let first = parts.next()?;
    let files = parse_count_suffix(first, &[" file changed", " files changed"])?;
    let mut additions = None;
    let mut deletions = None;
    if let Some(part) = parts.next() {
        if let Some(count) = parse_count_suffix(part, &[" insertion(+)", " insertions(+)"]) {
            additions = Some(count);
            if let Some(part) = parts.next() {
                deletions = Some(parse_count_suffix(part, &[" deletion(-)", " deletions(-)"])?);
            }
        } else {
            deletions = Some(parse_count_suffix(part, &[" deletion(-)", " deletions(-)"])?);
        }
    }
    if parts.next().is_some() {
        return None;
    }
    Some((files, additions, deletions))
}

fn parse_count_suffix(part: &str, suffixes: &[&str]) -> Option<u32> {
    let digits = suffixes.iter().find_map(|suffix| part.strip_suffix(suffix))?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

// -------------------------------------------------------------------------------------------------
// View

const DETAIL_MAX_HEIGHT: f32 = 320.;

/// A commit as D draws it: one line, like the lines about the work around it, with the commit's
/// mark and hash in the accent (it is the turn's milestone), its subject in the reply's ink and
/// its lines in the diff colours. A refused commit says so in danger's colour. Opens to the
/// command and git's output in a sheet under it.
pub fn view(commit: &CommitRow, theme: &Theme, handlers: &Handlers) -> gpui::AnyElement {
    let failed_only = matches!(commit.state, CommitState::Failed);
    let running = matches!(commit.state, CommitState::Running);
    let hash = match &commit.state {
        CommitState::Landed { hash, .. } => Some(hash.clone()),
        _ => None,
    };

    let mark = div().flex_none().child(Icon::Commit.view(px(14.), if failed_only { theme.danger } else { theme.accent_text }));
    let mark = if running {
        motion::breathing(SharedString::from(format!("{}-breath", commit.key)), mark, handlers.still)
    } else if commit.live {
        motion::land(SharedString::from(format!("{}-land", commit.key)), mark, handlers.still)
    } else {
        mark.into_any_element()
    };

    let (verb_text, verb_colour) = match &commit.state {
        CommitState::Running | CommitState::Background(_) => ("Committing", theme.text_dim),
        CommitState::Stopped => ("Commit", theme.text_dim),
        CommitState::Landed { .. } | CommitState::Quiet => ("Committed", theme.text_soft),
        CommitState::Failed => ("Couldn't commit", theme.danger),
    };
    let verb = if running {
        motion::shining_text(SharedString::from(format!("{}-shine", commit.key)), verb_text.into(), theme.text_dim, theme.accent_text, handlers.still)
    } else {
        verb_text.into_any_element()
    };

    let hash = hash.map(|hash| {
        div().flex_none().typography(Type::Mono).text_color(theme.accent_text).child(hash).tag("commit-hash")
    });

    let subject = (!commit.subject.is_empty()).then(|| {
        let label = div().min_w_0().truncate();
        if commit.subject_is_command {
            label.typography(Type::Mono).text_color(theme.text_soft).child(commit.subject.clone())
        } else {
            label.typography(Type::Ui).text_color(theme.text_body).child(commit.subject.clone())
        }
    });

    let has_stats = commit.files.is_some()
        || commit.additions.is_some_and(|count| count > 0)
        || commit.deletions.is_some_and(|count| count > 0);

    let stats = has_stats.then(|| {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(5.))
            .typography(Type::Small)
            .when_some(commit.files, |row, files| {
                row.child(div().text_color(theme.text_faint).child(plural(files as usize, "file", "files")))
            })
            .when_some(commit.additions.filter(|&count| count > 0), |row, added| {
                row.child(div().text_color(theme.diff_added).child(format!("+{added}")))
            })
            .when_some(commit.deletions.filter(|&count| count > 0), |row, removed| {
                row.child(div().text_color(theme.diff_removed).child(format!("−{removed}")))
            })
    });

    let status = match &commit.state {
        CommitState::Background(how) => Some(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(how.clone()).into_any_element()),
        CommitState::Landed { failed: true, .. } => Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(Icon::Close.view(px(11.), theme.danger))
                .child(div().typography(Type::Small).text_color(theme.danger).child("failed"))
                .into_any_element(),
        ),
        _ => None,
    };

    let toggle = handlers.toggle.clone();
    let key = commit.key.clone();
    let hover = theme.hover;
    let line = div()
        .id(ElementId::Name(format!("{}-head", commit.key).into()))
        .h(px(26.))
        .min_w_0()
        .ml(px(-6.))
        .px(px(6.))
        .rounded(px(7.))
        .flex()
        .items_center()
        .gap(px(7.))
        .cursor_pointer()
        .hover(move |line| line.bg(hover))
        .on_click(move |_, window, cx| toggle(key.clone(), window, cx))
        .child(mark)
        .child(div().flex_none().typography(Type::Ui).text_color(verb_colour).child(verb))
        .children(hash)
        .children(subject)
        .children(stats)
        .children(status)
        .child(div().flex_none().child(if commit.open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(11.), theme.text_faint)));

    let body = commit.open.then(|| {
        let mut content = div()
            .id(ElementId::Name(format!("{}-output", commit.key).into()))
            .max_h(px(DETAIL_MAX_HEIGHT))
            .overflow_y_scroll()
            .px(px(14.))
            .py(px(10.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .typography(Type::Code)
            .child(div().text_color(theme.text_dim).child(format!("$ {}", commit.command)));
        if let Some(output) = &commit.output {
            content = content.child(
                div()
                    .pt(px(8.))
                    .border_t_1()
                    .border_color(theme.border)
                    .text_color(if failed_only { theme.danger } else { theme.text_soft })
                    .child(output.clone()),
            );
        }
        div()
            .mt(px(4.))
            .rounded(px(10.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.code_ground)
            .overflow_hidden()
            .child(content)
            .tag("commit-output")
    });

    div()
        .flex()
        .flex_col()
        .child(div().flex().child(line))
        .children(body)
        .tag("commit-row")
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::conversation::ToolResult;

    fn bash(command: &str, result: Option<(&str, bool)>) -> ToolStep {
        ToolStep {
            id: "call-c".into(),
            name: "bash".into(),
            input: Some(json!({ "command": command })),
            result: result.map(|(text, is_error)| ToolResult { text: text.into(), is_error, pictures: Vec::new() }),
            writing: None,
            started: None,
        }
    }

    #[test]
    fn spots_a_commit_whatever_else_it_stages_on_the_way() {
        for command in [
            "git commit -m 'Fix upload'",
            "git commit --amend --no-edit",
            "git add src/a.ts && git commit -m 'Fix upload'",
            "git -c commit.gpgsign=false commit -m 'Fix upload'",
            "git -C repo commit -m 'Fix upload'",
            "cd src && git add . && git commit -m 'Fix upload'",
            "git commit -m 'Fix upload' && git push",
            "git add . && git commit -m 'Fix upload' 2>/dev/null",
        ] {
            assert!(is_git_commit(command), "{command}");
        }
    }

    #[test]
    fn refuses_anything_that_does_not_model_exactly_to_a_commit() {
        for command in [
            "git status",
            "git add .",
            "git add . && git push",
            "git commit-tree HEAD -p HEAD -m message",
            "git commit --dry-run -m 'Fix upload'",
            "echo done && git commit -m 'x'",
            "GIT_EDITOR=true git commit",
            "git commit -m \"$(cat message.txt)\"",
            "git commit -m `date`",
            "git commit -m 'unterminated",
            "pnpm test",
            "",
        ] {
            assert!(!is_git_commit(command), "{command}");
        }
    }

    #[test]
    fn reads_the_subject_from_the_first_message_flag() {
        assert_eq!(
            commit_message_subject("git commit -m 'Fix upload' -m 'Co-authored-by: GPT-5.6 (WackCode)'").as_deref(),
            Some("Fix upload")
        );
        assert_eq!(
            commit_message_subject("git commit --message=\"Two lines\n\\nBody\"").as_deref(),
            Some("Two lines")
        );
        assert_eq!(
            commit_message_subject("git add . && git commit -m 'Fix upload'").as_deref(),
            Some("Fix upload")
        );
        assert_eq!(commit_message_subject("git commit -am 'Quick fix'").as_deref(), Some("Quick fix"));
        assert_eq!(commit_message_subject("git commit --amend --no-edit"), None);
        assert_eq!(commit_message_subject("git commit -F message.txt"), None);
    }

    #[test]
    fn parses_git_commit_output_and_stats() {
        assert_eq!(
            parse_commit("[main a1b2c3d] Fix the flaky upload test\n 2 files changed, 12 insertions(+), 3 deletions(-)\n create mode 100644 src/upload.ts"),
            Some(ParsedCommit {
                hash: "a1b2c3d".into(),
                subject: "Fix the flaky upload test".into(),
                files: Some(2),
                additions: Some(12),
                deletions: Some(3),
            })
        );
        assert_eq!(
            parse_commit("[master (root-commit) b143e10] First commit\n 1 file changed, 1 insertion(+)"),
            Some(ParsedCommit {
                hash: "b143e10".into(),
                subject: "First commit".into(),
                files: Some(1),
                additions: Some(1),
                deletions: None,
            })
        );
        assert_eq!(
            parse_commit("[detached HEAD ab37977] Detached commit\n 1 file changed, 1 deletion(-)"),
            Some(ParsedCommit {
                hash: "ab37977".into(),
                subject: "Detached commit".into(),
                files: Some(1),
                additions: None,
                deletions: Some(1),
            })
        );
        // An amend prints its Date: line between the summary and the stats.
        assert_eq!(
            parse_commit("[main c733120] Amend it\n Date: Tue Oct 27 20:35:50 2026 +0000\n 2 files changed"),
            Some(ParsedCommit {
                hash: "c733120".into(),
                subject: "Amend it".into(),
                files: Some(2),
                additions: None,
                deletions: None,
            })
        );
        // An empty commit prints no stats line at all.
        assert_eq!(
            parse_commit("[feature/x 75a5a34] Empty"),
            Some(ParsedCommit { hash: "75a5a34".into(), subject: "Empty".into(), files: None, additions: None, deletions: None })
        );
        assert_eq!(
            parse_commit("[main 1f2e3d4] "),
            Some(ParsedCommit { hash: "1f2e3d4".into(), subject: "".into(), files: None, additions: None, deletions: None })
        );
    }

    #[test]
    fn keeps_the_newest_commit_of_two_and_subjects_with_brackets() {
        assert_eq!(
            parse_commit("[main 1111111] First\n 1 file changed, 1 insertion(+)\n[main 2222222] Second\n 2 files changed, 4 insertions(+)"),
            Some(ParsedCommit {
                hash: "2222222".into(),
                subject: "Second".into(),
                files: Some(2),
                additions: Some(4),
                deletions: None,
            })
        );
        assert_eq!(
            parse_commit("[main abc1234] Fix [beefcafe] warnings"),
            Some(ParsedCommit { hash: "abc1234".into(), subject: "Fix [beefcafe] warnings".into(), files: None, additions: None, deletions: None })
        );
    }

    #[test]
    fn ignores_output_that_is_not_gits_summary_line() {
        assert_eq!(parse_commit("On branch main\nnothing to commit, working tree clean\n\nExit code 1."), None);
        assert_eq!(parse_commit(""), None);
        assert_eq!(parse_commit("[main 12345] too short a hash"), None);
        assert_eq!(parse_commit("[main ABC1234] uppercase is not git"), None);
    }

    #[test]
    fn builds_running_landed_failed_and_partial_failure_rows() {
        let running = from_tool(&bash("git commit -m 'Fix upload' -m 'Co-authored-by: GLM (WackCode)'", None), true, false);
        assert_eq!(running.state, CommitState::Running);
        assert_eq!(running.subject.as_ref(), "Fix upload");
        assert!(!running.subject_is_command);
        assert_eq!(running.activity().as_deref(), Some("Committing Fix upload"));

        let amend = from_tool(&bash("git commit --amend --no-edit", None), true, false);
        assert!(amend.subject_is_command);
        assert_eq!(amend.subject.as_ref(), "git commit --amend --no-edit");

        let landed = from_tool(
            &bash(
                "git add src/a.ts && git commit -m 'Fix upload'",
                Some(("[main a1b2c3d] Fix upload\n 2 files changed, 12 insertions(+), 3 deletions(-)", false)),
            ),
            false,
            false,
        );
        assert_eq!(landed.state, CommitState::Landed { hash: "a1b2c3d".into(), failed: false });
        assert_eq!((landed.files, landed.additions, landed.deletions), (Some(2), Some(12), Some(3)));
        assert_eq!(landed.activity(), None);

        let refused = from_tool(
            &bash("git commit -m 'Fix upload'", Some(("On branch main\nnothing to commit, working tree clean\n\nExit code 1.", true))),
            false,
            false,
        );
        assert_eq!(refused.state, CommitState::Failed);
        assert_eq!(refused.subject.as_ref(), "Fix upload");

        let push_failed = from_tool(
            &bash(
                "git commit -m 'Fix upload' && git push",
                Some(("[main a1b2c3d] Fix upload\n 1 file changed, 2 insertions(+)\n ! [rejected] main -> main\n\nExit code 1.", true)),
            ),
            false,
            false,
        );
        assert_eq!(push_failed.state, CommitState::Landed { hash: "a1b2c3d".into(), failed: true });
        assert_eq!((push_failed.files, push_failed.additions, push_failed.deletions), (Some(1), Some(2), None));
    }
}
