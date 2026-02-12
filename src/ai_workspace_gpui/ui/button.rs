//! Button component (Zed-style API, refined dark theme)
use gpui::{AnyElement, App, ClickEvent, ElementId, IntoElement, ParentElement, SharedString, Styled, Window, div, prelude::*, px};
use super::{ThemeColors, Spacing, UiMetrics};

#[derive(Clone, Copy, Default, PartialEq)]
pub enum ButtonStyle { #[default] Ghost, Filled, Tinted(TintColor), Icon, Subtle }

#[derive(Clone, Copy, Default, PartialEq)]
pub enum TintColor { #[default] Accent, Success, Warning, Error }

#[derive(Clone, Copy, Default, PartialEq)]
pub enum ButtonSize { Compact, #[default] Default, Large }

pub struct Button {
    id: ElementId,
    label: SharedString,
    style: ButtonStyle,
    size: ButtonSize,
    disabled: bool,
    selected: bool,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self { id: id.into(), label: label.into(), style: ButtonStyle::default(), size: ButtonSize::default(), disabled: false, selected: false, on_click: None }
    }
    pub fn style(mut self, style: ButtonStyle) -> Self { self.style = style; self }
    pub fn size(mut self, size: ButtonSize) -> Self { self.size = size; self }
    pub fn disabled(mut self, disabled: bool) -> Self { self.disabled = disabled; self }
    pub fn toggle_state(mut self, selected: bool) -> Self { self.selected = selected; self }
    pub fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self { self.on_click = Some(Box::new(handler)); self }
}

impl IntoElement for Button {
    type Element = AnyElement;
    fn into_element(self) -> Self::Element {
        let is_icon = matches!(self.style, ButtonStyle::Icon);
        let (bg, text_color, border, hover_bg) = match self.style {
            ButtonStyle::Ghost => (gpui::transparent_black(), ThemeColors::text_secondary(), false, ThemeColors::bg_hover()),
            ButtonStyle::Filled => (ThemeColors::bg_active(), ThemeColors::text_primary(), true, ThemeColors::bg_selected()),
            ButtonStyle::Icon => (gpui::transparent_black(), ThemeColors::text_muted(), false, ThemeColors::bg_hover()),
            ButtonStyle::Subtle => (gpui::transparent_black(), ThemeColors::text_secondary(), false, ThemeColors::bg_elevated()),
            ButtonStyle::Tinted(tint) => match tint {
                TintColor::Accent => (ThemeColors::bg_selected(), ThemeColors::text_accent(), true, ThemeColors::border_focus()),
                TintColor::Success => (ThemeColors::btn_success(), ThemeColors::text_success(), true, ThemeColors::btn_success()),
                TintColor::Warning => (ThemeColors::bg_elevated(), ThemeColors::text_warning(), true, ThemeColors::bg_hover()),
                TintColor::Error => (ThemeColors::btn_danger(), ThemeColors::text_error(), true, ThemeColors::btn_danger()),
            },
        };
        let (px_h, px_v) = if is_icon {
            (Spacing::Base04.px(), Spacing::Base04.px())
        } else {
            match self.size {
                ButtonSize::Compact => (Spacing::Base04.px(), Spacing::Base02.px()),
                ButtonSize::Default => (Spacing::Base06.px(), Spacing::Base02.px()),
                ButtonSize::Large => (Spacing::Base08.px(), Spacing::Base04.px()),
            }
        };
        let base = div().id(self.id).flex_none().items_center().justify_center().px(px_h).py(px_v).bg(bg).text_color(text_color).rounded_sm().cursor_pointer()
            .text_size(px(UiMetrics::BUTTON_TEXT))
            .when(is_icon, |d| d.text_size(px(UiMetrics::BUTTON_ICON_TEXT)).min_w(px(UiMetrics::BUTTON_ICON_MIN)).min_h(px(UiMetrics::BUTTON_ICON_MIN)))
            .when(border, |d| d.border_1().border_color(ThemeColors::border()))
            .when(!self.disabled, |d| d.hover(|s| s.bg(hover_bg).text_color(ThemeColors::text_primary())).active(|s| s.opacity(0.85)))
            .when(self.disabled, |d| d.opacity(0.5).cursor_default())
            .when(self.selected, |d| d.bg(ThemeColors::bg_selected()).text_color(ThemeColors::text_accent()));
        let base = if let Some(handler) = self.on_click { base.on_click(handler) } else { base };
        base.child(self.label.to_string()).into_any_element()
    }
}
