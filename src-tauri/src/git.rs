use crate::models::{
    GitBranch, GitBranches, GitChangeFile, GitChanges, GitCommit, GitCommitFile, GitCommitFiles, GitDiffHunk, GitDiffLine,
    GitDiffSection, GitLogPage, GitPublishInfo, GitPullOutcome, GitRevertOutcome, GitRevertResult, GitSyncStatus, GitUndoResult,
};
use std::{collections::{HashMap, HashSet}, fs, hash::{Hash, Hasher}, path::{Component, Path, PathBuf}, process::{Command, Stdio}, thread, time::{Duration, Instant}};

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
        // Git's own markers start a line; changed content always starts with `+`, `-` or a space,
        // so a line of code that mentions them can't pass for one.
        binary |= diff.lines().any(|line| line.starts_with("Binary files ") || line == "GIT binary patch");
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
    // `--no-color`: a user's `color.ui=always` would otherwise break the hunk parser.
    command.args(["-C"]).arg(root).args(["diff", "--no-ext-diff", "--no-color", "--binary"]);
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
    // A staged rename is listed under its new path; its old path must be in the commit's
    // pathspec too, or `commit -- new` records the new file and leaves the deletion behind.
    let mut commit_paths = targets.clone();
    for file in snapshot.files.iter().filter(|file| targets.contains(&file.path.as_str())) {
        if let Some(old) = file.old_path.as_deref() {
            if !commit_paths.contains(&old) { commit_paths.push(old); }
        }
    }
    for file in &commit_paths {
        checked_path(root, file)?;
    }
    // Only files with unstaged or untracked changes need adding: a fully staged one is already
    // what gets committed, and `git add` refuses a staged deletion ("did not match any files").
    let unstaged: Vec<&str> = snapshot.files.iter()
        .filter(|file| targets.contains(&file.path.as_str()) && (file.unstaged || file.untracked))
        .map(|file| file.path.as_str())
        .collect();
    if !unstaged.is_empty() {
        let mut add = vec!["add", "--"];
        add.extend(unstaged);
        run_git(root, &add, None)?;
    }
    // Once the scope is added, the index holds exactly the commit unless a file outside the
    // scope has staged changes of its own. Only then does the commit need a pathspec (`--only`
    // semantics), which on a case-insensitive disk also refuses case-only renames.
    let others_staged = snapshot.files.iter().any(|file| !targets.contains(&file.path.as_str()) && file.staged);
    if files.is_empty() || !others_staged {
        run_git(root, &["commit", "-m", message], None)?;
    } else {
        let mut args = vec!["commit", "-m", message, "--"];
        args.extend(commit_paths.iter().copied());
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

/// Most branches a picker lists; the newest commits win.
const MAX_BRANCHES: usize = 500;

/// Local branches, then remote-tracking branches that have no local branch of the same name,
/// each newest commit first. Reads refs only: nothing is fetched.
pub fn branches(path: &Path) -> Result<GitBranches, String> {
    let root = inspect_project(path).root.ok_or("No Git repository")?;
    let current = current_branch(&root);
    let output = run_git(&root, &[
        "for-each-ref", "--sort=-committerdate",
        "--format=%(refname)%00%(refname:short)%00%(worktreepath)",
        "refs/heads", "refs/remotes",
    ], None)?;
    let mut local = Vec::new();
    let mut remote = Vec::new();
    for line in output.lines() {
        let mut fields = line.split('\0');
        let (Some(full), Some(name), worktree) = (fields.next(), fields.next(), fields.next()) else { continue };
        if full.starts_with("refs/heads/") {
            let worktree = worktree.filter(|path| !path.is_empty() && current.as_deref() != Some(name)).map(str::to_owned);
            local.push(GitBranch { name: name.to_string(), remote: false, worktree });
        } else if !full.ends_with("/HEAD") {
            remote.push(GitBranch { name: name.to_string(), remote: true, worktree: None });
        }
    }
    // `origin/main` is shadowed by a local `main`: switching to it would only switch to `main`.
    remote.retain(|branch| branch.name.split_once('/').is_none_or(|(_, short)| !local.iter().any(|item| item.name == short)));
    local.extend(remote);
    local.truncate(MAX_BRANCHES);
    Ok(GitBranches { current, branches: local })
}

/// Switch the checkout at `path` to a branch. `kind` is `local` (an existing branch), `remote`
/// (create the local tracking branch for a remote-tracking ref) or `create` (a new branch at
/// `HEAD`). Git's own safety applies: it refuses to overwrite uncommitted changes and carries
/// the ones that don't conflict across.
pub fn checkout(path: &Path, name: &str, kind: &str) -> Result<Option<String>, String> {
    let root = inspect_project(path).root.ok_or("No Git repository")?;
    let name = name.trim();
    if name.is_empty() || name.starts_with('-') || run_git(&root, &["check-ref-format", "--branch", name], None).is_err() {
        return Err(format!("“{name}” is not a valid branch name"));
    }
    let args: Vec<&str> = match kind {
        "local" => {
            run_git(&root, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")], None)
                .map_err(|_| format!("There is no branch named {name}"))?;
            vec!["switch", "--", name]
        }
        "remote" => {
            run_git(&root, &["rev-parse", "--verify", "--quiet", &format!("refs/remotes/{name}")], None)
                .map_err(|_| format!("There is no remote branch named {name}"))?;
            vec!["switch", "--track", "--", name]
        }
        "create" => {
            if run_git(&root, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")], None).is_ok() {
                return Err(format!("A branch named {name} already exists"));
            }
            vec!["switch", "-c", name]
        }
        _ => return Err("Unknown branch action".into()),
    };
    run_git(&root, &args, None).map_err(|error| checkout_error(name, &error))?;
    Ok(current_branch(&root))
}

fn checkout_error(name: &str, stderr: &str) -> String {
    if stderr.contains("would be overwritten") {
        format!("Commit or discard your changes first: switching to {name} would overwrite them")
    } else if stderr.contains("already used by worktree") || stderr.contains("already checked out") {
        format!("{name} is checked out in another worktree")
    } else {
        format!("Could not switch to {name}: {}", stderr.lines().find(|line| !line.trim().is_empty()).unwrap_or(stderr).trim())
    }
}

/// The remote an upstream such as `origin/main` lives on, and its branch there. Remote names
/// may contain slashes, so the longest configured name that prefixes the upstream wins.
fn upstream_remote(info: &GitPublishInfo) -> Option<(String, String)> {
    let upstream = info.upstream.as_deref()?;
    let name = info.remotes.iter().filter(|name| upstream.starts_with(&format!("{name}/"))).max_by_key(|name| name.len())?;
    let branch = &upstream[name.len() + 1..];
    (!branch.is_empty()).then(|| (name.clone(), branch.to_string()))
}

/// Where Fetch goes: the upstream's remote, else `origin`, else the first remote.
pub fn fetch_remote(info: &GitPublishInfo) -> Option<String> {
    upstream_remote(info).map(|(name, _)| name)
        .or_else(|| info.remotes.iter().find(|name| *name == "origin").cloned())
        .or_else(|| info.remotes.first().cloned())
}

pub fn push_args(info: GitPublishInfo, remote: Option<String>) -> Result<Vec<String>, String> {
    if info.branch.is_none() { return Err("Check out a branch before pushing".into()); }
    if info.upstream.is_some() {
        if remote.is_some() { return Err("This branch already has an upstream".into()); }
        let (name, target) = upstream_remote(&info).ok_or("Could not identify the upstream remote")?;
        Ok(vec!["push".into(), "--".into(), name, format!("HEAD:refs/heads/{target}")])
    } else {
        let branch = info.branch.unwrap_or_default();
        let selected = remote.ok_or("Choose a remote for the first push")?;
        if !info.remotes.contains(&selected) { return Err("Choose a configured Git remote".into()); }
        Ok(vec!["push".into(), "--set-upstream".into(), "--".into(), selected, branch])
    }
}

/// `git fetch` arguments for `remote`. Network: run through the login-shell `git_cli`.
pub fn fetch_args(remote: &str) -> Vec<String> {
    vec!["fetch".into(), "--prune".into(), "--".into(), remote.into()]
}

/// Ahead/behind counts, upstream and last-fetch time from local refs (no network).
pub fn sync_status(path: &Path) -> Result<GitSyncStatus, String> {
    let info = inspect_project(path);
    let root = info.root.ok_or("No Git repository")?;
    let publish = publish_info(&root)?;
    let head = if info.has_head { head_commit(&root) } else { None };
    let (ahead, behind) = if head.is_none() {
        (0, 0)
    } else if publish.upstream.is_some() {
        run_git(&root, &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"], None).ok()
            .and_then(|output| {
                let mut counts = output.split_whitespace().map(|value| value.parse::<usize>().ok());
                Some((counts.next()??, counts.next()??))
            })
            .unwrap_or((0, 0))
    } else if !publish.remotes.is_empty() {
        let unpushed = run_git(&root, &["rev-list", "--count", "HEAD", "--not", "--remotes"], None).ok()
            .and_then(|output| output.parse().ok()).unwrap_or(0);
        (unpushed, 0)
    } else {
        (0, 0)
    };
    // `--git-path` resolves the per-worktree location, so this also works in a worktree.
    let fetched_at = run_git(&root, &["rev-parse", "--path-format=absolute", "--git-path", "FETCH_HEAD"], None).ok()
        .and_then(|path| fs::metadata(path).ok())
        .and_then(|metadata| metadata.modified().ok())
        .map(|time| chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339());
    Ok(GitSyncStatus {
        fetch_remote: fetch_remote(&publish),
        branch: publish.branch,
        upstream: publish.upstream,
        remotes: publish.remotes,
        ahead,
        behind,
        fetched_at,
        head,
        has_head: info.has_head,
    })
}

/// A merge, rebase, cherry-pick or revert Git is still in the middle of, by name.
fn operation_in_progress(root: &Path) -> Option<&'static str> {
    let git_dir = PathBuf::from(run_git(root, &["rev-parse", "--path-format=absolute", "--git-dir"], None).ok()?);
    [("MERGE_HEAD", "the merge"), ("rebase-merge", "the rebase"), ("rebase-apply", "the rebase"),
        ("CHERRY_PICK_HEAD", "the cherry-pick"), ("REVERT_HEAD", "the revert")]
        .into_iter()
        .find(|(name, _)| git_dir.join(name).exists())
        .map(|(_, label)| label)
}

fn refuse_mid_operation(root: &Path) -> Result<(), String> {
    match operation_in_progress(root) {
        Some(label) => Err(format!("Finish or abort {label} in progress first")),
        None => Ok(()),
    }
}

/// The local half of a fast-forward-only pull, after the caller fetched: never merges or
/// rebases. Both sides having commits is reported as `Diverged` with HEAD untouched.
pub fn fast_forward(path: &Path) -> Result<(GitPullOutcome, usize), String> {
    let root = inspect_project(path).root.ok_or("No Git repository")?;
    refuse_mid_operation(&root)?;
    let status = sync_status(&root)?;
    if status.branch.is_none() { return Err("Check out a branch before pulling".into()); }
    if status.upstream.is_none() { return Err("This branch has no upstream yet. Publish it first".into()); }
    if status.behind == 0 { return Ok((GitPullOutcome::UpToDate, 0)); }
    if status.ahead > 0 { return Ok((GitPullOutcome::Diverged, 0)); }
    run_git(&root, &["merge", "--ff-only", "--no-edit", "@{upstream}"], None).map_err(|error| pull_error(&error))?;
    Ok((GitPullOutcome::Updated, status.behind))
}

/// Git's refusal to overwrite local files, as a sentence that names them.
fn pull_error(stderr: &str) -> String {
    let files: Vec<&str> = stderr.lines().filter(|line| line.starts_with('\t')).map(str::trim).collect();
    let mut list = files.iter().take(5).copied().collect::<Vec<_>>().join(", ");
    if files.len() > 5 { list.push_str(&format!(" and {} more", files.len() - 5)); }
    let one = files.len() == 1;
    if stderr.contains("untracked working tree files would be overwritten") {
        format!("Pulling would overwrite the untracked {} {list}. Move or delete {}, then pull again.",
            if one { "file" } else { "files" }, if one { "it" } else { "them" })
    } else if stderr.contains("would be overwritten") {
        format!("Pulling would overwrite your changes to {list}. Commit or discard them, then pull again.")
    } else {
        format!("Could not pull: {}", stderr.lines().find(|line| !line.trim().is_empty()).unwrap_or(stderr).trim())
    }
}

const LOG_FORMAT: &str = "--format=%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%s%x1f%b";
/// Most commits the unpushed marker looks at; older unpushed commits simply show no marker.
const MAX_UNPUSHED_SCAN: usize = 2000;
const MAX_BODY_CHARS: usize = 20_000;

/// One page of HEAD's history, newest first.
pub fn log(path: &Path, skip: usize, limit: usize) -> Result<GitLogPage, String> {
    let info = inspect_project(path);
    let root = info.root.ok_or("No Git repository")?;
    if !info.has_head { return Ok(GitLogPage { commits: Vec::new(), has_more: false, head: None }); }
    let limit = limit.clamp(1, 200);
    let bytes = git_bytes(&root, &[
        "log", "-z", "--no-show-signature", "--no-color", LOG_FORMAT,
        &format!("--skip={skip}"), &format!("--max-count={}", limit + 1), "HEAD", "--",
    ])?;
    let remotes = !publish_info(&root)?.remotes.is_empty();
    let unpushed: Option<HashSet<String>> = if remotes {
        Some(run_git(&root, &["rev-list", &format!("--max-count={MAX_UNPUSHED_SCAN}"), "HEAD", "--not", "--remotes"], None)?
            .lines().map(str::to_owned).collect())
    } else {
        None
    };
    let mut commits = parse_log(&bytes, unpushed.as_ref());
    let has_more = commits.len() > limit;
    commits.truncate(limit);
    Ok(GitLogPage { commits, has_more, head: head_commit(&root) })
}

/// Records are NUL-separated; fields use the unit separator, with the free-form body last so a
/// stray separator inside it can't shift the other fields. No remote at all: every commit is
/// unpushed.
fn parse_log(bytes: &[u8], unpushed: Option<&HashSet<String>>) -> Vec<GitCommit> {
    String::from_utf8_lossy(bytes).split('\0').filter_map(|record| {
        let record = record.trim_start_matches('\n');
        if record.is_empty() { return None; }
        let mut fields = record.splitn(8, '\x1f');
        let sha = fields.next()?.to_string();
        let short_sha = fields.next()?.to_string();
        let parents = fields.next()?.split_whitespace().map(str::to_owned).collect();
        let author_name = fields.next()?.to_string();
        let author_email = fields.next()?.to_string();
        let authored_at = fields.next()?.to_string();
        let subject = fields.next()?.to_string();
        let body = fields.next().unwrap_or("").trim_end().chars().take(MAX_BODY_CHARS).collect();
        let unpushed = unpushed.is_none_or(|set| set.contains(&sha));
        Some(GitCommit { sha, short_sha, parents, author_name, author_email, authored_at, subject, body, unpushed })
    }).collect()
}

const MAX_COMMIT_FILES: usize = 5000;

fn commit_parents(root: &Path, sha: &str) -> Result<Vec<String>, String> {
    if !has_commit(root, sha) { return Err("That commit is no longer in this repository".into()); }
    let output = run_git(root, &["rev-list", "--parents", "-n1", sha], None)?;
    Ok(output.split_whitespace().skip(1).map(str::to_owned).collect())
}

/// `diff-tree` arguments that compare a commit with its first parent (merges included), or
/// with the empty tree for a root commit.
fn diff_tree_range(sha: &str, parents: &[String]) -> Vec<String> {
    match parents.first() {
        Some(parent) => vec![parent.clone(), sha.to_string()],
        None => vec!["--root".into(), sha.to_string()],
    }
}

fn commit_status(letter: &str) -> &'static str {
    match letter.chars().next() {
        Some('A') => "added",
        Some('D') => "deleted",
        Some('R') => "renamed",
        Some('C') => "copied",
        _ => "modified",
    }
}

/// The files a commit changed, with line counts.
pub fn commit_files(path: &Path, sha: &str) -> Result<GitCommitFiles, String> {
    let root = inspect_project(path).root.ok_or("No Git repository")?;
    let parents = commit_parents(&root, sha)?;
    let range = diff_tree_range(sha, &parents);
    let mut names = vec!["diff-tree".to_string(), "-r".into(), "-M".into(), "-z".into(), "--no-commit-id".into(), "--name-status".into()];
    names.extend(range.iter().cloned());
    let mut stats = vec!["diff-tree".to_string(), "-r".into(), "-M".into(), "-z".into(), "--no-commit-id".into(), "--numstat".into()];
    stats.extend(range);
    let names = git_bytes(&root, &names.iter().map(String::as_str).collect::<Vec<_>>())?;
    let stats = parse_numstat_z(&git_bytes(&root, &stats.iter().map(String::as_str).collect::<Vec<_>>())?);
    let mut files: Vec<GitCommitFile> = parse_name_status_z(&names).into_iter().map(|(path, old_path, letter)| {
        let (additions, deletions) = stats.get(&path).copied().unwrap_or((None, None));
        GitCommitFile { binary: additions.is_none() && deletions.is_none() && stats.contains_key(&path), status: commit_status(&letter).into(), path, old_path, additions, deletions }
    }).collect();
    let truncated = files.len() > MAX_COMMIT_FILES;
    files.truncate(MAX_COMMIT_FILES);
    Ok(GitCommitFiles { sha: sha.to_string(), files, truncated })
}

/// `--name-status -z`: `M\0path\0`, or `R100\0old\0new\0` for renames and copies.
fn parse_name_status_z(bytes: &[u8]) -> Vec<(String, Option<String>, String)> {
    let entries: Vec<String> = bytes.split(|byte| *byte == 0).filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).into_owned()).collect();
    let mut files = Vec::new();
    let mut index = 0;
    while index < entries.len() {
        let letter = entries[index].clone();
        if letter.starts_with('R') || letter.starts_with('C') {
            let (Some(old), Some(new)) = (entries.get(index + 1), entries.get(index + 2)) else { break };
            files.push((new.clone(), Some(old.clone()), letter));
            index += 3;
        } else {
            let Some(path) = entries.get(index + 1) else { break };
            files.push((path.clone(), None, letter));
            index += 2;
        }
    }
    files
}

