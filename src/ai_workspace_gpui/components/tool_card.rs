//! Tool call card component with approval/cancel actions and optional diff rendering.
use gpui::{AnyElement, ElementId, IntoElement, ParentElement, Styled, div, px, prelude::*};
use crate::ai_workspace_gpui::{
    protocol::{
        DiffLineKindSnapshot, ToolCallSnapshot, ToolCallStatusSnapshot, ToolKindSnapshot,
        ToolLogLevelSnapshot, UiToHost,
    },
    ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Button, ButtonStyle, TintColor, Spacing, UiMetrics, ScrollbarState, scrollable_with_scrollbar},
};
use crossbeam_channel::Sender;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
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

// ─────────────────────────────────────────────────────────────────────────────
// ToolCardView (stateful: collapsible, default collapsed)
// ─────────────────────────────────────────────────────────────────────────────

pub struct ToolCardView {
    idx: usize,
    tool: ToolCallSnapshot,
    session_id: uuid::Uuid,
    ui_tx: Sender<UiToHost>,
    collapsed: bool,
}

impl ToolCardView {
    pub fn new(idx: usize, tool: ToolCallSnapshot, session_id: uuid::Uuid, ui_tx: Sender<UiToHost>) -> Self {
        Self { idx, tool, session_id, ui_tx, collapsed: true }
    }

    pub fn set_tool(&mut self, tool: ToolCallSnapshot, cx: &mut gpui::Context<Self>) {
        self.tool = tool;
        cx.notify();
    }

    fn toggle(&mut self, cx: &mut gpui::Context<Self>) { self.collapsed = !self.collapsed; cx.notify(); }
}

impl gpui::Render for ToolCardView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        let (status_icon, status_text, status_color) = ToolCard::status_info(&self.tool.status);
        let kind_icon = ToolCard::kind_icon(self.tool.kind);
        let rid = self.tool.id;
        let sid = self.session_id;

        let chevron = if self.collapsed { "▸" } else { "▾" };
        let header = h_flex()
            .w_full()
            .justify_between()
            .child(
                h_flex()
                    .gap(Spacing::Base04.px())
                    .items_center()
                    .child(Button::new(format!("tool-collapse-{rid}"), chevron).style(ButtonStyle::Ghost).on_click(cx.listener(|this, _, _, cx| this.toggle(cx))).into_any_element())
                    .child(Label::new(kind_icon).size(LabelSize::Small))
                    .child(Label::new(self.tool.tool_name.clone()).size(LabelSize::Small).color(LabelColor::Accent))
            )
            .child(
                h_flex()
                    .gap(Spacing::Base04.px())
                    .items_center()
                    .child(Label::new(status_icon).size(LabelSize::XSmall))
                    .child(Label::new(status_text).size(LabelSize::XSmall).color(status_color))
            );

        // Action buttons based on status (kept visible even when collapsed)
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

        let actions_row = (!actions.is_empty()).then(|| h_flex().w_full().mt(Spacing::Base06.px()).gap(Spacing::Base04.px()).justify_end().children(actions).into_any_element());

        // Collapsed summary (one-liner)
        let summary = {
            let mut bits: Vec<String> = Vec::new();
            if !self.tool.args_preview.trim().is_empty() { bits.push(ToolCard::truncate(&self.tool.args_preview, 120)); }
            if !self.tool.diffs.is_empty() { bits.push(format!("{} file(s) changed", self.tool.diffs.len())); }
            if let Some(r) = self.tool.raw_output.as_ref().or(self.tool.llm_result.as_ref()) {
                let t = r.lines().next().unwrap_or("").trim();
                if !t.is_empty() { bits.push(ToolCard::truncate(t, 140)); }
            }
            let s = if bits.is_empty() { String::new() } else { bits.join(" · ") };
            (!s.is_empty()).then(|| Label::new(s).size(LabelSize::XSmall).color(LabelColor::Muted).into_any_element())
        };

        let sections = ToolCard::build_sections(&self.tool, self.ui_tx.clone(), rid);
        let details = (!self.collapsed).then(|| {
            v_flex()
                .w_full()
                .gap(Spacing::Base04.px())
                .children(sections.args_preview)
                .children(sections.diff_section)
                .children(sections.result_section)
                .children(sections.error_section)
                .children(sections.logs_section)
                .into_any_element()
        });

        v_flex()
            .id(ElementId::Name(format!("tool-card-view-{}", self.tool.id).into()))
            .w_full()
            .my(Spacing::Base04.px())
            .p(Spacing::Base06.px())
            .bg(ThemeColors::bg_secondary())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_sm()
            .gap(Spacing::Base04.px())
            .child(header)
            .children(summary)
            .children(actions_row)
            .children(details.map(|d| {
                let scroller = scrollable_with_scrollbar(
                    format!("tool-card-scroll-{}", rid),
                    &ScrollbarState::default(),
                    div().w_full().child(d),
                );
                div()
                    .w_full()
                    .mt(Spacing::Base04.px())
                    .max_h(px(420.0))
                    .overflow_hidden()
                    .child(scroller)
                    .into_any_element()
            }))
            .into_any_element()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ToolGroupCardView (stateful: collapsible, default collapsed)
