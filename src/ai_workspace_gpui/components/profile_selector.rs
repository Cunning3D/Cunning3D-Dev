//! Profile selector component (Agent profile configuration switching).
use gpui::{AnyElement, App, Context, DismissEvent, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, prelude::*, px};
use crossbeam_channel::Sender;
use crate::ai_workspace_gpui::{ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Button, ButtonStyle, Spacing, Picker, PickerDelegate}, protocol::UiToHost};

// ─────────────────────────────────────────────────────────────────────────────
// AgentProfile
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub is_builtin: bool,
}

impl AgentProfile {
    pub fn builtin_profiles() -> Vec<Self> {
        vec![
            Self { id: "default".into(), name: "Default".into(), description: Some("Standard agent profile".into()), is_builtin: true },
            Self { id: "coding".into(), name: "Coding".into(), description: Some("Optimized for code generation".into()), is_builtin: true },
            Self { id: "chat".into(), name: "Chat".into(), description: Some("Conversational assistant".into()), is_builtin: true },
        ]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ProfileSelectorDelegate
// ─────────────────────────────────────────────────────────────────────────────

pub struct ProfileSelectorDelegate {
    profiles: Vec<AgentProfile>,
    filtered: Vec<usize>,
    selected_index: usize,
    active_profile_id: String,
    on_select: Option<Box<dyn Fn(&str, &mut Window, &mut App) + 'static>>,
}

impl ProfileSelectorDelegate {
    pub fn new(profiles: Vec<AgentProfile>, active_id: &str) -> Self {
        let filtered = (0..profiles.len()).collect();
        let selected_index = profiles.iter().position(|p| p.id == active_id).unwrap_or(0);
        Self { profiles, filtered, selected_index, active_profile_id: active_id.to_string(), on_select: None }
    }

    pub fn on_select(mut self, f: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Box::new(f));
        self
    }
}

impl PickerDelegate for ProfileSelectorDelegate {
    type ListItem = AnyElement;

    fn match_count(&self) -> usize { self.filtered.len() }
    fn selected_index(&self) -> usize { self.selected_index }
    fn set_selected_index(&mut self, ix: usize, cx: &mut Context<Picker<Self>>) { self.selected_index = ix; cx.notify(); }
    fn placeholder_text(&self, _: &App) -> SharedString { "Search profiles...".into() }

    fn update_matches(&mut self, query: &str, cx: &mut Context<Picker<Self>>) {
        let query = query.to_lowercase();
        self.filtered = self.profiles.iter().enumerate()
            .filter(|(_, p)| query.is_empty() || p.name.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
        if self.selected_index >= self.filtered.len() { self.selected_index = 0; }
        cx.notify();
    }

    fn confirm(&mut self, _secondary: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        if let Some(&idx) = self.filtered.get(self.selected_index) {
            let profile = &self.profiles[idx];
            if let Some(ref cb) = self.on_select { cb(&profile.id, window, cx); }
        }
    }

    fn dismissed(&mut self, _window: &mut Window, _cx: &mut Context<Picker<Self>>) {}

    fn render_match(&self, ix: usize, selected: bool, _cx: &mut Context<Picker<Self>>) -> Self::ListItem {
        let Some(&profile_idx) = self.filtered.get(ix) else { return div().into_any_element(); };
        let profile = &self.profiles[profile_idx];
        let is_active = profile.id == self.active_profile_id;

        v_flex()
            .w_full()
            .px(Spacing::Base06.px())
            .py(Spacing::Base02.px())
            .gap(Spacing::Base02.px())
            .rounded_sm()
            .when(selected, |d| d.bg(ThemeColors::bg_selected()))
            .when(!selected, |d| d.hover(|d| d.bg(ThemeColors::bg_hover())))
            .child(
                h_flex().w_full().justify_between()
                    .child(Label::new(&profile.name).size(LabelSize::Small).color(if is_active { LabelColor::Accent } else { LabelColor::Primary }))
                    .when(is_active, |d| d.child(Label::new("*").size(LabelSize::Small).color(LabelColor::Accent)))
            )
            .children(profile.description.as_ref().map(|d| Label::new(d.clone()).size(LabelSize::XSmall).color(LabelColor::Muted)))
            .into_any_element()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ProfileSelector
// ─────────────────────────────────────────────────────────────────────────────

pub struct ProfileSelector {
    picker: Entity<Picker<ProfileSelectorDelegate>>,
    focus_handle: FocusHandle,
}

impl ProfileSelector {
    pub fn new(profiles: Vec<AgentProfile>, active_id: &str, on_select: impl Fn(&str, &mut Window, &mut App) + 'static, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let delegate = ProfileSelectorDelegate::new(profiles, active_id).on_select(on_select);
        let picker = cx.new(|cx| Picker::new(delegate, window, cx).width(250.0).max_height(300.0));
        let focus_handle = picker.focus_handle(cx);
        Self { picker, focus_handle }
    }
}

impl Focusable for ProfileSelector { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for ProfileSelector {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement { self.picker.clone() }
}
