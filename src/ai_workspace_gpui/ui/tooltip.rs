//! Tooltip component (hover hints with optional shortcut display).
use gpui::{AnyElement, App, Bounds, ElementId, IntoElement, ParentElement, Pixels, Point, SharedString, Styled, Window, anchored, deferred, div, prelude::*, px};
use super::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Spacing};

// ─────────────────────────────────────────────────────────────────────────────
// Tooltip Element
// ─────────────────────────────────────────────────────────────────────────────

pub struct Tooltip {
    text: SharedString,
    shortcut: Option<SharedString>,
    meta: Option<SharedString>,
}

impl Tooltip {
    pub fn new(text: impl Into<SharedString>) -> Self { Self { text: text.into(), shortcut: None, meta: None } }
    pub fn with_shortcut(mut self, shortcut: impl Into<SharedString>) -> Self { self.shortcut = Some(shortcut.into()); self }
    pub fn with_meta(mut self, meta: impl Into<SharedString>) -> Self { self.meta = Some(meta.into()); self }
}

impl IntoElement for Tooltip {
    type Element = <gpui::Div as IntoElement>::Element;
    fn into_element(self) -> Self::Element {
        let content = h_flex().gap(Spacing::Base08.px())
            .child(Label::new(self.text).size(LabelSize::Small).color(LabelColor::Primary))
            .children(self.shortcut.map(|s| {
                div().px(Spacing::Base04.px()).py(px(1.0)).bg(ThemeColors::bg_active()).rounded_sm()
                    .child(Label::new(s).size(LabelSize::XSmall).color(LabelColor::Muted))
            }));

        v_flex()
            .px(Spacing::Base08.px())
            .py(Spacing::Base04.px())
            .bg(ThemeColors::bg_elevated())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_sm()
            .shadow_md()
            .child(content)
            .children(self.meta.map(|m| Label::new(m).size(LabelSize::XSmall).color(LabelColor::Muted)))
            .into_element()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TooltipExt trait for adding tooltips to elements
// ─────────────────────────────────────────────────────────────────────────────

pub fn tooltip(text: impl Into<SharedString>) -> Tooltip { Tooltip::new(text) }
pub fn tooltip_with_shortcut(text: impl Into<SharedString>, shortcut: impl Into<SharedString>) -> Tooltip { Tooltip::new(text).with_shortcut(shortcut) }
