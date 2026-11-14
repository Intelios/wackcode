//! API keys on disk: `secrets.json`, readable only by the user (mode 0600).
//!
//! This is the only place a key is ever stored. Nothing else may write one to disk: not
//! `state.json`, not a session file, not a log. See docs/security.md.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions, Permissions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub struct SecretStore {
    path: PathBuf,
    keys: BTreeMap<String, String>,
}

impl SecretStore {
    pub fn load(folder: &Path) -> Result<Self, String> {
        let path = folder.join("secrets.json");
        let keys = if path.exists() {
            // In case the file was copied or restored with looser permissions.
            fs::set_permissions(&path, Permissions::from_mode(0o600))
                .map_err(|error| format!("Could not protect the credentials file: {error}"))?;
            let content = fs::read_to_string(&path).map_err(|error| format!("Could not read the credentials file: {error}"))?;
            // The parser's message could quote the file, so it is not passed on.
            serde_json::from_str(&content).map_err(|_| "The credentials file is damaged.".to_string())?
        } else {
            BTreeMap::new()
        };
        Ok(Self { path, keys })
    }

    pub fn get(&self, id: &str) -> Option<&str> {
        self.keys.get(id).map(String::as_str)
    }

    pub fn set(&mut self, id: &str, value: &str) -> Result<(), String> {
        self.keys.insert(id.to_string(), value.to_string());
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        if self.keys.remove(id).is_some() {
            self.save()?;
        }
        Ok(())
    }

    /// Writes a private temporary file and moves it into place, so the file is never half
    /// written and never readable by anyone else, even for a moment.
    fn save(&self) -> Result<(), String> {
        let failed = |error: std::io::Error| format!("Could not save the credentials file: {error}");
        let temporary = self.path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(&self.keys).map_err(|_| "Could not save the credentials file.".to_string())?;
        let _ = fs::remove_file(&temporary);
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .and_then(|mut file| file.write_all(&json))
            .map_err(failed)?;
        fs::rename(&temporary, &self.path).map_err(failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_round_trip_in_a_private_file() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = SecretStore::load(folder.path()).unwrap();
        assert!(store.get("a").is_none());
        store.set("a", "sk-one").unwrap();
        store.set("b", "sk-two").unwrap();
        store.remove("a").unwrap();

        let path = folder.path().join("secrets.json");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        // A copy with looser permissions is tightened when it is next opened.
        fs::set_permissions(&path, Permissions::from_mode(0o644)).unwrap();
        let reloaded = SecretStore::load(folder.path()).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(reloaded.get("a").is_none());
        assert_eq!(reloaded.get("b"), Some("sk-two"));
    }

    #[test]
    fn a_damaged_file_is_reported_without_quoting_it() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(folder.path().join("secrets.json"), "{\"a\": \"sk-leak").unwrap();
        let error = SecretStore::load(folder.path()).err().unwrap();
        assert!(!error.contains("sk-leak"));
    }
}
