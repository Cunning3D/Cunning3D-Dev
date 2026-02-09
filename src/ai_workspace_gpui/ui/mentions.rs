//! @mentions and /slash commands autocomplete system.
use gpui::{AnyElement, App, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, prelude::*, px};
use super::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Spacing};

// ─────────────────────────────────────────────────────────────────────────────
// Mention Types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum MentionKind {
    File { path: String },
    Directory { path: String },
    Symbol { name: String, path: String },
    Selection,
    Diagnostics { errors: bool, warnings: bool },
    Thread { id: uuid::Uuid, title: String },
    Image { id: u64 },
    Url { url: String },
}

impl MentionKind {
    pub fn icon(&self) -> &'static str {
        match self {
            Self::File { .. } => "F",
            Self::Directory { .. } => "D",
            Self::Symbol { .. } => "S",
            Self::Selection => "SEL",
            Self::Diagnostics { .. } => "WARN",
            Self::Thread { .. } => "TH",
            Self::Image { .. } => "IMG",
            Self::Url { .. } => "URL",
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::File { path } => {
                let name = std::path::Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path);
                format!("@{}", name)
            }
            Self::Directory { path } => {
                let name = std::path::Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path);
                format!("@{}/", name)
            }
            Self::Symbol { name, .. } => format!("@{}", name),
            Self::Selection => "@selection".into(),
            Self::Diagnostics { errors, warnings } => {
                match (errors, warnings) {
                    (true, true) => "@diagnostics".into(),
                    (true, false) => "@errors".into(),
                    (false, true) => "@warnings".into(),
                    _ => "@diagnostics".into(),
                }
            }
            Self::Thread { title, .. } => format!("@thread:{}", title),
            Self::Image { id } => format!("@image:{}", id),
            Self::Url { url } => format!("@{}", url),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Slash Command
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub args_hint: Option<String>,
}

impl SlashCommand {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self { name: name.into(), description: description.into(), args_hint: None }
    }
    pub fn with_args(mut self, hint: impl Into<String>) -> Self { self.args_hint = Some(hint.into()); self }
}

pub fn default_slash_commands() -> Vec<SlashCommand> {
    vec![
        SlashCommand::new("file", "Include a file").with_args("<path>"),
        SlashCommand::new("folder", "Include a folder").with_args("<path>"),
        SlashCommand::new("symbol", "Include a symbol").with_args("<name>"),
        SlashCommand::new("selection", "Include current selection"),
        SlashCommand::new("diagnostics", "Include diagnostics"),
        SlashCommand::new("fetch", "Fetch URL content").with_args("<url>"),
        SlashCommand::new("thread", "Reference another thread").with_args("<id>"),
        SlashCommand::new("clear", "Clear conversation"),
        SlashCommand::new("model", "Switch model").with_args("<model>"),
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// Autocomplete Menu
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum AutocompleteItem {
    Mention(MentionKind),
    Command(SlashCommand),
}

impl AutocompleteItem {
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Mention(m) => m.icon(),
            Self::Command(_) => "/",
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Mention(m) => m.label(),
            Self::Command(c) => format!("/{}", c.name),
        }
    }

    pub fn description(&self) -> Option<String> {
        match self {
            Self::Mention(_) => None,
            Self::Command(c) => Some(c.description.clone()),
        }
    }

    pub fn hint(&self) -> Option<String> {
        match self {
            Self::Mention(_) => None,
            Self::Command(c) => c.args_hint.clone(),
        }
    }
}

pub struct AutocompleteMenu {
    items: Vec<AutocompleteItem>,
    selected_idx: usize,
    focus_handle: FocusHandle,
    query: String,
}

#[derive(Clone, Debug)]
pub enum AutocompleteMenuEvent { Confirm(AutocompleteItem), Dismiss }

impl EventEmitter<AutocompleteMenuEvent> for AutocompleteMenu {}

impl AutocompleteMenu {
    pub fn new(query: &str, cx: &mut Context<Self>) -> Self {
        let items = Self::filter_items(query);
        Self { items, selected_idx: 0, focus_handle: cx.focus_handle(), query: query.to_string() }
    }

    pub fn update_query(&mut self, query: &str, cx: &mut Context<Self>) {
        self.query = query.to_string();
        self.items = Self::filter_items(query);
        self.selected_idx = 0;
        cx.notify();
    }

    fn filter_items(query: &str) -> Vec<AutocompleteItem> {
        if query.starts_with('/') {
            let cmd_query = &query[1..].to_lowercase();
            default_slash_commands().into_iter()
                .filter(|c| c.name.to_lowercase().starts_with(cmd_query) || cmd_query.is_empty())
                .map(AutocompleteItem::Command)
                .collect()
        } else if query.starts_with('@') {
            let mention_query = &query[1..].to_lowercase();
            let mut items = vec![
                AutocompleteItem::Mention(MentionKind::Selection),
                AutocompleteItem::Mention(MentionKind::Diagnostics { errors: true, warnings: true }),
            ];
            items.retain(|i| i.label().to_lowercase().contains(mention_query) || mention_query.is_empty());
            items
        } else {
            vec![]
        }
    }

    pub fn select_next(&mut self, cx: &mut Context<Self>) {
        if !self.items.is_empty() { self.selected_idx = (self.selected_idx + 1) % self.items.len(); cx.notify(); }
    }

    pub fn select_prev(&mut self, cx: &mut Context<Self>) {
        if !self.items.is_empty() { self.selected_idx = (self.selected_idx + self.items.len() - 1) % self.items.len(); cx.notify(); }
    }

    pub fn selected_item(&self) -> Option<&AutocompleteItem> { self.items.get(self.selected_idx) }
    pub fn is_empty(&self) -> bool { self.items.is_empty() }
}

impl Focusable for AutocompleteMenu { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for AutocompleteMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.items.is_empty() {
            return div().into_any_element();
        }

        let items: Vec<AnyElement> = self.items.iter().cloned().enumerate().map(|(idx, item)| {
            let is_selected = idx == self.selected_idx;
            h_flex()
                .id(ElementId::NamedInteger("autocomplete-item".into(), idx as u64))
                .w_full()
                .px(Spacing::Base08.px())
                .py(Spacing::Base04.px())
                .gap(Spacing::Base08.px())
                .rounded_sm()
                .when(is_selected, |d| d.bg(ThemeColors::bg_selected()))
                .when(!is_selected, |d| d.hover(|d| d.bg(ThemeColors::bg_hover())))
                .on_hover(cx.listener(move |this, hovered, _, cx| { if *hovered { this.selected_idx = idx; cx.notify(); } }))
                .on_click(cx.listener({
                    let item = item.clone();
                    move |_, _, _, cx| cx.emit(AutocompleteMenuEvent::Confirm(item.clone()))
                }))
                .child(Label::new(item.icon()).size(LabelSize::Small))
                .child(Label::new(item.label()).size(LabelSize::Small).color(LabelColor::Primary))
                .children(item.hint().map(|h| Label::new(h).size(LabelSize::XSmall).color(LabelColor::Muted)))
                .child(div().flex_1())
                .children(item.description().map(|d| Label::new(d).size(LabelSize::XSmall).color(LabelColor::Secondary).truncate()))
                .into_any_element()
        }).collect();

        v_flex()
            .id("autocomplete-menu")
            .w(px(300.0))
            .max_h(px(200.0))
            .overflow_y_scroll()
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(AutocompleteMenuEvent::Dismiss)))
            .p(Spacing::Base04.px())
            .bg(ThemeColors::bg_elevated())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_md()
            .shadow_lg()
            .children(items)
            .into_any_element()
    }
}
