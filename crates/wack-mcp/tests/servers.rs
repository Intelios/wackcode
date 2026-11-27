//! MCP against the fixture server (`fixture/main.rs`): the three transports, credentials, failures,
//! and the hub under real runs of the agent loop. Ported from version 2's
//! `describe("MCP servers")` integration tests.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;
use wack_agent::{Agent, Control, Event, Outcome, Profile, Registry, Session, Settings};
use wack_llm::testing::{ScriptedProvider, call, say};
use wack_mcp::{Hub, ServerSpec, State, Transport, assemble, slug};

const FIXTURE: &str = env!("CARGO_BIN_EXE_wack-mcp-fixture");
const ENV_SECRET: &str = "mcp-env-secret-1234";

fn stdio(name: &str, args: &[&str]) -> ServerSpec {
    ServerSpec {
        id: format!("mcp-{}", slug(name)),
        name: name.into(),
        slug: slug(name),
        transport: Transport::Stdio,
        timeout: Duration::from_secs(10),
        command: FIXTURE.into(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        env: BTreeMap::from([("MOCK_MCP_SECRET".into(), ENV_SECRET.into())]),
        url: String::new(),
        headers: BTreeMap::new(),
        disabled_tools: Vec::new(),
    }
}

fn remote(name: &str, transport: Transport, url: &str, token: &str) -> ServerSpec {
    ServerSpec {
        transport,
        command: String::new(),
        args: Vec::new(),
        env: BTreeMap::new(),
        url: url.into(),
        headers: BTreeMap::from([("Authorization".into(), format!("Bearer {token}"))]),
        ..stdio(name, &[])
    }
}

/// Starts the fixture over HTTP and returns its /mcp URL. The process ends with the guard.
async fn http_fixture(args: &[&str]) -> (String, tokio::process::Child) {
    let mut child = tokio::process::Command::new(FIXTURE)
        .args(["--http", "0"])
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let line = lines.next_line().await.unwrap().unwrap();
    let url = line.rsplit(' ').next().unwrap().to_string();
    (url, child)
}

fn text(result: &Value) -> String {
    result["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

// -------------------------------------------------------------------------------------------------
// Connections

#[tokio::test]
async fn a_stdio_server_lists_tools_and_gets_its_environment_in_its_folder() {
    let folder = tempfile::tempdir().unwrap();
    let hub = Hub::new(None);
    let tools = hub.probe(&stdio("Mock", &[]), folder.path()).await.unwrap();
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, ["echo", "peek", "whoami", "fail", "hang"]);
    assert!(tools.iter().find(|tool| tool.name == "peek").unwrap().read_only());
    assert!(!tools.iter().find(|tool| tool.name == "echo").unwrap().read_only());

    let connection = wack_mcp::Connection::connect(&stdio("Mock", &[]), folder.path()).await.unwrap();
    let report: Value = serde_json::from_str(&text(&connection.call("whoami", json!({}), &CancellationToken::new()).await.unwrap())).unwrap();
    assert_eq!(report["secret"], ENV_SECRET);
    assert_eq!(report["authorization"], Value::Null);
    assert_eq!(PathBuf::from(report["cwd"].as_str().unwrap()).canonicalize().unwrap(), folder.path().canonicalize().unwrap());
    let echoed = connection.call("echo", json!({ "text": "hello" }), &CancellationToken::new()).await.unwrap();
    assert_eq!(text(&echoed), "hello");
    let unknown = connection.call("nope", json!({}), &CancellationToken::new()).await.unwrap_err();
    assert!(unknown.contains("Unknown tool: nope"), "{unknown}");
}

#[tokio::test]
async fn a_failure_is_a_sentence_without_the_servers_secrets() {
    let folder = tempfile::tempdir().unwrap();
    let mut broken = stdio("Broken", &["-c", "echo 'bad token secret-value-123' >&2; exit 1"]);
    broken.command = "/bin/sh".into();
    broken.env.insert("TOKEN".into(), "secret-value-123".into());
    let error = Hub::new(None).probe(&broken, folder.path()).await.unwrap_err();
    assert!(error.contains("[credential redacted]"), "{error}");
    assert!(!error.contains("secret-value-123"), "{error}");

    let mut missing = stdio("Missing", &[]);
    missing.command = "wackcode-no-such-command".into();
    let error = Hub::new(None).probe(&missing, folder.path()).await.unwrap_err();
    assert_eq!(error, "Command not found: wackcode-no-such-command. Check the command, or give its full path.");
}

#[tokio::test]
async fn a_call_that_never_answers_times_out() {
    let folder = tempfile::tempdir().unwrap();
    let mut spec = stdio("Mock", &[]);
    spec.timeout = Duration::from_millis(1_000);
    let connection = wack_mcp::Connection::connect(&spec, folder.path()).await.unwrap();
    let error = connection.call("hang", json!({}), &CancellationToken::new()).await.unwrap_err();
    assert_eq!(error, "It did not respond within 1000 ms.");
    // The connection is still good afterwards.
    assert_eq!(text(&connection.call("peek", json!({}), &CancellationToken::new()).await.unwrap()), "peeked");
}

#[tokio::test]
async fn http_and_sse_servers_get_their_headers() {
    let folder = tempfile::tempdir().unwrap();
    let (url, _server) = http_fixture(&["--require-auth", "http-token-5678"]).await;
    let whoami = |spec: ServerSpec| {
        let folder = folder.path().to_path_buf();
        async move {
            let connection = wack_mcp::Connection::connect(&spec, &folder).await.unwrap();
            let report: Value = serde_json::from_str(&text(&connection.call("whoami", json!({}), &CancellationToken::new()).await.unwrap())).unwrap();
            report["authorization"].as_str().unwrap_or_default().to_string()
        }
    };
    assert_eq!(whoami(remote("Remote", Transport::Http, &url, "http-token-5678")).await, "Bearer http-token-5678");
    assert_eq!(whoami(remote("Legacy", Transport::Sse, &url.replace("/mcp", "/sse"), "http-token-5678")).await, "Bearer http-token-5678");

    let refused = Hub::new(None).probe(&remote("Refused", Transport::Http, &url, "wrong-token-4321"), folder.path()).await.unwrap_err();
    assert!(refused.starts_with("The server refused the request (HTTP 401)."), "{refused}");
    assert!(!refused.contains("wrong-token-4321"));
}

#[tokio::test]
async fn streamable_http_answers_by_event_stream_and_keeps_its_session() {
    let folder = tempfile::tempdir().unwrap();
    let (url, _server) = http_fixture(&["--sse-replies", "--session"]).await;
    let connection = wack_mcp::Connection::connect(&remote("Streamed", Transport::Http, &url, "unused-token"), folder.path()).await.unwrap();
    assert_eq!(connection.tools().len(), 5);
    let echoed = connection.call("echo", json!({ "text": "over a stream" }), &CancellationToken::new()).await.unwrap();
    assert_eq!(text(&echoed), "over a stream");
}

#[tokio::test]
async fn redirects_stay_on_the_servers_origin() {
    let folder = tempfile::tempdir().unwrap();
    let (elsewhere, _other) = http_fixture(&[]).await;
    let (url, _server) = http_fixture(&["--redirect", &elsewhere]).await;
    let (same, _same) = http_fixture(&[]).await;

    // Within the origin: /moved → /mcp.
    let moved = same.replace("/mcp", "/moved");
    assert!(Hub::new(None).probe(&remote("Moved", Transport::Http, &moved, "token-abcdefgh"), folder.path()).await.is_ok());

    let error = Hub::new(None).probe(&remote("Away", Transport::Http, &url.replace("/mcp", "/moved"), "token-abcdefgh"), folder.path()).await.unwrap_err();
    assert!(error.contains("only follows redirects within"), "{error}");
    let error = Hub::new(None).probe(&remote("Away", Transport::Sse, &url.replace("/mcp", "/sse"), "token-abcdefgh"), folder.path()).await.unwrap_err();
    assert!(error.contains("only sends to"), "{error}");
}

// -------------------------------------------------------------------------------------------------
// The hub under the agent loop

struct Run {
    provider: ScriptedProvider,
    events: Arc<Mutex<Vec<Event>>>,
    starting: Arc<Mutex<Vec<Vec<String>>>>,
    outcome: Result<Outcome, String>,
}

impl Run {
    fn results(&self) -> Vec<(String, bool)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                Event::ToolResult { content, is_error, .. } => Some((content.clone(), *is_error)),
                _ => None,
            })
            .collect()
    }

    fn offered(&self, request: usize) -> Vec<String> {
        // Deferred tools reach a provider without its own search through the prompt's tool list.
        let request = &self.provider.requests()[request];
        request.system.iter().flat_map(|part| part.text.lines()).filter_map(|line| line.strip_prefix("- ")).filter_map(|line| line.split(':').next()).filter(|name| name.starts_with("mcp__")).map(str::to_string).collect()
    }

    fn noted(&self) -> Vec<String> {
        self.events.lock().unwrap().iter().filter_map(|event| match event {
            Event::Noted { tag, text } if tag.starts_with("mcp:") => Some(text.clone()),
            _ => None,
        }).collect()
    }
}

