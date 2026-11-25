//! The agent's computer tools (`computer_*`): they work inside the user's other Mac apps, in the
//! background, through one chat's `ChatComputer`.
//!
//! Every call, in order:
//! 1. **Only the chat's own agent.** A sub-agent's call (`ToolContext::caller`) is refused: the
//!    user allowed apps for the conversation they are having, not for work they never see. (The
//!    tools are also registered after the sub-agents' share and copy children deny the prefix:
//!    `wack_core::run`, `wack_agent::agents::child`.)
//! 2. **On, and permitted.** Off in Settings › Computer use, or missing the macOS permission the
//!    call needs, refuses with where the user turns it on.
//! 3. **One at a time.** Calls take the chat's lane first, so ones made side by side run in the
//!    order the model made them, and at most one access card is ever up.
//! 4. **Access.** The app is resolved and checked (`ChatComputer::access`), asking the user on
//!    first use.
//!
//! Invariants:
//! - Text from an app reaches the model between `<app-data>` markers carrying a fresh nonce, so
//!   an app can't forge their end.
//! - A ref the model was never shown in this chat is refused (`ChatComputer::was_shown`).
//! - A region zooms against the window's last full picture, and is refused once the window has
//!   changed size since.
//! - Windows of apps the chat may not use are never listed (their titles are the user's).

mod act;
mod clipboard;
mod look;
mod spec;
#[cfg(test)]
mod tests;

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use serde_json::Value;
use wack_agent::{Loading, Tool, ToolContext, ToolOutput, ToolSpec};

use crate::{
    backend::{AppInfo, Found, WindowInfo},
    computer::ChatComputer,
    policy::Tier,
};

pub use spec::{Kind, UNTRUSTED};

/// Every computer tool's name.
pub const NAMES: [&str; 8] = ["computer_apps", "computer_open", "computer_snapshot", "computer_screenshot", "computer_wait", "computer_act", "computer_batch", "computer_clipboard"];

/// The prefix every computer tool's name starts with (copy children deny it).
pub const PREFIX: &str = "computer_";

/// The computer tools for a chat.
pub fn tools(chat: ChatComputer) -> Vec<Arc<dyn Tool>> {
    Kind::ALL.iter().map(|kind| Arc::new(ComputerTool { kind: *kind, chat: chat.clone(), present: None }) as Arc<dyn Tool>).collect()
}

/// Runs one computer tool outside a run (the automation socket), as if the user were at the
/// window, so cards show and wait for a click.
pub async fn call(chat: ChatComputer, name: String, input: Value, cwd: PathBuf) -> Result<ToolOutput, String> {
    let kind = Kind::from_name(&name).ok_or_else(|| format!("There is no computer tool {name}."))?;
    chat.ensure_run();
    let tool = ComputerTool { kind, chat, present: Some(true) };
    tool.run(input, &ToolContext::detached(cwd)).await
}

pub(crate) struct ComputerTool {
    pub(crate) kind: Kind,
    pub(crate) chat: ChatComputer,
    /// Overrides whether someone can answer a card (the socket's calls).
    pub(crate) present: Option<bool>,
}

