//! Tests for the browser tools: the pure parts (keys, screenshots, the outline) and the rules
//! the core enforces, against an engine that answers DevTools from a script.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use wack_agent::{Tool, ToolContext};

use super::{AgentView, Frame, Node, input, shot, snapshot};
use crate::tabs::{Backend, BoxFuture, ChatTabs, Nav, Owner, Tab, TabId};

type Answer = Arc<dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync>;

/// An engine whose pages load at once and whose DevTools answers come from `answer`.
struct Scripted {
    answer: Answer,
    calls: Mutex<Vec<String>>,
}

impl Backend for Scripted {
    fn create(&self, tab: Arc<Tab>, url: String) -> BoxFuture<Result<(), String>> {
        Box::pin(async move {
            tab.set_live(true);
            tab.update(|state| {
                state.set_address(&url);
                state.page += 1;
            });
            Ok(())
        })
    }
    fn close(&self, _id: TabId) {}
    fn navigate(&self, _id: TabId, _nav: Nav) -> BoxFuture<Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    fn cdp(&self, _id: TabId, method: String, params: Value) -> BoxFuture<Result<Value, String>> {
        self.calls.lock().unwrap().push(method.clone());
        let answer = (self.answer)(&method, &params);
        Box::pin(async move { answer })
    }
    fn answer_dialog(&self, _id: TabId, _accept: bool, _text: Option<String>) -> BoxFuture<Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
}

fn tabs_with(answer: impl Fn(&str, &Value) -> Result<Value, String> + Send + Sync + 'static) -> (ChatTabs, Arc<Scripted>) {
    let backend = Arc::new(Scripted { answer: Arc::new(answer), calls: Mutex::default() });
    (ChatTabs::new(Some(backend.clone())), backend)
}

fn tool(tabs: &ChatTabs, name: &str) -> Arc<dyn Tool> {
    super::tools(tabs.clone(), None).into_iter().find(|tool| tool.spec().name == name).unwrap()
}

fn page_answers(method: &str, _params: &Value) -> Result<Value, String> {
    Ok(match method {
        "Runtime.evaluate" => json!({ "result": { "value": [1280, 800] } }),
        _ => json!({}),
    })
}

#[test]
fn keys_are_named_as_people_write_them() {
    let select = input::parse_key("cmd+a").unwrap();
    assert_eq!((select.modifiers, select.commands.clone(), select.text.clone()), (input::META, vec!["selectAll"], None));
    assert_eq!(input::parse_key("cmd+shift+z").unwrap().commands, vec!["redo"]);
    let capital = input::parse_key("shift+a").unwrap();
    assert_eq!((capital.key.as_str(), capital.text.as_deref()), ("A", Some("A")));
    assert_eq!(input::parse_key("Enter").unwrap().text.as_deref(), Some("\r"));
    assert_eq!(input::parse_key("ArrowDown").unwrap().key_code, 40);
    assert!(input::parse_key("hyper+a").is_err());
    assert!(input::parse_key("Banana").is_err());
}

#[test]
fn ascii_is_keyed_and_the_rest_inserted() {
    let parts = input::typing("hé\nok");
    assert!(matches!(&parts[0], input::Typed::Keys(keys) if keys.len() == 1 && keys[0].key == "h"));
    assert_eq!(parts[1], input::Typed::Insert("é".into()));
    assert!(matches!(&parts[2], input::Typed::Keys(keys) if keys.len() == 3 && keys[0].key == "Enter" && keys[2].key == "k"));
    assert_eq!(parts.len(), 3);
}

#[test]
fn screenshots_are_capped_and_points_map_back() {
    assert_eq!(shot::capture_scale(1280., 2.), 0.5);
    assert_eq!(shot::capture_scale(2560., 2.), 0.25);
    assert_eq!(shot::capture_scale(390., 3.), 1. / 3.);
    let frame = Frame { css: (2560., 1600.), image: (1280., 800.) };
    assert_eq!(shot::to_page(frame, 100., 50.).unwrap(), (200., 100.));
    assert!(shot::to_page(frame, 1300., 10.).is_err(), "outside the picture");
    // SOI, an APP0 segment, then a baseline frame header: 0x0320 × 0x0500 (800 × 1280).
    let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x03, 0x20, 0x05, 0x00, 0x03];
    assert_eq!(shot::jpeg_size(&jpeg), Some((1280, 800)));
}

