//! Git, through the system `git` CLI (never libgit2), so it behaves exactly as the user's own
//! `git` does, with their configuration.
//!
//! Every call is blocking: run it off the UI thread.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// What `git diff` compares against in a repository with no commits yet.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
/// An untracked file larger than this is not read to count or show its lines.
const MAX_UNTRACKED_BYTES: u64 = 2_000_000;
/// A diff longer than this is cut off, with a note.
const MAX_DIFF_BYTES: usize = 1_500_000;

/// The branch checked out in `folder`, a short commit hash when the head is detached, or `None`
/// outside a repository (or when Git is missing).
pub fn branch(folder: &Path) -> Option<String> {
    let run = |args: &[&str]| {
        let output = Command::new("git").arg("-C").arg(folder).args(args).output().ok()?;
        let text = String::from_utf8(output.stdout).ok()?;
        let text = text.trim();
        (output.status.success() && !text.is_empty()).then(|| text.to_string())
    };
    run(&["symbolic-ref", "--quiet", "--short", "HEAD"]).or_else(|| run(&["rev-parse", "--short", "HEAD"]))
}

/// The folder holding `folder`'s repository: the parent of its common `.git` folder, which every
/// worktree and subfolder of one repository shares. `None` outside a repository.
pub fn common_root(folder: &Path) -> Option<PathBuf> {
    let common = git(folder, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).ok()?;
    let common = PathBuf::from(String::from_utf8_lossy(&common).trim_end());
    common.parent().map(Path::to_path_buf)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

impl FileStatus {
    pub fn word(self) -> &'static str {
        match self {
            FileStatus::Added => "added",
            FileStatus::Modified => "modified",
            FileStatus::Deleted => "deleted",
            FileStatus::Renamed => "renamed",
            FileStatus::Untracked => "new",
            FileStatus::Conflicted => "conflict",
        }
    }
}

/// One file that differs from the last commit, staged or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFile {
    /// Relative to the repository's root.
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    /// Lines added and removed; `None` for a binary file.
    pub lines: Option<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Changes {
    NotARepository,
    Repository { root: PathBuf, files: Vec<ChangedFile> },
}