async fn run(hub: &Hub, specs: &[ServerSpec], folder: &Path, steps: Vec<wack_llm::testing::Step>, control: Control) -> Run {
    let session_path = folder.join(".sessions").join("chat.jsonl");
    let session = if session_path.exists() { Session::open(&session_path).unwrap() } else { Session::create(&session_path, folder).unwrap() };
    let starting = Arc::new(Mutex::new(Vec::new()));
    let starting_sink = {
        let starting = starting.clone();
        Arc::new(move |names: Vec<String>| starting.lock().unwrap().push(names)) as wack_mcp::Starting
    };
    let assembly = assemble(hub, specs, folder, &control.stop_token(), starting_sink).await;
    let provider = ScriptedProvider::new(steps);
    let registry = Arc::new(Registry::new(assembly.tools).unwrap());
    let settings = Settings {
        model: "m".into(),
        effort: None,
        output_limit: None,
        context_window: None,
        accepts_images: true,
        ultra: false,
        ultra_prompt: None,
        compaction_prompt: None,
        commit_attribution: false,
    };
    let mut agent = Agent::new(Arc::new(provider.clone()), registry, session, &Profile::code(), settings);
    if let Some(feature) = assembly.feature {
        agent = agent.with_feature(Box::new(feature));
    }
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let events = events.clone();
        move |event: Event| events.lock().unwrap().push(event)
    };
    let outcome = agent.run("go", &control, &sink).await;
    Run { provider, events, starting, outcome }
}

