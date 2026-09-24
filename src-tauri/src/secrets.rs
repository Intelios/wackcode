use std::{collections::BTreeMap, fs, path::{Path, PathBuf}, sync::Mutex};

pub struct SecretStore {
    path: PathBuf,
    keys: Mutex<BTreeMap<String, String>>,
}

impl SecretStore {
    pub fn load(directory: &Path) -> Result<Self, String> {
        let path = directory.join("secrets.json");
        let keys = if path.exists() {
            restrict_permissions(&path)?;
            let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
            serde_json::from_str::<BTreeMap<String, String>>(&content)
                .map_err(|error| format!("Could not read WackCode credentials: {error}"))?
        } else {
            BTreeMap::new()
        };
        Ok(Self { path, keys: Mutex::new(keys) })
    }

    /// A connection's API key. Its error is the sentence the user sees when a chat needs the key.
    pub fn get(&self, provider_id: &str) -> Result<String, String> {
        self.get_optional(provider_id)?
            .ok_or_else(|| "Add an API key for this connection in Settings".to_string())
    }

    /// Any stored value by id: a connection's API key, or an MCP server's `mcp:<id>` entry.
    pub fn get_optional(&self, id: &str) -> Result<Option<String>, String> {
        Ok(self.keys.lock().map_err(|_| "Credential lock was poisoned".to_string())?.get(id).cloned())
    }

    pub fn set(&self, id: &str, value: &str) -> Result<(), String> {
        let mut keys = self.keys.lock().map_err(|_| "Credential lock was poisoned".to_string())?;
        keys.insert(id.to_string(), value.to_string());
        self.save(&keys)
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        let mut keys = self.keys.lock().map_err(|_| "Credential lock was poisoned".to_string())?;
        if keys.remove(id).is_some() { self.save(&keys)?; }
        Ok(())
    }

    fn save(&self, keys: &BTreeMap<String, String>) -> Result<(), String> {
        let temporary = self.path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(keys).map_err(|error| error.to_string())?;
        write_private(&temporary, &json)?;
        fs::rename(&temporary, &self.path).map_err(|error| error.to_string())
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt};
    OpenOptions::new().write(true).create(true).truncate(true).mode(0o600)
        .open(path).map_err(|error| error.to_string())?
        .write_all(bytes).map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|error| error.to_string())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let store = SecretStore::load(directory.path()).unwrap();
        assert!(store.get("missing").is_err());
        store.set("a", "sk-one").unwrap();
        store.set("b", "sk-two").unwrap();
        store.remove("a").unwrap();
        let reloaded = SecretStore::load(directory.path()).unwrap();
        assert!(reloaded.get("a").is_err());
        assert_eq!(reloaded.get("b").unwrap(), "sk-two");
        assert!(fs::read_to_string(directory.path().join("secrets.json")).unwrap().contains("sk-two"));
    }

    #[cfg(unix)]
    #[test]
    fn secrets_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let store = SecretStore::load(directory.path()).unwrap();
        store.set("a", "sk-one").unwrap();
        let mode = fs::metadata(directory.path().join("secrets.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
