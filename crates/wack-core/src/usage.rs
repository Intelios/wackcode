//! The usage ledger: the published contract TokenTrail reads (`docs/wackcode-usage-v1.md`),
//! ported from version 2 (`2.5-Legacy:src-tauri/src/usage.rs`).
//!
//! Invariants:
//! - The writer owns `usage/v1/YYYY-MM/<writer-uuid>.jsonl` beside `v3`'s own data. One writer
//!   UUID per process, so two WackCodes never write the same file; TokenTrail reads complete
//!   newline-terminated records only, incrementally and read-only.
//! - Records are queued and flushed as they arrive, and the queue is retried: a record is never
//!   lost to a busy disk, only to the queue's cap, which drops the oldest and says so.
//! - Files are 0600 inside 0700 folders, and each append is synced before the next record.
//! - Chat deletion never touches this folder, and nothing before the writer exists is exported.
//! - No prompts, responses, titles, tool arguments, URLs, credentials or error bodies are
//!   exported: a record holds counts, names and timings, nothing else.

use std::{
    collections::VecDeque,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wack_agent::{ModelCall, Recorder};

use crate::store::{ChatKind, Connection};

/// The most records the queue holds; the oldest is dropped beyond that, and counted.
const MAX_QUEUE: usize = 1024;
/// The last instant a record may carry: the end of the year 9999, so a broken clock cannot
/// push a record into a folder no reader would look in.
const MAX_TS: i64 = 253_402_300_799_999;
/// A token count a JSON reader can hold exactly.
const MAX_COUNT: u64 = 9_007_199_254_740_991;

const PURPOSES: [&str; 7] =
    ["chat", "subagent", "title", "compaction", "branch_summary", "goal_verification", "commit_message"];
const OUTCOMES: [&str; 3] = ["completed", "failed", "cancelled"];
const MODES: [&str; 2] = ["code", "chat"];

/// Token counts as the contract defines them: the four categories are additive, `input` excludes
/// both cache categories, and `output` includes reasoning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

/// One record of the ledger, in the contract's shape. The field order is version 2's, so the
/// bytes this serializes to are the bytes version 2 wrote.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub v: u32,
    pub id: String,
    /// Milliseconds since 1970, at the moment the call finished.
    pub ts: i64,
    pub duration_ms: u64,
    /// The connection's stable id; `provider_name` beside it is what the user typed.
    pub provider: String,
    #[serde(default)]
    pub provider_name: Option<String>,
    pub model: String,
    pub purpose: String,
    #[serde(default)]
    pub subagent_id: Option<String>,
    pub outcome: String,
    /// `None` when the provider exposed no usable counts, never an estimate.
    #[serde(default)]
    pub tokens: Option<Tokens>,
    /// The owning WackCode chat.
    #[serde(default)]
    pub session_id: String,
    /// The project's repository root, when there is one.
    #[serde(default)]
    pub project: Option<String>,
    /// The folder the run worked in.
    #[serde(default)]
    pub workspace: Option<String>,
    /// The area that made the call: `code` or `chat` (the Chat area), as version 2 stamped it
    /// from the chat's kind. Additive and last: records written before it carry none, and
    /// readers treat an absent `mode` as `code`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

impl Record {
    /// Whether this record may enter the ledger. An invalid record is dropped, never mended.
    fn valid(&self) -> bool {
        let short = |text: &str| !text.is_empty() && text.len() <= 1024;
        self.v == 1
            && Uuid::parse_str(&self.id).is_ok()
            && self.ts > 0
            && self.ts <= MAX_TS
            && self.duration_ms <= i64::MAX as u64
            && short(&self.provider)
            && short(&self.model)
            && self.provider_name.as_ref().is_none_or(|name| short(name))
            && self.subagent_id.as_ref().is_none_or(|id| id.len() <= 256)
            && PURPOSES.contains(&self.purpose.as_str())
            && OUTCOMES.contains(&self.outcome.as_str())
            && self.mode.as_deref().is_none_or(|mode| MODES.contains(&mode))
            && self.tokens.as_ref().is_none_or(|tokens| {
                [tokens.input, tokens.output, tokens.cache_read, tokens.cache_write]
                    .iter()
                    .all(|count| *count <= MAX_COUNT)
            })
    }
}

/// The ledger's folder for a data folder: a sibling of `v3`'s own data, where the published
/// contract puts it. A folder given outright (tests, `--data-dir`) keeps its ledger inside.
pub fn root_for(folder: &Path) -> PathBuf {
    if folder.file_name().is_some_and(|name| name == "v3") {
        folder.parent().map(|data| data.join("usage/v1")).unwrap_or_else(|| folder.join("usage/v1"))
    } else {
        folder.join("usage/v1")
    }
}

/// The UTC month a timestamp falls in, as the ledger's folder name: `YYYY-MM`. Version 2
/// bucketed by UTC too, so both apps' records land in the same months.
pub fn month(ts: i64) -> String {
    let (year, month) = civil_from_days(ts.div_euclid(86_400_000));
    format!("{year:04}-{month:02}")
}

/// Days since 1970-01-01 to a year and month, in the proleptic Gregorian calendar (Howard
/// Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, month as u32)
}