/// `--numstat -z`: `add\tdel\tpath\0`, or `add\tdel\t\0old\0new\0` for renames. Binary files
/// report `-` for both counts. Keyed by the new path.
fn parse_numstat_z(bytes: &[u8]) -> HashMap<String, (Option<usize>, Option<usize>)> {
    let entries: Vec<String> = bytes.split(|byte| *byte == 0).filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).into_owned()).collect();
    let mut stats = HashMap::new();
    let mut index = 0;
    while index < entries.len() {
        let mut parts = entries[index].splitn(3, '\t');
        let additions = parts.next().and_then(|value| value.parse().ok());
        let deletions = parts.next().and_then(|value| value.parse().ok());
        let path = parts.next().unwrap_or("");
        if path.is_empty() {
            let Some(new) = entries.get(index + 2) else { break };
            stats.insert(new.clone(), (additions, deletions));
            index += 3;
        } else {
            stats.insert(path.to_string(), (additions, deletions));
            index += 1;
        }
    }
    stats
}

/// One file's change in a past commit, parsed like the working-tree diffs but in a read-only
/// `commit` section. Both paths of a rename go in the pathspec so Git still pairs them.
pub fn commit_diff(path: &Path, sha: &str, file: &str, old_path: Option<&str>) -> Result<GitChangeFile, String> {
    let root = inspect_project(path).root.ok_or("No Git repository")?;
    let parents = commit_parents(&root, sha)?;
    let mut args = vec!["--literal-pathspecs".to_string(), "diff-tree".into(), "-p".into(), "-r".into(), "-M".into(),
        "--no-commit-id".into(), "--no-ext-diff".into(), "--no-color".into()];
    args.extend(diff_tree_range(sha, &parents));
    args.push("--".into());
    if let Some(old) = old_path.filter(|old| *old != file) { args.push(old.into()); }
    args.push(file.into());
    let raw = git_output(&root, &args.iter().map(String::as_str).collect::<Vec<_>>())?;
    let binary = raw.lines().any(|line| line.starts_with("Binary files "));
    let status = if raw.contains("\nnew file mode ") { "added" }
        else if raw.contains("\ndeleted file mode ") { "deleted" }
        else if raw.contains("\nrename from ") { "renamed" }
        else { "modified" };
    let section = section("commit", &raw);
    Ok(GitChangeFile {
        path: file.to_string(),
        old_path: old_path.filter(|old| *old != file).map(str::to_owned),
        status: status.into(),
        staged: false,
        unstaged: false,
        untracked: false,
        binary,
        hunkable: false,
        truncated: section.truncated,
        sections: vec![section],
    })
}

