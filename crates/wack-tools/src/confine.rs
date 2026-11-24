//! Keeping a Chat chat's file tools inside its scratchpad (docs/security.md, "Chat's
//! scratchpad"), ported from version 2's `chat-mode/policy.ts`.
//!
//! Every file tool gets its path through [`crate::text::resolve`], which comes here whenever the
//! run is confined, so no tool can reach a path this check has not seen. The check fails closed:
//! anything that cannot be shown to be inside is refused. It compares *real* paths, so a symlink
//! out of the scratchpad, a sibling folder whose name starts with the scratchpad's, `..`, `~`,
//! absolute paths and `file://` addresses are all caught, and the tool is handed the location
//! that was checked (never the model's own spelling), so the check and the operation can never
//! disagree about which file is meant. `grep` and `find` never follow links while walking
//! (`walk.rs`), so a link inside cannot lead them out either.
//!
//! It is a rule about the agent's file tools, not a sandbox: it cannot see a hard link placed
//! inside the scratchpad by something else.

use std::path::{Component, Path, PathBuf};

const NO_SCRATCHPAD: &str = "This chat's scratchpad folder is not available.";

/// The real location `raw` names, for `tool`, if it is inside `root`; otherwise why not, in a
/// sentence for the model.
pub fn confine(tool: &str, raw: &str, root: &Path) -> Result<PathBuf, String> {
    let root = root.canonicalize().ok().filter(|root| root.is_dir()).ok_or(NO_SCRATCHPAD)?;
    let outside = || format!("This chat can only use files in its scratchpad folder, and '{raw}' is outside it.");
    let resolved = lexical(&spelled(raw, &root).ok_or_else(outside)?);
    let canonical = canonical(&resolved).ok_or_else(outside)?;
    if !canonical.starts_with(&root) {
        return Err(outside());
    }
    if tool == "read" && std::fs::symlink_metadata(&canonical).is_err() {
        return Err(format!("There is no file '{raw}' in the scratchpad."));
    }
    Ok(canonical)
}

