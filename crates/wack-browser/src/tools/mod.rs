//! The agent's browser tools (`browser_*`): they drive a chat's tabs over in-process DevTools.
//!
//! Every tool is deferred, so its rules live in its description (a deferred tool's guidelines
//! never reach the prompt). They share one core, here:
//! - **Lanes.** Each agent (the chat's, each sub-agent's) has a queue (`ChatTabs::lane`); every
//!   call takes it as its first step, so calls the model made side by side still run in its
//!   order (a `browser_navigate` and a `browser_snapshot` in one reply read the new page).
//! - **Which tab.** A call names a tab, or works in the agent's current one: the tab it last
//!   chose or opened. Sub-agents get tabs of their own (`ToolContext::caller`).
//! - **Guards.** A tab the user has taken control of is left alone, re-checked while a call
//!   waits. A tab the user opened is used only while its address passes the agent's rule, and
//!   using it makes the agent its driver, so its navigations and requests pass the rule too.
//! - **Dialogs.** A page blocked on `alert()` answers no DevTools call, so every call races a
//!   dialog opening (`Call::Dialog`): a click that opens one returns, and says so.
//! - **What changed.** An action's answer is short: what changed (address, title, a new page,
//!   which resets refs, a dialog, new console errors), never the page itself.
//!
//! Plan mode and read-only sub-agents get the tools marked `parallel_safe`: they look, wait and
//! move between pages but click, type, run scripts or answer dialogs never.
//!
//! Invariants:
//! - Refs are DOM nodes (`backendNodeId`), kept per page: the same node keeps its ref across
//!   snapshots, a pane resize never changes one, and a new page resets them.
//! - Coordinates refer to the last full screenshot, and an action at a coordinate is refused once
//!   the page's size has changed since it was taken.
//! - Scripts (`browser_eval`) and raw response bodies only on pages on this Mac
//!   (`policy::is_this_mac`, the owner's call): on the open web they could read a site's cookies
//!   and storage.

mod input;
mod shot;
mod snapshot;
mod tool;

use std::{collections::HashMap, sync::Arc, time::Duration};

use serde_json::{Value, json};
use wack_agent::{Tool, ToolContext};

use crate::{
    policy,
    tabs::{ChatTabs, Owner, Tab, TabId},
};

pub use tool::{BATCH, NAMES};

/// The browser tools for a chat's tabs. Nothing when the build has no browser.
pub fn tools(tabs: ChatTabs) -> Vec<Arc<dyn Tool>> {
    if !tabs.available() {
        return Vec::new();
    }
    let browser = Arc::new(Browser { tabs });
    tool::Kind::ALL.iter().map(|kind| Arc::new(tool::BrowserTool { kind: *kind, browser: browser.clone() }) as Arc<dyn Tool>).collect()
}

/// Calls the tool `name` as a run would, outside one: for the dev app's automation socket.
pub async fn call(tabs: ChatTabs, name: String, input: Value, cwd: std::path::PathBuf, caller: Option<String>) -> Result<wack_agent::ToolOutput, String> {
    let tool = tools(tabs).into_iter().find(|tool| tool.spec().name == name).ok_or_else(|| format!("There is no browser tool {name}."))?;
    let mut context = ToolContext::detached(cwd);
    context.caller = caller;
    tool.run(input, &context).await
}

/// What the tools know of one tab's page.
#[derive(Default)]
pub struct AgentView {
    /// The page these refs belong to (`TabState::page`).
    page: u64,
    /// `ref_N` → the node it names.
    refs: HashMap<u32, Node>,
    /// The ref each node already has, so a node keeps it across snapshots.
    by_node: HashMap<(String, i64), u32>,
    next: u32,
    /// The last full screenshot's frame, for coordinates.
    frame: Option<Frame>,
}

#[derive(Clone, Debug, PartialEq)]
struct Node {
    /// The frame the node is in; empty for the main frame.
    frame: String,
    backend: i64,
}

/// A screenshot's size against the page's, in CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    css: (f64, f64),
    image: (f64, f64),
}

impl AgentView {
    /// Forgets refs from an earlier page.
    fn on_page(&mut self, page: u64) {
        if self.page != page {
            self.page = page;
            self.refs.clear();
            self.by_node.clear();
            self.next = 0;
        }
    }

    fn ref_for(&mut self, node: Node) -> u32 {
        let key = (node.frame.clone(), node.backend);
        if let Some(number) = self.by_node.get(&key) {
            return *number;
        }
        self.next += 1;
        self.by_node.insert(key, self.next);
        self.refs.insert(self.next, node);
        self.next
    }
}

pub(crate) struct Browser {
    tabs: ChatTabs,
}

/// How long one DevTools call may take before the tool gives up on the page.
const CALL_LIMIT: Duration = Duration::from_secs(20);

