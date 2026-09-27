//! The user's login-shell environment, for processes that run the user's own tools.
//!
//! An app opened from Finder inherits launchd's minimal environment: no Homebrew, nvm or
//! `~/.local/bin` on PATH. Chat workers (the agent's bash tool, stdio MCP servers) and the MCP
//! test process therefore get the environment an interactive login shell would have, read once
//! per app run. Provider API keys are still stripped afterwards (`worker::strip_provider_env`).

use nix::{sys::signal::{killpg, Signal}, unistd::Pid};
use std::{collections::{HashMap, HashSet}, process::Stdio, time::Duration};
use tokio::{process::Command, sync::OnceCell};

const START: &str = "__WACKCODE_ENV_START__";
const END: &str = "__WACKCODE_ENV_END__";
const TIMEOUT: Duration = Duration::from_secs(5);
/// Shell bookkeeping that means nothing to a child started elsewhere.
const SKIPPED: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_"];
/// Where Homebrew and other tools usually live, for when the shell can't be read.
const FALLBACK_PATHS: &[&str] = &["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin"];

static ENVIRONMENT: OnceCell<HashMap<String, String>> = OnceCell::const_new();

/// Read the login environment now, so the first chat doesn't wait for it.
pub async fn warm() {
    environment().await;
}

/// Give `command` the login environment. Call before setting anything more specific.
pub async fn apply(command: &mut Command) {
    for (key, value) in environment().await {
        command.env(key, value);
    }
}

async fn environment() -> &'static HashMap<String, String> {
    ENVIRONMENT.get_or_init(|| async { capture().await.unwrap_or_else(fallback) }).await
}

/// Run the user's shell as an interactive login shell (so both `.zprofile` and `.zshrc` apply)
/// and print its environment between markers, which keeps any greeting a profile prints out of it.
async fn capture() -> Option<HashMap<String, String>> {
    let shell = std::env::var("SHELL").ok().filter(|shell| shell.starts_with('/')).unwrap_or_else(|| "/bin/zsh".into());
    let script = format!("printf '%s' '{START}'; /usr/bin/env -0; printf '%s' '{END}'");
    let mut command = Command::new(shell);
    command
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    let child = command.spawn().ok()?;
    let pid = child.id();
    let output = tokio::time::timeout(TIMEOUT, child.wait_with_output()).await;
    // Anything the profile started in the background goes too.
    if let Some(pid) = pid { let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGKILL); }
    parse(&output.ok()?.ok()?.stdout)
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

/// The app's own environment with the usual tool directories added to PATH.
fn fallback() -> HashMap<String, String> {
    let mut environment: HashMap<String, String> = std::env::vars().collect();
    let current = environment.get("PATH").cloned().unwrap_or_else(|| "/usr/bin:/bin:/usr/sbin:/sbin".into());
    let mut seen = HashSet::new();
    let paths: Vec<&str> = FALLBACK_PATHS.iter().copied().chain(current.split(':'))
        .filter(|path| !path.is_empty() && seen.insert(*path))
        .collect();
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
}
