//! Diff view component for displaying code changes.
use gpui::{AnyElement, ElementId, IntoElement, MouseButton, MouseDownEvent, ParentElement, Styled, div, px, prelude::*};
use crossbeam_channel::Sender;
use std::path::PathBuf;
use crate::ai_workspace_gpui::protocol::UiToHost;
use crate::ai_workspace_gpui::ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Spacing, UiMetrics};

// ─────────────────────────────────────────────────────────────────────────────
// DiffLine
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum DiffLineKind { Context, Added, Removed }

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub line_num_old: Option<usize>,
    pub line_num_new: Option<usize>,
    pub content: String,
}

impl DiffLine {
    pub fn context(old: usize, new: usize, content: impl Into<String>) -> Self {
        Self { kind: DiffLineKind::Context, line_num_old: Some(old), line_num_new: Some(new), content: content.into() }
    }
    pub fn added(new: usize, content: impl Into<String>) -> Self {
        Self { kind: DiffLineKind::Added, line_num_old: None, line_num_new: Some(new), content: content.into() }
    }
    pub fn removed(old: usize, content: impl Into<String>) -> Self {
        Self { kind: DiffLineKind::Removed, line_num_old: Some(old), line_num_new: None, content: content.into() }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DiffHunk
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

// ─────────────────────────────────────────────────────────────────────────────
// DiffView
// ─────────────────────────────────────────────────────────────────────────────

pub struct DiffView {
    id: usize,
    file_path: String,
    jump_path: PathBuf,
    hunks: Vec<DiffHunk>,
    additions: usize,
    deletions: usize,
    jump_tx: Option<Sender<UiToHost>>,
}

impl DiffView {
    pub fn new(id: usize, file_path: impl Into<String>, hunks: Vec<DiffHunk>) -> Self {
        let file_path = file_path.into();
        let additions = hunks.iter().flat_map(|h| &h.lines).filter(|l| matches!(l.kind, DiffLineKind::Added)).count();
        let deletions = hunks.iter().flat_map(|h| &h.lines).filter(|l| matches!(l.kind, DiffLineKind::Removed)).count();
        Self { id, jump_path: PathBuf::from(file_path.clone()), file_path, hunks, additions, deletions, jump_tx: None }
    }

    pub fn with_jump(mut self, tx: Sender<UiToHost>) -> Self {
        self.jump_tx = Some(tx);
        self
    }
}

impl IntoElement for DiffView {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let jump_tx = self.jump_tx.clone();
        let jump_path = self.jump_path.clone();
        let line_elements: Vec<_> = self.hunks.iter().enumerate().flat_map(|(hunk_ix, hunk)| {
            let mut out: Vec<AnyElement> = Vec::with_capacity(hunk.lines.len() + 1);
            out.push(
                h_flex()
                    .id(ElementId::NamedInteger("diff-hunk".into(), hunk_ix as u64))
                    .w_full()
                    .bg(ThemeColors::bg_secondary())
                    .border_b_1()
                    .border_color(ThemeColors::border())
                    .px(Spacing::Base06.px())
                    .py(Spacing::Base02.px())
                    .child(
                        Label::new(format!(
                            "@@ -{},{} +{},{} @@",
                            hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
                        ))
                        .size(LabelSize::XSmall)
                        .color(LabelColor::Muted),
                    )
                    .into_any_element(),
            );
            out.extend(hunk.lines.iter().map(|line| {
                let (bg, prefix, text_color) = match line.kind {
                    DiffLineKind::Context => (ThemeColors::bg_primary(), " ", ThemeColors::text_secondary()),
                    DiffLineKind::Added => (ThemeColors::diff_added_bg(), "+", ThemeColors::diff_added_text()),
                    DiffLineKind::Removed => (ThemeColors::diff_removed_bg(), "-", ThemeColors::diff_removed_text()),
                };
                let old_num = line.line_num_old.map(|n| format!("{:4}", n)).unwrap_or_else(|| "    ".into());
                let new_num = line.line_num_new.map(|n| format!("{:4}", n)).unwrap_or_else(|| "    ".into());
                let target_line = line
                    .line_num_new
                    .or(line.line_num_old)
                    .unwrap_or(1)
                    .saturating_sub(1) as u32;

                h_flex()
                    .w_full()
                    .bg(bg)
                    .text_size(px(UiMetrics::FONT_DEFAULT))
                    .child(div().w(px(40.0)).flex_none().text_color(ThemeColors::text_muted()).child(old_num))
                    .child(div().w(px(40.0)).flex_none().text_color(ThemeColors::text_muted()).child(new_num))
                    .child(div().w(px(16.0)).flex_none().text_color(text_color).child(prefix))
                    .child(div().flex_1().text_color(text_color).child(line.content.clone()))
                    .when(jump_tx.is_some(), |d| {
                        let tx = jump_tx.clone().unwrap();
                        let p = jump_path.clone();
                        d.cursor_pointer()
                            .hover(|s| s.bg(ThemeColors::bg_hover()))
                            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, _| { let _ = tx.send(UiToHost::IdeGotoLine { path: p.clone(), line: target_line, col: 0 }); })
                    })
                    .into_any_element()
            }));
            out
        }).collect();

        v_flex()
            .id(ElementId::NamedInteger("diff-view".into(), self.id as u64))
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
                    .child(Label::new(&self.file_path).size(LabelSize::Small).color(LabelColor::Primary))
                    .child(
                        h_flex().gap(Spacing::Base08.px())
                            .child(Label::new(format!("+{}", self.additions)).size(LabelSize::XSmall).color(LabelColor::Success))
                            .child(Label::new(format!("-{}", self.deletions)).size(LabelSize::XSmall).color(LabelColor::Error))
                    )
            )
            .child(
                div()
                    .id(ElementId::NamedInteger("diff-scroll".into(), self.id as u64))
                    .w_full()
                    .max_h(px(300.0))
                    .overflow_y_scroll()
                    .children(line_elements)
            )
            .into_any_element()
    }
}
