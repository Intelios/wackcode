//! The Browser pane: the chat's browser tabs, and the page of the one shown.
//!
//! - The pane only views and steers: the tabs belong to the chat's `ChatTabs` and their pages to
//!   the engine (`wack-browser`), so closing the pane, moving it or switching chats never reloads a
//!   page. Nothing here makes or closes a page except the user's own tab buttons.
//! - The page is a native view laid over the pane's page area (`crate::browser::show`), placed
//!   only while that area is painted whole: not while the pane slides in, while a pane tab is
//!   dragged (the drop zones would hide under it; resizing a dock keeps it), or while a card stands
//!   in for the page (a crash, a failed load). A page that is not placed is parked, still rendering for the agent.
//! - Nothing drawn over the page is deferred except the dialog card: GPUI's overlay plane takes
//!   the page's input while anything is drawn on it. The agent's glow rings the page from outside.
//! - The address field belongs to the user: what they type opens even an address the agent may
//!   not (only http and https, and never a web search, which would add a network origin). Typing
//!   an address makes the user the tab's driver, so the agent's address rule stops applying to it.
//! - The page has the keyboard only while the app's focus is in this pane: when it leaves (a
//!   shortcut, a click elsewhere), the page gives the keyboard back.
//! - A press on the page gives the pane the app's focus straight after it (`browser::attach`), so
//!   the app's own keys (⌘W, ⌘⇧B) act on the pane and not on whatever had focus before, and the
//!   page gets the ones it handles (⌘A). Where the keyboard itself goes is decided before the
//!   press is routed, in the engine (`cef::before_press`).

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use gpui::{
    Animation, AnimationExt, Bounds, Context, Entity, FocusHandle, Hsla, Pixels, SharedString, Subscription, Window,
    canvas, deferred, div, prelude::*, px, relative,
};
use gpui_tokio::Tokio;
use wack_browser::{ChatTabs, Dialog, DialogKind, Nav, Owner, Tab, TabId, TabState, Viewport};

use crate::{
    icon::Icon,
    tags::Tag,
    text_field::{TextField, TextFieldEvent},
    theme::Theme,
    tooltip::tip,
    typography::{Type, Typography},
};

/// The space around the page, inside the pane: the page is a card on the pane's glass.
const PAGE_MARGIN: f32 = 8.;
const PAGE_RADIUS: f32 = 10.;
const CIRCLE: f32 = 26.;
const TAB_WIDTH: f32 = 180.;
/// Room under a letterboxed page for its size.
const SIZE_LINE: f32 = 22.;

/// What the page's DevTools emulation was last set to: the tab, its viewport, and the scale it is
/// drawn at, in hundredths.
type Fitted = (TabId, Viewport, u32);

pub struct BrowserPane {
    tabs: ChatTabs,
    pub focus: FocusHandle,
    address: Entity<TextField>,
    /// The answer to a page's prompt(), while one is open.
    reply: Entity<TextField>,
    /// The dialog the reply field was last filled for.
    replying: Option<(TabId, String)>,
    /// The user has typed in the address field since it last showed the tab's address.
    editing: bool,
    /// What the pane last put in the address field.
    synced: String,
    /// What went wrong with the last thing the user asked for.
    note: Option<SharedString>,
    /// Why the window could not show pages, set from a paint pass.
    unshowable: Rc<RefCell<Option<String>>>,
    /// Restored tabs whose page has been asked for.
    waking: HashSet<TabId>,
    /// The viewport menu is open.
    viewport_menu: bool,
    /// What the last paint fitted the page to, and what the page has been told.
    fitted: Rc<Cell<Option<Fitted>>>,
    applied: Option<Fitted>,
    /// A still picture in place of the page, for gallery scenes (they have no engine).
    fixture: Option<std::path::PathBuf>,
    /// Hands the keyboard back from a page when the app's focus leaves this pane by any route
    /// a press doesn't cover (a shortcut, a new chat's composer): made on the first render.
    focus_out: Option<gpui::Subscription>,
    _field: Subscription,
    _reply: Subscription,
}

impl BrowserPane {
    pub fn new(tabs: ChatTabs, cx: &mut Context<Self>) -> Self {
        let address = cx.new(|cx| TextField::new(Type::Ui, "Type an address", cx));
        let subscription = cx.subscribe(&address, |pane, _, event: &TextFieldEvent, cx| match event {
            // Events arrive after the change, so the pane's own `set_text` is told apart by text.
            TextFieldEvent::Changed => pane.editing = pane.address.read(cx).text() != pane.synced,
            TextFieldEvent::Enter | TextFieldEvent::SendNow => pane.go(cx),
            _ => {}
        });
        let reply = cx.new(|cx| TextField::new(Type::Ui, "", cx));
        let reply_subscription = cx.subscribe(&reply, |pane, _, event: &TextFieldEvent, cx| {
            if matches!(event, TextFieldEvent::Enter | TextFieldEvent::SendNow) {
                pane.answer(true, cx);
            }
        });
        let mut pane = Self {
            tabs,
            focus: cx.focus_handle(),
            address,
            reply,
            replying: None,
            editing: false,
            synced: String::new(),
            note: None,
            unshowable: Rc::default(),
            waking: HashSet::new(),
            viewport_menu: false,
            fitted: Rc::default(),
            applied: None,
            fixture: None,
            focus_out: None,
            _field: subscription,
            _reply: reply_subscription,
        };
        pane.sync_address(cx);
        pane
    }

