//! The tools against `FakeDesktop`: access, refs, pictures and waiting.

use std::{path::PathBuf, sync::Arc};

use serde_json::json;
use wack_agent::ToolContext;

use super::*;
use crate::{
    backend::fake::FakeDesktop,
    computer::{Answer, Ask, Computer, Settings},
    geometry::Rect,
    outline::Node,
};

fn node(handle: usize, role: &str, title: Option<&str>, frame: Option<Rect>, children: Vec<Node>) -> Node {
    Node { handle, role: role.into(), title: title.map(str::to_string), frame, children, ..Node::default() }
}

/// TextEdit with a window holding a text area and a Save button.
fn textedit_tree() -> Node {
    node(
        1,
        "AXWindow",
        Some("Notes.txt"),
        Some(Rect::new(100.0, 100.0, 800.0, 600.0)),
        vec![
            Node { value: Some("Hello".into()), ..node(2, "AXTextArea", None, Some(Rect::new(100.0, 130.0, 800.0, 500.0)), vec![]) },
            node(3, "AXButton", Some("Save"), Some(Rect::new(800.0, 650.0, 80.0, 30.0)), vec![]),
        ],
    )
}

struct Setup {
    desktop: FakeDesktop,
    computer: Computer,
    chat: ChatComputer,
    window: u32,
}

fn setup() -> Setup {
    let desktop = FakeDesktop::new();
    let window = desktop.add_app(10, "TextEdit", "com.apple.TextEdit", textedit_tree());
    desktop.add_app(20, "Terminal", "com.apple.Terminal", Node::default());
    desktop.add_app(30, "Safari", "com.apple.Safari", Node::default());
    let computer = Computer::new(Arc::new(desktop.clone()), Settings { enabled: true, ..Settings::default() });
    let chat = computer.chat();
    Setup { desktop, computer, chat, window }
}

/// Answers every card with `answer`, and records what was asked.
fn answering(chat: &ChatComputer, answer: Answer) -> Arc<std::sync::Mutex<Vec<Ask>>> {
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = asked.clone();
    let answerer = chat.clone();
    chat.set_observer(move |_| {
        if let Some(card) = answerer.card() {
            seen.lock().unwrap().push(card.ask.clone());
            answerer.answer(card.id, answer);
        }
    });
    asked
}

async fn run(chat: &ChatComputer, name: &str, input: serde_json::Value) -> Result<wack_agent::ToolOutput, String> {
    call(chat.clone(), name.into(), input, std::env::temp_dir()).await
}

#[tokio::test]
async fn sub_agents_and_a_switched_off_computer_are_refused() {
    let setup = setup();
    let tool = tools(setup.chat.clone()).into_iter().find(|tool| tool.spec().name == "computer_apps").unwrap();
    let mut context = ToolContext::detached(PathBuf::from("/tmp"));
    context.caller = Some("child-1".into());
    let refused = tool.run(json!({}), &context).await.unwrap_err();
    assert!(refused.contains("sub-agents"), "{refused}");
    setup.computer.set_settings(Settings::default());
    let refused = run(&setup.chat, "computer_apps", json!({})).await.unwrap_err();
    assert!(refused.contains("Settings › Computer use"), "{refused}");
}

#[tokio::test]
async fn missing_permissions_say_where_to_grant_them() {
    let setup = setup();
    setup.desktop.state.lock().unwrap().permissions.screen_recording = false;
    let refused = run(&setup.chat, "computer_screenshot", json!({"app": "TextEdit"})).await.unwrap_err();
    assert!(refused.contains("Screen Recording"), "{refused}");
    setup.desktop.state.lock().unwrap().permissions.accessibility = false;
    let refused = run(&setup.chat, "computer_snapshot", json!({"app": "TextEdit"})).await.unwrap_err();
    assert!(refused.contains("Accessibility"), "{refused}");
    // Listing apps needs neither.
    assert!(run(&setup.chat, "computer_apps", json!({})).await.is_ok());
}

#[tokio::test]
async fn apps_show_access_and_list_windows_only_for_allowed_apps() {
    let setup = setup();
    let listed = run(&setup.chat, "computer_apps", json!({})).await.unwrap().text;
    assert!(listed.contains("TextEdit (com.apple.TextEdit): ask"), "{listed}");
    assert!(listed.contains("Terminal (com.apple.Terminal): blocked"), "{listed}");
    assert!(listed.contains("Safari (com.apple.Safari): ask (look-only)"), "{listed}");
    assert!(!listed.contains("Notes.txt"), "an unallowed app's window titles stay private: {listed}");
    answering(&setup.chat, Answer::Allow);
    run(&setup.chat, "computer_snapshot", json!({"app": "TextEdit"})).await.unwrap();
    let listed = run(&setup.chat, "computer_apps", json!({"query": "text"})).await.unwrap().text;
    assert!(listed.contains("TextEdit (com.apple.TextEdit): allowed"), "{listed}");
    assert!(listed.contains(&format!("window {} \"TextEdit window\"", setup.window)), "{listed}");
    assert!(!listed.contains("Terminal"), "the query narrows the list: {listed}");
}

