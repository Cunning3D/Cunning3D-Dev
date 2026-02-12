//! Model selector component (Picker-based model selection with favorites).
use gpui::{AnyElement, App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, prelude::*, px};
use crossbeam_channel::Sender;
use crate::ai_workspace_gpui::{ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Button, ButtonStyle, Spacing, Picker, PickerDelegate}, protocol::{ProviderSnapshot, UiToHost, VoiceModel}};

// ─────────────────────────────────────────────────────────────────────────────
// ModelInfo
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum SelectableItem {
    Voice { model: VoiceModel, is_selected: bool },
    Llm {
        provider_idx: usize,
        provider_name: String,
        model_id: String,
        is_selected: bool,
        is_favorite: bool,
    },
}

impl SelectableItem {
    fn title(&self) -> String {
        match self {
            SelectableItem::Voice { model, .. } => match model {
                VoiceModel::Off => "Voice: Off".to_string(),
                VoiceModel::Legacy => "Voice: Legacy".to_string(),
                VoiceModel::GeminiLive => "Voice: Gemini Live".to_string(),
            },
            SelectableItem::Llm { model_id, .. } => model_id.clone(),
        }
    }

    fn subtitle(&self) -> String {
        match self {
            SelectableItem::Voice { .. } => "Voice Model".to_string(),
            SelectableItem::Llm { provider_name, .. } => provider_name.clone(),
        }
    }

    fn search_text(&self) -> String {
        match self {
            SelectableItem::Voice { model, .. } => match model {
                VoiceModel::Off => "voice off legacy gemini live".to_string(),
                VoiceModel::Legacy => "voice legacy whisper tts".to_string(),
                VoiceModel::GeminiLive => "voice gemini live audio".to_string(),
            },
            SelectableItem::Llm { provider_name, model_id, .. } => {
                format!("{} {}", provider_name, model_id)
            }
        }
    }

    fn is_selected(&self) -> bool {
        match self {
            SelectableItem::Voice { is_selected, .. } => *is_selected,
            SelectableItem::Llm { is_selected, .. } => *is_selected,
        }
    }