fn now_millis() -> i64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|time| time.as_millis() as i64).unwrap_or(0)
}

/// The queue, its last failure and its bookkeeping. Guarded by the writer's lock.
#[derive(Default)]
struct Pending {
    queue: VecDeque<Record>,
    error: Option<String>,
    dropped: u64,
    last_written: Option<i64>,
}

/// What Settings › Integrations shows. `path` is the ledger's folder, `has_history` whether
/// anything has ever been written there.
#[derive(Clone, Debug, Default)]
pub struct UsageStatus {
    pub enabled: bool,
    pub path: String,
    pub has_history: bool,
    pub error: Option<String>,
    pub pending: usize,
    pub dropped: u64,
    pub last_written: Option<i64>,
}

/// The ledger's writer: one per process, shared by every run.
pub struct UsageWriter {
    writer_id: String,
    root: PathBuf,
    enabled: AtomicBool,
    pending: Mutex<Pending>,
}

impl UsageWriter {
    pub fn new(root: &Path, enabled: bool) -> Self {
        Self {
            writer_id: Uuid::new_v4().to_string(),
            root: root.to_path_buf(),
            enabled: AtomicBool::new(enabled),
            pending: Mutex::default(),
        }
    }

    /// Queues one record and writes what is queued. Recording is off, or the record invalid,
    /// and it is dropped on the floor.
    pub fn record(&self, record: Record) {
        if !self.enabled.load(Ordering::Relaxed) || !record.valid() {
            return;
        }
        let Ok(mut pending) = self.pending.lock() else { return };
        if pending.queue.len() == MAX_QUEUE {
            pending.queue.pop_front();
            pending.dropped += 1;
        }
        pending.queue.push_back(record);
        self.flush_locked(&mut pending);
    }

    /// Writes everything queued, stopping at the first failure.
    pub fn flush(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            self.flush_locked(&mut pending);
        }
    }

    fn flush_locked(&self, pending: &mut Pending) {
        while let Some(record) = pending.queue.front() {
            match append(&self.root, &self.writer_id, record) {
                Ok(()) => {
                    pending.last_written = Some(record.ts);
                    pending.queue.pop_front();
                    pending.error = None;
                }
                Err(error) => {
                    pending.error = Some(error);
                    break;
                }
            }
        }
    }

    /// Turns recording on, or off: off also gives up on everything still queued, counting it.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        if !enabled
            && let Ok(mut pending) = self.pending.lock()
        {
            pending.dropped += pending.queue.len() as u64;
            pending.queue.clear();
            pending.error = None;
        }
    }

    /// The page Settings shows, flushed first so it never reports records it could have written.
    pub fn status(&self) -> UsageStatus {
        let mut pending = self.pending.lock().unwrap_or_else(|poison| poison.into_inner());
        if self.enabled.load(Ordering::Relaxed) {
            self.flush_locked(&mut pending);
        }
        UsageStatus {
            enabled: self.enabled.load(Ordering::Relaxed),
            path: self.root.display().to_string(),
            has_history: self.root.is_dir(),
            error: pending.error.clone(),
            pending: pending.queue.len(),
            dropped: pending.dropped,
            last_written: pending.last_written,
        }
    }
}

