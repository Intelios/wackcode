use crate::models::{GitChangeFile, GitChanges};
use std::{fs, path::{Path, PathBuf}, process::Command};

const MAX_DIFF_BYTES: usize = 240_000;
const BINARY_SCAN_BYTES: usize = 8_192;

#[derive(Debug)]
pub struct ProjectGitInfo {
    pub root: Option<PathBuf>,
    pub has_head: bool,
}

pub fn inspect_project(path: &Path) -> ProjectGitInfo {
    let root = git_output(path, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(|value| PathBuf::from(value.trim_end()));
    let has_head = root.as_deref().is_some_and(|root| {
        Command::new("git")
            .args(["-C"])
            .arg(root)
            .args(["rev-parse", "--verify", "HEAD"])
            .output()
            .is_ok_and(|output| output.status.success())
    });
    ProjectGitInfo { root, has_head }
}

pub fn current_branch(path: &Path) -> Option<String> {
    git_output(path, &["branch", "--show-current"])
        .ok()
        .map(|branch| branch.trim().to_string())
        .filter(|branch| !branch.is_empty())
}

pub fn create_worktree(
    project_path: &Path,
    git_root: &Path,
    destination: &Path,
    branch: &str,
) -> Result<PathBuf, String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let output = Command::new("git")
        .args(["-C"])
        .arg(git_root)
        .args(["worktree", "add", "-b", branch])
        .arg(destination)
        .arg("HEAD")
        .output()
        .map_err(|error| format!("Could not start git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not create worktree: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let relative = project_path.strip_prefix(git_root).unwrap_or(Path::new(""));
    Ok(destination.join(relative))
}

/// Best-effort removal of a worktree. A missing or detached worktree is not an error.
pub fn remove_worktree(git_root: &Path, worktree_path: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(git_root)
        .args(["worktree", "remove", "--force"])
        .arg(worktree_path)
        .output()
        .map_err(|error| format!("Could not start git: {error}"))?;
    if output.status.success() { return Ok(()); }
    let _ = Command::new("git").args(["-C"]).arg(git_root).args(["worktree", "prune"]).output();
    if worktree_path.exists() {
        fs::remove_dir_all(worktree_path).map_err(|error| format!("Could not remove worktree: {error}"))?;
        let _ = Command::new("git").args(["-C"]).arg(git_root).args(["worktree", "prune"]).output();
    }
    Ok(())
}

pub fn changes(path: &Path) -> Result<GitChanges, String> {
    let info = inspect_project(path);
    let Some(root) = info.root else {
        return Ok(GitChanges { is_git: false, root: None, branch: None, files: Vec::new() });
    };
    let status_output = git_bytes(&root, &["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
    let records = parse_status(&status_output);
    let mut files = Vec::with_capacity(records.len());
    for record in records {
        files.push(build_change(&root, record)?);
    }
    Ok(GitChanges {
        is_git: true,
        root: Some(root.to_string_lossy().into_owned()),
        branch: current_branch(&root),
        files,
    })
}

#[derive(Debug)]
struct StatusRecord {
    path: String,
    x: u8,
    y: u8,
}

fn parse_status(bytes: &[u8]) -> Vec<StatusRecord> {
    let entries: Vec<&[u8]> = bytes.split(|byte| *byte == 0).filter(|entry| !entry.is_empty()).collect();
    let mut records = Vec::new();
    let mut index = 0;
    while index < entries.len() {
        let entry = entries[index];
        if entry.len() < 4 {
            index += 1;
            continue;
        }
        let x = entry[0];
        let y = entry[1];
        let path = String::from_utf8_lossy(&entry[3..]).into_owned();
        records.push(StatusRecord { path, x, y });
        if x == b'R' || x == b'C' || y == b'R' || y == b'C' { index += 2; } else { index += 1; }
    }
    records
}

fn build_change(root: &Path, record: StatusRecord) -> Result<GitChangeFile, String> {
    let untracked = record.x == b'?' && record.y == b'?';
    let staged = !untracked && record.x != b' ';
    let unstaged = !untracked && record.y != b' ';
    let file_path = root.join(&record.path);
    let mut binary = is_binary_file(&file_path);
    let mut diff = String::new();
    let mut truncated = false;

    if untracked {
        match fs::read(&file_path) {
            Ok(content) if binary => {
                diff = format!("Binary file · {} bytes", content.len());
            }
            Ok(content) => {
                let text = String::from_utf8_lossy(&content);
                let header = format!(
                    "diff --git a/{0} b/{0}\nnew file mode 100644\n--- /dev/null\n+++ b/{0}\n@@ -0,0 +1,{1} @@\n",
                    record.path,
                    text.lines().count()
                );
                diff.push_str(&header);
                for line in text.lines() {
                    diff.push('+');
                    diff.push_str(line);
                    diff.push('\n');
                    if diff.len() > MAX_DIFF_BYTES { truncated = true; break; }
                }
            }
            Err(error) => diff = format!("Could not preview this untracked file: {error}"),
        }
    } else {
        if staged {
            diff.push_str("# Staged changes\n");
            diff.push_str(&git_diff(root, true, &record.path)?);
        }
        if unstaged {
            if !diff.is_empty() { diff.push('\n'); }
            diff.push_str("# Working tree changes\n");
            diff.push_str(&git_diff(root, false, &record.path)?);
        }
        binary |= diff.contains("Binary files ") || diff.contains("GIT binary patch");
        if binary {
            let size = fs::metadata(&file_path).map(|metadata| metadata.len()).unwrap_or(0);
            diff = format!("Binary file · {size} bytes");
        }
    }
    if diff.len() > MAX_DIFF_BYTES {
        diff.truncate(MAX_DIFF_BYTES);
        diff.push_str("\n… diff preview truncated …\n");
        truncated = true;
    }
    Ok(GitChangeFile {
        path: record.path,
        status: status_label(record.x, record.y).into(),
        staged,
        unstaged,
        untracked,
        binary,
        truncated,
        diff,
    })
}

fn git_diff(root: &Path, cached: bool, path: &str) -> Result<String, String> {
    let mut command = Command::new("git");
    command.args(["-C"]).arg(root).args(["diff", "--no-ext-diff", "--binary"]);
    if cached { command.arg("--cached"); }
    let output = command.arg("--").arg(path).output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn is_binary_file(path: &Path) -> bool {
    fs::read(path)
        .ok()
        .is_some_and(|bytes| bytes.iter().take(BINARY_SCAN_BYTES).any(|byte| *byte == 0))
}

fn status_label(x: u8, y: u8) -> &'static str {
    if x == b'?' && y == b'?' { return "untracked"; }
    if x == b'A' { return "added"; }
    if x == b'D' || y == b'D' { return "deleted"; }
    if x == b'R' || y == b'R' { return "renamed"; }
    if x == b'C' || y == b'C' { return "copied"; }
    if x == b'U' || y == b'U' { return "conflict"; }
    "modified"
}

fn git_output(path: &Path, args: &[&str]) -> Result<String, String> {
    let output = git_bytes(path, args)?;
    Ok(String::from_utf8_lossy(&output).into_owned())
}

fn git_bytes(path: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(path)
        .args(args)
        .output()
        .map_err(|error| format!("Could not start git: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn git(path: &Path, args: &[&OsStr]) {
        let status = Command::new("git").arg("-C").arg(path).args(args).status().unwrap();
        assert!(status.success());
    }

    #[test]
    fn reports_existing_and_untracked_changes() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &[OsStr::new("init")]);
        git(root, &[OsStr::new("config"), OsStr::new("user.email"), OsStr::new("test@example.com")]);
        git(root, &[OsStr::new("config"), OsStr::new("user.name"), OsStr::new("Test")]);
        fs::write(root.join("tracked file.txt"), "first\n").unwrap();
        git(root, &[OsStr::new("add"), OsStr::new(".")]);
        git(root, &[OsStr::new("commit"), OsStr::new("-m"), OsStr::new("initial")]);
        fs::write(root.join("tracked file.txt"), "changed\n").unwrap();
        fs::write(root.join("new\nfile.txt"), "hello\n").unwrap();

        let result = changes(root).unwrap();
        assert_eq!(result.files.len(), 2);
        assert!(result.files.iter().any(|file| file.path == "tracked file.txt" && file.diff.contains("changed")));
        assert!(result.files.iter().any(|file| file.path == "new\nfile.txt" && file.untracked));
    }

    #[test]
    fn reports_staged_deleted_and_binary_files_with_bounded_previews() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &[OsStr::new("init")]);
        git(root, &[OsStr::new("config"), OsStr::new("user.email"), OsStr::new("test@example.com")]);
        git(root, &[OsStr::new("config"), OsStr::new("user.name"), OsStr::new("Test")]);
        fs::write(root.join("staged.txt"), "before\n").unwrap();
        fs::write(root.join("deleted.txt"), "remove me\n").unwrap();
        git(root, &[OsStr::new("add"), OsStr::new(".")]);
        git(root, &[OsStr::new("commit"), OsStr::new("-m"), OsStr::new("initial")]);

        fs::write(root.join("staged.txt"), "after\n").unwrap();
        git(root, &[OsStr::new("add"), OsStr::new("staged.txt")]);
        fs::remove_file(root.join("deleted.txt")).unwrap();
        fs::write(root.join("binary.dat"), [0, 1, 2, 3]).unwrap();
        fs::write(root.join("large.txt"), "x".repeat(MAX_DIFF_BYTES + 10_000)).unwrap();

        let result = changes(root).unwrap();
        assert!(result.files.iter().any(|file| file.path == "staged.txt" && file.staged && file.diff.contains("# Staged changes")));
        assert!(result.files.iter().any(|file| file.path == "deleted.txt" && file.status == "deleted" && file.unstaged));
        assert!(result.files.iter().any(|file| file.path == "binary.dat" && file.binary && file.diff.contains("4 bytes")));
        let large = result.files.iter().find(|file| file.path == "large.txt").unwrap();
        assert!(large.truncated);
        assert!(large.diff.len() <= MAX_DIFF_BYTES + 40);
    }

    #[test]
    fn repositories_without_commits_are_detected() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &[OsStr::new("init")]);
        let info = inspect_project(directory.path());
        assert!(info.root.is_some());
        assert!(!info.has_head);
    }

    #[test]
    fn worktrees_start_at_head_without_copying_uncommitted_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("repo");
        fs::create_dir(&root).unwrap();
        git(&root, &[OsStr::new("init")]);
        git(&root, &[OsStr::new("config"), OsStr::new("user.email"), OsStr::new("test@example.com")]);
        git(&root, &[OsStr::new("config"), OsStr::new("user.name"), OsStr::new("Test")]);
        fs::write(root.join("tracked.txt"), "committed\n").unwrap();
        git(&root, &[OsStr::new("add"), OsStr::new(".")]);
        git(&root, &[OsStr::new("commit"), OsStr::new("-m"), OsStr::new("initial")]);
        let expected_head = git_output(&root, &["rev-parse", "HEAD"]).unwrap();
        fs::write(root.join("tracked.txt"), "dirty original\n").unwrap();
        fs::write(root.join("untracked.txt"), "only original\n").unwrap();

        let destination = directory.path().join("worktree");
        let workspace = create_worktree(&root, &root, &destination, "wackcode/test-worktree").unwrap();
        assert_eq!(workspace, destination);
        assert_eq!(fs::read_to_string(workspace.join("tracked.txt")).unwrap(), "committed\n");
        assert!(!workspace.join("untracked.txt").exists());
        assert_eq!(git_output(&workspace, &["rev-parse", "HEAD"]).unwrap(), expected_head);
        assert_eq!(current_branch(&workspace).as_deref(), Some("wackcode/test-worktree"));
        assert_eq!(fs::read_to_string(root.join("tracked.txt")).unwrap(), "dirty original\n");
    }
}
