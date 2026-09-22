//! Workspace checkpoints: snapshots of a chat's files in a private "shadow" Git repository under
//! WackCode's application data, so rewind, retry, edit and fork can also put files back.
//!
//! Invariants:
//! - The shadow never creates, changes or deletes objects or refs in the project's own
//!   repository. It borrows the project's objects through `objects/info/alternates`, so unchanged
//!   files cost nothing; Git may still refresh the mtime of an object it re-finds there.
//! - A snapshot is a tree object with a ref pointing straight at it: no commits, so no author
//!   identity, hooks or signing are involved.
//! - A restore only ever touches paths whose content differs between the current snapshot and
//!   the checkpoint, and never writes over a file the current snapshot does not hold (ignored,
//!   oversized, or otherwise untracked files).
use crate::models::{CheckpointChange, CheckpointRef, RestoreResult};
use std::{
    collections::HashSet,
    ffi::OsStr,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

/// A file above this size is left out of a snapshot, and so never restored.
const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
/// More new files than this in one snapshot means the root is not a project (e.g. a home folder).
const MAX_NEW_FILES: usize = 20_000;
const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const NO_COMMIT_MARKER: &str = "does not have a commit checked out";

/// Where one chat's snapshots live.
pub fn shadow_dir(app_data: &Path, task_id: &str) -> PathBuf {
    app_data.join("checkpoints").join(task_id)
}

/// A shadow repository bound to a work tree and an index.
struct Shadow {
    git_dir: PathBuf,
    work_tree: PathBuf,
    index: PathBuf,
}

impl Shadow {
    fn new(git_dir: &Path, work_tree: &Path) -> Self {
        Self { git_dir: git_dir.to_path_buf(), work_tree: work_tree.to_path_buf(), index: git_dir.join("index") }
    }

    fn with_index(&self, index: PathBuf) -> Self {
        Self { git_dir: self.git_dir.clone(), work_tree: self.work_tree.clone(), index }
    }

    fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = clean_git();
        command
            .env("GIT_DIR", &self.git_dir)
            .env("GIT_WORK_TREE", &self.work_tree)
            .env("GIT_INDEX_FILE", &self.index)
            // The user's system and global config may define hooks, filters, fsmonitor or
            // signing; none of it may run against the shadow.
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .current_dir(&self.work_tree);
        if let Some(excludes) = user_excludes_file() {
            command.arg("-c").arg(format!("core.excludesFile={}", excludes.display()));
        }
        command.args(args);
        command
    }

    fn git<I, S>(&self, args: I, input: Option<Vec<u8>>) -> Result<Vec<u8>, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = run(self.command(args), input, GIT_TIMEOUT)?;
        if output.status.success() { Ok(output.stdout) } else { Err(stderr_of(&output)) }
    }

    fn text<I, S>(&self, args: I) -> Result<String, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Ok(String::from_utf8_lossy(&self.git(args, None)?).trim().to_string())
    }
}

/// `git` with nothing inherited from the environment that could redirect it to another repo.
fn clean_git() -> Command {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") { command.env_remove(&key); }
    }
    command.env("GIT_TERMINAL_PROMPT", "0");
    command
}

/// Read-only questions about the project's own repository, with its normal configuration.
fn project_git(root: &Path, args: &[&str]) -> Option<String> {
    let mut command = clean_git();
    command.env("GIT_OPTIONAL_LOCKS", "0").arg("-C").arg(root).args(args);
    let output = run(command, None, GIT_TIMEOUT).ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The user's own `core.excludesFile`, which ignoring the global config would otherwise drop.
fn user_excludes_file() -> Option<PathBuf> {
    static EXCLUDES: OnceLock<Option<PathBuf>> = OnceLock::new();
    EXCLUDES.get_or_init(|| {
        let mut command = clean_git();
        command.args(["config", "--global", "--path", "--get", "core.excludesFile"]);
        let output = run(command, None, GIT_TIMEOUT).ok()?;
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (output.status.success() && !value.is_empty()).then(|| PathBuf::from(value)).filter(|path| path.is_file())
    }).clone()
}

/// Without the Command Line Tools, `/usr/bin/git` opens an installer dialog instead of running.
fn git_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        if cfg!(target_os = "macos") {
            let probe = Command::new("xcode-select").arg("-p").stdout(Stdio::null()).stderr(Stdio::null()).status();
            if !probe.is_ok_and(|status| status.success()) { return false; }
        }
        Command::new("git").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status()
            .is_ok_and(|status| status.success())
    })
}

