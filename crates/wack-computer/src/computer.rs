//! `Computer`, the app's one link to the Mac, and `ChatComputer`, one chat's use of it: its
//! grants, the access card it is waiting on, and what its agent has seen.
//!
//! Invariants:
//! - Only the user grants access, by answering a live card (`ChatComputer::answer` with the id the
//!   card was shown with). No tool can answer one, and a stale id grants nothing.
//! - A card waits as long as the user takes; stopping the run takes it down.
//! - An answer is checked against the policy again, so a block added while the card was up
//!   wins.
//! - A denial lasts until the user's next message (`clear_denials`); an allow until the chat ends
//!   or the user takes it back. Taking back an Always allow in Settings takes back every chat's
//!   allow of that app (`Computer::set_settings`).
//! - Saved approvals (Always allow) are written by the app, from a card's answer only, before
//!   the card is answered; this crate only reads them (`Settings::always`).
//! - One card at a time per chat: the chat's agent's computer calls run one after another
//!   (`ChatComputer::lane`), and sub-agents never reach the tools.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, RwLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::{
    backend::Desktop,
    cursor::{self, Cursor},
    geometry::Rect,
    policy::{AppIdentity, AppKey, ChatGrants, Grant, Tier},
};

/// An app the user always allows, as Settings saves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedApp {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    /// The canonical `.app` path.
    pub path: String,
}

impl SavedApp {
    pub fn of(app: &AppIdentity) -> Option<Self> {
        let key = AppKey::of(app).as_persistent()?;
        Some(Self { name: app.name.clone(), bundle_id: key.bundle_id, path: key.path? })
    }

    pub fn key(&self) -> Option<AppKey> {
        AppKey::persistent(self.bundle_id.as_deref(), &self.path)
    }
}

/// Settings › Computer use, as the engine reads it at every check.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub always: Vec<SavedApp>,
    /// Bundle ids the user never allows.
    pub never: Vec<String>,
}

#[derive(Clone)]
pub struct Computer {
    shared: Arc<Shared>,
}

struct Shared {
    desktop: Arc<dyn Desktop>,
    settings: RwLock<Settings>,
    chats: Mutex<Vec<Weak<ChatInner>>>,
    next_chat: AtomicU64,
    cursor: Cursor,
}

impl Computer {
    /// A computer whose cursor draws nothing (tests, the fake desktop).
    pub fn new(desktop: Arc<dyn Desktop>, settings: Settings) -> Self {
        Self::with_cursor(desktop, settings, Cursor::headless())
    }

    fn with_cursor(desktop: Arc<dyn Desktop>, settings: Settings, cursor: Cursor) -> Self {
        Self { shared: Arc::new(Shared { desktop, settings: RwLock::new(settings), chats: Mutex::new(Vec::new()), next_chat: AtomicU64::new(1), cursor }) }
    }

    /// The real Mac, with the agent's cursor on screen. Main thread only (it reads the keyboard
    /// layout).
    pub fn native(settings: Settings) -> Self {
        Self::with_cursor(Arc::new(crate::native::desktop::Mac::new()), settings, crate::native::cursor::cursor())
    }

    pub fn cursor(&self) -> &Cursor {
        &self.shared.cursor
    }

    /// The theme's colours for the agent's cursor; the app sends them whenever they change.
    pub fn set_cursor_appearance(&self, appearance: cursor::Appearance) {
        self.shared.cursor.set_appearance(appearance);
    }

    pub fn desktop(&self) -> &Arc<dyn Desktop> {
        &self.shared.desktop
    }

