//! A chat's browser tabs: what the Browser pane shows and what the agent's tools drive.
//!
//! `ChatTabs` is owned by the chat, like its shell `Tasks` and its `Agents`: it is made when the
//! chat loads, handed to every run and to the pane, and outlives both. The pane only views it.
//!
//! Invariants (version 2's browser broke each of these):
//! - A tab's page lives in the engine and is created or destroyed only here: by `open`, `close`,
//!   or the last handle going away (the chat deleted). Nothing a view does recreates it.
//! - Opening the address a tab already shows is not a reload (`navigate`).
//! - Tab ids are unique for the life of the app and never reused.
//! - A tab has at most one page: making it is exclusive per tab (`Tab::making`), so a view that
//!   asks for a page while `open` is still making one waits for it rather than making a second.
//! - Saved tabs come back lazily: `restore` lists them, and the engine makes a page only when one
//!   is shown or used (`ensure_live`), so reopening a chat starts no browser on its own.
//! - The engine is reached only through `Backend`, so every rule here is tested without CEF.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::TabState;

pub type TabId = u64;
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Who opened a tab. Sub-agents' tabs carry their id, so one cannot close another's.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Owner {
    User,
    Agent,
    SubAgent(String),
}

/// A navigation request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nav {
    Open(String),
    Back,
    Forward,
    Reload,
    Stop,
}

/// The engine, as `ChatTabs` sees it. `cef.rs` implements it; tests use a fake.
pub trait Backend: Send + Sync {
    /// Makes the page for `tab` at `url`, starting the engine if it is not running yet.
    fn create(&self, tab: Arc<Tab>, url: String) -> BoxFuture<Result<(), String>>;
    /// Closes the page. Nothing happens for a tab the engine never made.
    fn close(&self, id: TabId);
    fn navigate(&self, id: TabId, nav: Nav) -> BoxFuture<Result<(), String>>;
    /// One DevTools method on the tab's page; the result object, or the error as a sentence.
    fn cdp(&self, id: TabId, method: String, params: Value) -> BoxFuture<Result<Value, String>>;
    /// Answers the tab's open JavaScript dialog.
    fn answer_dialog(
        &self,
        id: TabId,
        accept: bool,
        text: Option<String>,
    ) -> BoxFuture<Result<(), String>>;
}

type Notify = Arc<dyn Fn(TabId) + Send + Sync>;
/// Makes a tab for a popup the page of tab `.1` opened, in the same chat, with owner `.0`.
type PopupSink = Arc<dyn Fn(Owner, TabId) -> Arc<Tab> + Send + Sync>;

/// One tab: its state, and whether the engine has a page for it.
pub struct Tab {
    pub id: TabId,
    pub owner: Owner,
    state: Mutex<TabState>,
    live: AtomicBool,
    /// Held while the engine makes this tab's page.
    making: tokio::sync::Mutex<()>,
    user_control: AtomicBool,
    agent_driving: AtomicBool,
    notify: Mutex<Option<Notify>>,
    popups: Mutex<Option<PopupSink>>,
}

impl Tab {
    fn new(owner: Owner) -> Arc<Self> {
        let agents = !matches!(owner, Owner::User);
        Arc::new(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            owner,
            state: Mutex::new(TabState::default()),
            live: AtomicBool::new(false),
            making: tokio::sync::Mutex::new(()),
            user_control: AtomicBool::new(false),
            // An agent's tab is the agent's from its first load, so the address policy covers it.
            agent_driving: AtomicBool::new(agents),
            notify: Mutex::new(None),
            popups: Mutex::new(None),
        })
    }

    pub fn state(&self) -> MutexGuard<'_, TabState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Changes the state and tells the chat's observer.
    pub fn update(&self, change: impl FnOnce(&mut TabState)) {
        change(&mut self.state());
        let notify = self
            .notify
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(notify) = notify {
            notify(self.id);
        }
    }

    /// The engine has a page for this tab.
    pub fn is_live(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }

    /// Called by the engine when it makes or loses the page.
    pub fn set_live(&self, live: bool) {
        self.live.store(live, Ordering::Release);
        self.update(|_| {});
    }

    /// The user took control of this tab in the pane: the agent's tools leave it alone.
    pub fn user_has_control(&self) -> bool {
        self.user_control.load(Ordering::Acquire)
    }

    pub fn set_user_control(&self, on: bool) {
        self.user_control.store(on, Ordering::Release);
        self.update(|_| {});
    }

    /// The agent's tools last drove this tab, so its navigations (redirects and script
    /// navigations included) must pass the address policy. The user acting in the pane clears it.
    pub fn agent_driving(&self) -> bool {
        self.agent_driving.load(Ordering::Acquire)
    }

    pub fn set_agent_driving(&self, on: bool) {
        self.agent_driving.store(on, Ordering::Release);
    }

    /// A tab for a popup this tab's page opened, already listed in the chat.
    pub fn spawn_popup(&self) -> Option<Arc<Tab>> {
        let sink = self
            .popups
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()?;
        let child = sink(self.owner.clone(), self.id);
        child.set_agent_driving(self.agent_driving());
        Some(child)
    }
}

