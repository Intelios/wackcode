//! Workspace file listing and `@` path mentions for Code composers, ported from version 2
//! (`src-tauri/src/files.rs` and `src/mention-utils.ts`).
//!
//! - Inside a Git repository [`list`] reads tracked and non-ignored untracked files through the
//!   system `git` CLI (`git ls-files`), relative to the project folder; outside Git it does a
//!   bounded walk skipping `.git` and `node_modules`. Both stop past [`MAX_FILES`] (20,000),
//!   setting `truncated` so the picker can say so.
//! - Listing the user's home folder is refused so opening `~` never walks personal folders or
//!   triggers macOS privacy prompts.
//! - Every call to [`list`] is blocking: run it off the UI thread.
//! - The composer keeps `@path` (or `@"path with spaces"`) as plain text and the model reads the
//!   file itself; nothing is injected into the message.

use std::{
    collections::BTreeSet,
    fs,
    ops::Range,
    path::{Path, PathBuf},
};

pub use crate::chat_refs::active_mention;
use crate::git;

/// More files than this are cut off; the picker says so.
pub const MAX_FILES: usize = 20_000;
/// A folder outside Git is walked at most this deep.
const MAX_DEPTH: usize = 12;
/// Never descended into by the plain walk (Git repositories use their own ignore rules).
const SKIPPED_DIRS: [&str; 2] = [".git", "node_modules"];

/// One row the `@` file picker can offer: a file, or a folder that holds one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// Relative to the project folder; folders end with `/`.
    pub path: String,
    pub directory: bool,
}

impl FileEntry {
    /// The last path segment, with a folder's trailing `/`.
    pub fn name(&self) -> &str {
        let bare = self.path.strip_suffix('/').unwrap_or(&self.path);
        match bare.rfind('/') {
            Some(slash) => &self.path[slash + 1..],
            None => &self.path,
        }
    }

    /// The parent folder with its trailing `/`, or empty at the top level.
    pub fn parent(&self) -> &str {
        let bare = self.path.strip_suffix('/').unwrap_or(&self.path);
        match bare.rfind('/') {
            Some(slash) => &bare[..=slash],
            None => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IndexedEntry {
    entry: FileEntry,
    lower: String,
    depth: usize,
    chars: usize,
}

/// Every file under a project folder (up to [`MAX_FILES`]), with ancestor folders precomputed so
/// ranking on each keystroke stays cheap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceFiles {
    pub files: Vec<String>,
    pub truncated: bool,
    entries: Vec<IndexedEntry>,
}

impl WorkspaceFiles {
    /// Indexes `files` and every folder holding one.
    pub fn from_files(files: Vec<String>, truncated: bool) -> Self {
        let mut directories = BTreeSet::new();
        for file in &files {
            let mut start = 0;
            while let Some(offset) = file[start..].find('/') {
                let end = start + offset + 1;
                directories.insert(file[..end].to_string());
                start = end;
            }
        }
        let mut entries = Vec::with_capacity(directories.len() + files.len());
        for path in directories {
            entries.push(index_entry(path, true));
        }
        for path in &files {
            entries.push(index_entry(path.clone(), false));
        }
        Self { files, truncated, entries }
    }

    /// The best matches for what follows `@`: folders end with `/`, and an empty query lists the
    /// top level.
    pub fn rank(&self, query: &str, limit: usize) -> Vec<FileEntry> {
        let needle = query.to_lowercase();
        let mut scored: Vec<(u32, &IndexedEntry)> = self
            .entries
            .iter()
            .filter_map(|entry| {
                let value = score(entry, &needle);
                (value > 0).then_some((value, entry))
            })
            .collect();
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.depth.cmp(&b.1.depth))
                .then_with(|| a.1.chars.cmp(&b.1.chars))
                .then_with(|| a.1.lower.cmp(&b.1.lower))
                .then_with(|| a.1.entry.path.cmp(&b.1.entry.path))
        });
        scored.into_iter().take(limit).map(|(_, item)| item.entry.clone()).collect()
    }
}

fn index_entry(path: String, directory: bool) -> IndexedEntry {
    let lower = path.to_lowercase();
    let depth = path_depth(&path);
    let chars = path.chars().count();
    IndexedEntry { entry: FileEntry { path, directory }, lower, depth, chars }
}

fn path_depth(path: &str) -> usize {
    let bare = path.strip_suffix('/').unwrap_or(path);
    bare.bytes().filter(|byte| *byte == b'/').count()
}

