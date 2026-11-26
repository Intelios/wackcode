//! Settings: one section at a time on the stage, its navigation in the sidebar's pane.
//!
//! The navigation groups the sections (Look, Models, Agent, Reach) and lists those still to come,
//! dimmed, so it is laid out for the dozen-odd sections planned. Each page is a head and cards of
//! rows, built from `settings_kit.rs`.
//!
//! Settings › Providers is here, after version 2: an overview of the connections to model APIs,
//! each with an on / off switch (off keeps the key and models, out of the model menu), and a page
//! per connection that edits its address, its key and its models. A new connection opens the same
//! page, and a bar at the foot saves or discards what changed. The model catalog is not here yet,
//! so a model is its id, its context window if the user knows it, and, as in version 2, a
//! Reasoning switch with the efforts its endpoint accepts (the chat's effort menu offers only
//! those). A new model does not reason; switching Reasoning on starts it at Off, Low, Medium and
//! High, version 2's default.
//!
//! A key is typed into a masked field, handed to the store once, and the field is cleared: it is
//! never drawn, copied or kept anywhere else (docs/security.md). Editing a connection with a saved
//! key leaves the field empty: blank keeps the key, and the form never asks the store for it until
//! the user asks to see it with the eye beside the field. The eye shows the saved key in place,
//! plainly; until it is edited it is still the saved key, so saving keeps it rather than replacing
//! it, and hiding it puts the field back to empty bullets. A key typed into the field can be
//! looked at the same way. Leaving Settings puts the field away again.
//! Adding or saving a connection sends nothing over the network.
//!
//! The form's fields live in [`ProviderForm`], owned by the app state, so leaving Settings
//! half-way keeps what was typed. The page itself draws from plain data and dispatches actions.

use std::{collections::HashSet, f32::consts::PI};

use gpui::{AnyElement, App, AppContext, Entity, Pixels, SharedString, Transformation, div, prelude::*, px, radians};
use wack_core::{Connection, DEFAULT_EFFORTS, Model, NewConnection, Reasoning, WireFormat};

use crate::{
    actions::{
        AddModelRow, ChooseFormat, CloseConnectionForm, CloseSettings, DiscardConnection, EditConnection, OpenConnectionForm,
        OpenSettingsSection, RemoveConnection, RemoveModelRow, SaveConnection, SwitchConnection, ToggleConnectionKey,
        ToggleModelEffort, ToggleModelReasoning, ToggleModelRow, ToggleModelVision,
    },
    appearance_page::{self, AppearanceData},
    button::{Button, Kind},
    icon::Icon,
    settings_kit::{PAGE_WIDTH, card, dispatch, pill, segmented, staged_head},
    tags::Tag,
    text_field::TextField,
    theme::Theme,
    tools_page::switch,
    typography::{Type, Typography},
};

/// One model in the editor: its fields, and what reasoning its endpoint accepts.
#[derive(Clone)]
pub struct ModelField {
    pub id: Entity<TextField>,
    pub context: Entity<TextField>,
    /// The most it writes in one reply; optional, a ceiling on what requests ask for.
    pub output: Entity<TextField>,
    /// The Reasoning switch. Off saves no efforts: the model does not reason.
    pub reasoning: bool,
    /// The efforts ticked while Reasoning is on, low to high.
    pub efforts: Vec<Reasoning>,
    /// The Vision switch: whether the model is sent pictures. A new row accepts.
    pub accepts_images: bool,
    /// The row is open to its fields, as version 2's were. A new row opens; a saved model's
    /// row starts folded to one line.
    pub open: bool,
    /// The model as last read, for the folded line.
    pub summary: Option<ModelSummary>,
}

/// A model row's folded line: its id, context and max output as typed.
#[derive(Clone, Default)]
pub struct ModelSummary {
    pub id: SharedString,
    pub context: SharedString,
    pub output: SharedString,
}

/// A list of efforts as saved: sorted, and empty when it reasons at nothing above Off.
fn saved_efforts(efforts: &[Reasoning]) -> Vec<Reasoning> {
    let mut efforts: Vec<Reasoning> = efforts.iter().copied().filter(|effort| !effort.is_ultra()).collect();
    efforts.sort();
    efforts.dedup();
    if efforts.iter().all(|effort| *effort == Reasoning::Off) { Vec::new() } else { efforts }
}

/// The new or edited connection, kept while the user looks elsewhere.
pub struct ProviderForm {
    /// The saved connection being edited; `None` while adding one.
    pub editing: Option<String>,
    pub name: Entity<TextField>,
    pub address: Entity<TextField>,
    pub key: Entity<TextField>,
    /// The saved key the field is showing for a look. While it is what the field holds, the key
    /// is unchanged: saving sends none, and the form is not dirty.
    revealed_key: Option<String>,
    pub format: WireFormat,
    pub models: Vec<ModelField>,
    pub error: Option<SharedString>,
    pub notice: Option<SharedString>,
    /// Tells a save's notice from a later one, so the first one's timer cannot hide it.
    pub notice_serial: u64,
    /// The connection editor is on the page.
    pub open: bool,
}

impl ProviderForm {
    pub fn new(cx: &mut App) -> Self {
        Self {
            editing: None,
            name: cx.new(|cx| TextField::new(Type::Ui, "Work, Local, OpenRouter…", cx)),
            address: cx.new(|cx| TextField::new(Type::Ui, format_placeholder(WireFormat::OpenaiChat), cx)),
            key: cx.new(|cx| TextField::new(Type::Ui, "Paste the key", cx).masked()),
            revealed_key: None,
            format: WireFormat::OpenaiChat,
            models: Vec::new(),
            error: None,
            notice: None,
            notice_serial: 0,
            open: false,
        }
    }

    /// Every field on the page, for the app to watch as they change.
    pub fn all_fields(&self) -> Vec<Entity<TextField>> {
        let mut fields = vec![self.name.clone(), self.address.clone(), self.key.clone()];
        for model in &self.models {
            fields.push(model.id.clone());
            fields.push(model.context.clone());
            fields.push(model.output.clone());
        }
        fields
    }

    /// A blank editor, ready for the first model.
    pub fn open_new(&mut self, cx: &mut App) {
        self.clear(cx);
        self.open = true;
        self.set_format(WireFormat::OpenaiChat, cx);
        self.models.push(Self::model_field(cx));
    }

    /// The editor showing a saved connection. Its key field stays empty: blank keeps the key.
    pub fn edit(&mut self, connection: &Connection, cx: &mut App) {
        self.clear(cx);
        self.open = true;
        self.editing = Some(connection.id.clone());
        self.set_format(connection.format, cx);
        self.name.update(cx, |field, cx| field.set_text(&connection.name, cx));
        self.address.update(cx, |field, cx| field.set_text(&connection.base_url, cx));
        for model in &connection.models {
            let context = model.context_window.map(tokens_text);
            self.add_model(&model.id, context.as_deref(), cx);
            if let Some(row) = self.models.last_mut() {
                if let Some(output) = model.max_output {
                    row.output.update(cx, |field, cx| field.set_text(&tokens_text(output), cx));
                }
                row.open = false;
                row.accepts_images = model.accepts_images;
                if model.reasons() {
                    row.reasoning = true;
                    row.efforts = model.levels();
                }
            }
        }
        if self.models.is_empty() {
            self.models.push(Self::model_field(cx));
        }
    }

    /// Changes the wire format, and the address placeholder that goes with it.
    pub fn set_format(&mut self, format: WireFormat, cx: &mut App) {
        self.format = format;
        self.address.update(cx, |field, cx| field.set_placeholder(format_placeholder(format), cx));
    }

    pub fn add_model(&mut self, id: &str, context: Option<&str>, cx: &mut App) {
        let field = Self::model_field(cx);
        if !id.is_empty() {
            field.id.update(cx, |field, cx| field.set_text(id, cx));
        }
        if let Some(context) = context {
            field.context.update(cx, |field, cx| field.set_text(context, cx));
        }
        self.models.push(field);
    }