// ─────────────────────────────────────────────────────────────────────────────

pub struct ToolGroupCardView {
    group_id: u64,
    session_id: uuid::Uuid,
    ui_tx: Sender<UiToHost>,
    tools: Vec<ToolCallSnapshot>,
    tool_cards: Arc<RwLock<std::collections::HashMap<u64, gpui::Entity<ToolCardView>>>>,
    collapsed: bool,
}

impl ToolGroupCardView {
    pub fn new(
        group_id: u64,
        tools: Vec<ToolCallSnapshot>,
        session_id: uuid::Uuid,
        ui_tx: Sender<UiToHost>,
        tool_cards: Arc<RwLock<std::collections::HashMap<u64, gpui::Entity<ToolCardView>>>>,
    ) -> Self {
        Self { group_id, tools, session_id, ui_tx, tool_cards, collapsed: true }
    }

    pub fn set_tools(&mut self, tools: Vec<ToolCallSnapshot>, cx: &mut gpui::Context<Self>) {
        self.tools = tools;
        cx.notify();
    }

    fn toggle(&mut self, cx: &mut gpui::Context<Self>) { self.collapsed = !self.collapsed; cx.notify(); }

    fn aggregate_status(&self, cards: &[ToolCallSnapshot]) -> ToolCallStatusSnapshot {
        // Priority: Failed/Rejected > InProgress > AwaitingApproval > Pending > Completed > Canceled
        if cards.iter().any(|c| matches!(c.status, ToolCallStatusSnapshot::Failed(_) | ToolCallStatusSnapshot::Rejected(_))) {
            return ToolCallStatusSnapshot::Failed("".into());
        }
        if cards.iter().any(|c| matches!(c.status, ToolCallStatusSnapshot::InProgress)) { return ToolCallStatusSnapshot::InProgress; }
        if cards.iter().any(|c| matches!(c.status, ToolCallStatusSnapshot::AwaitingApproval)) { return ToolCallStatusSnapshot::AwaitingApproval; }
        if cards.iter().any(|c| matches!(c.status, ToolCallStatusSnapshot::Pending)) { return ToolCallStatusSnapshot::Pending; }
        if cards.iter().all(|c| matches!(c.status, ToolCallStatusSnapshot::Canceled)) { return ToolCallStatusSnapshot::Canceled; }
        ToolCallStatusSnapshot::Completed
    }
}

impl gpui::Render for ToolGroupCardView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        let mut children: Vec<AnyElement> = Vec::new();
        if let Ok(map) = self.tool_cards.read() {
            for t in &self.tools {
                if let Some(ent) = map.get(&t.id) {
                    children.push(ent.clone().into_any_element());
                } else {
                    children.push(ToolCard::new(0, t.clone(), self.session_id, self.ui_tx.clone()).into_any_element());
                }
            }
        } else {
            for t in &self.tools {
                children.push(ToolCard::new(0, t.clone(), self.session_id, self.ui_tx.clone()).into_any_element());
            }
        }

        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for t in &self.tools { *counts.entry(t.tool_name.clone()).or_insert(0) += 1; }
        let summary = if counts.is_empty() {
            format!("{} tool call(s)", self.tools.len())
        } else {
            let top = counts.into_iter().map(|(k, v)| format!("{k}×{v}")).take(3).collect::<Vec<_>>().join(" · ");
            if self.tools.len() > 3 { format!("{top} · … ({})", self.tools.len()) } else { top }
        };

        let chevron = if self.collapsed { "▸" } else { "▾" };
        let agg = self.aggregate_status(&self.tools);
        let (status_icon, status_text, status_color) = ToolCard::status_info(&agg);

        let header = h_flex()
            .w_full()
            .justify_between()
            .child(
                h_flex()
                    .gap(Spacing::Base04.px())
                    .items_center()
                    .child(Button::new(format!("tool-group-collapse-{}", self.group_id), chevron).style(ButtonStyle::Ghost).on_click(cx.listener(|this, _, _, cx| this.toggle(cx))).into_any_element())
                    .child(Label::new("T").size(LabelSize::Small))
                    .child(Label::new("Tools").size(LabelSize::Small).color(LabelColor::Accent))
                    .child(Label::new(summary).size(LabelSize::XSmall).color(LabelColor::Muted))
            )
            .child(
                h_flex()
                    .gap(Spacing::Base04.px())
                    .items_center()
                    .child(Label::new(status_icon).size(LabelSize::XSmall))
                    .child(Label::new(status_text).size(LabelSize::XSmall).color(status_color))
            );

        // Expanded: show list of tool cards (each already collapsed by default)
        let details = (!self.collapsed).then(|| {
            let scroller = scrollable_with_scrollbar(
                format!("tool-group-scroll-{}", self.group_id),
                &ScrollbarState::default(),
                v_flex().w_full().gap(Spacing::Base06.px()).children(children),
            );
            div()
                .w_full()
                .mt(Spacing::Base04.px())
                .max_h(px(520.0))
                .overflow_hidden()
                .child(scroller)
                .into_any_element()
        });

        v_flex()
            .id(ElementId::Name(format!("tool-group-{}", self.group_id).into()))
            .w_full()
            .my(Spacing::Base04.px())
            .p(Spacing::Base06.px())
            .bg(ThemeColors::bg_secondary())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_sm()
            .gap(Spacing::Base04.px())
            .child(header)
            .children(details)
            .into_any_element()
    }
}

