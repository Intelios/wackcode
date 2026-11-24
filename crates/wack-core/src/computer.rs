//! Settings › Computer use, as saved: whether it is on, the apps the user always allows, and the
//! ones they never allow. The engine itself is `wack_computer`; this is what it reads.
//!
//! Invariants:
//! - Off until the user turns it on.
//! - An app is always allowed only by the user's answer to a live access card: the app calls
//!   `Store::always_allow_app` while answering one, and from nowhere else. Settings can only take
//!   an approval back.
//! - The never-allow list holds checked bundle ids (`policy::validate_never_allow`).

use serde::{Deserialize, Serialize};
use wack_computer::computer::{SavedApp, Settings};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ComputerConfig {
    pub enabled: bool,
    pub always: Vec<SavedApp>,
    pub never: Vec<String>,
}

impl ComputerConfig {
    /// What the engine reads at every check.
    pub fn settings(&self) -> Settings {
        Settings { enabled: self.enabled, always: self.always.clone(), never: self.never.clone() }
    }

    /// Adds `app` to the always-allowed apps, replacing an entry for the same bundle.
    pub(crate) fn always_allow(&mut self, app: SavedApp) -> Result<(), String> {
        let key = app.key().ok_or("That app has no fixed place on disk, so it can only be allowed for this chat.")?;
        self.always.retain(|saved| saved.key() != Some(key.clone()));
        self.always.push(app);
        Ok(())
    }

    /// Adds `bundle_id` to the never-allowed apps, and takes back any approval of it.
    pub(crate) fn never_allow(&mut self, bundle_id: &str) -> Result<(), String> {
        let mut never = self.never.clone();
        never.push(bundle_id.to_string());
        self.never = wack_computer::policy::validate_never_allow(&never)?;
        self.always.retain(|saved| !saved.bundle_id.as_deref().is_some_and(|id| id.eq_ignore_ascii_case(bundle_id)));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn textedit() -> SavedApp {
        SavedApp { name: "TextEdit".into(), bundle_id: Some("com.apple.textedit".into()), path: "/System/Applications/TextEdit.app".into() }
    }

    #[test]
    fn off_by_default_and_reads_old_files() {
        let config: ComputerConfig = serde_json::from_str("{}").unwrap();
        assert!(!config.enabled);
        assert!(config.always.is_empty() && config.never.is_empty());
    }

    #[test]
    fn allowing_twice_keeps_one_entry_and_never_allow_takes_it_back() {
        let mut config = ComputerConfig::default();
        config.always_allow(textedit()).unwrap();
        config.always_allow(textedit()).unwrap();
        assert_eq!(config.always.len(), 1);
        config.never_allow("com.apple.TextEdit").unwrap();
        assert!(config.always.is_empty());
        assert_eq!(config.never, vec!["com.apple.TextEdit"]);
        config.never_allow("com.apple.textedit").unwrap();
        assert_eq!(config.never.len(), 1, "bundle ids compare without case");
    }

    #[test]
    fn a_relative_path_is_never_saved() {
        let mut config = ComputerConfig::default();
        let app = SavedApp { path: "TextEdit.app".into(), ..textedit() };
        assert!(config.always_allow(app).is_err());
        assert!(config.never_allow("not a bundle id!").is_err());
    }
}
