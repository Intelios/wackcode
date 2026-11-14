//! The window's root view: the sidebar beside one screen.
//!
//! The app draws its own title strip, so the 40-point band across the top of both columns is what
//! moves the window. The traffic lights sit in the sidebar's band.
//!
//! Composition is deliberately off-centre: content hangs from one left edge (`CONTENT_INSET` into
//! the main column) and the space to its right is left open, because that is where a chat's panes
//! will open. Nothing is centred in the window.

use gpui::{
    Context, Hsla, MouseButton, Pixels, SharedString, Window, WindowControlArea, div,
    linear_color_stop, linear_gradient, prelude::*, px, relative,
};

use crate::{
    composer::{Composer, ComposerData},
    icon::Icon,
    sidebar::{SIDEBAR_WIDTH, Sidebar, SidebarData},
    tags::{self, Tag},
    theme::Theme,
    transcript::{Item, Transcript},
    typography::{Type, Typography},
};

/// Height of the strip across the top that holds the traffic lights and drags the window.
pub const TITLE_STRIP_HEIGHT: Pixels = px(40.);
/// Where content starts, measured from the main column's left edge.
pub const CONTENT_INSET: Pixels = px(64.);
/// The transcript's widest line.
pub const TRANSCRIPT_MAX_WIDTH: Pixels = px(700.);

#[derive(Clone, Debug)]
pub struct ProjectInfo {
    pub name: SharedString,
    /// Shown shortened, with `~` for the home folder.
    pub path: SharedString,
    pub branch: Option<SharedString>,
}

#[derive(Clone, Debug)]
pub enum Screen {
    /// No project yet: the first launch.
    Welcome,
    NewChat { project: ProjectInfo, composer: ComposerData },
    Chat { title: SharedString, project: ProjectInfo, items: Vec<Item>, composer: ComposerData },
}

pub struct Shell {
    sidebar: SidebarData,
    screen: Screen,
    /// The gallery has no real window controls; it draws neutral circles where they would be.
    traffic_light_stand_ins: bool,
}

impl Shell {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self { sidebar: SidebarData::default(), screen: Screen::Welcome, traffic_light_stand_ins: false }
    }

    pub fn showing(sidebar: SidebarData, screen: Screen) -> Self {
        Self { sidebar, screen, traffic_light_stand_ins: true }
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();

        let lights = div()
            .flex()
            .gap(px(8.))
            .pl(px(14.))
            .pt(px(14.))
            .when(self.traffic_light_stand_ins, |lights| {
                lights.children((0..3).map(|_| {
                    div().size(px(12.)).rounded_full().bg(theme.border_strong)
                }))
            });
        let sidebar = div()
            .flex_none()
            .w(SIDEBAR_WIDTH)
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.sidebar)
            .child(drag_strip("sidebar-strip", lights))
            .child(Sidebar::new(self.sidebar.clone()));

        let (strip, body) = match self.screen.clone() {
            Screen::Welcome => (div(), welcome(&theme).into_any_element()),
            Screen::NewChat { project, composer } => {
                (div(), new_chat(project, composer, &theme).into_any_element())
            }
            Screen::Chat { title, project, items, composer } => (
                chat_strip(title, project, &theme),
                chat(items, composer, &theme).into_any_element(),
            ),
        };

        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.window)
            .child(drag_strip("title-strip", strip))
            .child(div().flex_1().min_h_0().relative().child(body));

        tags::root(
            div()
                .size_full()
                .flex()
                .bg(theme.window)
                .text_color(theme.text)
                .typography(Type::Ui)
                .child(sidebar)
                .child(main),
        )
    }
}

fn drag_strip(name: &'static str, content: impl IntoElement) -> impl IntoElement {
    div()
        .id(name)
        .flex_none()
        .w_full()
        .h(TITLE_STRIP_HEIGHT)
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count == 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
        })
        .child(content)
        .tag(name)
}

/// The words above a screen's display line, with the duck.
fn kicker(text: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(Icon::Duck.view(px(15.), theme.text_soft))
        .child(div().typography(Type::Label).text_color(theme.text_dim).child(text))
}

