//! Message feedback component (thumbs up/down).
use gpui::{AnyElement, ElementId, IntoElement, ParentElement, Styled, Div, prelude::*};
use crate::ai_workspace_gpui::ui::{h_flex, Button, ButtonStyle, Spacing, TintColor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feedback { Positive, Negative }

pub struct MessageFeedback {
    id: usize,
    feedback: Option<Feedback>,
    on_feedback: Option<Box<dyn Fn(Feedback, &mut gpui::Window, &mut gpui::App) + 'static>>,
}

impl MessageFeedback {
    pub fn new(id: usize) -> Self { Self { id, feedback: None, on_feedback: None } }
    pub fn feedback(mut self, f: Option<Feedback>) -> Self { self.feedback = f; self }
    pub fn on_feedback(mut self, f: impl Fn(Feedback, &mut gpui::Window, &mut gpui::App) + 'static) -> Self { self.on_feedback = Some(Box::new(f)); self }
}

impl IntoElement for MessageFeedback {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let id = self.id;
        let current = self.feedback;
        let _on_feedback = self.on_feedback;

        h_flex()
            .id(ElementId::NamedInteger("msg-feedback".into(), id as u64))
            .gap(Spacing::Base02.px())
            .child({
                let is_selected = current == Some(Feedback::Positive);
                Button::new(format!("fb-up-{id}"), if is_selected { "[+]" } else { "+" })
                    .style(if is_selected { ButtonStyle::Tinted(TintColor::Accent) } else { ButtonStyle::Ghost })
            })
            .child({
                let is_selected = current == Some(Feedback::Negative);
                Button::new(format!("fb-dn-{id}"), if is_selected { "[-]" } else { "-" })
                    .style(if is_selected { ButtonStyle::Tinted(TintColor::Error) } else { ButtonStyle::Ghost })
            })
            .into_any_element()
    }
}