    /// The Reasoning switch of one model: on starts at version 2's default efforts.
    pub fn toggle_reasoning(&mut self, index: usize) {
        if let Some(row) = self.models.get_mut(index) {
            row.reasoning = !row.reasoning;
            row.efforts = if row.reasoning { DEFAULT_EFFORTS.to_vec() } else { Vec::new() };
        }
    }

    /// The Vision switch of one model.
    pub fn toggle_vision(&mut self, index: usize) {
        if let Some(row) = self.models.get_mut(index) {
            row.accepts_images = !row.accepts_images;
        }
    }

    /// Ticks or unticks one effort. Unticking the last leaves Off.
    pub fn toggle_effort(&mut self, index: usize, effort: Reasoning) {
        if let Some(row) = self.models.get_mut(index) {
            match row.efforts.iter().position(|listed| *listed == effort) {
                Some(at) => {
                    row.efforts.remove(at);
                }
                None => row.efforts.push(effort),
            }
            row.efforts.sort();
            if row.efforts.is_empty() {
                row.efforts.push(Reasoning::Off);
            }
        }
    }

    pub fn remove_model(&mut self, index: usize) {
        if index < self.models.len() {
            self.models.remove(index);
        }
    }

    /// The rows the user left empty are not part of the connection.
    pub fn prune_blank_models(&mut self, cx: &App) {
        self.models.retain(|row| !row.id.read(cx).text().trim().is_empty());
    }

    /// Back to the overview: the form is forgotten, not kept for next time.
    pub fn close(&mut self, cx: &mut App) {
        self.clear(cx);
    }

    fn clear(&mut self, cx: &mut App) {
        for field in [&self.name, &self.address] {
            field.update(cx, |field, cx| field.set_text("", cx));
        }
        self.clear_key(cx);
        self.models.clear();
        self.editing = None;
        self.format = WireFormat::OpenaiChat;
        self.address.update(cx, |field, cx| field.set_placeholder(format_placeholder(WireFormat::OpenaiChat), cx));
        self.error = None;
        self.notice = None;
        self.notice_serial = self.notice_serial.wrapping_add(1);
        self.open = false;
    }

    /// The field holds the saved key, only being looked at: neither a change nor a replacement.
    fn key_is_saved(&self, cx: &App) -> bool {
        let text = self.key.read(cx).text();
        self.revealed_key.as_deref().is_some_and(|saved| saved == text.trim())
    }

    /// Puts a saved key in the field plainly, for the user to look at. Until it is edited it is
    /// still the saved key: saving sends none and the form stays as it was.
    pub fn show_saved_key(&mut self, key: &str, cx: &mut App) {
        self.revealed_key = Some(key.to_string());
        self.key.update(cx, |field, cx| {
            field.set_text(key, cx);
            field.set_masked(false, cx);
        });
    }

    /// The eye at the key field: shows what is there, or hides it again. Hiding a saved key that
    /// was only being looked at forgets it; a key the user typed stays, masked.
    pub fn toggle_key_reveal(&mut self, cx: &mut App) {
        if self.key.read(cx).is_masked() {
            self.key.update(cx, |field, cx| field.set_masked(false, cx));
        } else if self.key_is_saved(cx) {
            self.clear_key(cx);
        } else {
            self.key.update(cx, |field, cx| field.set_masked(true, cx));
        }
    }

    /// The key field empty and masked, with nothing remembered from a reveal.
    pub fn clear_key(&mut self, cx: &mut App) {
        self.revealed_key = None;
        self.key.update(cx, |field, cx| {
            field.set_text("", cx);
            field.set_masked(true, cx);
        });
    }

    /// Leaving Settings masks the field again. A saved key that was shown is forgotten; a key the
    /// user typed stays, hidden, and is still there on the way back.
    pub fn put_key_away(&mut self, cx: &mut App) {
        if self.key_is_saved(cx) {
            self.key.update(cx, |field, cx| field.set_text("", cx));
        }
        self.revealed_key = None;
        self.key.update(cx, |field, cx| field.set_masked(true, cx));
    }

    fn model_field(cx: &mut App) -> ModelField {
        ModelField {
            id: cx.new(|cx| TextField::new(Type::Mono, "model-id", cx)),
            context: cx.new(|cx| TextField::new(Type::Ui, "200k", cx)),
            output: cx.new(|cx| TextField::new(Type::Ui, "32k", cx)),
            reasoning: false,
            efforts: Vec::new(),
            accepts_images: true,
            open: true,
            summary: None,
        }
    }

    /// Opens or folds one model's row.
    pub fn toggle_open(&mut self, index: usize) {
        if let Some(row) = self.models.get_mut(index) {
            row.open = !row.open;
        }
    }

    fn values(&self, cx: &App) -> ProviderValues {
        let text = |field: &Entity<TextField>| field.read(cx).text().trim().to_string();
        ProviderValues {
            name: text(&self.name),
            address: text(&self.address),
            key: text(&self.key),
            format: self.format,
            models: self
                .models
                .iter()
                .map(|row| ModelValue {
                    id: text(&row.id),
                    context: text(&row.context),
                    output: text(&row.output),
                    efforts: if row.reasoning { saved_efforts(&row.efforts) } else { Vec::new() },
                    accepts_images: row.accepts_images,
                })
                .filter(|model| !model.id.is_empty() || !model.context.is_empty() || !model.output.is_empty())
                .collect(),
        }
    }

    /// What the form describes and its key, or the first thing to fix, as a sentence.
    pub fn read(&self, cx: &App, key_required: bool) -> Result<(NewConnection, String), String> {
        let values = self.values(cx);
        if values.name.is_empty() {
            return Err("Give the connection a name.".to_string());
        }
        if values.address.is_empty() {
            return Err("Enter the address of the API.".to_string());
        }
        let mut models = Vec::new();
        let mut seen = HashSet::new();
        for row in &values.models {
            if row.id.is_empty() {
                return Err("Give each model its id, or remove the row.".to_string());
            }
            let context_window = if row.context.is_empty() {
                None
            } else {
                Some(parse_tokens(&row.context).ok_or_else(|| {
                    format!("The context window for {} must be a number of tokens, such as 200000 or 200k.", row.id)
                })?)
            };
            let max_output = if row.output.is_empty() {
                None
            } else {
                Some(parse_tokens(&row.output).ok_or_else(|| {
                    format!("The max output for {} must be a number of tokens, such as 32000 or 32k.", row.id)
                })?)
            };
            if !seen.insert(row.id.clone()) {
                return Err(format!("The model {} is listed twice.", row.id));
            }
            models.push(Model {
                id: row.id.clone(),
                context_window,
                efforts: row.efforts.clone(),
                accepts_images: row.accepts_images,
                max_output,
            });
        }
        if models.is_empty() {
            return Err("Add at least one model.".to_string());
        }
        let key = key_change(&values.key, self.revealed_key.as_deref()).unwrap_or_default().to_string();
        if key_required && key.is_empty() {
            return Err("Enter the API key.".to_string());
        }
        Ok((
            NewConnection {
                name: values.name,
                format: values.format,
                base_url: values.address,
                models,
            },
            key,
        ))
    }

    /// Enough is filled in to try saving.
    fn complete(&self, cx: &App, has_saved_key: bool) -> bool {
        let values = self.values(cx);
        !values.name.is_empty()
            && !values.address.is_empty()
            && values.models.iter().any(|model| !model.id.is_empty())
            && (has_saved_key || !values.key.is_empty())
    }

