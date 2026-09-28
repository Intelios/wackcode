//! Which apps computer use may touch. Pure logic, so every rule is unit-tested.
//!
//! Three layers, checked in order on every request:
//! 1. WackCode itself — its own pid, any `com.wackcode.*` bundle (dev and installed builds share
//!    `com.wackcode.desktop`), and its own executable or bundle path. The access card lives in
//!    WackCode's window, so this is what keeps the agent from ever answering it.
//! 2. The hard list: apps that run arbitrary commands (terminals, script runners), hold secrets
//!    (password managers, Keychain Access) or are the system's own security and chrome UI.
//! 3. The user's never-allow list (Settings › Computer use).
//!
//! Past those, a chat needs a grant, which only the user gives through the access card. Grants
//! and denials are per chat and in memory: nothing here is ever written to disk.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const MAX_NEVER_ALLOW: usize = 200;

/// Bundle ids refused outright, with the reason shown to the model. Matched case-insensitively.
const HARD_BLOCKED: &[(&str, &str)] = &[
    // Terminals and script runners: controlling one is running arbitrary commands.
    ("com.apple.terminal", "terminal"),
    ("com.googlecode.iterm2", "terminal"),
    ("com.mitchellh.ghostty", "terminal"),
    ("dev.warp.warp-stable", "terminal"),
    ("dev.warp.warp", "terminal"),
    ("dev.warp.warp-preview", "terminal"),
    ("net.kovidgoyal.kitty", "terminal"),
    ("org.alacritty", "terminal"),
    ("io.alacritty", "terminal"),
    ("com.github.wez.wezterm", "terminal"),
    ("co.zeit.hyper", "terminal"),
    ("com.raphaelamorim.rio", "terminal"),
    ("dev.commandline.waveterm", "terminal"),
    ("org.tabby", "terminal"),
    ("com.termius-dmg.mac", "terminal"),
    ("com.apple.scripteditor2", "script runner"),
    ("com.apple.automator", "script runner"),
    ("com.apple.shortcuts", "script runner"),
    ("com.apple.installer", "installer"),
    // Secrets.
    ("com.apple.keychainaccess", "password manager"),
    ("com.apple.passwords", "password manager"),
    ("com.1password.1password", "password manager"),
    ("com.agilebits.onepassword7", "password manager"),
    ("com.agilebits.onepassword-osx", "password manager"),
    ("com.bitwarden.desktop", "password manager"),
    ("com.lastpass.lastpass", "password manager"),
    ("com.dashlane.dashlane", "password manager"),
    ("in.sinew.enpass-desktop", "password manager"),
    ("org.keepassxc.keepassxc", "password manager"),
    ("com.proton.pass", "password manager"),
    // The system's own security prompts and chrome.
    ("com.apple.loginwindow", "system security UI"),
    ("com.apple.securityagent", "system security UI"),
    ("com.apple.coreservices.uiagent", "system security UI"),
    ("com.apple.accessibility.universalaccessauthwarn", "system security UI"),
    ("com.apple.usernotificationcenter", "system UI"),
    ("com.apple.dock", "system UI"),
    ("com.apple.controlcenter", "system UI"),
    ("com.apple.systemuiserver", "system UI"),
    ("com.apple.notificationcenterui", "system UI"),
    ("com.apple.spotlight", "system UI"),
    ("com.apple.windowmanager", "system UI"),
];

/// Bundle-id prefixes refused outright (System Settings has changed ids across releases).
const HARD_BLOCKED_PREFIXES: &[(&str, &str)] = &[
    ("com.wackcode.", "WackCode"),
    ("com.apple.systempreferences", "System Settings"),
    ("com.apple.settings", "System Settings"),
];

/// A target app as the host resolved it: `path` is the canonical `.app` bundle path, or the
/// executable for apps without a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIdentity {
    pub pid: Option<i32>,
    pub name: String,
    pub bundle_id: Option<String>,
    pub path: Option<PathBuf>,
}

/// WackCode's own process, taken once at startup.
#[derive(Debug, Clone, Default)]
pub struct OwnIdentity {
    pub pid: i32,
    pub executable: Option<PathBuf>,
    pub bundle: Option<PathBuf>,
}

impl OwnIdentity {
    pub fn current() -> Self {
        let executable = std::env::current_exe().ok().and_then(|path| path.canonicalize().ok());
        let bundle = executable.as_deref().and_then(enclosing_bundle);
        Self { pid: std::process::id() as i32, executable, bundle }
    }
}

