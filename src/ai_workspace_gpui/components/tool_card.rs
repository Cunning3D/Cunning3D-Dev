//! Tool call card component with approval/cancel actions and optional diff rendering.
use gpui::{AnyElement, ElementId, IntoElement, ParentElement, Styled, div, px, prelude::*};
use crate::ai_workspace_gpui::{
    protocol::{
        DiffLineKindSnapshot, ToolCallSnapshot, ToolCallStatusSnapshot, ToolKindSnapshot,
        ToolLogLevelSnapshot, UiToHost,
    },
    ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Button, ButtonStyle, TintColor, Spacing},
};
use crossbeam_channel::Sender;
use super::{DiffHunk, DiffLine, DiffLineKind, DiffView};

pub struct ToolCard {
    idx: usize,
    tool: ToolCallSnapshot,
    session_id: uuid::Uuid,
    ui_tx: Sender<UiToHost>,
}

impl ToolCard {
    pub fn new(idx: usize, tool: ToolCallSnapshot, session_id: uuid::Uuid, ui_tx: Sender<UiToHost>) -> Self {
        Self { idx, tool, session_id, ui_tx }
    }

    fn status_info(status: &ToolCallStatusSnapshot) -> (&'static str, &'static str, LabelColor) {
        match status {
            ToolCallStatusSnapshot::Pending => ("...", "Pending", LabelColor::Muted),
            ToolCallStatusSnapshot::AwaitingApproval => ("!", "Awaiting Approval", LabelColor::Warning),
            ToolCallStatusSnapshot::InProgress => ("RUN", "Running", LabelColor::Accent),
            ToolCallStatusSnapshot::Completed => ("OK", "Completed", LabelColor::Success),
            ToolCallStatusSnapshot::Failed(_) => ("FAIL", "Failed", LabelColor::Error),
            ToolCallStatusSnapshot::Rejected(_) => ("NO", "Rejected", LabelColor::Error),
            ToolCallStatusSnapshot::Canceled => ("STOP", "Canceled", LabelColor::Muted),
        }
    }

    fn kind_icon(kind: ToolKindSnapshot) -> &'static str {
        match kind {
            ToolKindSnapshot::Read => "R",
            ToolKindSnapshot::Search => "S",
            ToolKindSnapshot::Execute => "X",
            ToolKindSnapshot::Edit => "E",
            ToolKindSnapshot::Other => "T",
        }
    }

    fn log_color(level: &ToolLogLevelSnapshot) -> LabelColor {
        match level {
            ToolLogLevelSnapshot::Info => LabelColor::Secondary,
            ToolLogLevelSnapshot::Warn => LabelColor::Warning,
            ToolLogLevelSnapshot::Error => LabelColor::Error,
            ToolLogLevelSnapshot::Progress => LabelColor::Accent,
        }
    }

    fn truncate(s: &str, max: usize) -> String {
        if s.chars().count() <= max { s.to_string() } else { s.chars().take(max).collect::<String>() + "…" }
    }
}

impl IntoElement for ToolCard {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let (status_icon, status_text, status_color) = Self::status_info(&self.tool.status);
        let kind_icon = Self::kind_icon(self.tool.kind);
        let rid = self.tool.id;
        let sid = self.session_id;

        // Action buttons based on status
        let actions: Vec<AnyElement> = match &self.tool.status {
            ToolCallStatusSnapshot::AwaitingApproval => {
                let tx1 = self.ui_tx.clone();
                let tx2 = self.ui_tx.clone();
                let tx3 = self.ui_tx.clone();
                vec![
                    Button::new(format!("approve-{rid}"), "✓ Approve")
                        .style(ButtonStyle::Tinted(TintColor::Success))
                        .on_click(move |_, _, _| { let _ = tx1.send(UiToHost::ApproveTool { session_id: sid, request_id: rid, remember: false }); })
                        .into_any_element(),
                    Button::new(format!("approve-always-{rid}"), "✓ Always")
                        .style(ButtonStyle::Ghost)
                        .on_click(move |_, _, _| { let _ = tx2.send(UiToHost::ApproveTool { session_id: sid, request_id: rid, remember: true }); })
                        .into_any_element(),
                    Button::new(format!("deny-{rid}"), "✗ Deny")
                        .style(ButtonStyle::Tinted(TintColor::Error))
                        .on_click(move |_, _, _| { let _ = tx3.send(UiToHost::DenyTool { session_id: sid, request_id: rid }); })
                        .into_any_element(),
                ]
            }
            ToolCallStatusSnapshot::InProgress => {
                let tx = self.ui_tx.clone();
                vec![
                    Button::new(format!("cancel-{rid}"), "Cancel")
                        .style(ButtonStyle::Tinted(TintColor::Warning))
                        .on_click(move |_, _, _| { let _ = tx.send(UiToHost::CancelTool { session_id: sid, request_id: rid }); })
                        .into_any_element()
                ]
            }
            _ => vec![],
        };

