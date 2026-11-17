//! The user's login-shell environment, for the processes that run the user's own tools: `bash`
//! commands and stdio MCP servers. Ported from version 2's `shell_env.rs`.
//!
//! An app opened from the Dock inherits launchd's minimal environment: no Homebrew, nvm or
//! `~/.local/bin` on PATH, so `npx` and `uvx` are not found. The app calls [`warm`] at launch,
//! which reads the environment an interactive login shell would have, once, on its own thread.
//!
//! Invariants:
//! - Nothing reads the shell unless [`warm`] was called. `wack run` and the tests never call it,
//!   so they keep the environment of the shell that started them.
//! - [`resolved`] never waits: a command started before the shell answered inherits the app's
//!   own environment. [`wait`] waits a bounded time, for a server about to start.
//! - Callers apply these variables first, then remove provider keys (`SECRET_ENV`), then add
//!   anything specific to the process (an MCP server's own variables).

use std::{
    collections::{HashMap, HashSet},
    io::Read,
    process::{Command, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use std::os::unix::process::CommandExt;

const START: &str = "__WACKCODE_ENV_START__";
const END: &str = "__WACKCODE_ENV_END__";
/// A profile that takes longer than this is abandoned, and the fallback is used.
const TIMEOUT: Duration = Duration::from_secs(5);
/// Shell bookkeeping that means nothing to a child started elsewhere.
const SKIPPED: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_"];
/// Where Homebrew and other tools usually live, for when the shell can't be read.
const FALLBACK_PATHS: &[&str] = &["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin"];

static ENVIRONMENT: OnceLock<HashMap<String, String>> = OnceLock::new();
static STARTED: AtomicBool = AtomicBool::new(false);

/// Starts reading the login environment on its own thread. Calling it again does nothing.
pub fn warm() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let _ = ENVIRONMENT.set(capture().unwrap_or_else(fallback));
    });
}

/// The login environment, once it has been read.
pub fn resolved() -> Option<&'static HashMap<String, String>> {
    ENVIRONMENT.get()
}

/// The login environment, waiting up to the shell's own timeout for it when it is being read.
/// `None` when [`warm`] was never called.
pub async fn wait() -> Option<&'static HashMap<String, String>> {
    if !STARTED.load(Ordering::SeqCst) {
        return None;
    }
    let deadline = Instant::now() + TIMEOUT + Duration::from_secs(1);
    loop {
        if let Some(environment) = ENVIRONMENT.get() {
            return Some(environment);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Runs the user's shell as an interactive login shell (so both `.zprofile` and `.zshrc` apply)
/// and prints its environment between markers, which keeps any greeting a profile prints out.
fn capture() -> Option<HashMap<String, String>> {
    let shell = std::env::var("SHELL").ok().filter(|shell| shell.starts_with('/')).unwrap_or_else(|| "/bin/zsh".into());
    let script = format!("printf '%s' '{START}'; /usr/bin/env -0; printf '%s' '{END}'");
    let mut child = Command::new(shell)
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .ok()?;
    let pgid = child.id() as i32;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + TIMEOUT;
    let finished = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => break false,
        }
    };
    // Anything the profile started in the background goes too, which also ends the reader.
    // SAFETY: a plain system call on the group this function started.
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
    let _ = child.wait();
    let bytes = reader.join().ok()?;
    if !finished {
        return None;
    }
    parse(&bytes)
}

fn parse(stdout: &[u8]) -> Option<HashMap<String, String>> {
    let text = String::from_utf8_lossy(stdout);
    let start = text.find(START)? + START.len();
    let end = start + text[start..].find(END)?;
    let environment: HashMap<String, String> = text[start..end]
        .split('\0')
        .filter_map(|entry| entry.split_once('='))
        .filter(|(key, _)| !key.is_empty() && !SKIPPED.contains(key))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    environment.contains_key("PATH").then_some(environment)
}

/// The app's own environment with the usual tool folders added to PATH.
fn fallback() -> HashMap<String, String> {
    let mut environment: HashMap<String, String> = std::env::vars().collect();
    let current = environment.get("PATH").cloned().unwrap_or_else(|| "/usr/bin:/bin:/usr/sbin:/sbin".into());
    let mut seen = HashSet::new();
    let paths: Vec<&str> =
        FALLBACK_PATHS.iter().copied().chain(current.split(':')).filter(|path| !path.is_empty() && seen.insert(*path)).collect();
    environment.insert("PATH".into(), paths.join(":"));
    environment
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_what_sits_between_the_markers() {
        let output = format!("Welcome back!\n{START}PATH=/opt/homebrew/bin:/usr/bin\0HOME=/Users/me\0SHLVL=2\0EMPTY=\0MULTI=a\nb\0{END}bye");
        let environment = parse(output.as_bytes()).unwrap();
        assert_eq!(environment["PATH"], "/opt/homebrew/bin:/usr/bin");
        assert_eq!(environment["HOME"], "/Users/me");
        assert_eq!(environment["EMPTY"], "");
        assert_eq!(environment["MULTI"], "a\nb");
        assert!(!environment.contains_key("SHLVL"));
    }

    #[test]
    fn output_without_markers_or_path_is_rejected() {
        assert!(parse(b"PATH=/usr/bin").is_none());
        assert!(parse(format!("{START}HOME=/Users/me\0{END}").as_bytes()).is_none());
    }

    #[test]
    fn the_fallback_adds_homebrew_to_path_once() {
        let path = fallback()["PATH"].clone();
        assert!(path.split(':').any(|entry| entry == "/opt/homebrew/bin"));
        assert_eq!(path.split(':').filter(|entry| *entry == "/opt/homebrew/bin").count(), 1);
    }

    #[tokio::test]
    async fn nothing_is_read_unless_warmed() {
        // No test here warms, so commands keep the environment they were started with.
        assert!(wait().await.is_none());
        assert!(resolved().is_none());
    }
}
