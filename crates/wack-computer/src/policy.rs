//! Which apps computer use may touch, and how far. Pure logic, so every rule is unit-tested;
//! ported from version 2's `wackdev/src/policy.rs`.
//!
//! Checked in order on every request (docs/security.md › Computer use):
//! 1. **WackCode**, every build: any bundle id under `com.wackcode.`, this process's pid, and its
//!    own executable or bundle path. The access card lives in this app's window, so this is what
//!    keeps the agent from ever answering it. Unlike version 2, the dev app and the installed app
//!    may not drive each other either: the installed app holds the owner's real data and its
//!    Settings › Providers eye shows a key (the owner's call, 2026-11-24). The automation socket
//!    covers testing the app.
//! 2. **The hard list**: apps that run arbitrary commands (terminals, script runners), install
//!    software, hold secrets (password managers, Keychain Access) or are the system's own security
//!    and chrome UI, and System Settings.
//! 3. **The user's never-allow list** (Settings › Computer use).
//!
//! Past those an app has a [`Tier`]: browsers are look-only (the agent reads them; it acts on the
//! web through WackCode's own browser, whose address policy holds), everything else is full.
//! An app still needs the user's grant, which only the access card gives (`computer.rs`).

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

pub const MAX_NEVER_ALLOW: usize = 200;

/// Every WackCode build's bundle id starts with this.
pub const WACKCODE_PREFIX: &str = "com.wackcode.";

/// Bundle ids refused outright, with what they are. Matched case-insensitively.
pub const HARD_BLOCKED: &[(&str, &str)] = &[
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
pub const HARD_BLOCKED_PREFIXES: &[(&str, &str)] = &[("com.apple.systempreferences", "System Settings"), ("com.apple.settings", "System Settings")];

/// Web browsers, by bundle id or bundle-id prefix (a family's betas and canaries share one).
/// Matched case-insensitively.
pub const BROWSERS: &[&str] = &[
    "com.apple.safari", // Safari and Safari Technology Preview
    "com.google.chrome",
    "org.chromium.chromium",
    "org.mozilla.",
    "com.microsoft.edgemac",
    "com.brave.browser",
    "company.thebrowser.",
    "com.operasoftware.",
    "com.vivaldi.vivaldi",
    "com.kagi.kagimacos",
    "app.zen-browser.zen",
    "com.duckduckgo.macos.browser",
    "net.imput.helium",
    "org.torproject.torbrowser",
    "net.waterfox.waterfox",
    "io.gitlab.librewolf-community",
    "one.ablaze.floorp",
    "ai.perplexity.comet",
    "com.openai.atlas",
    "com.sigmaos.sigmaos",
    "com.pushplaylabs.sidekick",
    "org.webkit.minibrowser",
];

/// How far computer use may go in an app it may touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Look and act.
    Full,
    /// Look only: windows, pictures and outlines, never input. Web browsers.
    LookOnly,
}

impl Tier {
    pub fn word(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::LookOnly => "look-only",
        }
    }
}

/// A target app as the engine resolved it: `path` is the canonical `.app` bundle path, or the
/// executable for apps without a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIdentity {
    pub pid: Option<i32>,
    pub name: String,
    pub bundle_id: Option<String>,
    pub path: Option<PathBuf>,
}

/// This WackCode's own identity, taken once at startup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

fn is_browser(id: &str) -> bool {
    BROWSERS.iter().any(|entry| if entry.ends_with('.') { id.starts_with(entry) } else { id == *entry || id.starts_with(&format!("{entry}.")) || id.starts_with(&format!("{entry}technologypreview")) })
}

/// Why `app` can never be used, as a sentence for the model; `None` when a grant may allow it.
pub fn block_reason(app: &AppIdentity, own: &OwnIdentity, never_allow: &[String]) -> Option<String> {
    let bundle_id = app.bundle_id.as_deref().map(str::to_ascii_lowercase);
    let is_wackcode = app.pid == Some(own.pid)
        || bundle_id.as_deref().is_some_and(|id| id.starts_with(WACKCODE_PREFIX))
        || app.path.as_ref().is_some_and(|path| Some(path) == own.executable.as_ref() || Some(path) == own.bundle.as_ref());
    if is_wackcode {
        return Some("WackCode can't control or look at any WackCode window.".into());
    }
    if let Some(id) = bundle_id.as_deref() {
        let hard = HARD_BLOCKED
            .iter()
            .find(|(blocked, _)| *blocked == id)
            .map(|(_, kind)| *kind)
            .or_else(|| HARD_BLOCKED_PREFIXES.iter().find(|(prefix, _)| id.starts_with(prefix)).map(|(_, kind)| *kind));
        if let Some(kind) = hard {
            let article = if kind.starts_with(['a', 'e', 'i', 'o', 'u', 'A', 'E', 'I', 'O', 'U']) { "an" } else { "a" };
            return Some(if kind == "System Settings" {
                format!("{} is System Settings, which computer use never controls.", app.name)
            } else {
                format!("{} is {article} {kind}, which computer use never controls.", app.name)
            });
        }
        if never_allow.iter().any(|entry| entry.eq_ignore_ascii_case(id)) {
            return Some(format!("The user never allows computer use in {}.", app.name));
        }
    }
    None
}