        // Header with tool name and status
        let header = h_flex()
            .w_full()
            .justify_between()
            .child(
                h_flex().gap(Spacing::Base04.px())
                    .child(Label::new(kind_icon).size(LabelSize::Small))
                    .child(Label::new(self.tool.tool_name.clone()).size(LabelSize::Small).color(LabelColor::Accent))
            )
            .child(
                h_flex().gap(Spacing::Base04.px())
                    .child(Label::new(status_icon).size(LabelSize::XSmall))
                    .child(Label::new(status_text).size(LabelSize::XSmall).color(status_color))
            );

        // Arguments preview
        let args_preview = h_flex()
            .w_full()
            .child(Label::new(Self::truncate(&self.tool.args_preview, 150)).size(LabelSize::XSmall).color(LabelColor::Muted));

        // Diff section (if present)
        let diff_section = (!self.tool.diffs.is_empty()).then(|| {
            let diffs: Vec<AnyElement> = self.tool.diffs.iter().enumerate().map(|(i, d)| {
                let hunks: Vec<DiffHunk> = d.hunks.iter().map(|h| DiffHunk {
                    old_start: h.old_start,
                    old_count: h.old_count,
                    new_start: h.new_start,
                    new_count: h.new_count,
                    lines: h.lines.iter().map(|l| DiffLine {
                        kind: match l.kind {
                            DiffLineKindSnapshot::Context => DiffLineKind::Context,
                            DiffLineKindSnapshot::Added => DiffLineKind::Added,
                            DiffLineKindSnapshot::Removed => DiffLineKind::Removed,
                        },
                        line_num_old: l.line_num_old,
                        line_num_new: l.line_num_new,
                        content: l.content.clone(),
                    }).collect(),
                }).collect();
                DiffView::new(i, d.file_path.clone(), hunks).into_any_element()
            }).collect();
            v_flex().w_full().mt(Spacing::Base04.px()).children(diffs).into_any_element()
        });

        // Result section (if completed) - prefer raw_output for UI excerpt
        let result_text = self.tool.raw_output.as_ref().or(self.tool.llm_result.as_ref());
        let result_section = result_text.map(|result| {
            v_flex()
                .w_full()
                .mt(Spacing::Base04.px())
                .p(Spacing::Base04.px())
                .bg(ThemeColors::bg_primary())
                .border_1()
                .border_color(ThemeColors::border())
                .rounded_sm()
                .child(Label::new("Result:").size(LabelSize::XSmall).color(LabelColor::Muted))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ThemeColors::text_secondary())
                        .child(Self::truncate(result, 200))
                )
        });

        // Error section (if failed)
        let error_section = match &self.tool.status {
            ToolCallStatusSnapshot::Failed(err) | ToolCallStatusSnapshot::Rejected(err) => {
                Some(
                    v_flex()
                        .w_full()
                        .mt(Spacing::Base04.px())
                        .p(Spacing::Base04.px())
                        .bg(ThemeColors::btn_danger())
                        .border_1()
                        .border_color(ThemeColors::text_error())
                        .rounded_sm()
                        .child(Label::new("Error:").size(LabelSize::XSmall).color(LabelColor::Error))
                        .child(Label::new(Self::truncate(err, 150)).size(LabelSize::XSmall).color(LabelColor::Error))
                )
            }
            _ => None,
        };

        // Logs section (collapsible)
        let logs_section = if !self.tool.logs.is_empty() {
            let logs_preview: Vec<AnyElement> = self.tool.logs.iter().rev().take(3).map(|log| {
                h_flex()
                    .gap(Spacing::Base02.px())
                    .child(Label::new("-").size(LabelSize::XSmall).color(Self::log_color(&log.level)))
                    .child(Label::new(Self::truncate(&log.message, 80)).size(LabelSize::XSmall).color(Self::log_color(&log.level)))
                    .into_any_element()
            }).collect();
            Some(
                v_flex()
                    .w_full()
                    .mt(Spacing::Base04.px())
                    .gap(Spacing::Base02.px())
                    .child(Label::new(format!("Logs ({}):", self.tool.logs.len())).size(LabelSize::XSmall).color(LabelColor::Muted))
                    .children(logs_preview)
            )
        } else { None };

        // Actions row
        let actions_row = if !actions.is_empty() {
            Some(h_flex().w_full().mt(Spacing::Base06.px()).gap(Spacing::Base04.px()).justify_end().children(actions))
        } else { None };

        // Card container
        v_flex()
            .id(ElementId::Name(format!("tool-card-{}", self.tool.id).into()))
            .w_full()
            .my(Spacing::Base04.px())
            .p(Spacing::Base08.px())
            .bg(ThemeColors::bg_secondary())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_md()
            .gap(Spacing::Base04.px())
            .child(header)
            .child(args_preview)
            .children(diff_section)
            .children(result_section)
            .children(error_section)
            .children(logs_section)
            .children(actions_row)
            .into_any_element()
    }
}