/// Run a command with a deadline, draining its output on threads so large output never stalls it.
fn run(mut command: Command, input: Option<Vec<u8>>, timeout: Duration) -> Result<Output, String> {
    command
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| format!("Could not start git: {error}"))?;
    let writer = input.map(|bytes| {
        let mut stdin = child.stdin.take();
        thread::spawn(move || { if let Some(stdin) = stdin.as_mut() { let _ = stdin.write_all(&bytes); } })
    });
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out = thread::spawn(move || { let mut buffer = Vec::new(); if let Some(pipe) = stdout.as_mut() { let _ = pipe.read_to_end(&mut buffer); } buffer });
    let err = thread::spawn(move || { let mut buffer = Vec::new(); if let Some(pipe) = stderr.as_mut() { let _ = pipe.read_to_end(&mut buffer); } buffer });
    let deadline = Instant::now() + timeout;
    let mut pause = Duration::from_millis(1);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? { break status; }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Git took too long to answer.".into());
        }
        thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_millis(20));
    };
    if let Some(writer) = writer { let _ = writer.join(); }
    Ok(Output { status, stdout: out.join().unwrap_or_default(), stderr: err.join().unwrap_or_default() })
}

fn stderr_of(output: &Output) -> String {
    let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if message.is_empty() { "Git failed without saying why.".into() } else { message }
}

fn nul_list<'a>(items: impl IntoIterator<Item = &'a str>) -> Vec<u8> {
    let mut bytes = Vec::new();
    for item in items {
        bytes.extend_from_slice(item.as_bytes());
        bytes.push(0);
    }
    bytes
}

pub fn valid_checkpoint_id(id: &str) -> bool {
    (id.len() == 40 || id.len() == 64) && id.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// The project repository the root is the top of, if it is one. Never walks up: a scratch
/// folder inside a home directory that happens to be a repository is not that repository.
fn project_repo(root: &Path) -> Option<PathBuf> {
    let top = project_git(root, &["rev-parse", "--show-toplevel"])?;
    same_path(Path::new(&top), root).then(|| PathBuf::from(top))
}

fn refuse_unsafe_root(root: &Path, app_data: &Path) -> Result<(), String> {
    if !root.is_dir() { return Err("The chat's folder no longer exists.".into()); }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if same_path(root, &home) {
            return Err("Checkpoints are off for a home folder. Open a project folder instead.".into());
        }
    }
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let app_data = app_data.canonicalize().unwrap_or_else(|_| app_data.to_path_buf());
    if app_data.starts_with(&root) {
        return Err("Checkpoints are off for a folder that contains WackCode's own data.".into());
    }
    Ok(())
}

