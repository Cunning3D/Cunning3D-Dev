//! Mode selector component (Agent mode switching: Agent/Edit/Ask).
use gpui::{App, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled, Window, prelude::*};
use crate::ai_workspace_gpui::ui::{h_flex, ThemeColors, Label, LabelColor, LabelSize, Spacing};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AgentMode { #[default] Agent, Edit, Ask }

impl AgentMode {
    pub fn name(&self) -> &'static str { match self { Self::Agent => "Agent", Self::Edit => "Edit", Self::Ask => "Ask" } }
    pub fn next(&self) -> Self { match self { Self::Agent => Self::Edit, Self::Edit => Self::Ask, Self::Ask => Self::Agent } }
}

pub struct ModeSelector {
    current_mode: AgentMode,
    focus_handle: FocusHandle,
    on_change: Option<Box<dyn Fn(AgentMode, &mut Window, &mut App) + 'static>>,
}

impl ModeSelector {
    pub fn new(cx: &mut Context<Self>) -> Self { Self { current_mode: AgentMode::default(), focus_handle: cx.focus_handle(), on_change: None } }
    pub fn mode(&self) -> AgentMode { self.current_mode }
    pub fn set_mode(&mut self, mode: AgentMode, window: &mut Window, cx: &mut Context<Self>) { self.current_mode = mode; if let Some(ref cb) = self.on_change { cb(mode, window, cx); } cx.notify(); }
    pub fn on_change(mut self, f: impl Fn(AgentMode, &mut Window, &mut App) + 'static) -> Self { self.on_change = Some(Box::new(f)); self }
    pub fn cycle(&mut self, window: &mut Window, cx: &mut Context<Self>) { self.set_mode(self.current_mode.next(), window, cx); }
}

impl Focusable for ModeSelector { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for ModeSelector {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = self.current_mode;
        h_flex()
            .id("mode-selector")
            .items_center()
            .px(Spacing::Base04.px())
            .py(Spacing::Base02.px())
            .gap(Spacing::Base04.px())
            .rounded_sm()
            .cursor_pointer()
            .hover(|s| s.bg(ThemeColors::bg_hover()))
            .on_click(cx.listener(|this, _, window, cx| this.cycle(window, cx)))
            .child(Label::new(mode.name()).size(LabelSize::Small).color(LabelColor::Secondary))
            .child(Label::new("▾").size(LabelSize::XSmall).color(LabelColor::Muted))
    }
}
