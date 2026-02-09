//! Scrollbar component (custom scrollbar with drag support and auto-hide).
use gpui::{AnyElement, App, Bounds, Context, ElementId, Entity, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, Styled, Window, div, prelude::*, px};
use super::ThemeColors;

// ─────────────────────────────────────────────────────────────────────────────
// ScrollbarState
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct ScrollbarState {
    pub scroll_offset: f32,
    pub content_height: f32,
    pub viewport_height: f32,
    pub dragging: bool,
    pub drag_start_y: f32,
    pub drag_start_offset: f32,
    pub hovered: bool,
    pub visible: ScrollbarVisibility,
}

#[derive(Clone, Copy, Default, PartialEq)]
pub enum ScrollbarVisibility { #[default] Auto, Always, Never }

impl ScrollbarState {
    pub fn new() -> Self { Self::default() }

    pub fn update(&mut self, scroll_offset: f32, content_height: f32, viewport_height: f32) {
        self.scroll_offset = scroll_offset;
        self.content_height = content_height;
        self.viewport_height = viewport_height;
    }

    pub fn thumb_height(&self) -> f32 {
        if self.content_height <= 0.0 { return 0.0; }
        let ratio = (self.viewport_height / self.content_height).min(1.0);
        (ratio * self.viewport_height).max(20.0)
    }

    pub fn thumb_offset(&self) -> f32 {
        if self.content_height <= self.viewport_height { return 0.0; }
        let scrollable = self.content_height - self.viewport_height;
        let ratio = (self.scroll_offset / scrollable).clamp(0.0, 1.0);
        ratio * (self.viewport_height - self.thumb_height())
    }

    pub fn is_needed(&self) -> bool { self.content_height > self.viewport_height }

    pub fn should_show(&self) -> bool {
        match self.visible {
            ScrollbarVisibility::Always => self.is_needed(),
            ScrollbarVisibility::Auto => self.is_needed() && (self.hovered || self.dragging),
            ScrollbarVisibility::Never => false,
        }
    }

    pub fn offset_for_thumb_position(&self, thumb_y: f32, track_height: f32) -> f32 {
        if track_height <= self.thumb_height() { return 0.0; }
        let ratio = thumb_y / (track_height - self.thumb_height());
        let scrollable = self.content_height - self.viewport_height;
        (ratio * scrollable).clamp(0.0, scrollable)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scrollbar Element
// ─────────────────────────────────────────────────────────────────────────────

pub struct Scrollbar {
    id: ElementId,
    state: ScrollbarState,
    on_scroll: Option<Box<dyn Fn(f32, &mut Window, &mut App) + 'static>>,
    width: Pixels,
}

impl Scrollbar {
    pub fn new(id: impl Into<ElementId>, state: ScrollbarState) -> Self {
        Self { id: id.into(), state, on_scroll: None, width: px(8.0) }
    }

    pub fn on_scroll(mut self, f: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self { self.on_scroll = Some(Box::new(f)); self }
    pub fn width(mut self, width: Pixels) -> Self { self.width = width; self }
}

impl IntoElement for Scrollbar {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        if !self.state.should_show() {
            return div().w(self.width).h_full().flex_none().into_any_element();
        }

        let thumb_h = px(self.state.thumb_height());
        let thumb_y = px(self.state.thumb_offset());

        div()
            .id(self.id)
            .flex_none()
            .w(self.width)
            .h_full()
            .bg(ThemeColors::bg_secondary())
            .rounded_sm()
            .child(
                div()
                    .absolute()
                    .top(thumb_y)
                    .left_0()
                    .w_full()
                    .h(thumb_h)
                    .bg(ThemeColors::bg_active())
                    .rounded_sm()
                    .hover(|d| d.bg(ThemeColors::bg_hover()))
            )
            .into_any_element()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scrollable container helper
// ─────────────────────────────────────────────────────────────────────────────

pub fn scrollable_with_scrollbar<E: IntoElement>(id: impl Into<ElementId>, state: &ScrollbarState, content: E) -> impl IntoElement {
    h_flex()
        .size_full()
        .overflow_hidden()
        .child(div().id("scrollable-with-scrollbar-content").flex_1().overflow_y_scroll().child(content))
        .child(Scrollbar::new(id, state.clone()))
}

fn h_flex() -> gpui::Div { div().flex().flex_row() }