#[async_trait]
impl Tool for ComputerTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.kind.name().to_string(),
            description: self.kind.description(),
            input_schema: self.kind.schema(),
            loading: Loading::Deferred,
            parallel_safe: self.kind.looks_only(),
            hint: self.kind.hint().to_string(),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        if context.caller.is_some() {
            return Err("Computer use is for the chat's own agent: sub-agents can't use the user's apps. Ask in your result for what you need done.".into());
        }
        if !self.chat.computer().enabled() {
            return Err("Computer use is off. The user can turn it on in Settings › Computer use.".into());
        }
        let permissions = self.chat.desktop().permissions();
        // A batch may take pictures; it is refused at the step if the permission is missing.
        let needs_screen = self.kind == Kind::Screenshot;
        if !matches!(self.kind, Kind::Apps | Kind::Clipboard) && !permissions.accessibility {
            return Err("WackCode doesn't have the Accessibility permission computer use needs. The user can allow it in Settings › Computer use.".into());
        }
        if needs_screen && !permissions.screen_recording {
            return Err("WackCode doesn't have the Screen Recording permission pictures need. The user can allow it in Settings › Computer use; computer_snapshot works without it.".into());
        }
        let _turn = self.chat.lane().lock().await;
        let call = Call { chat: &self.chat, context, present: self.present.unwrap_or_else(|| context.user.present()) };
        match self.kind {
            Kind::Apps => look::apps(&call, input).await,
            Kind::Open => look::open(&call, input).await,
            Kind::Snapshot => look::snapshot(&call, input).await,
            Kind::Screenshot => look::screenshot(&call, input).await,
            Kind::Wait => look::wait(&call, input).await,
            Kind::Act => act::act(&call, input).await,
            Kind::Batch => act::batch(&call, input).await,
            Kind::Clipboard => clipboard::clipboard(&call, input).await,
        }
    }
}

/// One call's view of the chat.
pub(crate) struct Call<'a> {
    pub chat: &'a ChatComputer,
    pub context: &'a ToolContext<'a>,
    pub present: bool,
}

impl Call<'_> {
    /// A running app the chat may use, asking the user on first use.
    pub async fn running_app(&self, query: &str) -> Result<(AppInfo, Tier), String> {
        let info = match self.chat.desktop().resolve(query).await? {
            Found::Running(info) => info,
            Found::Installed(app) => return Err(format!("{} isn't running. computer_open starts it in the background.", app.name)),
        };
        if info.pid == self.chat.desktop().own().pid {
            return Err("WackCode can't control or look at any WackCode window.".into());
        }
        let tier = self.chat.access(&info.identity, false, self.present, &self.context.cancel).await?;
        Ok((info, tier))
    }

    /// The window a call names, or the app's main one.
    pub async fn window(&self, app: &AppInfo, requested: Option<u32>) -> Result<WindowInfo, String> {
        let windows = self.chat.desktop().windows(app.pid).await?;
        let name = &app.identity.name;
        if let Some(id) = requested {
            return windows.into_iter().find(|window| window.id == id).ok_or_else(|| format!("{name} has no window {id}. computer_apps lists its windows."));
        }
        let usable = |window: &&WindowInfo| !window.minimized;
        windows
            .iter()
            .filter(usable)
            .find(|window| window.main && window.on_screen)
            .or_else(|| windows.iter().filter(usable).find(|window| window.on_screen))
            .or_else(|| windows.iter().find(usable))
            .or_else(|| windows.first())
            .cloned()
            .ok_or_else(|| format!("{name} has no open window. computer_open with a file, or a menu command once actions are allowed, can open one."))
    }
}

/// Text from an app, fenced so the model reads it as data: the nonce is new each time, so the
/// app can't write the closing marker.
pub(crate) fn app_data(text: &str) -> String {
    let nonce = nonce();
    format!("<app-data {nonce}>\n{}\n</app-data {nonce}>", text.trim_end())
}

fn nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or(0));
    format!("{:012x}", hasher.finish() & 0xFFFF_FFFF_FFFF)
}

/// `window 41792 "Untitled" (656×422)`.
pub(crate) fn window_words(window: &WindowInfo) -> String {
    let mut text = format!("window {} \"{}\"", window.id, crate::outline::clip(&window.title, 80));
    if let Some(frame) = window.frame {
        text.push_str(&format!(" ({:.0}×{:.0})", frame.width, frame.height));
    }
    if window.minimized {
        text.push_str(", minimized");
    } else if !window.on_screen {
        text.push_str(", hidden or in another Space");
    }
    if window.kind.as_deref().is_some_and(|kind| kind != "AXStandardWindow") {
        text.push_str(&format!(", {}", crate::outline::display_role(window.kind.as_deref().unwrap_or_default(), None)));
    }
    text
}
