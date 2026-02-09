//! Config options view for session configuration (model, mode, etc.).
use gpui::{AnyElement, App, Context, DismissEvent, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, prelude::*, px};
use crate::ai_workspace_gpui::ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Button, ButtonStyle, Spacing, Picker, PickerDelegate};

// ─────────────────────────────────────────────────────────────────────────────
// ConfigOption
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: ConfigCategory,
    pub values: Vec<ConfigValue>,
    pub current_value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigCategory { Model, Mode, Profile, Custom(String) }

#[derive(Clone, Debug)]
pub struct ConfigValue {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub group: Option<String>,
    pub is_favorite: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// ConfigOptionSelectorDelegate
// ─────────────────────────────────────────────────────────────────────────────

pub struct ConfigOptionSelectorDelegate {
    option: ConfigOption,
    filtered: Vec<usize>,
    selected_index: usize,
    on_select: Option<Box<dyn Fn(&str, &str, &mut Window, &mut App) + 'static>>,
}

impl ConfigOptionSelectorDelegate {
    pub fn new(option: ConfigOption) -> Self {
        let current = option.current_value.clone();
        let filtered = (0..option.values.len()).collect();
        let selected_index = option.values.iter().position(|v| v.id == current).unwrap_or(0);
        Self { option, filtered, selected_index, on_select: None }
    }

    pub fn on_select(mut self, f: impl Fn(&str, &str, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Box::new(f));
        self
    }
}

impl PickerDelegate for ConfigOptionSelectorDelegate {
    type ListItem = AnyElement;

    fn match_count(&self) -> usize { self.filtered.len() }
    fn selected_index(&self) -> usize { self.selected_index }
    fn set_selected_index(&mut self, ix: usize, cx: &mut Context<Picker<Self>>) { self.selected_index = ix; cx.notify(); }
    fn placeholder_text(&self, _: &App) -> SharedString { format!("Search {}...", self.option.name).into() }

    fn update_matches(&mut self, query: &str, cx: &mut Context<Picker<Self>>) {
        let q = query.to_lowercase();
        self.filtered = self.option.values.iter().enumerate()
            .filter(|(_, v)| q.is_empty() || v.name.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        // Favorites first
        self.filtered.sort_by_key(|&i| (!self.option.values[i].is_favorite, i));
        if self.selected_index >= self.filtered.len() { self.selected_index = 0; }
        cx.notify();
    }

    fn confirm(&mut self, _secondary: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        if let Some(&idx) = self.filtered.get(self.selected_index) {
            let value = &self.option.values[idx];
            if let Some(ref cb) = self.on_select { cb(&self.option.id, &value.id, window, cx); }
        }
    }

    fn dismissed(&mut self, _window: &mut Window, _cx: &mut Context<Picker<Self>>) {}

    fn render_match(&self, ix: usize, selected: bool, _cx: &mut Context<Picker<Self>>) -> Self::ListItem {
        let Some(&value_idx) = self.filtered.get(ix) else { return div().into_any_element(); };
        let value = &self.option.values[value_idx];
        let is_current = value.id == self.option.current_value;
        let is_fav = value.is_favorite;

        // Group separator
        let show_group = ix == 0 || {
            let prev_idx = self.filtered.get(ix - 1).copied().unwrap_or(0);
            self.option.values.get(prev_idx).and_then(|v| v.group.as_ref()) != value.group.as_ref()
        };

        v_flex()
            .w_full()
            .when(show_group && value.group.is_some(), |d| {
                d.child(
                    div().w_full().px(Spacing::Base06.px()).py(Spacing::Base02.px())
                        .child(Label::new(value.group.clone().unwrap_or_default()).size(LabelSize::XSmall).color(LabelColor::Muted))
                )
            })
            .child(
                h_flex()
                    .w_full()
                    .px(Spacing::Base06.px())
                    .py(Spacing::Base04.px())
                    .gap(Spacing::Base04.px())
                    .rounded_sm()
                    .when(selected, |d| d.bg(ThemeColors::bg_selected()))
                    .when(!selected, |d| d.hover(|d| d.bg(ThemeColors::bg_hover())))
                    .when(is_fav, |d| d.child(Label::new("*").size(LabelSize::Small).color(LabelColor::Accent)))
                    .child(Label::new(&value.name).size(LabelSize::Small).color(if is_current { LabelColor::Accent } else { LabelColor::Primary }))
                    .child(div().flex_1())
                    .when(is_current, |d| d.child(Label::new("v").size(LabelSize::XSmall).color(LabelColor::Accent)))
            )
            .into_any_element()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ConfigOptionsView
// ─────────────────────────────────────────────────────────────────────────────

pub struct ConfigOptionsView {
    options: Vec<ConfigOption>,
    selectors: Vec<Entity<ConfigOptionSelector>>,
}

impl ConfigOptionsView {
    pub fn new(options: Vec<ConfigOption>, on_select: impl Fn(&str, &str, &mut Window, &mut App) + Clone + 'static, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let selectors = options.iter().map(|opt| {
            let opt = opt.clone();
            let cb = on_select.clone();
            cx.new(|cx| ConfigOptionSelector::new(opt, cb, window, cx))
        }).collect();
        Self { options, selectors }
    }

    pub fn set_options(&mut self, options: Vec<ConfigOption>, on_select: impl Fn(&str, &str, &mut Window, &mut App) + Clone + 'static, window: &mut Window, cx: &mut Context<Self>) {
        self.options = options.clone();
        self.selectors = options.iter().map(|opt| {
            let opt = opt.clone();
            let cb = on_select.clone();
            cx.new(|cx| ConfigOptionSelector::new(opt, cb, window, cx))
        }).collect();
        cx.notify();
    }
}

impl Render for ConfigOptionsView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        h_flex().gap(Spacing::Base04.px()).children(self.selectors.iter().cloned())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ConfigOptionSelector
// ─────────────────────────────────────────────────────────────────────────────

pub struct ConfigOptionSelector {
    option: ConfigOption,
    picker: Entity<Picker<ConfigOptionSelectorDelegate>>,
    picker_open: bool,
    focus_handle: FocusHandle,
}

impl ConfigOptionSelector {
    pub fn new(option: ConfigOption, on_select: impl Fn(&str, &str, &mut Window, &mut App) + 'static, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let delegate = ConfigOptionSelectorDelegate::new(option.clone()).on_select(on_select);
        let picker = cx.new(|cx| Picker::new(delegate, window, cx).width(220.0).max_height(300.0));
        let focus_handle = cx.focus_handle();
        cx.subscribe(&picker, |this, _, _: &DismissEvent, cx| { this.picker_open = false; cx.notify(); }).detach();
        Self { option, picker, picker_open: false, focus_handle }
    }

    fn toggle(&mut self, cx: &mut Context<Self>) { self.picker_open = !self.picker_open; cx.notify(); }
    fn current_value_name(&self) -> &str { self.option.values.iter().find(|v| v.id == self.option.current_value).map(|v| v.name.as_str()).unwrap_or("Unknown") }
}

impl Focusable for ConfigOptionSelector { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for ConfigOptionSelector {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let picker_el = if self.picker_open { Some(gpui::deferred(gpui::anchored().snap_to_window().child(self.picker.clone())).with_priority(2)) } else { None };

        div()
            .relative()
            .child(
                Button::new(format!("cfg-{}", self.option.id), self.current_value_name().to_string())
                    .style(ButtonStyle::Ghost)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle(cx)))
            )
            .children(picker_el)
    }
}