    /// The form differs from the saved connection. A new connection always does.
    fn dirty(&self, cx: &App, saved: Option<&Connection>) -> bool {
        let Some(saved) = saved else {
            return true;
        };
        let values = self.values(cx);
        if key_change(&values.key, self.revealed_key.as_deref()).is_some()
            || values.format != saved.format
            || values.name != saved.name
            || values.address.trim_end_matches('/') != saved.base_url
        {
            return true;
        }
        if values.models.iter().any(|model| model.id.is_empty() && (!model.context.is_empty() || !model.output.is_empty())) {
            return true;
        }
        let models: Vec<&ModelValue> = values.models.iter().filter(|model| !model.id.is_empty()).collect();
        if models.len() != saved.models.len() {
            return true;
        }
        models.iter().zip(&saved.models).any(|(row, saved)| {
            row.id != saved.id
                || row.efforts != saved_efforts(&saved.efforts)
                || row.accepts_images != saved.accepts_images
                || tokens_changed(&row.context, saved.context_window)
                || tokens_changed(&row.output, saved.max_output)
        })
    }
}

struct ProviderValues {
    name: String,
    address: String,
    key: String,
    format: WireFormat,
    models: Vec<ModelValue>,
}

/// The key a save sends: what the user typed, except a saved key they only asked to see.
fn key_change<'a>(typed: &'a str, shown_saved: Option<&str>) -> Option<&'a str> {
    (!typed.is_empty() && Some(typed) != shown_saved).then_some(typed)
}

/// A token count as typed differs from the saved one.
fn tokens_changed(text: &str, saved: Option<u64>) -> bool {
    match (text, saved) {
        ("", None) => false,
        (text, Some(saved)) => parse_tokens(text) != Some(saved),
        _ => true,
    }
}

struct ModelValue {
    id: String,
    context: String,
    output: String,
    efforts: Vec<Reasoning>,
    accepts_images: bool,
}

/// "200000", "200k", "1M", "1.5m" in tokens.
pub fn parse_tokens(text: &str) -> Option<u64> {
    let text = text.trim().replace(['_', ','], "");
    let (number, scale) = match text.chars().last()?.to_ascii_lowercase() {
        'k' => (&text[..text.len() - 1], 1_000.),
        'm' => (&text[..text.len() - 1], 1_000_000.),
        _ => (text.as_str(), 1.),
    };
    let value: f64 = number.trim().parse().ok()?;
    (value > 0.).then(|| (value * scale).round() as u64)
}

pub fn format_name(format: WireFormat) -> &'static str {
    match format {
        WireFormat::OpenaiChat => "Chat Completions",
        WireFormat::OpenaiResponses => "OpenAI Responses",
        WireFormat::AnthropicMessages => "Anthropic Messages",
    }
}

fn format_short(format: WireFormat) -> &'static str {
    match format {
        WireFormat::OpenaiChat => "Chat Completions",
        WireFormat::OpenaiResponses => "Responses",
        WireFormat::AnthropicMessages => "Anthropic",
    }
}

fn format_hint(format: WireFormat) -> &'static str {
    match format {
        WireFormat::OpenaiChat => "Most servers speak this. /chat/completions is added to the address for you.",
        WireFormat::OpenaiResponses => "OpenAI’s Responses API. /responses is added to the address for you.",
        WireFormat::AnthropicMessages => "Anthropic’s Messages API. /v1/messages is added for you; a pasted /v1 is kept.",
    }
}

fn format_placeholder(format: WireFormat) -> &'static str {
    match format {
        WireFormat::OpenaiChat | WireFormat::OpenaiResponses => "https://api.example.com/v1",
        WireFormat::AnthropicMessages => "https://api.anthropic.com",
    }
}

/// The host a connection talks to, for the overview and the editor's head.
fn host_of(url: &str) -> String {
    let without_scheme = url.trim().split_once("://").map(|(_, rest)| rest).unwrap_or(url.trim());
    without_scheme.split('/').next().unwrap_or(without_scheme).to_string()
}

