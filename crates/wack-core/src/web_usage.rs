//! Settings › Web's usage meter: how many searches and page reads each search provider answered
//! this month, kept only on this Mac in `v3/web-usage.json`. It is not the TokenTrail ledger
//! (`usage.rs`, a published contract) and never leaves the machine. Ported from version 2.
//!
//! Counts live in memory and are written a few seconds after the last change, so a burst of
//! searches writes the file once. "Billed" follows how DeepSeek bills: an HTTP 200 with a
//! non-empty result.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use wack_web::search::{Meter, Op, Outcome};

/// Months kept; older ones are dropped on the next record.
const KEEP_MONTHS: usize = 12;
const FLUSH_DELAY: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Default, Serialize, Deserialize)]
struct UsageFile {
    #[serde(default)]
    v: u32,
    /// `YYYY-MM` (local time) → provider id → counters.
    #[serde(default)]
    months: BTreeMap<String, BTreeMap<String, Counters>>,
}

pub struct WebUsage {
    path: PathBuf,
    file: Mutex<UsageFile>,
    flush_scheduled: Arc<AtomicBool>,
}

impl WebUsage {
    /// A missing or unreadable file starts the meter from zero rather than blocking launch.
    pub fn load(folder: &Path) -> Self {
        let path = folder.join("web-usage.json");
        let file = fs::read_to_string(&path).ok().and_then(|content| serde_json::from_str(&content).ok()).unwrap_or_default();
        Self { path, file: Mutex::new(file), flush_scheduled: Arc::new(AtomicBool::new(false)) }
    }

    fn count(&self, month: &str, provider: &str, op: Op, outcome: Outcome) {
        let mut file = self.file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let counters = file.months.entry(month.to_string()).or_default().entry(provider.to_string()).or_default();
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

    /// This month's counters for one provider.
    pub fn this_month(&self, provider: &str) -> Counters {
        self.month(&current_month(), provider)
    }

    fn month(&self, month: &str, provider: &str) -> Counters {
        let file = self.file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        file.months.get(month).and_then(|providers| providers.get(provider)).copied().unwrap_or_default()
    }

    /// Drops one provider's counters (every month), or all of them, and writes at once.
    pub fn reset(&self, provider: Option<&str>) -> Result<(), String> {
        {
            let mut file = self.file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            match provider {
                Some(id) => {
                    for month in file.months.values_mut() {
                        month.remove(id);
                    }
                    file.months.retain(|_, providers| !providers.is_empty());
                }
                None => file.months.clear(),
            }
        }
        self.flush()
    }

    pub fn flush(&self) -> Result<(), String> {
        let json = {
            let file = self.file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            serde_json::to_vec_pretty(&UsageFile { v: 1, months: file.months.clone() }).map_err(|error| error.to_string())?
        };
        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, json).map_err(|error| format!("Could not save web usage: {error}"))?;
        fs::rename(&temporary, &self.path).map_err(|error| format!("Could not save web usage: {error}"))
    }
}

/// `Arc<WebUsage>` is what a search client is handed: counting schedules a write.
pub struct Recorder(pub Arc<WebUsage>);

impl Meter for Recorder {
    fn record(&self, provider: &str, op: Op, outcome: Outcome) {
        self.0.count(&current_month(), provider, op, outcome);
        if self.0.flush_scheduled.swap(true, Ordering::SeqCst) {
            return;
        }
        let usage = self.0.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(async move {
                    tokio::time::sleep(FLUSH_DELAY).await;
                    usage.flush_scheduled.store(false, Ordering::SeqCst);
                    let _ = usage.flush();
                });
            }
            Err(_) => {
                usage.flush_scheduled.store(false, Ordering::SeqCst);
                let _ = usage.flush();
            }
        }
    }
}

/// `YYYY-MM` in local time.
pub fn current_month() -> String {
    wack_agent::prompt::today()[..7].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_attempts_billed_and_failed_separately() {
        let folder = tempfile::tempdir().unwrap();
        let usage = WebUsage::load(folder.path());
        usage.count("2026-10", "a", Op::Search, Outcome::Billed);
        usage.count("2026-10", "a", Op::Search, Outcome::Empty);
        usage.count("2026-10", "a", Op::Fetch, Outcome::Failed);
        usage.count("2026-10", "b", Op::Fetch, Outcome::Billed);
        assert_eq!(usage.month("2026-10", "a"), Counters { searches: 2, fetches: 1, billed_searches: 1, billed_fetches: 0, failed: 1 });
        assert_eq!(usage.month("2026-09", "a"), Counters::default());
    }

    #[test]
    fn round_trips_through_the_file() {
        let folder = tempfile::tempdir().unwrap();
        let usage = WebUsage::load(folder.path());
        usage.count("2026-10", "a", Op::Search, Outcome::Billed);
        usage.flush().unwrap();
        assert_eq!(WebUsage::load(folder.path()).month("2026-10", "a").billed_searches, 1);
    }

    #[test]
    fn keeps_only_the_last_twelve_months() {
        let folder = tempfile::tempdir().unwrap();
        let usage = WebUsage::load(folder.path());
        for month in 1..=14 {
            let key = if month <= 12 { format!("2025-{month:02}") } else { format!("2026-{:02}", month - 12) };
            usage.count(&key, "a", Op::Search, Outcome::Billed);
        }
        let file = usage.file.lock().unwrap();
        assert_eq!(file.months.len(), 12);
        assert!(!file.months.contains_key("2025-01"));
        assert!(file.months.contains_key("2026-02"));
    }

    #[test]
    fn reset_one_provider_or_all() {
        let folder = tempfile::tempdir().unwrap();
        let usage = WebUsage::load(folder.path());
        usage.count("2026-10", "a", Op::Search, Outcome::Billed);
        usage.count("2026-10", "b", Op::Search, Outcome::Billed);
        usage.reset(Some("a")).unwrap();
        assert_eq!(usage.month("2026-10", "a"), Counters::default());
        assert_eq!(usage.month("2026-10", "b").searches, 1);
        usage.reset(None).unwrap();
        assert_eq!(usage.month("2026-10", "b"), Counters::default());
    }

    #[test]
    fn an_unreadable_file_starts_from_zero() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(folder.path().join("web-usage.json"), "not json").unwrap();
        assert_eq!(WebUsage::load(folder.path()).month("2026-10", "a"), Counters::default());
    }

    #[test]
    fn the_month_is_a_local_year_and_month() {
        let month = current_month();
        assert_eq!(month.len(), 7);
        assert_eq!(&month[4..5], "-");
    }
}
