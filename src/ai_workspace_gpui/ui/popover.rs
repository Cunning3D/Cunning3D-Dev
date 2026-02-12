//! Popover component (dropdown/popup container with anchored positioning).
use gpui::{AnyElement, App, Context, DismissEvent, ElementId, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Pixels, Point, Render, SharedString, Styled, Window, anchored, deferred, div, prelude::*, px};
use super::{v_flex, ThemeColors, Spacing};

// ─────────────────────────────────────────────────────────────────────────────
// Popover
// ─────────────────────────────────────────────────────────────────────────────

pub struct Popover {
    focus_handle: FocusHandle,
    content: Box<dyn Fn(&mut Window, &mut App) -> AnyElement + 'static>,
}

impl EventEmitter<DismissEvent> for Popover {}

impl Popover {
    pub fn new<F>(cx: &mut Context<Self>, content: F) -> Self
    where F: Fn(&mut Window, &mut App) -> AnyElement + 'static
    {
        Self { focus_handle: cx.focus_handle(), content: Box::new(content) }
    }
}

impl Focusable for Popover { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for Popover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = (self.content)(window, cx);
        v_flex()
            .id("popover")
            .track_focus(&self.focus_handle)
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(DismissEvent)))
            .p(Spacing::Base02.px())
            .bg(ThemeColors::bg_elevated())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_sm()
            .shadow_lg()
            .child(content)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PopoverMenu (button + popover dropdown)
// ─────────────────────────────────────────────────────────────────────────────

pub struct PopoverMenuState {
    pub is_open: bool,
    pub popover: Option<Entity<Popover>>,
}

impl Default for PopoverMenuState { fn default() -> Self { Self { is_open: false, popover: None } } }

impl PopoverMenuState {
    pub fn toggle<F>(&mut self, window: &mut Window, cx: &mut App, content: F)
    where F: Fn(&mut Window, &mut App) -> AnyElement + 'static
    {
        if self.is_open {
            self.close();
        } else {
            let popover = cx.new(|cx| Popover::new(cx, content));
            popover.focus_handle(cx).focus(window, cx);
            self.popover = Some(popover);
            self.is_open = true;
        }
    }

    pub fn close(&mut self) {
        self.popover = None;
        self.is_open = false;
    }

    pub fn render(&self, anchor: Point<Pixels>) -> Option<impl IntoElement> {
        self.popover.as_ref().map(|p| deferred(anchored().position(anchor).child(p.clone())).with_priority(1))
    }
}