fn mcp_call(tool: &str, input: Value) -> wack_llm::testing::Step {
    call(&[("c1", tool, input)])
}

#[tokio::test]
async fn an_uncached_server_is_waited_for_and_its_connection_reused() {
    let folder = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let hub = Hub::new(Some(cache.path().to_path_buf()));
    let specs = [stdio("Mock", &[])];

    let first = run(&hub, &specs, folder.path(), vec![mcp_call("mcp__mock__whoami", json!({})), say("done")], Control::new()).await;
    assert_eq!(first.outcome, Ok(Outcome::Done));
    assert_eq!(first.starting.lock().unwrap().as_slice(), [vec!["Mock".to_string()], vec![]]);
    assert!(first.offered(0).contains(&"mcp__mock__whoami".to_string()));
    let (content, is_error) = &first.results()[0];
    assert!(!is_error, "{content}");
    assert_eq!(serde_json::from_str::<Value>(content).unwrap()["secret"], ENV_SECRET);

    // An error result is a failed call, and the next run reuses the connection without waiting.
    let second = run(&hub, &specs, folder.path(), vec![mcp_call("mcp__mock__fail", json!({})), say("done")], Control::new()).await;
    assert_eq!(second.results()[0], ("The mock tool failed on purpose.".to_string(), true));
    assert!(second.starting.lock().unwrap().is_empty());
    assert_eq!(hub.states().len(), 1);
    assert!(matches!(hub.states()[0].state, State::Connected { tools: 5 }));
    hub.terminate_all();
}

#[tokio::test]
async fn switched_off_tools_and_servers_are_not_offered() {
    let folder = tempfile::tempdir().unwrap();
    let hub = Hub::new(None);
    let mut spec = stdio("Mock", &[]);
    spec.disabled_tools = vec!["echo".into()];
    let first = run(&hub, std::slice::from_ref(&spec), folder.path(), vec![say("hi")], Control::new()).await;
    assert!(first.offered(0).contains(&"mcp__mock__peek".to_string()));
    assert!(!first.offered(0).contains(&"mcp__mock__echo".to_string()));
    let none = run(&hub, &[], folder.path(), vec![say("hi")], Control::new()).await;
    assert!(none.offered(0).is_empty());
    hub.retain(|_, _, _| false);
    assert!(hub.states().is_empty());
}

#[tokio::test]
async fn retain_can_retire_one_folder_while_keeping_another_alive() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let hub = Hub::new(None);
    let spec = stdio("Mock", &[]);
    let fp = spec.fingerprint();

    let a = run(&hub, std::slice::from_ref(&spec), first.path(), vec![say("hi")], Control::new()).await;
    let b = run(&hub, std::slice::from_ref(&spec), second.path(), vec![say("hi")], Control::new()).await;
    assert!(a.offered(0).contains(&"mcp__mock__peek".to_string()));
    assert!(b.offered(0).contains(&"mcp__mock__peek".to_string()));
    assert_eq!(hub.states().len(), 2);

    // Scope the server away from `second`: only `first` stays connected.
    hub.retain(|id, fingerprint, folder| id == spec.id && fingerprint == fp && folder == first.path());
    let states = hub.states();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].folder, first.path());
    assert!(matches!(states[0].state, State::Connected { .. }));
    hub.terminate_all();
}