    /// Shows a still picture where the page would be: gallery scenes have no engine.
    pub fn with_fixture(mut self, picture: std::path::PathBuf) -> Self {
        self.fixture = Some(picture);
        self
    }

    /// Opens the viewport menu, for a gallery scene.
    pub fn with_viewport_menu(mut self) -> Self {
        self.viewport_menu = true;
        self
    }

    /// A tab changed: show its address again unless the user is typing one.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.sync_address(cx);
        cx.notify();
    }

    fn current(&self) -> Option<Arc<Tab>> {
        let shown = self.tabs.shown();
        shown.and_then(|id| self.tabs.get(id)).or_else(|| self.tabs.tabs().last().cloned())
    }

    fn sync_address(&mut self, cx: &mut Context<Self>) {
        if self.editing {
            return;
        }
        let url = self.current().map(|tab| tab.state().url.clone()).unwrap_or_default();
        let shown = if url == "about:blank" { String::new() } else { url };
        if self.address.read(cx).text() != shown {
            // From its start: the host is the part that says where the page is.
            self.address.update(cx, |field, cx| field.set_text_at_start(&shown, cx));
        }
        self.synced = shown;
    }

    fn select(&mut self, id: TabId, cx: &mut Context<Self>) {
        self.tabs.set_shown(Some(id));
        self.editing = false;
        self.note = None;
        self.sync_address(cx);
        cx.notify();
    }

    /// Opens what the user typed, in the tab shown or a new one.
    fn go(&mut self, cx: &mut Context<Self>) {
        let typed = self.address.read(cx).text().to_string();
        let url = match address(&typed) {
            Ok(url) => url,
            Err(message) => {
                self.note = Some(message.into());
                cx.notify();
                return;
            }
        };
        self.note = None;
        self.editing = false;
        let tabs = self.tabs.clone();
        let current = self.current();
        if let Some(tab) = &current {
            // The user is driving this tab now: the agent's address rule is for the agent.
            tab.set_agent_driving(false);
        }
        let work = Tokio::spawn(cx, async move {
            match current {
                Some(tab) => tabs.navigate(tab.id, Nav::Open(url)).await.map(|_| tab.id),
                None => {
                    let tab = tabs.open(Owner::User, &url).await?;
                    tabs.set_shown(Some(tab.id));
                    Ok(tab.id)
                }
            }
        });
        cx.spawn(async move |pane, cx| {
            let result = work.await.unwrap_or_else(|_| Err("The browser stopped.".into()));
            let _ = pane.update(cx, |pane, cx| {
                match result {
                    Ok(id) => crate::browser::focus_page(id),
                    Err(message) => pane.note = Some(message.into()),
                }
                pane.sync_address(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tabs = self.tabs.clone();
        let work = Tokio::spawn(cx, async move {
            let tab = tabs.open(Owner::User, "about:blank").await?;
            tabs.set_shown(Some(tab.id));
            Ok::<_, String>(())
        });
        cx.spawn(async move |pane, cx| {
            let result = work.await.unwrap_or_else(|_| Err("The browser stopped.".into()));
            let _ = pane.update(cx, |pane, cx| {
                if let Err(message) = result {
                    pane.note = Some(message.into());
                }
                pane.editing = false;
                pane.sync_address(cx);
                cx.notify();
            });
        })
        .detach();
        self.address.update(cx, |field, cx| field.set_text("", cx));
        self.synced.clear();
        crate::browser::release_page_focus();
        self.address.update(cx, |field, cx| field.focus(window, cx));
    }

    fn close_tab(&mut self, id: TabId, cx: &mut Context<Self>) {
        self.tabs.close(id);
        self.editing = false;
        self.sync_address(cx);
        cx.notify();
    }

    fn navigate(&mut self, nav: Nav, cx: &mut Context<Self>) {
        let Some(tab) = self.current() else { return };
        let tabs = self.tabs.clone();
        let work = Tokio::spawn(cx, async move { tabs.navigate(tab.id, nav).await });
        cx.spawn(async move |pane, cx| {
            if let Ok(Err(message)) = work.await {
                let _ = pane.update(cx, |pane, cx| {
                    pane.note = Some(message.into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn answer(&mut self, accept: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.current() else { return };
        let prompt = tab.state().dialog.as_ref().is_some_and(|dialog| dialog.kind == DialogKind::Prompt);
        let text = (prompt && accept).then(|| self.reply.read(cx).text().to_string());
        let (tabs, id) = (self.tabs.clone(), tab.id);
        Tokio::spawn(cx, async move { tabs.answer_dialog(id, accept, text).await }).detach();
        self.replying = None;
    }

    fn choose_viewport(&mut self, viewport: Viewport, cx: &mut Context<Self>) {
        self.viewport_menu = false;
        if let Some(tab) = self.current() {
            tab.update(|state| state.viewport = viewport);
        }
        cx.notify();
    }

    /// Tells the page the size it is laid out at and the scale it is drawn at, when the last
    /// paint fitted it differently from what the page was last told.
    fn apply_viewport(&mut self, cx: &mut Context<Self>) {
        let Some(fitted) = self.fitted.get() else { return };
        if self.applied == Some(fitted) || self.fixture.is_some() {
            return;
        }
        self.applied = Some(fitted);
        let (id, viewport, scale) = fitted;
        let (method, params) = match viewport.size() {
            None => ("Emulation.clearDeviceMetricsOverride", serde_json::json!({})),
            Some((width, height, mobile)) => (
                "Emulation.setDeviceMetricsOverride",
                serde_json::json!({
                    "width": width,
                    "height": height,
                    "deviceScaleFactor": 0,
                    "mobile": mobile,
                    "scale": scale as f64 / 100.,
                }),
            ),
        };
        // Phones and tablets are touch devices: pages that ask (`pointer: coarse`, touch events)
        // are told so.
        let touch = serde_json::json!({ "enabled": viewport.size().is_some_and(|(_, _, mobile)| mobile), "maxTouchPoints": 5 });
        let tabs = self.tabs.clone();
        Tokio::spawn(cx, async move {
            tabs.cdp(id, method, params).await?;
            tabs.cdp(id, "Emulation.setTouchEmulationEnabled", touch).await
        })
        .detach();
    }

    /// A restored tab shown in the pane gets its page.
    fn wake(&mut self, tab: &Arc<Tab>, cx: &mut Context<Self>) {
        if tab.is_live() || self.fixture.is_some() || !self.waking.insert(tab.id) {
            return;
        }
        let (tabs, id) = (self.tabs.clone(), tab.id);
        let work = Tokio::spawn(cx, async move { tabs.ensure_live(id).await.map(|_| ()) });
        cx.spawn(async move |pane, cx| {
            if let Ok(Err(message)) = work.await {
                let _ = pane.update(cx, |pane, cx| {
                    pane.note = Some(message.into());
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

/// Turns what the user typed into an address: a scheme added where it is plain which one is
/// meant, and never a web search (that would send what they typed to a search engine).
pub fn address(typed: &str) -> Result<String, String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Err("Type an address.".into());
    }
    if typed.chars().any(char::is_whitespace) {
        return Err("Type a web address, like example.com. The browser does not search the web.".into());
    }
    let lower = typed.to_ascii_lowercase();
    if lower == "about:blank" || lower.starts_with("http://") || lower.starts_with("https://") {
        return Ok(typed.to_string());
    }
    if let Some((scheme, _)) = lower.split_once(':')
        && lower.contains("://")
    {
        return Err(format!("The browser opens http and https addresses, not {scheme}."));
    }
    let host = lower.split(['/', '?', '#']).next().unwrap_or_default();
    let name = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => host.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(host, |(name, _)| name),
    };
    let local = name == "localhost"
        || name.ends_with(".localhost")
        || name.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
        || host.starts_with('[');
    if local || name.parse::<std::net::IpAddr>().is_ok() {
        return Ok(format!("http://{typed}"));
    }
    if name.contains('.') && !name.starts_with('.') && !name.ends_with('.') {
        return Ok(format!("https://{typed}"));
    }
    Err("Type a web address, like example.com. The browser does not search the web.".into())
}

/// What the address field's badge says about where a page is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Secure,
    ThisMac,
    NotSecure,
    Blank,
}

pub fn place_of(url: &str) -> Place {
    let lower = url.to_ascii_lowercase();
    let rest = lower.strip_prefix("http://");
    if lower.is_empty() || lower == "about:blank" {
        Place::Blank
    } else if lower.starts_with("https://") {
        Place::Secure
    } else if let Some(rest) = rest {
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let name = host.rsplit_once(':').map_or(host, |(name, _)| name);
        if name == "localhost" || name.ends_with(".localhost") || name.starts_with("127.") || name == "[::1]" {
            Place::ThisMac
        } else {
            Place::NotSecure
        }
    } else {
        Place::NotSecure
    }
}

/// The host of an http(s) address, without credentials or port.
fn host_of(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = match host.strip_prefix('[') {
        Some(v6) => format!("[{}]", v6.split(']').next()?),
        None => host.split(':').next()?.to_string(),
    };
    (!host.is_empty()).then_some(host)
}

/// A page laid out at `width` × `height`, scaled down (never up) to fit `area` above its size
/// line, and centred: its frame in the area, and the scale.
fn letterbox(area: Bounds<Pixels>, width: f32, height: f32) -> (Bounds<Pixels>, f32) {
    let room_width = f32::from(area.size.width);
    let room_height = (f32::from(area.size.height) - SIZE_LINE).max(1.);
    let scale = (room_width / width).min(room_height / height).min(1.);
    let size = gpui::size(px((width * scale).floor()), px((height * scale).floor()));
    let origin = gpui::point(
        area.origin.x + (area.size.width - size.width) / 2.,
        area.origin.y + px(((room_height - f32::from(size.height)) / 2.).max(0.)),
    );
    (Bounds { origin, size }, scale)
}

/// The bounds lie wholly inside the mask, give or take a point of rounding.
fn inside(mask: Bounds<Pixels>, bounds: Bounds<Pixels>) -> bool {
    let slack = px(1.);
    bounds.origin.x + slack >= mask.origin.x
        && bounds.origin.y + slack >= mask.origin.y
        && bounds.right() <= mask.right() + slack
        && bounds.bottom() <= mask.bottom() + slack
}

fn circle(id: &'static str, icon: Icon, enabled: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(CIRCLE))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .when(enabled, |circle| circle.cursor_pointer().hover(|circle| circle.bg(theme.hover)))
        .child(icon.view(px(14.), if enabled { theme.text_soft } else { theme.text_faint }))
}

impl BrowserPane {
    fn toolbar(&self, tab: Option<&Arc<Tab>>, state: Option<&TabState>, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pane = cx.entity();
        let (back, forward, loading) = state.map_or((false, false, false), |s| (s.can_go_back, s.can_go_forward, s.loading));
        let place = state.map_or(Place::Blank, |s| place_of(&s.url));
        let badge = match place {
            Place::Secure => Some(Icon::Lock.view(px(12.), theme.text_dim).into_any_element()),
            Place::ThisMac => Some(
                div()
                    .flex_none()
                    .px(px(6.))
                    .h(px(18.))
                    .rounded_full()
                    .bg(theme.hover)
                    .flex()
                    .items_center()
                    .typography(Type::Small)
                    .text_color(theme.text_soft)
                    .child("This Mac")
                    .into_any_element(),
            ),
            Place::NotSecure => Some(
                div()
                    .id("not-secure")
                    .flex_none()
                    .tooltip(tip("Not secure: this page is not sent over https"))
                    .child(Icon::Alert.view(px(12.), theme.warning))
                    .into_any_element(),
            ),
            Place::Blank => Some(Icon::Globe.view(px(12.), theme.text_faint).into_any_element()),
        };
        let url_pill = div()
            .flex_1()
            .min_w_0()
            .h(px(CIRCLE + 4.))
            .px(px(12.))
            .rounded_full()
            .bg(theme.hover)
            .border_1()
            .border_color(theme.border)
            .flex()
            .items_center()
            .gap(px(8.))
            .children(badge)
            .child(div().flex_1().min_w_0().child(self.address.clone()))
            .tag("browser-address");

        let (back_pane, forward_pane, reload_pane) = (pane.clone(), pane.clone(), pane.clone());
        let mut bar = div()
            .flex_none()
            .px(px(10.))
            .pt(px(8.))
            .pb(px(6.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(
                circle("browser-back", Icon::ArrowLeft, back, theme)
                    .tooltip(tip("Back"))
                    .when(back, |circle| circle.on_click(move |_, _, cx| back_pane.update(cx, |pane, cx| pane.navigate(Nav::Back, cx))))
                    .tag("browser-back"),
            )
            .child(
                circle("browser-forward", Icon::ArrowRight, forward, theme)
                    .tooltip(tip("Forward"))
                    .when(forward, |circle| circle.on_click(move |_, _, cx| forward_pane.update(cx, |pane, cx| pane.navigate(Nav::Forward, cx))))
                    .tag("browser-forward"),
            )
            .child(
                circle("browser-reload", if loading { Icon::Close } else { Icon::Reload }, tab.is_some(), theme)
                    .tooltip(tip(if loading { "Stop loading" } else { "Reload" }))
                    .on_click(move |_, _, cx| {
                        reload_pane.update(cx, |pane, cx| pane.navigate(if loading { Nav::Stop } else { Nav::Reload }, cx))
                    })
                    .tag("browser-reload"),
            )
            .child(div().w(px(4.)))
            .child(url_pill);

        // The size the page is laid out at.
        let viewport = state.map_or(Viewport::Fit, |state| state.viewport);
        let menu_pane = pane.clone();
        bar = bar.child(
            div()
                .id("browser-viewport")
                .flex_none()
                .ml(px(4.))
                .size(px(CIRCLE + 4.))
                .rounded_full()
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .when(viewport != Viewport::Fit, |circle| circle.bg(theme.accent_wash))
                .when(self.viewport_menu, |circle| circle.bg(theme.hover))
                .hover(|circle| circle.bg(theme.hover))
                .tooltip(tip(format!("Page size: {}", viewport.name())))
                .on_click(move |_, _, cx| {
                    menu_pane.update(cx, |pane, cx| {
                        pane.viewport_menu = !pane.viewport_menu;
                        cx.notify();
                    })
                })
                .child(Icon::Phone.view(px(14.), if viewport != Viewport::Fit { theme.accent_text } else { theme.text_soft }))
                .tag("browser-viewport"),
        );

        bar
    }

    /// Take control, offered on a tab the agent opened or has driven: at the right of the tab row,
    /// so the address keeps the toolbar's width.
    fn take_control(tab: Option<&Arc<Tab>>, theme: &Theme) -> Option<gpui::AnyElement> {
        let tab = tab.filter(|tab| tab.owner != Owner::User || tab.agent_driving())?;
        let taken = tab.user_has_control();
        let tab = tab.clone();
        Some(
            div()
                .id("take-control")
                .flex_none()
                .ml(px(6.))
                .h(px(26.))
                .px(px(10.))
                .rounded_full()
                .cursor_pointer()
                .flex()
                .items_center()
                .gap(px(6.))
                .border_1()
                .border_color(if taken { theme.accent_line } else { theme.border })
                .when(taken, |pill| pill.bg(theme.accent_wash))
                .hover(|pill| pill.bg(theme.hover))
                .tooltip(tip(if taken {
                    "Give the page back: the agent may use it again"
                } else {
                    "Take control: the agent's tools leave this page alone until you give it back"
                }))
                .on_click(move |_, _, _| tab.set_user_control(!taken))
                .child(Icon::Cursor.view(px(12.), if taken { theme.accent_text } else { theme.text_soft }))
                .child(div().typography(Type::Small).text_color(if taken { theme.accent_text } else { theme.text_soft }).child(
                    if taken { "You have control" } else { "Take control" },
                ))
                .tag("take-control")
                .into_any_element(),
        )
    }

    fn strip(&self, list: &[Arc<Tab>], shown: Option<TabId>, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut strip = div().id("browser-tabs").flex_1().min_w_0().flex().items_center().gap(px(6.)).overflow_x_scroll();
        for tab in list {
            let picked = Some(tab.id) == shown;
            let state = tab.state();
            let title = if !state.title.is_empty() {
                state.title.clone()
            } else if state.url.is_empty() || state.url == "about:blank" {
                "New tab".into()
            } else {
                state.url.clone()
            };
            let loading = state.loading;
            drop(state);
            let owner_icon = match &tab.owner {
                Owner::User => Icon::Globe.view(px(11.), theme.text_dim).into_any_element(),
                _ => Icon::Duck.view(px(11.), if picked { theme.accent_text } else { theme.text_dim }).into_any_element(),
            };
            let who = match &tab.owner {
                Owner::User => "Opened by you".to_string(),
                Owner::Agent => "Opened by the agent".to_string(),
                Owner::SubAgent(id) => format!("Opened by sub-agent {id}"),
            };
            let (select_pane, close_pane, id) = (cx.entity(), cx.entity(), tab.id);
            let group: SharedString = format!("browser-tab-{}", tab.id).into();
            strip = strip.child(
                div()
                    .id(("browser-tab", tab.id as usize))
                    .group(group.clone())
                    .flex_none()
                    .max_w(px(TAB_WIDTH))
                    .h(px(26.))
                    .pl(px(9.))
                    .pr(px(4.))
                    .rounded_full()
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .border_1()
                    .border_color(if picked { theme.border_strong } else { theme.border })
                    .when(picked, |pill| pill.bg(theme.hover))
                    .hover(|pill| pill.bg(theme.hover))
                    .tooltip(tip(format!("{title} · {who}")))
                    .on_click(move |_, _, cx| select_pane.update(cx, |pane, cx| pane.select(id, cx)))
                    .child(owner_icon)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .typography(Type::Small)
                            .text_color(if picked { theme.text } else { theme.text_soft })
                            .when(loading, |title| title.text_color(theme.text_dim))
                            .child(title),
                    )
                    .child(
                        div()
                            .id(("close-browser-tab", tab.id as usize))
                            .flex_none()
                            .size(px(18.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(!picked, |close| close.invisible().group_hover(group.clone(), |close| close.visible()))
                            .hover(|close| close.bg(theme.hover))
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                close_pane.update(cx, |pane, cx| pane.close_tab(id, cx));
                            })
                            .child(Icon::Close.view(px(10.), theme.text_dim))
                            .tag("close-browser-tab"),
                    )
                    .tag("browser-tab"),
            );
        }
        let new_pane = cx.entity();
        strip.child(
            circle("new-browser-tab", Icon::Plus, true, theme)
                .size(px(26.))
                .tooltip(tip("New tab"))
                .on_click(move |_, window, cx| new_pane.update(cx, |pane, cx| pane.new_tab(window, cx)))
                .tag("new-browser-tab"),
        )
    }

    /// The page sizes, under the toolbar's size button. Drawn on the overlay plane, like every
    /// menu: while it is open the page takes no input, and a press outside closes it.
    fn viewport_menu(&self, current: Viewport, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let close_pane = cx.entity();
        let backdrop = gpui::anchored().position_mode(gpui::AnchoredPositionMode::Window).position(gpui::point(px(0.), px(0.))).child(
            div().id("browser-viewport-backdrop").size(px(10000.)).on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                close_pane.update(cx, |pane, cx| {
                    pane.viewport_menu = false;
                    cx.notify();
                });
            }),
        );
        let mut list = div()
            .id("browser-viewport-menu")
            .occlude()
            .w(px(220.))
            .p(px(6.))
            .rounded(px(12.))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border_strong)
            .shadow_lg()
            .flex()
            .flex_col();
        for viewport in Viewport::ALL {
            let pane = cx.entity();
            let picked = viewport == current;
            let size = viewport.size().map(|(width, height, _)| format!("{width} × {height}"));
            list = list.child(
                div()
                    .id(("browser-viewport-choice", viewport as usize))
                    .h(px(30.))
                    .px(px(10.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .hover(|row| row.bg(theme.hover))
                    .on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.choose_viewport(viewport, cx)))
                    .child(div().w(px(14.)).children(picked.then(|| Icon::Check.view(px(12.), theme.accent_text))))
                    .child(div().flex_1().typography(Type::Ui).text_color(theme.text).child(viewport.name()))
                    .children(size.map(|size| div().typography(Type::Small).text_color(theme.text_faint).child(size)))
                    .tag("browser-viewport-choice"),
            );
        }
        div()
            .child(deferred(backdrop).with_priority(1))
            .child(deferred(div().absolute().top(px(CIRCLE + 12.)).right(px(10.)).child(crate::motion::enter("browser-viewport-arrive", list))).with_priority(2))
    }

    /// The loading line under the toolbar: a sweep, or a still line with Reduce Motion.
    fn loading_line(loading: bool, still: bool, theme: &Theme) -> impl IntoElement + use<> {
        let track = div().flex_none().relative().h(px(2.)).mx(px(PAGE_MARGIN + PAGE_RADIUS)).overflow_hidden();
        if !loading {
            return track.into_any_element();
        }
        if still {
            return track.child(div().size_full().rounded_full().bg(Hsla { a: 0.6, ..theme.accent_line })).into_any_element();
        }
        track
            .child(
                div().absolute().top_0().h_full().w(relative(0.3)).rounded_full().bg(theme.accent_line).with_animation(
                    "browser-loading",
                    Animation::new(Duration::from_millis(1100)).repeat(),
                    |bar, t| bar.left(relative(t * 1.3 - 0.3)),
                ),
            )
            .into_any_element()
    }

    /// What stands in for the page when there is nothing to show, or the page failed.
    fn card(icon: Icon, title: SharedString, detail: Option<String>, action: Option<(&'static str, Nav)>, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pane = cx.entity();
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .px(px(24.))
            .child(icon.view(px(18.), theme.text_faint))
            .child(div().typography(Type::UiStrong).text_color(theme.text_soft).text_center().child(title))
            .children(detail.map(|detail| div().max_w(px(360.)).text_center().typography(Type::Small).text_color(theme.text_dim).child(detail)))
            .children(action.map(|(label, nav)| {
                div()
                    .id("browser-card-action")
                    .mt(px(6.))
                    .h(px(26.))
                    .px(px(12.))
                    .rounded_full()
                    .border_1()
                    .border_color(theme.border)
                    .cursor_pointer()
                    .hover(|button| button.bg(theme.hover))
                    .flex()
                    .items_center()
                    .typography(Type::Small)
                    .text_color(theme.text_soft)
                    .on_click(move |_, _, cx| pane.update(cx, |pane, cx| pane.navigate(nav.clone(), cx)))
                    .child(label)
                    .tag("browser-card-action")
            }))
    }

    fn dialog_card(&self, dialog: &Dialog, host: &str, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (accept_pane, dismiss_pane) = (cx.entity(), cx.entity());
        let asks = matches!(dialog.kind, DialogKind::Confirm | DialogKind::Prompt | DialogKind::BeforeUnload);
        let (accept_label, dismiss_label) = match dialog.kind {
            DialogKind::BeforeUnload => ("Leave", "Stay"),
            _ if asks => ("OK", "Cancel"),
            _ => ("OK", ""),
        };
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .h(px(28.))
                .px(px(14.))
                .rounded_full()
                .cursor_pointer()
                .flex()
                .items_center()
                .typography(Type::UiStrong)
                .when(primary, |button| button.bg(theme.accent).text_color(theme.on_accent).hover(|button| button.bg(theme.accent_hover)))
                .when(!primary, |button| button.border_1().border_color(theme.border).text_color(theme.text_soft).hover(|button| button.bg(theme.hover)))
                .child(label)
        };
        div()
            .w(px(340.))
            .max_w(relative(0.9))
            .p(px(16.))
            .rounded(px(16.))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border_strong)
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().typography(Type::Small).text_color(theme.text_dim).child(format!("{host} says")))
            .child(div().typography(Type::Ui).text_color(theme.text).child(dialog.message.clone()))
            .when(dialog.kind == DialogKind::Prompt, |card| {
                card.child(
                    div()
                        .h(px(30.))
                        .px(px(12.))
                        .rounded_full()
                        .bg(theme.hover)
                        .border_1()
                        .border_color(theme.border)
                        .flex()
                        .items_center()
                        .child(self.reply.clone()),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .when(asks, |row| {
                        row.child(button("browser-dialog-dismiss", dismiss_label, false).on_click(move |_, _, cx| {
                            dismiss_pane.update(cx, |pane, cx| pane.answer(false, cx))
                        }).tag("browser-dialog-dismiss"))
                    })
                    .child(button("browser-dialog-accept", accept_label, true).on_click(move |_, _, cx| accept_pane.update(cx, |pane, cx| pane.answer(true, cx))).tag("browser-dialog-accept")),
            )
            .tag("browser-dialog")
    }
}

impl Render for BrowserPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_out.is_none() {
            // Not when the window goes inactive (another app in front): the page keeps the keyboard
            // for when the window comes back, as a field in the app does.
            self.focus_out = Some(cx.on_focus_out(&self.focus, window, |_, _, window, _| {
                if window.is_window_active() {
                    crate::browser::release_page_focus();
                }
            }));
        }
        let theme = Theme::get(cx).clone();
        let still = cx.reduce_motion();
        let list = self.tabs.tabs();
        let tab = self.current();
        let shown = tab.as_ref().map(|tab| tab.id);
        let state = tab.as_ref().map(|tab| tab.state().clone());
        if let Some(tab) = &tab {
            self.wake(tab, cx);
        }
        self.apply_viewport(cx);

        // A prompt() arriving: its field starts on the page's default answer, and takes the keys.
        if let (Some(id), Some(dialog)) = (shown, state.as_ref().and_then(|state| state.dialog.as_ref()))
            && dialog.kind == DialogKind::Prompt
            && self.replying.as_ref() != Some(&(id, dialog.message.clone()))
        {
            self.replying = Some((id, dialog.message.clone()));
            let default = dialog.default_prompt.clone();
            self.reply.update(cx, |field, cx| {
                field.set_text(&default, cx);
                field.select_all_text(cx);
            });
            crate::browser::release_page_focus();
            self.reply.update(cx, |field, cx| field.focus(window, cx));
        }

        let toolbar = self.toolbar(tab.as_ref(), state.as_ref(), &theme, cx);
        let take_control = Self::take_control(tab.as_ref(), &theme);
        let strip = (!list.is_empty() || take_control.is_some()).then(|| {
            div().flex_none().px(px(10.)).pb(px(6.)).flex().items_center().child(self.strip(&list, shown, &theme, cx)).children(take_control)
        });
        let loading = state.as_ref().is_some_and(|state| state.loading);
        // The agent's address rule stopped a navigation: the page it would have left stays, and
        // this says why under the toolbar. A tab with no page yet shows it as a card instead.
        let blank = state.as_ref().is_none_or(|state| state.url.is_empty() || state.url == "about:blank");
        let refused = state.as_ref().and_then(|state| state.refused.clone());
        let note = self.note.clone().or_else(|| refused.as_ref().filter(|_| !blank).map(|(_, why)| SharedString::from(format!("Not opened: {why}")))).map(|note| {
            div().flex_none().px(px(18.)).pb(px(6.)).typography(Type::Small).text_color(theme.danger).child(note).tag("browser-note")
        });
        let unshowable = self.unshowable.borrow().clone();

        // What fills the page area: the page itself, or a card standing in for it.
        let stand_in: Option<gpui::AnyElement> = if !self.tabs.available() && self.fixture.is_none() {
            Some(Self::card(Icon::Globe, "The browser isn't available here".into(), Some("It needs WackCode's app bundle: start the dev app with `cargo dev`.".into()), None, &theme, cx).into_any_element())
        } else if let Some(message) = unshowable {
            Some(Self::card(Icon::Alert, "The page can't be shown".into(), Some(message), None, &theme, cx).into_any_element())
        } else {
            match &state {
                None => Some(
                    Self::card(Icon::Globe, "No pages open".into(), Some("Type an address above. Pages the agent opens show here too.".into()), None, &theme, cx)
                        .into_any_element(),
                ),
                Some(state) if state.crashed.is_some() => Some(
                    Self::card(Icon::Alert, "This page stopped working".into(), state.crashed.clone().filter(|reason| !reason.is_empty()), Some(("Reload", Nav::Reload)), &theme, cx)
                        .into_any_element(),
                ),
                Some(state) if blank && state.refused.is_some() => {
                    let (url, why) = state.refused.clone().unwrap_or_default();
                    let host = host_of(&url).unwrap_or_else(|| url.clone());
                    Some(Self::card(Icon::Alert, format!("Didn't open {host}").into(), Some(why), None, &theme, cx).into_any_element())
                }
                Some(state) if state.load_error.is_some() => {
                    let (url, error) = state.load_error.clone().unwrap_or_default();
                    let host = host_of(&url).unwrap_or_else(|| url.clone());
                    Some(Self::card(Icon::Alert, format!("Couldn't open {host}").into(), Some(humane_error(&error)), Some(("Try again", Nav::Open(url))), &theme, cx).into_any_element())
                }
                _ => None,
            }
        };

        let stand_in_absent = stand_in.is_none();
        let driving = tab.as_ref().is_some_and(|tab| tab.agent_driving() && !tab.user_has_control()) && stand_in.is_none();
        let dialog = state.as_ref().and_then(|state| state.dialog.clone()).filter(|_| stand_in.is_none());
        let host = state.as_ref().and_then(|state| host_of(&state.url)).unwrap_or_else(|| "This page".into());

        let page: gpui::AnyElement = match (stand_in, &self.fixture) {
            (Some(card), _) => card,
            (None, Some(picture)) => {
                let sized = state.as_ref().is_some_and(|state| state.viewport.size().is_some());
                div()
                    .size_full()
                    .when(sized, |area| area.pb(px(SIZE_LINE)))
                    .child(gpui::img(picture.clone()).size_full().rounded(px(PAGE_RADIUS)).object_fit(if sized { gpui::ObjectFit::Contain } else { gpui::ObjectFit::Cover }))
                    .into_any_element()
            }
            (None, None) => {
                let id = shown.unwrap_or_default();
                let unshowable = self.unshowable.clone();
                let fitted = self.fitted.clone();
                let viewport = state.as_ref().map_or(Viewport::Fit, |state| state.viewport);
                let focus = self.focus.clone();
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let mask = window.content_mask().bounds;
                        if !inside(mask, bounds) {
                            return;
                        }
                        let (frame, scale) = match viewport.size() {
                            Some((width, height, _)) => letterbox(bounds, width as f32, height as f32),
                            None => (bounds, 1.),
                        };
                        fitted.set(Some((id, viewport, (scale * 100.).round() as u32)));
                        if let Err(message) = crate::browser::show(window, id, &focus, frame, PAGE_RADIUS) {
                            *unshowable.borrow_mut() = Some(message);
                        }
                    },
                )
                .size_full()
                .into_any_element()
            }
        };

        // A letterboxed page says what size it is laid out at, and how far it is scaled down.
        let sized_line = state.as_ref().and_then(|state| state.viewport.size().map(|size| (state.viewport, size))).filter(|_| stand_in_absent).map(|(viewport, (width, height, _))| {
            let scale = self.fitted.get().filter(|fitted| Some(fitted.0) == shown).map(|fitted| format!(" · {}%", fitted.2)).unwrap_or_default();
            div()
                .absolute()
                .bottom(px(3.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .typography(Type::Small)
                .text_color(theme.text_faint)
                .child(format!("{} · {width} × {height}{scale}", viewport.name()))
                .tag("browser-size-line")
        });
        let letterboxed = sized_line.is_some();
        let viewport_menu = self.viewport_menu.then(|| self.viewport_menu(state.as_ref().map_or(Viewport::Fit, |state| state.viewport), &theme, cx));
        let page_area = div()
            .flex_1()
            .min_h_0()
            .relative()
            .m(px(PAGE_MARGIN))
            .mt(px(2.))
            .rounded(px(PAGE_RADIUS))
            .bg(theme.surface)
            // A page at a fixed size sits on a softer ground, so a white page's edge shows.
            .when(letterboxed, |area| area.child(div().absolute().top_0().left_0().size_full().rounded(px(PAGE_RADIUS)).bg(theme.hover).border_1().border_color(theme.border)))
            // The agent at work: a ring around the page, outside it, so the page never covers it.
            .when(driving, |area| {
                area.child(
                    div()
                        .absolute()
                        .top(px(-3.))
                        .left(px(-3.))
                        .right(px(-3.))
                        .bottom(px(-3.))
                        .rounded(px(PAGE_RADIUS + 3.))
                        .border_2()
                        .border_color(theme.accent_line)
                        .tag("browser-driving"),
                )
            })
            .child(page)
            .when_some(sized_line, |area, line| area.child(line))
            .when_some(dialog, |area, dialog| {
                area.child(deferred(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(self.dialog_card(&dialog, &host, &theme, cx)),
                ))
            })
            .tag("browser-page");

        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .child(toolbar)
            .children(viewport_menu)
            .children(strip)
            .children(note)
            .child(Self::loading_line(loading, still, &theme))
            .child(page_area)
            .tag("browser-pane")
    }
}

/// Chromium's error names, as a sentence.
fn humane_error(error: &str) -> String {
    let known = match error {
        "ERR_NAME_NOT_RESOLVED" => "There's no site at that name.",
        "ERR_CONNECTION_REFUSED" => "Nothing answered there. If it's a dev server, is it running?",
        "ERR_INTERNET_DISCONNECTED" => "This Mac is offline.",
        "ERR_CONNECTION_TIMED_OUT" | "ERR_TIMED_OUT" => "It took too long to answer.",
        "ERR_ADDRESS_UNREACHABLE" => "That address can't be reached from this Mac.",
        "ERR_CERT_AUTHORITY_INVALID" | "ERR_CERT_COMMON_NAME_INVALID" | "ERR_CERT_DATE_INVALID" => {
            "Its certificate isn't valid, so the connection isn't private."
        }
        _ => "",
    };
    if !known.is_empty() {
        known.to_string()
    } else if error.starts_with("ERR_") {
        format!("The page failed to load ({error}).")
    } else {
        // The engine's own sentences, such as the agent's address rule.
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_addresses_get_the_scheme_they_mean() {
        assert_eq!(address("example.com").unwrap(), "https://example.com");
        assert_eq!(address(" example.com/a?b=1 ").unwrap(), "https://example.com/a?b=1");
        assert_eq!(address("localhost:5173").unwrap(), "http://localhost:5173");
        assert_eq!(address("app.localhost:3000/login").unwrap(), "http://app.localhost:3000/login");
        assert_eq!(address("127.0.0.1:8080").unwrap(), "http://127.0.0.1:8080");
        assert_eq!(address("[::1]:4000").unwrap(), "http://[::1]:4000");
        assert_eq!(address("192.168.1.1").unwrap(), "http://192.168.1.1", "the user may open their own network");
        assert_eq!(address("HTTPS://Example.com").unwrap(), "HTTPS://Example.com");
        assert_eq!(address("about:blank").unwrap(), "about:blank");
    }

    #[test]
    fn words_are_not_searched() {
        assert!(address("rust gpui").unwrap_err().contains("does not search"));
        assert!(address("wackcode").unwrap_err().contains("does not search"));
        assert!(address("").is_err());
        assert!(address("file:///etc/hosts").unwrap_err().contains("not file"));
        assert!(address("chrome://settings").unwrap_err().contains("not chrome"));
    }

    #[test]
    fn the_badge_tells_where_a_page_is() {
        assert_eq!(place_of("https://example.com/"), Place::Secure);
        assert_eq!(place_of("http://localhost:3000/"), Place::ThisMac);
        assert_eq!(place_of("http://127.0.0.1:8765/"), Place::ThisMac);
        assert_eq!(place_of("http://example.com/"), Place::NotSecure);
        assert_eq!(place_of("about:blank"), Place::Blank);
        assert_eq!(place_of(""), Place::Blank);
    }

    #[test]
    fn a_fixed_size_page_is_scaled_down_and_centred_never_up() {
        let area = Bounds { origin: gpui::point(px(100.), px(50.)), size: gpui::size(px(440.), px(622.)) };
        let (frame, scale) = letterbox(area, 390., 844.);
        assert!((scale - 600. / 844.).abs() < 0.001, "{scale}");
        assert_eq!(frame.size.height, px((844. * scale).floor()));
        assert!(frame.origin.x > area.origin.x && frame.right() < area.right(), "centred across");
        let roomy = Bounds { origin: gpui::point(px(0.), px(0.)), size: gpui::size(px(2000.), px(2000.)) };
        let (frame, scale) = letterbox(roomy, 390., 844.);
        assert_eq!(scale, 1.);
        assert_eq!(frame.size.width, px(390.));
    }

    #[test]
    fn hosts_drop_credentials_and_ports() {
        assert_eq!(host_of("https://a:b@example.com:8443/x").as_deref(), Some("example.com"));
        assert_eq!(host_of("http://[::1]:3000/").as_deref(), Some("[::1]"));
        assert_eq!(host_of("about:blank"), None);
    }
}