/// Create the shadow repository on first use.
fn ensure_initialized(shadow: &Shadow) -> Result<(), String> {
    if shadow.git_dir.join("HEAD").exists() { return Ok(()); }
    fs::create_dir_all(&shadow.git_dir).map_err(|error| format!("Could not create the checkpoint store: {error}"))?;
    let project = project_repo(&shadow.work_tree);
    let format = project.as_deref()
        .and_then(|root| project_git(root, &["rev-parse", "--show-object-format"]))
        .filter(|format| format == "sha1" || format == "sha256")
        .unwrap_or_else(|| "sha1".into());
    let mut init = clean_git();
    init.env("GIT_DIR", &shadow.git_dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(["init", "--quiet", "--template="])
        .arg(format!("--object-format={format}"));
    let output = run(init, None, GIT_TIMEOUT)?;
    if !output.status.success() {
        let _ = fs::remove_dir_all(&shadow.git_dir);
        return Err(format!("Could not create the checkpoint store: {}", stderr_of(&output)));
    }
    let mut settings = vec![
        ("core.bare", "false".to_string()),
        ("gc.auto", "0".to_string()),
        ("core.autocrlf", "false".to_string()),
        ("core.fsmonitor", "false".to_string()),
        ("core.untrackedCache", "false".to_string()),
    ];
    if let Some(root) = project.as_deref() {
        // `git init` probed the app-data volume; the work tree's own repository knows its volume.
        for key in ["core.ignorecase", "core.precomposeunicode", "core.symlinks", "core.filemode"] {
            if let Some(value) = project_git(root, &["config", "--get", key]) { settings.push((key, value)); }
        }
    }
    for (key, value) in settings {
        shadow.git(["config", key, value.as_str()], None)?;
    }
    let info = shadow.git_dir.join("info");
    fs::create_dir_all(&info).map_err(|error| error.to_string())?;
    // Highest-precedence attributes: no line-ending conversion, filters (LFS) or keyword
    // expansion, so every snapshot and restore is byte-exact.
    fs::write(info.join("attributes"), "* -text -filter -ident !eol !working-tree-encoding\n").map_err(|error| error.to_string())?;
    let mut exclude = String::from(".DS_Store\n");
    if let Some(root) = project.as_deref() {
        if let Some(common) = project_git(root, &["rev-parse", "--path-format=absolute", "--git-common-dir"]) {
            let common = PathBuf::from(common);
            let objects = common.join("objects");
            if objects.is_dir() {
                let alternates = shadow.git_dir.join("objects").join("info");
                fs::create_dir_all(&alternates).map_err(|error| error.to_string())?;
                fs::write(alternates.join("alternates"), format!("{}\n", objects.display())).map_err(|error| error.to_string())?;
            }
            if let Ok(project_exclude) = fs::read_to_string(common.join("info").join("exclude")) {
                exclude.push_str(&project_exclude);
                exclude.push('\n');
            }
        }
        // Start from the project's HEAD tree rather than a copy of its index, whose flags
        // (assume-unchanged, skip-worktree, split index) would leak into snapshots.
        if let Some(tree) = project_git(root, &["rev-parse", "--verify", "--quiet", "HEAD^{tree}"]) {
            let _ = shadow.git(["read-tree", tree.as_str()], None);
        }
    }
    fs::write(info.join("exclude"), exclude).map_err(|error| error.to_string())?;
    Ok(())
}

/// Write the work tree's current state into the shadow's index and return its tree id.
fn write_current_tree(shadow: &Shadow) -> Result<String, String> {
    let _ = fs::remove_file(PathBuf::from(format!("{}.lock", shadow.index.display())));
    let new_files = shadow.git(["ls-files", "-z", "--others", "--exclude-standard"], None)?;
    let new_files: Vec<String> = new_files.split(|byte| *byte == 0).filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned()).collect();
    if new_files.len() > MAX_NEW_FILES {
        return Err(format!("Checkpoints are off here: {} new files would be copied. Add a .gitignore for generated folders.", new_files.len()));
    }
    let modified = shadow.git(["ls-files", "-z", "--modified"], None)?;
    let mut oversized = Vec::new();
    for path in new_files.iter().map(String::as_str).chain(
        modified.split(|byte| *byte == 0).filter(|path| !path.is_empty()).map(|path| std::str::from_utf8(path).unwrap_or(""))
    ) {
        if path.is_empty() { continue; }
        if fs::symlink_metadata(shadow.work_tree.join(path)).is_ok_and(|metadata| metadata.is_file() && metadata.len() > MAX_FILE_BYTES) {
            oversized.push(path.to_string());
        }
    }
    if !oversized.is_empty() {
        // Out of the index entirely, so the snapshot never holds a stale copy of a big file.
        shadow.git(["update-index", "-z", "--force-remove", "--stdin"], Some(nul_list(oversized.iter().map(String::as_str))))?;
    }
    let mut excluded: Vec<String> = oversized.iter().map(|path| format!(":(exclude,literal){path}")).collect();
    for _ in 0..16 {
        let pathspecs = std::iter::once(".".to_string()).chain(excluded.iter().cloned()).collect::<Vec<_>>();
        match shadow.git(["add", "-A", "--pathspec-from-file=-", "--pathspec-file-nul"], Some(nul_list(pathspecs.iter().map(String::as_str)))) {
            Ok(_) => return shadow.text(["write-tree"]),
            // A nested repository with no commit (e.g. the agent ran `git init` in a subfolder)
            // makes `add` fail outright; leave that folder out and try again.
            Err(message) if message.contains(NO_COMMIT_MARKER) => {
                let Some(path) = message.split('\'').nth(1).map(|path| path.trim_end_matches('/').to_string()) else { return Err(message) };
                let spec = format!(":(exclude,literal){path}");
                if excluded.contains(&spec) { return Err(message); }
                excluded.push(spec);
            }
            Err(message) => return Err(message),
        }
    }
    Err("Too many nested repositories without commits to take a checkpoint.".into())
}

fn keep(shadow: &Shadow, tree: &str) -> Result<(), String> {
    shadow.git(["update-ref", &format!("refs/wackcode/{tree}"), tree], None).map(|_| ())
}

/// Snapshot `root` into the chat's shadow repository.
pub fn snapshot(shadow_dir: &Path, root: &Path, app_data: &Path) -> Result<CheckpointRef, String> {
    if !git_available() { return Err("Checkpoints need Git from the Xcode Command Line Tools.".into()); }
    refuse_unsafe_root(root, app_data)?;
    let shadow = Shadow::new(shadow_dir, root);
    ensure_initialized(&shadow)?;
    let tree = write_current_tree(&shadow)?;
    keep(&shadow, &tree)?;
    let head = project_repo(root).and_then(|repo| project_git(&repo, &["rev-parse", "--verify", "--quiet", "HEAD"]))
        .filter(|head| valid_checkpoint_id(head));
    Ok(CheckpointRef { id: tree, head })
}