    pub fn settings(&self) -> Settings {
        self.shared.settings.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn enabled(&self) -> bool {
        self.shared.settings.read().unwrap_or_else(|e| e.into_inner()).enabled
    }

    /// Takes in new settings. Apps no longer always allowed lose every chat's allow too, so
    /// "Ask again" means the next use asks.
    pub fn set_settings(&self, settings: Settings) {
        let removed: Vec<AppKey> = {
            let old = self.shared.settings.read().unwrap_or_else(|e| e.into_inner());
            old.always.iter().filter(|app| !settings.always.contains(app)).filter_map(SavedApp::key).collect()
        };
        let turned_off = !settings.enabled;
        *self.shared.settings.write().unwrap_or_else(|e| e.into_inner()) = settings;
        if turned_off {
            self.shared.cursor.clear();
        }
        if !removed.is_empty() {
            for chat in self.chats() {
                let mut state = chat.state();
                for key in &removed {
                    state.grants.revoke(key);
                }
            }
        }
    }

    /// A new chat's use of the computer.
    pub fn chat(&self) -> ChatComputer {
        let id = self.shared.next_chat.fetch_add(1, Ordering::Relaxed);
        let inner = Arc::new(ChatInner { id, computer: self.clone(), state: Mutex::new(ChatState::default()), lane: tokio::sync::Mutex::new(()), observer: Mutex::new(None) });
        let mut chats = self.shared.chats.lock().unwrap_or_else(|e| e.into_inner());
        chats.retain(|chat| chat.strong_count() > 0);
        chats.push(Arc::downgrade(&inner));
        ChatComputer { inner }
    }

    fn chats(&self) -> Vec<ChatComputer> {
        self.shared.chats.lock().unwrap_or_else(|e| e.into_inner()).iter().filter_map(Weak::upgrade).map(|inner| ChatComputer { inner }).collect()
    }
}

/// What the agent is asking for.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// To use an app. `launching` when the app isn't running and would be opened.
    App { app: AppIdentity, tier: Tier, launching: bool, icon: Option<Vec<u8>> },
    /// To read the clipboard, or (`write`) to replace what is on it. Allow or Deny only.
    Clipboard { write: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub id: u64,
    pub ask: Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// For this chat, until it ends.
    Allow,
    /// Saved: every chat, until revoked in Settings.
    AlwaysAllow,
    /// Until the user's next message.
    Deny,
    /// Saved to the never-allow list.
    NeverAllow,
}

/// What changed, for the app to redraw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// A card went up or came down.
    Card,
}

type Observer = Arc<dyn Fn(Signal) + Send + Sync>;

/// A window's last full picture: what the model's coordinates refer to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LastShot {
    /// The picture's frame, in pixels.
    pub frame: (u32, u32),
    /// What the picture covered, in global points.
    pub bounds: Rect,
    /// The window's frame then, as Accessibility reports it: a window that has moved since keeps
    /// its picture's coordinates, shifted by the move.
    pub window: Option<Rect>,
}

/// What this chat's agent has seen of one app.
#[derive(Debug, Default)]
pub struct AppView {
    /// Refs it has been shown; any other is refused.
    pub shown: HashSet<u32>,
    /// Each window's last full picture.
    pub shots: HashMap<u32, LastShot>,
}

#[derive(Default)]
struct ChatState {
    grants: ChatGrants,
    /// The clipboard: `Some(true)` allowed for the chat, `Some(false)` denied until the user's
    /// next message.
    clipboard_read: Option<bool>,
    clipboard_write: Option<bool>,
    card: Option<(Card, oneshot::Sender<Answer>)>,
    next_card: u64,
    views: HashMap<i32, AppView>,
}

struct ChatInner {
    /// Unique among the app's chats: what the cursor knows the chat by.
    id: u64,
    computer: Computer,
    state: Mutex<ChatState>,
    lane: tokio::sync::Mutex<()>,
    observer: Mutex<Option<Observer>>,
}

/// Held while a run of a chat goes; dropping it lets go of what the run held.
pub struct RunGuard {
    cursor: Cursor,
    chat: u64,
    generation: u64,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.cursor.end_run(self.chat, self.generation);
    }
}

/// One chat's computer use. Clones share it.
#[derive(Clone)]
pub struct ChatComputer {
    inner: Arc<ChatInner>,
}