/// What the model wrote as an absolute path: `~` is the home folder, `file://` an address of a
/// file on this Mac, anything relative is under the scratchpad.
fn spelled(raw: &str, root: &Path) -> Option<PathBuf> {
    let raw = raw.trim();
    let raw = match raw.strip_prefix("file://") {
        // Only this Mac's files: `file:///path` or `file://localhost/path`.
        Some(rest) => {
            let path = rest.strip_prefix("localhost").unwrap_or(rest);
            if !path.starts_with('/') {
                return None;
            }
            return Some(PathBuf::from(percent_decode(path)?));
        }
        None => raw,
    };
    if raw == "~" || raw.starts_with("~/") {
        let home = std::env::var_os("HOME")?;
        return Some(Path::new(&home).join(raw[1..].trim_start_matches('/')));
    }
    Some(root.join(raw))
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = std::str::from_utf8(bytes.get(at + 1..at + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `.` and `..` taken out by their spelling, as the model meant them. The operation then uses the
/// result, so what the OS would make of `link/..` never comes into it.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The real location an absolute path names: links resolved through the deepest part that
/// exists, the part not yet made appended as written. `None` when that part cannot be resolved
/// (a dangling link), which counts as outside: writing through it would make the file it points
/// at.
fn canonical(path: &Path) -> Option<PathBuf> {
    let mut existing = path.to_path_buf();
    let mut rest = Vec::new();
    while std::fs::symlink_metadata(&existing).is_err() {
        rest.push(existing.file_name()?.to_os_string());
        existing = existing.parent()?.to_path_buf();
    }
    let mut real = existing.canonicalize().ok()?;
    for part in rest.iter().rev() {
        real.push(part);
    }
    Some(real)
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink};

    use super::*;

    struct Folders {
        _base: tempfile::TempDir,
        scratch: PathBuf,
        outside: PathBuf,
    }

    /// A scratchpad holding `notes/a.md`, and a sibling folder whose name starts with its own.
    fn folders() -> Folders {
        let base = tempfile::tempdir().unwrap();
        // macOS's temp folder is itself behind a link; the check compares real paths.
        let real = base.path().canonicalize().unwrap();
        let scratch = real.join("scratch");
        let outside = real.join("scratch-other");
        fs::create_dir_all(scratch.join("notes")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(scratch.join("notes/a.md"), "inside").unwrap();
        fs::write(outside.join("secret.txt"), "outside").unwrap();
        Folders { _base: base, scratch, outside }
    }

    #[test]
    fn allows_files_in_the_scratchpad_at_their_real_location() {
        let f = folders();
        let a = f.scratch.join("notes/a.md");
        assert_eq!(confine("read", "notes/a.md", &f.scratch).unwrap(), a);
        assert_eq!(confine("read", a.to_str().unwrap(), &f.scratch).unwrap(), a);
        assert_eq!(confine("edit", "./notes/../notes/a.md", &f.scratch).unwrap(), a);
        assert_eq!(confine("read", &format!("file://{}", a.display()), &f.scratch).unwrap(), a);
        assert_eq!(confine("ls", ".", &f.scratch).unwrap(), f.scratch);
        assert_eq!(confine("ls", f.scratch.to_str().unwrap(), &f.scratch).unwrap(), f.scratch);
    }

    #[test]
    fn allows_a_new_file_even_in_folders_not_made_yet() {
        let f = folders();
        assert_eq!(confine("write", "draft.md", &f.scratch).unwrap(), f.scratch.join("draft.md"));
        assert_eq!(confine("write", "new/deep/draft.md", &f.scratch).unwrap(), f.scratch.join("new/deep/draft.md"));
    }

    #[test]
    fn refuses_every_way_of_naming_a_file_outside() {
        let f = folders();
        let secret = f.outside.join("secret.txt");
        let escapes = [
            "../scratch-other/secret.txt".to_string(),
            "notes/../../scratch-other/secret.txt".to_string(),
            "new/../../scratch-other/secret.txt".to_string(),
            secret.display().to_string(),
            "/etc/hosts".to_string(),
            "~".to_string(),
            "~/.ssh/id_rsa".to_string(),
            format!("file://{}", secret.display()),
            "file://example.com/etc/hosts".to_string(),
            "file:///etc/hosts".to_string(),
            "..".to_string(),
        ];
        for tool in ["read", "write", "edit", "ls", "grep", "find"] {
            for path in &escapes {
                let refused = confine(tool, path, &f.scratch).unwrap_err();
                assert!(refused.contains("outside it"), "{tool} {path}: {refused}");
            }
        }
    }

    #[test]
    fn is_not_fooled_by_a_sibling_whose_name_starts_with_the_scratchpads() {
        let f = folders();
        let sibling = format!("{}-other/secret.txt", f.scratch.display());
        assert!(confine("read", &sibling, &f.scratch).is_err());
    }

    #[test]
    fn refuses_a_link_that_leaves_as_a_folder_a_file_or_dangling() {
        let f = folders();
        symlink(&f.outside, f.scratch.join("link-dir")).unwrap();
        symlink(f.outside.join("secret.txt"), f.scratch.join("link-file")).unwrap();
        symlink(f.outside.join("not-there.txt"), f.scratch.join("dangling")).unwrap();
        assert!(confine("read", "link-dir/secret.txt", &f.scratch).is_err());
        assert!(confine("write", "link-dir/new.txt", &f.scratch).is_err());
        assert!(confine("ls", "link-dir", &f.scratch).is_err());
        assert!(confine("read", "link-file", &f.scratch).is_err());
        assert!(confine("edit", "link-file", &f.scratch).is_err());
        // Writing through a dangling link would make the file it points at.
        assert!(confine("write", "dangling", &f.scratch).is_err());
    }

    #[test]
    fn follows_a_link_that_stays_inside_to_its_real_location() {
        let f = folders();
        symlink(f.scratch.join("notes"), f.scratch.join("alias")).unwrap();
        assert_eq!(confine("read", "alias/a.md", &f.scratch).unwrap(), f.scratch.join("notes/a.md"));
    }

    #[test]
    fn says_a_missing_file_is_missing_rather_than_outside() {
        let f = folders();
        assert!(confine("read", "nope.md", &f.scratch).unwrap_err().contains("no file"));
    }

    #[test]
    fn keeps_names_that_look_like_other_spellings_inside() {
        let f = folders();
        fs::create_dir(f.scratch.join("~")).unwrap();
        fs::write(f.scratch.join("~/x.md"), "tilde folder").unwrap();
        fs::write(f.scratch.join("@handle.md"), "at file").unwrap();
        assert_eq!(confine("read", "./~/x.md", &f.scratch).unwrap(), f.scratch.join("~/x.md"));
        assert_eq!(confine("read", "@handle.md", &f.scratch).unwrap(), f.scratch.join("@handle.md"));
    }

    #[test]
    fn refuses_everything_when_the_scratchpad_is_gone() {
        let f = folders();
        let gone = f.scratch.parent().unwrap().join("missing");
        assert_eq!(confine("write", "a.md", &gone).unwrap_err(), NO_SCRATCHPAD);
    }
}