fn require_tree(shadow: &Shadow, id: &str) -> Result<(), String> {
    if !valid_checkpoint_id(id) { return Err("That checkpoint id is not valid.".into()); }
    shadow.git(["cat-file", "-e", &format!("{id}^{{tree}}")], None)
        .map(|_| ()).map_err(|_| "This checkpoint is no longer available.".to_string())
}

#[derive(Debug, Clone, PartialEq)]
struct Difference {
    path: String,
    /// "revert", "delete" or "recreate": what restoring does to the file.
    action: &'static str,
    /// The checkpoint's object, for writes.
    object: Option<String>,
}

/// What restoring `to` would change, starting from the snapshot `from`.
fn differences(shadow: &Shadow, from: &str, to: &str) -> Result<Vec<Difference>, String> {
    let raw = shadow.git(["diff-tree", "-r", "-z", "--no-renames", "--raw", from, to], None)?;
    let mut fields = raw.split(|byte| *byte == 0).filter(|field| !field.is_empty());
    let mut result = Vec::new();
    while let (Some(meta), Some(path)) = (fields.next(), fields.next()) {
        let meta = String::from_utf8_lossy(meta);
        let parts: Vec<&str> = meta.trim_start_matches(':').split_whitespace().collect();
        let [old_mode, new_mode, _old, new, status] = parts[..] else { continue };
        // Submodules and nested repositories are recorded as links only; never touch them.
        if old_mode == "160000" || new_mode == "160000" { continue; }
        let action = match status.chars().next() {
            Some('A') => "recreate",
            Some('D') => "delete",
            _ => "revert",
        };
        result.push(Difference {
            path: String::from_utf8_lossy(path).into_owned(),
            action,
            object: (action != "delete").then(|| new.to_string()),
        });
    }
    Ok(result)
}

/// The files restoring checkpoint `id` would change.
pub fn changes(shadow_dir: &Path, root: &Path, app_data: &Path, id: &str) -> Result<Vec<CheckpointChange>, String> {
    if !git_available() { return Err("Checkpoints need Git from the Xcode Command Line Tools.".into()); }
    refuse_unsafe_root(root, app_data)?;
    let shadow = Shadow::new(shadow_dir, root);
    require_tree(&shadow, id)?;
    let current = write_current_tree(&shadow)?;
    Ok(differences(&shadow, &current, id)?.into_iter()
        .map(|difference| CheckpointChange { path: difference.path, status: difference.action.to_string() })
        .collect())
}

/// Put the files of checkpoint `id` back, optionally only `paths`. The state just before is
/// kept as a checkpoint too and returned as `undo`.
pub fn restore(shadow_dir: &Path, root: &Path, app_data: &Path, id: &str, paths: Option<&[String]>) -> Result<RestoreResult, String> {
    if !git_available() { return Err("Checkpoints need Git from the Xcode Command Line Tools.".into()); }
    refuse_unsafe_root(root, app_data)?;
    let shadow = Shadow::new(shadow_dir, root);
    require_tree(&shadow, id)?;
    let current = write_current_tree(&shadow)?;
    keep(&shadow, &current)?;
    let (restored, skipped) = apply(&shadow, &current, id, paths)?;
    Ok(RestoreResult { restored, skipped, undo: CheckpointRef { id: current, head: None } })
}

/// Write checkpoint `id` from one chat's shadow into another folder (a fork's new workspace).
pub fn materialize(shadow_dir: &Path, id: &str, target: &Path) -> Result<Vec<String>, String> {
    if !git_available() { return Err("Checkpoints need Git from the Xcode Command Line Tools.".into()); }
    let base = Shadow::new(shadow_dir, target);
    require_tree(&base, id)?;
    let index = shadow_dir.join(format!("index-materialize-{}", uuid::Uuid::new_v4()));
    let shadow = base.with_index(index.clone());
    // A fresh worktree already holds its HEAD: start from that tree so only real differences
    // count as new files.
    if let Some(tree) = project_repo(target).and_then(|repo| project_git(&repo, &["rev-parse", "--verify", "--quiet", "HEAD^{tree}"])) {
        let _ = shadow.git(["read-tree", tree.as_str()], None);
    }
    let result = write_current_tree(&shadow).and_then(|current| apply(&shadow, &current, id, None)).map(|(_, skipped)| skipped);
    let _ = fs::remove_file(&index);
    result
}

