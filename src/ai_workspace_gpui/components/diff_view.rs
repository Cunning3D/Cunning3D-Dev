//! Diff view component for displaying code changes.
use gpui::{AnyElement, ElementId, IntoElement, ParentElement, Styled, div, px, prelude::*};
use crate::ai_workspace_gpui::ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Spacing};

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
    hunks: Vec<DiffHunk>,
    additions: usize,
    deletions: usize,
}

impl DiffView {
    pub fn new(id: usize, file_path: impl Into<String>, hunks: Vec<DiffHunk>) -> Self {
        let additions = hunks.iter().flat_map(|h| &h.lines).filter(|l| matches!(l.kind, DiffLineKind::Added)).count();
        let deletions = hunks.iter().flat_map(|h| &h.lines).filter(|l| matches!(l.kind, DiffLineKind::Removed)).count();
        Self { id, file_path: file_path.into(), hunks, additions, deletions }
    }
}

impl IntoElement for DiffView {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let line_elements: Vec<_> = self.hunks.iter().flat_map(|hunk| {
            hunk.lines.iter().enumerate().map(|(idx, line)| {
                let (bg, prefix, text_color) = match line.kind {
                    DiffLineKind::Context => (ThemeColors::bg_primary(), " ", ThemeColors::text_secondary()),
                    DiffLineKind::Added => (ThemeColors::diff_added_bg(), "+", ThemeColors::diff_added_text()),
                    DiffLineKind::Removed => (ThemeColors::diff_removed_bg(), "-", ThemeColors::diff_removed_text()),
                };
                let old_num = line.line_num_old.map(|n| format!("{:4}", n)).unwrap_or_else(|| "    ".into());
                let new_num = line.line_num_new.map(|n| format!("{:4}", n)).unwrap_or_else(|| "    ".into());

                h_flex()
                    .w_full()
                    .bg(bg)
                    .text_size(px(12.0))
                    .child(div().w(px(40.0)).flex_none().text_color(ThemeColors::text_muted()).child(old_num))
                    .child(div().w(px(40.0)).flex_none().text_color(ThemeColors::text_muted()).child(new_num))
                    .child(div().w(px(16.0)).flex_none().text_color(text_color).child(prefix))
                    .child(div().flex_1().text_color(text_color).child(line.content.clone()))
            }).collect::<Vec<_>>()
        }).collect();

        v_flex()
            .id(ElementId::NamedInteger("diff-view".into(), self.id as u64))
            .w_full()
            .my(Spacing::Base04.px())
            .bg(ThemeColors::bg_elevated())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_md()
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .px(Spacing::Base06.px())
                    .py(Spacing::Base04.px())
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
