//! Settings › Web: whether the agent may read web pages. The tools themselves are `wack_web`'s;
//! this is what the store keeps and what a run is told.

use serde::{Deserialize, Serialize};

use crate::store::Store;

/// What `state.json` keeps for Settings › Web.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebConfig {
    /// Web Fetch is on unless the user turns it off, as in version 2.
    #[serde(default = "on")]
    pub fetch: bool,
}

fn on() -> bool {
    true
}

impl Default for WebConfig {
    fn default() -> Self {
        Self { fetch: true }
    }
}

impl Store {
    /// Settings › Web: whether the agent may read public web pages.
    pub fn web_fetch_enabled(&self) -> bool {
        self.web_config().fetch
    }

    pub fn set_web_fetch(&mut self, enabled: bool) -> Result<(), String> {
        if self.web_config().fetch == enabled {
            return Ok(());
        }
        self.web_config_mut().fetch = enabled;
        self.save_state()
    }

    /// The web tools a run offers.
    pub fn web_setup(&self) -> wack_web::WebSetup {
        wack_web::WebSetup { fetch: self.web_config().fetch }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_fetch_is_on_by_default_and_its_switch_persists() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        assert!(store.web_fetch_enabled());
        assert!(store.web_setup().fetch);
        store.set_web_fetch(false).unwrap();
        let store = Store::open(folder.path()).unwrap();
        assert!(!store.web_fetch_enabled());
    }

    #[test]
    fn a_settings_file_from_before_web_loads_with_fetch_on() {
        let config: WebConfig = serde_json::from_str("{}").unwrap();
        assert!(config.fetch);
    }
}