/// Appends one record to this writer's file for the record's month. Only our own unfinished
/// tail is repaired after a failed write; readers wait for a newline.
fn append(root: &Path, writer: &str, record: &Record) -> Result<(), String> {
    let dir = root.join(month(record.ts));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(|_| "Could not create the usage folder.")?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join(format!("{writer}.jsonl")))
        .map_err(|_| "Could not open the usage ledger.")?;
    let len = file.metadata().map_err(|_| "Could not inspect the usage ledger.")?.len();
    let start = len.saturating_sub(65_536);
    file.seek(SeekFrom::Start(start)).map_err(|_| "Could not seek the usage ledger.")?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).map_err(|_| "Could not read the usage ledger tail.")?;
    if tail.last().is_some_and(|byte| *byte != b'\n') {
        let end = tail
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map(|at| start + at as u64 + 1)
            .unwrap_or(start);
        file.set_len(end).map_err(|_| "Could not repair the usage ledger tail.")?;
    }
    file.seek(SeekFrom::End(0)).map_err(|_| "Could not seek the usage ledger.")?;
    let mut bytes = serde_json::to_vec(record).map_err(|_| "Could not encode usage.")?;
    bytes.push(b'\n');
    file.write_all(&bytes)
        .and_then(|_| file.sync_data())
        .map_err(|_| "Could not save usage. Recording will retry.".to_string())
}

/// The ledger's recorder for one run: what every call of that run shares, and the writer they
/// go to. The loop reports each model invocation; this stamps it and queues it.
pub struct RunRecorder {
    writer: Arc<UsageWriter>,
    session: String,
    provider: String,
    provider_name: Option<String>,
    project: Option<String>,
    workspace: Option<String>,
    mode: ChatKind,
}

impl RunRecorder {
    /// `project` is `None` for a Chat chat, which has none; its `workspace` is its scratchpad.
    pub fn new(
        writer: &Arc<UsageWriter>,
        connection: &Connection,
        session: &str,
        project: Option<&Path>,
        workspace: &Path,
        mode: ChatKind,
    ) -> Self {
        Self {
            writer: writer.clone(),
            session: session.to_string(),
            provider: connection.id.clone(),
            provider_name: Some(connection.name.clone()),
            project: project.map(|project| project.display().to_string()),
            workspace: Some(workspace.display().to_string()),
            mode,
        }
    }
}