/// A context window as the editor shows it, in a form [`parse_tokens`] reads back exactly.
fn tokens_text(tokens: u64) -> String {
    if tokens >= 1_000_000 && tokens.is_multiple_of(1_000_000) {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1_000 && tokens.is_multiple_of(1_000) {
        format!("{}k", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

/// A connection as its row shows it.
#[derive(Clone, Debug)]
pub struct ConnectionRow {
    pub connection: Connection,
    /// A key is kept for it. The key itself is not part of a row.
    pub has_key: bool,
}

/// The connection editor, as the page shows it.
#[derive(Clone)]
pub struct ProviderEditor {
    /// The saved connection being edited; `None` while adding one.
    pub editing: Option<String>,
    /// Whether a saved connection is switched on; `None` while adding one.
    pub enabled: Option<bool>,
    pub has_saved_key: bool,
    /// The key field is showing its text instead of bullets.
    pub key_revealed: bool,
    /// What it shows is the saved key, only being looked at.
    pub key_from_store: bool,
    /// The key field holds something: what was typed, or the saved key.
    pub key_has_text: bool,
    /// There is a key to look at: a saved one, or one typed into the field.
    pub key_present: bool,
    pub name: Entity<TextField>,
    pub address: Entity<TextField>,
    pub key: Entity<TextField>,
    pub format: WireFormat,
    pub models: Vec<ModelField>,
    pub title: SharedString,
    pub host: SharedString,
    pub model_count: usize,
    pub error: Option<SharedString>,
    pub notice: Option<SharedString>,
    pub dirty: bool,
    pub complete: bool,
}

/// Everything the page shows.
#[derive(Clone)]
pub struct ProvidersData {
    pub connections: Vec<ConnectionRow>,
    pub editor: Option<ProviderEditor>,
    /// A failure from the overview: switching or removing a connection.
    pub error: Option<SharedString>,
}

impl ProvidersData {
    pub fn new(connections: &[Connection], has_key: impl Fn(&str) -> bool, form: &ProviderForm, cx: &App) -> Self {
        let rows: Vec<ConnectionRow> = connections
            .iter()
            .map(|connection| ConnectionRow { connection: connection.clone(), has_key: has_key(&connection.id) })
            .collect();
        let editor = form.open.then(|| {
            let saved = form.editing.as_deref().and_then(|id| rows.iter().find(|row| row.connection.id == id)).cloned();
            let values = form.values(cx);
            let title: SharedString = if values.name.is_empty() { "New connection".into() } else { values.name.clone().into() };
            let host = host_of(&values.address);
            let host: SharedString = if host.is_empty() { "No address yet".into() } else { host.into() };
            let key = form.key.read(cx);
            let key_revealed = !key.is_masked();
            let key_from_store = form.key_is_saved(cx);
            ProviderEditor {
                editing: form.editing.clone(),
                enabled: saved.as_ref().map(|row| row.connection.enabled),
                has_saved_key: saved.as_ref().is_some_and(|row| row.has_key),
                key_revealed,
                key_from_store,
                key_present: saved.as_ref().is_some_and(|row| row.has_key) || !values.key.is_empty(),
                key_has_text: !values.key.is_empty(),
                name: form.name.clone(),
                address: form.address.clone(),
                key: form.key.clone(),
                format: form.format,
                // Each row's summary from its own fields: `values` leaves blank rows out, so
                // pairing the two lists would shift every summary after a blank row.
                models: form
                    .models
                    .iter()
                    .map(|row| {
                        let text = |field: &Entity<TextField>| SharedString::from(field.read(cx).text().trim().to_string());
                        let summary = ModelSummary { id: text(&row.id), context: text(&row.context), output: text(&row.output) };
                        ModelField { summary: Some(summary), ..row.clone() }
                    })
                    .collect(),
                title,
                host,
                model_count: values.models.iter().filter(|model| !model.id.is_empty()).count(),
                error: form.error.clone(),
                notice: form.notice.clone(),
                dirty: form.dirty(cx, saved.as_ref().map(|row| &row.connection)),
                complete: form.complete(cx, saved.as_ref().is_some_and(|row| row.has_key)),
            }
        });
        Self {
            connections: rows,
            editor,
            error: form.error.clone().filter(|_| !form.open),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Appearance,
    Providers,
    Prompts,
    Tools,
    Memory,
    Skills,
    Commands,
    Subagents,
    Git,
    Integrations,
    Mcp,
    Web,
    Browser,
    Computer,
}

impl Section {
    pub const ALL: [Section; 14] = [
        Section::Appearance,
        Section::Providers,
        Section::Prompts,
        Section::Tools,
        Section::Memory,
        Section::Skills,
        Section::Commands,
        Section::Subagents,
        Section::Git,
        Section::Integrations,
        Section::Mcp,
        Section::Web,
        Section::Browser,
        Section::Computer,
    ];

    /// The section's icon in the navigation, as version 2 drew them.
    pub fn icon(self) -> Icon {
        match self {
            Section::Appearance => Icon::Palette,
            Section::Providers => Icon::Key,
            Section::Prompts => Icon::Pencil,
            Section::Tools => Icon::Hammer,
            Section::Memory => Icon::Brain,
            Section::Skills => Icon::Book,
            Section::Commands => Icon::Slash,
            Section::Subagents => Icon::Duck,
            Section::Git => Icon::Branch,
            Section::Integrations => Icon::Plug,
            Section::Mcp => Icon::Plug,
            Section::Web => Icon::Globe,
            Section::Browser => Icon::Window,
            Section::Computer => Icon::Cursor,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Providers => "Providers",
            Section::Prompts => "Prompts",
            Section::Tools => "Tools",
            Section::Memory => "Memory",
            Section::Skills => "Skills",
            Section::Commands => "Commands",
            Section::Subagents => "Sub-agents",
            Section::Git => "Git",
            Section::Integrations => "Integrations",
            Section::Mcp => "MCP servers",
            Section::Web => "Web",
            Section::Browser => "Browser",
            Section::Computer => "Computer use",
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Section::Appearance => "section-appearance",
            Section::Providers => "section-providers",
            Section::Prompts => "section-prompts",
            Section::Tools => "section-tools",
            Section::Memory => "section-memory",
            Section::Skills => "section-skills",
            Section::Commands => "section-commands",
            Section::Subagents => "section-subagents",
            Section::Git => "section-git",
            Section::Integrations => "section-integrations",
            Section::Mcp => "section-mcp",
            Section::Web => "section-web",
            Section::Browser => "section-browser",
            Section::Computer => "section-computer",
        }
    }
}

/// The navigation's groups. `None` is a section still to come: listed, dimmed, not yet a place
/// to go, so the shape of Settings is clear from the start and the list never reflows as they
/// arrive.
/// A section in the navigation: where it goes (none yet for one to come), and its name.
type Entry = (Option<Section>, &'static str);

const GROUPS: [(&str, &[Entry]); 4] = [
    ("Look", &[(Some(Section::Appearance), "Appearance")]),
    ("Models", &[(Some(Section::Providers), "Providers"), (Some(Section::Prompts), "Prompts")]),
    (
        "Agent",
        &[(Some(Section::Tools), "Tools"), (Some(Section::Memory), "Memory"), (Some(Section::Skills), "Skills"), (Some(Section::Commands), "Commands"), (Some(Section::Subagents), "Sub-agents"), (Some(Section::Git), "Git")],
    ),
    (
        "Reach",
        &[(Some(Section::Integrations), "Integrations"), (Some(Section::Mcp), "MCP servers"), (Some(Section::Web), "Web"), (Some(Section::Browser), "Browser"), (Some(Section::Computer), "Computer use")],
    ),
];

/// Everything the Settings page shows.
#[derive(Clone)]
pub struct SettingsData {
    pub section: Section,
    pub providers: ProvidersData,
    pub appearance: AppearanceData,
    pub prompts: crate::prompts_page::PromptsData,
    pub tools: crate::tools_page::ToolsData,
    pub memory: crate::memory_page::MemoryData,
    pub skills: crate::skills_page::SkillsData,
    pub commands: crate::commands_page::CommandsData,
    pub subagents: crate::subagents_page::SubagentsData,
    pub git: crate::git_page::GitData,
    pub integrations: crate::integrations_page::IntegrationsData,
    pub web: crate::web_page::WebData,
    pub mcp: crate::mcp_page::McpData,
    pub browser: crate::browser_page::BrowserData,
    pub computer: crate::computer_page::ComputerData,
}

/// The title strip's words: `Settings › Providers`.
/// Over a picture the strip's words sit on `ground`, the panes' glass, so they read on the frame.
pub fn strip(section: Section, inset: Pixels, ground: Option<gpui::Hsla>, theme: &Theme) -> impl IntoElement {
    let pill = |content: gpui::Div| content.h(px(28.)).px(px(12.)).rounded_full().flex().items_center().when_some(ground, |pill, ground| pill.bg(ground));
    div()
        .size_full()
        .pl((inset - px(12.)).max(px(4.)))
        .pr(px(4.))
        .flex()
        .items_center()
        .typography(Type::UiStrong)
        .child(
            pill(div().gap(px(8.)))
                .child(div().text_color(theme.text_dim).child("Settings"))
                .child(div().text_color(theme.text_faint).child("›"))
                .child(div().text_color(theme.text).child(section.name())),
        )
        .child(div().flex_1())
        .child(pill(div()).typography(Type::Small).text_color(theme.text_faint).child("esc to close"))
}

/// The navigation, in the sidebar's pane while Settings is open: back to the chats, then the
/// sections in their groups. `connections` is counted beside Providers.
/// A connection as the navigation lists it under Providers: its id, name, whether it is on, and
/// whether its editor is the one open.
#[derive(Clone)]
pub struct NavConnection {
    pub id: String,
    pub name: SharedString,
    pub enabled: bool,
    pub open: bool,
}

impl NavConnection {
    pub fn list(providers: &ProvidersData) -> Vec<NavConnection> {
        let open = providers.editor.as_ref().and_then(|editor| editor.editing.clone());
        providers
            .connections
            .iter()
            .map(|row| NavConnection {
                id: row.connection.id.clone(),
                name: row.connection.name.clone().into(),
                enabled: row.connection.enabled,
                open: open.as_deref() == Some(row.connection.id.as_str()),
            })
            .collect()
    }
}

pub fn nav(section: Section, connections: Vec<NavConnection>, prompt_count: usize, theme: &Theme) -> impl IntoElement {
    let count = connections.len();
    let back = div()
        .id("settings-back")
        .h(px(30.))
        .px(px(10.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(8.))
        .cursor_pointer()
        .hover(|row| row.bg(theme.hover))
        .on_click(dispatch(CloseSettings))
        .child(Icon::ChevronRight.view(px(13.), theme.text_dim).with_transformation(Transformation::rotate(radians(PI))))
        .child(div().flex_1().typography(Type::Ui).text_color(theme.text_soft).child("Back to chats"))
        .child(div().typography(Type::Small).text_color(theme.text_faint).child("esc"))
        .tag("settings-back");

    let mut list = div().flex().flex_col().gap(px(1.)).px(px(10.)).pb(px(16.));
    for (group, entries) in GROUPS {
        list = list.child(div().px(px(10.)).pt(px(16.)).pb(px(4.)).typography(Type::Label).text_color(theme.text_dim).child(group));
        for (target, name) in entries.iter().copied() {
            let row = div().h(px(30.)).px(px(10.)).rounded_full().flex().items_center().gap(px(8.));
            list = list.child(match target {
                Some(target) => {
                    let lit = target == section;
                    row.id(SharedString::from(format!("section-{name}")))
                        .cursor_pointer()
                        .when(lit, |row| row.bg(theme.accent_wash))
                        .when(!lit, |row| row.hover(|row| row.bg(theme.hover)))
                        .on_click(dispatch(OpenSettingsSection { section: target }))
                        .child(target.icon().view(px(14.), if lit { theme.accent_text } else { theme.text_dim }))
                        .child(
                            div()
                                .flex_1()
                                .typography(if lit { Type::UiStrong } else { Type::Ui })
                                .text_color(if lit { theme.accent_text } else { theme.text_soft })
                                .child(name),
                        )
                        .when(target == Section::Providers && count > 0, |row| {
                            row.child(div().typography(Type::Small).text_color(theme.text_faint).child(count.to_string()))
                        })
                        .when(target == Section::Prompts && prompt_count > 0, |row| {
                            row.child(div().typography(Type::Small).text_color(theme.text_faint).child(prompt_count.to_string()))
                        })
                        .tag(target.tag())
                        .into_any_element()
                }
                None => row
                    .child(Icon::Pencil.view(px(14.), theme.text_faint))
                    .child(div().flex_1().typography(Type::Ui).text_color(theme.text_faint).child(name))
                    .child(div().typography(Type::Small).text_color(theme.text_faint).child("soon"))
                    .into_any_element(),
            });
            // On Providers the navigation opens up with every connection beneath it, as version
            // 2's did: each opens its editor.
            if target == Some(Section::Providers) && section == Section::Providers && !connections.is_empty() {
                let mut sub = div().flex().flex_col().gap(px(1.)).ml(px(17.)).pl(px(10.)).border_l_1().border_color(theme.border).my(px(2.));
                for connection in &connections {
                    let id = connection.id.clone();
                    sub = sub.child(
                        div()
                            .id(SharedString::from(format!("nav-connection-{}", connection.id)))
                            .h(px(26.))
                            .px(px(10.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .cursor_pointer()
                            .when(connection.open, |row| row.bg(theme.hover))
                            .when(!connection.open, |row| row.hover(|row| row.bg(theme.hover)))
                            .on_click(move |_, window, cx| window.dispatch_action(Box::new(EditConnection { id: id.clone() }), cx))
                            .child(div().flex_none().size(px(6.)).rounded_full().bg(if connection.enabled { theme.success } else { theme.text_faint }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .typography(Type::Small)
                                    .text_color(if connection.open { theme.text } else if connection.enabled { theme.text_soft } else { theme.text_faint })
                                    .child(connection.name.clone()),
                            ),
                    );
                }
                list = list.child(crate::motion::enter("nav-connections", sub));
            }
        }
    }
    div()
        .flex_1()
        .min_h_0()
        .w_full()
        .flex()
        .flex_col()
        .child(div().flex_none().px(px(10.)).pt(px(4.)).child(back))
        .child(div().flex_1().min_h_0().child(crate::pixel_scroll::PixelScroll::new("settings-nav", list)))
}

/// Below this stage width the connection editor's Name and API format stack, so the format
/// choice has room for all three of its words.
pub const FORM_STACK_BELOW: f32 = 820.;

/// The page, centred on the stage. With the sidebar put away the sections are a row of pills
/// above it instead (`inline_nav`), since the sidebar's pane is where they otherwise live.
/// A connection being edited keeps a save bar at the foot, below the scrolling page.
pub fn page(data: SettingsData, inline_nav: bool, narrow: bool, stack_form: bool, theme: &Theme) -> impl IntoElement {
    let section = data.section;
    let nav = inline_nav.then(|| {
        let choices: Vec<(Section, &'static str)> = Section::ALL.iter().map(|section| (*section, section.name())).collect();
        div().pb(px(26.)).flex().child(segmented("section", &choices, section, |section| Box::new(OpenSettingsSection { section }), theme))
    });
    let editor = data.providers.editor.clone();
    let prompt_editor = data.prompts.editor.clone();
    let content = match section {
        Section::Appearance => appearance_page::page(data.appearance, theme).into_any_element(),
        Section::Providers => providers(data.providers, stack_form, theme).into_any_element(),
        Section::Prompts => crate::prompts_page::page(data.prompts, theme),
        Section::Tools => crate::tools_page::page(data.tools, theme).into_any_element(),
        Section::Memory => crate::memory_page::page(data.memory, theme),
        Section::Skills => crate::skills_page::page(data.skills, theme),
        Section::Commands => crate::commands_page::page(data.commands, theme),
        Section::Subagents => crate::subagents_page::page(data.subagents, theme),
        Section::Git => crate::git_page::page(data.git, theme).into_any_element(),
        Section::Integrations => crate::integrations_page::page(data.integrations, theme).into_any_element(),
        Section::Mcp => crate::mcp_page::page(data.mcp, theme),
        Section::Web => crate::web_page::page(data.web, theme),
        Section::Browser => crate::browser_page::page(data.browser, theme).into_any_element(),
        Section::Computer => crate::computer_page::page(data.computer, theme).into_any_element(),
    };
    let footer = if section == Section::Providers {
        editor.as_ref().and_then(|editor| save_bar(editor, narrow, theme))
    } else if section == Section::Prompts {
        prompt_editor.as_ref().and_then(|editor| crate::prompts_page::save_bar(editor, narrow, theme))
    } else {
        None
    };
    let padding = px(if narrow { 24. } else { 48. });
    div()
        .id("settings-page")
        .size_full()
        .flex()
        .flex_col()
        .child(
            // Scrolled on whole pixels, so its cards and words move as one (`pixel_scroll.rs`).
            div().flex_1().min_h_0().w_full().child(crate::pixel_scroll::PixelScroll::new(
                "settings-body",
                div()
                    .w_full()
                    .px(padding)
                    .pt(px(28.))
                    .pb(px(64.))
                    .flex()
                    .justify_center()
                    .child(div().flex_1().min_w_0().max_w(px(PAGE_WIDTH)).flex().flex_col().children(nav).child(content)),
            )),
        )
        .children(footer)
}

fn providers(data: ProvidersData, stack_form: bool, theme: &Theme) -> impl IntoElement {
    match data.editor {
        Some(editor) => editor_page(&editor, stack_form, theme).into_any_element(),
        None => overview(data, theme).into_any_element(),
    }
}

/// The connections and the way to add one.
fn overview(data: ProvidersData, theme: &Theme) -> impl IntoElement {
    let empty = data.connections.is_empty();
    let on = data.connections.iter().filter(|row| row.connection.enabled).count();
    let mut list = None;
    if !empty {
        let mut connections = card(Some("Connections"), Some(format!("{on} on").into()), Some("Off keeps the key and models, out of the model menu"), theme);
        for (index, row) in data.connections.iter().enumerate() {
            connections = connections.child(connection_row(index, row, theme));
        }
        list = Some(connections);
    }
    div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .child(staged_head(Section::Providers, 
            "Providers",
            "Connections to model APIs. A key is kept in WackCode’s own secrets file and only ever sent to the address it belongs to.",
            None,
            theme,
        ))
        .when_some(data.error, |page, error| page.child(div().typography(Type::Ui).text_color(theme.danger).child(error)))
        .children(list)
        .child(add_connection(empty, theme))
}

fn add_connection(empty: bool, theme: &Theme) -> impl IntoElement {
    let title = if empty { "Connect your first model" } else { "Add a connection" };
    let note: SharedString = if empty {
        "WackCode needs at least one model to chat with. Add a connection with an API key.".into()
    } else {
        "Another provider, a gateway or a local server.".into()
    };
    let tile = div()
        .id("new-connection")
        .cursor_pointer()
        .p(px(14.))
        .rounded(px(12.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.window)
        .hover(|tile| tile.bg(theme.hover).border_color(theme.border_strong))
        .on_click(dispatch(OpenConnectionForm))
        .flex()
        .items_center()
        .gap(px(12.))
        .child(
            div()
                .flex_none()
                .size(px(34.))
                .rounded_full()
                .bg(theme.accent_wash)
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::Key.view(px(16.), theme.accent_text)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().typography(Type::UiStrong).text_color(theme.text).child("API key"))
                .child(div().typography(Type::Small).text_color(theme.text_dim).child(
                    "Chat Completions, Responses or Anthropic Messages: a provider’s API, a gateway, or a server on your Mac.",
                )),
        )
        .child(Icon::ChevronRight.view(px(14.), theme.text_faint))
        .tag("new-connection");
    card(Some(title), None, None, theme).child(
        div()
            .px(px(18.))
            .pb(px(18.))
            .flex()
            .flex_col()
            .gap(px(11.))
            .child(div().typography(Type::Small).text_color(theme.text_faint).child(note))
            .child(tile),
    )
}

/// Tags for the automation socket, which takes only fixed names.
const ROW_TAGS: [&str; 4] = ["connection-row-0", "connection-row-1", "connection-row-2", "connection-row-3"];
const SWITCH_TAGS: [&str; 4] = ["switch-connection-0", "switch-connection-1", "switch-connection-2", "switch-connection-3"];
const REMOVE_MODEL_TAGS: [&str; 12] = [
    "remove-model-0", "remove-model-1", "remove-model-2", "remove-model-3", "remove-model-4", "remove-model-5", "remove-model-6", "remove-model-7",
    "remove-model-8", "remove-model-9", "remove-model-10", "remove-model-11",
];
const MODEL_ID_TAGS: [&str; 12] = [
    "model-id-0", "model-id-1", "model-id-2", "model-id-3", "model-id-4", "model-id-5", "model-id-6", "model-id-7", "model-id-8", "model-id-9",
    "model-id-10", "model-id-11",
];
const MODEL_CONTEXT_TAGS: [&str; 12] = [
    "model-context-0", "model-context-1", "model-context-2", "model-context-3", "model-context-4", "model-context-5", "model-context-6",
    "model-context-7", "model-context-8", "model-context-9", "model-context-10", "model-context-11",
];
const MODEL_OUTPUT_TAGS: [&str; 12] = [
    "model-output-0", "model-output-1", "model-output-2", "model-output-3", "model-output-4", "model-output-5", "model-output-6",
    "model-output-7", "model-output-8", "model-output-9", "model-output-10", "model-output-11",
];

fn connection_row(index: usize, row: &ConnectionRow, theme: &Theme) -> impl IntoElement {
    let connection = &row.connection;
    let on = connection.enabled;
    let (state, state_ink) = if !on {
        ("Off", theme.text_faint)
    } else if row.has_key {
        ("Key saved", theme.text_faint)
    } else {
        ("No key", theme.warning)
    };
    let count = connection.models.len();
    let models = format!("{count} model{}", if count == 1 { "" } else { "s" });
    let id = connection.id.clone();
    let switch_id = connection.id.clone();
    div()
        .id(("connection", index))
        .px(px(18.))
        .py(px(13.))
        .cursor_pointer()
        .when(index > 0, |row| row.border_t_1().border_color(theme.border))
        .flex()
        .items_center()
        .gap(px(14.))
        .hover(|row| row.bg(theme.hover))
        .on_click(dispatch(EditConnection { id }))
        .child(monogram(&connection.name, on, false, theme))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .typography(Type::UiStrong)
                        .text_color(if on { theme.text } else { theme.text_dim })
                        .truncate()
                        .child(connection.name.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .min_w_0()
                        .child(div().flex_none().typography(Type::Small).text_color(theme.text_dim).child(format_name(connection.format)))
                        .child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child("·"))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .typography(Type::Mono)
                                .text_color(theme.text_faint)
                                .child(host_of(&connection.base_url)),
                        ),
                ),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .items_end()
                .gap(px(2.))
                .child(div().typography(Type::Small).text_color(state_ink).child(state))
                .child(div().typography(Type::Small).text_color(if count == 0 { theme.warning } else { theme.text_faint }).child(models)),
        )
        .child(Icon::ChevronRight.view(px(14.), theme.text_faint))
        .child(
            div()
                .id(("switch-connection", index))
                .flex_none()
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(Box::new(SwitchConnection { id: switch_id.clone(), on: !on }), cx);
                })
                .child(switch(on, theme))
                .tag(SWITCH_TAGS.get(index).copied().unwrap_or("switch-connection")),
        )
        .tag(ROW_TAGS.get(index).copied().unwrap_or("connection-row"))
}

/// One connection: its head, its fields, and its models.
fn editor_page(editor: &ProviderEditor, stack_form: bool, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .child(back_link(theme))
        .child(editor_head(editor, theme))
        .child(connection_card(editor, stack_form, theme))
        .child(div().pt(px(18.)).child(models_card(editor, theme)))
}

fn back_link(theme: &Theme) -> impl IntoElement {
    div()
        .id("back-connections")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .pb(px(18.))
        .cursor_pointer()
        .typography(Type::Ui)
        .text_color(theme.text_dim)
        .hover(|link| link.text_color(theme.text))
        .on_click(dispatch(CloseConnectionForm))
        .child(Icon::ChevronRight.view(px(13.), theme.text_faint).with_transformation(Transformation::rotate(radians(PI))))
        .child("All connections")
        .tag("back-connections")
}

fn editor_head(editor: &ProviderEditor, theme: &Theme) -> impl IntoElement {
    let switch = match (editor.editing.clone(), editor.enabled) {
        (Some(id), Some(on)) => Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(div().typography(Type::Small).text_color(theme.text_dim).child("Use this connection"))
                .child(
                    div()
                        .id("switch-editor")
                        .cursor_pointer()
                        .on_click(move |_, window, cx| window.dispatch_action(Box::new(SwitchConnection { id: id.clone(), on: !on }), cx))
                        .child(switch(on, theme))
                        .tag("switch-editor"),
                ),
        ),
        _ => None,
    };
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .pb(px(22.))
        .child(monogram(&editor.title, editor.enabled.unwrap_or(true), true, theme))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(div().typography(Type::Title).text_color(theme.text).truncate().child(editor.title.clone()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .min_w_0()
                        .child(div().flex_none().typography(Type::Small).text_color(theme.text_dim).child(format_name(editor.format)))
                        .child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child("·"))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .typography(Type::Mono)
                                .text_color(theme.text_faint)
                                .child(editor.host.clone()),
                        ),
                )
                .child(editor_status(editor, theme)),
        )
        .children(switch)
}

fn editor_status(editor: &ProviderEditor, theme: &Theme) -> impl IntoElement {
    let (state, ink) = match editor.enabled {
        Some(false) => ("Off", theme.text_dim),
        _ if editor.editing.is_some() && !editor.has_saved_key => ("No key", theme.warning),
        Some(_) => ("Key saved", theme.text_faint),
        None => ("Not saved yet", theme.text_faint),
    };
    let count = editor.model_count;
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(div().typography(Type::Small).text_color(ink).child(state))
        .child(div().typography(Type::Small).text_color(theme.text_faint).child("·"))
        .child(
            div()
                .typography(Type::Small)
                .text_color(if count == 0 { theme.warning } else { theme.text_faint })
                .child(format!("{count} model{}", if count == 1 { "" } else { "s" })),
        )
}

fn connection_card(editor: &ProviderEditor, stack_form: bool, theme: &Theme) -> impl IntoElement {
    let formats: Vec<(WireFormat, &'static str)> = [WireFormat::OpenaiChat, WireFormat::OpenaiResponses, WireFormat::AnthropicMessages]
        .map(|format| (format, format_short(format)))
        .to_vec();
    let formats = segmented("format", &formats, editor.format, |format| Box::new(ChooseFormat { format }), theme);
    let key_hint: SharedString = if editor.key_from_store {
        "The saved key. Change it to replace it.".into()
    } else if editor.has_saved_key && editor.key_has_text {
        "Typing a key replaces the saved one.".into()
    } else if editor.has_saved_key {
        "Leave blank to keep the saved key.".into()
    } else {
        "Stored on this Mac. Sent only to the address above.".into()
    };
    let mut fields = div().flex().gap(px(14.));
    fields = if stack_form { fields.flex_col() } else { fields.flex_row() };
    let mut card = card(Some("Connection"), None, Some("Where the API lives, and the key that reaches it"), theme).child(
        div()
            .px(px(18.))
            .pt(px(2.))
            .pb(px(18.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                fields
                    .child(field("Name", None, text_input(editor.name.clone(), "connection-name", theme), theme))
                    .child(field("API format", None, formats, theme)),
            )
            .child(field(
                "Base URL",
                Some(format_hint(editor.format).into()),
                text_input(editor.address.clone(), "connection-address", theme),
                theme,
            ))
            .child(field("API key", Some(key_hint), key_input(editor, theme), theme)),
    );
    if let Some(id) = editor.editing.clone() {
        card = card.child(
            div()
                .px(px(18.))
                .py(px(12.))
                .border_t_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .gap(px(12.))
                .child(div().flex_1().typography(Type::Small).text_color(theme.text_faint).child("Deleting removes the connection and its saved API key."))
                .child(Button::new("delete-connection", "Delete connection", Kind::Danger).action(RemoveConnection { id }).tag("delete-connection")),
        );
    }
    card
}

fn models_card(editor: &ProviderEditor, theme: &Theme) -> impl IntoElement {
    let count = editor.model_count;
    let badge: SharedString = format!("{count} model{}", if count == 1 { "" } else { "s" }).into();
    let mut card = card(Some("Models"), Some(badge), Some("Context is optional; it decides when chats compact"), theme);
    if editor.models.is_empty() {
        card = card.child(div().px(px(18.)).pt(px(2.)).pb(px(14.)).typography(Type::Ui).text_color(theme.text_dim).child("No models yet. Add at least one below."));
    } else {
        for (index, row) in editor.models.iter().enumerate() {
            card = card.child(model_row(index, row, theme));
        }
    }
    card.child(
        div()
            .px(px(18.))
            .py(px(12.))
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .child(pill("add-model", "Add model", Some(Icon::Plus), false, theme).on_click(dispatch(AddModelRow)).tag("add-model")),
    )
}

/// The width of a model's context and max output fields.
const TOKENS_WIDTH: f32 = 112.;

fn model_row(index: usize, row: &ModelField, theme: &Theme) -> impl IntoElement {
    let ModelSummary { id, context, output } = row.summary.clone().unwrap_or_default();
    let name: SharedString = if id.is_empty() { "New model".into() } else { id };
    let limits: SharedString = match (context.is_empty(), output.is_empty()) {
        (false, false) => format!("{context} context · {output} out").into(),
        (false, true) => format!("{context} context").into(),
        (true, false) => format!("{output} out").into(),
        (true, true) => SharedString::default(),
    };
    let badge = |word: &'static str| {
        div().flex_none().h(px(18.)).px(px(7.)).rounded_full().flex().items_center().bg(theme.hover).typography(Type::Small).text_color(theme.text_soft).child(word)
    };
    // The folded line, as version 2's: a chevron, the model, its context and what it can do.
    let head = div()
        .id(("model-row", index))
        .flex()
        .items_center()
        .gap(px(9.))
        .cursor_pointer()
        .on_click(dispatch(ToggleModelRow { index }))
        .child(
            Icon::ChevronRight
                .view(px(12.), theme.text_dim)
                .with_transformation(gpui::Transformation::rotate(gpui::radians(if row.open { std::f32::consts::FRAC_PI_2 } else { 0. }))),
        )
        .child(div().min_w_0().truncate().typography(Type::Mono).text_color(theme.text).child(name))
        .child(div().flex_1())
        .when(!limits.is_empty(), |head| head.child(div().flex_none().typography(Type::Small).text_color(theme.text_dim).child(limits)))
        .when(row.reasoning, |head| head.child(badge("Reasoning")))
        .when(row.accepts_images, |head| head.child(badge("Vision")))
        .when(!row.open, |head| head.child(
            div()
                .id(("remove-model-head", index))
                .flex_none()
                .size(px(26.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .hover(|button| button.bg(theme.hover))
                .tooltip(crate::tooltip::tip("Remove this model"))
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(Box::new(RemoveModelRow { index }), cx);
                })
                .child(Icon::Trash.view(px(14.), theme.text_dim)),
        ));
    div()
        .px(px(18.))
        .py(px(10.))
        .border_t_1()
        .border_color(theme.border)
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(head)
        .when(row.open, |body| {
            body.child(crate::motion::enter(
                gpui::ElementId::NamedInteger("model-row-open".into(), index as u64),
                div().flex().flex_col().gap(px(8.)).pl(px(21.)).child(model_fields(index, row, theme)).child(reasoning_line(index, row, theme)),
            ))
        })
}

/// Tags for the automation socket, which takes only fixed names.
const VISION_TAGS: [&str; 4] = ["model-vision-0", "model-vision-1", "model-vision-2", "model-vision-3"];

/// The Vision switch: off and the model is never sent pictures (screenshots, image files, the
/// user's attachments), and is told they were left out.
fn vision_toggle(index: usize, row: &ModelField, theme: &Theme) -> impl IntoElement {
    div()
        .id(("model-vision", index))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .cursor_pointer()
        .tooltip(crate::tooltip::tip("Whether this model is sent pictures: screenshots, image files and your attachments"))
        .on_click(dispatch(ToggleModelVision { index }))
        .child(switch(row.accepts_images, theme))
        .child(div().typography(Type::Ui).text_color(if row.accepts_images { theme.text_soft } else { theme.text_dim }).child("Vision"))
        .tag(VISION_TAGS.get(index).copied().unwrap_or("model-vision"))
}

/// Tags for the automation socket, which takes only fixed names.
const REASONING_TAGS: [&str; 4] = ["model-reasoning-0", "model-reasoning-1", "model-reasoning-2", "model-reasoning-3"];

/// The Reasoning switch and, while it is on, the efforts the endpoint accepts.
fn reasoning_line(index: usize, row: &ModelField, theme: &Theme) -> impl IntoElement {
    let toggle = div()
        .id(("model-reasoning", index))
        .flex()
        .items_center()
        .gap(px(8.))
        .cursor_pointer()
        .on_click(dispatch(ToggleModelReasoning { index }))
        .child(switch(row.reasoning, theme))
        .child(div().typography(Type::Ui).text_color(if row.reasoning { theme.text_soft } else { theme.text_dim }).child("Reasoning"))
        .tag(REASONING_TAGS.get(index).copied().unwrap_or("model-reasoning"));
    let chips = row.reasoning.then(|| {
        div().flex().flex_wrap().items_center().gap(px(6.)).children(Reasoning::LEVELS.iter().map(|&effort| {
            let on = row.efforts.contains(&effort);
            div()
                .id(SharedString::from(format!("effort-{index}-{}", effort.label())))
                .h(px(24.))
                .px(px(10.))
                .rounded_full()
                .flex()
                .items_center()
                .cursor_pointer()
                .typography(Type::Small)
                .when(on, |chip| chip.bg(theme.accent_wash).text_color(theme.accent_text))
                .when(!on, |chip| chip.border_1().border_color(theme.border_strong).text_color(theme.text_dim).hover(|chip| chip.bg(theme.hover)))
                .on_click(dispatch(ToggleModelEffort { index, effort }))
                .child(effort.label().to_lowercase())
        }))
    });
    let only_off = row.reasoning && row.efforts.iter().all(|effort| *effort == Reasoning::Off);
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(14.))
                .child(vision_toggle(index, row, theme))
                .child(div().h(px(16.)).w(px(1.)).bg(theme.border))
                .child(toggle)
                .when_some(chips, |line, chips| line.child(div().h(px(16.)).w(px(1.)).bg(theme.border)).child(chips)),
        )
        .when(only_off, |line| {
            line.child(div().typography(Type::Small).text_color(theme.text_faint).child("Only off is ticked. Tick the efforts this endpoint accepts."))
        })
}

fn model_fields(index: usize, row: &ModelField, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(div().flex_1().min_w_0().child(text_input(
            row.id.clone(),
            MODEL_ID_TAGS.get(index).copied().unwrap_or("model-id"),
            theme,
        )))
        .child(div().w(px(TOKENS_WIDTH)).flex_none().child(tokens_input(
            row.context.clone(),
            "context",
            "The model's context window in tokens. It decides when chats compact.",
            MODEL_CONTEXT_TAGS.get(index).copied().unwrap_or("model-context"),
            theme,
        )))
        .child(div().w(px(TOKENS_WIDTH)).flex_none().child(tokens_input(
            row.output.clone(),
            "out",
            "The most the model writes in one reply. Requests never ask for more.",
            MODEL_OUTPUT_TAGS.get(index).copied().unwrap_or("model-output"),
            theme,
        )))
        .child(
            div()
                .id(("remove-model", index))
                .flex_none()
                .size(px(26.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .hover(|button| button.bg(theme.hover))
                .tooltip(crate::tooltip::tip("Remove this model"))
                .on_click(dispatch(RemoveModelRow { index }))
                .child(Icon::Trash.view(px(14.), theme.text_dim))
                .tag(REMOVE_MODEL_TAGS.get(index).copied().unwrap_or("remove-model")),
        )
}

/// A labelled control in the connection card.
pub(crate) fn field(label: &'static str, hint: Option<SharedString>, control: impl IntoElement, theme: &Theme) -> impl IntoElement {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(7.))
        .child(div().typography(Type::Ui).text_color(theme.text_soft).child(label))
        .child(control)
        .when_some(hint, |block, hint| block.child(div().typography(Type::Small).text_color(theme.text_faint).child(hint)))
}

/// A field's ground, as round as the app's other controls but wide enough for an address.
pub(crate) fn text_input(field: Entity<TextField>, tag: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .w_full()
        .h(px(34.))
        .px(px(11.))
        .rounded(px(9.))
        .bg(theme.window)
        .border_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .child(field)
        .tag(tag)
}

/// A token count's field: [`text_input`]'s ground with its unit written faintly at the end, so
/// two numbers side by side can't be mistaken for each other.
fn tokens_input(field: Entity<TextField>, unit: &'static str, tip: &'static str, tag: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .id(tag)
        .w_full()
        .h(px(34.))
        .pl(px(11.))
        .pr(px(10.))
        .rounded(px(9.))
        .bg(theme.window)
        .border_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .gap(px(6.))
        .tooltip(crate::tooltip::tip(tip))
        .child(div().flex_1().min_w_0().child(field))
        .child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(unit))
        .tag(tag)
}

/// The key's field: its ground, and the eye that shows or hides what it holds. The eye is there
/// only when there is something to look at, as version 2 had it.
fn key_input(editor: &ProviderEditor, theme: &Theme) -> impl IntoElement {
    let eye = editor.key_present.then(|| {
        let (icon, tip) = if editor.key_revealed {
            (Icon::EyeOff, "Hide the API key")
        } else if editor.has_saved_key && !editor.key_has_text {
            (Icon::Eye, "Show the saved API key")
        } else {
            (Icon::Eye, "Show the API key")
        };
        div()
            .id("reveal-key")
            .flex_none()
            .size(px(24.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|button| button.bg(theme.hover))
            .tooltip(crate::tooltip::tip(tip))
            .on_click(dispatch(ToggleConnectionKey))
            .child(icon.view(px(14.), if editor.key_revealed { theme.text_soft } else { theme.text_dim }))
            .tag("reveal-key")
    });
    div()
        .w_full()
        .h(px(34.))
        .pl(px(11.))
        .pr(px(5.))
        .rounded(px(9.))
        .bg(theme.window)
        .border_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .child(div().flex_1().min_w_0().child(editor.key.clone()))
        .children(eye)
        .tag("connection-key")
}

/// A tall field's ground (a description, a prompt): the whole box takes a click, not only the
/// lines already written.
pub(crate) fn text_area(field: Entity<TextField>, min_height: f32, theme: &Theme) -> gpui::Div {
    let target = field.clone();
    div()
        .w_full()
        .min_h(px(min_height))
        .px(px(11.))
        .py(px(9.))
        .rounded(px(9.))
        .bg(theme.window)
        .border_1()
        .border_color(theme.border)
        .cursor_text()
        .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
            // The shell's root would take focus back as the click bubbles up.
            cx.stop_propagation();
            let handle = gpui::Focusable::focus_handle(target.read(cx), cx);
            window.focus(&handle, cx);
        })
        .child(field)
}

/// The bar under the page while there is something to save, or a word about the last save.
fn save_bar(editor: &ProviderEditor, narrow: bool, theme: &Theme) -> Option<AnyElement> {
    if !editor.dirty && editor.error.is_none() && editor.notice.is_none() {
        return None;
    }
    let status: AnyElement = if let Some(error) = &editor.error {
        div().typography(Type::Ui).text_color(theme.danger).child(error.clone()).into_any_element()
    } else if let Some(notice) = &editor.notice {
        div().typography(Type::Ui).text_color(theme.success).child(notice.clone()).into_any_element()
    } else {
        div()
            .flex()
            .items_center()
            .gap(px(7.))
            .child(div().size(px(6.)).rounded_full().bg(theme.accent_text))
            .child(
                div()
                    .typography(Type::Small)
                    .text_color(theme.text_dim)
                    .child(if editor.editing.is_some() { "Unsaved changes" } else { "Not saved yet" }),
            )
            .into_any_element()
    };
    let buttons = editor.dirty.then(|| {
        let (cancel, cancel_tag) = if editor.editing.is_some() {
            (Button::new("discard-connection", "Discard", Kind::Secondary).action(DiscardConnection), "discard-connection")
        } else {
            (Button::new("cancel-connection", "Cancel", Kind::Secondary).action(CloseConnectionForm), "cancel-connection")
        };
        let primary = if editor.editing.is_some() { "Save connection" } else { "Create connection" };
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(cancel.tag(cancel_tag))
            .child(
                Button::new("save-connection", primary, Kind::Primary)
                    .action(SaveConnection)
                    .enabled(editor.complete)
                    .tag("save-connection"),
            )
    });
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
                            .max_w(px(PAGE_WIDTH))
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(div().flex_1().min_w_0().child(status))
                            .children(buttons),
                    ),
            )
            .into_any_element(),
    )
}

