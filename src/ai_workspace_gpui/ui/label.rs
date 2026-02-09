//! Label component (Zed-style API)

use gpui::{IntoElement, ParentElement, SharedString, Styled, div, px, Div, prelude::*};
use super::ThemeColors;

#[derive(Clone, Copy, Default, PartialEq)]
pub enum LabelSize { XSmall, Small, #[default] Default, Large }

#[derive(Clone, Copy, Default, PartialEq)]
pub enum LabelColor { #[default] Primary, Secondary, Muted, Accent, Success, Warning, Error }

pub struct Label {
    text: SharedString,
    size: LabelSize,
    color: LabelColor,
    truncate: bool,
}

impl Label {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into(), size: LabelSize::default(), color: LabelColor::default(), truncate: false }
    }
    pub fn size(mut self, size: LabelSize) -> Self { self.size = size; self }
    pub fn color(mut self, color: LabelColor) -> Self { self.color = color; self }
    pub fn truncate(mut self) -> Self { self.truncate = true; self }
}

impl IntoElement for Label {
    type Element = <Div as IntoElement>::Element;
    fn into_element(self) -> Self::Element {
        let text_color = match self.color {
            LabelColor::Primary => ThemeColors::text_primary(),
            LabelColor::Secondary => ThemeColors::text_secondary(),
            LabelColor::Muted => ThemeColors::text_muted(),
            LabelColor::Accent => ThemeColors::text_accent(),
            LabelColor::Success => ThemeColors::text_success(),
            LabelColor::Warning => ThemeColors::text_warning(),
            LabelColor::Error => ThemeColors::text_error(),
        };
        let font_size = match self.size {
            LabelSize::XSmall => px(10.0), LabelSize::Small => px(12.0), LabelSize::Default => px(14.0), LabelSize::Large => px(16.0),
        };
        div().text_color(text_color).text_size(font_size).when(self.truncate, |d| d.truncate()).child(self.text.to_string()).into_element()
    }
}
