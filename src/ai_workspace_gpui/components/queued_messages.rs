//! Queued messages component for pending message management.
use gpui::{ElementId, Entity, IntoElement, InteractiveElement, ParentElement, Render, Styled, Context, Window, App, FocusHandle, Focusable, div, px, prelude::*};
use crate::ai_workspace_gpui::ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Button, ButtonStyle, Spacing};

// ─────────────────────────────────────────────────────────────────────────────
// QueuedMessage
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct QueuedMessage {
    pub id: usize,
    pub content: String,
    pub timestamp: std::time::Instant,
}

// ─────────────────────────────────────────────────────────────────────────────
// QueuedMessagesView
// ─────────────────────────────────────────────────────────────────────────────

pub struct QueuedMessagesView {
    messages: Vec<QueuedMessage>,
    focus_handle: FocusHandle,
    on_send: Option<Box<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    on_edit: Option<Box<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    on_remove: Option<Box<dyn Fn(usize, &mut Window, &mut App) + 'static>>,
    on_clear: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
}

impl QueuedMessagesView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self { messages: Vec::new(), focus_handle: cx.focus_handle(), on_send: None, on_edit: None, on_remove: None, on_clear: None }
    }

    pub fn set_messages(&mut self, messages: Vec<QueuedMessage>, cx: &mut Context<Self>) {
        self.messages = messages;
        cx.notify();
    }

    pub fn on_send(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self { self.on_send = Some(Box::new(f)); self }
    pub fn on_edit(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self { self.on_edit = Some(Box::new(f)); self }
    pub fn on_remove(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self { self.on_remove = Some(Box::new(f)); self }
    pub fn on_clear(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self { self.on_clear = Some(Box::new(f)); self }

    pub fn is_empty(&self) -> bool { self.messages.is_empty() }
    pub fn count(&self) -> usize { self.messages.len() }
}

impl Focusable for QueuedMessagesView { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for QueuedMessagesView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if self.messages.is_empty() { return div().into_any_element(); }

        v_flex()
            .w_full()
            .p(Spacing::Base06.px())
            .gap(Spacing::Base04.px())
            .bg(ThemeColors::bg_elevated())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_md()
            .child(
                h_flex().w_full().justify_between()
                    .child(Label::new(format!("{} message(s) queued", self.messages.len())).size(LabelSize::Small).color(LabelColor::Muted))
                    .child(Button::new("clear-queue", "Clear All").style(ButtonStyle::Ghost))
            )
            .children(
                self.messages.iter().enumerate().map(|(i, msg)| {
                    let preview = if msg.content.len() > 50 { format!("{}...", &msg.content[..50]) } else { msg.content.clone() };
                    h_flex()
                        .id(ElementId::NamedInteger("queued-msg".into(), i as u64))
                        .w_full()
                        .px(Spacing::Base04.px())
                        .py(Spacing::Base02.px())
                        .gap(Spacing::Base04.px())
                        .bg(ThemeColors::bg_secondary())
                        .rounded_sm()
                        .child(Label::new(format!("{}.", i + 1)).size(LabelSize::XSmall).color(LabelColor::Muted))
                        .child(Label::new(preview).size(LabelSize::Small).color(LabelColor::Primary))
                        .child(div().flex_1())
                        .child(Button::new(format!("send-{i}"), "Send").style(ButtonStyle::Ghost))
                        .child(Button::new(format!("edit-{i}"), "Edit").style(ButtonStyle::Ghost))
                        .child(Button::new(format!("rm-{i}"), "X").style(ButtonStyle::Ghost))
                })
            )
            .into_any_element()
    }
}