/// Apply the difference between `current` (what is on disk) and `target`. Deletes run first so a
/// case-only rename or a file replacing a folder frees its path before the write.
fn apply(shadow: &Shadow, current: &str, target: &str, paths: Option<&[String]>) -> Result<(Vec<String>, Vec<String>), String> {
    let selected: Option<HashSet<&str>> = paths.map(|paths| paths.iter().map(String::as_str).collect());
    let differences: Vec<Difference> = differences(shadow, current, target)?.into_iter()
        .filter(|difference| selected.as_ref().map_or(true, |selected| selected.contains(difference.path.as_str())))
        .collect();
    // Every object must be readable before anything on disk changes: a borrowed object can be
    // pruned from the project's repository.
    let objects: Vec<&str> = differences.iter().filter_map(|difference| difference.object.as_deref()).collect();
    if !objects.is_empty() {
        let mut input = objects.join("\n");
        input.push('\n');
        let report = String::from_utf8_lossy(&shadow.git(["cat-file", "--batch-check"], Some(input.into_bytes()))?).into_owned();
        if report.lines().any(|line| line.ends_with(" missing")) {
            return Err("Some files in this checkpoint are no longer available, so nothing was restored.".into());
        }
    }
    let root = &shadow.work_tree;
    let mut restored = Vec::new();
    let mut skipped = Vec::new();
    let mut emptied = Vec::new();
    for difference in differences.iter().filter(|difference| difference.action == "delete") {
        let path = root.join(&difference.path);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => skipped.push(difference.path.clone()),
            Ok(_) => match fs::remove_file(&path) {
                Ok(()) => {
                    restored.push(difference.path.clone());
                    if let Some(parent) = path.parent() { emptied.push(parent.to_path_buf()); }
                }
                Err(_) => skipped.push(difference.path.clone()),
            },
            Err(_) => restored.push(difference.path.clone()),
        }
    }
    for directory in emptied { prune_empty(&directory, root); }

    let mut writes: Vec<String> = Vec::new();
    for difference in differences.iter().filter(|difference| difference.action != "delete") {
        if write_blocked(root, &difference.path, difference.action == "recreate") {
            skipped.push(difference.path.clone());
        } else {
            writes.push(difference.path.clone());
        }
    }
    // On a case-insensitive volume a snapshot can hold two spellings of one file; deleting the
    // spelling the checkpoint lacks also removed the one it keeps. Put those back.
    let deleted: HashSet<String> = differences.iter()
        .filter(|difference| difference.action == "delete")
        .map(|difference| difference.path.to_lowercase())
        .collect();
    if !deleted.is_empty() {
        let listing = shadow.git(["ls-tree", "-r", "-z", "--name-only", target], None)?;
        for path in listing.split(|byte| *byte == 0).filter(|path| !path.is_empty()) {
            let path = String::from_utf8_lossy(path).into_owned();
            if deleted.contains(&path.to_lowercase())
                && !differences.iter().any(|difference| difference.path == path)
                && fs::symlink_metadata(root.join(&path)).is_err()
            {
                writes.push(path);
            }
        }
    }
    if !writes.is_empty() {
        let index = shadow.git_dir.join(format!("index-restore-{}", uuid::Uuid::new_v4()));
        let temporary = shadow.with_index(index.clone());
        let result = temporary.git(["read-tree", target], None)
            .and_then(|_| temporary.git(["checkout-index", "-f", "-z", "--stdin"], Some(nul_list(writes.iter().map(String::as_str)))));
        let _ = fs::remove_file(&index);
        result?;
        restored.extend(writes);
    }
    restored.sort();
    skipped.sort();
    Ok((restored, skipped))
}

/// A write that would replace something the current snapshot does not hold: a file or folder
/// that is ignored, oversized, or otherwise untracked, at the path or at one of its parents.
fn write_blocked(root: &Path, relative: &str, absent_from_current: bool) -> bool {
    let path = root.join(relative);
    let mut ancestor = path.parent();
    while let Some(directory) = ancestor {
        if directory == root { break; }
        if fs::symlink_metadata(directory).is_ok_and(|metadata| !metadata.is_dir()) { return true; }
        ancestor = directory.parent();
    }
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() => fs::read_dir(&path).map(|mut entries| entries.next().is_some()).unwrap_or(true)
            || fs::remove_dir(&path).is_err(),
        Ok(_) => absent_from_current,
        Err(_) => false,
    }
}

/// Remove folders a restore emptied, up to (not including) the root.
fn prune_empty(directory: &Path, root: &Path) {
    let mut current = Some(directory.to_path_buf());
    while let Some(directory) = current {
        if directory == root || !directory.starts_with(root) { break; }
        if fs::remove_dir(&directory).is_err() { break; }
        current = directory.parent().map(Path::to_path_buf);
    }
}

