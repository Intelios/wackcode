//! Settings › Prompts: every built-in system prompt across WackCode, grouped by what it steers,
//! with a dedicated page to read, copy, customise or restore each one.
//!
//! - **Overview**: three grouped cards (*Chats*, *Planning & reasoning*, *Background & helpers*),
//!   each row showing its mark (Ultra's pixel wildcard in the fixed prism for Ultra Plan and
//!   Ultra, or its icon on the accent), title, `Default` / `Customised` badge, character and line
//!   counts, and one-line description. When two or more prompts are customised, the head offers
//!   *Restore all defaults* behind an in-page confirmation.
//! - **Prompt page**: opens directly into a tall monospace editor prefilled with the current text
//!   so reading, copying and editing happen in one place. Beneath the field, a locked note names
//!   the app-owned guide or wrapper that stays around the prompt, followed by a live 20,000
//!   character meter, *Restore default* (with inline confirmation when customised), *Cancel* and
//!   *Save*. When the prompt is customised or edited away from WackCode's text, a collapsible
//!   *Built-in prompt* card below lets the user inspect or copy the shipped default without losing
//!   their draft.
//!
//! The open [`PromptEditor`] lives in [`PromptsState`] on `AppState`, so switching sections keeps
//! an unsaved draft until the user saves, cancels or restores.

use std::f32::consts::PI;

use gpui::{AnyElement, App, AppContext, Entity, Hsla, SharedString, Transformation, div, prelude::*, px, radians};
use wack_core::{MAX_PROMPT_CHARS, PromptConfig, PromptGroup, PromptId};

use crate::{
    actions::{
        AskRestoreAllPrompts, AskRestorePrompt, CancelRestoreAllPrompts, CancelRestorePrompt, ClosePrompt, ConfirmRestoreAllPrompts,
        ConfirmRestorePrompt, OpenPrompt, SavePrompt, TogglePromptDefault,
    },
    button::{Button, Kind as ButtonKind},
    copy::CopyButton,
    icon::Icon,
    settings::text_area,
    settings_kit::{card, dispatch, pill, staged_head},
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    typography::{Type, Typography},
    ultra::UltraMark,
};

/// One prompt open on its sub-page.
pub struct PromptEditor {
    pub id: PromptId,
    pub body: Entity<TextField>,
    pub confirming_restore: bool,
    pub show_default: bool,
    pub error: Option<SharedString>,
}

impl PromptEditor {
    pub fn new(id: PromptId, config: &PromptConfig, cx: &mut App) -> Self {
        let initial = config.effective(id);
        let body = cx.new(|cx| {
            let mut field = TextField::new(Type::Mono, "Write the instructions…", cx).multiline(22);
            field.set_text_at_start(&initial, cx);
            field
        });
        Self {
            id,
            body,
            confirming_restore: false,
            show_default: false,
            error: None,
        }
    }

    pub fn view(&self, config: &PromptConfig, cx: &App) -> PromptEditorData {
        let text = self.body.read(cx).text();
        let trimmed = text.trim();
        let chars = trimmed.chars().count();
        let lines = if text.is_empty() { 0 } else { text.lines().count().max(1) };
        let saved = config.effective(self.id);
        let default_text = self.id.default_text();
        let customised = config.is_customised(self.id);
        let dirty = trimmed != saved.trim();
        let differs_from_default = trimmed != default_text.trim();
        PromptEditorData {
            id: self.id,
            body: self.body.clone(),
            current_text: text.to_string().into(),
            default_text: default_text.into_owned().into(),
            customised,
            dirty,
            blank: trimmed.is_empty(),
            chars,
            lines,
            differs_from_default,
            show_default: self.show_default,
            confirming_restore: self.confirming_restore,
            error: self.error.clone(),
        }
    }
}

#[derive(Clone)]
pub struct PromptEditorData {
    pub id: PromptId,
    pub body: Entity<TextField>,
    pub current_text: SharedString,
    pub default_text: SharedString,
    pub customised: bool,
    pub dirty: bool,
    pub blank: bool,
    pub chars: usize,
    pub lines: usize,
    pub differs_from_default: bool,
    pub show_default: bool,
    pub confirming_restore: bool,
    pub error: Option<SharedString>,
}

impl PromptEditorData {
    pub fn over_limit(&self) -> bool {
        self.chars > MAX_PROMPT_CHARS
    }