/// A commit message as the commit form's two fields: the first non-empty line, then the rest.
/// Tolerates a model wrapping the message in a code fence.
pub fn split_commit_message(text: &str) -> (String, String) {
    let text = text.replace("\r\n", "\n");
    let mut body = text.trim();
    if body.starts_with("```") {
        body = body.split_once('\n').map_or("", |(_, rest)| rest);
        body = body.trim_end().strip_suffix("```").unwrap_or(body).trim();
    }
    let mut lines = body.lines();
    let summary: String = lines.by_ref().map(str::trim).find(|line| !line.is_empty()).unwrap_or("").chars().take(200).collect();
    let description = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    (summary, description)
}

/// Desktop's Undo: move the branch back past its latest commit and keep that commit's changes
/// staged. Only the unpushed tip of a branch, and never a merge or the first commit.
pub fn undo_commit(path: &Path, sha: &str) -> Result<GitUndoResult, String> {
    let root = inspect_project(path).root.ok_or("No Git repository")?;
    refuse_mid_operation(&root)?;
    if current_branch(&root).is_none() { return Err("Check out a branch before undoing a commit".into()); }
    if !has_commit(&root, sha) || head_commit(&root).as_deref() != Some(sha) {
        return Err("History moved; refresh and try again".into());
    }
    let parents = commit_parents(&root, sha)?;
    if parents.is_empty() { return Err("The first commit can't be undone".into()); }
    if parents.len() > 1 { return Err("Only a regular commit can be undone, not a merge".into()); }
    let pushed = run_git(&root, &["for-each-ref", "--contains", sha, "--count=1", "--format=%(refname)", "refs/remotes"], None)?;
    if !pushed.is_empty() { return Err("This commit is already pushed, so it can't be undone here".into()); }
    let message = run_git(&root, &["log", "-1", "--no-show-signature", "--format=%B", sha], None)?;
    let files = commit_files(&root, sha)?.files.into_iter().map(|file| file.path).collect();
    run_git(&root, &["reset", "--soft", &format!("{sha}^")], None)?;
    let (summary, description) = split_commit_message(&message);
    Ok(GitUndoResult { summary, description, files, changes: changes(path)? })
}