/// Files under `root`, relative to it and `/`-separated. Inside a Git repository this is every
/// tracked or untracked file that isn't ignored; anywhere else a bounded walk of the folder.
pub fn list(root: &Path) -> Result<WorkspaceFiles, String> {
    list_with_cap(root, MAX_FILES)
}

fn list_with_cap(root: &Path, max_files: usize) -> Result<WorkspaceFiles, String> {
    if !root.is_dir() {
        return Err("This project's folder no longer exists.".into());
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && same_path(root, &home)
    {
        return Err("File mentions are off for a home folder. Open a project folder instead.".into());
    }
    let mut files = match git::ls_files(root) {
        Ok(listed) => listed.into_iter().filter(|path| root.join(path).is_file()).take(max_files + 1).collect(),
        Err(_) => walk(root, max_files),
    };
    let truncated = files.len() > max_files;
    files.truncate(max_files);
    files.sort();
    Ok(WorkspaceFiles::from_files(files, truncated))
}

/// Stops once it has one file more than `max_files`, so a huge folder costs no more than that.
fn walk(root: &Path, max_files: usize) -> Vec<String> {
    let mut files = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                let name = entry.file_name();
                if depth < MAX_DEPTH && !SKIPPED_DIRS.iter().any(|skip| name == *skip) {
                    stack.push((path, depth + 1));
                }
            } else if (kind.is_file() || (kind.is_symlink() && path.is_file()))
                && let Ok(relative) = path.strip_prefix(root)
            {
                files.push(relative.to_string_lossy().into_owned());
                if files.len() > max_files {
                    return files;
                }
            }
        }
    }
    files
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn is_subsequence(query: &str, target: &str) -> bool {
    let mut chars = target.chars();
    query.chars().all(|wanted| chars.any(|c| c == wanted))
}

fn score(entry: &IndexedEntry, query: &str) -> u32 {
    let path = entry.lower.as_str();
    let bare = path.strip_suffix('/').unwrap_or(path);
    let value = if query.is_empty() {
        u32::from(entry.depth == 0)
    } else if query.contains('/') {
        if path == query {
            0
        } else if let Some(rest) = path.strip_prefix(query) {
            let rest = rest.strip_suffix('/').unwrap_or(rest);
            if rest.contains('/') { 80 } else { 100 }
        } else if path.contains(query) {
            50
        } else if is_subsequence(query, bare) {
            10
        } else {
            0
        }
    } else {
        let name = bare.rsplit('/').next().unwrap_or(bare);
        let stem = match name.rfind('.') {
            Some(dot) if dot > 0 => &name[..dot],
            _ => name,
        };
        if name == query || stem == query {
            100
        } else if name.starts_with(query) {
            80
        } else if name.contains(query) {
            60
        } else if bare.contains(query) {
            40
        } else if is_subsequence(query, name) {
            // Letters scattered across a whole path match almost anything; keep fuzzy to the name.
            10
        } else {
            0
        }
    };
    // Folders edge ahead of files that match equally well, as in version 2.
    if value > 0 && entry.entry.directory { value + 2 } else { value }
}

/// Convenience wrapper for one-off ranking over a list of file paths.
pub fn rank(files: &[String], query: &str, limit: usize) -> Vec<FileEntry> {
    WorkspaceFiles::from_files(files.to_vec(), false).rank(query, limit)
}

/// The token written into the composer: quotes paths whose characters would otherwise end an
/// unquoted mention token early. A quoted folder stays open so typing keeps completing inside it.
pub fn mention_value(path: &str, directory: bool) -> String {
    let needs_quotes = path.chars().any(|c| c.is_whitespace() || "'`()[]{}".contains(c));
    if !needs_quotes {
        format!("@{path}")
    } else if directory {
        format!("@\"{path}")
    } else {
        format!("@\"{path}\"")
    }
}

/// Replaces the `@word` at `range` with `entry`'s token. Picking a file closes the token with a
/// trailing space; picking a folder leaves the caret right after `/` so the picker stays open
/// inside it.
pub fn pick(text: &str, range: Range<usize>, entry: &FileEntry) -> (String, usize) {
    let value = mention_value(&entry.path, entry.directory);
    let before = &text[..range.start];
    let rest = &text[range.end..];
    if entry.directory {
        let caret = range.start + value.len();
        (format!("{before}{value}{rest}"), caret)
    } else {
        let after = rest.strip_prefix(' ').unwrap_or(rest);
        let caret = range.start + value.len() + 1;
        (format!("{before}{value} {after}"), caret)
    }
}

/// One piece of a sent message: plain prose, or an `@path` / `@"…"` mention to highlight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MentionSegment<'a> {
    pub text: &'a str,
    pub mention: bool,
}

