//! Model selector component (Picker-based model selection with favorites).
use gpui::{AnyElement, App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, prelude::*, px};
use crossbeam_channel::Sender;
use crate::ai_workspace_gpui::{ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Button, ButtonStyle, Spacing, Picker, PickerDelegate}, protocol::{ProviderSnapshot, UiToHost}};

// ─────────────────────────────────────────────────────────────────────────────
// ModelInfo
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub provider_idx: usize,
    pub provider_name: String,
    pub model_id: String,
    pub is_selected: bool,
    pub is_favorite: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// ModelSelectorDelegate
// ─────────────────────────────────────────────────────────────────────────────

pub struct ModelSelectorDelegate {
    models: Vec<ModelInfo>,
    filtered: Vec<usize>,
    selected_index: usize,
    ui_tx: Sender<UiToHost>,
}

impl ModelSelectorDelegate {
    pub fn new(providers: &[ProviderSnapshot], active_idx: usize, ui_tx: Sender<UiToHost>) -> Self {
        let mut models = Vec::new();
        for (idx, provider) in providers.iter().enumerate() {
            for model in &provider.models {
                models.push(ModelInfo {
                    provider_idx: idx,
                    provider_name: provider.name.clone(),
                    model_id: model.clone(),
                    is_selected: idx == active_idx && model == &provider.selected_model,
                    is_favorite: false,
                });
            }
        }
        let filtered = (0..models.len()).collect();
        let selected_index = models.iter().position(|m| m.is_selected).unwrap_or(0);
        Self { models, filtered, selected_index, ui_tx }
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
        self.filtered = self.models.iter().enumerate()
            .filter(|(_, m)| query.is_empty() || m.model_id.to_lowercase().contains(&query) || m.provider_name.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
        if self.selected_index >= self.filtered.len() { self.selected_index = 0; }
        cx.notify();
    }

    fn confirm(&mut self, _secondary: bool, _window: &mut Window, cx: &mut Context<Picker<Self>>) {
        if let Some(&idx) = self.filtered.get(self.selected_index) {
            let model = &self.models[idx];
            // First switch provider (handles Gemini at idx 0 vs OpenAI at idx 1+)
            let _ = self.ui_tx.send(UiToHost::SelectProvider { profile_idx: model.provider_idx });
            // Then select the specific model within that provider
            let _ = self.ui_tx.send(UiToHost::SelectModel { profile_idx: model.provider_idx, model: model.model_id.clone() });
        }
    }

    fn dismissed(&mut self, _window: &mut Window, _cx: &mut Context<Picker<Self>>) {}

    fn render_match(&self, ix: usize, selected: bool, _cx: &mut Context<Picker<Self>>) -> Self::ListItem {
        let Some(&model_idx) = self.filtered.get(ix) else { return div().into_any_element(); };
        let model = &self.models[model_idx];

        h_flex()
            .w_full()
            .px(Spacing::Base06.px())
            .py(Spacing::Base04.px())
            .gap(Spacing::Base08.px())
            .rounded_sm()
            .when(selected, |d| d.bg(ThemeColors::bg_selected()))
            .when(!selected, |d| d.hover(|d| d.bg(ThemeColors::bg_hover())))
            .child(
                v_flex().flex_1().overflow_hidden()
                    .child(Label::new(&model.model_id).size(LabelSize::Small).color(if model.is_selected { LabelColor::Accent } else { LabelColor::Primary }))
                    .child(Label::new(&model.provider_name).size(LabelSize::XSmall).color(LabelColor::Muted))
            )
            .when(model.is_selected, |d| d.child(Label::new("*").size(LabelSize::Small).color(LabelColor::Accent)))
            .into_any_element()
    }

    fn separators_after(&self, ix: usize) -> bool {
        let Some(&curr_idx) = self.filtered.get(ix) else { return false; };
        let Some(&next_idx) = self.filtered.get(ix + 1) else { return false; };
        self.models[curr_idx].provider_idx != self.models[next_idx].provider_idx
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
    pub fn new(providers: &[ProviderSnapshot], active_idx: usize, ui_tx: Sender<UiToHost>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let delegate = ModelSelectorDelegate::new(providers, active_idx, ui_tx);
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