impl Recorder for RunRecorder {
    fn record(&self, call: ModelCall) {
        // Counts the provider did not expose, or exposed as nothing, are null: never estimated.
        let usage = call.usage.filter(|usage| {
            usage.input + usage.output + usage.cache_read + usage.cache_write > 0
        });
        self.writer.record(Record {
            v: 1,
            id: Uuid::new_v4().to_string(),
            ts: now_millis(),
            duration_ms: call.duration_ms,
            provider: self.provider.clone(),
            provider_name: self.provider_name.clone(),
            model: call.model,
            purpose: call.purpose.as_str().to_string(),
            subagent_id: call.subagent_id,
            outcome: call.outcome.as_str().to_string(),
            tokens: usage.map(|usage| Tokens {
                input: usage.input,
                output: usage.output,
                cache_read: usage.cache_read,
                cache_write: usage.cache_write,
            }),
            session_id: self.session.clone(),
            project: self.project.clone(),
            workspace: self.workspace.clone(),
            mode: Some(self.mode.ledger_mode().to_string()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use wack_agent::{CallOutcome, Purpose};
    use wack_llm::Usage;

    const FIXTURE: &str = include_str!("../fixtures/wackcode_usage_v1.jsonl");

    fn records() -> Vec<Record> {
        FIXTURE.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }

    #[test]
    fn the_writer_produces_the_contract_fixture_in_private_files() {
        let temp = tempfile::tempdir().unwrap();
        for record in records() {
            assert!(record.valid());
            append(temp.path(), "writer", &record).unwrap();
        }
        let month = month(records()[0].ts);
        let file = temp.path().join(month).join("writer.jsonl");
        assert_eq!(fs::read_to_string(&file).unwrap(), FIXTURE);
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(file.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn repairs_its_own_tail_and_keeps_writers_and_months_separate() {
        let temp = tempfile::tempdir().unwrap();
        let mut record = records().remove(0);
        append(temp.path(), "a", &record).unwrap();
        let month = month(record.ts);
        let path = temp.path().join(&month).join("a.jsonl");
        fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{broken").unwrap();
        append(temp.path(), "a", &record).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 2);
        append(temp.path(), "b", &record).unwrap();
        assert!(temp.path().join(&month).join("b.jsonl").exists());
        record.ts += 40 * 86_400_000;
        append(temp.path(), "a", &record).unwrap();
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    }

    #[test]
    fn a_failed_write_keeps_the_queue_and_recovers() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("ledger");
        fs::write(&root, b"blocked").unwrap();
        let writer = UsageWriter::new(&root, true);
        writer.record(records().remove(0));
        let status = writer.status();
        assert!(status.error.is_some());
        assert_eq!(status.pending, 1);
        fs::remove_file(&root).unwrap();
        writer.flush();
        assert!(writer.status().error.is_none());
        assert_eq!(writer.status().pending, 0);
    }

    #[test]
    fn turning_recording_off_gives_up_on_what_is_queued() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("ledger");
        fs::write(&root, b"blocked").unwrap();
        let writer = UsageWriter::new(&root, true);
        writer.record(records().remove(0));
        writer.record(records().remove(1));
        writer.set_enabled(false);
        let status = writer.status();
        assert!(!status.enabled);
        assert_eq!(status.pending, 0);
        assert_eq!(status.dropped, 2);
        assert!(status.error.is_none(), "off clears the failure too");
        // Records while off are not kept.
        writer.record(records().remove(2));
        assert_eq!(writer.status().pending, 0);
    }

    #[test]
    fn the_queue_drops_the_oldest_beyond_its_cap() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("ledger");
        fs::write(&root, b"blocked").unwrap();
        let writer = UsageWriter::new(&root, true);
        for index in 0..(MAX_QUEUE + 2) {
            let mut record = records().remove(0);
            record.id = Uuid::new_v4().to_string();
            record.duration_ms = index as u64;
            writer.record(record);
        }
        let status = writer.status();
        assert_eq!(status.pending, MAX_QUEUE);
        assert_eq!(status.dropped, 2);
        let pending = writer.pending.lock().unwrap();
        assert_eq!(pending.queue.front().unwrap().duration_ms, 2, "the oldest went first");
    }

    #[test]
    fn an_invalid_record_is_dropped_not_mended() {
        let temp = tempfile::tempdir().unwrap();
        let writer = UsageWriter::new(temp.path(), true);
        let fresh = || records().remove(0);

        let mut record = fresh();
        record.v = 2;
        writer.record(record);
        let mut record = fresh();
        record.id = "not-a-uuid".into();
        writer.record(record);
        let mut record = fresh();
        record.ts = 0;
        writer.record(record);
        let mut record = fresh();
        record.ts = MAX_TS + 1;
        writer.record(record);
        let mut record = fresh();
        record.duration_ms = i64::MAX as u64 + 1;
        writer.record(record);
        let mut record = fresh();
        record.provider = String::new();
        writer.record(record);
        let mut record = fresh();
        record.provider = "x".repeat(1025);
        writer.record(record);
        let mut record = fresh();
        record.model = String::new();
        writer.record(record);
        let mut record = fresh();
        record.provider_name = Some(String::new());
        writer.record(record);
        let mut record = fresh();
        record.provider_name = Some("x".repeat(1025));
        writer.record(record);
        let mut record = fresh();
        record.subagent_id = Some("x".repeat(257));
        writer.record(record);
        let mut record = fresh();
        record.purpose = "wondering".into();
        writer.record(record);
        let mut record = fresh();
        record.outcome = "interrupted".into();
        writer.record(record);
        let mut record = fresh();
        record.mode = Some("plan".into());
        writer.record(record);
        let mut record = fresh();
        record.tokens = Some(Tokens { input: MAX_COUNT + 1, output: 0, cache_read: 0, cache_write: 0 });
        writer.record(record);

        assert_eq!(writer.status().pending, 0, "none of them entered the ledger");
        assert!(writer.status().last_written.is_none());

        // The boundary values themselves are fine.
        let mut record = fresh();
        record.ts = MAX_TS;
        record.provider_name = Some("x".repeat(1024));
        writer.record(record);
        assert_eq!(writer.status().pending, 0, "it was written, not queued");
        assert_eq!(writer.status().last_written, Some(MAX_TS));
    }

    #[test]
    fn months_are_utc_and_span_the_calendar_rules() {
        assert_eq!(month(0), "1970-01");
        assert_eq!(month(1), "1970-01");
        assert_eq!(month(951_782_400_000), "2000-02", "a leap day lands in its own month");
        assert_eq!(month(1_793_491_199_999), "2026-10", "the last millisecond before November");
        assert_eq!(month(1_793_491_200_000), "2026-11");
        assert_eq!(month(4_107_542_400_000), "2100-03", "2100 is not a leap year");
        assert_eq!(month(MAX_TS), "9999-12");
    }

    #[test]
    fn the_ledger_sits_beside_v3_and_inside_a_folder_given_outright() {
        let app = Path::new("/Users/you/Library/Application Support/com.wackcode.desktop");
        assert_eq!(root_for(&app.join("v3")), app.join("usage/v1"));
        assert_eq!(root_for(Path::new("/tmp/throwaway")), PathBuf::from("/tmp/throwaway/usage/v1"));
    }

    fn recorder(temp: &Path) -> RunRecorder {
        let connection = Connection {
            id: "conn-1".into(),
            name: "Testing".into(),
            format: wack_llm::WireFormat::OpenaiChat,
            base_url: "https://example.test/v1".into(),
            models: Vec::new(),
            enabled: true,
        };
        RunRecorder::new(&Arc::new(UsageWriter::new(&temp.join("usage/v1"), true)), &connection, "chat-9", Some(Path::new("/repo/root")), Path::new("/repo/root/worktree"), ChatKind::Code)
    }

    #[test]
    fn validates_mode_and_chat_kind_spells_the_ledger_values() {
        assert_eq!(ChatKind::Code.ledger_mode(), "code");
        assert_eq!(ChatKind::Chat.ledger_mode(), "chat");
        let mut record = records()[0].clone();
        for (mode, valid) in [(None, true), (Some("code"), true), (Some("chat"), true), (Some("plan"), false), (Some(""), false)] {
            record.mode = mode.map(str::to_string);
            assert_eq!(record.valid(), valid, "{mode:?}");
        }
        // Absent, it is not written at all: the contract's older records stay byte for byte.
        record.mode = None;
        assert!(!serde_json::to_string(&record).unwrap().contains("\"mode\""));
        record.mode = Some("chat".into());
        assert!(serde_json::to_string(&record).unwrap().ends_with(r#","mode":"chat"}"#), "the last field, as version 2 wrote it");
    }

    #[test]
    fn a_chat_area_run_has_no_project_and_says_chat() {
        let temp = tempfile::tempdir().unwrap();
        let connection = Connection {
            id: "conn-1".into(),
            name: "Testing".into(),
            format: wack_llm::WireFormat::OpenaiChat,
            base_url: "https://example.test/v1".into(),
            models: Vec::new(),
            enabled: true,
        };
        let writer = Arc::new(UsageWriter::new(&temp.path().join("usage/v1"), true));
        RunRecorder::new(&writer, &connection, "chat-7", None, Path::new("/data/v3/scratch/chat-7"), ChatKind::Chat).record(ModelCall {
            purpose: Purpose::Chat,
            model: "m".into(),
            duration_ms: 1,
            outcome: CallOutcome::Completed,
            usage: None,
            subagent_id: None,
        });
        let month = fs::read_dir(temp.path().join("usage/v1")).unwrap().next().unwrap().unwrap().path();
        let file = fs::read_dir(&month).unwrap().next().unwrap().unwrap().path();
        let record: serde_json::Value = serde_json::from_str(fs::read_to_string(&file).unwrap().trim()).unwrap();
        assert_eq!(record["project"], serde_json::Value::Null);
        assert_eq!(record["workspace"], "/data/v3/scratch/chat-7");
        assert_eq!(record["mode"], "chat");
    }

    #[test]
    fn a_run_recorder_stamps_the_contract_fields() {
        let temp = tempfile::tempdir().unwrap();
        recorder(temp.path()).record(ModelCall {
            purpose: Purpose::Chat,
            model: "big-model".into(),
            duration_ms: 1234,
            outcome: CallOutcome::Completed,
            usage: Some(Usage { input: 10, output: 5, cache_read: 3, cache_write: 2 }),
            subagent_id: None,
        });
        let root = temp.path().join("usage/v1");
        let month = fs::read_dir(&root).unwrap().next().unwrap().unwrap().path();
        let file = fs::read_dir(&month).unwrap().next().unwrap().unwrap().path();
        let line = fs::read_to_string(&file).unwrap();
        let record: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(record["v"], 1);
        assert!(Uuid::parse_str(record["id"].as_str().unwrap()).is_ok());
        assert_eq!(record["provider"], "conn-1");
        assert_eq!(record["provider_name"], "Testing");
        assert_eq!(record["model"], "big-model");
        assert_eq!(record["purpose"], "chat");
        assert_eq!(record["subagent_id"], serde_json::Value::Null);
        assert_eq!(record["outcome"], "completed");
        assert_eq!(record["tokens"], serde_json::json!({ "input": 10, "output": 5, "cache_read": 3, "cache_write": 2 }));
        assert_eq!(record["session_id"], "chat-9");
        assert_eq!(record["project"], "/repo/root");
        assert_eq!(record["workspace"], "/repo/root/worktree");
        assert_eq!(record["mode"], "code");
        assert!(record["ts"].as_i64().unwrap() > 0);
        assert_eq!(record["duration_ms"], 1234);
    }

    #[test]
    fn counts_the_provider_did_not_expose_are_null_never_estimated() {
        let temp = tempfile::tempdir().unwrap();
        let run = recorder(temp.path());
        run.record(ModelCall {
            purpose: Purpose::Title,
            model: "m".into(),
            duration_ms: 5,
            outcome: CallOutcome::Failed,
            usage: None,
            subagent_id: None,
        });
        run.record(ModelCall {
            purpose: Purpose::Compaction,
            model: "m".into(),
            duration_ms: 5,
            outcome: CallOutcome::Completed,
            usage: Some(Usage::default()),
            subagent_id: None,
        });
        let root = temp.path().join("usage/v1");
        let month = fs::read_dir(&root).unwrap().next().unwrap().unwrap().path();
        let file = fs::read_dir(&month).unwrap().next().unwrap().unwrap().path();
        let tokens: Vec<Option<Tokens>> = fs::read_to_string(&file)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Record>(line).unwrap().tokens)
            .collect();
        assert_eq!(tokens, [None, None], "absent counts and all-zero counts are both null");
    }

    #[test]
    fn a_sub_agents_calls_carry_its_id_and_its_compactions_keep_their_purpose() {
        let temp = tempfile::tempdir().unwrap();
        let child = wack_agent::usage::ForSubagent { inner: Arc::new(recorder(temp.path())), id: "chat-9/a1".into() };
        for purpose in [Purpose::Chat, Purpose::Compaction] {
            child.record(ModelCall {
                purpose,
                model: "m".into(),
                duration_ms: 5,
                outcome: CallOutcome::Completed,
                usage: None,
                subagent_id: None,
            });
        }
        let root = temp.path().join("usage/v1");
        let month = fs::read_dir(&root).unwrap().next().unwrap().unwrap().path();
        let file = fs::read_dir(&month).unwrap().next().unwrap().unwrap().path();
        let records: Vec<(String, Option<String>)> = fs::read_to_string(&file)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Record>(line).unwrap())
            .map(|record| (record.purpose, record.subagent_id))
            .collect();
        assert_eq!(
            records,
            [("subagent".to_string(), Some("chat-9/a1".to_string())), ("compaction".to_string(), Some("chat-9/a1".to_string()))]
        );
    }
}