/// A round monogram: the name's first letter.
fn monogram(name: &str, on: bool, large: bool, theme: &Theme) -> impl IntoElement {
    let letter: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default();
    div()
        .flex_none()
        .size(px(if large { 42. } else { 30. }))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(if on { theme.accent_wash } else { theme.hover })
        .typography(if large { Type::Title } else { Type::UiStrong })
        .text_color(if on { theme.accent_text } else { theme.text_faint })
        .child(letter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_token_counts() {
        assert_eq!(parse_tokens("200000"), Some(200_000));
        assert_eq!(parse_tokens("200k"), Some(200_000));
        assert_eq!(parse_tokens("1M"), Some(1_000_000));
        assert_eq!(parse_tokens("1.5m"), Some(1_500_000));
        assert_eq!(parse_tokens("128_000"), Some(128_000));
        assert_eq!(parse_tokens("lots"), None);
        assert_eq!(parse_tokens("0"), None);
        assert_eq!(parse_tokens(""), None);
    }

    #[test]
    fn context_windows_read_back_exactly() {
        for tokens in [128_000, 200_000, 1_000_000, 1_048_576, 999, 64_000] {
            assert_eq!(parse_tokens(&tokens_text(tokens)), Some(tokens), "{tokens}");
        }
        assert_eq!(tokens_text(1_000_000), "1M");
        assert_eq!(tokens_text(128_000), "128k");
    }

    #[test]
    fn a_saved_key_shown_for_a_look_is_not_a_typed_one() {
        assert_eq!(key_change("sk-typed", None), Some("sk-typed"));
        assert_eq!(key_change("", None), None);
        assert_eq!(key_change("", Some("sk-saved")), None);
        assert_eq!(key_change("sk-saved", Some("sk-saved")), None);
        assert_eq!(key_change("sk-replacement", Some("sk-saved")), Some("sk-replacement"));
    }

    #[test]
    fn a_host_is_the_address_without_its_scheme_or_path() {
        assert_eq!(host_of("https://openrouter.ai/api/v1"), "openrouter.ai");
        assert_eq!(host_of("http://localhost:1234/v1"), "localhost:1234");
        assert_eq!(host_of("api.example.com/v1"), "api.example.com");
        assert_eq!(host_of(" https://api.anthropic.com "), "api.anthropic.com");
    }
}