fn ax(id: &str, role: &str, name: &str, backend: i64, children: &[&str], extra: Value) -> Value {
    let mut node = json!({
        "nodeId": id, "ignored": false, "role": { "value": role }, "name": { "value": name },
        "backendDOMNodeId": backend, "childIds": children,
    });
    if let Value::Object(more) = extra {
        for (key, value) in more {
            node[key] = value;
        }
    }
    node
}

fn sample() -> (Vec<snapshot::FrameTree>, snapshot::Dom) {
    let mut root = ax("1", "RootWebArea", "Sign up", 1, &["2", "3", "4", "5", "6"], json!({}));
    root.as_object_mut().unwrap().remove("parentId");
    let nodes = vec![
        root,
        ax("2", "heading", "Sign up", 2, &["7"], json!({ "parentId": "1", "properties": [{ "name": "level", "value": { "value": 1 } }] })),
        ax("3", "textbox", "Email", 3, &[], json!({ "parentId": "1", "value": { "value": "ada@example.com" } })),
        ax("4", "textbox", "Password", 4, &[], json!({ "parentId": "1", "value": { "value": "hunter2" } })),
        ax("5", "generic", "", 5, &["8"], json!({ "parentId": "1" })),
        ax("6", "link", "Terms", 6, &[], json!({ "parentId": "1" })),
        ax("7", "StaticText", "Sign up", 7, &[], json!({ "parentId": "2" })),
        ax("8", "button", "Create account", 8, &[], json!({ "parentId": "5" })),
    ];
    let mut dom = snapshot::Dom::default();
    dom.attributes.insert(4, [("type".to_string(), "password".to_string())].into());
    dom.attributes.insert(6, [("href".to_string(), "/terms".to_string())].into());
    dom.boxes.insert(8, (10., 2000., 100., 30.));
    dom.visible = Some((0., 0., 1280., 800.));
    (vec![snapshot::FrameTree { frame: String::new(), url: "http://localhost:5173/".into(), nodes }], dom)
}

#[test]
fn the_outline_names_what_to_act_on_and_hides_secrets() {
    let (frames, dom) = sample();
    let mut view = AgentView::default();
    let outline = snapshot::render(&frames, &[], &dom, &mut view, &snapshot::Options::default());
    assert_eq!(
        outline,
        [
            "document \"Sign up\"",
            "  heading \"Sign up\" level=1",
            "  textbox \"Email\" [ref_1] value=\"ada@example.com\"",
            "  textbox \"Password\" [ref_2] value=[redacted] type=password",
            "  button \"Create account\" [ref_3] (off-screen)",
            "  link \"Terms\" [ref_4] href=/terms",
        ]
        .join("\n")
    );
    assert!(!outline.contains("hunter2"));
}

#[test]
fn refs_hold_across_snapshots_and_reset_with_the_page() {
    let (frames, dom) = sample();
    let mut view = AgentView::default();
    view.on_page(1);
    snapshot::render(&frames, &[], &dom, &mut view, &snapshot::Options::default());
    let button = view.refs.get(&3).cloned();
    snapshot::render(&frames, &[], &dom, &mut view, &snapshot::Options::default());
    assert_eq!(view.refs.get(&3).cloned(), button, "the same node keeps its ref");
    assert_eq!(view.refs.len(), 4);
    view.on_page(2);
    assert!(view.refs.is_empty(), "a new page resets refs");
    assert_eq!(view.ref_for(Node { frame: String::new(), backend: 8 }), 1);
}