/// Warms the cache with a gated server, then starts a new hub (a relaunch) whose server is held
/// behind the gate.
async fn cached(folder: &Path, cache: &Path, tools: Value) -> (Hub, ServerSpec, PathBuf, PathBuf) {
    let gate = folder.join("startup-gate");
    let tools_file = folder.join("tools.json");
    std::fs::write(&gate, "open").unwrap();
    std::fs::write(&tools_file, tools.to_string()).unwrap();
    let spec = stdio("Mock", &["--startup-gate", gate.to_str().unwrap(), "--tools-file", tools_file.to_str().unwrap()]);
    let warm = Hub::new(Some(cache.to_path_buf()));
    warm.probe(&spec, folder).await.unwrap();
    std::fs::remove_file(&gate).unwrap();
    (Hub::new(Some(cache.to_path_buf())), spec, gate, tools_file)
}

fn peek_tool() -> Value {
    json!([{ "name": "peek", "description": "Read a value.", "inputSchema": { "type": "object", "properties": {} }, "annotations": { "readOnlyHint": true } }])
}

#[tokio::test]
async fn cached_tools_start_at_once_and_an_early_call_waits_for_its_server() {
    let folder = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let (hub, spec, gate, _) = cached(folder.path(), cache.path(), peek_tool()).await;
    let opener = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        std::fs::write(gate, "open").unwrap();
    });
    let started = Instant::now();
    let result = run(&hub, &[spec], folder.path(), vec![mcp_call("mcp__mock__peek", json!({})), say("done")], Control::new()).await;
    opener.await.unwrap();
    assert!(result.offered(0).contains(&"mcp__mock__peek".to_string()));
    assert_eq!(result.results()[0], ("peeked".to_string(), false));
    // The run did not wait before its first request; the call waited, reporting it.
    assert_eq!(result.starting.lock().unwrap().as_slice(), [vec!["Mock".to_string()], vec![]]);
    assert!(started.elapsed() >= Duration::from_millis(300));
    hub.terminate_all();
}

#[tokio::test]
async fn a_cached_tool_is_checked_against_the_live_server() {
    for change in ["removed", "schema", "read-only"] {
        let folder = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let (hub, spec, gate, tools_file) = cached(folder.path(), cache.path(), peek_tool()).await;
        let mut updated = peek_tool();
        match change {
            "removed" => updated = json!([]),
            "schema" => updated[0]["inputSchema"] = json!({ "type": "object", "properties": { "newInput": { "type": "string" } }, "required": ["newInput"] }),
            _ => updated[0]["annotations"] = json!({ "readOnlyHint": false }),
        }
        std::fs::write(&tools_file, updated.to_string()).unwrap();
        std::fs::write(&gate, "open").unwrap();
        let result = run(&hub, std::slice::from_ref(&spec), folder.path(), vec![mcp_call("mcp__mock__peek", json!({})), say("done")], Control::new()).await;
        let (content, is_error) = result.results()[0].clone();
        assert!(is_error, "{change}: {content}");
        let expected = match change {
            "removed" => "no longer offered",
            "schema" => "changed its input schema",
            _ => "no longer marks MCP tool \"peek\" read-only",
        };
        assert!(content.contains(expected), "{change}: {content}");
        // The next run offers what the server declares now.
        let next = run(&hub, std::slice::from_ref(&spec), folder.path(), vec![say("ok")], Control::new()).await;
        assert_eq!(next.offered(0).contains(&"mcp__mock__peek".to_string()), change != "removed", "{change}");
        hub.terminate_all();
    }
}

#[tokio::test]
async fn a_credential_change_misses_the_cache() {
    let folder = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let (hub, mut spec, gate, _) = cached(folder.path(), cache.path(), peek_tool()).await;
    spec.env.insert("MOCK_MCP_SECRET".into(), "changed-secret-5678".into());
    let opener = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        std::fs::write(gate, "open").unwrap();
    });
    let result = run(&hub, &[spec], folder.path(), vec![say("ok")], Control::new()).await;
    opener.await.unwrap();
    // No catalogue: the run waited for the server before its first request.
    assert_eq!(result.starting.lock().unwrap().first(), Some(&vec!["Mock".to_string()]));
    assert!(result.offered(0).contains(&"mcp__mock__peek".to_string()));
    hub.terminate_all();
}