fn at_token_start(text: &str, index: usize) -> bool {
    index == 0 || text[..index].chars().next_back().is_some_and(|c| c.is_whitespace() || "([{\"'`".contains(c))
}

/// Splits sent text so path-like and quoted `@` mentions can be highlighted. Only quoted mentions
/// (`@"…"`) and unquoted tokens containing `/` or `.` count, so `@someone` and emails stay plain;
/// trailing sentence punctuation is left out.
pub fn split_mentions(text: &str) -> Vec<MentionSegment<'_>> {
    let mut segments = Vec::new();
    let mut cursor = 0;
    let mut scan = 0;
    while let Some(rel) = text[scan..].find('@') {
        let index = scan + rel;
        if !at_token_start(text, index) {
            scan = index + 1;
            continue;
        }
        if text[index..].starts_with("@\"") {
            let body = &text[index + 2..];
            if let Some(close) = body.find(['"', '\n'])
                && close > 0
                && body.as_bytes()[close] == b'"'
            {
                let end = index + 2 + close + 1;
                if index > cursor {
                    segments.push(MentionSegment { text: &text[cursor..index], mention: false });
                }
                segments.push(MentionSegment { text: &text[index..end], mention: true });
                cursor = end;
                scan = end;
                continue;
            }
            scan = index + 1;
            continue;
        }
        let tail = &text[index + 1..];
        let raw_len = tail.find(|c: char| c.is_whitespace() || "\"'`()[]{}".contains(c)).unwrap_or(tail.len());
        let raw_end = index + 1 + raw_len;
        let trimmed = text[index + 1..raw_end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
        if !trimmed.is_empty() && trimmed.contains(['.', '/']) {
            let token_end = index + 1 + trimmed.len();
            if index > cursor {
                segments.push(MentionSegment { text: &text[cursor..index], mention: false });
            }
            segments.push(MentionSegment { text: &text[index..token_end], mention: true });
            cursor = token_end;
            scan = raw_end;
        } else {
            scan = raw_end.max(index + 1);
        }
    }
    if cursor < text.len() {
        segments.push(MentionSegment { text: &text[cursor..], mention: false });
    }
    segments
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    fn git(path: &Path, args: &[&str]) {
        let status = Command::new("git").arg("-C").arg(path).args(args).output().unwrap().status;
        assert!(status.success(), "git {args:?} failed");
    }

    fn write(root: &Path, path: &str) {
        let target = root.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, "x").unwrap();
    }

    #[test]
    fn lists_tracked_and_untracked_files_but_not_ignored_ones() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "--quiet"]);
        write(root, ".gitignore");
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        write(root, "README.md");
        write(root, "src/main.rs");
        write(root, "target/out.bin");
        git(root, &["add", "README.md", ".gitignore"]);
        write(root, "notes with space.txt");
        let listed = list(root).unwrap();
        assert_eq!(listed.files, vec![".gitignore", "README.md", "notes with space.txt", "src/main.rs"]);
        assert!(!listed.truncated);
    }

    #[test]
    fn a_subfolder_of_a_repository_lists_paths_relative_to_itself() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "--quiet"]);
        write(root, "top.txt");
        write(root, "app/src/lib.rs");
        git(root, &["add", "."]);
        fs::remove_file(root.join("top.txt")).unwrap();
        assert_eq!(list(&root.join("app")).unwrap().files, vec!["src/lib.rs"]);
        // Deleted but still tracked files are left out.
        assert_eq!(list(root).unwrap().files, vec!["app/src/lib.rs"]);
    }

    #[test]
    fn plain_folders_are_walked_without_git_or_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.txt");
        write(root, "deep/er/b.txt");
        write(root, "node_modules/pkg/index.js");
        write(root, ".git/config");
        let mut files = walk(root, MAX_FILES);
        files.sort();
        assert_eq!(files, vec!["a.txt", "deep/er/b.txt"]);
    }

    #[test]
    fn both_git_and_walk_stop_past_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("many")).unwrap();
        for index in 0..30 {
            fs::write(root.join("many").join(format!("f{index:02}.txt")), "").unwrap();
        }
        let plain = list_with_cap(root, 20).unwrap();
        assert_eq!(plain.files.len(), 20);
        assert!(plain.truncated);

        git(root, &["init", "--quiet"]);
        git(root, &["add", "."]);
        let repo = list_with_cap(root, 20).unwrap();
        assert_eq!(repo.files.len(), 20);
        assert!(repo.truncated);
    }

    #[test]
    fn a_missing_folder_and_home_folder_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list(&dir.path().join("gone")).is_err());
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
            && home.is_dir()
        {
            assert!(list(&home).unwrap_err().contains("home folder"));
        }
    }

    #[test]
    fn entry_name_and_parent_split_files_and_folders() {
        let entry = |path: &str, directory| FileEntry { path: path.into(), directory };
        assert_eq!((entry("README.md", false).name(), entry("README.md", false).parent()), ("README.md", ""));
        assert_eq!((entry("src/", true).name(), entry("src/", true).parent()), ("src/", ""));
        assert_eq!((entry("src/App.tsx", false).name(), entry("src/App.tsx", false).parent()), ("App.tsx", "src/"));
        assert_eq!((entry("src/components/", true).name(), entry("src/components/", true).parent()), ("components/", "src/"));
    }

    #[test]
    fn rank_lists_top_level_prefers_basenames_derives_folders_and_fuzzy_matches() {
        let files: Vec<String> = [
            "README.md",
            "src/App.tsx",
            "src/components/Composer.tsx",
            "src/readme-utils.ts",
            "docs/guide/read.md",
            "package.json",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let index = WorkspaceFiles::from_files(files, false);
        let paths = |query: &str| index.rank(query, 20).into_iter().map(|entry| entry.path).collect::<Vec<_>>();

        assert_eq!(paths(""), vec!["src/", "docs/", "README.md", "package.json"]);
        let read = paths("read");
        assert_eq!(read[0], "docs/guide/read.md");
        assert!(read.contains(&"README.md".to_string()));
        assert!(read.contains(&"src/readme-utils.ts".to_string()));
        assert_eq!(paths("src/"), vec!["src/components/", "src/App.tsx", "src/readme-utils.ts", "src/components/Composer.tsx"]);
        assert_eq!(index.rank("comp", 20)[0], FileEntry { path: "src/components/".into(), directory: true });
        assert_eq!(paths("cmpsr"), vec!["src/components/Composer.tsx"]);
        assert_eq!(paths("rad"), vec!["README.md", "src/readme-utils.ts", "docs/guide/read.md"]);
        assert!(paths("zzz").is_empty());

        let many: Vec<String> = (0..50).map(|index| format!("file{index}.ts")).collect();
        assert_eq!(rank(&many, "file", 20).len(), 20);
    }

    #[test]
    fn mention_value_quotes_paths_with_spaces_and_pick_handles_files_and_folders() {
        assert_eq!(mention_value("README.md", false), "@README.md");
        assert_eq!(mention_value("my notes.txt", false), "@\"my notes.txt\"");
        assert_eq!(mention_value("my dir/", true), "@\"my dir/");
        assert_eq!(mention_value("app/(auth)/", true), "@\"app/(auth)/");
        assert_eq!(mention_value("routes/[id]/page.tsx", false), "@\"routes/[id]/page.tsx\"");

        let file = FileEntry { path: "README.md".into(), directory: false };
        assert_eq!(pick("look at @rea", 8..12, &file), ("look at @README.md ".into(), 19));
        assert_eq!(pick("look at @rea now", 8..12, &file), ("look at @README.md now".into(), 19));

        let folder = FileEntry { path: "src/components/".into(), directory: true };
        assert_eq!(pick("check @comp next", 6..11, &folder), ("check @src/components/ next".into(), 22));

        let spaced_dir = FileEntry { path: "my dir/".into(), directory: true };
        assert_eq!(pick("@my", 0..3, &spaced_dir), ("@\"my dir/".into(), 9));
        let spaced_file = FileEntry { path: "my dir/notes.txt".into(), directory: false };
        assert_eq!(pick("@\"my dir/", 0..9, &spaced_file), ("@\"my dir/notes.txt\" ".into(), 20));
    }

    #[test]
    fn split_mentions_marks_paths_and_quoted_mentions_only() {
        let seg = |text, mention| MentionSegment { text, mention };
        assert_eq!(
            split_mentions("fix @src/App.tsx, then @README.md."),
            vec![seg("fix ", false), seg("@src/App.tsx", true), seg(", then ", false), seg("@README.md", true), seg(".", false)]
        );
        assert_eq!(
            split_mentions("ask @someone about @\"my notes.txt\" or a@b.com"),
            vec![seg("ask @someone about ", false), seg("@\"my notes.txt\"", true), seg(" or a@b.com", false)]
        );
        assert_eq!(
            split_mentions("check (@src/) and @\"Trip ideas\""),
            vec![seg("check (", false), seg("@src/", true), seg(") and ", false), seg("@\"Trip ideas\"", true)]
        );
        assert_eq!(split_mentions("plain words"), vec![seg("plain words", false)]);
    }
}