#[test]
fn an_outline_can_be_narrowed() {
    let (frames, dom) = sample();
    let mut view = AgentView::default();
    let options = snapshot::Options { query: Some("ACCOUNT".into()), ..Default::default() };
    let found = snapshot::render(&frames, &[], &dom, &mut view, &options);
    assert_eq!(found, "1 lines match \"account\":\n  button \"Create account\" [ref_3] (off-screen)");
    let options = snapshot::Options { under: Some(5), ..Default::default() };
    assert_eq!(snapshot::render(&frames, &[], &dom, &mut view, &options), "button \"Create account\" [ref_3] (off-screen)");
    let options = snapshot::Options { max_chars: 40, ..Default::default() };
    assert!(snapshot::render(&frames, &[], &dom, &mut view, &options).contains("cut at 40 characters"));
}

#[tokio::test]
async fn an_agents_calls_run_in_the_order_it_made_them() {
    let (tabs, _) = tabs_with(page_answers);
    tabs.open(Owner::Agent, "http://localhost:5173/").await.unwrap();
    let wait = tool(&tabs, "browser_wait");
    let console = tool(&tabs, "browser_console");
    let context = ToolContext::detached("/");
    let order = Mutex::new(Vec::new());
    let first = async {
        wait.run(json!({ "ms": 60 }), &context).await.unwrap();
        order.lock().unwrap().push("wait");
    };
    let second = async {
        console.run(json!({}), &context).await.unwrap();
        order.lock().unwrap().push("console");
    };
    futures_join(first, second).await;
    assert_eq!(*order.lock().unwrap(), ["wait", "console"], "the quick call waited its turn");
}

async fn futures_join(a: impl std::future::Future<Output = ()>, b: impl std::future::Future<Output = ()>) {
    tokio::join!(a, b);
}

#[tokio::test]
async fn a_tab_the_user_controls_is_left_alone() {
    let (tabs, backend) = tabs_with(page_answers);
    let tab = tabs.open(Owner::Agent, "http://localhost:5173/").await.unwrap();
    tab.set_user_control(true);
    let error = tool(&tabs, "browser_act").run(json!({ "action": "click", "x": 1, "y": 1 }), &ToolContext::detached("/")).await.unwrap_err();
    assert!(error.contains("taken control"), "{error}");
    assert!(backend.calls.lock().unwrap().is_empty(), "nothing reached the page");
}

#[tokio::test]
async fn scripts_run_only_on_this_mac() {
    let (tabs, _) = tabs_with(page_answers);
    tabs.open(Owner::Agent, "https://example.com/").await.unwrap();
    let error = tool(&tabs, "browser_eval").run(json!({ "expression": "document.cookie" }), &ToolContext::detached("/")).await.unwrap_err();
    assert!(error.contains("only on pages on this Mac"), "{error}");
}

#[tokio::test]
async fn sub_agents_get_their_own_tabs_and_close_only_those() {
    let (tabs, _) = tabs_with(page_answers);
    let users = tabs.open(Owner::User, "http://localhost:5173/").await.unwrap();
    let mut child = ToolContext::detached("/");
    child.caller = Some("a1".into());
    let navigate = tool(&tabs, "browser_navigate");
    navigate.run(json!({ "url": "http://localhost:5173/a" }), &child).await.unwrap();
    let theirs = tabs.current_of(&Owner::SubAgent("a1".into())).unwrap();
    assert_eq!(theirs.owner, Owner::SubAgent("a1".into()));
    let close = tool(&tabs, "browser_tabs");
    let error = close.run(json!({ "action": "close", "tab": users.id }), &ToolContext::detached("/")).await.unwrap_err();
    assert!(error.contains("the user's"), "{error}");
    close.run(json!({ "action": "close", "tab": theirs.id }), &ToolContext::detached("/")).await.expect("the chat's agent may close a sub-agent's tab");
}