/// The `.app` directory an executable lives in (`X.app/Contents/MacOS/x`), if any.
pub fn enclosing_bundle(executable: &Path) -> Option<PathBuf> {
    executable.ancestors().find(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app"))).map(Path::to_path_buf)
}

/// Why `app` can never be used, as a sentence for the model; `None` when a grant may allow it.
pub fn block_reason(app: &AppIdentity, own: &OwnIdentity, never_allow: &[String]) -> Option<String> {
    let bundle_id = app.bundle_id.as_deref().map(str::to_ascii_lowercase);
    let is_own = app.pid == Some(own.pid)
        || bundle_id.as_deref().is_some_and(|id| id.starts_with("com.wackcode."))
        || app.path.as_ref().is_some_and(|path| Some(path) == own.executable.as_ref() || Some(path) == own.bundle.as_ref());
    if is_own {
        return Some("WackCode can't control or look at its own windows.".into());
    }
    if let Some(id) = bundle_id.as_deref() {
        let hard = HARD_BLOCKED.iter().find(|(blocked, _)| *blocked == id).map(|(_, kind)| *kind).or_else(|| {
            HARD_BLOCKED_PREFIXES.iter().find(|(prefix, _)| id.starts_with(prefix)).map(|(_, kind)| *kind)
        });
        if let Some(kind) = hard {
            return Some(format!("{} is a {kind}, which computer use never controls.", app.name));
        }
        if never_allow.iter().any(|entry| entry.eq_ignore_ascii_case(id)) {
            return Some(format!("The user never allows computer use in {}.", app.name));
        }
    }
    None
}

/// Checks and normalizes the Settings never-allow list: trimmed bundle ids, deduplicated
/// case-insensitively, in the order given.
pub fn validate_never_allow(entries: &[String]) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for entry in entries {
        let id = entry.trim();
        if id.is_empty() {
            continue;
        }
        if id.len() > 255 || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_')) {
            return Err(format!("“{id}” isn't a bundle id. Use letters, digits, dots, dashes and underscores, like com.example.App."));
        }
        if seen.insert(id.to_ascii_lowercase()) {
            result.push(id.to_string());
        }
    }
    if result.len() > MAX_NEVER_ALLOW {
        return Err(format!("The never-allow list can hold up to {MAX_NEVER_ALLOW} apps."));
    }
    Ok(result)
}

/// What a grant is keyed by: the bundle id **and** the canonical path, so an app at another
/// path that claims the same bundle id never inherits the grant. Apps without a bundle id are
/// keyed by path alone.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AppKey {
    pub bundle_id: Option<String>,
    pub path: Option<String>,
}

