//! Resizable panel helpers (Cursor-style).
use gpui::{CursorStyle, Div, ElementId, IntoElement, Styled, div, prelude::*, px};
use super::ThemeColors;

/// Resize direction for drag handles
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ResizeDirection { Horizontal, Vertical }

/// Build a drag handle element (bind events outside).
pub fn drag_handle(id: impl Into<ElementId>, direction: ResizeDirection) -> gpui::Stateful<Div> {
    let (cursor, build) = match direction {
        ResizeDirection::Horizontal => (CursorStyle::ResizeLeftRight, div().flex_none().w(px(4.0)).h_full()),
        ResizeDirection::Vertical => (CursorStyle::ResizeUpDown, div().flex_none().h(px(4.0)).w_full()),
    };
    build.id(id).cursor(cursor).bg(ThemeColors::border()).hover(|s| s.bg(ThemeColors::border_focus())).active(|s| s.bg(ThemeColors::text_accent()))
}

/// Resizable panel state stored in window
#[derive(Clone)]
pub struct ResizablePanelState {
    pub sidebar_width: f32,
    pub chat_width_ratio: f32,
    pub is_dragging_sidebar: bool,
    pub is_dragging_chat: bool,
}

impl Default for ResizablePanelState {
    fn default() -> Self {
        Self { sidebar_width: 240.0, chat_width_ratio: 0.35, is_dragging_sidebar: false, is_dragging_chat: false }
    }
}

impl ResizablePanelState {
    pub fn clamp_sidebar(&mut self, min: f32, max: f32) { self.sidebar_width = self.sidebar_width.clamp(min, max); }
    pub fn clamp_chat_ratio(&mut self, min: f32, max: f32) { self.chat_width_ratio = self.chat_width_ratio.clamp(min, max); }
}