    pub fn can_save(&self) -> bool {
        !self.blank && !self.over_limit() && self.dirty
    }
}

/// Settings › Prompts' state on `AppState`.
#[derive(Default)]
pub struct PromptsState {
    pub editor: Option<PromptEditor>,
    pub confirming_restore_all: bool,
    pub error: Option<SharedString>,
}

impl PromptsState {
    pub fn data(&self, config: &PromptConfig, unrestricted_planning: bool, cx: &App) -> PromptsData {
        PromptsData {
            config: config.clone(),
            unrestricted_planning,
            editor: self.editor.as_ref().map(|editor| editor.view(config, cx)),
            confirming_restore_all: self.confirming_restore_all,
            error: self.error.clone(),
        }
    }
}

#[derive(Clone, Default)]
pub struct PromptsData {
    pub config: PromptConfig,
    pub unrestricted_planning: bool,
    pub editor: Option<PromptEditorData>,
    pub confirming_restore_all: bool,
    pub error: Option<SharedString>,
}

pub fn icon(id: PromptId) -> Icon {
    match id {
        PromptId::Code => Icon::Spark,
        PromptId::Chat => Icon::Comment,
        PromptId::Plan => Icon::Brain,
        PromptId::UltraPlan | PromptId::Ultra => Icon::Spark,
        PromptId::Compaction => Icon::Archive,
        PromptId::Subagent => Icon::Duck,
        PromptId::Team => Icon::Duck,
        PromptId::Title => Icon::Pencil,
        PromptId::StepSummary => Icon::Check,
    }
}

fn row_tag(id: PromptId) -> &'static str {
    match id {
        PromptId::Code => "prompt-row-code",
        PromptId::Chat => "prompt-row-chat",
        PromptId::Plan => "prompt-row-plan",
        PromptId::UltraPlan => "prompt-row-ultra-plan",
        PromptId::Ultra => "prompt-row-ultra",
        PromptId::Compaction => "prompt-row-compaction",
        PromptId::Subagent => "prompt-row-subagent",
        PromptId::Team => "prompt-row-team",
        PromptId::Title => "prompt-row-title",
        PromptId::StepSummary => "prompt-row-step-summary",
    }
}

/// Formats an integer with comma thousands separators (`7811` → `"7,811"`).
pub fn format_count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn counts_line(chars: usize, lines: usize) -> String {
    format!(
        "{} {} · {} {}",
        format_count(chars),
        if chars == 1 { "character" } else { "characters" },
        format_count(lines),
        if lines == 1 { "line" } else { "lines" },
    )
}

fn alpha(colour: Hsla, a: f32) -> Hsla {
    Hsla { a: (colour.a * a).clamp(0., 1.), ..colour }
}