/// Copy a chat's checkpoint store for a fork. Objects are immutable, so they are hard-linked
/// where possible; the index describes the source's work tree and is left behind.
pub fn copy_shadow(from: &Path, to: &Path) -> Result<(), String> {
    if !from.join("HEAD").exists() { return Ok(()); }
    copy_tree(from, to, from).map_err(|error| format!("Could not copy the chat's checkpoints: {error}"))
}

fn copy_tree(from: &Path, to: &Path, root: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let source = entry.path();
        let destination = to.join(entry.file_name());
        let name = entry.file_name().to_string_lossy().into_owned();
        if source.parent() == Some(root) && (name == "index" || name.starts_with("index-") || name.ends_with(".lock")) { continue; }
        if entry.file_type()?.is_dir() {
            copy_tree(&source, &destination, root)?;
        } else if source.strip_prefix(root).is_ok_and(|relative| relative.starts_with("objects")) {
            if fs::hard_link(&source, &destination).is_err() { fs::copy(&source, &destination)?; }
        } else {
            fs::copy(&source, &destination)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn git(path: &Path, args: &[&str]) {
        let status = clean_git().arg("-C").arg(path).args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null")
            .stdout(Stdio::null()).status().unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn repo(path: &Path) {
        fs::create_dir_all(path).unwrap();
        git(path, &["init", "--quiet"]);
        git(path, &["config", "user.email", "test@example.com"]);
        git(path, &["config", "user.name", "Test"]);
    }

    fn commit_all(path: &Path) {
        git(path, &["add", "-A"]);
        git(path, &["commit", "--quiet", "-m", "commit"]);
    }

    struct Fixture {
        _directory: tempfile::TempDir,
        root: PathBuf,
        shadow: PathBuf,
        app_data: PathBuf,
    }

    fn fixture(with_repo: bool) -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        let app_data = directory.path().join("app-data");
        fs::create_dir_all(&app_data).unwrap();
        if with_repo { repo(&root) } else { fs::create_dir_all(&root).unwrap() }
        let shadow = shadow_dir(&app_data, "task");
        Fixture { root, shadow, app_data, _directory: directory }
    }

    impl Fixture {
        fn snapshot(&self) -> CheckpointRef { snapshot(&self.shadow, &self.root, &self.app_data).unwrap() }
        fn restore(&self, id: &str, paths: Option<&[String]>) -> RestoreResult {
            restore(&self.shadow, &self.root, &self.app_data, id, paths).unwrap()
        }
        fn read(&self, path: &str) -> Option<Vec<u8>> { fs::read(self.root.join(path)).ok() }
    }

    #[test]
    fn restores_modified_added_and_deleted_files_byte_for_byte() {
        let fixture = fixture(true);
        fs::write(fixture.root.join("tracked.txt"), "one\r\ntwo\r\n").unwrap();
        fs::write(fixture.root.join(".gitattributes"), "* text=auto eol=lf\n").unwrap();
        fs::write(fixture.root.join("gone.txt"), "keep me\n").unwrap();
        commit_all(&fixture.root);
        fs::create_dir_all(fixture.root.join("nested/deep")).unwrap();
        fs::write(fixture.root.join("nested/deep/new\nline ü.txt"), "untracked\n").unwrap();
        fs::write(fixture.root.join("tool.sh"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(fixture.root.join("tool.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        symlink("tracked.txt", fixture.root.join("link")).unwrap();
        let before = fixture.snapshot();
        assert!(before.head.is_some());

        fs::write(fixture.root.join("tracked.txt"), "changed\n").unwrap();
        fs::remove_file(fixture.root.join("gone.txt")).unwrap();
        fs::remove_file(fixture.root.join("tool.sh")).unwrap();
        fs::remove_file(fixture.root.join("link")).unwrap();
        fs::remove_dir_all(fixture.root.join("nested")).unwrap();
        fs::create_dir_all(fixture.root.join("added/dir")).unwrap();
        fs::write(fixture.root.join("added/dir/file.txt"), "agent wrote this\n").unwrap();

        let mut listed = changes(&fixture.shadow, &fixture.root, &fixture.app_data, &before.id).unwrap();
        listed.sort_by(|left, right| left.path.cmp(&right.path));
        let summary: Vec<(String, String)> = listed.into_iter().map(|change| (change.path, change.status)).collect();
        assert!(summary.contains(&("added/dir/file.txt".into(), "delete".into())));
        assert!(summary.contains(&("gone.txt".into(), "recreate".into())));
        assert!(summary.contains(&("tracked.txt".into(), "revert".into())));

        let result = fixture.restore(&before.id, None);
        assert!(result.skipped.is_empty(), "{:?}", result.skipped);
        assert_eq!(fixture.read("tracked.txt").unwrap(), b"one\r\ntwo\r\n");
        assert_eq!(fixture.read("gone.txt").unwrap(), b"keep me\n");
        assert_eq!(fixture.read("nested/deep/new\nline ü.txt").unwrap(), b"untracked\n");
        assert_eq!(fs::metadata(fixture.root.join("tool.sh")).unwrap().permissions().mode() & 0o111, 0o111);
        assert_eq!(fs::read_link(fixture.root.join("link")).unwrap(), PathBuf::from("tracked.txt"));
        assert!(!fixture.root.join("added").exists(), "emptied folders are pruned");

        // The state just before the restore is itself a checkpoint.
        fixture.restore(&result.undo.id, None);
        assert_eq!(fixture.read("tracked.txt").unwrap(), b"changed\n");
        assert!(fixture.root.join("added/dir/file.txt").exists());
    }

    #[test]
    fn never_writes_over_ignored_or_untracked_files_and_honours_a_selection() {
        let fixture = fixture(true);
        fs::write(fixture.root.join("a.txt"), "a1\n").unwrap();
        fs::write(fixture.root.join("b.txt"), "b1\n").unwrap();
        fs::write(fixture.root.join("secret.env"), "old secret\n").unwrap();
        let first = fixture.snapshot();

        // Deleted, then recreated as an ignored file: the current snapshot no longer holds it.
        fs::remove_file(fixture.root.join("secret.env")).unwrap();
        fixture.snapshot();
        fs::write(fixture.root.join(".gitignore"), "secret.env\n").unwrap();
        fs::write(fixture.root.join("secret.env"), "new secret\n").unwrap();
        fs::write(fixture.root.join("a.txt"), "a2\n").unwrap();
        fs::write(fixture.root.join("b.txt"), "b2\n").unwrap();

        let selection = vec!["a.txt".to_string(), "secret.env".to_string(), "not-a-change.txt".to_string()];
        let result = fixture.restore(&first.id, Some(&selection));
        assert_eq!(fixture.read("a.txt").unwrap(), b"a1\n");
        assert_eq!(fixture.read("b.txt").unwrap(), b"b2\n", "unselected files stay");
        assert_eq!(fixture.read("secret.env").unwrap(), b"new secret\n", "an ignored file is never overwritten");
        assert_eq!(result.skipped, vec!["secret.env".to_string()]);
        assert_eq!(result.restored, vec!["a.txt".to_string()]);
    }

    #[test]
    fn leaves_oversized_files_out_and_never_restores_a_stale_copy() {
        let fixture = fixture(false);
        fs::write(fixture.root.join("big.bin"), b"small at first").unwrap();
        let first = fixture.snapshot();
        let file = fs::File::create(fixture.root.join("big.bin")).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        let second = fixture.snapshot();
        let listed = changes(&fixture.shadow, &fixture.root, &fixture.app_data, &second.id).unwrap();
        assert!(listed.is_empty());
        let result = fixture.restore(&first.id, None);
        assert_eq!(result.skipped, vec!["big.bin".to_string()]);
        assert_eq!(fs::metadata(fixture.root.join("big.bin")).unwrap().len(), MAX_FILE_BYTES + 1);
    }

    #[test]
    fn handles_case_only_renames_and_a_file_replacing_a_folder() {
        let fixture = fixture(false);
        fs::write(fixture.root.join("Readme.md"), "title\n").unwrap();
        fs::create_dir_all(fixture.root.join("out")).unwrap();
        fs::write(fixture.root.join("out/one.txt"), "1\n").unwrap();
        let first = fixture.snapshot();
        fs::rename(fixture.root.join("Readme.md"), fixture.root.join("tmp.md")).unwrap();
        fs::rename(fixture.root.join("tmp.md"), fixture.root.join("README.md")).unwrap();
        fs::remove_dir_all(fixture.root.join("out")).unwrap();
        fs::write(fixture.root.join("out"), "now a file\n").unwrap();
        fixture.restore(&first.id, None);
        // Whatever spelling survives, the file itself must.
        assert_eq!(fixture.read("Readme.md").unwrap(), b"title\n");
        assert_eq!(fixture.read("out/one.txt").unwrap(), b"1\n");
    }

    #[test]
    fn a_case_only_rename_never_deletes_the_file_even_when_the_store_is_case_sensitive() {
        let fixture = fixture(false);
        fs::write(fixture.root.join("Readme.md"), "title\n").unwrap();
        let first = fixture.snapshot();
        // A store that believes the volume is case-sensitive records both spellings.
        Shadow::new(&fixture.shadow, &fixture.root).git(["config", "core.ignorecase", "false"], None).unwrap();
        fs::rename(fixture.root.join("Readme.md"), fixture.root.join("tmp.md")).unwrap();
        fs::rename(fixture.root.join("tmp.md"), fixture.root.join("README.md")).unwrap();
        fixture.restore(&first.id, None);
        assert_eq!(fixture.read("Readme.md").unwrap(), b"title\n");
    }

    #[test]
    fn keeps_a_folder_with_ignored_content_when_a_file_would_replace_it() {
        let fixture = fixture(false);
        fs::write(fixture.root.join(".gitignore"), "build/cache\n").unwrap();
        fs::write(fixture.root.join("build"), "was a file\n").unwrap();
        let first = fixture.snapshot();
        fs::remove_file(fixture.root.join("build")).unwrap();
        fs::create_dir_all(fixture.root.join("build")).unwrap();
        fs::write(fixture.root.join("build/cache"), "ignored\n").unwrap();
        let result = fixture.restore(&first.id, None);
        assert_eq!(result.skipped, vec!["build".to_string()]);
        assert_eq!(fixture.read("build/cache").unwrap(), b"ignored\n");
    }

    #[test]
    fn borrows_unchanged_objects_from_the_project_instead_of_copying_them() {
        let fixture = fixture(true);
        for index in 0..50 { fs::write(fixture.root.join(format!("file-{index}.txt")), format!("content {index}\n")).unwrap(); }
        commit_all(&fixture.root);
        fixture.snapshot();
        let loose = fs::read_dir(fixture.shadow.join("objects")).unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().len() == 2)
            .map(|entry| fs::read_dir(entry.path()).unwrap().count())
            .sum::<usize>();
        assert_eq!(loose, 0, "every blob and the root tree already exist in the project");
        assert!(fs::read_to_string(fixture.shadow.join("objects/info/alternates")).unwrap().contains(".git/objects"));
        assert!(fs::read_dir(fixture.root.join(".git/refs")).unwrap().all(|entry| entry.unwrap().file_name() != "wackcode"));
    }

    #[test]
    fn skips_a_nested_repository_without_a_commit() {
        let fixture = fixture(false);
        fs::write(fixture.root.join("main.txt"), "main\n").unwrap();
        repo(&fixture.root.join("nested"));
        fs::write(fixture.root.join("nested/inner.txt"), "inner\n").unwrap();
        let first = fixture.snapshot();
        fs::write(fixture.root.join("main.txt"), "changed\n").unwrap();
        fixture.restore(&first.id, None);
        assert_eq!(fixture.read("main.txt").unwrap(), b"main\n");
        assert_eq!(fixture.read("nested/inner.txt").unwrap(), b"inner\n");
    }

    #[test]
    fn refuses_home_folders_folders_holding_app_data_and_unknown_checkpoints() {
        let fixture = fixture(false);
        let parent = fixture.root.parent().unwrap().to_path_buf();
        assert!(snapshot(&fixture.shadow, &parent, &fixture.app_data).unwrap_err().contains("WackCode's own data"));
        fixture.snapshot();
        assert!(restore(&fixture.shadow, &fixture.root, &fixture.app_data, &"0".repeat(40), None).unwrap_err().contains("no longer available"));
        assert!(restore(&fixture.shadow, &fixture.root, &fixture.app_data, "HEAD", None).unwrap_err().contains("not valid"));
        assert!(!valid_checkpoint_id("--output=/tmp/x"));
        assert!(valid_checkpoint_id(&"a".repeat(64)));
    }

    #[test]
    fn materializes_a_checkpoint_into_a_fresh_folder_and_copies_the_store() {
        let fixture = fixture(false);
        fs::create_dir_all(fixture.root.join("src")).unwrap();
        fs::write(fixture.root.join("src/lib.rs"), "fn main() {}\n").unwrap();
        let checkpoint = fixture.snapshot();

        let copy = fixture.app_data.join("checkpoints").join("fork");
        copy_shadow(&fixture.shadow, &copy).unwrap();
        assert!(!copy.join("index").exists());
        let target = fixture.root.parent().unwrap().join("fork-workspace");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("stale.txt"), "left from the base\n").unwrap();
        let skipped = materialize(&copy, &checkpoint.id, &target).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(fs::read(target.join("src/lib.rs")).unwrap(), b"fn main() {}\n");
        assert!(!target.join("stale.txt").exists());
        // The copy stands alone: the source store can go.
        fs::remove_dir_all(&fixture.shadow).unwrap();
        assert!(changes(&copy, &target, &fixture.app_data, &checkpoint.id).unwrap().is_empty());
    }
}