/// The tier an app gets once nothing blocks it.
pub fn tier(app: &AppIdentity) -> Tier {
    match app.bundle_id.as_deref().map(str::to_ascii_lowercase) {
        Some(id) if is_browser(&id) => Tier::LookOnly,
        _ => Tier::Full,
    }
}

/// [`block_reason`], then [`tier`]: `Err` is the sentence the model gets.
pub fn assess(app: &AppIdentity, own: &OwnIdentity, never_allow: &[String]) -> Result<Tier, String> {
    match block_reason(app, own, never_allow) {
        Some(reason) => Err(reason),
        None => Ok(tier(app)),
    }
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

/// What a grant is keyed by: the bundle id **and** the canonical path, so an app at another path
/// that claims the same bundle id never inherits the grant. Apps without a bundle id are keyed by
/// path alone.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct AppKey {
    pub bundle_id: Option<String>,
    pub path: Option<String>,
}

impl AppKey {
    /// A saved identity must have an absolute, plain path; bundle-id-only grants are never valid.
    /// The engine supplies canonical paths and never re-canonicalizes saved ones at use time (a
    /// changed symlink must not carry a grant to another location).
    pub fn persistent(bundle_id: Option<&str>, path: &str) -> Option<Self> {
        let absolute = Path::new(path);
        if !absolute.is_absolute() || absolute.components().any(|part| matches!(part, Component::CurDir | Component::ParentDir)) {
            return None;
        }
        Some(Self { bundle_id: bundle_id.map(str::to_ascii_lowercase), path: Some(path.to_string()) })
    }

    pub fn of(app: &AppIdentity) -> Self {
        Self { bundle_id: app.bundle_id.as_deref().map(str::to_ascii_lowercase), path: app.path.as_deref().map(|path| path.to_string_lossy().into_owned()) }
    }

    /// The key as a saved approval, when the app has a usable path.
    pub fn as_persistent(&self) -> Option<Self> {
        self.path.as_deref().and_then(|path| Self::persistent(self.bundle_id.as_deref(), path))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    Allowed,
    Denied,
    Unknown,
}

/// One chat's grants and denials, in memory only. Denials last until the user's next message
/// (`clear_denials`); allows until the app quits or the user revokes them.
#[derive(Debug, Default, Clone)]
pub struct ChatGrants {
    allowed: HashSet<AppKey>,
    denied: HashSet<AppKey>,
}

impl ChatGrants {
    /// The access decision. Blocks always win, then this chat's denial, then its allow or an
    /// exact saved approval. Names and pids never confer saved access, and a saved entry never
    /// matches an app without a path.
    pub fn decide(&self, app: &AppIdentity, own: &OwnIdentity, never_allow: &[String], always: &[AppKey]) -> Result<(Tier, Grant), String> {
        let tier = assess(app, own, never_allow)?;
        let key = AppKey::of(app);
        let grant = if self.denied.contains(&key) {
            Grant::Denied
        } else if self.allowed.contains(&key) || key.as_persistent().is_some_and(|key| always.contains(&key)) {
            Grant::Allowed
        } else {
            Grant::Unknown
        };
        Ok((tier, grant))
    }

    pub fn allow(&mut self, key: AppKey) {
        self.denied.remove(&key);
        self.allowed.insert(key);
    }

    pub fn deny(&mut self, key: AppKey) {
        self.allowed.remove(&key);
        self.denied.insert(key);
    }

    /// A message from the user lifts this chat's denials: they may now say "go ahead".
    pub fn clear_denials(&mut self) {
        self.denied.clear();
    }

    /// Revoking takes back this chat's allow for exactly that app; denials stay.
    pub fn revoke(&mut self, key: &AppKey) {
        self.allowed.remove(key);
    }