/// The round mark at the left of a prompt row or head: Ultra's wildcard in the prism, or an icon.
fn prompt_mark(id: PromptId, element_id: &'static str, theme: &Theme) -> gpui::Div {
    if id.is_ultra() {
        div()
            .flex_none()
            .size(px(30.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(alpha(theme.ultra, 0.14))
            .border_1()
            .border_color(alpha(theme.ultra, 0.34))
            .child(UltraMark::new(element_id, px(14.)))
    } else {
        div()
            .flex_none()
            .size(px(30.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.accent_wash)
            .border_1()
            .border_color(theme.accent_line)
            .child(icon(id).view(px(14.), theme.accent_text))
    }
}

/// The `Default` or `Customised` pill beside a prompt's title.
fn status_badge(id: PromptId, customised: bool, theme: &Theme) -> gpui::Div {
    let (bg, border, ink, label) = match (customised, id.is_ultra()) {
        (false, _) => (theme.hover, None, theme.text_dim, "Default"),
        (true, true) => (alpha(theme.ultra, 0.16), Some(alpha(theme.ultra, 0.36)), theme.ultra, "Customised"),
        (true, false) => (theme.accent_wash, Some(theme.accent_line), theme.accent_text, "Customised"),
    };
    div()
        .flex_none()
        .h(px(18.))
        .px(px(7.))
        .rounded_full()
        .flex()
        .items_center()
        .bg(bg)
        .when_some(border, |pill, border| pill.border_1().border_color(border))
        .typography(Type::Small)
        .text_color(ink)
        .child(label)
}

pub fn page(data: PromptsData, theme: &Theme) -> AnyElement {
    if let Some(editor) = &data.editor {
        return editor_page(editor, theme).into_any_element();
    }
    let customised = data.config.customised_count();
    let restore_all = (customised >= 2 && !data.confirming_restore_all).then(|| {
        pill("prompts-restore-all", "Restore all defaults", Some(Icon::Reload), false, theme)
            .cursor_pointer().on_click(dispatch(AskRestoreAllPrompts))
            .tag("prompts-restore-all")
            .into_any_element()
    });
    let mut page = div().flex().flex_col().gap(px(18.)).child(staged_head(
        crate::settings::Section::Prompts,
        "Prompts",
        "The instructions WackCode starts every chat, plan, summary and helper with. Read them as they ship, or rewrite any of them; changes apply from the next message.",
        restore_all,
        theme,
    ));
    if let Some(error) = data.error {
        page = page.child(div().typography(Type::Ui).text_color(theme.danger).child(error));
    }
    if data.confirming_restore_all {
        page = page.child(restore_all_box(customised, theme));
    }
    if data.unrestricted_planning {
        page = page.child(
            div()
                .px(px(14.))
                .py(px(11.))
                .rounded(px(12.))
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .gap(px(10.))
                .child(Icon::Lock.view(px(14.), theme.warning))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .typography(Type::Small)
                        .text_color(theme.text_soft)
                        .child("Planning read-only restrictions are off in Settings › Tools. WackCode adds current access guidance after the Plan and Ultra Plan prompts; plan review and approval still apply."),
                )
                .tag("prompts-unrestricted-note"),
        );
    }
    for group in PromptGroup::ALL {
        let ids = group.prompts();
        let group_customised = ids.iter().filter(|&&id| data.config.is_customised(id)).count();
        let badge: Option<SharedString> = if group_customised > 0 {
            Some(format!("{group_customised} customised").into())
        } else {
            None
        };
        let mut section = card(Some(group.title()), badge, None, theme);
        for &id in ids {
            section = section.child(prompt_row(id, &data.config, theme));
        }
        page = page.child(section);
    }
    page.into_any_element()
}

fn restore_all_box(count: usize, theme: &Theme) -> impl IntoElement {
    div()
        .p(px(16.))
        .rounded(px(14.))
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border_strong)
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().typography(Type::UiStrong).text_color(theme.text).child("Restore all built-in prompts?"))
                .child(div().typography(Type::Ui).text_color(theme.text_dim).child(format!(
                    "This discards your {count} customised prompts and returns every prompt to WackCode’s built-in text. Chats pick up the built-in prompts from their next message."
                ))),
        )
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(Button::new("prompts-restore-all-keep", "Keep", ButtonKind::Secondary).action(CancelRestoreAllPrompts).tag("prompts-restore-all-keep"))
                .child(
                    Button::new("prompts-restore-all-confirm", "Restore all", ButtonKind::Danger)
                        .action(ConfirmRestoreAllPrompts)
                        .tag("prompts-restore-all-confirm"),
                ),
        )
        .tag("prompts-restore-all-box")
}

fn prompt_row(id: PromptId, config: &PromptConfig, theme: &Theme) -> impl IntoElement {
    let effective = config.effective(id);
    let customised = config.is_customised(id);
    let chars = effective.chars().count();
    let lines = effective.lines().count().max(1);
    let mark_id = match id {
        PromptId::UltraPlan => "prompt-mark-ultra-plan",
        PromptId::Ultra => "prompt-mark-ultra",
        _ => "prompt-mark",
    };
    div()
        .id(SharedString::from(format!("prompt-{}", id.slug())))
        .px(px(18.))
        .py(px(13.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(14.))
        .cursor_pointer()
        .hover(|row| row.bg(theme.hover))
        .on_click(dispatch(OpenPrompt { id }))
        .child(prompt_mark(id, mark_id, theme))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(div().typography(Type::UiStrong).text_color(theme.text).child(id.title()))
                        .child(status_badge(id, customised, theme))
                        .child(div().typography(Type::Small).text_color(theme.text_faint).child(counts_line(chars, lines))),
                )
                .child(div().typography(Type::Small).text_color(theme.text_dim).child(id.description())),
        )
        .child(Icon::ChevronRight.view(px(14.), theme.text_faint))
        .tag(row_tag(id))
}