/// Why a DevTools call didn't answer.
#[derive(Debug, PartialEq)]
enum Call {
    /// The page opened a JavaScript dialog: it answers nothing until the dialog is answered.
    Dialog,
    Failed(String),
}

impl From<Call> for String {
    fn from(call: Call) -> Self {
        match call {
            Call::Dialog => dialog_open_message(),
            Call::Failed(message) => message,
        }
    }
}

fn dialog_open_message() -> String {
    "The page has a dialog open and does nothing else until it is answered: use browser_dialog.".into()
}

/// Shows the pane's ring around the page while an agent's tool works in it.
struct Acting(Arc<Tab>);

impl Acting {
    fn new(tab: &Arc<Tab>) -> Self {
        tab.update(|state| state.agent_acting = true);
        Self(tab.clone())
    }
}

impl Drop for Acting {
    fn drop(&mut self) {
        self.0.update(|state| state.agent_acting = false);
    }
}

/// Where a tab stood before an action, to say what the action changed.
struct Mark {
    url: String,
    title: String,
    page: u64,
    console: u64,
    dialog: bool,
}

pub(crate) fn owner_of(context: &ToolContext<'_>) -> Owner {
    context.caller.as_ref().map_or(Owner::Agent, |id| Owner::SubAgent(id.clone()))
}

fn owner_word(tab: &Tab, owner: &Owner) -> &'static str {
    match &tab.owner {
        Owner::User => "the user's",
        found if found == owner => "yours",
        Owner::Agent => "the agent's",
        Owner::SubAgent(_) => "a sub-agent's",
    }
}

impl Browser {
    /// The tab a call names, or the agent's current one.
    fn resolve(&self, input: &Value, owner: &Owner) -> Result<Arc<Tab>, String> {
        match input.get("tab").and_then(Value::as_u64) {
            Some(id) => self.tabs.get(id).ok_or_else(|| format!("There is no tab {id}. browser_tabs lists the open tabs.")),
            None => self.tabs.current_of(owner).ok_or_else(|| {
                "You have no tab open yet: open a page with browser_navigate (url), which opens a tab for it.".to_string()
            }),
        }
    }

    /// Makes `tab` one the agent may use now, and the agent its driver.
    async fn enter(&self, tab: &Arc<Tab>) -> Result<(), String> {
        if tab.user_has_control() {
            return Err(format!("The user has taken control of tab {}: leave it alone until they hand it back.", tab.id));
        }
        // A page the user opened is the agent's only while its address is one the agent could
        // have opened itself (not the user's router, say).
        if !tab.agent_driving() {
            let url = tab.state().url.clone();
            if !url.is_empty() && url != "about:blank" {
                policy::check_resolved(&url).await.map_err(|why| format!("Tab {} shows a page the agent may not use: {why}", tab.id))?;
            }
            tab.set_agent_driving(true);
        }
        self.tabs.ensure_live(tab.id).await.map(|_| ())
    }

    /// One DevTools call, which gives way to a dialog opening and to a page that hangs.
    async fn cdp(&self, tab: &Arc<Tab>, method: &str, params: Value) -> Result<Value, Call> {
        if tab.state().dialog.is_some() {
            return Err(Call::Dialog);
        }
        let call = self.tabs.cdp(tab.id, method, params);
        let dialog = async {
            loop {
                tokio::time::sleep(Duration::from_millis(40)).await;
                if tab.state().dialog.is_some() {
                    return;
                }
            }
        };
        tokio::select! {
            answer = call => answer.map_err(Call::Failed),
            () = dialog => Err(Call::Dialog),
            () = tokio::time::sleep(CALL_LIMIT) => Err(Call::Failed(format!("The page did not answer within {} seconds.", CALL_LIMIT.as_secs()))),
        }
    }

    /// Runs `expression` in the page and returns its value.
    async fn evaluate(&self, tab: &Arc<Tab>, expression: &str) -> Result<Value, Call> {
        let answer = self
            .cdp(tab, "Runtime.evaluate", json!({ "expression": expression, "returnByValue": true, "awaitPromise": true, "userGesture": true }))
            .await?;
        if let Some(details) = answer.get("exceptionDetails") {
            let text = details["exception"]["description"].as_str().or_else(|| details["text"].as_str()).unwrap_or("The script threw.");
            return Err(Call::Failed(text.to_string()));
        }
        Ok(answer["result"]["value"].clone())
    }

    /// The page's size in CSS pixels.
    async fn page_size(&self, tab: &Arc<Tab>) -> Result<(f64, f64), Call> {
        let size = self.evaluate(tab, "[innerWidth, innerHeight]").await?;
        Ok((size[0].as_f64().unwrap_or(0.), size[1].as_f64().unwrap_or(0.)))
    }