    pub fn revoke_bundle_id(&mut self, bundle_id: &str) {
        self.allowed.retain(|key| !key.bundle_id.as_deref().is_some_and(|id| id.eq_ignore_ascii_case(bundle_id)));
    }

    pub fn allowed(&self) -> impl Iterator<Item = &AppKey> {
        self.allowed.iter()
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
    fn every_wackcode_build_is_blocked_by_pid_bundle_id_and_path() {
        assert!(block_reason(&app(100, Some("com.example.app"), "/tmp/X.app"), &own(), &[]).is_some());
        assert!(block_reason(&app(5, Some("com.wackcode.desktop"), "/tmp/X.app"), &own(), &[]).is_some());
        assert!(block_reason(&app(5, Some("COM.WackCode.Desktop"), "/tmp/X.app"), &own(), &[]).is_some());
        // The dev app and the installed app never drive each other.
        assert!(block_reason(&app(5, Some("com.wackcode.desktop.dev"), "/repo/target/debug/WackCode Dev.app"), &own(), &[]).is_some());
        assert!(block_reason(&app(5, Some("com.wackcode.desktop.helper.renderer"), "/x.app"), &own(), &[]).is_some());
        assert!(block_reason(&app(5, None, "/Applications/WackCode.app"), &own(), &[]).is_some());
        assert!(block_reason(&app(5, None, "/Applications/WackCode.app/Contents/MacOS/wackcode"), &own(), &[]).is_some());
        assert!(block_reason(&app(5, Some("com.example.app"), "/tmp/X.app"), &own(), &[]).is_none());
        assert!(block_reason(&app(5, Some("com.wackcodex.app"), "/tmp/X.app"), &own(), &[]).is_none());
    }

    #[test]
    fn hard_list_and_prefixes_are_case_insensitive() {
        let reason = block_reason(&app(5, Some("com.apple.Terminal"), "/System/Applications/Utilities/Terminal.app"), &own(), &[]).unwrap();
        assert!(reason.contains("a terminal"));
        assert!(block_reason(&app(5, Some("com.1password.1password"), "/x.app"), &own(), &[]).unwrap().contains("password manager"));
        assert!(block_reason(&app(5, Some("com.apple.installer"), "/x.app"), &own(), &[]).unwrap().contains("an installer"));
        assert!(block_reason(&app(5, Some("com.apple.systempreferences"), "/x.app"), &own(), &[]).unwrap().contains("System Settings"));
        assert!(block_reason(&app(5, Some("com.apple.Settings.PrivacySecurity.extension"), "/x.app"), &own(), &[]).is_some());
        assert!(block_reason(&app(5, Some("com.apple.TextEdit"), "/x.app"), &own(), &[]).is_none());
    }

    #[test]
    fn browsers_are_look_only_and_other_apps_full() {
        for id in [
            "com.apple.Safari",
            "com.apple.SafariTechnologyPreview",
            "com.google.Chrome",
            "com.google.Chrome.canary",
            "org.mozilla.firefox",
            "company.thebrowser.Browser",
            "net.imput.helium",
            "com.microsoft.edgemac.Dev",
        ] {
            assert_eq!(assess(&app(5, Some(id), "/x.app"), &own(), &[]), Ok(Tier::LookOnly), "{id}");
        }
        for id in ["com.apple.TextEdit", "com.apple.calculator", "com.google.drivefs", "com.apple.safarifake"] {
            assert_eq!(assess(&app(5, Some(id), "/x.app"), &own(), &[]), Ok(Tier::Full), "{id}");
        }
        assert_eq!(tier(&app(5, None, "/x.app")), Tier::Full);
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
    fn grants_are_keyed_by_bundle_id_and_path() {
        let mut grants = ChatGrants::default();
        let real = app(5, Some("com.example.App"), "/Applications/App.app");
        let spoof = app(6, Some("com.example.app"), "/tmp/Evil/App.app");
        grants.allow(AppKey::of(&real));
        assert_eq!(grants.decide(&real, &own(), &[], &[]), Ok((Tier::Full, Grant::Allowed)));
        assert_eq!(grants.decide(&spoof, &own(), &[], &[]), Ok((Tier::Full, Grant::Unknown)));
        assert_eq!(ChatGrants::default().decide(&real, &own(), &[], &[]), Ok((Tier::Full, Grant::Unknown)), "another chat");
        grants.deny(AppKey::of(&real));
        assert_eq!(grants.decide(&real, &own(), &[], &[]).unwrap().1, Grant::Denied);
        grants.allow(AppKey::of(&real));
        assert_eq!(grants.decide(&real, &own(), &[], &[]).unwrap().1, Grant::Allowed);
    }

    #[test]
    fn a_denial_lasts_until_the_users_next_message() {
        let mut grants = ChatGrants::default();
        let target = app(5, Some("com.example.App"), "/Applications/App.app");
        grants.deny(AppKey::of(&target));
        assert_eq!(grants.decide(&target, &own(), &[], &[]).unwrap().1, Grant::Denied);
        grants.clear_denials();
        assert_eq!(grants.decide(&target, &own(), &[], &[]).unwrap().1, Grant::Unknown);
    }

    #[test]
    fn saved_access_is_exact_key_and_below_denials_and_blocks() {
        let mut grants = ChatGrants::default();
        let target = app(5, Some("COM.EXAMPLE.APP"), "/Applications/App.app");
        let saved = vec![AppKey::persistent(Some("com.example.App"), "/Applications/App.app").unwrap()];
        let decide = |grants: &ChatGrants, target: &AppIdentity, never: &[String]| grants.decide(target, &own(), never, &saved);
        assert_eq!(decide(&grants, &target, &[]).unwrap().1, Grant::Allowed);
        assert_eq!(decide(&grants, &app(6, Some("com.example.App"), "/tmp/App.app"), &[]).unwrap().1, Grant::Unknown);
        assert_eq!(decide(&grants, &app(6, None, "/Applications/App.app"), &[]).unwrap().1, Grant::Unknown);
        grants.deny(AppKey::of(&target));
        assert_eq!(decide(&grants, &target, &[]).unwrap().1, Grant::Denied);
        assert!(decide(&grants, &target, &["com.example.app".into()]).is_err());
        // Even a forged saved or chat grant cannot get past WackCode or the hard list.
        for blocked in [app(100, Some("com.example.App"), "/Applications/App.app"), app(6, Some("com.apple.Terminal"), "/tmp/Terminal.app")] {
            let key = AppKey::of(&blocked);
            grants.allow(key.clone());
            assert!(grants.decide(&blocked, &own(), &[], &[key]).is_err());
        }
    }

    #[test]
    fn path_only_access_is_exact_and_pathless_entries_cannot_allow() {
        let grants = ChatGrants::default();
        let target = app(5, None, "/tmp/local-app");
        let saved = AppKey::persistent(None, "/tmp/local-app").unwrap();
        assert_eq!(grants.decide(&target, &own(), &[], std::slice::from_ref(&saved)).unwrap().1, Grant::Allowed);
        assert_eq!(grants.decide(&app(5, None, "/tmp/other-app"), &own(), &[], &[saved]).unwrap().1, Grant::Unknown);
        assert!(AppKey::persistent(Some("com.example.App"), "").is_none());
        assert!(AppKey::persistent(None, "relative/app").is_none());
        assert!(AppKey::persistent(None, "/tmp/../app").is_none());
        let pathless = AppIdentity { path: None, ..target };
        assert_eq!(grants.decide(&pathless, &own(), &[], &[AppKey::of(&pathless)]).unwrap().1, Grant::Unknown);
    }

    #[test]
    fn revoking_takes_back_only_the_exact_allow() {
        let mut grants = ChatGrants::default();
        let key = AppKey::of(&app(5, Some("com.example.App"), "/Applications/App.app"));
        let other = AppKey::of(&app(6, Some("com.example.App"), "/tmp/App.app"));
        let denied = AppKey::of(&app(7, Some("com.example.Denied"), "/Applications/Denied.app"));
        grants.allow(key.clone());
        grants.allow(other.clone());
        grants.deny(denied.clone());
        grants.revoke(&key);
        assert!(!grants.allowed().any(|allowed| *allowed == key));
        assert!(grants.allowed().any(|allowed| *allowed == other));
        grants.revoke_bundle_id("COM.EXAMPLE.APP");
        assert_eq!(grants.allowed().count(), 0);
        assert!(grants.denied.contains(&denied));
    }

    #[test]
    fn enclosing_bundle_finds_the_app_directory() {
        assert_eq!(enclosing_bundle(Path::new("/Applications/My App.app/Contents/MacOS/my-app")), Some(PathBuf::from("/Applications/My App.app")));
        assert_eq!(enclosing_bundle(Path::new("/usr/local/bin/tool")), None);
    }
}
