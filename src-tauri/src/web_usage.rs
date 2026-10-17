//! Settings › Web's usage meter: how many searches and fetches each provider answered this
//! month, kept only on this Mac in `web-usage.json`. It is not the TokenTrail ledger
//! (`usage.rs`, a published contract) and never leaves the machine.
//!
//! Counts live in memory and are written a few seconds after the last change (and on quit), so
//! a burst of sub-agent searches writes the file once instead of rewriting `wackcode.json` per
//! call. "Billed" follows how DeepSeek bills: an HTTP 200 with a non-empty result.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{atomic::{AtomicBool, Ordering}, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Manager};

/// Months kept; older ones are dropped on the next write.
const KEEP_MONTHS: usize = 12;
const FLUSH_DELAY: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Counters {
    #[serde(default)]
    pub searches: u64,
    #[serde(default)]
    pub fetches: u64,
    #[serde(default)]
    pub billed_searches: u64,
    #[serde(default)]
    pub billed_fetches: u64,
    #[serde(default)]
    pub failed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Search,
    Fetch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// HTTP 200 with results or page content: what a provider charges for.
    Billed,
    /// HTTP 200 with nothing in it.
    Empty,
    /// An error status or no answer at all. A cancelled request is not recorded.
    Failed,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct UsageFile {
    #[serde(default)]
    v: u32,
    /// `YYYY-MM` (local time) → provider id → counters.
    #[serde(default)]
    months: BTreeMap<String, BTreeMap<String, Counters>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsage {
    pub id: String,
    #[serde(flatten)]
    pub counters: Counters,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebUsageSummary {
    pub month: String,
    pub providers: Vec<ProviderUsage>,
}

pub struct WebUsageState {
    path: PathBuf,
    file: Mutex<UsageFile>,
    flush_scheduled: AtomicBool,
}

impl WebUsageState {
    /// A missing or unreadable file starts the meter from zero rather than blocking launch.
    pub fn load(directory: &Path) -> Self {
        let path = directory.join("web-usage.json");
        let file = fs::read_to_string(&path)
            .ok()
            .and_then(|content| serde_json::from_str::<UsageFile>(&content).ok())
            .unwrap_or_default();
        Self { path, file: Mutex::new(file), flush_scheduled: AtomicBool::new(false) }
    }

    pub fn record(&self, month: &str, provider_id: &str, op: Op, outcome: Outcome) {
        let Ok(mut file) = self.file.lock() else { return };
        let counters = file.months.entry(month.to_string()).or_default().entry(provider_id.to_string()).or_default();
        match op {
            Op::Search => counters.searches += 1,
            Op::Fetch => counters.fetches += 1,
        }
        match (op, outcome) {
            (Op::Search, Outcome::Billed) => counters.billed_searches += 1,
            (Op::Fetch, Outcome::Billed) => counters.billed_fetches += 1,
            (_, Outcome::Failed) => counters.failed += 1,
            (_, Outcome::Empty) => {}
        }
        while file.months.len() > KEEP_MONTHS {
            let Some(oldest) = file.months.keys().next().cloned() else { break };
            file.months.remove(&oldest);
        }
    }

    pub fn summary(&self, month: &str) -> WebUsageSummary {
        let providers = self
            .file
            .lock()
            .ok()
            .and_then(|file| file.months.get(month).cloned())
            .unwrap_or_default()
            .into_iter()
            .map(|(id, counters)| ProviderUsage { id, counters })
            .collect();
        WebUsageSummary { month: month.to_string(), providers }
    }

    /// Drops one provider's counters (every month), or all of them.
    pub fn reset(&self, provider_id: Option<&str>) {
        let Ok(mut file) = self.file.lock() else { return };
        match provider_id {
            Some(id) => {
                for month in file.months.values_mut() {
                    month.remove(id);
                }
                file.months.retain(|_, providers| !providers.is_empty());
            }
            None => file.months.clear(),
        }
    }

    pub fn flush(&self) -> Result<(), String> {
        let json = {
            let file = self.file.lock().map_err(|_| "Usage lock was poisoned".to_string())?;
            serde_json::to_vec_pretty(&UsageFile { v: 1, months: file.months.clone() }).map_err(|error| error.to_string())?
        };
        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, json).map_err(|error| error.to_string())?;
        fs::rename(&temporary, &self.path).map_err(|error| error.to_string())
    }
}

pub fn current_month() -> String {
    chrono::Local::now().format("%Y-%m").to_string()
}

/// Counts one answered request and schedules a write.
pub fn record(app: &AppHandle, provider_id: &str, op: Op, outcome: Outcome) {
    let state = app.state::<WebUsageState>();
    state.record(&current_month(), provider_id, op, outcome);
    schedule_flush(app);
}

pub fn schedule_flush(app: &AppHandle) {
    if app.state::<WebUsageState>().flush_scheduled.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FLUSH_DELAY).await;
        let state = app.state::<WebUsageState>();
        state.flush_scheduled.store(false, Ordering::SeqCst);
        let _ = state.flush();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_attempts_billed_and_failed_separately() {
        let directory = tempfile::tempdir().unwrap();
        let state = WebUsageState::load(directory.path());
        state.record("2026-10", "a", Op::Search, Outcome::Billed);
        state.record("2026-10", "a", Op::Search, Outcome::Empty);
        state.record("2026-10", "a", Op::Fetch, Outcome::Failed);
        state.record("2026-10", "b", Op::Fetch, Outcome::Billed);
        let summary = state.summary("2026-10");
        let a = summary.providers.iter().find(|p| p.id == "a").unwrap().counters;
        assert_eq!(a, Counters { searches: 2, fetches: 1, billed_searches: 1, billed_fetches: 0, failed: 1 });
        assert_eq!(state.summary("2026-09").providers.len(), 0);
    }

    #[test]
    fn round_trips_through_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let state = WebUsageState::load(directory.path());
        state.record("2026-10", "a", Op::Search, Outcome::Billed);
        state.flush().unwrap();
        let again = WebUsageState::load(directory.path());
        assert_eq!(again.summary("2026-10").providers[0].counters.billed_searches, 1);
    }

    #[test]
    fn keeps_only_the_last_twelve_months() {
        let directory = tempfile::tempdir().unwrap();
        let state = WebUsageState::load(directory.path());
        for month in 1..=14 {
            let key = if month <= 12 { format!("2025-{month:02}") } else { format!("2026-{:02}", month - 12) };
            state.record(&key, "a", Op::Search, Outcome::Billed);
        }
        let file = state.file.lock().unwrap();
        assert_eq!(file.months.len(), 12);
        assert!(!file.months.contains_key("2025-01"));
        assert!(file.months.contains_key("2026-02"));
    }

    #[test]
    fn reset_one_provider_or_all() {
        let directory = tempfile::tempdir().unwrap();
        let state = WebUsageState::load(directory.path());
        state.record("2026-10", "a", Op::Search, Outcome::Billed);
        state.record("2026-10", "b", Op::Search, Outcome::Billed);
        state.reset(Some("a"));
        assert_eq!(state.summary("2026-10").providers.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["b"]);
        state.reset(None);
        assert!(state.summary("2026-10").providers.is_empty());
    }

    #[test]
    fn an_unreadable_file_starts_from_zero() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("web-usage.json"), "not json").unwrap();
        assert!(WebUsageState::load(directory.path()).summary("2026-10").providers.is_empty());
    }
}