impl ChatInner {
    fn state(&self) -> std::sync::MutexGuard<'_, ChatState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl ChatComputer {
    fn state(&self) -> std::sync::MutexGuard<'_, ChatState> {
        self.inner.state()
    }

    pub fn computer(&self) -> &Computer {
        &self.inner.computer
    }

    pub fn desktop(&self) -> &Arc<dyn Desktop> {
        self.inner.computer.desktop()
    }

    /// The queue the chat's agent's calls take turns in.
    pub fn lane(&self) -> &tokio::sync::Mutex<()> {
        &self.inner.lane
    }

    /// A run of the chat starts; what it holds is let go when the guard drops (the run ends or
    /// is stopped): its cursor goes.
    pub fn begin_run(&self) -> RunGuard {
        let cursor = self.inner.computer.cursor().clone();
        let generation = cursor.start_run(self.inner.id);
        RunGuard { cursor, chat: self.inner.id, generation }
    }

    /// For calls made outside a run (the automation socket): the cursor draws as if one were
    /// going, until the chat's next run ends.
    pub fn ensure_run(&self) {
        let cursor = self.inner.computer.cursor();
        if !cursor.has_run(self.inner.id) {
            cursor.start_run(self.inner.id);
        }
    }

    /// One call's use of the agent's cursor, while the chat has a run.
    pub fn cursor(&self, cancel: &CancellationToken) -> Option<cursor::Session> {
        self.inner.computer.cursor().session(self.inner.id, cancel.clone())
    }

    pub fn set_observer(&self, observer: impl Fn(Signal) + Send + Sync + 'static) {
        *self.inner.observer.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(observer));
    }

    fn signal(&self, signal: Signal) {
        let observer = self.inner.observer.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(observer) = observer {
            observer(signal);
        }
    }

    /// The card waiting on the user, if any.
    pub fn card(&self) -> Option<Card> {
        self.state().card.as_ref().map(|(card, _)| card.clone())
    }

    /// The user answered card `id`. False when it isn't up any more (answered, or the run
    /// stopped), and then nothing is granted.
    pub fn answer(&self, id: u64, answer: Answer) -> bool {
        let sender = {
            let mut state = self.state();
            match state.card.take() {
                Some((card, sender)) if card.id == id => sender,
                other => {
                    state.card = other;
                    return false;
                }
            }
        };
        let delivered = sender.send(answer).is_ok();
        self.signal(Signal::Card);
        delivered
    }

    /// The user wrote to the agent: what they said no to may be asked again.
    pub fn clear_denials(&self) {
        let mut guard = self.state();
        let state = &mut *guard;
        state.grants.clear_denials();
        for grant in [&mut state.clipboard_read, &mut state.clipboard_write] {
            if *grant == Some(false) {
                *grant = None;
            }
        }
    }

    /// The apps this chat may use, for the compaction summary.
    pub fn allowed(&self) -> Vec<AppKey> {
        let settings = self.inner.computer.settings();
        let mut keys: Vec<AppKey> = self.state().grants.allowed().cloned().collect();
        keys.extend(settings.always.iter().filter_map(SavedApp::key));
        keys.sort_by(|a, b| a.path.cmp(&b.path));
        keys.dedup();
        keys
    }

    /// The access this chat has to `app` without asking: `Ok(Some(tier))` allowed,
    /// `Ok(None)` it would have to ask, `Err` blocked or denied (the sentence for the model).
    pub fn access_now(&self, app: &AppIdentity) -> Result<Option<Tier>, String> {
        let settings = self.inner.computer.settings();
        let own = self.desktop().own();
        let always: Vec<AppKey> = settings.always.iter().filter_map(SavedApp::key).collect();
        let (tier, grant) = self.state().grants.decide(app, &own, &settings.never, &always)?;
        match grant {
            Grant::Allowed => Ok(Some(tier)),
            Grant::Denied => Err(denied(&app.name)),
            Grant::Unknown => Ok(None),
        }
    }

    /// Access to `app`, asking the user with a card when this chat has no answer yet.
    /// `present` is false when nobody is at the window to answer.
    pub async fn access(&self, app: &AppIdentity, launching: bool, present: bool, cancel: &CancellationToken) -> Result<Tier, String> {
        let tier = match self.access_now(app)? {
            Some(tier) => return Ok(tier),
            None => crate::policy::tier(app),
        };
        if !present {
            return Err(format!("Using {} needs the user's go-ahead, and nobody is at the window to give it.", app.name));
        }
        let icon = self.desktop().icon(app).await;
        let answer = self.raise(Ask::App { app: app.clone(), tier, launching, icon }, cancel).await?;
        let key = AppKey::of(app);
        match answer {
            Answer::Allow | Answer::AlwaysAllow => {
                // A block or never-allow added while the card was up wins.
                let settings = self.inner.computer.settings();
                crate::policy::assess(app, &self.desktop().own(), &settings.never)?;
                self.state().grants.allow(key);
                Ok(tier)
            }
            Answer::Deny => {
                self.state().grants.deny(key);
                Err(denied(&app.name))
            }
            Answer::NeverAllow => {
                self.state().grants.deny(key);
                Err(format!("The user never allows computer use in {}. Don't ask for it again; do the task another way or tell the user what you'd need.", app.name))
            }
        }
    }

    /// Shows `ask` on a card and waits for the answer; the run stopping takes it down.
    async fn raise(&self, ask: Ask, cancel: &CancellationToken) -> Result<Answer, String> {
        let (sender, receiver) = oneshot::channel();
        let id = {
            let mut state = self.state();
            state.next_card += 1;
            let id = state.next_card;
            state.card = Some((Card { id, ask }, sender));
            id
        };
        self.signal(Signal::Card);
        let answer = tokio::select! {
            answer = receiver => answer.ok(),
            () = cancel.cancelled() => None,
        };
        answer.ok_or_else(|| {
            // Stopped, or the card was dropped: take it down if it's still ours.
            let mut state = self.state();
            if state.card.as_ref().is_some_and(|(card, _)| card.id == id) {
                state.card = None;
                drop(state);
                self.signal(Signal::Card);
            }
            "The run was stopped before the user answered.".to_string()
        })
    }

    /// Reading (or, with `write`, replacing) the clipboard, asking the user the first time.
    pub async fn clipboard_access(&self, write: bool, present: bool, cancel: &CancellationToken) -> Result<(), String> {
        let current = {
            let state = self.state();
            if write { state.clipboard_write } else { state.clipboard_read }
        };
        let what = if write { "copy to the clipboard" } else { "read the clipboard" };
        match current {
            Some(true) => return Ok(()),
            Some(false) => return Err(format!("The user said no to letting you {what} for now. Don't ask again in this turn.")),
            None if !present => return Err(format!("Nobody is at the window to let you {what}.")),
            None => {}
        }
        let answer = self.raise(Ask::Clipboard { write }, cancel).await?;
        let allowed = matches!(answer, Answer::Allow | Answer::AlwaysAllow);
        {
            let mut state = self.state();
            let grant = if write { &mut state.clipboard_write } else { &mut state.clipboard_read };
            *grant = Some(allowed);
        }
        if allowed { Ok(()) } else { Err(format!("The user said no to letting you {what} for now. Don't ask again in this turn.")) }
    }

    /// Records refs the model was shown.
    pub fn saw(&self, pid: i32, refs: &[u32]) {
        self.state().views.entry(pid).or_default().shown.extend(refs.iter().copied());
    }

    pub fn was_shown(&self, pid: i32, reference: u32) -> bool {
        self.state().views.get(&pid).is_some_and(|view| view.shown.contains(&reference))
    }

    pub fn remember_shot(&self, pid: i32, window: u32, shot: LastShot) {
        self.state().views.entry(pid).or_default().shots.insert(window, shot);
    }

    pub fn last_shot(&self, pid: i32, window: u32) -> Option<LastShot> {
        self.state().views.get(&pid).and_then(|view| view.shots.get(&window).copied())
    }
}