#[tokio::test]
async fn a_first_look_asks_and_a_denial_holds_until_cleared() {
    let setup = setup();
    let asked = answering(&setup.chat, Answer::Deny);
    let refused = run(&setup.chat, "computer_snapshot", json!({"app": "TextEdit"})).await.unwrap_err();
    assert!(refused.contains("said no"), "{refused}");
    assert_eq!(asked.lock().unwrap().len(), 1);
    assert!(matches!(&asked.lock().unwrap()[0], Ask::App { app, launching: false, .. } if app.name == "TextEdit"));
    // Denied: refused again without a second card.
    assert!(run(&setup.chat, "computer_snapshot", json!({"app": "TextEdit"})).await.is_err());
    assert_eq!(asked.lock().unwrap().len(), 1);
    setup.chat.clear_denials();
    let asked = answering(&setup.chat, Answer::Allow);
    assert!(run(&setup.chat, "computer_snapshot", json!({"app": "TextEdit"})).await.is_ok());
    assert_eq!(asked.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn blocked_apps_and_wackcode_never_raise_a_card() {
    let setup = setup();
    let asked = answering(&setup.chat, Answer::Allow);
    let refused = run(&setup.chat, "computer_snapshot", json!({"app": "Terminal"})).await.unwrap_err();
    assert!(refused.contains("never controls"), "{refused}");
    assert!(asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn snapshots_fence_app_text_and_remember_refs() {
    let setup = setup();
    answering(&setup.chat, Answer::Allow);
    let text = run(&setup.chat, "computer_snapshot", json!({"app": "TextEdit"})).await.unwrap().text;
    assert!(text.contains("[e3] button \"Save\""), "{text}");
    assert!(text.contains("<app-data "), "{text}");
    // The nonce changes, so an app can't close the fence.
    let again = run(&setup.chat, "computer_snapshot", json!({"app": "TextEdit"})).await.unwrap().text;
    let nonce = |text: &str| text.split("<app-data ").nth(1).unwrap().split('>').next().unwrap().to_string();
    assert_ne!(nonce(&text), nonce(&again));
    // A shown ref outlines its subtree; one never shown is refused.
    let part = run(&setup.chat, "computer_snapshot", json!({"app": "TextEdit", "ref": "e3"})).await.unwrap().text;
    assert!(part.contains("Save") && !part.contains("textArea"), "{part}");
    let refused = run(&setup.chat, "computer_snapshot", json!({"app": "TextEdit", "ref": "e99"})).await.unwrap_err();
    assert!(refused.contains("hasn't been shown"), "{refused}");
    let filtered = run(&setup.chat, "computer_snapshot", json!({"app": "TextEdit", "query": "save"})).await.unwrap().text;
    assert!(filtered.contains("Save") && !filtered.contains("textArea"), "{filtered}");
    let menu = run(&setup.chat, "computer_snapshot", json!({"app": "TextEdit", "menu": []})).await.unwrap().text;
    assert!(menu.contains("menu bar") && menu.contains("- File"), "{menu}");
}

#[tokio::test]
async fn screenshots_list_elements_and_zoom_against_the_last_full_picture() {
    let setup = setup();
    answering(&setup.chat, Answer::Allow);
    let refused = run(&setup.chat, "computer_screenshot", json!({"app": "TextEdit", "region": [0, 0, 100, 100]})).await.unwrap_err();
    assert!(refused.contains("full computer_screenshot"), "{refused}");
    let shot = run(&setup.chat, "computer_screenshot", json!({"app": "TextEdit"})).await.unwrap();
    assert_eq!(shot.images.len(), 1);
    // 800×600 points fit the budget whole.
    assert!(shot.text.contains("picture 800×600"), "{}", shot.text);
    assert!(shot.text.contains("[e3] button \"Save\" at (740, 565)"), "{}", shot.text);
    assert!(setup.chat.was_shown(10, 3), "listed refs count as shown");
    let zoom = run(&setup.chat, "computer_screenshot", json!({"app": "TextEdit", "region": [0, 0, 100, 50]})).await.unwrap();
    assert!(zoom.text.contains("Coordinates still refer to the last full picture (800×600)"), "{}", zoom.text);
    // The window grows: the old picture's pixels mean nothing now.
    setup.desktop.state.lock().unwrap().windows[0].1.frame = Some(Rect::new(100.0, 100.0, 900.0, 600.0));
    let refused = run(&setup.chat, "computer_screenshot", json!({"app": "TextEdit", "region": [0, 0, 100, 50]})).await.unwrap_err();
    assert!(refused.contains("changed size"), "{refused}");
}

#[tokio::test]
async fn opening_asks_as_launching_and_keeps_files_in_the_chat_folder() {
    let setup = setup();
    setup.desktop.state.lock().unwrap().installed.push(FakeDesktop::identity("Freeform", "com.apple.freeform"));
    let asked = answering(&setup.chat, Answer::Allow);
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("board.txt"), "x").unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    let refused = call(setup.chat.clone(), "computer_open".into(), json!({"app": "Freeform", "file": outside.path()}), folder.path().to_path_buf()).await.unwrap_err();
    assert!(refused.contains("chat's folder"), "{refused}");
    let opened = call(setup.chat.clone(), "computer_open".into(), json!({"app": "Freeform", "file": "board.txt"}), folder.path().to_path_buf()).await.unwrap().text;
    assert!(opened.contains("Freeform started in the background"), "{opened}");
    assert!(matches!(&asked.lock().unwrap()[0], Ask::App { launching: true, .. }));
    let launched = setup.desktop.state.lock().unwrap().launched.clone();
    assert_eq!(launched.len(), 1);
    assert!(launched[0].1.as_ref().unwrap().ends_with("board.txt"));
}

#[tokio::test]
async fn browsers_are_look_only() {
    let setup = setup();
    answering(&setup.chat, Answer::Allow);
    assert!(run(&setup.chat, "computer_snapshot", json!({"app": "Safari"})).await.is_ok());
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("page.html"), "x").unwrap();
    let refused = call(setup.chat.clone(), "computer_open".into(), json!({"app": "Safari", "file": "page.html"}), folder.path().to_path_buf()).await.unwrap_err();
    assert!(refused.contains("look-only"), "{refused}");
}

#[tokio::test]
async fn waiting_finds_text_and_times_out() {
    let setup = setup();
    answering(&setup.chat, Answer::Allow);
    let found = run(&setup.chat, "computer_wait", json!({"app": "TextEdit", "text": "save", "role": "button"})).await.unwrap().text;
    assert!(found.contains("[e3] button \"Save\""), "{found}");
    assert!(setup.chat.was_shown(10, 3));
    let titled = run(&setup.chat, "computer_wait", json!({"app": "TextEdit", "text": "TextEdit window"})).await.unwrap().text;
    assert!(titled.contains("showed"), "{titled}");
    let refused = run(&setup.chat, "computer_wait", json!({"app": "TextEdit", "text": "Export", "timeout_ms": 300})).await.unwrap_err();
    assert!(refused.contains("hasn't shown"), "{refused}");
    let gone = run(&setup.chat, "computer_wait", json!({"app": "TextEdit", "text": "Export", "gone": true})).await.unwrap().text;
    assert!(gone.contains("is gone"), "{gone}");
    assert!(run(&setup.chat, "computer_wait", json!({"app": "TextEdit"})).await.is_err());
}

#[test]
fn only_looks_are_parallel_safe_for_plan_mode() {
    let setup = setup();
    for tool in tools(setup.chat.clone()) {
        let spec = tool.spec();
        assert!(spec.name.starts_with(PREFIX));
        assert!(NAMES.contains(&spec.name.as_str()));
        assert_eq!(spec.loading, wack_agent::Loading::Deferred);
        let changes_things = ["computer_open", "computer_act", "computer_batch", "computer_clipboard"].contains(&spec.name.as_str());
        assert_eq!(spec.parallel_safe, !changes_things, "{}", spec.name);
    }
}

mod acting {
    use super::*;
    use crate::{
        backend::{Act, Target},
        cursor,
        ladder::Reason,
    };

    fn allowed() -> Setup {
        let setup = setup();
        answering(&setup.chat, Answer::Allow);
        setup
    }

    #[tokio::test]
    async fn acting_by_ref_needs_the_ref_shown_and_reports_changes() {
        let setup = allowed();
        let refused = run(&setup.chat, "computer_act", json!({"app": "TextEdit", "action": "click", "ref": "e3", "summary": "Save"})).await.unwrap_err();
        assert!(refused.contains("hasn't been shown"), "{refused}");
        run(&setup.chat, "computer_snapshot", json!({"app": "TextEdit"})).await.unwrap();
        let done = run(&setup.chat, "computer_act", json!({"app": "TextEdit", "action": "click", "ref": "e3", "summary": "Save"})).await.unwrap().text;
        assert!(done.contains("pressed e3 in TextEdit") && done.contains("Nothing visible changed"), "{done}");
        let typed = run(&setup.chat, "computer_act", json!({"app": "TextEdit", "action": "type", "ref": "e2", "text": " world", "summary": "Type"})).await.unwrap().text;
        assert!(typed.contains("its value is now \"Hello world\""), "{typed}");
        let acts = setup.desktop.acts();
        assert!(matches!(&acts[0].2, Act::Click { target: Target::Ref(3), count: 1, .. }));
    }

    #[tokio::test]
    async fn the_cursor_shows_each_action_and_goes_when_the_run_ends() {
        let setup = allowed();
        let marker = || setup.computer.cursor().state().marker.clone();
        run(&setup.chat, "computer_snapshot", json!({"app": "TextEdit"})).await.unwrap();
        // Calls from the socket draw as if a run were going.
        run(&setup.chat, "computer_act", json!({"app": "TextEdit", "action": "click", "ref": "e3", "summary": "Save the notes"})).await.unwrap();
        let shown = marker().unwrap();
        assert_eq!((shown.target.point, shown.target.anchor.window), ((840.0, 665.0), setup.window), "the middle of Save");
        assert_eq!(shown.label(false), "Save the notes");
        assert!(shown.completed.is_some());
        // A key press has no place of its own: the marker stays where it was.
        run(&setup.chat, "computer_act", json!({"app": "TextEdit", "action": "key", "keys": "tab", "summary": "Next field"})).await.unwrap();
        let kept = marker().unwrap();
        assert_eq!((kept.target.point, kept.kind), ((840.0, 665.0), cursor::Kind::Pressing));
        // A failed action takes it down.
        setup.desktop.state.lock().unwrap().outcomes.push(Err(Reason::NoElement));
        run(&setup.chat, "computer_act", json!({"app": "TextEdit", "action": "click", "ref": "e2", "summary": "Click"})).await.unwrap_err();
        assert!(marker().is_none());
        // A real run's marker goes when the run ends.
        let running = setup.chat.begin_run();
        run(&setup.chat, "computer_act", json!({"app": "TextEdit", "action": "click", "ref": "e2", "summary": "Click the text"})).await.unwrap();
        assert_eq!(marker().unwrap().target.point, (500.0, 380.0));
        drop(running);
        assert!(marker().is_none());
    }

    #[tokio::test]
    async fn coordinates_come_from_the_last_picture_and_follow_a_moved_window() {
        let setup = allowed();
        let refused = run(&setup.chat, "computer_act", json!({"app": "TextEdit", "action": "click", "x": 740, "y": 565, "summary": "Save"})).await.unwrap_err();
        assert!(refused.contains("computer_screenshot"), "{refused}");
        run(&setup.chat, "computer_screenshot", json!({"app": "TextEdit"})).await.unwrap();
        // The window moves 50 points right: the same picture point still lands on Save.
        setup.desktop.state.lock().unwrap().windows[0].1.frame = Some(Rect::new(150.0, 100.0, 800.0, 600.0));
        for (_, tree) in setup.desktop.state.lock().unwrap().trees.iter_mut() {
            fn shift(node: &mut Node) {
                if let Some(frame) = &mut node.frame {
                    frame.x += 50.0;
                }
                node.children.iter_mut().for_each(shift);
            }
            shift(tree);
        }
        run(&setup.chat, "computer_act", json!({"app": "TextEdit", "action": "click", "x": 740, "y": 565, "summary": "Save"})).await.unwrap();
        let acts = setup.desktop.acts();
        match &acts.last().unwrap().2 {
            Act::Click { target: Target::At { point, element }, .. } => {
                assert_eq!(*point, (890.0, 665.0));
                assert_eq!(*element, Some(3));
            }
            other => panic!("{other:?}"),
        }
        let outside = run(&setup.chat, "computer_act", json!({"app": "TextEdit", "action": "click", "x": 5000, "y": 5, "summary": "x"})).await.unwrap_err();
        assert!(outside.contains("outside the picture"), "{outside}");
        setup.desktop.state.lock().unwrap().windows[0].1.frame = Some(Rect::new(150.0, 100.0, 1000.0, 600.0));
        let resized = run(&setup.chat, "computer_act", json!({"app": "TextEdit", "action": "click", "x": 740, "y": 565, "summary": "x"})).await.unwrap_err();
        assert!(resized.contains("changed size"), "{resized}");
    }

    #[tokio::test]
    async fn browsers_system_chords_and_stops_are_refused() {
        let setup = allowed();
        let refused = run(&setup.chat, "computer_act", json!({"app": "Safari", "action": "key", "keys": "enter", "summary": "x"})).await.unwrap_err();
        assert!(refused.contains("look-only"), "{refused}");
        let refused = run(&setup.chat, "computer_act", json!({"app": "TextEdit", "action": "key", "keys": "cmd+q", "summary": "Quit"})).await.unwrap_err();
        assert!(!refused.is_empty());
        assert!(setup.desktop.acts().is_empty(), "nothing reached the app");
        setup.desktop.state.lock().unwrap().outcomes.push(Err(Reason::Secure));
        let refused = run(&setup.chat, "computer_act", json!({"app": "TextEdit", "action": "type", "text": "hunter2", "summary": "x"})).await.unwrap_err();
        assert!(refused.contains("password"), "{refused}");
        setup.desktop.state.lock().unwrap().outcomes.push(Err(Reason::NoElement));
        run(&setup.chat, "computer_screenshot", json!({"app": "TextEdit"})).await.unwrap();
        let refused = run(&setup.chat, "computer_act", json!({"app": "TextEdit", "action": "click", "x": 10, "y": 10, "summary": "x"})).await.unwrap_err();
        assert!(refused.contains("background"), "{refused}");
    }

    #[tokio::test]
    async fn a_batch_runs_in_order_and_stops_at_the_first_failure() {
        let setup = allowed();
        let done = run(
            &setup.chat,
            "computer_batch",
            json!({"steps": [
                {"tool": "computer_snapshot", "input": {"app": "TextEdit", "query": "save"}},
                {"tool": "computer_act", "input": {"app": "TextEdit", "action": "click", "ref": "e3", "summary": "Save"}},
                {"tool": "computer_wait", "input": {"app": "TextEdit", "text": "save"}},
            ]}),
        )
        .await
        .unwrap()
        .text;
        assert!(done.contains("Step 1 (computer_snapshot)") && done.contains("Step 3 (computer_wait)"), "{done}");
        let failed = run(
            &setup.chat,
            "computer_batch",
            json!({"steps": [
                {"tool": "computer_act", "input": {"app": "TextEdit", "action": "click", "ref": "e3", "summary": "Save"}},
                {"tool": "computer_act", "input": {"app": "TextEdit", "action": "click", "ref": "e77", "summary": "x"}},
                {"tool": "computer_act", "input": {"app": "TextEdit", "action": "click", "ref": "e3", "summary": "Save"}},
            ]}),
        )
        .await
        .unwrap_err();
        assert!(failed.contains("Step 1 (computer_act): ") && failed.contains("Step 2 (computer_act) failed") && failed.contains("Nothing after it ran"), "{failed}");
        assert_eq!(setup.desktop.acts().len(), 2, "the third step never ran");
        let refused = run(&setup.chat, "computer_batch", json!({"steps": [{"tool": "computer_open", "input": {"app": "TextEdit"}}]})).await.unwrap_err();
        assert!(refused.contains("not “computer_open”"), "{refused}");
        let pictures: Vec<_> = (0..6).map(|_| json!({"tool": "computer_screenshot", "input": {"app": "TextEdit"}})).collect();
        let refused = run(&setup.chat, "computer_batch", json!({ "steps": pictures })).await.unwrap_err();
        assert!(refused.contains("at most 5 pictures"), "{refused}");
    }
}

#[tokio::test]
async fn the_clipboard_is_read_and_written_behind_its_own_cards() {
    let setup = setup();
    let asked = answering(&setup.chat, Answer::Allow);
    setup.desktop.state.lock().unwrap().clipboard = Some("copied text".into());
    let read = run(&setup.chat, "computer_clipboard", json!({"action": "read"})).await.unwrap().text;
    assert!(read.contains("<app-data ") && read.contains("copied text"), "{read}");
    run(&setup.chat, "computer_clipboard", json!({"action": "write", "text": "new"})).await.unwrap();
    assert_eq!(setup.desktop.state.lock().unwrap().clipboard.as_deref(), Some("new"));
    run(&setup.chat, "computer_clipboard", json!({"action": "read"})).await.unwrap();
    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 2, "one card to read, one to write");
    assert_eq!(asked[0], Ask::Clipboard { write: false });
    assert_eq!(asked[1], Ask::Clipboard { write: true });
}