/// A new commit that reverses `sha`. A revert that conflicts is aborted, leaving the tree as
/// it was, and reported as `Conflict` rather than as an error.
pub fn revert_commit(path: &Path, sha: &str) -> Result<GitRevertResult, String> {
    let root = inspect_project(path).root.ok_or("No Git repository")?;
    refuse_mid_operation(&root)?;
    let parents = commit_parents(&root, sha)?;
    if run_git(&root, &["merge-base", "--is-ancestor", sha, "HEAD"], None).is_err() {
        return Err("That commit isn't on the current branch".into());
    }
    if parents.len() > 1 { return Err("Reverting merge commits isn't supported here".into()); }
    if !changes(&root)?.files.is_empty() { return Err("Commit or discard your changes before reverting".into()); }
    let outcome = match run_git(&root, &["revert", "--no-edit", sha], None) {
        Ok(_) => GitRevertOutcome::Reverted,
        Err(error) => {
            if operation_in_progress(&root).is_none() {
                return Err(format!("Could not revert: {}", error.lines().find(|line| !line.trim().is_empty()).unwrap_or(&error).trim()));
            }
            run_git(&root, &["revert", "--abort"], None)
                .map_err(|_| "The revert conflicted and could not be aborted. Run `git revert --abort` in a terminal".to_string())?;
            GitRevertOutcome::Conflict
        }
    };
    Ok(GitRevertResult { outcome, changes: changes(path)? })
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

/// The fetch remote's configured URL, as Git reports it (HTTPS, SSH or scp-like). None when
/// the checkout has no remote. The renderer turns it into the repository's web page.
pub fn remote_url(path: &Path) -> Result<Option<String>, String> {
    let root = inspect_project(path).root.ok_or("No Git repository")?;
    let publish = publish_info(&root)?;
    let Some(remote) = fetch_remote(&publish) else { return Ok(None) };
    Ok(Some(run_git(&root, &["remote", "get-url", &remote], None)?.trim().to_string()))
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
        assert!(result.files.iter().any(|file| file.path == "tracked file.txt" && file.sections.iter().any(|section| section.diff.contains("changed"))));
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
        assert!(result.files.iter().any(|file| file.path == "staged.txt" && file.staged && file.sections.iter().any(|section| section.layer == "staged")));
        assert!(result.files.iter().any(|file| file.path == "deleted.txt" && file.status == "deleted" && file.unstaged));
        assert!(result.files.iter().any(|file| file.path == "binary.dat" && file.binary && file.sections[0].diff.contains("4 bytes")));
        let staged_file = result.files.iter().find(|file| file.path == "staged.txt").unwrap();
        let staged_section = staged_file.sections.iter().find(|section| section.layer == "staged").unwrap();
        assert_eq!((staged_section.additions, staged_section.deletions), (1, 1));
        let large = result.files.iter().find(|file| file.path == "large.txt").unwrap();
        assert!(large.truncated);
        assert!(large.sections[0].diff.len() <= MAX_DIFF_BYTES);
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
        assert!(!link.sections.iter().any(|section| section.diff.contains("keep")));
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

    #[test]
    fn remote_url_reports_the_fetch_remote_as_configured() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        init_repo(root);
        commit_file(root, "file.txt", "one\n", "first");
        assert_eq!(remote_url(root).unwrap(), None);
        run(root, &["remote", "add", "origin", "git@github.com:owner/repo.git"]);
        assert_eq!(remote_url(root).unwrap().as_deref(), Some("git@github.com:owner/repo.git"));
    }

    fn run(path: &Path, args: &[&str]) -> String {
        let output = Command::new("git").arg("-C").arg(path).args(args).output().unwrap();
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_repo(path: &Path) {
        fs::create_dir_all(path).unwrap();
        run(path, &["init", "-b", "main"]);
        run(path, &["config", "user.email", "test@example.com"]);
        run(path, &["config", "user.name", "Test"]);
    }

    fn commit_file(path: &Path, file: &str, content: &str, message: &str) -> String {
        fs::write(path.join(file), content).unwrap();
        run(path, &["add", "--", file]);
        run(path, &["commit", "-m", message]);
        run(path, &["rev-parse", "HEAD"])
    }

    /// A checkout with `origin` (a bare repository) and a second clone of it that can push.
    fn with_remote(directory: &Path) -> (PathBuf, PathBuf) {
        let root = directory.join("checkout");
        let bare = directory.join("remote.git");
        let other = directory.join("other");
        init_repo(&root);
        fs::create_dir_all(&bare).unwrap();
        run(&bare, &["init", "--bare", "-b", "main"]);
        commit_file(&root, "file.txt", "one\n", "first");
        run(&root, &["remote", "add", "origin", bare.to_str().unwrap()]);
        run(&root, &["push", "-u", "origin", "main"]);
        run(directory, &["clone", bare.to_str().unwrap(), other.to_str().unwrap()]);
        run(&other, &["config", "user.email", "other@example.com"]);
        run(&other, &["config", "user.name", "Other"]);
        (root, other)
    }

    fn fetch(root: &Path) {
        let args = fetch_args("origin");
        run_git(root, &args.iter().map(String::as_str).collect::<Vec<_>>(), None).unwrap();
    }

    #[test]
    fn commit_includes_the_old_path_of_a_staged_rename() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        init_repo(root);
        commit_file(root, "before.txt", "content\n", "initial");
        run(root, &["mv", "before.txt", "after.txt"]);
        let snapshot = changes(root).unwrap();
        let renamed = snapshot.files.iter().find(|file| file.path == "after.txt").unwrap();
        assert_eq!(renamed.old_path.as_deref(), Some("before.txt"));
        let after = commit(root, "rename", &["after.txt".to_string()], &snapshot.changes_revision).unwrap();
        assert!(after.files.is_empty(), "the deletion must be committed too: {:?}", after.files.iter().map(|file| &file.path).collect::<Vec<_>>());
        assert_eq!(run(root, &["ls-tree", "--name-only", "HEAD"]), "after.txt");
    }

    #[test]
    fn scoped_commits_keep_other_staged_work_out_and_allow_case_only_renames() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        init_repo(root);
        commit_file(root, "notes.md", "notes\n", "initial");
        commit_file(root, "other.txt", "other\n", "second");
        run(root, &["mv", "notes.md", "NOTES.md"]);
        fs::write(root.join("loose.txt"), "untracked\n").unwrap();
        let snapshot = changes(root).unwrap();
        let renamed = snapshot.files.iter().find(|file| file.status == "renamed").unwrap().path.clone();
        let after = commit(root, "rename notes", &[renamed], &snapshot.changes_revision).unwrap();
        assert_eq!(after.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(), vec!["loose.txt"]);

        // Staged work outside the scope stays staged and out of the commit.
        fs::write(root.join("other.txt"), "staged elsewhere\n").unwrap();
        run(root, &["add", "other.txt"]);
        fs::write(root.join("loose.txt"), "commit me\n").unwrap();
        let snapshot = changes(root).unwrap();
        commit(root, "just loose", &["loose.txt".to_string()], &snapshot.changes_revision).unwrap();
        assert_eq!(run(root, &["show", "--format=", "--name-only", "HEAD"]), "loose.txt");
        assert!(changes(root).unwrap().files.iter().any(|file| file.path == "other.txt" && file.staged));
    }

    #[test]
    fn code_that_mentions_binary_markers_is_still_text() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        init_repo(root);
        commit_file(root, "detect.rs", "fn main() {}\n", "initial");
        fs::write(root.join("detect.rs"), "fn main() {}\nlet a = \"Binary files \";\nlet b = \"GIT binary patch\";\n").unwrap();
        let result = changes(root).unwrap();
        assert!(!result.files[0].binary);
        assert_eq!(result.files[0].sections[0].additions, 2);
    }

    #[test]
    fn commit_accepts_staged_deletions_scoped_or_whole() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        init_repo(root);
        commit_file(root, "gone.txt", "bye\n", "initial");
        commit_file(root, "also.txt", "bye too\n", "second");
        commit_file(root, "keep.txt", "keep\n", "third");
        run(root, &["rm", "-q", "gone.txt"]);
        fs::write(root.join("keep.txt"), "changed\n").unwrap();
        let snapshot = changes(root).unwrap();
        let scoped = commit(root, "remove gone", &["gone.txt".to_string()], &snapshot.changes_revision).unwrap();
        assert!(!run(root, &["ls-tree", "--name-only", "HEAD"]).contains("gone.txt"));
        assert!(scoped.files.iter().any(|file| file.path == "keep.txt"));
        run(root, &["rm", "-q", "also.txt"]);
        let snapshot = changes(root).unwrap();
        let everything = commit(root, "the rest", &[], &snapshot.changes_revision).unwrap();
        assert!(everything.files.is_empty());
    }

    #[test]
    fn sync_status_fetch_and_fast_forward_follow_the_remote() {
        let directory = tempfile::tempdir().unwrap();
        let (root, other) = with_remote(directory.path());
        let start = sync_status(&root).unwrap();
        assert_eq!(start.upstream.as_deref(), Some("origin/main"));
        assert_eq!(start.fetch_remote.as_deref(), Some("origin"));
        assert_eq!((start.ahead, start.behind), (0, 0));
        assert!(start.fetched_at.is_none());
        assert_eq!(fast_forward(&root).unwrap(), (GitPullOutcome::UpToDate, 0));

        commit_file(&other, "file.txt", "two\n", "second");
        run(&other, &["push"]);
        assert_eq!(sync_status(&root).unwrap().behind, 0, "nothing is known before a fetch");
        fetch(&root);
        let fetched = sync_status(&root).unwrap();
        assert_eq!((fetched.ahead, fetched.behind), (0, 1));
        assert!(fetched.fetched_at.is_some());
        assert_eq!(fast_forward(&root).unwrap(), (GitPullOutcome::Updated, 1));
        assert_eq!(fs::read_to_string(root.join("file.txt")).unwrap(), "two\n");

        // Local edits a fast-forward would overwrite are named, and nothing moves.
        commit_file(&other, "file.txt", "three\n", "third");
        run(&other, &["push"]);
        fetch(&root);
        fs::write(root.join("file.txt"), "mine\n").unwrap();
        let refused = fast_forward(&root).unwrap_err();
        assert!(refused.contains("overwrite your changes to file.txt"), "{refused}");
        assert_eq!(fs::read_to_string(root.join("file.txt")).unwrap(), "mine\n");

        // Commits on both sides: reported as diverged, HEAD untouched.
        run(&root, &["checkout", "--", "file.txt"]);
        let local = commit_file(&root, "local.txt", "local\n", "local work");
        let diverged = sync_status(&root).unwrap();
        assert_eq!((diverged.ahead, diverged.behind), (1, 1));
        assert_eq!(fast_forward(&root).unwrap(), (GitPullOutcome::Diverged, 0));
        assert_eq!(head_commit(&root).as_deref(), Some(local.as_str()));

        // Without an upstream, `ahead` counts commits no remote has.
        run(&root, &["switch", "-c", "topic"]);
        commit_file(&root, "topic.txt", "topic\n", "topic work");
        let topic = sync_status(&root).unwrap();
        assert!(topic.upstream.is_none());
        assert_eq!(topic.fetch_remote.as_deref(), Some("origin"));
        assert_eq!((topic.ahead, topic.behind), (2, 0));
        assert!(fast_forward(&root).unwrap_err().contains("no upstream"));
    }

    #[test]
    fn fetch_remote_prefers_the_upstream_then_origin() {
        let info = |upstream: Option<&str>, remotes: &[&str]| GitPublishInfo {
            branch: Some("main".into()),
            upstream: upstream.map(str::to_owned),
            remotes: remotes.iter().map(|name| name.to_string()).collect(),
        };
        assert_eq!(fetch_remote(&info(Some("team/sub/main"), &["origin", "team", "team/sub"])).as_deref(), Some("team/sub"));
        assert_eq!(fetch_remote(&info(None, &["fork", "origin"])).as_deref(), Some("origin"));
        assert_eq!(fetch_remote(&info(None, &["fork"])).as_deref(), Some("fork"));
        assert_eq!(fetch_remote(&info(None, &[])), None);
    }

    #[test]
    fn log_pages_carry_bodies_parents_and_unpushed_flags() {
        let directory = tempfile::tempdir().unwrap();
        let empty = directory.path().join("empty");
        init_repo(&empty);
        let page = log(&empty, 0, 50).unwrap();
        assert!(page.commits.is_empty() && page.head.is_none());

        let (root, _) = with_remote(directory.path());
        fs::write(root.join("notes.txt"), "a\n").unwrap();
        run(&root, &["add", "notes.txt"]);
        run(&root, &["commit", "-m", "Tricky body", "-m", "Line with \u{1f} separator\nand more"]);
        let first = log(&root, 0, 1).unwrap();
        assert!(first.has_more);
        assert_eq!(first.commits.len(), 1);
        let latest = &first.commits[0];
        assert_eq!(latest.subject, "Tricky body");
        assert_eq!(latest.body, "Line with \u{1f} separator\nand more");
        assert!(latest.unpushed);
        assert_eq!(latest.parents.len(), 1);
        assert_eq!(first.head.as_deref(), Some(latest.sha.as_str()));
        let second = log(&root, 1, 50).unwrap();
        assert!(!second.has_more);
        assert_eq!(second.commits.len(), 1);
        assert!(!second.commits[0].unpushed, "the pushed commit has no marker");
        assert_eq!(second.commits[0].author_name, "Test");
    }

    #[test]
    fn commit_files_and_diffs_cover_renames_binaries_roots_and_merges() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        init_repo(root);
        let first = commit_file(root, "old name.txt", "same\ncontent\nhere\n", "first");
        let root_files = commit_files(root, &first).unwrap();
        assert_eq!(root_files.files.len(), 1);
        assert_eq!(root_files.files[0].status, "added");
        assert_eq!(root_files.files[0].additions, Some(3));

        run(root, &["mv", "old name.txt", "new name.txt"]);
        fs::write(root.join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
        run(root, &["add", "-A"]);
        run(root, &["commit", "-m", "rename and binary"]);
        let second = run(root, &["rev-parse", "HEAD"]);
        let listed = commit_files(root, &second).unwrap();
        let renamed = listed.files.iter().find(|file| file.path == "new name.txt").unwrap();
        assert_eq!((renamed.status.as_str(), renamed.old_path.as_deref()), ("renamed", Some("old name.txt")));
        let binary = listed.files.iter().find(|file| file.path == "blob.bin").unwrap();
        assert!(binary.binary && binary.additions.is_none());

        let diff = commit_diff(root, &second, "new name.txt", Some("old name.txt")).unwrap();
        assert_eq!(diff.status, "renamed");
        assert_eq!(diff.sections[0].layer, "commit");
        assert!(!diff.hunkable);
        let binary_diff = commit_diff(root, &second, "blob.bin", None).unwrap();
        assert!(binary_diff.binary);

        let first_diff = commit_diff(root, &first, "old name.txt", None).unwrap();
        assert_eq!(first_diff.sections[0].additions, 3);

        // A merge compares with its first parent.
        run(root, &["switch", "-c", "side"]);
        commit_file(root, "side.txt", "side\n", "side work");
        run(root, &["switch", "main"]);
        commit_file(root, "main.txt", "main\n", "main work");
        run(root, &["merge", "--no-ff", "--no-edit", "side"]);
        let merge = run(root, &["rev-parse", "HEAD"]);
        let merged = commit_files(root, &merge).unwrap();
        assert_eq!(merged.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(), vec!["side.txt"]);
        assert!(commit_files(root, "not-a-sha").is_err());
    }

    #[test]
    fn undo_moves_back_only_an_unpushed_regular_tip() {
        let directory = tempfile::tempdir().unwrap();
        let (root, _) = with_remote(directory.path());
        let pushed = commit_file(&root, "file.txt", "two\n", "second");
        run(&root, &["push"]);
        assert!(undo_commit(&root, &pushed).unwrap_err().contains("already pushed"));

        fs::write(root.join("notes.txt"), "a\n").unwrap();
        run(&root, &["add", "notes.txt"]);
        run(&root, &["commit", "-m", "Add notes", "-m", "Because notes help."]);
        let tip = head_commit(&root).unwrap();
        assert!(undo_commit(&root, &pushed).unwrap_err().contains("History moved"));
        let undone = undo_commit(&root, &tip).unwrap();
        assert_eq!(undone.summary, "Add notes");
        assert_eq!(undone.description, "Because notes help.");
        assert_eq!(undone.files, vec!["notes.txt".to_string()]);
        assert_eq!(head_commit(&root).as_deref(), Some(pushed.as_str()));
        assert!(undone.changes.files.iter().any(|file| file.path == "notes.txt" && file.staged));

        let lonely = directory.path().join("lonely");
        init_repo(&lonely);
        let only = commit_file(&lonely, "a.txt", "a\n", "only");
        assert!(undo_commit(&lonely, &only).unwrap_err().contains("first commit"));
        run(&lonely, &["switch", "-c", "side"]);
        commit_file(&lonely, "b.txt", "b\n", "side");
        run(&lonely, &["switch", "main"]);
        commit_file(&lonely, "c.txt", "c\n", "main");
        run(&lonely, &["merge", "--no-ff", "--no-edit", "side"]);
        let merge = head_commit(&lonely).unwrap();
        assert!(undo_commit(&lonely, &merge).unwrap_err().contains("merge"));
    }

    #[test]
    fn revert_refuses_dirty_trees_and_aborts_conflicts() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        init_repo(root);
        commit_file(root, "file.txt", "one\n", "one");
        let two = commit_file(root, "file.txt", "two\n", "two");
        commit_file(root, "file.txt", "three\n", "three");

        fs::write(root.join("scratch.txt"), "dirty\n").unwrap();
        assert!(revert_commit(root, &two).unwrap_err().contains("Commit or discard"));
        fs::remove_file(root.join("scratch.txt")).unwrap();

        // "three" rewrote the line "two" introduced, so reverting "two" conflicts.
        let conflict = revert_commit(root, &two).unwrap();
        assert_eq!(conflict.outcome, GitRevertOutcome::Conflict);
        assert!(conflict.changes.files.is_empty());
        assert!(operation_in_progress(root).is_none());
        assert_eq!(fs::read_to_string(root.join("file.txt")).unwrap(), "three\n");

        let add = commit_file(root, "extra.txt", "extra\n", "add extra");
        let reverted = revert_commit(root, &add).unwrap();
        assert_eq!(reverted.outcome, GitRevertOutcome::Reverted);
        assert!(!root.join("extra.txt").exists());
        assert!(run(root, &["log", "-1", "--format=%s"]).starts_with("Revert"));
    }

    #[test]
    fn commit_messages_split_into_summary_and_description() {
        assert_eq!(split_commit_message("Fix it"), ("Fix it".into(), String::new()));
        assert_eq!(split_commit_message("\r\n Fix it \r\n\r\n- one\r\n- two\r\n"), ("Fix it".into(), "- one\n- two".into()));
        assert_eq!(split_commit_message("```\nAdd thing\n\nWhy it matters.\n```"), ("Add thing".into(), "Why it matters.".into()));
        assert_eq!(split_commit_message("").0, "");
    }

    #[test]
    fn pull_errors_name_the_files() {
        let tracked = "error: Your local changes to the following files would be overwritten by merge:\n\ta.ts\n\tb.ts\nPlease commit your changes or stash them before you merge.\nAborting";
        assert_eq!(pull_error(tracked), "Pulling would overwrite your changes to a.ts, b.ts. Commit or discard them, then pull again.");
        let untracked = "error: The following untracked working tree files would be overwritten by merge:\n\tnew.txt\nPlease move or remove them before you merge.";
        assert_eq!(pull_error(untracked), "Pulling would overwrite the untracked file new.txt. Move or delete it, then pull again.");
        assert_eq!(pull_error("fatal: something else\nmore"), "Could not pull: fatal: something else");
    }

    #[test]
    fn lists_and_switches_branches() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("checkout");
        let bare = directory.path().join("remote.git");
        let other = directory.path().join("other");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&bare).unwrap();
        git(&root, &[OsStr::new("init"), OsStr::new("-b"), OsStr::new("main")]);
        git(&bare, &[OsStr::new("init"), OsStr::new("--bare")]);
        git(&root, &[OsStr::new("config"), OsStr::new("user.email"), OsStr::new("test@example.com")]);
        git(&root, &[OsStr::new("config"), OsStr::new("user.name"), OsStr::new("Test")]);
        fs::write(root.join("file.txt"), "main\n").unwrap();
        git(&root, &[OsStr::new("add"), OsStr::new("file.txt")]);
        git(&root, &[OsStr::new("commit"), OsStr::new("-m"), OsStr::new("first")]);
        git(&root, &[OsStr::new("remote"), OsStr::new("add"), OsStr::new("origin"), bare.as_os_str()]);
        git(&root, &[OsStr::new("switch"), OsStr::new("-c"), OsStr::new("feature")]);
        fs::write(root.join("file.txt"), "feature\n").unwrap();
        git(&root, &[OsStr::new("commit"), OsStr::new("-am"), OsStr::new("feature")]);
        git(&root, &[OsStr::new("push"), OsStr::new("origin"), OsStr::new("main"), OsStr::new("feature")]);
        git(&root, &[OsStr::new("switch"), OsStr::new("main")]);
        git(&root, &[OsStr::new("branch"), OsStr::new("-D"), OsStr::new("feature")]);
        git(&root, &[OsStr::new("worktree"), OsStr::new("add"), OsStr::new("-b"), OsStr::new("elsewhere"), other.as_os_str()]);

        let listed = branches(&root).unwrap();
        assert_eq!(listed.current.as_deref(), Some("main"));
        let find = |name: &str| listed.branches.iter().find(|branch| branch.name == name);
        assert!(find("main").is_some_and(|branch| !branch.remote && branch.worktree.is_none()));
        assert!(find("elsewhere").is_some_and(|branch| branch.worktree.is_some()));
        assert!(find("origin/feature").is_some_and(|branch| branch.remote));
        assert!(find("origin/main").is_none(), "a local branch shadows its remote");

        assert!(checkout(&root, "elsewhere", "local").unwrap_err().contains("another worktree"));
        assert!(checkout(&root, "--force", "local").is_err());
        assert!(checkout(&root, "missing", "local").is_err());
        assert_eq!(checkout(&root, "origin/feature", "remote").unwrap().as_deref(), Some("feature"));
        assert_eq!(fs::read_to_string(root.join("file.txt")).unwrap(), "feature\n");
        assert!(checkout(&root, "feature", "create").unwrap_err().contains("already exists"));

        fs::write(root.join("file.txt"), "edited\n").unwrap();
        assert!(checkout(&root, "main", "local").unwrap_err().contains("Commit or discard"));
        assert_eq!(current_branch(&root).as_deref(), Some("feature"));
        assert_eq!(checkout(&root, "topic/new", "create").unwrap().as_deref(), Some("topic/new"));
        assert_eq!(fs::read_to_string(root.join("file.txt")).unwrap(), "edited\n");
    }
}