    fn mark(tab: &Tab) -> Mark {
        let state = tab.state();
        Mark { url: state.url.clone(), title: state.title.clone(), page: state.page, console: state.console.last_seq(), dialog: state.dialog.is_some() }
    }

    /// Waits a moment for what an action set off: a navigation it started, to its load.
    async fn settle(&self, tab: &Arc<Tab>, mark: &Mark, context: &ToolContext<'_>) {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let navigating = {
            let state = tab.state();
            state.page != mark.page || state.loading
        };
        if navigating {
            let _ = self.until(tab, context, Duration::from_secs(10), |tab| !tab.state().loading).await;
        }
    }

    /// Polls `done` until it holds, the time runs out (false), or the call must stop (an error).
    async fn until(&self, tab: &Arc<Tab>, context: &ToolContext<'_>, limit: Duration, mut done: impl FnMut(&Tab) -> bool) -> Result<bool, String> {
        let start = std::time::Instant::now();
        loop {
            if done(tab) {
                return Ok(true);
            }
            if context.cancel.is_cancelled() {
                return Err("Stopped.".into());
            }
            if tab.user_has_control() {
                return Err(format!("The user took control of tab {} while you waited.", tab.id));
            }
            if tab.state().dialog.is_some() {
                return Ok(false);
            }
            if start.elapsed() >= limit {
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// No request on the current page has been open for half a second (long-lived streams aside).
    async fn network_idle(&self, tab: &Arc<Tab>, context: &ToolContext<'_>, limit: Duration) -> Result<bool, String> {
        let mut quiet_since: Option<std::time::Instant> = None;
        self.until(tab, context, limit, |tab| {
            let state = tab.state();
            let busy = state
                .network
                .iter()
                .any(|(_, entry)| entry.page == state.page && !entry.finished && !matches!(entry.kind.as_str(), "WebSocket" | "EventSource"));
            if busy || state.loading {
                quiet_since = None;
                return false;
            }
            let since = *quiet_since.get_or_insert_with(std::time::Instant::now);
            since.elapsed() >= Duration::from_millis(500)
        })
        .await
    }

    /// The short account of what an action changed.
    fn changes(tab: &Tab, mark: &Mark, owner: &Owner) -> String {
        let state = tab.state();
        let mut lines = vec![tab_line(tab, &state, owner)];
        if state.page != mark.page {
            lines.push("A new page loaded: refs from earlier snapshots no longer apply.".into());
        } else if state.url != mark.url {
            lines.push("The address changed (same page).".into());
        }
        if state.title != mark.title && state.page == mark.page {
            lines.push(format!("The title is now \"{}\".", state.title));
        }
        if let Some((url, why)) = &state.refused {
            lines.push(format!("A navigation to {url} was refused: {why}"));
        }
        if let Some((url, error)) = &state.load_error {
            lines.push(format!("{url} failed to load: {error}"));
        }
        if let Some(dialog) = &state.dialog
            && !mark.dialog
        {
            lines.push(format!("The page opened {} dialog: \"{}\". Answer it with browser_dialog.", dialog_word(dialog.kind), dialog.message));
        }
        let errors = state.errors_since(mark.console);
        if errors > 0 {
            lines.push(format!("{errors} new console error{}: browser_console shows them.", if errors == 1 { "" } else { "s" }));
        }
        lines.join("\n")
    }

    /// The page in `tab`'s view, with refs reset if the page changed.
    fn view(tab: &Tab) -> std::sync::MutexGuard<'_, AgentView> {
        let page = tab.state().page;
        let mut view = tab.agent.lock().unwrap_or_else(|e| e.into_inner());
        view.on_page(page);
        view
    }

    fn owned_by(tab: &Tab, owner: &Owner) -> bool {
        &tab.owner == owner
    }

    fn tab_ids(&self) -> Vec<TabId> {
        self.tabs.tabs().iter().map(|tab| tab.id).collect()
    }
}

fn dialog_word(kind: crate::state::DialogKind) -> &'static str {
    match kind {
        crate::state::DialogKind::Alert => "an alert",
        crate::state::DialogKind::Confirm => "a confirm",
        crate::state::DialogKind::Prompt => "a prompt",
        crate::state::DialogKind::BeforeUnload => "a leave-page",
    }
}

/// One line naming a tab: its number, whose it is, its title and address.
fn tab_line(tab: &Tab, state: &crate::state::TabState, owner: &Owner) -> String {
    let mut line = format!("Tab {} ({}): \"{}\" {}", tab.id, owner_word(tab, owner), state.title, if state.url.is_empty() { "about:blank" } else { &state.url });
    if state.loading {
        line.push_str(" [loading]");
    }
    if tab.user_has_control() {
        line.push_str(" [the user has control]");
    }
    line
}

#[cfg(test)]
mod tests;