/// What changed, for the chat's observer (the UI redraws on it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Opened(TabId),
    Changed(TabId),
    Closed(TabId),
}

type Observer = Arc<dyn Fn(Signal) + Send + Sync>;

/// A tab as it is saved with the chat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTab {
    pub url: String,
    #[serde(default)]
    pub title: String,
    pub owner: Owner,
}

struct Inner {
    backend: Option<Arc<dyn Backend>>,
    tabs: Mutex<Vec<Arc<Tab>>>,
    shown: Mutex<Option<TabId>>,
    observer: Mutex<Option<Observer>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(backend) = &self.backend {
            for tab in self
                .tabs
                .get_mut()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
            {
                if tab.is_live() {
                    backend.close(tab.id);
                }
            }
        }
    }
}

/// The chat's tabs. Clones share them.
#[derive(Clone)]
pub struct ChatTabs {
    inner: Arc<Inner>,
}

const NO_BROWSER: &str = "The browser is not available in this build of WackCode. It needs the app bundle (`cargo dev`).";

impl ChatTabs {
    /// `backend` is `None` where there is no browser (the CLI, or the bare binary).
    pub fn new(backend: Option<Arc<dyn Backend>>) -> Self {
        Self {
            inner: Arc::new(Inner {
                backend,
                tabs: Mutex::default(),
                shown: Mutex::default(),
                observer: Mutex::default(),
            }),
        }
    }

    pub fn available(&self) -> bool {
        self.inner.backend.is_some()
    }

    fn backend(&self) -> Result<&Arc<dyn Backend>, String> {
        self.inner
            .backend
            .as_ref()
            .ok_or_else(|| NO_BROWSER.to_string())
    }

