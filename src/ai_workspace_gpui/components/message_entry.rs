//! Message entry component (User/Assistant bubbles with Markdown and context menu).
use gpui::{ClipboardItem, ElementId, IntoElement, InteractiveElement, MouseButton, ParentElement, StatefulInteractiveElement, Styled, div, px, Div, prelude::*};
use crate::ai_workspace_gpui::{ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Button, ButtonStyle, Spacing, Markdown}, protocol::{MessageStateSnapshot, ThinkingSnapshot}};

/// Message role for adaptive spacing
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MessageRole { User, Assistant, Tool }

pub struct MessageEntry {
    idx: usize,
    role: MessageRole,
    content: String,
    state: Option<MessageStateSnapshot>,
    thinking: Option<String>,
    thinking_collapsed: bool,
    prev_role: Option<MessageRole>,
    timestamp: Option<i64>,
}

/// Format relative time from unix timestamp
fn format_relative_time(ts: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let diff = now - ts;
    if diff < 60 { return "just now".to_string(); }
    if diff < 3600 { return format!("{}m ago", diff / 60); }
    if diff < 86400 { return format!("{}h ago", diff / 3600); }
    format!("{}d ago", diff / 86400)
}

impl MessageEntry {
    pub fn user(idx: usize, content: &str) -> Self {
        Self { idx, role: MessageRole::User, content: content.to_string(), state: None, thinking: None, thinking_collapsed: true, prev_role: None, timestamp: None }
    }

    pub fn assistant(idx: usize, content: &str, state: MessageStateSnapshot, thinking: Option<&ThinkingSnapshot>) -> Self {
        Self { idx, role: MessageRole::Assistant, content: content.to_string(), state: Some(state), thinking: thinking.map(|t| t.content.clone()), thinking_collapsed: thinking.map(|t| t.collapsed).unwrap_or(true), prev_role: None, timestamp: None }
    }

    pub fn with_prev_role(mut self, prev: Option<MessageRole>) -> Self { self.prev_role = prev; self }
    pub fn with_timestamp(mut self, ts: Option<i64>) -> Self { self.timestamp = ts; self }
}

impl IntoElement for MessageEntry {
    type Element = <Div as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let (bg, border_color, avatar, name_color) = match self.role {
            MessageRole::User => (ThemeColors::bg_selected(), ThemeColors::border_focus(), "U", LabelColor::Accent),
            MessageRole::Assistant => (ThemeColors::bg_secondary(), ThemeColors::border(), "AI", LabelColor::Secondary),
            MessageRole::Tool => (ThemeColors::bg_elevated(), ThemeColors::border(), "T", LabelColor::Muted),
        };

        let status_indicator = match self.state {
            Some(MessageStateSnapshot::Pending) => Some(("...", LabelColor::Muted)),
            Some(MessageStateSnapshot::Streaming) => Some(("LIVE", LabelColor::Accent)),
            Some(MessageStateSnapshot::Done) => None,
            Some(MessageStateSnapshot::Error) => Some(("ERR", LabelColor::Error)),
            None => None,
        };

        let content_for_copy = self.content.clone();
        let idx = self.idx;
        let is_assistant = matches!(self.role, MessageRole::Assistant);

        // Thinking section (collapsible)
        let thinking_section = self.thinking.clone().map(|thinking_text| {
            let preview = if thinking_text.len() > 200 { format!("{}...", &thinking_text[..200]) } else { thinking_text };
            v_flex()
                .w_full()
                .p(Spacing::Base04.px())
                .mb(Spacing::Base04.px())
                .bg(ThemeColors::bg_elevated())
                .border_1()
                .border_color(ThemeColors::border())
                .rounded_sm()
                .child(h_flex().gap(Spacing::Base04.px()).child(Label::new("Thinking").size(LabelSize::XSmall).color(LabelColor::Muted)))
                .child(div().text_size(px(11.0)).text_color(ThemeColors::text_muted()).child(preview))
        });

        // Content section (Markdown for assistant, plain text for user)
        let content_section = if self.content.is_empty() && self.state == Some(MessageStateSnapshot::Pending) {
            div().child(Label::new("...").color(LabelColor::Muted))
        } else if self.content.is_empty() && self.state == Some(MessageStateSnapshot::Streaming) {
            div().child(Label::new("...").color(LabelColor::Accent))
        } else if is_assistant {
            div().text_size(px(14.0)).child(Markdown::new(&self.content))
        } else {
            div().text_size(px(14.0)).text_color(ThemeColors::text_primary()).child(self.content.clone())
        };

        // Copy button (shown on hover for assistant messages)
        let copy_button = if is_assistant && !self.content.is_empty() {
            Some(
                Button::new(format!("copy-{idx}"), "Copy")
                    .style(ButtonStyle::Ghost)
                    .on_click(move |_: &gpui::ClickEvent, _: &mut gpui::Window, cx: &mut gpui::App| cx.write_to_clipboard(ClipboardItem::new_string(content_for_copy.clone())))
            )
        } else { None };

        // Message bubble with hover actions (use relative max-width for responsive layout)
        let bubble = v_flex()
            .id(ElementId::NamedInteger("msg-bubble".into(), idx as u64))
            .group("message")
            .relative()
            .max_w(gpui::relative(0.85))
            .min_w(px(80.0))
            .p(Spacing::Base08.px())
            .bg(bg)
            .border_1()
            .border_color(border_color)
            .rounded_md()
            .gap(Spacing::Base04.px())
            .overflow_hidden()
            .children(thinking_section)
            .child(content_section)
            // Hover actions
            .child(
                h_flex()
                    .invisible()
                    .group_hover("message", |d| d.visible())
                    .absolute()
                    .top(px(-8.0))
                    .right(px(4.0))
                    .gap(Spacing::Base02.px())
                    .children(copy_button)
            );

        // Timestamp label
        let time_label = self.timestamp.map(|ts| Label::new(format_relative_time(ts)).size(LabelSize::XSmall).color(LabelColor::Muted));

        // Header with avatar, name, timestamp, and status
        let header = h_flex()
            .w_full()
            .justify_between()
            .mb(Spacing::Base02.px())
            .child(
                h_flex().gap(Spacing::Base04.px())
                    .child(Label::new(avatar).size(LabelSize::Small))
                    .child(Label::new(if matches!(self.role, MessageRole::User) { "You" } else { "Assistant" }).size(LabelSize::Small).color(name_color))
                    .children(time_label)
            )
            .children(status_indicator.map(|(icon, color)| Label::new(icon).size(LabelSize::XSmall).color(color)));

        // Adaptive spacing: 4px same role, 16px different role
        let top_spacing = match (self.prev_role, self.role) {
            (Some(prev), curr) if prev == curr => Spacing::Base04,
            (None, _) => Spacing::Base04,
            _ => Spacing::Base16,
        };

        // Outer container with alignment and adaptive spacing
        v_flex()
            .w_full()
            .pt(top_spacing.px())
            .pb(Spacing::Base04.px())
            .child(header)
            .child(
                h_flex()
                    .w_full()
                    .when(matches!(self.role, MessageRole::User), |d| d.justify_end())
                    .when(matches!(self.role, MessageRole::Assistant), |d| d.justify_start())
                    .child(bubble)
            )
            .into_element()
    }
}