    fn separators_after(&self, next: &SelectableItem) -> bool {
        match (self, next) {
            (SelectableItem::Voice { .. }, SelectableItem::Llm { .. }) => true,
            (SelectableItem::Llm { provider_idx: a, .. }, SelectableItem::Llm { provider_idx: b, .. }) => a != b,
            _ => false,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ModelSelectorDelegate
// ─────────────────────────────────────────────────────────────────────────────

pub struct ModelSelectorDelegate {
    items: Vec<SelectableItem>,
    filtered: Vec<usize>,
    selected_index: usize,
    ui_tx: Sender<UiToHost>,
}

impl ModelSelectorDelegate {
    pub fn new(providers: &[ProviderSnapshot], active_idx: usize, voice_model: VoiceModel, ui_tx: Sender<UiToHost>) -> Self {
        let mut items = Vec::new();

        items.push(SelectableItem::Voice { model: VoiceModel::Off, is_selected: matches!(voice_model, VoiceModel::Off) });
        items.push(SelectableItem::Voice { model: VoiceModel::Legacy, is_selected: matches!(voice_model, VoiceModel::Legacy) });
        items.push(SelectableItem::Voice { model: VoiceModel::GeminiLive, is_selected: matches!(voice_model, VoiceModel::GeminiLive) });

        for (idx, provider) in providers.iter().enumerate() {
            for model in &provider.models {
                items.push(SelectableItem::Llm {
                    provider_idx: idx,
                    provider_name: provider.name.clone(),
                    model_id: model.clone(),
                    is_selected: idx == active_idx && model == &provider.selected_model,
                    is_favorite: false,
                });
            }
        }
        let filtered = (0..items.len()).collect();
        let selected_index = items.iter().position(|m| m.is_selected()).unwrap_or(0);
        Self { items, filtered, selected_index, ui_tx }
    }
}

impl PickerDelegate for ModelSelectorDelegate {
    type ListItem = AnyElement;

    fn match_count(&self) -> usize { self.filtered.len() }
    fn selected_index(&self) -> usize { self.selected_index }
    fn set_selected_index(&mut self, ix: usize, cx: &mut Context<Picker<Self>>) { self.selected_index = ix; cx.notify(); }
    fn placeholder_text(&self, _: &App) -> SharedString { "Search models...".into() }

    fn update_matches(&mut self, query: &str, cx: &mut Context<Picker<Self>>) {
        let query = query.to_lowercase();
        self.filtered = self.items.iter().enumerate()
            .filter(|(_, m)| query.is_empty() || m.search_text().to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
        if self.selected_index >= self.filtered.len() { self.selected_index = 0; }
        cx.notify();
    }

    fn confirm(&mut self, _secondary: bool, _window: &mut Window, _cx: &mut Context<Picker<Self>>) {
        if let Some(&idx) = self.filtered.get(self.selected_index) {
            match &self.items[idx] {
                SelectableItem::Voice { model, .. } => {
                    let _ = self.ui_tx.send(UiToHost::SetVoiceModel { model: *model });
                }
                SelectableItem::Llm { provider_idx, model_id, .. } => {
                    let _ = self.ui_tx.send(UiToHost::SelectProvider { profile_idx: *provider_idx });
                    let _ = self.ui_tx.send(UiToHost::SelectModel { profile_idx: *provider_idx, model: model_id.clone() });
                }
            }
        }
    }

    fn dismissed(&mut self, _window: &mut Window, _cx: &mut Context<Picker<Self>>) {}

    fn render_match(&self, ix: usize, selected: bool, _cx: &mut Context<Picker<Self>>) -> Self::ListItem {
        let Some(&model_idx) = self.filtered.get(ix) else { return div().into_any_element(); };
        let item = &self.items[model_idx];

        h_flex()
            .w_full()
            .px(Spacing::Base06.px())
            .py(Spacing::Base02.px())
            .gap(Spacing::Base06.px())
            .rounded_sm()
            .when(selected, |d| d.bg(ThemeColors::bg_selected()))
            .when(!selected, |d| d.hover(|d| d.bg(ThemeColors::bg_hover())))
            .child(
                v_flex().flex_1().overflow_hidden()
                    .child(Label::new(item.title()).size(LabelSize::Small).color(if item.is_selected() { LabelColor::Accent } else { LabelColor::Primary }))
                    .child(Label::new(item.subtitle()).size(LabelSize::XSmall).color(LabelColor::Muted))
            )
            .when(item.is_selected(), |d| d.child(Label::new("*").size(LabelSize::Small).color(LabelColor::Accent)))
            .into_any_element()
    }

    fn separators_after(&self, ix: usize) -> bool {
        let Some(&curr_idx) = self.filtered.get(ix) else { return false; };
        let Some(&next_idx) = self.filtered.get(ix + 1) else { return false; };
        self.items[curr_idx].separators_after(&self.items[next_idx])
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ModelSelector (wrapper)
// ─────────────────────────────────────────────────────────────────────────────

pub struct ModelSelector {
    picker: Entity<Picker<ModelSelectorDelegate>>,
    focus_handle: FocusHandle,
}

impl EventEmitter<DismissEvent> for ModelSelector {}

impl ModelSelector {
    pub fn new(providers: &[ProviderSnapshot], active_idx: usize, voice_model: VoiceModel, ui_tx: Sender<UiToHost>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let delegate = ModelSelectorDelegate::new(providers, active_idx, voice_model, ui_tx);
        let picker = cx.new(|cx| Picker::new(delegate, window, cx).width(300.0).max_height(350.0));
        cx.subscribe(&picker, |_, _, _: &DismissEvent, cx| cx.emit(DismissEvent)).detach();
        let focus_handle = picker.focus_handle(cx);
        Self { picker, focus_handle }
    }
}

impl Focusable for ModelSelector { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for ModelSelector {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.picker.clone()
    }
}