    /// One observer at a time; it is told of every opened, changed and closed tab.
    pub fn set_observer(&self, observer: impl Fn(Signal) + Send + Sync + 'static) {
        *self
            .inner
            .observer
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(observer));
    }

    fn signal(&self, signal: Signal) {
        let observer = self
            .inner
            .observer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(observer) = observer {
            observer(signal);
        }
    }

    fn adopt(&self, tab: &Arc<Tab>) {
        let weak = Arc::downgrade(&self.inner);
        let notify: Notify = Arc::new(move |id| {
            if let Some(inner) = weak.upgrade() {
                ChatTabs { inner }.signal(Signal::Changed(id));
            }
        });
        *tab.notify.lock().unwrap_or_else(|e| e.into_inner()) = Some(notify);
        let weak = Arc::downgrade(&self.inner);
        let popups: PopupSink = Arc::new(move |owner, opener| {
            let child = Tab::new(owner);
            if let Some(inner) = weak.upgrade() {
                let tabs = ChatTabs { inner };
                tabs.adopt(&child);
                // A popup from the page in front comes to the front, as in any browser.
                if tabs.shown() == Some(opener) {
                    tabs.set_shown(Some(child.id));
                }
            }
            child
        });
        *tab.popups.lock().unwrap_or_else(|e| e.into_inner()) = Some(popups);
        self.inner
            .tabs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(tab.clone());
        self.signal(Signal::Opened(tab.id));
    }

    pub fn tabs(&self) -> Vec<Arc<Tab>> {
        self.inner
            .tabs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn get(&self, id: TabId) -> Option<Arc<Tab>> {
        self.tabs().into_iter().find(|tab| tab.id == id)
    }

    /// The tab the pane shows, if any.
    pub fn shown(&self) -> Option<TabId> {
        *self.inner.shown.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_shown(&self, id: Option<TabId>) {
        *self.inner.shown.lock().unwrap_or_else(|e| e.into_inner()) = id;
        if let Some(id) = id {
            self.signal(Signal::Changed(id));
        }
    }

    /// The newest tab `owner` opened that is still open: where its tools act by default.
    pub fn latest_of(&self, owner: &Owner) -> Option<Arc<Tab>> {
        self.tabs()
            .into_iter()
            .rev()
            .find(|tab| &tab.owner == owner)
    }

    /// Opens a new tab at `url`. The address is not checked here: the agent's tools check it
    /// first (`policy`), and the user may open anything.
    pub async fn open(&self, owner: Owner, url: &str) -> Result<Arc<Tab>, String> {
        let backend = self.backend()?.clone();
        let tab = Tab::new(owner);
        tab.update(|state| state.set_address(url));
        // Held before the tab is listed, so nothing that sees it can start a second page.
        let made = {
            let making = tab.making.try_lock().map_err(|_| "The browser is busy with that tab.".to_string())?;
            self.adopt(&tab);
            if self.shown().is_none() {
                self.set_shown(Some(tab.id));
            }
            let made = backend.create(tab.clone(), url.to_string()).await;
            drop(making);
            made
        };
        match made {
            Ok(()) => Ok(tab),
            Err(error) => {
                self.forget(tab.id);
                Err(error)
            }
        }
    }

    fn forget(&self, id: TabId) {
        let removed = {
            let mut tabs = self.inner.tabs.lock().unwrap_or_else(|e| e.into_inner());
            let before = tabs.len();
            tabs.retain(|tab| tab.id != id);
            before != tabs.len()
        };
        if !removed {
            return;
        }
        let mut shown = self.inner.shown.lock().unwrap_or_else(|e| e.into_inner());
        if *shown == Some(id) {
            *shown = self
                .inner
                .tabs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .last()
                .map(|tab| tab.id);
        }
        drop(shown);
        self.signal(Signal::Closed(id));
    }

    pub fn close(&self, id: TabId) {
        if let Some(tab) = self.get(id)
            && tab.is_live()
            && let Some(backend) = &self.inner.backend
        {
            backend.close(id);
        }
        self.forget(id);
    }

    /// Makes sure the engine has a page for a restored tab, at its saved address.
    pub async fn ensure_live(&self, id: TabId) -> Result<Arc<Tab>, String> {
        let tab = self
            .get(id)
            .ok_or_else(|| "That tab is closed.".to_string())?;
        let making = tab.making.lock().await;
        if !tab.is_live() {
            let url = tab.state().url.clone();
            let url = if url.is_empty() {
                "about:blank".to_string()
            } else {
                url
            };
            self.backend()?.create(tab.clone(), url).await?;
        }
        drop(making);
        Ok(tab)
    }

    /// Navigates a tab. Opening the address it already shows does nothing: only `Reload` reloads.
    pub async fn navigate(&self, id: TabId, nav: Nav) -> Result<(), String> {
        let tab = self.ensure_live(id).await?;
        if let Nav::Open(url) = &nav
            && same_address(&tab.state().url, url)
        {
            return Ok(());
        }
        self.backend()?.navigate(id, nav).await
    }

    pub async fn cdp(&self, id: TabId, method: &str, params: Value) -> Result<Value, String> {
        self.ensure_live(id).await?;
        self.backend()?.cdp(id, method.to_string(), params).await
    }

    pub async fn answer_dialog(
        &self,
        id: TabId,
        accept: bool,
        text: Option<String>,
    ) -> Result<(), String> {
        let tab = self
            .get(id)
            .ok_or_else(|| "That tab is closed.".to_string())?;
        if tab.state().dialog.is_none() {
            return Err("The page has no dialog open.".into());
        }
        self.backend()?.answer_dialog(id, accept, text).await
    }

    /// The tabs to save with the chat, in order. Blank tabs are not worth keeping.
    pub fn saved(&self) -> Vec<SavedTab> {
        self.tabs()
            .iter()
            .filter_map(|tab| {
                let state = tab.state();
                (!state.url.is_empty() && state.url != "about:blank").then(|| SavedTab {
                    url: state.url.clone(),
                    title: state.title.clone(),
                    owner: tab.owner.clone(),
                })
            })
            .collect()
    }

    /// Lists saved tabs again without making their pages (see `ensure_live`).
    pub fn restore(&self, saved: Vec<SavedTab>) {
        for saved in saved {
            let tab = Tab::new(saved.owner);
            tab.update(|state| {
                state.set_address(&saved.url);
                state.set_title(&saved.title);
            });
            self.adopt(&tab);
        }
        if self.shown().is_none() {
            self.set_shown(self.tabs().first().map(|tab| tab.id));
        }
    }
}

/// Two addresses name the same page: equal, or equal but for a trailing slash on an empty path.
fn same_address(current: &str, wanted: &str) -> bool {
    let normal = |raw: &str| match url::Url::parse(raw.trim()) {
        Ok(url) => url.to_string(),
        Err(_) => raw.trim().to_string(),
    };
    !current.is_empty() && normal(current) == normal(wanted)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// Records what the engine was asked to do.
    #[derive(Default)]
    pub struct FakeBackend {
        pub calls: StdMutex<Vec<String>>,
        pub fail_create: AtomicBool,
        /// Makes pages after a pause, as the real engine does.
        pub slow: AtomicBool,
    }

    impl Backend for FakeBackend {
        fn create(&self, tab: Arc<Tab>, url: String) -> BoxFuture<Result<(), String>> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("create {} {url}", tab.id));
            let fail = self.fail_create.load(Ordering::Relaxed);
            let slow = self.slow.load(Ordering::Relaxed);
            Box::pin(async move {
                if slow {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                if fail {
                    return Err("engine failed".into());
                }
                tab.set_live(true);
                Ok(())
            })
        }
        fn close(&self, id: TabId) {
            self.calls.lock().unwrap().push(format!("close {id}"));
        }
        fn navigate(&self, id: TabId, nav: Nav) -> BoxFuture<Result<(), String>> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("navigate {id} {nav:?}"));
            Box::pin(async { Ok(()) })
        }
        fn cdp(
            &self,
            id: TabId,
            method: String,
            _params: Value,
        ) -> BoxFuture<Result<Value, String>> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("cdp {id} {method}"));
            Box::pin(async { Ok(Value::Null) })
        }
        fn answer_dialog(
            &self,
            id: TabId,
            accept: bool,
            _text: Option<String>,
        ) -> BoxFuture<Result<(), String>> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("dialog {id} {accept}"));
            Box::pin(async { Ok(()) })
        }
    }

    fn calls(backend: &FakeBackend) -> Vec<String> {
        backend.calls.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn opening_the_shown_address_is_not_a_reload() {
        let backend = Arc::new(FakeBackend::default());
        let tabs = ChatTabs::new(Some(backend.clone()));
        let tab = tabs
            .open(Owner::Agent, "http://localhost:5173")
            .await
            .unwrap();
        tabs.navigate(tab.id, Nav::Open("http://localhost:5173/".into()))
            .await
            .unwrap();
        tabs.navigate(tab.id, Nav::Open("http://localhost:5173/about".into()))
            .await
            .unwrap();
        tabs.navigate(tab.id, Nav::Reload).await.unwrap();
        let id = tab.id;
        assert_eq!(
            calls(&backend),
            vec![
                format!("create {id} http://localhost:5173"),
                format!("navigate {id} Open(\"http://localhost:5173/about\")"),
                format!("navigate {id} Reload"),
            ]
        );
    }

    #[tokio::test]
    async fn restored_tabs_start_no_page_until_used() {
        let backend = Arc::new(FakeBackend::default());
        let tabs = ChatTabs::new(Some(backend.clone()));
        tabs.restore(vec![
            SavedTab {
                url: "https://docs.rs/".into(),
                title: "Docs".into(),
                owner: Owner::User,
            },
            SavedTab {
                url: "http://localhost:3000/".into(),
                title: String::new(),
                owner: Owner::Agent,
            },
        ]);
        assert!(calls(&backend).is_empty(), "restoring makes no pages");
        assert_eq!(tabs.shown(), Some(tabs.tabs()[0].id));
        let agent = tabs.latest_of(&Owner::Agent).unwrap();
        tabs.cdp(agent.id, "Runtime.evaluate", Value::Null)
            .await
            .unwrap();
        assert_eq!(
            calls(&backend),
            vec![
                format!("create {} http://localhost:3000/", agent.id),
                format!("cdp {} Runtime.evaluate", agent.id)
            ]
        );
        assert_eq!(tabs.saved().len(), 2);
    }

    #[tokio::test]
    async fn closing_and_dropping_close_live_pages_only() {
        let backend = Arc::new(FakeBackend::default());
        let tabs = ChatTabs::new(Some(backend.clone()));
        let a = tabs.open(Owner::User, "https://a.example/").await.unwrap();
        let b = tabs.open(Owner::Agent, "https://b.example/").await.unwrap();
        tabs.restore(vec![SavedTab {
            url: "https://c.example/".into(),
            title: String::new(),
            owner: Owner::User,
        }]);
        assert_eq!(tabs.shown(), Some(a.id));
        tabs.close(a.id);
        assert_eq!(
            tabs.shown(),
            tabs.tabs().last().map(|t| t.id),
            "the pane moves to another tab"
        );
        drop(tabs);
        let closes: Vec<_> = calls(&backend)
            .into_iter()
            .filter(|c| c.starts_with("close"))
            .collect();
        assert_eq!(
            closes,
            vec![format!("close {}", a.id), format!("close {}", b.id)]
        );
    }

    #[tokio::test]
    async fn a_failed_page_is_not_kept_and_no_browser_is_a_sentence() {
        let backend = Arc::new(FakeBackend::default());
        backend.fail_create.store(true, Ordering::Relaxed);
        let tabs = ChatTabs::new(Some(backend));
        assert_eq!(
            tabs.open(Owner::Agent, "https://a.example/")
                .await
                .err()
                .as_deref(),
            Some("engine failed")
        );
        assert!(tabs.tabs().is_empty());
        let none = ChatTabs::new(None);
        assert!(
            none.open(Owner::Agent, "https://a.example/")
                .await
                .err()
                .unwrap()
                .contains("not available")
        );
    }

    #[tokio::test]
    async fn the_observer_hears_every_change() {
        let tabs = ChatTabs::new(Some(Arc::new(FakeBackend::default())));
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let log = seen.clone();
        tabs.set_observer(move |signal| log.lock().unwrap().push(signal));
        let tab = tabs.open(Owner::Agent, "https://a.example/").await.unwrap();
        tab.update(|state| state.set_title("A"));
        tabs.close(tab.id);
        let seen = seen.lock().unwrap().clone();
        assert!(seen.contains(&Signal::Opened(tab.id)));
        assert!(seen.contains(&Signal::Changed(tab.id)));
        assert_eq!(seen.last(), Some(&Signal::Closed(tab.id)));
    }

    #[tokio::test]
    async fn popups_join_the_same_chat_with_the_same_owner() {
        let tabs = ChatTabs::new(Some(Arc::new(FakeBackend::default())));
        let parent = tabs
            .open(Owner::SubAgent("explore-1".into()), "https://a.example/")
            .await
            .unwrap();
        parent.set_agent_driving(true);
        let child = parent.spawn_popup().unwrap();
        assert_eq!(child.owner, Owner::SubAgent("explore-1".into()));
        assert!(child.agent_driving());
        assert_eq!(tabs.tabs().len(), 2);
        assert!(!child.is_live(), "the engine makes the popup's page");
        assert_eq!(tabs.shown(), Some(child.id), "a popup from the page in front comes to the front");
    }

    #[tokio::test]
    async fn an_agents_tab_is_driven_from_its_first_load() {
        let tabs = ChatTabs::new(Some(Arc::new(FakeBackend::default())));
        let agent = tabs.open(Owner::Agent, "https://a.example/").await.unwrap();
        let sub = tabs.open(Owner::SubAgent("explore-1".into()), "https://b.example/").await.unwrap();
        let user = tabs.open(Owner::User, "https://c.example/").await.unwrap();
        assert!(agent.agent_driving() && sub.agent_driving());
        assert!(!user.agent_driving());
    }

    #[tokio::test]
    async fn a_page_asked_for_while_open_makes_it_is_made_once() {
        let backend = Arc::new(FakeBackend::default());
        backend.slow.store(true, Ordering::Relaxed);
        let tabs = ChatTabs::new(Some(backend.clone()));
        // A view that sees the new tab before its page exists asks for one, as the pane does.
        let view = tabs.clone();
        tabs.set_observer(move |signal| {
            if let Signal::Opened(id) = signal {
                let view = view.clone();
                tokio::spawn(async move { view.ensure_live(id).await.map(|_| ()) });
            }
        });
        let tab = tabs.open(Owner::User, "https://a.example/").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let calls = backend.calls.lock().unwrap().clone();
        assert_eq!(calls.iter().filter(|call| call.starts_with("create")).count(), 1, "{calls:?}");
        assert!(tab.is_live());
    }

    #[test]
    fn same_address_ignores_the_root_slash_only() {
        assert!(same_address(
            "http://localhost:5173",
            "http://localhost:5173/"
        ));
        assert!(!same_address(
            "http://localhost:5173/a",
            "http://localhost:5173/b"
        ));
        assert!(!same_address("", "about:blank"));
    }
}