fn denied(name: &str) -> String {
    format!(
        "The user said no to computer use in {name} for now. Don't ask again in this turn; if you need it, say why in your reply, and the user can allow it once they answer."
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{backend::fake::FakeDesktop, outline::Node};

    fn computer() -> (Computer, FakeDesktop) {
        let desktop = FakeDesktop::new();
        desktop.add_app(10, "TextEdit", "com.apple.TextEdit", Node::default());
        (Computer::new(Arc::new(desktop.clone()), Settings { enabled: true, ..Settings::default() }), desktop)
    }

    fn textedit() -> AppIdentity {
        let mut app = FakeDesktop::identity("TextEdit", "com.apple.TextEdit");
        app.pid = Some(10);
        app
    }

    /// Answers the card as soon as it shows.
    fn answer_when_asked(chat: &ChatComputer, answer: Answer) {
        let answering = chat.clone();
        chat.set_observer(move |_| {
            if let Some(card) = answering.card() {
                answering.answer(card.id, answer);
            }
        });
    }

    #[tokio::test]
    async fn allow_lasts_for_the_chat_and_deny_until_the_next_message() {
        let (computer, _) = computer();
        let chat = computer.chat();
        let cancel = CancellationToken::new();
        answer_when_asked(&chat, Answer::Deny);
        let refused = chat.access(&textedit(), false, true, &cancel).await.unwrap_err();
        assert!(refused.contains("said no"), "{refused}");
        // Denied: no second card in the same turn.
        assert!(chat.access_now(&textedit()).is_err());
        chat.clear_denials();
        assert_eq!(chat.access_now(&textedit()), Ok(None));
        answer_when_asked(&chat, Answer::Allow);
        assert_eq!(chat.access(&textedit(), false, true, &cancel).await, Ok(Tier::Full));
        assert_eq!(chat.access_now(&textedit()), Ok(Some(Tier::Full)));
        // Another chat still has to ask.
        assert_eq!(computer.chat().access_now(&textedit()), Ok(None));
    }

    #[tokio::test]
    async fn a_stale_card_grants_nothing() {
        let (computer, _) = computer();
        let chat = computer.chat();
        let cancel = CancellationToken::new();
        let asking = chat.clone();
        let waiting = tokio::spawn(async move { asking.access(&textedit(), false, true, &cancel).await });
        while chat.card().is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let id = chat.card().unwrap().id;
        assert!(!chat.answer(id + 1, Answer::Allow), "a wrong id must not answer");
        assert!(chat.card().is_some(), "the card stays up");
        assert!(chat.answer(id, Answer::Allow));
        assert!(!chat.answer(id, Answer::Allow), "answered once");
        assert_eq!(waiting.await.unwrap(), Ok(Tier::Full));
    }

    #[tokio::test]
    async fn stopping_takes_the_card_down() {
        let (computer, _) = computer();
        let chat = computer.chat();
        let cancel = CancellationToken::new();
        let asking = chat.clone();
        let stop = cancel.clone();
        let waiting = tokio::spawn(async move { asking.access(&textedit(), false, true, &stop).await });
        while chat.card().is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        cancel.cancel();
        assert!(waiting.await.unwrap().is_err());
        assert!(chat.card().is_none());
    }

    #[tokio::test]
    async fn a_block_added_while_asking_wins() {
        let (computer, _) = computer();
        let chat = computer.chat();
        let cancel = CancellationToken::new();
        let settings_side = computer.clone();
        let answering = chat.clone();
        chat.set_observer(move |_| {
            if let Some(card) = answering.card() {
                settings_side.set_settings(Settings { enabled: true, never: vec!["com.apple.TextEdit".into()], ..Settings::default() });
                answering.answer(card.id, Answer::Allow);
            }
        });
        let refused = chat.access(&textedit(), false, true, &cancel).await.unwrap_err();
        assert!(refused.contains("never allows"), "{refused}");
    }

    #[tokio::test]
    async fn nobody_there_means_no_card() {
        let (computer, _) = computer();
        let chat = computer.chat();
        assert!(chat.access(&textedit(), false, false, &CancellationToken::new()).await.is_err());
        assert!(chat.card().is_none());
    }

    #[tokio::test]
    async fn blocked_apps_never_ask() {
        let (computer, _) = computer();
        let chat = computer.chat();
        let terminal = FakeDesktop::identity("Terminal", "com.apple.Terminal");
        let refused = chat.access(&terminal, false, true, &CancellationToken::new()).await.unwrap_err();
        assert!(refused.contains("never controls"), "{refused}");
        assert!(chat.card().is_none());
    }

    #[test]
    fn always_allow_comes_from_settings_and_asking_again_revokes_it_everywhere() {
        let (computer, _) = computer();
        let chat = computer.chat();
        let saved = SavedApp::of(&textedit()).unwrap();
        computer.set_settings(Settings { enabled: true, always: vec![saved.clone()], ..Settings::default() });
        assert_eq!(chat.access_now(&textedit()), Ok(Some(Tier::Full)));
        // The card's answer also allowed it for this chat.
        chat.state().grants.allow(AppKey::of(&textedit()));
        computer.set_settings(Settings { enabled: true, ..Settings::default() });
        assert_eq!(chat.access_now(&textedit()), Ok(None));
    }

    #[tokio::test]
    async fn the_clipboard_asks_once_for_reading_and_once_for_writing() {
        let (computer, _) = computer();
        let chat = computer.chat();
        let cancel = CancellationToken::new();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let seen = asked.clone();
        let answering = chat.clone();
        chat.set_observer(move |_| {
            if let Some(card) = answering.card() {
                let answer = if matches!(card.ask, Ask::Clipboard { write: true }) { Answer::Deny } else { Answer::Allow };
                seen.lock().unwrap().push(card.ask.clone());
                answering.answer(card.id, answer);
            }
        });
        assert!(chat.clipboard_access(false, true, &cancel).await.is_ok());
        assert!(chat.clipboard_access(false, true, &cancel).await.is_ok());
        assert!(chat.clipboard_access(true, true, &cancel).await.is_err());
        assert!(chat.clipboard_access(true, true, &cancel).await.is_err(), "denied without a second card");
        assert_eq!(asked.lock().unwrap().len(), 2);
        chat.clear_denials();
        assert!(chat.clipboard_access(true, true, &cancel).await.is_err());
        assert_eq!(asked.lock().unwrap().len(), 3, "asked again after the user's message");
    }

    #[test]
    fn refs_must_have_been_shown() {
        let (computer, _) = computer();
        let chat = computer.chat();
        chat.saw(10, &[3, 4]);
        assert!(chat.was_shown(10, 3));
        assert!(!chat.was_shown(10, 5));
        assert!(!chat.was_shown(11, 3));
    }
}