#[tokio::test]
async fn a_server_that_cannot_start_is_reported_once_and_holds_nothing_up() {
    let folder = tempfile::tempdir().unwrap();
    let hub = Hub::new(None);
    let mut missing = stdio("Mock", &[]);
    missing.command = "wackcode-no-such-command".into();
    missing.env.insert("TOKEN".into(), "broken-secret-9876".into());
    let first = run(&hub, std::slice::from_ref(&missing), folder.path(), vec![say("hi")], Control::new()).await;
    let notes = first.noted();
    assert_eq!(notes, ["MCP server \"Mock\" isn't available: Command not found: wackcode-no-such-command. Check the command, or give its full path. Check it in Settings › MCP servers."]);
    assert!(!notes[0].contains("broken-secret-9876"));

    // Within the back-off it is not retried, not offered, and not reported again in this chat.
    let started = Instant::now();
    let second = run(&hub, std::slice::from_ref(&missing), folder.path(), vec![say("again")], Control::new()).await;
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(second.noted().is_empty());
    assert!(second.starting.lock().unwrap().is_empty());
    assert!(matches!(&hub.states()[0].state, State::Failed(error) if error.starts_with("Command not found")));
}

#[tokio::test]
async fn stopping_while_a_server_starts_stops_only_the_wait() {
    let folder = tempfile::tempdir().unwrap();
    let hub = Hub::new(None);
    let gate = folder.path().join("gate");
    let spec = stdio("Mock", &["--startup-gate", gate.to_str().unwrap()]);
    let control = Control::new();
    let stopper = {
        let control = control.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            control.stop();
        })
    };
    let started = Instant::now();
    let stopped = run(&hub, std::slice::from_ref(&spec), folder.path(), vec![], control).await;
    stopper.await.unwrap();
    assert_eq!(stopped.outcome, Ok(Outcome::Stopped));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(stopped.provider.requests().is_empty());
    // The connection carried on, and the next message finds it.
    assert!(matches!(hub.states()[0].state, State::Connecting));
    std::fs::write(&gate, "open").unwrap();
    let next = run(&hub, std::slice::from_ref(&spec), folder.path(), vec![mcp_call("mcp__mock__peek", json!({})), say("done")], Control::new()).await;
    assert_eq!(next.results()[0], ("peeked".to_string(), false));
    hub.terminate_all();
}

#[tokio::test]
async fn projects_get_their_own_connections_and_idle_ones_close() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let hub = Hub::with_idle_after(None, Duration::from_millis(300));
    let specs = [stdio("Mock", &[])];
    let one = run(&hub, &specs, first.path(), vec![mcp_call("mcp__mock__whoami", json!({})), say("done")], Control::new()).await;
    let two = run(&hub, &specs, second.path(), vec![mcp_call("mcp__mock__whoami", json!({})), say("done")], Control::new()).await;
    let cwd = |run: &Run| PathBuf::from(serde_json::from_str::<Value>(&run.results()[0].0).unwrap()["cwd"].as_str().unwrap().to_string()).canonicalize().unwrap();
    assert_eq!(cwd(&one), first.path().canonicalize().unwrap());
    assert_eq!(cwd(&two), second.path().canonicalize().unwrap());
    assert_eq!(hub.states().len(), 2);
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert!(hub.states().is_empty());
}

#[tokio::test]
async fn quitting_ends_every_server_process() {
    let folder = tempfile::tempdir().unwrap();
    let hub = Hub::new(None);
    let pid_file = folder.path().join("pid");
    let mut spec = stdio("Shell", &["-c", &format!("echo $$ > {}; exec {FIXTURE}", pid_file.display())]);
    spec.command = "/bin/sh".into();
    let result = run(&hub, &[spec], folder.path(), vec![mcp_call("mcp__shell__peek", json!({})), say("done")], Control::new()).await;
    assert_eq!(result.results()[0], ("peeked".to_string(), false));
    let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
    assert!(running(pid));
    hub.terminate_all();
    let deadline = Instant::now() + Duration::from_secs(3);
    while running(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!running(pid), "the server outlived the hub");
}

/// Whether a process is alive: a zombie waiting to be reaped is not.
fn running(pid: i32) -> bool {
    let output = std::process::Command::new("/bin/ps").args(["-o", "stat=", "-p", &pid.to_string()]).output().unwrap();
    let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
    !state.is_empty() && !state.starts_with('Z')
}