#[tokio::test]
async fn coordinates_wait_for_a_screenshot_and_go_stale_with_a_resize() {
    let size = Arc::new(Mutex::new(json!([1280, 800])));
    let shared = size.clone();
    let (tabs, _) = tabs_with(move |method, _| {
        Ok(match method {
            "Runtime.evaluate" => json!({ "result": { "value": shared.lock().unwrap().clone() } }),
            _ => json!({}),
        })
    });
    let tab = tabs.open(Owner::Agent, "http://localhost:5173/").await.unwrap();
    let act = tool(&tabs, "browser_act");
    let context = ToolContext::detached("/");
    let error = act.run(json!({ "action": "click", "x": 10, "y": 10 }), &context).await.unwrap_err();
    assert!(error.contains("screenshot first"), "{error}");
    tab.agent.lock().unwrap().frame = Some(Frame { css: (1280., 800.), image: (1280., 800.) });
    act.run(json!({ "action": "click", "x": 10, "y": 10 }), &context).await.expect("the frame still holds");
    *size.lock().unwrap() = json!([900, 800]);
    let error = act.run(json!({ "action": "click", "x": 10, "y": 10 }), &context).await.unwrap_err();
    assert!(error.contains("size changed"), "{error}");
}

#[test]
fn addresses_get_the_scheme_they_mean() {
    assert_eq!(super::tool::with_scheme_for_tests("localhost:3000/a"), "http://localhost:3000/a");
    assert_eq!(super::tool::with_scheme_for_tests("127.0.0.1:8080"), "http://127.0.0.1:8080");
    assert_eq!(super::tool::with_scheme_for_tests("example.com"), "https://example.com");
    assert_eq!(super::tool::with_scheme_for_tests("https://a.b/"), "https://a.b/");
}

#[test]
fn every_tool_is_deferred_and_only_lookers_are_safe() {
    let (tabs, _) = tabs_with(page_answers);
    let specs: Vec<_> = super::tools(tabs.clone(), None).iter().map(|tool| tool.spec()).collect();
    assert_eq!(specs.len(), super::NAMES.len() - 1, "no browser_preview without a project");
    assert!(specs.iter().all(|spec| spec.name != "browser_preview"));
    let with_preview = super::tools(tabs.clone(), Some(Arc::new(Server::default()))).iter().map(|tool| tool.spec()).collect::<Vec<_>>();
    assert_eq!(with_preview.len(), super::NAMES.len());
    assert!(with_preview.iter().any(|spec| spec.name == "browser_preview" && !spec.parallel_safe), "it starts a command: not for Plan mode");
    assert!(specs.iter().all(|spec| spec.loading == wack_agent::Loading::Deferred));
    let safe: Vec<&str> = specs.iter().filter(|spec| spec.parallel_safe).map(|spec| spec.name.as_str()).collect();
    assert_eq!(safe, ["browser_tabs", "browser_navigate", "browser_snapshot", "browser_screenshot", "browser_wait", "browser_text", "browser_console", "browser_network"]);
}

/// A dev server that is always running, counting how often it was asked for.
#[derive(Default)]
struct Server {
    asked: Mutex<u32>,
}

#[async_trait::async_trait]
impl super::Preview for Server {
    async fn serve(&self, _port: Option<u16>) -> Result<super::Served, String> {
        let mut asked = self.asked.lock().unwrap();
        *asked += 1;
        Ok(super::Served { url: "http://localhost:5173/".into(), command: "npm run dev".into(), started: *asked == 1 })
    }
}

#[tokio::test]
async fn a_preview_opens_the_server_once_and_never_reloads_it() {
    let (tabs, _) = tabs_with(page_answers);
    let preview = super::tools(tabs.clone(), Some(Arc::new(Server::default()))).into_iter().find(|tool| tool.spec().name == "browser_preview").unwrap();
    let context = ToolContext::detached(std::path::PathBuf::from("/"));
    let first = preview.run(json!({}), &context).await.unwrap().text;
    assert!(first.contains("Started the project's Run command (`npm run dev`)") && first.contains("Opened a new tab"), "{first}");
    let page = tabs.tabs()[0].state().page;
    let second = preview.run(json!({}), &context).await.unwrap().text;
    assert!(second.contains("already running") && second.contains("already shows it (not reloaded)"), "{second}");
    assert_eq!(tabs.tabs().len(), 1);
    assert_eq!(tabs.tabs()[0].state().page, page, "the page was not loaded again");
}
