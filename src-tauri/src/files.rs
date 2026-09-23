//! The workspace file list behind `@` mentions in the composer.
use crate::{git, models::WorkspaceFiles};
use std::{fs, path::{Path, PathBuf}};

/// More files than this are cut off; the picker says so.
pub const MAX_FILES: usize = 20_000;
/// A folder outside Git is walked at most this deep.
const MAX_DEPTH: usize = 12;
/// Never descended into by the plain walk (Git repositories use their own ignore rules).
const SKIPPED_DIRS: [&str; 2] = [".git", "node_modules"];

/// Files under `root`, relative to it and `/`-separated. Inside a Git repository this is every
/// tracked or untracked file that isn't ignored; anywhere else a bounded walk of the folder.
pub fn list(root: &Path) -> Result<WorkspaceFiles, String> {
    if !root.is_dir() { return Err("This chat's folder no longer exists.".into()); }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if same_path(root, &home) {
            return Err("File mentions are off for a home folder. Open a project folder instead.".into());
        }
    }
    let mut files = match git::ls_files(root) {
        Ok(listed) => listed.into_iter().filter(|path| root.join(path).is_file()).take(MAX_FILES + 1).collect(),
        Err(_) => walk(root),
    };
    let truncated = files.len() > MAX_FILES;
    files.truncate(MAX_FILES);
    files.sort();
    Ok(WorkspaceFiles { files, truncated })
}

/// Stops once it has one file more than the cap, so a huge folder costs no more than that.
fn walk(root: &Path) -> Vec<String> {
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
                if depth < MAX_DEPTH && !SKIPPED_DIRS.iter().any(|skip| name == *skip) { stack.push((path, depth + 1)); }
            } else if kind.is_file() || (kind.is_symlink() && path.is_file()) {
                if let Ok(relative) = path.strip_prefix(root) {
                    files.push(relative.to_string_lossy().into_owned());
                    if files.len() > MAX_FILES { return files; }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(path: &Path, args: &[&str]) {
        let status = Command::new("git").arg("-C").arg(path).args(args).status().unwrap();
        assert!(status.success());
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
        git(root, &["init", "-q"]);
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
        git(root, &["init", "-q"]);
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
        let mut files = walk(root);
        files.sort();
        assert_eq!(files, vec!["a.txt", "deep/er/b.txt"]);
    }

    #[test]
    fn the_walk_stops_past_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("many")).unwrap();
        for index in 0..=MAX_FILES + 5 { fs::write(root.join("many").join(index.to_string()), "").unwrap(); }
        let listed = list(root).unwrap();
        assert_eq!(listed.files.len(), MAX_FILES);
        assert!(listed.truncated);
    }

    #[test]
    fn a_missing_folder_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list(&dir.path().join("gone")).is_err());
    }
}