fn git(folder: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(["-c", "core.quotepath=off", "-c", "color.ui=false"])
        .args(args)
        .output()
        .map_err(|error| format!("Could not run git: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if message.is_empty() { format!("git {} failed.", args.first().unwrap_or(&"")) } else { message })
    }
}

/// The commit to compare with: HEAD, or the empty tree before the first commit.
fn base(root: &Path) -> &'static str {
    if git(root, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok() { "HEAD" } else { EMPTY_TREE }
}

/// Everything in `folder`'s repository that differs from its last commit, staged or not, with
/// untracked files. Read-only: nothing here changes the repository.
pub fn changes(folder: &Path) -> Result<Changes, String> {
    let Ok(top) = git(folder, &["rev-parse", "--show-toplevel"]) else {
        return Ok(Changes::NotARepository);
    };
    let root = PathBuf::from(String::from_utf8_lossy(&top).trim());
    let status = git(&root, &["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
    let numstat = git(&root, &["diff", base(&root), "--numstat", "-z", "-M"])?;
    let counts = parse_numstat(&numstat);
    let mut files: Vec<ChangedFile> = parse_status(&status)
        .into_iter()
        .map(|(x, y, path, old_path)| {
            let status = match (x, y) {
                (b'?', b'?') => FileStatus::Untracked,
                (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => FileStatus::Conflicted,
                (b'R', _) | (_, b'R') => FileStatus::Renamed,
                (b'A', _) => FileStatus::Added,
                (b'D', _) | (_, b'D') => FileStatus::Deleted,
                _ => FileStatus::Modified,
            };
            let lines = if status == FileStatus::Untracked {
                untracked_lines(&root.join(&path))
            } else {
                counts.iter().find(|(counted, _)| *counted == path).and_then(|(_, lines)| *lines)
            };
            ChangedFile { path, old_path, status, lines }
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Changes::Repository { root, files })
}

/// One file's change as a unified diff against the last commit. An untracked file reads as
/// all added.
pub fn file_diff(root: &Path, file: &ChangedFile) -> Result<String, String> {
    if file.status == FileStatus::Untracked {
        let path = root.join(&file.path);
        let size = std::fs::metadata(&path).map(|metadata| metadata.len()).unwrap_or(0);
        if size > MAX_UNTRACKED_BYTES {
            return Ok(format!("Binary files /dev/null and b/{} differ\n", file.path));
        }
        let bytes = std::fs::read(&path).map_err(|error| format!("Could not read {}: {error}", file.path))?;
        if bytes.contains(&0) {
            return Ok(format!("Binary files /dev/null and b/{} differ\n", file.path));
        }
        let text = String::from_utf8_lossy(&bytes);
        let count = text.lines().count();
        let mut diff = format!("--- /dev/null\n+++ b/{}\n@@ -0,0 +1,{count} @@\n", file.path);
        for line in text.lines() {
            diff.push('+');
            diff.push_str(line);
            diff.push('\n');
        }
        return Ok(cap(diff));
    }
    let mut args = vec!["diff", base(root), "-M", "--no-ext-diff", "--"];
    if let Some(old) = &file.old_path {
        args.push(old);
    }
    args.push(&file.path);
    let bytes = git(root, &args)?;
    Ok(cap(String::from_utf8_lossy(&bytes).into_owned()))
}

fn cap(mut diff: String) -> String {
    if diff.len() > MAX_DIFF_BYTES {
        let mut end = MAX_DIFF_BYTES;
        while !diff.is_char_boundary(end) {
            end -= 1;
        }
        diff.truncate(end);
        diff.push_str("\n\\ The rest of this diff is too long to show.\n");
    }
    diff
}

fn untracked_lines(path: &Path) -> Option<(usize, usize)> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_UNTRACKED_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    Some((String::from_utf8_lossy(&bytes).lines().count(), 0))
}

/// `git status --porcelain=v1 -z`: `XY path`, with a rename's old path as the next entry.
fn parse_status(bytes: &[u8]) -> Vec<(u8, u8, String, Option<String>)> {
    let entries: Vec<&[u8]> = bytes.split(|byte| *byte == 0).filter(|entry| !entry.is_empty()).collect();
    let mut records = Vec::new();
    let mut index = 0;
    while index < entries.len() {
        let entry = entries[index];
        index += 1;
        if entry.len() < 4 {
            continue;
        }
        let (x, y) = (entry[0], entry[1]);
        let path = String::from_utf8_lossy(&entry[3..]).into_owned();
        let old_path = if matches!(x, b'R' | b'C') || matches!(y, b'R' | b'C') {
            index += 1;
            entries.get(index - 1).map(|old| String::from_utf8_lossy(old).into_owned())
        } else {
            None
        };
        // A directory entry only appears for an untracked folder, which --untracked-files=all
        // expands; skip one if it ever shows.
        if path.ends_with('/') {
            continue;
        }
        records.push((x, y, path, old_path));
    }
    records
}

/// `git diff --numstat -z`: `added\tremoved\tpath\0`, or for a rename
/// `added\tremoved\t\0old\0new\0`. Binary files count as `-`.
fn parse_numstat(bytes: &[u8]) -> Vec<(String, Option<(usize, usize)>)> {
    let mut parts = bytes.split(|byte| *byte == 0);
    let mut found = Vec::new();
    while let Some(head) = parts.next() {
        if head.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(head);
        let mut fields = text.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        let path = if path.is_empty() {
            let _old = parts.next();
            parts.next().map(|new| String::from_utf8_lossy(new).into_owned()).unwrap_or_default()
        } else {
            path.to_string()
        };
        let lines = added.parse().ok().zip(removed.parse().ok());
        found.push((path, lines));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(folder: &Path, args: &[&str]) {
        let status = Command::new("git").arg("-C").arg(folder).args(args).output().unwrap().status;
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn names_the_branch_or_nothing() {
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(branch(folder.path()), None);

        git(folder.path(), &["init", "--quiet", "--initial-branch", "trunk"]);
        // An unborn branch still has a name.
        assert_eq!(branch(folder.path()).as_deref(), Some("trunk"));
    }

    fn commit_all(folder: &Path) {
        git(folder, &["add", "-A"]);
        git(folder, &["-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "--quiet", "-m", "x"]);
    }

    #[test]
    fn lists_what_changed_since_the_last_commit() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        assert_eq!(changes(root).unwrap(), Changes::NotARepository);

        git(root, &["init", "--quiet"]);
        std::fs::write(root.join("kept.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(root.join("gone.txt"), "bye\n").unwrap();
        std::fs::write(root.join("old name.txt"), "a\nb\nc\nd\ne\n").unwrap();
        // Before the first commit everything is new, compared with the empty tree.
        let Changes::Repository { files, .. } = changes(root).unwrap() else { panic!() };
        assert!(files.iter().all(|file| file.status == FileStatus::Untracked));
        commit_all(root);

        std::fs::write(root.join("kept.txt"), "one\n2\nthree\nfour\n").unwrap();
        std::fs::remove_file(root.join("gone.txt")).unwrap();
        git(root, &["mv", "old name.txt", "new name.txt"]);
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/fresh.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("blob.bin"), [0u8, 1, 2, 3]).unwrap();

        let Changes::Repository { root: found_root, files } = changes(root).unwrap() else { panic!() };
        assert_eq!(found_root.canonicalize().unwrap(), root.canonicalize().unwrap());
        let file = |path: &str| files.iter().find(|file| file.path == path).unwrap_or_else(|| panic!("{path} in {files:?}"));
        assert_eq!(file("kept.txt").status, FileStatus::Modified);
        assert_eq!(file("kept.txt").lines, Some((2, 1)));
        assert_eq!(file("gone.txt").status, FileStatus::Deleted);
        assert_eq!(file("new name.txt").status, FileStatus::Renamed);
        assert_eq!(file("new name.txt").old_path.as_deref(), Some("old name.txt"));
        assert_eq!(file("src/fresh.rs").status, FileStatus::Untracked);
        assert_eq!(file("src/fresh.rs").lines, Some((1, 0)));
        assert_eq!(file("blob.bin").lines, None, "binary");

        let diff = file_diff(root, file("kept.txt")).unwrap();
        assert!(diff.contains("-two") && diff.contains("+2") && diff.contains("+four"), "{diff}");
        let fresh = file_diff(root, file("src/fresh.rs")).unwrap();
        assert!(fresh.contains("@@ -0,0 +1,1 @@") && fresh.contains("+fn main() {}"), "{fresh}");
        assert!(file_diff(root, file("blob.bin")).unwrap().starts_with("Binary files"));
        let renamed = file_diff(root, file("new name.txt")).unwrap();
        assert!(renamed.contains("rename from old name.txt"), "{renamed}");
    }
}
