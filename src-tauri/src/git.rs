use crate::models::{GitChangeFile, GitChanges, GitDiffHunk, GitDiffLine, GitDiffSection, GitPublishInfo};
use std::{fs, hash::{Hash, Hasher}, path::{Component, Path, PathBuf}, process::{Command, Stdio}, thread, time::{Duration, Instant}};

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

/// The repository one working tree belongs to: `--git-common-dir` names the shared `.git`
/// directory (a worktree's own `.git` is a file pointing there), so every worktree of a
/// repository resolves to the same root. None outside a repository.
pub fn common_root(path: &Path) -> Option<PathBuf> {
    let common = git_output(path, &["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .ok()
        .map(|value| PathBuf::from(value.trim_end()))?;
    common.parent().map(Path::to_path_buf)
}

/// The commit `HEAD` points at in the checkout or worktree at `path`.
pub fn head_commit(path: &Path) -> Option<String> {
    git_output(path, &["rev-parse", "--verify", "--quiet", "HEAD"]).ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Whether `commit` names a commit the repository at `path` still has.
pub fn has_commit(path: &Path, commit: &str) -> bool {
    commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        && git_output(path, &["cat-file", "-e", &format!("{commit}^{{commit}}")]).is_ok()
}

pub fn current_branch(path: &Path) -> Option<String> {
    git_output(path, &["branch", "--show-current"])
        .ok()
        .map(|branch| branch.trim().to_string())
        .filter(|branch| !branch.is_empty())
}

/// Add a worktree on a new `branch` starting at `base` (a commit, or `HEAD` of the checkout).
pub fn create_worktree(
    project_path: &Path,
    git_root: &Path,
    destination: &Path,
    branch: &str,
    base: &str,
) -> Result<PathBuf, String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let output = Command::new("git")
        .args(["-C"])
        .arg(git_root)
        .args(["worktree", "add", "-b", branch])
        .arg(destination)
        .arg(base)
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
        return Ok(GitChanges { is_git: false, root: None, branch: None, files: Vec::new(), changes_revision: String::new() });
    };
    let status_output = git_bytes(&root, &["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
    let records = parse_status(&status_output);
    let mut files = Vec::with_capacity(records.len());
    for record in records {
        files.push(build_change(&root, record)?);
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for file in &files {
        file.path.hash(&mut hasher);
        file.status.hash(&mut hasher);
        for section in &file.sections {
            section.revision.hash(&mut hasher);
        }
    }
    Ok(GitChanges {
        is_git: true,
        root: Some(root.to_string_lossy().into_owned()),
        branch: current_branch(&root),
        files,
        changes_revision: format!("{:016x}", hasher.finish()),
    })
}

#[derive(Debug)]
struct StatusRecord {
    path: String,
    old_path: Option<String>,
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
        let renamed = x == b'R' || x == b'C' || y == b'R' || y == b'C';
        let old_path = if renamed { entries.get(index + 1).map(|entry| String::from_utf8_lossy(entry).into_owned()) } else { None };
        records.push(StatusRecord { path, old_path, x, y });
        if renamed { index += 2; } else { index += 1; }
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
    let mut sections = Vec::new();
    let mut untracked_additions: Option<usize> = None;

    if untracked {
        if fs::symlink_metadata(&file_path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            diff = "Symbolic link (target is not previewed)".into();
        } else { match fs::read(&file_path) {
            Ok(content) if binary => {
                diff = format!("Binary file · {} bytes", content.len());
            }
            Ok(content) => {
                let text = String::from_utf8_lossy(&content);
                // The preview loop below stops at MAX_DIFF_BYTES; keep the true total for the UI.
                untracked_additions = Some(text.lines().count());
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
        }}
    } else {
        if staged {
            diff.push_str("# Staged changes\n");
            let raw = git_diff(root, true, &record.path)?;
            sections.push(section("staged", &raw));
            diff.push_str(&raw);
        }
        if unstaged {
            if !diff.is_empty() { diff.push('\n'); }
            diff.push_str("# Working tree changes\n");
            let raw = git_diff(root, false, &record.path)?;
            sections.push(section("working", &raw));
            diff.push_str(&raw);
        }
        binary |= diff.contains("Binary files ") || diff.contains("GIT binary patch");
        if binary {
            let size = fs::metadata(&file_path).map(|metadata| metadata.len()).unwrap_or(0);
            diff = format!("Binary file · {size} bytes");
        }
    }
    if untracked {
        let mut working = section("working", &diff);
        if let Some(total) = untracked_additions { working.additions = total; }
        sections.push(working);
    }
    if diff.len() > MAX_DIFF_BYTES {
        diff.truncate(MAX_DIFF_BYTES);
        diff.push_str("\n… diff preview truncated …\n");
        truncated = true;
    }
    let hunkable = status_label(record.x, record.y) == "modified" && !binary && !truncated && record.old_path.is_none()
        && fs::symlink_metadata(&file_path).is_ok_and(|metadata| metadata.is_file());
    Ok(GitChangeFile {
        path: record.path,
        old_path: record.old_path,
        status: status_label(record.x, record.y).into(),
        staged,
        unstaged,
        untracked,
        binary,
        hunkable,
        truncated,
        diff,
        sections,
    })
}

fn revision(bytes: &[u8]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn count_changed_lines(raw: &str) -> (usize, usize) {
    let mut additions = 0;
    let mut deletions = 0;
    for line in raw.lines() {
        if line.starts_with('+') && !line.starts_with("+++") { additions += 1; }
        else if line.starts_with('-') && !line.starts_with("---") { deletions += 1; }
    }
    (additions, deletions)
}

fn section(layer: &str, raw: &str) -> GitDiffSection {
    let mut hunks: Vec<GitDiffHunk> = Vec::new();
    let mut old = 0;
    let mut new = 0;
    let (additions, deletions) = count_changed_lines(raw);
    let preview_end = raw.char_indices().map(|(index, _)| index).chain(std::iter::once(raw.len()))
        .take_while(|index| *index <= MAX_DIFF_BYTES).last().unwrap_or(0);
    for line in raw[..preview_end].lines() {
        if line.starts_with("@@ ") {
            let ranges: Vec<&str> = line.split_whitespace().take(3).collect();
            old = ranges.get(1).and_then(|range| range.trim_start_matches('-').split(',').next())
                .and_then(|value| value.parse().ok()).unwrap_or(0);
            new = ranges.get(2).and_then(|range| range.trim_start_matches('+').split(',').next())
                .and_then(|value| value.parse().ok()).unwrap_or(0);
            hunks.push(GitDiffHunk { id: hunks.len(), header: line.to_string(), old_start: old, new_start: new, lines: Vec::new() });
            continue;
        }
        let Some(hunk) = hunks.last_mut() else { continue };
        let (kind, old_line, new_line) = if line.starts_with('+') && !line.starts_with("+++") {
            let number = new; new += 1; ("addition", None, Some(number))
        } else if line.starts_with('-') && !line.starts_with("---") {
            let number = old; old += 1; ("deletion", Some(number), None)
        } else if line.starts_with(' ') {
            let a = old; let b = new; old += 1; new += 1; ("context", Some(a), Some(b))
        } else { ("meta", None, None) };
        hunk.lines.push(GitDiffLine { kind: kind.into(), text: line.to_string(), old_line, new_line });
    }
    GitDiffSection { layer: layer.into(), revision: revision(raw.as_bytes()), diff: raw[..preview_end].to_string(), hunks, truncated: raw.len() > MAX_DIFF_BYTES, additions, deletions }
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

fn checked_path(root: &Path, path: &str) -> Result<(), String> {
    if path.is_empty() || Path::new(path).components().any(|part| !matches!(part, Component::Normal(_))) {
        return Err("Invalid repository path".into());
    }
    let mut parent = root.to_path_buf();
    for part in Path::new(path).parent().unwrap_or(Path::new("")).components() {
        parent.push(part);
        match fs::symlink_metadata(&parent) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err("Cannot change a file through a symlinked directory".into()),
            Ok(_) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn run_git(root: &Path, args: &[&str], input: Option<&[u8]>) -> Result<String, String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).args(args);
    if input.is_some() { command.stdin(Stdio::piped()); }
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|error| format!("Could not start git: {error}"))?;
    if let Some(input) = input {
        use std::io::Write;
        child.stdin.take().ok_or("Could not send patch to git")?.write_all(input).map_err(|error| error.to_string())?;
    }
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if child.try_wait().map_err(|error| error.to_string())?.is_some() { break; }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Git did not finish in time".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The renderer identifies a displayed section; the patch always comes from a fresh Git diff.
pub fn change_action(path: &Path, file: &str, layer: &str, action: &str, hunk_id: Option<usize>, expected: &str) -> Result<GitChanges, String> {
    let snapshot = changes(path)?;
    let root = snapshot.root.as_deref().ok_or("No Git repository")?;
    let root = Path::new(root);
    checked_path(root, file)?;
    let item = snapshot.files.iter().find(|item| item.path == file).ok_or("Changes have moved; refresh and try again")?;
    let section = item.sections.iter().find(|section| section.layer == layer).ok_or("Changes have moved; refresh and try again")?;
    if section.revision != expected { return Err("Changes have moved; refresh and try again".into()); }
    if item.status == "conflict" { return Err("Resolve this conflict before changing its staging state".into()); }
    if let Some(id) = hunk_id {
        if !item.hunkable || section.truncated {
            return Err("This change only supports whole-file actions".into());
        }
        let _hunk = section.hunks.get(id).ok_or("Hunk no longer exists")?;
        let raw = git_diff(root, layer == "staged", file)?;
        let start = raw.match_indices("\n@@ ").next().map(|(index, _)| index + 1).ok_or("Hunk no longer exists")?;
        let header = &raw[..start];
        let chunks: Vec<&str> = raw[start..].split_inclusive('\n').collect();
        let mut found = 0;
        let mut chosen = String::new();
        let mut taking = false;
        for line in chunks {
            if line.starts_with("@@ ") {
                if taking { break; }
                taking = found == id;
                found += 1;
            }
            if taking { chosen.push_str(line); }
        }
        if chosen.is_empty() { return Err("Hunk no longer exists".into()); }
        let patch = format!("{header}{chosen}");
        let mut args = vec!["apply"];
        if action == "stage" || action == "unstage" { args.push("--cached"); }
        if action == "unstage" || action == "discard" { args.push("--reverse"); }
        if !matches!((layer, action), ("working", "stage" | "discard") | ("staged", "unstage" | "discard")) {
            return Err("Unsupported change action".into());
        }
        // Discarding a staged hunk rewrites index and worktree; the others touch one side only.
        if (layer, action) == ("staged", "discard") { args = vec!["apply", "--index", "--reverse"]; }
        let mut check = args.clone(); check.push("--check");
        run_git(root, &check, Some(patch.as_bytes()))?;
        run_git(root, &args, Some(patch.as_bytes()))?;
    } else {
        match (layer, action) {
            ("working", "stage") => { run_git(root, &["add", "--", file], None)?; }
            ("staged", "unstage") => {
                if inspect_project(root).has_head { run_git(root, &["restore", "--staged", "--", file], None)?; }
                else { run_git(root, &["rm", "--cached", "--", file], None)?; }
            }
            ("working", "discard") if item.untracked => {
                let target = root.join(file);
                let meta = fs::symlink_metadata(&target).map_err(|error| error.to_string())?;
                if !meta.is_file() && !meta.file_type().is_symlink() { return Err("Only files can be discarded".into()); }
                fs::remove_file(target).map_err(|error| error.to_string())?;
            }
            ("working", "discard") => { run_git(root, &["restore", "--worktree", "--", file], None)?; }
            ("staged", "discard") if inspect_project(root).has_head => {
                run_git(root, &["restore", "--staged", "--worktree", "--", file], None)?;
            }
            // No HEAD yet: the staged entry is the file's only history, so unstage and remove it.
            ("staged", "discard") => {
                run_git(root, &["rm", "-f", "--cached", "--", file], None)?;
                let target = root.join(file);
                if fs::symlink_metadata(&target).is_ok() { fs::remove_file(target).map_err(|error| error.to_string())?; }
            }
            _ => return Err("Unsupported change action".into()),
        }
    }
    changes(path)
}

/// Commit changed files in one step: `files` names the scope (empty = every changed file).
/// The panel doesn't expose staging, so this stages the targets and commits them together,
/// guarded by `changes_revision` so a moved working tree is rejected rather than committed blind.
pub fn commit(path: &Path, message: &str, files: &[String], expected: &str) -> Result<GitChanges, String> {
    if message.trim().is_empty() { return Err("Write a commit message".into()); }
    let snapshot = changes(path)?;
    if snapshot.changes_revision != expected { return Err("Changes have moved; refresh and review the commit".into()); }
    let root = Path::new(snapshot.root.as_deref().ok_or("No Git repository")?);
    let mut targets: Vec<&str> = Vec::new();
    if files.is_empty() {
        targets.extend(snapshot.files.iter().map(|file| file.path.as_str()));
    } else {
        for file in files {
            if !snapshot.files.iter().any(|item| item.path == *file) { return Err("Changes have moved; refresh and try again".into()); }
            targets.push(file);
        }
    }
    if targets.is_empty() { return Err("Nothing to commit".into()); }
    if snapshot.files.iter().any(|file| targets.contains(&file.path.as_str()) && file.status == "conflict") {
        return Err("Resolve conflicts before committing".into());
    }
    for file in &targets {
        checked_path(root, file)?;
    }
    let mut add = vec!["add", "--"];
    add.extend(targets.iter().copied());
    run_git(root, &add, None)?;
    if files.is_empty() {
        run_git(root, &["commit", "-m", message], None)?;
    } else {
        let mut args = vec!["commit", "-m", message, "--"];
        args.extend(targets.iter().copied());
        run_git(root, &args, None)?;
    }
    changes(path)
}

pub fn publish_info(path: &Path) -> Result<GitPublishInfo, String> {
    let root = inspect_project(path).root.ok_or("No Git repository")?;
    let branch = current_branch(&root);
    let upstream = run_git(&root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"], None).ok();
    let remotes = run_git(&root, &["remote"], None)?.lines().map(str::to_owned).collect();
    Ok(GitPublishInfo { branch, upstream, remotes })
}

pub fn push_args(info: GitPublishInfo, remote: Option<String>) -> Result<Vec<String>, String> {
    let branch = info.branch.ok_or("Check out a branch before pushing")?;
    if let Some(upstream) = info.upstream {
        if remote.is_some() { return Err("This branch already has an upstream".into()); }
        let name = info.remotes.iter().filter(|name| upstream.starts_with(&format!("{name}/"))).max_by_key(|name| name.len())
            .ok_or("Could not identify the upstream remote")?;
        let target = upstream.strip_prefix(&format!("{name}/")).ok_or("Invalid upstream branch")?;
        Ok(vec!["push".into(), "--".into(), name.clone(), format!("HEAD:refs/heads/{target}")])
    } else {
        let selected = remote.ok_or("Choose a remote for the first push")?;
        if !info.remotes.contains(&selected) { return Err("Choose a configured Git remote".into()); }
        Ok(vec!["push".into(), "--set-upstream".into(), "--".into(), selected, branch])
    }
}

pub fn remote_repo(path: &Path, remote: &str) -> Result<String, String> {
    let root = inspect_project(path).root.ok_or("No Git repository")?;
    if !publish_info(&root)?.remotes.iter().any(|name| name == remote) { return Err("Choose a configured Git remote".into()); }
    let url = run_git(&root, &["remote", "get-url", remote], None)?;
    let source = if let Some(value) = url.strip_prefix("https://") { value }
        else if let Some(value) = url.strip_prefix("git@") { value }
        else if let Some(value) = url.strip_prefix("ssh://git@") { value }
        else { return Err("This remote does not have a GitHub HTTPS or SSH URL".into()); };
    let source = source.replace(':', "/");
    let parts: Vec<&str> = source.trim_end_matches(".git").split('/').collect();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) { return Err("Could not identify the GitHub repository for this remote".into()); }
    Ok(format!("{}/{}/{}", parts[0], parts[1], parts[2]))
}

fn is_binary_file(path: &Path) -> bool {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) { return false; }
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

/// Tracked and untracked, non-ignored files under `path`, relative to it. Fails outside a repository.
pub fn ls_files(path: &Path) -> Result<Vec<String>, String> {
    let bytes = git_bytes(path, &["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--deduplicate"])?;
    Ok(bytes.split(|byte| *byte == 0).filter(|entry| !entry.is_empty()).map(|entry| String::from_utf8_lossy(entry).into_owned()).collect())
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
        let untracked = result.files.iter().find(|file| file.path == "new\nfile.txt").unwrap();
        let staged = change_action(root, "new\nfile.txt", "working", "stage", None, &untracked.sections[0].revision).unwrap();
        assert!(staged.files.iter().any(|file| file.path == "new\nfile.txt" && file.staged));
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
        let staged_file = result.files.iter().find(|file| file.path == "staged.txt").unwrap();
        let staged_section = staged_file.sections.iter().find(|section| section.layer == "staged").unwrap();
        assert_eq!((staged_section.additions, staged_section.deletions), (1, 1));
        let large = result.files.iter().find(|file| file.path == "large.txt").unwrap();
        assert!(large.truncated);
        assert!(large.diff.len() <= MAX_DIFF_BYTES + 40);
        // Untracked counts are exact even though the preview cut the diff off.
        assert_eq!(large.sections[0].additions, 1);
        assert_eq!(large.sections[0].deletions, 0);
    }

    #[test]
    fn commit_stages_its_scope_honors_hooks_and_rejects_stale_revisions() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &[OsStr::new("init")]);
        git(root, &[OsStr::new("config"), OsStr::new("user.email"), OsStr::new("test@example.com")]);
        git(root, &[OsStr::new("config"), OsStr::new("user.name"), OsStr::new("Test")]);
        fs::write(root.join("tracked.txt"), "base\n").unwrap();
        git(root, &[OsStr::new("add"), OsStr::new(".")]);
        git(root, &[OsStr::new("commit"), OsStr::new("-m"), OsStr::new("initial")]);
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        fs::write(root.join("new.txt"), "brand new\n").unwrap();

        let first = changes(root).unwrap();
        assert!(commit(root, "nope", &[], "stale-revision").unwrap_err().contains("moved"));
        // A scoped commit stages only its files; everything else stays untouched.
        let scoped = commit(root, "only tracked", &["tracked.txt".to_string()], &first.changes_revision).unwrap();
        assert_eq!(run_git(root, &["show", "--format=", "--name-only", "HEAD"], None).unwrap(), "tracked.txt");
        assert!(scoped.files.iter().any(|file| file.path == "new.txt" && file.untracked));

        let hook = root.join(".git/hooks/pre-commit");
        fs::write(&hook, "#!/bin/sh\necho hook refused >&2\nexit 1\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
        let second = changes(root).unwrap();
        assert!(commit(root, "add the rest", &[], &second.changes_revision).unwrap_err().contains("hook refused"));
        fs::remove_file(&hook).unwrap();
        // The failed commit still staged its files, so the revision moved — re-snapshot.
        let third = changes(root).unwrap();
        let clean = commit(root, "add the rest", &[], &third.changes_revision).unwrap();
        assert!(clean.files.is_empty());
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
        let workspace = create_worktree(&root, &root, &destination, "wackcode/test-worktree", "HEAD").unwrap();
        assert_eq!(workspace, destination);
        assert_eq!(fs::read_to_string(workspace.join("tracked.txt")).unwrap(), "committed\n");
        assert!(!workspace.join("untracked.txt").exists());
        assert_eq!(git_output(&workspace, &["rev-parse", "HEAD"]).unwrap(), expected_head);
        assert_eq!(current_branch(&workspace).as_deref(), Some("wackcode/test-worktree"));
        assert_eq!(fs::read_to_string(root.join("tracked.txt")).unwrap(), "dirty original\n");
    }

    #[test]
    fn worktrees_can_start_at_an_earlier_commit() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("repo");
        fs::create_dir(&root).unwrap();
        git(&root, &[OsStr::new("init")]);
        git(&root, &[OsStr::new("config"), OsStr::new("user.email"), OsStr::new("test@example.com")]);
        git(&root, &[OsStr::new("config"), OsStr::new("user.name"), OsStr::new("Test")]);
        fs::write(root.join("file.txt"), "first\n").unwrap();
        git(&root, &[OsStr::new("add"), OsStr::new(".")]);
        git(&root, &[OsStr::new("commit"), OsStr::new("-m"), OsStr::new("first")]);
        let first = head_commit(&root).unwrap();
        fs::write(root.join("file.txt"), "second\n").unwrap();
        git(&root, &[OsStr::new("commit"), OsStr::new("-am"), OsStr::new("second")]);
        assert!(has_commit(&root, &first));
        assert!(!has_commit(&root, &"0".repeat(40)));
        assert!(!has_commit(&root, "HEAD --output=x"));

        let destination = directory.path().join("worktree");
        create_worktree(&root, &root, &destination, "wackcode/earlier", &first).unwrap();
        assert_eq!(fs::read_to_string(destination.join("file.txt")).unwrap(), "first\n");
        assert_eq!(head_commit(&destination).as_deref(), Some(first.as_str()));
    }

    #[test]
    fn hunk_actions_preserve_the_other_hunk_and_reject_stale_previews() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &[OsStr::new("init")]);
        git(root, &[OsStr::new("config"), OsStr::new("user.email"), OsStr::new("test@example.com")]);
        git(root, &[OsStr::new("config"), OsStr::new("user.name"), OsStr::new("Test")]);
        let original = (1..=25).map(|line| format!("line {line}\n")).collect::<String>();
        fs::write(root.join("notes.txt"), &original).unwrap();
        git(root, &[OsStr::new("add"), OsStr::new("notes.txt")]);
        git(root, &[OsStr::new("commit"), OsStr::new("-m"), OsStr::new("initial")]);
        let changed = original.replace("line 2\n", "line two\n").replace("line 22\n", "line twenty two\n");
        fs::write(root.join("notes.txt"), changed).unwrap();

        let first = changes(root).unwrap();
        let working = &first.files[0].sections[0];
        assert_eq!(working.hunks.len(), 2);
        let staged = change_action(root, "notes.txt", "working", "stage", Some(0), &working.revision).unwrap();
        assert!(run_git(root, &["show", ":notes.txt"], None).unwrap().contains("line two"));
        assert!(!run_git(root, &["show", ":notes.txt"], None).unwrap().contains("line twenty two"));
        let stale = change_action(root, "notes.txt", "working", "stage", Some(1), &working.revision);
        assert!(stale.unwrap_err().contains("moved"));
        let file = &staged.files[0];
        let working = file.sections.iter().find(|section| section.layer == "working").unwrap();
        let after_discard = change_action(root, "notes.txt", "working", "discard", Some(0), &working.revision).unwrap();
        assert!(!fs::read_to_string(root.join("notes.txt")).unwrap().contains("line twenty two"));
        let staged_section = after_discard.files[0].sections.iter().find(|section| section.layer == "staged").unwrap();
        change_action(root, "notes.txt", "staged", "unstage", Some(0), &staged_section.revision).unwrap();
        assert_eq!(run_git(root, &["show", ":notes.txt"], None).unwrap(), original.trim_end());
        assert!(fs::read_to_string(root.join("notes.txt")).unwrap().contains("line two"));
    }

    #[test]
    fn discard_untracked_file_refuses_symlinked_parent() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &[OsStr::new("init")]);
        fs::write(root.join("new file.txt"), "content\n").unwrap();
        let snapshot = changes(root).unwrap();
        let section = &snapshot.files[0].sections[0];
        change_action(root, "new file.txt", "working", "discard", None, &section.revision).unwrap();
        assert!(!root.join("new file.txt").exists());
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("keep.txt"), "keep").unwrap();
        symlink(outside.path(), root.join("outside")).unwrap();
        symlink(outside.path().join("keep.txt"), root.join("link.txt")).unwrap();
        let link = changes(root).unwrap().files.into_iter().find(|file| file.path == "link.txt").unwrap();
        assert!(!link.diff.contains("keep"));
        assert!(change_action(root, "outside/keep.txt", "working", "discard", None, "unknown").is_err());
        assert_eq!(fs::read_to_string(outside.path().join("keep.txt")).unwrap(), "keep");
    }

    #[test]
    fn pushes_only_the_selected_branch_to_a_local_bare_remote() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("checkout");
        let bare = directory.path().join("remote.git");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&bare).unwrap();
        git(&root, &[OsStr::new("init")]);
        git(&bare, &[OsStr::new("init"), OsStr::new("--bare")]);
        git(&root, &[OsStr::new("config"), OsStr::new("user.email"), OsStr::new("test@example.com")]);
        git(&root, &[OsStr::new("config"), OsStr::new("user.name"), OsStr::new("Test")]);
        fs::write(root.join("file.txt"), "first\n").unwrap();
        git(&root, &[OsStr::new("add"), OsStr::new("file.txt")]);
        git(&root, &[OsStr::new("commit"), OsStr::new("-m"), OsStr::new("first")]);
        git(&root, &[OsStr::new("remote"), OsStr::new("add"), OsStr::new("origin"), bare.as_os_str()]);
        let first = publish_info(&root).unwrap();
        assert!(first.upstream.is_none());
        let args = push_args(first, Some("origin".into())).unwrap();
        run_git(&root, &args.iter().map(String::as_str).collect::<Vec<_>>(), None).unwrap();
        let second = publish_info(&root).unwrap();
        assert!(second.upstream.as_deref().unwrap().starts_with("origin/"));
        fs::write(root.join("file.txt"), "second\n").unwrap();
        git(&root, &[OsStr::new("commit"), OsStr::new("-am"), OsStr::new("second")]);
        let args = push_args(second, None).unwrap();
        assert_eq!(args[0], "push");
        assert_eq!(args[1], "--");
        run_git(&root, &args.iter().map(String::as_str).collect::<Vec<_>>(), None).unwrap();
        let branch = current_branch(&root).unwrap();
        assert_eq!(run_git(&bare, &["show", &format!("refs/heads/{branch}:file.txt")], None).unwrap(), "second");
        git(&root, &[OsStr::new("remote"), OsStr::new("set-url"), OsStr::new("origin"), OsStr::new("git@github.com:owner/repo.git")]);
        assert_eq!(remote_repo(&root, "origin").unwrap(), "github.com/owner/repo");
    }
}
