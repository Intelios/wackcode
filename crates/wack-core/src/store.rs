//! The app's own data: where it lives and the provider connections in it.
//!
//! `state.json` describes a connection (name, address, models); its key is in `secrets.json`
//! under the connection's id and is read only to build the provider that sends requests. No
//! function here returns a key.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use wack_llm::{HttpProvider, WireFormat};

use crate::secrets::SecretStore;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    /// The model's context size in tokens, if the user gave one. It decides when chats compact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub format: WireFormat,
    pub base_url: String,
    pub models: Vec<Model>,
}

pub struct NewConnection {
    pub name: String,
    pub format: WireFormat,
    pub base_url: String,
    pub models: Vec<Model>,
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    connections: Vec<Connection>,
}

pub struct Store {
    folder: PathBuf,
    state: State,
    secrets: SecretStore,
}

impl Store {
    /// Version 3's folder inside the app's data folder. A debug build uses the dev app's folder,
    /// so nothing under development can touch the data of the installed app.
    pub fn default_folder() -> Result<PathBuf, String> {
        let home = std::env::var_os("HOME").ok_or("Could not find your home folder.")?;
        let app = if cfg!(debug_assertions) { "com.wackcode.desktop.dev" } else { "com.wackcode.desktop" };
        Ok(Path::new(&home).join("Library/Application Support").join(app).join("v3"))
    }

    pub fn open(folder: &Path) -> Result<Self, String> {
        fs::create_dir_all(folder).map_err(|error| format!("Could not create WackCode's data folder: {error}"))?;
        let _ = fs::set_permissions(folder, fs::Permissions::from_mode(0o700));
        let path = folder.join("state.json");
        let state = if path.exists() {
            let content = fs::read_to_string(&path).map_err(|error| format!("Could not read WackCode's settings: {error}"))?;
            serde_json::from_str(&content).map_err(|error| format!("WackCode's settings file is damaged: {error}"))?
        } else {
            State::default()
        };
        Ok(Self { folder: folder.to_path_buf(), state, secrets: SecretStore::load(folder)? })
    }

    fn save(&self) -> Result<(), String> {
        let failed = |error: std::io::Error| format!("Could not save WackCode's settings: {error}");
        let path = self.folder.join("state.json");
        let temporary = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(&self.state).map_err(|error| format!("Could not save WackCode's settings: {error}"))?;
        fs::write(&temporary, json).map_err(failed)?;
        fs::rename(&temporary, &path).map_err(failed)
    }

    pub fn sessions_folder(&self) -> PathBuf {
        self.folder.join("sessions")
    }

    pub fn connections(&self) -> &[Connection] {
        &self.state.connections
    }

    /// A connection by its name (any letter case) or id.
    pub fn connection(&self, name: &str) -> Option<&Connection> {
        self.state.connections.iter().find(|connection| connection.id == name || connection.name.eq_ignore_ascii_case(name))
    }

    pub fn add_connection(&mut self, new: NewConnection, key: &str) -> Result<Connection, String> {
        let name = new.name.trim();
        let base_url = new.base_url.trim().trim_end_matches('/');
        let key = key.trim();
        if name.is_empty() {
            return Err("Give the connection a name.".to_string());
        }
        if self.connection(name).is_some() {
            return Err(format!("There is already a connection named {name}."));
        }
        if !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
            return Err("The address must start with https:// or http://.".to_string());
        }
        let models: Vec<Model> = new
            .models
            .into_iter()
            .map(|model| Model { id: model.id.trim().to_string(), ..model })
            .filter(|model| !model.id.is_empty())
            .collect();
        if models.is_empty() {
            return Err("Add at least one model.".to_string());
        }
        if key.is_empty() {
            return Err("Enter the API key.".to_string());
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or(0);
        let connection = Connection {
            id: format!("connection-{stamp:x}"),
            name: name.to_string(),
            format: new.format,
            base_url: base_url.to_string(),
            models,
        };
        // The key first: a connection listed without its key would fail on first use.
        self.secrets.set(&connection.id, key)?;
        self.state.connections.push(connection.clone());
        self.save()?;
        Ok(connection)
    }

    pub fn remove_connection(&mut self, id: &str) -> Result<(), String> {
        self.state.connections.retain(|connection| connection.id != id);
        self.save()?;
        self.secrets.remove(id)
    }

    /// The provider that sends this connection's requests. This is the one place a key leaves
    /// the secret store, and it goes straight into the provider that needs it.
    pub fn provider(&self, connection: &Connection) -> Result<HttpProvider, String> {
        let key = self.secrets.get(&connection.id).ok_or_else(|| format!("The connection {} has no API key. Add it again.", connection.name))?;
        HttpProvider::new(connection.format, &connection.base_url, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(name: &str) -> NewConnection {
        NewConnection {
            name: name.into(),
            format: WireFormat::OpenaiChat,
            base_url: "https://llm.example/v1/".into(),
            models: vec![Model { id: " big-model ".into(), context_window: Some(200_000) }, Model { id: "".into(), context_window: None }],
        }
    }

    #[test]
    fn a_connection_is_saved_with_its_key_kept_apart() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        let added = store.add_connection(new("Testing"), " sk-secret-value \n").unwrap();
        assert_eq!(added.base_url, "https://llm.example/v1");
        assert_eq!(added.models, [Model { id: "big-model".into(), context_window: Some(200_000) }]);

        let state = fs::read_to_string(folder.path().join("state.json")).unwrap();
        assert!(state.contains("Testing") && state.contains("openai-chat"));
        assert!(!state.contains("sk-secret-value"), "the key must never reach state.json");

        let reopened = Store::open(folder.path()).unwrap();
        let found = reopened.connection("testing").unwrap();
        assert_eq!(*found, added);
        assert!(reopened.provider(found).is_ok());
    }

    #[test]
    fn bad_connections_are_refused_and_removal_forgets_the_key() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        assert!(store.add_connection(new("Testing"), "").is_err());
        assert!(store.add_connection(NewConnection { base_url: "llm.example".into(), ..new("A") }, "k").is_err());
        assert!(store.add_connection(NewConnection { models: vec![], ..new("B") }, "k").is_err());
        assert!(store.connections().is_empty());

        let added = store.add_connection(new("Testing"), "sk-secret-value").unwrap();
        assert!(store.add_connection(new("TESTING"), "k").unwrap_err().contains("already"));
        store.remove_connection(&added.id).unwrap();
        assert!(store.connections().is_empty());
        assert!(!fs::read_to_string(folder.path().join("secrets.json")).unwrap().contains("sk-secret-value"));
    }
}