impl AppKey {
    pub fn of(app: &AppIdentity) -> Self {
        Self {
            bundle_id: app.bundle_id.as_deref().map(str::to_ascii_lowercase),
            path: app.path.as_deref().map(|path| path.to_string_lossy().into_owned()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    Allowed,
    Denied,
    Unknown,
}

#[derive(Debug, Default)]
struct ChatGrants {
    allowed: HashSet<AppKey>,
    denied: HashSet<AppKey>,
}

/// Per-chat grants and denials, in memory only.
#[derive(Debug, Default)]
pub struct GrantStore {
    chats: HashMap<String, ChatGrants>,
}

impl GrantStore {
    pub fn decision(&self, task_id: &str, key: &AppKey) -> Grant {
        match self.chats.get(task_id) {
            Some(chat) if chat.denied.contains(key) => Grant::Denied,
            Some(chat) if chat.allowed.contains(key) => Grant::Allowed,
            _ => Grant::Unknown,
        }
    }

    pub fn allow(&mut self, task_id: &str, key: AppKey) {
        let chat = self.chats.entry(task_id.to_string()).or_default();
        chat.denied.remove(&key);
        chat.allowed.insert(key);
    }

    pub fn deny(&mut self, task_id: &str, key: AppKey) {
        let chat = self.chats.entry(task_id.to_string()).or_default();
        chat.allowed.remove(&key);
        chat.denied.insert(key);
    }

    pub fn allowed(&self, task_id: &str) -> Vec<AppKey> {
        self.chats.get(task_id).map(|chat| chat.allowed.iter().cloned().collect()).unwrap_or_default()
    }

    pub fn denied(&self, task_id: &str) -> Vec<AppKey> {
        self.chats.get(task_id).map(|chat| chat.denied.iter().cloned().collect()).unwrap_or_default()
    }

    pub fn forget_chat(&mut self, task_id: &str) {
        self.chats.remove(task_id);
    }

    pub fn clear(&mut self) {
        self.chats.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own() -> OwnIdentity {
        OwnIdentity {
            pid: 100,
            executable: Some(PathBuf::from("/Applications/WackCode.app/Contents/MacOS/wackcode")),
            bundle: Some(PathBuf::from("/Applications/WackCode.app")),
        }
    }

    fn app(pid: i32, bundle_id: Option<&str>, path: &str) -> AppIdentity {
        AppIdentity { pid: Some(pid), name: "Some App".into(), bundle_id: bundle_id.map(str::to_string), path: Some(PathBuf::from(path)) }
    }

    #[test]
    fn wackcode_is_blocked_by_pid_bundle_id_and_path() {
        assert!(block_reason(&app(100, Some("com.example.app"), "/tmp/X.app"), &own(), &[]).is_some());
        assert!(block_reason(&app(5, Some("com.wackcode.desktop"), "/tmp/X.app"), &own(), &[]).is_some());
        assert!(block_reason(&app(5, Some("COM.WackCode.Foo"), "/tmp/X.app"), &own(), &[]).is_some());
        assert!(block_reason(&app(5, None, "/Applications/WackCode.app"), &own(), &[]).is_some());
        assert!(block_reason(&app(5, None, "/Applications/WackCode.app/Contents/MacOS/wackcode"), &own(), &[]).is_some());
        assert!(block_reason(&app(5, Some("com.example.app"), "/tmp/X.app"), &own(), &[]).is_none());
    }

    #[test]
    fn hard_list_and_prefixes_are_case_insensitive() {
        let reason = block_reason(&app(5, Some("com.apple.Terminal"), "/System/Applications/Utilities/Terminal.app"), &own(), &[]).unwrap();
        assert!(reason.contains("terminal"));
        assert!(block_reason(&app(5, Some("com.1password.1password"), "/x.app"), &own(), &[]).unwrap().contains("password manager"));
        assert!(block_reason(&app(5, Some("com.apple.systempreferences"), "/x.app"), &own(), &[]).is_some());
        assert!(block_reason(&app(5, Some("com.apple.Settings.PrivacySecurity.extension"), "/x.app"), &own(), &[]).is_some());
        assert!(block_reason(&app(5, Some("com.apple.TextEdit"), "/x.app"), &own(), &[]).is_none());
    }

    #[test]
    fn never_allow_list_blocks_by_bundle_id() {
        let never = vec!["com.example.Secret".to_string()];
        assert!(block_reason(&app(5, Some("com.example.secret"), "/x.app"), &own(), &never).unwrap().contains("never allows"));
        assert!(block_reason(&app(5, Some("com.example.other"), "/x.app"), &own(), &never).is_none());
    }

    #[test]
    fn never_allow_validation_trims_dedupes_and_rejects_junk() {
        let input = vec![" com.example.A ".to_string(), "COM.EXAMPLE.a".to_string(), String::new(), "com.example.b".to_string()];
        assert_eq!(validate_never_allow(&input).unwrap(), vec!["com.example.A", "com.example.b"]);
        assert!(validate_never_allow(&["com example".to_string()]).is_err());
        assert!(validate_never_allow(&["/Applications/X.app".to_string()]).is_err());
        let many = (0..=MAX_NEVER_ALLOW).map(|index| format!("com.example.a{index}")).collect::<Vec<_>>();
        assert!(validate_never_allow(&many).is_err());
    }

    #[test]
    fn grants_are_per_chat_and_keyed_by_bundle_id_and_path() {
        let mut store = GrantStore::default();
        let real = AppKey::of(&app(5, Some("com.example.App"), "/Applications/App.app"));
        let spoof = AppKey::of(&app(6, Some("com.example.app"), "/tmp/Evil/App.app"));
        store.allow("chat", real.clone());
        assert_eq!(store.decision("chat", &real), Grant::Allowed);
        assert_eq!(store.decision("chat", &spoof), Grant::Unknown);
        assert_eq!(store.decision("other", &real), Grant::Unknown);
        store.deny("chat", real.clone());
        assert_eq!(store.decision("chat", &real), Grant::Denied);
        store.allow("chat", real.clone());
        assert_eq!(store.decision("chat", &real), Grant::Allowed);
        store.forget_chat("chat");
        assert_eq!(store.decision("chat", &real), Grant::Unknown);
    }

    #[test]
    fn enclosing_bundle_finds_the_app_directory() {
        assert_eq!(
            enclosing_bundle(Path::new("/Applications/My App.app/Contents/MacOS/my-app")),
            Some(PathBuf::from("/Applications/My App.app"))
        );
        assert_eq!(enclosing_bundle(Path::new("/usr/local/bin/tool")), None);
    }
}
