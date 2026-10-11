//! Which process the driver may touch. The rule, checked fresh on every call:
//!
//! - a running process has bundle id **exactly** `com.wackcode.desktop.dev`, and
//! - its executable lives inside this checkout's `src-tauri/target/` (symlinks followed).
//!
//! Anything else fails with a plain sentence; the driver never falls back to the installed
//! app or anything else, and has no option to aim elsewhere. The window itself ("WackCode
//! Dev" titled) is picked with Accessibility at call time in `wackdev-helper`, which also
//! reports it in `status`. `pick` is pure so the whole rule is unit-tested.

use crate::apps;
use std::path::{Path, PathBuf};

/// The dev app's bundle id — `tauri.dev.conf.json` — never shared with any installed build.
pub const DEV_BUNDLE_ID: &str = "com.wackcode.desktop.dev";

/// The dev window's title, set from `productName` in debug builds (`lib.rs` setup).
pub const DEV_WINDOW_TITLE: &str = "WackCode Dev";

/// One running process, as much of it as the targeting rule reads.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub pid: i32,
    pub bundle_id: Option<String>,
    pub executable: Option<PathBuf>,
}

/// The dev app process the driver resolved this call.
#[derive(Debug, Clone, PartialEq)]
pub struct DevTarget {
    pub pid: i32,
    /// The dev executable's canonical path (inside the target dir).
    pub executable: PathBuf,
}

pub const NOT_RUNNING: &str = "The dev app isn't running. Start it with `pnpm dev:background`.";

fn canonical(path: &Path) -> Option<PathBuf> {
    path.canonicalize().ok()
}

fn inside(path: &Path, root: &Path) -> bool {
    canonical(path).is_some_and(|path| path.starts_with(root))
}

/// Applies the rule to `candidates` (already-running apps) under `target_root` (the checkout's
/// `src-tauri/target`, canonicalized). Errs with the sentence to show instead of acting.
pub fn pick(candidates: &[Candidate], target_root: &Path) -> Result<DevTarget, String> {
    let claimed: Vec<&Candidate> = candidates
        .iter()
        .filter(|app| app.bundle_id.as_deref() == Some(DEV_BUNDLE_ID))
        .collect();
    if claimed.is_empty() {
        return Err(NOT_RUNNING.into());
    }
    let inside: Vec<&Candidate> = claimed
        .iter()
        .filter(|app| app.executable.as_deref().is_some_and(|path| inside(path, target_root)))
        .copied()
        .collect();
    match inside.as_slice() {
        [] => Err(format!(
            "A process claims {DEV_BUNDLE_ID}, but its executable isn't under {}, so it isn't this checkout's dev app.",
            target_root.display()
        )),
        [target] => Ok(DevTarget { pid: target.pid, executable: target.executable.clone().expect("checked") }),
        several => Err(format!(
            "More than one dev app process is running (pids {}). Stop them with `pnpm dev:stop`, then try again.",
            several.iter().map(|app| app.pid.to_string()).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// The running-app candidates `pick` filters.
pub fn candidates() -> Vec<Candidate> {
    apps::running()
        .into_iter()
        .map(|app| Candidate { pid: app.pid, bundle_id: app.bundle_id, executable: app.executable })
        .collect()
}

/// This checkout's `src-tauri/target`, found by walking up from the helper's own executable
/// (which Cargo builds inside it).
pub fn target_root() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let target = executable.ancestors().find(|dir| dir.file_name().is_some_and(|name| name == "target"))?;
    canonical(target)
}

/// One-call resolution: running candidates + this checkout's target dir.
pub fn resolve() -> Result<DevTarget, String> {
    let root = target_root().ok_or("The dev driver can't find its target directory; it must run from inside this checkout's src-tauri/target.")?;
    pick(&candidates(), &root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(pid: i32, bundle_id: &str, executable: &str) -> Candidate {
        Candidate { pid, bundle_id: Some(bundle_id.into()), executable: Some(PathBuf::from(executable)) }
    }

    fn root() -> PathBuf {
        PathBuf::from("/repo/src-tauri/target").canonicalize().unwrap_or_else(|_| PathBuf::from("/repo/src-tauri/target"))
    }

    #[test]
    fn the_only_dev_app_wins() {
        let candidates = vec![
            candidate(1, "com.wackcode.desktop", "/repo/src-tauri/target/debug/dev-app/WackCode Dev.app/Contents/MacOS/wackcode"),
            candidate(2, DEV_BUNDLE_ID, "/repo/src-tauri/target/debug/dev-app/WackCode Dev.app/Contents/MacOS/wackcode"),
        ];
        // /repo doesn't exist; canonicalization fails for both, so use real paths under target/.
        let root = std::env::temp_dir().canonicalize().unwrap();
        let inside = root.join("src-tauri/target/debug/wackcode");
        std::fs::create_dir_all(inside.parent().unwrap()).unwrap();
        std::fs::write(&inside, b"x").unwrap();
        let candidates = vec![
            Candidate { pid: 1, bundle_id: Some("com.wackcode.desktop".into()), executable: Some(inside.clone()) },
            Candidate { pid: 2, bundle_id: Some(DEV_BUNDLE_ID.into()), executable: Some(inside.clone()) },
        ];
        let target_root = root.join("src-tauri/target");
        let found = pick(&candidates, &target_root).unwrap();
        assert_eq!(found.pid, 2);
        assert_eq!(pick(&candidates[..1], &target_root).unwrap_err(), NOT_RUNNING);
    }

    #[test]
    fn a_dev_bundle_id_outside_the_target_dir_is_refused() {
        let root = std::env::temp_dir().canonicalize().unwrap();
        let target_root = root.join("target");
        let outside = root.join("elsewhere/wackcode");
        std::fs::create_dir_all(&target_root).unwrap();
        std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
        std::fs::write(&outside, b"x").unwrap();
        let candidates = vec![Candidate { pid: 7, bundle_id: Some(DEV_BUNDLE_ID.into()), executable: Some(outside) }];
        let error = pick(&candidates, &target_root).unwrap_err();
        assert!(error.contains("isn't this checkout's dev app"), "{error}");
    }

    #[test]
    fn nothing_running_and_ambiguity_are_plain_sentences() {
        let root = std::env::temp_dir().canonicalize().unwrap().join("target");
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(pick(&[], &root).unwrap_err(), NOT_RUNNING);
        let other = Candidate { pid: 9, bundle_id: Some("com.example.App".into()), executable: None };
        assert_eq!(pick(&[other], &root).unwrap_err(), NOT_RUNNING);
        let inside = root.join("wackcode");
        std::fs::write(&inside, b"x").unwrap();
        let make = |pid| Candidate { pid, bundle_id: Some(DEV_BUNDLE_ID.into()), executable: Some(inside.clone()) };
        let error = pick(&[make(3), make(4)], &root).unwrap_err();
        assert!(error.contains("More than one dev app") && error.contains("3") && error.contains("4"), "{error}");
    }
}
