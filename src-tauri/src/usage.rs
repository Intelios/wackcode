//! Usage-only ledger. A writer UUID isolates simultaneous app processes. Chat deletion
//! never touches this directory; TokenTrail only reads it.
use crate::storage::MetadataState;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager, State};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageRecord {
    pub v: u32,
    pub id: String,
    pub ts: i64,
    pub duration_ms: u64,
    pub provider: String,
    pub model: String,
    pub purpose: String,
    pub subagent_id: Option<String>,
    pub outcome: String,
    pub tokens: Option<Tokens>,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
}
impl UsageRecord {
    fn validate(&self) -> bool {
        self.v == 1
            && uuid::Uuid::parse_str(&self.id).is_ok()
            && self.ts > 0
            && chrono::DateTime::from_timestamp_millis(self.ts).is_some()
            && self.duration_ms <= i64::MAX as u64
            && [&self.provider, &self.model]
                .iter()
                .all(|s| !s.is_empty() && s.len() <= 1024)
            && self.subagent_id.as_ref().is_none_or(|s| s.len() <= 256)
            && [
                "chat",
                "subagent",
                "title",
                "compaction",
                "branch_summary",
                "goal_verification",
                "commit_message",
            ]
            .contains(&self.purpose.as_str())
            && ["completed", "failed", "cancelled"].contains(&self.outcome.as_str())
            && self.tokens.as_ref().is_none_or(|t| {
                [t.input, t.output, t.cache_read, t.cache_write]
                    .iter()
                    .all(|n| *n <= 9_007_199_254_740_991)
            })
    }
}
#[derive(Default)]
struct Writer {
    queue: VecDeque<UsageRecord>,
    error: Option<String>,
    dropped: u64,
    last_written: Option<i64>,
}
pub struct UsageState {
    writer_id: String,
    writer: Mutex<Writer>,
}
impl Default for UsageState {
    fn default() -> Self {
        Self {
            writer_id: uuid::Uuid::new_v4().to_string(),
            writer: Mutex::new(Writer::default()),
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStatus {
    enabled: bool,
    path: String,
    has_history: bool,
    error: Option<String>,
    pending: usize,
    dropped: u64,
    last_written: Option<i64>,
}
fn root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|p| p.join("usage/v1"))
        .map_err(|_| "The usage folder is unavailable.".into())
}
fn append(root: &Path, writer: &str, record: &UsageRecord) -> Result<(), String> {
    let month = chrono::DateTime::from_timestamp_millis(record.ts)
        .ok_or("Invalid usage timestamp.")?
        .format("%Y-%m")
        .to_string();
    let dir = root.join(month);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(|_| "Could not create the usage folder.")?;
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(dir.join(format!("{writer}.jsonl")))
        .map_err(|_| "Could not open the usage ledger.")?;
    // Repair only our own unfinished tail after a failed write; readers wait for a newline.
    let len = file
        .metadata()
        .map_err(|_| "Could not inspect the usage ledger.")?
        .len();
    let start = len.saturating_sub(65_536);
    file.seek(SeekFrom::Start(start))
        .map_err(|_| "Could not seek the usage ledger.")?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)
        .map_err(|_| "Could not read the usage ledger tail.")?;
    if tail.last().is_some_and(|b| *b != b'\n') {
        let end = tail
            .iter()
            .rposition(|b| *b == b'\n')
            .map(|i| start + i as u64 + 1)
            .unwrap_or(start);
        file.set_len(end)
            .map_err(|_| "Could not repair the usage ledger tail.")?;
    }
    file.seek(SeekFrom::End(0))
        .map_err(|_| "Could not seek the usage ledger.")?;
    let mut bytes = serde_json::to_vec(record).map_err(|_| "Could not encode usage.")?;
    bytes.push(b'\n');
    file.write_all(&bytes)
        .and_then(|_| file.sync_data())
        .map_err(|_| "Could not save usage. Recording will retry.".to_string())
}
impl UsageState {
    fn flush(&self, root: &Path, writer: &mut Writer) {
        while let Some(record) = writer.queue.front() {
            match append(root, &self.writer_id, record) {
                Ok(()) => {
                    writer.last_written = Some(record.ts);
                    writer.queue.pop_front();
                    writer.error = None;
                }
                Err(error) => {
                    writer.error = Some(error);
                    break;
                }
            }
        }
    }
}
pub fn start_retry(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let app = app.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                let metadata = app.state::<MetadataState>();
                let Ok(data) = metadata.data.lock() else {
                    return;
                };
                if !data.record_usage {
                    return;
                }
                let state = app.state::<UsageState>();
                let Ok(mut writer) = state.writer.lock() else {
                    return;
                };
                if !writer.queue.is_empty() {
                    if let Ok(path) = root(&app) {
                        state.flush(&path, &mut writer);
                    }
                }
            })
            .await;
        }
    });
}
pub fn record(app: &AppHandle, task_id: &str, value: serde_json::Value) {
    let Ok(mut record) = serde_json::from_value::<UsageRecord>(value) else {
        return;
    };
    if !record.validate() {
        return;
    }
    let metadata = app.state::<MetadataState>();
    let Ok(data) = metadata.data.lock() else {
        return;
    };
    if !data.record_usage {
        return;
    }
    let Some(task) = data.tasks.iter().find(|t| t.id == task_id) else {
        return;
    };
    record.session_id = task.id.clone();
    record.workspace = Some(task.workspace_path.clone());
    record.project = task
        .project_id
        .as_ref()
        .and_then(|id| data.projects.iter().find(|p| &p.id == id))
        .map(|p| p.git_root.clone().unwrap_or_else(|| p.path.clone()));
    let state = app.state::<UsageState>();
    let Ok(mut writer) = state.writer.lock() else {
        return;
    };
    // Lock order is metadata then writer, also used by the toggle and status commands.
    if writer.queue.len() == 1024 {
        writer.queue.pop_front();
        writer.dropped += 1;
    }
    writer.queue.push_back(record);
    match root(app) {
        Ok(path) => state.flush(&path, &mut writer),
        Err(e) => writer.error = Some(e),
    }
}
#[tauri::command]
pub fn usage_status(
    app: AppHandle,
    metadata: State<'_, MetadataState>,
    state: State<'_, UsageState>,
) -> Result<UsageStatus, String> {
    let data = metadata
        .data
        .lock()
        .map_err(|_| "Could not read usage settings.")?;
    let mut writer = state
        .writer
        .lock()
        .map_err(|_| "Could not read usage status.")?;
    let path = root(&app)?;
    if data.record_usage {
        state.flush(&path, &mut writer);
    }
    Ok(UsageStatus {
        enabled: data.record_usage,
        path: path.display().to_string(),
        has_history: path.is_dir(),
        error: writer.error.clone(),
        pending: writer.queue.len(),
        dropped: writer.dropped,
        last_written: writer.last_written,
    })
}
#[tauri::command]
pub fn set_usage_recording(
    metadata: State<'_, MetadataState>,
    state: State<'_, UsageState>,
    enabled: bool,
) -> Result<bool, String> {
    metadata.mutate(|data| {
        data.record_usage = enabled;
        if !enabled {
            let mut writer = state
                .writer
                .lock()
                .map_err(|_| "Could not update usage recording.")?;
            writer.dropped += writer.queue.len() as u64;
            writer.queue.clear();
            writer.error = None;
        }
        Ok(enabled)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    const FIXTURE: &str = include_str!("../fixtures/wackcode_usage_v1.jsonl");
    fn records() -> Vec<UsageRecord> {
        FIXTURE
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
    #[test]
    fn writer_produces_contract_fixture_and_private_files() {
        let temp = tempfile::tempdir().unwrap();
        for record in records() {
            assert!(record.validate());
            append(temp.path(), "writer", &record).unwrap();
        }
        let month = chrono::DateTime::from_timestamp_millis(records()[0].ts)
            .unwrap()
            .format("%Y-%m")
            .to_string();
        let file = temp.path().join(month).join("writer.jsonl");
        assert_eq!(fs::read_to_string(&file).unwrap(), FIXTURE);
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(file.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    #[test]
    fn repairs_partial_tail_and_keeps_writers_and_months_separate() {
        let temp = tempfile::tempdir().unwrap();
        let mut record = records().remove(0);
        append(temp.path(), "a", &record).unwrap();
        let month = chrono::DateTime::from_timestamp_millis(record.ts)
            .unwrap()
            .format("%Y-%m")
            .to_string();
        let path = temp.path().join(&month).join("a.jsonl");
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{broken")
            .unwrap();
        append(temp.path(), "a", &record).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 2);
        append(temp.path(), "b", &record).unwrap();
        assert!(temp.path().join(month).join("b.jsonl").exists());
        record.ts += 40 * 86_400_000;
        append(temp.path(), "a", &record).unwrap();
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    }
    #[test]
    fn failed_write_retains_queue_and_recovers_without_new_usage() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("ledger");
        fs::write(&root, b"blocked").unwrap();
        let state = UsageState::default();
        let mut writer = Writer::default();
        writer.queue.push_back(records().remove(0));
        state.flush(&root, &mut writer);
        assert!(writer.error.is_some());
        assert_eq!(writer.queue.len(), 1);
        fs::remove_file(&root).unwrap();
        state.flush(&root, &mut writer);
        assert!(writer.error.is_none());
        assert!(writer.queue.is_empty());
    }
    #[test]
    fn rejects_unexpected_payload_and_old_settings_enable_recording() {
        let mut value = serde_json::to_value(&records()[0]).unwrap();
        value["prompt"] = "private".into();
        assert!(serde_json::from_value::<UsageRecord>(value).is_err());
        let old = serde_json::json!({ "version": 1 });
        assert!(
            serde_json::from_value::<crate::models::AppData>(old)
                .unwrap()
                .record_usage
        );
    }
}