fn welcome(theme: &Theme) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .pl(CONTENT_INSET)
        .pr(px(48.))
        .child(div().h(relative(0.26)))
        .child(
            div()
                .max_w(px(560.))
                .flex()
                .flex_col()
                .child(kicker("Welcome to WackCode", theme))
                .child(
                    div()
                        .pt(px(10.))
                        .typography(Type::Display)
                        .text_color(theme.text)
                        .child("Open a folder to begin."),
                )
                .child(
                    div()
                        .pt(px(14.))
                        .typography(Type::Body)
                        .text_color(theme.text_soft)
                        .child(
                            "WackCode works inside a folder on your Mac: it reads, edits and runs \
                             what is there. Add one, then start a chat in it.",
                        ),
                )
                .child(
                    div()
                        .pt(px(28.))
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .child(
                            div()
                                .h(px(34.))
                                .px(px(16.))
                                .rounded(px(9.))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .bg(theme.accent)
                                .child(Icon::Folder.view(px(15.), theme.on_accent))
                                .child(
                                    div()
                                        .typography(Type::UiStrong)
                                        .text_color(theme.on_accent)
                                        .child("Add folder…"),
                                ),
                        )
                        .child(
                            div()
                                .typography(Type::Small)
                                .text_color(theme.text_faint)
                                .child("⌘O"),
                        ),
                ),
        )
}

fn new_chat(project: ProjectInfo, composer: ComposerData, theme: &Theme) -> impl IntoElement {
    let hint = |keys: &'static str, what: &'static str| {
        div()
            .flex()
            .gap(px(6.))
            .child(div().text_color(theme.text_dim).child(keys))
            .child(div().text_color(theme.text_faint).child(what))
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .pl(CONTENT_INSET)
        .pr(px(48.))
        .child(div().h(relative(0.22)))
        .child(
            div()
                .max_w(px(640.))
                .flex()
                .flex_col()
                .child(kicker("New chat in", theme))
                .child(
                    div()
                        .pt(px(6.))
                        .typography(Type::Display)
                        .text_color(theme.text)
                        .child(project.name.clone()),
                )
                .child(project_line(&project, theme).pt(px(8.)))
                .child(div().pt(px(30.)).child(Composer::new(composer)))
                .child(
                    div()
                        .pt(px(12.))
                        .px(px(16.))
                        .flex()
                        .gap(px(20.))
                        .typography(Type::Small)
                        .child(hint("↩", "send"))
                        .child(hint("⇧↩", "new line"))
                        .child(hint("⌘P", "switch project")),
                ),
        )
}

fn project_line(project: &ProjectInfo, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .typography(Type::Mono)
        .text_color(theme.text_dim)
        .child(project.path.clone())
        .when_some(project.branch.clone(), |line, branch| {
            line.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(Icon::Branch.view(px(13.), theme.text_faint))
                    .child(branch),
            )
        })
}

fn chat_strip(title: SharedString, project: ProjectInfo, theme: &Theme) -> gpui::Div {
    div()
        .size_full()
        .pl(CONTENT_INSET)
        .pr(px(20.))
        .flex()
        .items_center()
        .gap(px(12.))
        .child(div().typography(Type::UiStrong).text_color(theme.text).child(title))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .typography(Type::Small)
                .text_color(theme.text_faint)
                .child(project.name)
                .when_some(project.branch, |line, branch| {
                    line.child("·").child(branch)
                }),
        )
}

fn chat(items: Vec<Item>, composer: ComposerData, theme: &Theme) -> impl IntoElement {
    let clear = Hsla { a: 0., ..theme.window };
    // Room for the docked composer, so the last line of the transcript is never under it.
    let dock_height = px(150.) + px(46.) * composer.queued.len() as f32;
    div()
        .size_full()
        .relative()
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .justify_end()
                .pl(CONTENT_INSET)
                .pr(px(48.))
                .pb(dock_height)
                .child(div().max_w(TRANSCRIPT_MAX_WIDTH).child(Transcript::new(items))),
        )
        // Text scrolls out under the title strip through a short fade, not a hard edge.
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(28.))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(theme.window, 0.),
                    linear_color_stop(clear, 1.),
                )),
        )
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .pl(CONTENT_INSET - px(16.))
                .pr(px(32.))
                .pt(px(36.))
                .pb(px(18.))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(clear, 0.),
                    linear_color_stop(theme.window, 0.3),
                ))
                .child(div().max_w(TRANSCRIPT_MAX_WIDTH + px(32.)).child(Composer::new(composer))),
        )
}
