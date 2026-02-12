//! Terminal output display component for tool execution results.
use gpui::{AnyElement, ElementId, IntoElement, ParentElement, Styled, div, px, prelude::*};
use crate::ai_workspace_gpui::ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Spacing, UiMetrics};

// ─────────────────────────────────────────────────────────────────────────────
// TerminalLine
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum TerminalLineKind { Stdout, Stderr, System }

#[derive(Clone, Debug)]
pub struct TerminalLine {
    pub kind: TerminalLineKind,
    pub content: String,
}

impl TerminalLine {
    pub fn stdout(content: impl Into<String>) -> Self { Self { kind: TerminalLineKind::Stdout, content: content.into() } }
    pub fn stderr(content: impl Into<String>) -> Self { Self { kind: TerminalLineKind::Stderr, content: content.into() } }
    pub fn system(content: impl Into<String>) -> Self { Self { kind: TerminalLineKind::System, content: content.into() } }
}

// ─────────────────────────────────────────────────────────────────────────────
// TerminalOutput
// ─────────────────────────────────────────────────────────────────────────────

pub struct TerminalOutput {
    id: usize,
    title: Option<String>,
    lines: Vec<TerminalLine>,
    exit_code: Option<i32>,
    collapsed: bool,
}

impl TerminalOutput {
    pub fn new(id: usize, lines: Vec<TerminalLine>) -> Self {
        Self { id, title: None, lines, exit_code: None, collapsed: false }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self { self.title = Some(title.into()); self }
    pub fn exit_code(mut self, code: i32) -> Self { self.exit_code = Some(code); self }
    pub fn collapsed(mut self, collapsed: bool) -> Self { self.collapsed = collapsed; self }
}

impl IntoElement for TerminalOutput {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let exit_status = self.exit_code.map(|c| {
            let (text, color) = if c == 0 { ("exit 0".to_string(), LabelColor::Success) } else { (format!("exit {}", c), LabelColor::Error) };
            Label::new(text).size(LabelSize::XSmall).color(color)
        });

        let line_elements: Vec<_> = self.lines.iter().map(|line| {
            let text_color = match line.kind {
                TerminalLineKind::Stdout => ThemeColors::text_primary(),
                TerminalLineKind::Stderr => ThemeColors::text_error(),
                TerminalLineKind::System => ThemeColors::text_muted(),
            };
            div()
                .w_full()
                .text_size(px(UiMetrics::FONT_DEFAULT))
                .text_color(text_color)
                .child(line.content.clone())
        }).collect();

        v_flex()
            .id(ElementId::NamedInteger("terminal-output".into(), self.id as u64))
            .w_full()
            .my(Spacing::Base04.px())
            .bg(ThemeColors::bg_elevated())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_sm()
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .px(Spacing::Base06.px())
                    .py(Spacing::Base02.px())
                    .bg(ThemeColors::bg_secondary())
                    .border_b_1()
                    .border_color(ThemeColors::border())
                    .justify_between()
                    .child(
                        h_flex().gap(Spacing::Base04.px())
                            .child(Label::new("Terminal").size(LabelSize::XSmall).color(LabelColor::Muted))
                            .children(self.title.map(|t| Label::new(t).size(LabelSize::Small).color(LabelColor::Primary)))
                    )
                    .children(exit_status)
            )
            .when(!self.collapsed, |d| {
                d.child(
                    div()
                        .id(ElementId::NamedInteger("terminal-scroll".into(), self.id as u64))
                        .w_full()
                        .max_h(px(200.0))
                        .overflow_y_scroll()
                        .p(Spacing::Base04.px())
                        .children(line_elements)
                )
            })
            .into_any_element()
    }
}