fn editor_page(editor: &PromptEditorData, theme: &Theme) -> impl IntoElement {
    let id = editor.id;
    let back = div()
        .id("back-prompts")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .pb(px(18.))
        .cursor_pointer()
        .typography(Type::Ui)
        .text_color(theme.text_dim)
        .hover(|link| link.text_color(theme.text))
        .on_click(dispatch(ClosePrompt))
        .child(Icon::ChevronRight.view(px(13.), theme.text_faint).with_transformation(Transformation::rotate(radians(PI))))
        .child("All prompts")
        .tag("back-prompts");

    let head = div()
        .flex()
        .items_start()
        .gap(px(16.))
        .pb(px(22.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(prompt_mark(id, "prompt-editor-mark", theme))
                        .child(div().typography(Type::Title).text_color(theme.text).child(id.title()))
                        .child(status_badge(id, editor.customised, theme)),
                )
                .child(div().max_w(px(580.)).typography(Type::Body).text_color(theme.text_soft).child(id.description())),
        )
        .child(
            div()
                .flex_none()
                .pt(px(6.))
                .child(CopyButton::new(format!("prompt-{}", id.slug()), editor.current_text.clone(), "Copy prompt", "prompt-copy")),
        );

    // The editor's own row holds the meter and the way back to the built-in text; Save and
    // Cancel sit in the fixed footer (`save_bar`) so small windows keep them in reach.
    let fill = (editor.chars as f32 / MAX_PROMPT_CHARS as f32).clamp(0., 1.);
    let meter_colour = if editor.over_limit() {
        theme.danger
    } else if fill >= 0.9 {
        theme.warning
    } else if id.is_ultra() {
        theme.ultra
    } else {
        theme.accent_text
    };
    let meter = div()
        .w(px(88.))
        .h(px(5.))
        .rounded_full()
        .bg(theme.hover)
        .overflow_hidden()
        .child(div().h_full().w(px(88. * fill)).rounded_full().bg(meter_colour));
    let count_ink = if editor.over_limit() { theme.danger } else { theme.text_dim };
    let mut tools = div()
        .px(px(18.))
        .py(px(10.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(10.))
        .child(meter)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .typography(Type::Small)
                .text_color(count_ink)
                .child(format!(
                    "{} / {} characters · {} {}",
                    format_count(editor.chars),
                    format_count(MAX_PROMPT_CHARS),
                    format_count(editor.lines),
                    if editor.lines == 1 { "line" } else { "lines" },
                )),
        );
    if editor.customised && !editor.confirming_restore {
        tools = tools.child(Button::new("prompt-restore", "Restore default", ButtonKind::Secondary).action(AskRestorePrompt).tag("prompt-restore"));
    }

    let mut editor_card = card(Some("Prompt instructions"), None, Some("Applies from each chat’s next message"), theme)
        .child(
            div()
                .px(px(18.))
                .py(px(14.))
                .border_t_1()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(text_area(editor.body.clone(), 300., theme).tag("prompt-body"))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .child(div().pt(px(2.)).child(Icon::Lock.view(px(13.), theme.text_faint)))
                        .child(div().flex_1().min_w_0().typography(Type::Small).text_color(theme.text_dim).child(id.wrapper_note())),
                ),
        );
    if editor.confirming_restore {
        editor_card = editor_card.child(restore_prompt_box(id, theme));
    }
    editor_card = editor_card.child(tools);

    let reference = (editor.customised || editor.differs_from_default).then(|| default_reference_card(editor, theme));

    div()
        .flex()
        .flex_col()
        .child(back)
        .child(head)
        .child(editor_card)
        .children(reference.map(|card| div().pt(px(18.)).child(card)))
}