struct ToolCardSections {
    args_preview: Option<AnyElement>,
    diff_section: Option<AnyElement>,
    result_section: Option<AnyElement>,
    error_section: Option<AnyElement>,
    logs_section: Option<AnyElement>,
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

        let sections = Self::build_sections(&self.tool, self.ui_tx.clone(), rid);

        // Actions row
        let actions_row = if !actions.is_empty() {
            Some(h_flex().w_full().mt(Spacing::Base06.px()).gap(Spacing::Base04.px()).justify_end().children(actions))
        } else { None };

        // Card container
        v_flex()
            .id(ElementId::Name(format!("tool-card-{}", self.tool.id).into()))
            .w_full()
            .my(Spacing::Base04.px())
            .p(Spacing::Base06.px())
            .bg(ThemeColors::bg_secondary())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_sm()
            .gap(Spacing::Base04.px())
            .child(header)
            .children(sections.args_preview)
            .children(sections.diff_section)
            .children(sections.result_section)
            .children(sections.error_section)
            .children(sections.logs_section)
            .children(actions_row)
            .into_any_element()
    }
}

impl ToolCard {
    fn build_sections(tool: &ToolCallSnapshot, ui_tx: Sender<UiToHost>, rid: u64) -> ToolCardSections {
        // Arguments preview
        let args_preview = matches!(tool.status, ToolCallStatusSnapshot::AwaitingApproval).then(|| {
            v_flex()
                .w_full()
                .mt(Spacing::Base02.px())
                .p(Spacing::Base04.px())
                .bg(ThemeColors::bg_primary())
                .border_1()
                .border_color(ThemeColors::border())
                .rounded_sm()
                .child(Label::new("Details (for approval):").size(LabelSize::XSmall).color(LabelColor::Muted))
                .child(Label::new(Self::truncate(&tool.args_preview, 400)).size(LabelSize::XSmall).color(LabelColor::Secondary))
                .into_any_element()
        });

        // Diff section (if present)
        let diff_section = (!tool.diffs.is_empty()).then(|| {
            let files_row = {
                let buttons: Vec<AnyElement> = tool.diffs.iter().enumerate().map(|(i, d)| {
                    let p = PathBuf::from(d.file_path.clone());
                    let label = p.file_name().and_then(|n| n.to_str()).unwrap_or(&d.file_path).to_string();
                    let tx = ui_tx.clone();
                    Button::new(format!("open-diff-{rid}-{i}"), format!("Open {label}"))
                        .style(ButtonStyle::Subtle)
                        .on_click(move |_, _, _| {
                            let _ = tx.send(UiToHost::IdeOpenFile { path: p.clone() });
                            let _ = tx.send(UiToHost::IdeSetActiveFile { path: p.clone() });
                        })
                        .into_any_element()
                }).collect();
                v_flex()
                    .w_full()
                    .mt(Spacing::Base04.px())
                    .gap(Spacing::Base02.px())
                    .child(Label::new("Changed files:").size(LabelSize::XSmall).color(LabelColor::Muted))
                    .child(h_flex().w_full().gap(Spacing::Base04.px()).children(buttons))
                    .into_any_element()
            };

            let diffs: Vec<AnyElement> = tool.diffs.iter().enumerate().map(|(i, d)| {
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
                DiffView::new(i, d.file_path.clone(), hunks).with_jump(ui_tx.clone()).into_any_element()
            }).collect();

            v_flex()
                .w_full()
                .mt(Spacing::Base04.px())
                .child(files_row)
                .children(diffs)
                .into_any_element()
        });

        // Result section (if completed) - prefer raw_output for UI excerpt
        let result_text = tool.raw_output.as_ref().or(tool.llm_result.as_ref());
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
                        .text_size(px(UiMetrics::FONT_SMALL))
                        .text_color(ThemeColors::text_secondary())
                        .child(Self::truncate(result, 200))
                )
                .into_any_element()
        });

        // Error section (if failed)
        let error_section = match &tool.status {
            ToolCallStatusSnapshot::Failed(err) | ToolCallStatusSnapshot::Rejected(err) => Some(
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
                    .into_any_element()
            ),
            _ => None,
        };

        // Logs section (preview)
        let logs_section = if !tool.logs.is_empty() {
            let logs_preview: Vec<AnyElement> = tool.logs.iter().rev().take(3).map(|log| {
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
                    .child(Label::new(format!("Logs ({}):", tool.logs.len())).size(LabelSize::XSmall).color(LabelColor::Muted))
                    .children(logs_preview)
                    .into_any_element()
            )
        } else { None };

        ToolCardSections { args_preview, diff_section, result_section, error_section, logs_section }
    }
}