/// The editor's fixed footer beneath the scrolling page, as Providers' connection editor has
/// one: Save and Cancel stay in reach however small the window. It shows once the text differs
/// from what is saved, or a save failed.
pub fn save_bar(editor: &PromptEditorData, narrow: bool, theme: &Theme) -> Option<AnyElement> {
    if !editor.dirty && editor.error.is_none() {
        return None;
    }
    let status: AnyElement = if let Some(error) = &editor.error {
        div().typography(Type::Ui).text_color(theme.danger).child(error.clone()).into_any_element()
    } else if editor.blank {
        div()
            .typography(Type::Small)
            .text_color(theme.text_dim)
            .child("Write the prompt, or use Restore default to return to WackCode’s text.")
            .into_any_element()
    } else {
        div()
            .flex()
            .items_center()
            .gap(px(7.))
            .child(div().size(px(6.)).rounded_full().bg(theme.accent_text))
            .child(div().typography(Type::Small).text_color(theme.text_dim).child("Unsaved changes — apply from each chat’s next message"))
            .into_any_element()
    };
    Some(
        div()
            .flex_none()
            .w_full()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .child(
                div()
                    .w_full()
                    .px(px(if narrow { 24. } else { 48. }))
                    .py(px(12.))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(crate::settings_kit::PAGE_WIDTH))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().flex_1().min_w_0().child(status))
                            .child(Button::new("prompt-cancel", "Cancel", ButtonKind::Secondary).action(ClosePrompt).tag("prompt-cancel"))
                            .child(Button::new("prompt-save", "Save", ButtonKind::Primary).action(SavePrompt).enabled(editor.can_save()).tag("prompt-save")),
                    ),
            )
            .into_any_element(),
    )
}

fn restore_prompt_box(id: PromptId, theme: &Theme) -> impl IntoElement {
    div()
        .mx(px(18.))
        .mb(px(12.))
        .p(px(14.))
        .rounded(px(12.))
        .bg(theme.window)
        .border_1()
        .border_color(theme.border_strong)
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().typography(Type::UiStrong).text_color(theme.text).child("Restore the default prompt?"))
                .child(div().typography(Type::Ui).text_color(theme.text_dim).child(format!(
                    "This discards your custom {} prompt and returns to WackCode’s built-in text. Chats pick up the built-in text from their next message.",
                    id.title()
                ))),
        )
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(Button::new("prompt-restore-keep", "Keep", ButtonKind::Secondary).action(CancelRestorePrompt).tag("prompt-restore-keep"))
                .child(Button::new("prompt-restore-confirm", "Restore", ButtonKind::Danger).action(ConfirmRestorePrompt).tag("prompt-restore-confirm")),
        )
        .tag("prompt-restore-box")
}

fn default_reference_card(editor: &PromptEditorData, theme: &Theme) -> impl IntoElement {
    let open = editor.show_default;
    let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
    let header = div()
        .id("prompt-toggle-default")
        .px(px(18.))
        .py(px(12.))
        .flex()
        .items_center()
        .gap(px(10.))
        .cursor_pointer()
        .hover(|row| row.bg(theme.hover))
        .on_click(dispatch(TogglePromptDefault))
        .child(chevron.view(px(14.), theme.text_dim))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .typography(Type::UiStrong)
                .text_color(theme.text)
                .child(if open { "Hide built-in prompt" } else { "Show built-in prompt" }),
        )
        .child(div().typography(Type::Small).text_color(theme.text_faint).child("WackCode’s default for reference"))
        .tag("prompt-toggle-default");

    let mut reference = card(None, None, None, theme).child(header);
    if open {
        reference = reference.child(
            div()
                .px(px(18.))
                .py(px(14.))
                .border_t_1()
                .border_color(theme.border)
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(div().flex_1().typography(Type::Label).text_color(theme.text_dim).child("Built-in text"))
                        .child(CopyButton::new(
                            format!("prompt-default-{}", editor.id.slug()),
                            editor.default_text.clone(),
                            "Copy built-in",
                            "prompt-copy-default",
                        )),
                )
                .child(
                    div()
                        .id("prompt-default-scroll")
                        .w_full()
                        .max_h(px(340.))
                        .overflow_y_scroll()
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(9.))
                        .bg(theme.window)
                        .border_1()
                        .border_color(theme.border)
                        .typography(Type::Mono)
                        .text_color(theme.text_soft)
                        .child(editor.default_text.clone())
                        .tag("prompt-default-text"),
                ),
        );
    }
    reference
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_format_with_thousands_separators() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(347), "347");
        assert_eq!(format_count(7_811), "7,811");
        assert_eq!(format_count(20_000), "20,000");
        assert_eq!(format_count(1_234_567), "1,234,567");
        assert_eq!(counts_line(1, 1), "1 character · 1 line");
        assert_eq!(counts_line(7_811, 54), "7,811 characters · 54 lines");
    }
}
