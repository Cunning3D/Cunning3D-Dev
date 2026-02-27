//! Code Editor: Virtualized multi-line editor with line numbers (Zed-isomorphic)

use gpui::{actions, AnyElement, App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render, SharedString, Size, Style, TextRun, UTF16Selection, Window, div, fill, point, prelude::*, px, relative, size};
use crossbeam_channel::Sender;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::ops::Range;
use std::path::PathBuf;
use crate::ai_workspace_gpui::{ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Spacing, UiMetrics, Button, ButtonStyle, TintColor}, protocol::{DiagnosticSnapshot, TextEdit, UiToHost}};
use crate::ai_workspace_gpui::ide::{DisplayMap, HighlightKind, Span, SyntaxViewport, detect_language, highlight_viewport, kinds};
use crate::tabs_registry::ai_workspace::tools::diff::{DiffLineKind, FileDiff, compute_file_diff};
use unicode_segmentation::*;
use std::time::{Duration, Instant};

actions!(code_editor, [Undo, Redo, LineUp, LineDown, PageUp, PageDown, GotoLine, Paste, Cut, Copy, SelectAll, Complete, GotoDefinition, Hover]);

// ─────────────────────────────────────────────────────────────────────────────
// CodeEditor
// ─────────────────────────────────────────────────────────────────────────────

pub struct CodeEditor {
    focus_handle: FocusHandle,
    path: Option<PathBuf>,
    content: String,
    version: u64,
    is_dirty: bool,
    lines: Vec<String>,
    display_map: DisplayMap,
    wrap_cols: Option<usize>,
    syntax: Option<SyntaxViewport>,
    syntax_cached_version: u64,
    syntax_cached_first_row: usize,
    syntax_cached_last_row: usize,
    cursor_line: usize,
    cursor_col: usize,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    is_selecting: bool,
    selection_anchor_line: Option<usize>,
    scroll_offset: f32,
    visible_lines: usize,
    line_height: f32,
    gutter_width: f32,
    input_bounds: Option<Bounds<Pixels>>,
    ui_tx: Sender<UiToHost>,

    pending_edits: VecDeque<Vec<TextEdit>>,
    external_line_marks: HashMap<usize, LineMarkKind>,
    diagnostics: HashMap<usize, u8>,
    completion_items: Vec<String>,
    hover_markdown: Option<String>,
    tool_diff: Option<FileDiff>,
    tool_base: Option<String>,
    tool_target: Option<String>,
    visual_rows: Vec<VisualRow>,
    display_row_to_visual: Vec<usize>,
    external_playback: Option<ExternalPlayback>,
    replay_active: bool,
}

#[derive(Clone, Copy)]
enum LineMarkKind { Added, Removed, Modified }

#[derive(Clone)]
enum VisualRow {
    Buffer { display_row: usize },
    HunkHeader { hunk_idx: usize, anchor_line: usize, text: String },
    RemovedLine { hunk_idx: usize, anchor_line: usize, old_line: Option<usize>, text: String },
}

#[derive(Clone)]
enum PlaybackStep {
    MarkRemoved { line: usize },
    RemoveLine { line: usize },
    SetLine { line: usize, text: String, kind: LineMarkKind },
    InsertLine { line: usize, text: String, kind: LineMarkKind },
}

#[derive(Clone)]
enum SweepApply {
    SetLine { line: usize, text: String },
    FinalizeInsert { line: usize, text: String },
}

#[derive(Clone)]
struct SweepState {
    line: usize,
    started_at: Instant,
    duration: Duration,
    kind: LineMarkKind,
    before: String,
    after: String,
    apply: Option<SweepApply>,
}

#[derive(Clone)]
struct ExternalPlayback {
    steps: VecDeque<PlaybackStep>,
    first_changed_line: Option<usize>,
    next_step_at: Instant,
    sweep: Option<SweepState>,
}

impl ExternalPlayback {
    fn build(old_lines: &[String], new_lines: &[String]) -> Self {
        let mut prefix = 0usize;
        while prefix < old_lines.len() && prefix < new_lines.len() && old_lines[prefix] == new_lines[prefix] { prefix += 1; }
        let mut suffix = 0usize;
        while suffix + prefix < old_lines.len() && suffix + prefix < new_lines.len() {
            let oi = old_lines.len() - 1 - suffix;
            let ni = new_lines.len() - 1 - suffix;
            if old_lines[oi] != new_lines[ni] { break; }
            suffix += 1;
        }

        let old_mid = &old_lines[prefix..old_lines.len().saturating_sub(suffix)];
        let new_mid = &new_lines[prefix..new_lines.len().saturating_sub(suffix)];
        let mut steps = VecDeque::new();
        let min_len = old_mid.len().min(new_mid.len());
        let first = (old_mid.len() > 0 || new_mid.len() > 0).then_some(prefix);

        for i in 0..min_len {
            if old_mid[i] != new_mid[i] {
                // Inline diff shows old line separately (red); the new line should render as green.
                steps.push_back(PlaybackStep::SetLine { line: prefix + i, text: new_mid[i].clone(), kind: LineMarkKind::Added });
            }
        }
        for j in (min_len..old_mid.len()).rev() {
            let line = prefix + j;
            steps.push_back(PlaybackStep::MarkRemoved { line });
            steps.push_back(PlaybackStep::RemoveLine { line });
        }
        for k in min_len..new_mid.len() {
            let line = prefix + k;
            steps.push_back(PlaybackStep::InsertLine { line, text: new_mid[k].clone(), kind: LineMarkKind::Added });
        }

        Self { steps, first_changed_line: first, next_step_at: Instant::now(), sweep: None }
    }
}

impl CodeEditor {
    fn clamp_char_boundary_back(s: &str, i: usize) -> usize {
        let mut i = i.min(s.len());
        while i > 0 && !s.is_char_boundary(i) { i -= 1; }
        i
    }

    fn clamp_char_boundary_fwd(s: &str, i: usize) -> usize {
        let mut i = i.min(s.len());
        while i < s.len() && !s.is_char_boundary(i) { i += 1; }
        i
    }

    fn is_word_char(ch: char) -> bool { ch == '_' || ch.is_alphanumeric() }

    fn word_range_in_line(line: &str, col_byte: usize) -> Range<usize> {
        if line.is_empty() { return 0..0; }
        let col = Self::clamp_char_boundary_back(line, col_byte.min(line.len()));
        let mut left = col;
        while left > 0 {
            let p = Self::clamp_char_boundary_back(line, left.saturating_sub(1));
            let ch = line[p..left].chars().next().unwrap_or(' ');
            if !Self::is_word_char(ch) { break; }
            left = p;
        }
        let mut right = col;
        while right < line.len() {
            let n = Self::clamp_char_boundary_fwd(line, right.saturating_add(1));
            let ch = line[right..n].chars().next().unwrap_or(' ');
            if !Self::is_word_char(ch) { break; }
            right = n;
        }
        if left == right {
            let mut r = col;
            while r < line.len() {
                let n = Self::clamp_char_boundary_fwd(line, r.saturating_add(1));
                let ch = line[r..n].chars().next().unwrap_or(' ');
                if Self::is_word_char(ch) { right = n; break; }
                r = n;
            }
        }
        left..right.max(left)
    }

    pub fn new(ui_tx: Sender<UiToHost>, cx: &mut Context<Self>) -> Self {
        let lines = vec![String::new()];
        let wrap_cols = Some(120);
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            path: None,
            content: String::new(),
            version: 0,
            is_dirty: false,
            lines: lines.clone(),
            display_map: DisplayMap::new(&lines, wrap_cols),
            wrap_cols,
            syntax: None,
            syntax_cached_version: 0,
            syntax_cached_first_row: usize::MAX,
            syntax_cached_last_row: usize::MAX,
            cursor_line: 0,
            cursor_col: 0,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            is_selecting: false,
            selection_anchor_line: None,
            scroll_offset: 0.0,
            visible_lines: 30,
            line_height: 18.0,
            gutter_width: 44.0,
            input_bounds: None,
            ui_tx,

            pending_edits: VecDeque::new(),
            external_line_marks: HashMap::new(),
            diagnostics: HashMap::new(),
            completion_items: Vec::new(),
            hover_markdown: None,
            tool_diff: None,
            tool_base: None,
            tool_target: None,
            visual_rows: Vec::new(),
            display_row_to_visual: Vec::new(),
            external_playback: None,
            replay_active: false,
        };
        this.rebuild_visual_rows();
        this
    }

    fn rehighlight(&mut self) {
        let Some(path) = self.path.as_ref() else { self.syntax = None; return; };
        let Some(lang) = detect_language(path, None) else { self.syntax = None; return; };

        let lh = self.line_height.max(1.0);
        let total = self.display_row_to_visual.len();
        let first_row = (self.scroll_offset / lh).floor().max(0.0) as usize;
        let last_row = (first_row + self.visible_lines + 24).min(total);
        if self.syntax_cached_version == self.version
            && self.syntax_cached_first_row == first_row
            && self.syntax_cached_last_row == last_row
        {
            return;
        }

        let mut min = usize::MAX;
        let mut max = 0usize;
        for dr in first_row..last_row {
            let Some(&vi) = self.display_row_to_visual.get(dr) else { continue; };
            let Some(vr) = self.visual_rows.get(vi) else { continue; };
            if let VisualRow::Buffer { display_row } = *vr {
                let (line, _) = self.display_map.display_to_buffer(display_row);
                min = min.min(line);
                max = max.max(line);
            }
        }

        self.syntax = if min == usize::MAX {
            None
        } else {
            highlight_viewport(lang, &self.content, min, max.saturating_add(1))
        };
        self.syntax_cached_version = self.version;
        self.syntax_cached_first_row = first_row;
        self.syntax_cached_last_row = last_row;
    }

    pub fn set_content(&mut self, path: PathBuf, content: String, version: u64, cx: &mut Context<Self>) {
        self.path = Some(path);
        self.lines = content.split('\n').map(|l| l.to_string()).collect();
        if self.lines.is_empty() { self.lines.push(String::new()); }
        self.content = content;
        self.version = version;
        self.syntax_cached_first_row = usize::MAX;
        self.syntax_cached_last_row = usize::MAX;
        self.display_map = DisplayMap::new(&self.lines, self.wrap_cols);
        self.rehighlight();
        self.is_dirty = false;
        self.external_line_marks.clear();
        self.diagnostics.clear();
        self.completion_items.clear();
        self.hover_markdown = None;
        self.tool_diff = None;
        self.tool_base = None;
        self.tool_target = None;
        self.rebuild_visual_rows();
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.is_selecting = false;
        self.selection_anchor_line = None;
        self.scroll_offset = 0.0;
        self.pending_edits.clear();
        self.external_playback = None;
        self.replay_active = false;
        if let Some(p) = self.path.clone() {
            let _ = self.ui_tx.send(UiToHost::IdeCursorChanged { path: p, line: 0, col: 0 });
            let _ = self.ui_tx.send(UiToHost::IdeSelectionCleared { path: self.path.as_ref().unwrap().clone() });
        }
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.path = None;
        self.content.clear();
        self.lines = vec![String::new()];
        self.display_map = DisplayMap::new(&self.lines, self.wrap_cols);
        self.syntax = None;
        self.syntax_cached_version = 0;
        self.syntax_cached_first_row = usize::MAX;
        self.syntax_cached_last_row = usize::MAX;
        self.version = 0;
        self.is_dirty = false;
        self.external_line_marks.clear();
        self.diagnostics.clear();
        self.completion_items.clear();
        self.hover_markdown = None;
        self.tool_diff = None;
        self.tool_base = None;
        self.tool_target = None;
        self.rebuild_visual_rows();
        self.external_playback = None;
        self.replay_active = false;
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.is_selecting = false;
        self.selection_anchor_line = None;
        self.pending_edits.clear();
        cx.notify();
    }

    pub fn set_dirty(&mut self, is_dirty: bool, cx: &mut Context<Self>) {
        self.is_dirty = is_dirty;
        cx.notify();
    }

    pub fn set_diagnostics(&mut self, path: PathBuf, diagnostics: Vec<DiagnosticSnapshot>, cx: &mut Context<Self>) {
        if self.path.as_ref() != Some(&path) { return; }
        self.diagnostics.clear();
        for d in diagnostics {
            let ln = d.start_line as usize;
            let sev = d.severity;
            self.diagnostics.entry(ln).and_modify(|s| *s = (*s).min(sev)).or_insert(sev);
        }
        cx.notify();
    }

    pub fn set_completions(&mut self, path: PathBuf, items: Vec<String>, cx: &mut Context<Self>) {
        if self.path.as_ref() != Some(&path) { return; }
        self.completion_items = items;
        cx.notify();
    }

    pub fn set_hover(&mut self, path: PathBuf, markdown: String, cx: &mut Context<Self>) {
        if self.path.as_ref() != Some(&path) { return; }
        self.hover_markdown = (!markdown.trim().is_empty()).then_some(markdown);
        cx.notify();
    }

    fn complete(&mut self, _: &Complete, _: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.path.clone() else { return; };
        let _ = self.ui_tx.send(UiToHost::IdeRequestCompletion { path: p, line: self.cursor_line as u32, col: self.cursor_col as u32 });
        cx.notify();
    }

    fn goto_definition(&mut self, _: &GotoDefinition, _: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.path.clone() else { return; };
        let _ = self.ui_tx.send(UiToHost::IdeRequestDefinition { path: p, line: self.cursor_line as u32, col: self.cursor_col as u32 });
        cx.notify();
    }

    fn hover(&mut self, _: &Hover, _: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.path.clone() else { return; };
        let _ = self.ui_tx.send(UiToHost::IdeRequestHover { path: p, line: self.cursor_line as u32, col: self.cursor_col as u32 });
        cx.notify();
    }

    pub fn set_cursor_from_host(&mut self, path: PathBuf, line: usize, col: usize, cx: &mut Context<Self>) {
        if self.path.as_ref() != Some(&path) { return; }
        self.cursor_line = line.min(self.lines.len().saturating_sub(1));
        self.cursor_col = col.min(self.lines.get(self.cursor_line).map(|l| l.len()).unwrap_or(0));
        let off = self.offset_at(self.cursor_line, self.cursor_col);
        self.selected_range = off..off;
        self.selection_reversed = false;
        self.marked_range = None;
        self.is_selecting = false;
        self.selection_anchor_line = None;
        self.ensure_cursor_visible();
        cx.notify();
    }

    pub fn tick_external_playback(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(mut pb) = self.external_playback.take() else { return false; };
        self.replay_active = true;
        let now = Instant::now();

        if let Some(mut sw) = pb.sweep.take() {
            if now.duration_since(sw.started_at) < sw.duration {
                pb.sweep = Some(sw);
                self.external_playback = Some(pb);
                cx.notify();
                return true;
            }
            if let Some(apply) = sw.apply.take() {
                match apply {
                    SweepApply::SetLine { line, text } | SweepApply::FinalizeInsert { line, text } => {
                        if line < self.lines.len() { self.lines[line] = text; }
                    }
                }
            }
            if self.lines.is_empty() { self.lines.push(String::new()); }
            self.content = self.lines.join("\n");
            self.display_map = DisplayMap::new(&self.lines, self.wrap_cols);
            self.rehighlight();
            self.rebuild_visual_rows();
            pb.next_step_at = now + Duration::from_millis(UiMetrics::TOOL_REPLAY_STEP_MS);
            self.external_playback = Some(pb);
            cx.notify();
            return true;
        }

        if now < pb.next_step_at {
            self.external_playback = Some(pb);
            return false;
        }

        let Some(step) = pb.steps.pop_front() else {
            self.external_playback = None;
            self.replay_active = false;
            self.rebuild_visual_rows();
            cx.notify();
            return false;
        };
        match step {
            PlaybackStep::MarkRemoved { line } => { self.external_line_marks.insert(line, LineMarkKind::Removed); }
            PlaybackStep::RemoveLine { line } => {
                if line < self.lines.len() { self.lines.remove(line); }
                self.external_line_marks.remove(&line);
            }
            PlaybackStep::SetLine { line, text, kind } => {
                self.external_line_marks.insert(line, kind);
                if line < self.lines.len() {
                    let before = self.lines[line].clone();
                    let after = text.clone();
                    pb.sweep = Some(SweepState {
                        line,
                        started_at: now,
                        duration: Duration::from_millis(UiMetrics::TOOL_REPLAY_SWEEP_MS),
                        kind,
                        before,
                        after,
                        apply: Some(SweepApply::SetLine { line, text }),
                    });
                }
            }
            PlaybackStep::InsertLine { line, text, kind } => {
                let line = line.min(self.lines.len());
                self.external_line_marks.insert(line, kind);
                // Insert placeholder first, then sweep-reveal and finalize.
                self.lines.insert(line, String::new());
                pb.sweep = Some(SweepState {
                    line,
                    started_at: now,
                    duration: Duration::from_millis(UiMetrics::TOOL_REPLAY_SWEEP_MS),
                    kind,
                    before: String::new(),
                    after: text.clone(),
                    apply: Some(SweepApply::FinalizeInsert { line, text }),
                });
            }
        }
        if self.lines.is_empty() { self.lines.push(String::new()); }
        self.content = self.lines.join("\n");
        self.display_map = DisplayMap::new(&self.lines, self.wrap_cols);
        self.rehighlight();
        self.rebuild_visual_rows();
        if let Some(first) = pb.first_changed_line {
            self.cursor_line = first.min(self.lines.len().saturating_sub(1));
            self.cursor_col = 0;
            self.ensure_cursor_visible();
        }
        pb.next_step_at = now + Duration::from_millis(UiMetrics::TOOL_REPLAY_STEP_MS);
        self.external_playback = Some(pb);
        cx.notify();
        true
    }

    fn rebuild_visual_rows(&mut self) {
        self.visual_rows.clear();
        self.display_row_to_visual.clear();

        if self.external_playback.is_some() || self.replay_active {
            let rows = self.display_map.row_count();
            for di in 0..rows {
                self.display_row_to_visual.push(self.visual_rows.len());
                self.visual_rows.push(VisualRow::Buffer { display_row: di });
            }
            return;
        }

        let mut blocks: HashMap<usize, (usize, String, Vec<(Option<usize>, String)>)> = HashMap::new();
        let mut end_blocks: Vec<(usize, usize, String, Vec<(Option<usize>, String)>)> = Vec::new(); // (anchor, hunk_idx, header, removed)
        if let Some(fd) = self.tool_diff.as_ref() {
            for (hi, h) in fd.hunks.iter().enumerate() {
                let anchor_raw = h.new_start.saturating_sub(1);
                let anchor_line = anchor_raw.min(self.lines.len().saturating_sub(1));
                let header = format!("@@ -{},{} +{},{} @@", h.old_start, h.old_count, h.new_start, h.new_count);
                let removed = h
                    .lines
                    .iter()
                    .filter(|l| l.kind == DiffLineKind::Removed)
                    .map(|l| (l.line_num_old, l.content.clone()))
                    .collect::<Vec<_>>();
                if anchor_raw >= self.lines.len() {
                    end_blocks.push((anchor_line, hi, header, removed));
                } else {
                    blocks.insert(anchor_line, (hi, header, removed));
                }
            }
        }

        let rows = self.display_map.row_count();
        for di in 0..rows {
            let dr = self.display_map.row(di);
            if !dr.is_continuation {
                if let Some((hunk_idx, header, removed)) = blocks.get(&dr.buffer_line).cloned() {
                    self.visual_rows.push(VisualRow::HunkHeader { hunk_idx, anchor_line: dr.buffer_line, text: header });
                    for (old_line, text) in removed {
                        self.visual_rows.push(VisualRow::RemovedLine { hunk_idx, anchor_line: dr.buffer_line, old_line, text });
                    }
                }
            }
            self.display_row_to_visual.push(self.visual_rows.len());
            self.visual_rows.push(VisualRow::Buffer { display_row: di });
        }

        end_blocks.sort_by_key(|(_, hi, _, _)| *hi);
        for (anchor_line, hunk_idx, header, removed) in end_blocks {
            self.visual_rows.push(VisualRow::HunkHeader { hunk_idx, anchor_line, text: header });
            for (old_line, text) in removed {
                self.visual_rows.push(VisualRow::RemovedLine { hunk_idx, anchor_line, old_line, text });
            }
        }
    }

    fn can_edit(&self) -> bool {
        self.external_playback.is_none()
    }

    // If the user starts editing during tool replay, immediately jump to the final tool target
    // and cancel playback so typing/backspace always works.
    fn ensure_editable(&mut self, cx: &mut Context<Self>) {
        if self.external_playback.is_none() { return; }
        self.external_playback = None;
        self.replay_active = false;
        if let Some(after) = self.tool_target.clone() {
            self.content = after;
            self.lines = self.content.split('\n').map(|l| l.to_string()).collect();
            if self.lines.is_empty() { self.lines.push(String::new()); }
            self.display_map = DisplayMap::new(&self.lines, self.wrap_cols);
            self.rehighlight();
        }
        self.rebuild_visual_rows();
        cx.notify();
    }

    fn tool_apply_all(&mut self, cx: &mut Context<Self>) {
        self.tool_diff = None;
        self.tool_base = None;
        self.tool_target = None;
        self.external_line_marks.clear();
        self.external_playback = None;
        self.replay_active = false;
        self.rebuild_visual_rows();
        self.scroll_offset = 0.0;
        cx.notify();
    }

    fn tool_discard_all(&mut self, cx: &mut Context<Self>) {
        let Some(base) = self.tool_base.clone() else { return; };
        let Some(path) = self.path.clone() else { return; };
        let edit = TextEdit { start_offset: 0, end_offset: self.content.len(), new_text: base };
        let _ = self.ui_tx.send(UiToHost::IdeEditFile { path, version: self.version, edits: vec![edit.clone()] });
        self.pending_edits.push_back(vec![edit.clone()]);
        self.apply_local_edit(&edit, cx);
        self.tool_apply_all(cx);
    }

    fn tool_replay(&mut self, cx: &mut Context<Self>) {
        let Some(base) = self.tool_base.clone() else { return; };
        let Some(target) = self.tool_target.clone() else { return; };
        let mut old_lines: Vec<String> = base.split('\n').map(|l| l.to_string()).collect();
        let mut new_lines: Vec<String> = target.split('\n').map(|l| l.to_string()).collect();
        if old_lines.is_empty() { old_lines.push(String::new()); }
        if new_lines.is_empty() { new_lines.push(String::new()); }
        self.scroll_offset = 0.0;
        self.external_line_marks.clear();
        self.external_playback = Some(ExternalPlayback::build(&old_lines, &new_lines));
        self.replay_active = true;
        self.content = old_lines.join("\n");
        self.lines = old_lines;
        self.display_map = DisplayMap::new(&self.lines, self.wrap_cols);
        self.rehighlight();
        self.rebuild_visual_rows();
        cx.notify();
    }

    fn tool_apply_hunk(&mut self, hunk_idx: usize, cx: &mut Context<Self>) {
        let Some(base) = self.tool_base.clone() else { return; };
        let Some(fd) = self.tool_diff.clone() else { return; };
        let Some(h) = fd.hunks.get(hunk_idx) else { return; };
        let mut base_lines: Vec<String> = base.split('\n').map(|l| l.to_string()).collect();
        let cur_lines: Vec<String> = self.content.split('\n').map(|l| l.to_string()).collect();
        let o0 = h.old_start.saturating_sub(1);
        let n0 = h.new_start.saturating_sub(1);
        let oe = o0.saturating_add(h.old_count).min(base_lines.len());
        let ne = n0.saturating_add(h.new_count).min(cur_lines.len());
        base_lines.splice(o0.min(base_lines.len())..oe, cur_lines[n0.min(cur_lines.len())..ne].iter().cloned());
        let new_base = base_lines.join("\n");
        self.tool_base = Some(new_base.clone());
        self.tool_diff = compute_file_diff(fd.file_path, &new_base, &self.content);
        self.external_line_marks = self.tool_diff.as_ref().map(Self::marks_from_tool_diff).unwrap_or_default();
        self.rebuild_visual_rows();
        if self.tool_diff.is_none() { self.tool_target = None; self.tool_base = None; }
        cx.notify();
    }

    fn tool_discard_hunk(&mut self, hunk_idx: usize, cx: &mut Context<Self>) {
        let Some(base) = self.tool_base.clone() else { return; };
        let Some(fd) = self.tool_diff.clone() else { return; };
        let Some(h) = fd.hunks.get(hunk_idx) else { return; };
        let base_lines: Vec<String> = base.split('\n').map(|l| l.to_string()).collect();
        let mut cur_lines: Vec<String> = self.content.split('\n').map(|l| l.to_string()).collect();
        let o0 = h.old_start.saturating_sub(1);
        let n0 = h.new_start.saturating_sub(1);
        let oe = o0.saturating_add(h.old_count).min(base_lines.len());
        let ne = n0.saturating_add(h.new_count).min(cur_lines.len());
        cur_lines.splice(n0.min(cur_lines.len())..ne, base_lines[o0.min(base_lines.len())..oe].iter().cloned());
        let new_cur = cur_lines.join("\n");
        let Some(path) = self.path.clone() else { return; };
        let edit = TextEdit { start_offset: 0, end_offset: self.content.len(), new_text: new_cur.clone() };
        let _ = self.ui_tx.send(UiToHost::IdeEditFile { path, version: self.version, edits: vec![edit.clone()] });
        self.pending_edits.push_back(vec![edit.clone()]);
        self.apply_local_edit(&edit, cx);
        self.tool_target = Some(self.content.clone());
        self.tool_diff = compute_file_diff(fd.file_path, &base, &self.content);
        self.external_line_marks = self.tool_diff.as_ref().map(Self::marks_from_tool_diff).unwrap_or_default();
        self.rebuild_visual_rows();
        cx.notify();
    }

    pub fn apply_file_changed(&mut self, path: PathBuf, version: u64, edits: Vec<TextEdit>, cx: &mut Context<Self>) {
        if self.path.as_ref() != Some(&path) {
            return;
        }

        let is_ack = self
            .pending_edits
            .front()
            .is_some_and(|p| Self::edits_eq(p, &edits));
        if is_ack {
            self.pending_edits.pop_front();
        } else {
            // External change (tool / reload / other source). Capture base+after for Zed-like diff preview.
            let before = self.content.clone();
            let mut after = self.content.clone();
            Self::apply_edits_to_string(&mut after, &edits);
            self.tool_diff = compute_file_diff(path.to_string_lossy().to_string(), &before, &after);
            self.tool_base = Some(before.clone());
            self.tool_target = Some(after.clone());
            self.scroll_offset = 0.0;
            self.rebuild_visual_rows();

            if self.tool_diff.is_some() {
                // Zed-like "instant edits": replay every tool-driven change inline (top-to-bottom).
                self.tool_replay(cx);
            } else {
                self.external_line_marks = self.tool_diff.as_ref().map(Self::marks_from_tool_diff).unwrap_or_default();
                self.content = after;
                self.lines = self.content.split('\n').map(|l| l.to_string()).collect();
                if self.lines.is_empty() { self.lines.push(String::new()); }
                self.display_map = DisplayMap::new(&self.lines, self.wrap_cols);
                self.rehighlight();
                self.rebuild_visual_rows();
            }
        }

        self.version = version;
        cx.notify();
    }

    fn marks_from_edits(before: &str, edits: &[TextEdit]) -> HashMap<usize, LineMarkKind> {
        fn line_at_offset(s: &str, off: usize) -> usize {
            let off = off.min(s.len());
            s[..off].chars().filter(|&c| c == '\n').count()
        }
        let mut out = HashMap::new();
        for e in edits {
            let start_ln = line_at_offset(before, e.start_offset);
            let end_ln = line_at_offset(before, e.end_offset);
            let kind = if e.start_offset == e.end_offset && !e.new_text.is_empty() {
                LineMarkKind::Added
            } else if e.new_text.is_empty() && e.end_offset > e.start_offset {
                LineMarkKind::Removed
            } else {
                LineMarkKind::Modified
            };
            let n = (end_ln.saturating_sub(start_ln)).max(0).min(12); // clamp for safety
            for ln in start_ln..=start_ln + n {
                out.entry(ln).or_insert(kind);
            }
        }
        out
    }

    fn marks_from_tool_diff(fd: &FileDiff) -> HashMap<usize, LineMarkKind> {
        let mut out = HashMap::new();
        for h in &fd.hunks {
            for l in &h.lines {
                match l.kind {
                    DiffLineKind::Added => {
                        if let Some(n) = l.line_num_new.and_then(|n| n.checked_sub(1)) {
                            out.entry(n).or_insert(LineMarkKind::Added);
                        }
                    }
                    DiffLineKind::Removed => {}
                    DiffLineKind::Context => {}
                }
            }
        }
        out
    }

    fn edits_eq(a: &[TextEdit], b: &[TextEdit]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b.iter()).all(|(x, y)| {
            x.start_offset == y.start_offset
                && x.end_offset == y.end_offset
                && x.new_text == y.new_text
        })
    }

    fn apply_edits_to_string(content: &mut String, edits: &[TextEdit]) {
        let mut offset_delta: isize = 0;
        for e in edits {
            let start = (e.start_offset as isize + offset_delta).max(0) as usize;
            let end = (e.end_offset as isize + offset_delta).max(0) as usize;
            let start = start.min(content.len());
            let end = end.min(content.len());
            if start > end {
                continue;
            }
            content.replace_range(start..end, &e.new_text);
            offset_delta += e.new_text.len() as isize - (end - start) as isize;
        }
    }

    pub fn is_dirty(&self) -> bool { self.is_dirty }
    pub fn path(&self) -> Option<&PathBuf> { self.path.as_ref() }
    pub fn line_count(&self) -> usize { self.visual_rows.len() }

    // ─────────────────────────────────────────────────────────────────────────
    // Navigation
    // ─────────────────────────────────────────────────────────────────────────

    fn move_cursor(&mut self, line: usize, col: usize, cx: &mut Context<Self>) {
        self.cursor_line = line.min(self.lines.len().saturating_sub(1));
        self.cursor_col = col.min(self.lines.get(self.cursor_line).map(|l| l.len()).unwrap_or(0));
        let off = self.offset_at(self.cursor_line, self.cursor_col);
        self.selected_range = off..off;
        self.selection_reversed = false;
        self.marked_range = None;
        self.is_selecting = false;
        self.selection_anchor_line = None;
        if let Some(p) = self.path.clone() {
            let _ = self.ui_tx.send(UiToHost::IdeCursorChanged {
                path: p,
                line: self.cursor_line as u32,
                col: self.cursor_col as u32,
            });
            let _ = self.ui_tx.send(UiToHost::IdeSelectionCleared { path: self.path.as_ref().unwrap().clone() });
        }
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn set_line_selection(&mut self, anchor: usize, head: usize, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else { return; };
        let start = anchor.min(head);
        let end = anchor.max(head);
        if start == end {
            let off = self.offset_at(head, 0);
            self.selected_range = off..off;
            self.selection_reversed = false;
            let _ = self.ui_tx.send(UiToHost::IdeSelectionCleared { path });
            cx.notify();
            return;
        }
        let so = self.offset_at(start, 0);
        let eo = self.offset_at(end, self.lines.get(end).map(|l| l.len()).unwrap_or(0));
        self.selected_range = so..eo;
        self.selection_reversed = false;
        let _ = self.ui_tx.send(UiToHost::IdeSelectionChanged {
            path,
            start_line: start as u32,
            end_line: end as u32,
        });
        cx.notify();
    }

    fn mouse_down_on_row(
        &mut self,
        row_idx: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.is_focusing() {
            return;
        }
        self.focus_handle.focus(window, cx);
        self.is_selecting = true;
        let (line, col) = self.display_map.display_to_buffer(row_idx);
        self.selection_anchor_line = Some(line);
        self.cursor_line = line.min(self.lines.len().saturating_sub(1));
        self.cursor_col = col.min(self.lines.get(self.cursor_line).map(|l| l.len()).unwrap_or(0));
        let off = self.offset_at(self.cursor_line, self.cursor_col);
        match event.click_count {
            2 => {
                let l = self.lines.get(self.cursor_line).cloned().unwrap_or_default();
                let r = Self::word_range_in_line(&l, self.cursor_col);
                let so = self.offset_at(self.cursor_line, r.start);
                let eo = self.offset_at(self.cursor_line, r.end);
                self.selected_range = so..eo;
                self.cursor_col = r.end.min(l.len());
            }
            3.. => {
                let len = self.lines.get(self.cursor_line).map(|l| l.len()).unwrap_or(0);
                let so = self.offset_at(self.cursor_line, 0);
                let eo = self.offset_at(self.cursor_line, len);
                self.selected_range = so..eo;
                self.cursor_col = len;
            }
            _ => {
                self.selected_range = off..off;
            }
        }
        self.selection_reversed = false;
        self.marked_range = None;
        if let Some(p) = self.path.clone() {
            let _ = self.ui_tx.send(UiToHost::IdeCursorChanged {
                path: p,
                line: self.cursor_line as u32,
                col: self.cursor_col as u32,
            });
            if self.selected_range.is_empty() {
                let _ = self.ui_tx.send(UiToHost::IdeSelectionCleared { path: self.path.as_ref().unwrap().clone() });
            } else {
                let a = self.offset_to_line_col(self.selected_range.start).0;
                let b = self.offset_to_line_col(self.selected_range.end).0;
                let _ = self.ui_tx.send(UiToHost::IdeSelectionChanged { path: self.path.as_ref().unwrap().clone(), start_line: a.min(b) as u32, end_line: a.max(b) as u32 });
            }
        }
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn mouse_down_at_point(&mut self, point: Point<Pixels>, click_count: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = self.input_bounds else { return; };
        self.focus_handle.focus(window, cx);
        self.is_selecting = true;
        let Some((line, col)) = self.point_to_line_col_byte(point, window, bounds) else { return; };
        self.selection_anchor_line = Some(line);
        self.cursor_line = line.min(self.lines.len().saturating_sub(1));
        self.cursor_col = col.min(self.lines.get(self.cursor_line).map(|l| l.len()).unwrap_or(0));
        let off = self.offset_at(self.cursor_line, self.cursor_col);
        match click_count {
            2 => {
                let l = self.lines.get(self.cursor_line).cloned().unwrap_or_default();
                let r = Self::word_range_in_line(&l, self.cursor_col);
                let so = self.offset_at(self.cursor_line, r.start);
                let eo = self.offset_at(self.cursor_line, r.end);
                self.selected_range = so..eo;
                self.cursor_col = r.end.min(l.len());
            }
            3.. => {
                let len = self.lines.get(self.cursor_line).map(|l| l.len()).unwrap_or(0);
                let so = self.offset_at(self.cursor_line, 0);
                let eo = self.offset_at(self.cursor_line, len);
                self.selected_range = so..eo;
                self.cursor_col = len;
            }
            _ => self.selected_range = off..off,
        }
        self.selection_reversed = false;
        self.marked_range = None;
        if let Some(p) = self.path.clone() {
            let _ = self.ui_tx.send(UiToHost::IdeCursorChanged { path: p, line: self.cursor_line as u32, col: self.cursor_col as u32 });
            if self.selected_range.is_empty() {
                let _ = self.ui_tx.send(UiToHost::IdeSelectionCleared { path: self.path.as_ref().unwrap().clone() });
            } else {
                let a = self.offset_to_line_col(self.selected_range.start).0;
                let b = self.offset_to_line_col(self.selected_range.end).0;
                let _ = self.ui_tx.send(UiToHost::IdeSelectionChanged { path: self.path.as_ref().unwrap().clone(), start_line: a.min(b) as u32, end_line: a.max(b) as u32 });
            }
        }
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn mouse_move_on_row(
        &mut self,
        row_idx: usize,
        _event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_selecting {
            return;
        }
        let Some(anchor) = self.selection_anchor_line else {
            return;
        };
        let h = self.display_map.row(row_idx).buffer_line;
        self.set_line_selection(anchor, h, cx);
    }

    fn mouse_move_at_point(&mut self, point: Point<Pixels>, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_selecting { return; }
        let Some(bounds) = self.input_bounds else { return; };
        let local_y: f32 = (point.y - bounds.origin.y).into();
        let y = (local_y + self.scroll_offset).max(0.0);
        let mut vr = (y / self.line_height).floor() as usize;
        vr = vr.min(self.visual_rows.len().saturating_sub(1));
        let Some(anchor) = self.selection_anchor_line else { return; };
        let h = match self.visual_rows.get(vr) {
            Some(VisualRow::Buffer { display_row }) => self.display_map.row(*display_row).buffer_line,
            Some(VisualRow::HunkHeader { anchor_line, .. }) => *anchor_line,
            Some(VisualRow::RemovedLine { anchor_line, .. }) => *anchor_line,
            None => anchor,
        };
        self.set_line_selection(anchor, h, cx);
    }

    fn mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_selecting = false;
        self.selection_anchor_line = None;
        cx.notify();
    }

    fn ensure_cursor_visible(&mut self) {
        let dr = self.display_map.buffer_to_display_row(self.cursor_line, self.cursor_col);
        let vr = self.display_row_to_visual.get(dr).copied().unwrap_or(0);
        let cursor_y = vr as f32;
        let first_visible = self.scroll_offset / self.line_height;
        let last_visible = first_visible + self.visible_lines as f32 - 1.0;
        if cursor_y < first_visible { self.scroll_offset = cursor_y * self.line_height; }
        else if cursor_y > last_visible { self.scroll_offset = (cursor_y - self.visible_lines as f32 + 1.0) * self.line_height; }
        self.scroll_offset = self.scroll_offset.max(0.0);
    }

    fn line_up(&mut self, _: &LineUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.cursor_line > 0 { self.move_cursor(self.cursor_line - 1, self.cursor_col, cx); }
    }

    fn line_down(&mut self, _: &LineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(self.cursor_line + 1, self.cursor_col, cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        let new_line = self.cursor_line.saturating_sub(self.visible_lines);
        self.move_cursor(new_line, self.cursor_col, cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        let new_line = (self.cursor_line + self.visible_lines).min(self.lines.len().saturating_sub(1));
        self.move_cursor(new_line, self.cursor_col, cx);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Editing (sends edits to host)
    // ─────────────────────────────────────────────────────────────────────────

    fn insert_char(&mut self, ch: char, cx: &mut Context<Self>) {
        self.ensure_editable(cx);
        let Some(path) = self.path.clone() else { return; };
        let range = self.marked_range.clone().unwrap_or(self.selected_range.clone());
        let edit = TextEdit { start_offset: range.start, end_offset: range.end, new_text: ch.to_string() };
        let _ = self.ui_tx.send(UiToHost::IdeEditFile { path, version: self.version, edits: vec![edit.clone()] });
        self.pending_edits.push_back(vec![edit.clone()]);
        self.apply_local_edit(&edit, cx);
    }

    fn delete_backward(&mut self, cx: &mut Context<Self>) {
        self.ensure_editable(cx);
        let Some(path) = self.path.clone() else { return; };
        let range = self.marked_range.clone().unwrap_or(self.selected_range.clone());
        let (start, end) = if range.is_empty() {
            let cur = self.offset_at(self.cursor_line, self.cursor_col);
            let prev = self.previous_boundary(cur);
            (prev, cur)
        } else { (range.start, range.end) };
        if start == end { return; }
        let edit = TextEdit { start_offset: start, end_offset: end, new_text: String::new() };
        let _ = self.ui_tx.send(UiToHost::IdeEditFile { path, version: self.version, edits: vec![edit.clone()] });
        self.pending_edits.push_back(vec![edit.clone()]);
        self.apply_local_edit(&edit, cx);
    }

    fn insert_newline(&mut self, cx: &mut Context<Self>) {
        self.ensure_editable(cx);
        let Some(path) = self.path.clone() else { return; };
        let range = self.marked_range.clone().unwrap_or(self.selected_range.clone());
        let edit = TextEdit { start_offset: range.start, end_offset: range.end, new_text: "\n".to_string() };
        let _ = self.ui_tx.send(UiToHost::IdeEditFile { path, version: self.version, edits: vec![edit.clone()] });
        self.pending_edits.push_back(vec![edit.clone()]);
        self.apply_local_edit(&edit, cx);
        self.ensure_cursor_visible();
    }

    fn insert_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_editable(cx);
        let Some(path) = self.path.clone() else { return; };
        let range = self.marked_range.clone().unwrap_or(self.selected_range.clone());
        let edit = TextEdit { start_offset: range.start, end_offset: range.end, new_text: text.to_string() };
        let _ = self.ui_tx.send(UiToHost::IdeEditFile { path, version: self.version, edits: vec![edit.clone()] });
        self.pending_edits.push_back(vec![edit.clone()]);
        self.apply_local_edit(&edit, cx);
        let _ = window;
    }

    fn apply_local_edit(&mut self, edit: &TextEdit, cx: &mut Context<Self>) {
        let s = edit.start_offset.min(self.content.len());
        let e = edit.end_offset.min(self.content.len());
        if s > e { return; }
        self.content.replace_range(s..e, &edit.new_text);
        self.lines = self.content.split('\n').map(|l| l.to_string()).collect();
        if self.lines.is_empty() { self.lines.push(String::new()); }
        self.display_map = DisplayMap::new(&self.lines, self.wrap_cols);
        self.rehighlight();
        self.rebuild_visual_rows();
        let new_off = s + edit.new_text.len();
        self.cursor_line = self.content[..new_off].chars().filter(|&c| c == '\n').count().min(self.lines.len().saturating_sub(1));
        self.cursor_col = new_off.saturating_sub(self.offset_at(self.cursor_line, 0)).min(self.lines.get(self.cursor_line).map(|l| l.len()).unwrap_or(0));
        self.selected_range = new_off..new_off;
        self.selection_reversed = false;
        self.marked_range = None;
        self.is_dirty = true;
        self.version = self.version.saturating_add(1);
        if let Some(p) = self.path.clone() {
            let _ = self.ui_tx.send(UiToHost::IdeCursorChanged { path: p, line: self.cursor_line as u32, col: self.cursor_col as u32 });
            let _ = self.ui_tx.send(UiToHost::IdeSelectionCleared { path: self.path.as_ref().unwrap().clone() });
        }
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        let o = offset.min(self.content.len());
        self.content.grapheme_indices(true).rev().find_map(|(i, _)| (i < o).then_some(i)).unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        let o = offset.min(self.content.len());
        self.content.grapheme_indices(true).find_map(|(i, _)| (i > o).then_some(i)).unwrap_or(self.content.len())
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() { return; }
        cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected_range.clone()].to_string()));
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() { return; }
        cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected_range.clone()].to_string()));
        self.insert_text("", window, cx);
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
            self.insert_text(&text, window, cx);
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
        if let Some(p) = self.path.clone() {
            let _ = self.ui_tx.send(UiToHost::IdeSelectionChanged { path: p, start_line: 0, end_line: self.lines.len().saturating_sub(1) as u32 });
        }
        cx.notify();
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else { return; };
        let _ = self.ui_tx.send(UiToHost::IdeUndo { path });
        cx.notify();
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else { return; };
        let _ = self.ui_tx.send(UiToHost::IdeRedo { path });
        cx.notify();
    }

    fn offset_at(&self, line: usize, col: usize) -> usize {
        let mut offset = 0;
        for (i, l) in self.lines.iter().enumerate() {
            if i == line { return offset + col.min(l.len()); }
            offset += l.len() + 1; // +1 for newline
        }
        offset
    }

    fn offset_to_line_col(&self, offset: usize) -> (usize, usize) {
        let off = offset.min(self.content.len());
        let mut line = 0usize;
        let mut line_start = 0usize;
        for (i, b) in self.content.as_bytes().iter().enumerate() {
            if i >= off { break; }
            if *b == b'\n' { line += 1; line_start = i + 1; }
        }
        (line.min(self.lines.len().saturating_sub(1)), off.saturating_sub(line_start))
    }

    fn changed_lines_sorted(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.external_line_marks.keys().copied().collect();
        v.sort_unstable();
        v
    }

    fn jump_change(&mut self, dir: i32, cx: &mut Context<Self>) {
        let v = self.changed_lines_sorted();
        if v.is_empty() { return; }
        let cur = self.cursor_line;
        let next = if dir > 0 {
            v.iter().copied().find(|&l| l > cur).unwrap_or(v[0])
        } else {
            v.iter().copied().rev().find(|&l| l < cur).unwrap_or(*v.last().unwrap())
        };
        self.move_cursor(next, 0, cx);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Rendering
    // ─────────────────────────────────────────────────────────────────────────

    fn render_row(&self, row_idx: usize, is_current: bool, cx: &mut Context<Self>) -> AnyElement {
        let r = self.display_map.row(row_idx);
        let buf_line = r.buffer_line.min(self.lines.len().saturating_sub(1));
        let full = self.lines.get(buf_line).cloned().unwrap_or_default();
        let mut bs = r.byte_start.min(full.len());
        let mut be = r.byte_end.min(full.len()).max(bs);
        bs = Self::clamp_char_boundary_back(&full, bs);
        be = Self::clamp_char_boundary_fwd(&full, be);
        if be < bs { be = bs; }
        let slice = full.get(bs..be).unwrap_or("").to_string();
        let line_num = if r.is_continuation { "".to_string() } else { (buf_line + 1).to_string() };
        let row_idx_down = row_idx;
        let row_idx_move = row_idx;
        let mark = self.external_line_marks.get(&buf_line).copied();
        let mark_bg = match mark {
            Some(LineMarkKind::Added) => ThemeColors::diff_added_bg(),
            Some(LineMarkKind::Removed) => ThemeColors::diff_removed_bg(),
            // Cursor-style: treat "modified" as the new line (green); old line is rendered separately (red).
            Some(LineMarkKind::Modified) => ThemeColors::diff_added_bg(),
            None => gpui::transparent_black(),
        };
        let mark_bar = match mark {
            Some(LineMarkKind::Added) => ThemeColors::diff_added_text(),
            Some(LineMarkKind::Removed) => ThemeColors::diff_removed_text(),
            Some(LineMarkKind::Modified) => ThemeColors::diff_added_text(),
            None => gpui::transparent_black(),
        };
        let mark_sym = match mark {
            Some(LineMarkKind::Added) => "+",
            Some(LineMarkKind::Removed) => "-",
            Some(LineMarkKind::Modified) => "+",
            None => " ",
        };
        let diag = self.diagnostics.get(&buf_line).copied();
        let diag_sym = diag.map(|_| "●").unwrap_or(" ");
        let diag_color = match diag {
            Some(1) => LabelColor::Error,
            Some(2) => LabelColor::Warning,
            _ => LabelColor::Muted,
        };
        // Keep diff colors even on the current line (Cursor-style); only use selection bg when unmarked.
        let row_bg = if mark.is_some() { mark_bg } else if is_current { ThemeColors::bg_selected() } else { mark_bg };
        let sweep = self.external_playback.as_ref().and_then(|pb| pb.sweep.as_ref()).cloned();
        let sweep_progress = sweep
            .filter(|sw| sw.line == buf_line)
            .map(|sw| {
                let t = Instant::now().duration_since(sw.started_at);
                let p = if sw.duration.as_millis() == 0 { 1.0 } else { (t.as_secs_f32() / sw.duration.as_secs_f32()).clamp(0.0, 1.0) };
                let c = match sw.kind {
                    LineMarkKind::Added => ThemeColors::diff_added_text().opacity(0.16),
                    LineMarkKind::Removed => ThemeColors::diff_removed_text().opacity(0.16),
                    LineMarkKind::Modified => ThemeColors::diff_added_text().opacity(0.14),
                };
                (p, c)
            });

        fn mix_slice(before: &str, after: &str, p: f32) -> String {
            let b: Vec<char> = before.chars().collect();
            let a: Vec<char> = after.chars().collect();
            let n = b.len().max(a.len());
            let k = ((p.clamp(0.0, 1.0)) * (n as f32)).ceil() as usize;
            let mut out = String::new();
            for i in 0..n {
                let ch = if i < k { a.get(i).copied().unwrap_or(' ') } else { b.get(i).copied().unwrap_or(' ') };
                out.push(ch);
            }
            out
        }

        let content_el: AnyElement = if let Some((p, _)) = sweep_progress {
            let after_full = self
                .external_playback
                .as_ref()
                .and_then(|pb| pb.sweep.as_ref())
                .filter(|sw| sw.line == buf_line)
                .map(|sw| sw.after.as_str())
                .unwrap_or("");
            let mut abs = bs.min(after_full.len());
            let mut abe = be.min(after_full.len()).max(abs);
            abs = Self::clamp_char_boundary_back(after_full, abs);
            abe = Self::clamp_char_boundary_fwd(after_full, abe);
            if abe < abs { abe = abs; }
            let after_slice = after_full.get(abs..abe).unwrap_or("");
            SharedString::from(mix_slice(&slice, after_slice, p)).into_any_element()
        } else if let Some(syn_line) = self.syntax.as_ref().and_then(|s| -> Option<&Vec<Span>> {
            if buf_line < s.start_line { None } else { s.lines.get(buf_line - s.start_line) }
        }) {
            let syn: &[Span] = syn_line;
            let mut parts: Vec<(String, gpui::Hsla)> = Vec::new();
            let mut pos = 0usize;
            for sp in syn {
                let mut ss = sp.range.start.max(bs).saturating_sub(bs);
                let mut ee = sp.range.end.min(be).saturating_sub(bs);
                if ss >= slice.len() { continue; }
                ss = Self::clamp_char_boundary_back(&slice, ss);
                ee = Self::clamp_char_boundary_fwd(&slice, ee);
                if ee <= ss { continue; }
                if ss > pos {
                    let a = Self::clamp_char_boundary_back(&slice, pos);
                    let b = ss;
                    if b > a { parts.push((slice.get(a..b).unwrap_or("").to_string(), ThemeColors::text_primary())); }
                }
                parts.push((slice.get(ss..ee).unwrap_or("").to_string(), color_for_kind(sp.kind)));
                pos = ee.max(pos);
            }
            if pos < slice.len() {
                let a = Self::clamp_char_boundary_back(&slice, pos);
                if a < slice.len() { parts.push((slice.get(a..).unwrap_or("").to_string(), ThemeColors::text_primary())); }
            }
            h_flex()
                .gap_0()
                .children(parts.into_iter().filter(|(t, _)| !t.is_empty()).map(|(t, c)| div().flex_none().text_color(c).child(SharedString::from(t)).into_any_element()).collect::<Vec<_>>())
                .into_any_element()
        } else {
            SharedString::from(slice).into_any_element()
        };

        h_flex()
            .w_full()
            .h(px(self.line_height))
            .bg(row_bg)
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, event, window, cx| {
                this.mouse_down_on_row(row_idx_down, event, window, cx)
            }))
            .on_mouse_move(cx.listener(move |this, event, window, cx| {
                this.mouse_move_on_row(row_idx_move, event, window, cx)
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(self.gutter_width))
                    .h_full()
                    .pr(Spacing::Base04.px())
                    .text_right()
                    .child(
                        h_flex()
                            .w_full()
                            .h_full()
                            .items_center()
                            .justify_end()
                            .gap(Spacing::Base04.px())
                            .child(div().w(px(3.0)).h_full().bg(mark_bar))
                            .child(Label::new(diag_sym).size(LabelSize::XSmall).color(diag_color))
                            .child(Label::new(mark_sym).size(LabelSize::Small).color(match mark {
                                Some(LineMarkKind::Added) => LabelColor::Success,
                                Some(LineMarkKind::Removed) => LabelColor::Muted,
                                Some(LineMarkKind::Modified) => LabelColor::Success,
                                None => LabelColor::Muted,
                            }))
                            .child(Label::new(line_num).size(LabelSize::Small).color(LabelColor::Muted))
                    )
            )
            .child(
                h_flex()
                    .flex_1()
                    .h_full()
                    .justify_between()
                    .child({
                        let base = div()
                            .relative()
                            .flex_1()
                            .h_full()
                            .overflow_x_hidden()
                            .text_size(px(UiMetrics::FONT_DEFAULT))
                            .text_color(ThemeColors::text_primary())
                            .child(content_el);
                        if let Some((p, c)) = sweep_progress {
                            base.child(
                                div()
                                    .absolute()
                                    .top(px(0.0))
                                    .left(px(0.0))
                                    .bottom(px(0.0))
                                    .w(relative(p.max(0.02)))
                                    .bg(c),
                            )
                        } else {
                            base
                        }
                    })
            )
            .into_any_element()
    }
}

fn color_for_kind(k: HighlightKind) -> gpui::Hsla {
    if k == kinds::COMMENT { return ThemeColors::text_muted(); }
    if k == kinds::KEYWORD { return ThemeColors::text_accent(); }
    if k == kinds::STRING { return ThemeColors::text_success(); }
    if k == kinds::TYPE { return ThemeColors::text_warning(); }
    if k == kinds::FUNCTION { return ThemeColors::text_primary(); }
    if k == kinds::CONSTANT { return ThemeColors::text_warning(); }
    if k == kinds::NUMBER { return ThemeColors::text_warning(); }
    if k == kinds::OPERATOR { return ThemeColors::text_secondary(); }
    if k == kinds::PUNCTUATION { return ThemeColors::text_secondary(); }
    if k == kinds::VARIABLE { return ThemeColors::text_primary(); }
    ThemeColors::text_primary()
}

impl Focusable for CodeEditor { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

struct CodeEditorInputElement { editor: Entity<CodeEditor> }
impl IntoElement for CodeEditorInputElement { type Element = Self; fn into_element(self) -> Self { self } }
impl Element for CodeEditorInputElement {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }
    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), _: &mut Window, _: &mut App) {}
    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.editor.clone()), cx);
        self.editor.update(cx, |ed, _| {
            ed.input_bounds = Some(bounds);
            let h: f32 = bounds.size.height.into();
            let line_h = ed.line_height.max(1.0);
            let new_visible = ((h / line_h).floor() as usize).max(6);
            if new_visible != ed.visible_lines {
                ed.visible_lines = new_visible;
                ed.rehighlight();
            }
        });
    }
}

impl EntityInputHandler for CodeEditor {
    fn text_for_range(&mut self, range_utf16: Range<usize>, actual_range: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range_to_utf16(&self.selected_range), reversed: self.selection_reversed })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) { self.marked_range = None; }

    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, new_text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16.as_ref().map(|r| self.range_from_utf16(r)).or(self.marked_range.clone()).unwrap_or(self.selected_range.clone());
        self.selected_range = range.clone();
        self.selection_reversed = false;
        self.marked_range = None;
        self.insert_text(new_text, window, cx);
    }

    fn replace_and_mark_text_in_range(&mut self, range_utf16: Option<Range<usize>>, new_text: &str, new_selected_range_utf16: Option<Range<usize>>, window: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16.as_ref().map(|r| self.range_from_utf16(r)).or(self.marked_range.clone()).unwrap_or(self.selected_range.clone());
        self.selected_range = range.clone();
        self.selection_reversed = false;
        self.insert_text(new_text, window, cx);
        if new_text.is_empty() { self.marked_range = None; return; }
        let mark = range.start..range.start + new_text.len();
        self.marked_range = Some(mark.clone());
        if let Some(r) = new_selected_range_utf16.as_ref().map(|r| self.range_from_utf16(r)) {
            self.selected_range = (range.start + r.start)..(range.start + r.end);
        } else {
            self.selected_range = (mark.end)..(mark.end);
        }
        cx.notify();
    }

    fn bounds_for_range(&mut self, range_utf16: Range<usize>, bounds: Bounds<Pixels>, window: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let r = self.range_from_utf16(&range_utf16);
        let off = r.start.min(self.content.len());
        let (line, col) = self.offset_to_line_col(off);
        let dr = self.display_map.buffer_to_display_row(line, col);
        let row = self.display_row_to_visual.get(dr).copied().unwrap_or(0);
        let cw = self.char_width_px(window);
        let cells = self.col_byte_to_col_cells(line, col) as f32;
        let x = bounds.left() + px(self.gutter_width + cells * cw);
        let y = bounds.top() + px((row as f32) * self.line_height - self.scroll_offset);
        Some(Bounds::new(point(x, y), size(px(1.0), px(self.line_height))))
    }

    fn character_index_for_point(&mut self, point: Point<Pixels>, window: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let bounds = self.input_bounds?;
        let (line, col) = self.point_to_line_col_byte(point, window, bounds)?;
        let off = self.offset_at(line, col);
        Some(self.offset_to_utf16(off))
    }
}

impl CodeEditor {
    fn char_width_px(&self, window: &Window) -> f32 {
        let ts = window.text_system();
        let fid = ts.resolve_font(&gpui::font(".ZedMono"));
        let adv: f32 = ts.ch_advance(fid, px(UiMetrics::FONT_DEFAULT))
            .ok()
            .map(|p| p.into())
            .unwrap_or(8.0f32);
        adv.max(1.0f32)
    }

    fn col_byte_to_col_cells(&self, line: usize, col_byte: usize) -> usize {
        let s = self.lines.get(line).map(|l| l.as_str()).unwrap_or("");
        let mut cells = 0usize;
        let mut b = 0usize;
        for ch in s.chars() {
            if b >= col_byte { break; }
            b += ch.len_utf8();
            cells += if ch == '\t' { 4 } else { 1 };
        }
        cells
    }

    fn cells_to_col_byte_in_row(&self, line: usize, r: crate::ai_workspace_gpui::ide::DisplayRow, mut cells: usize) -> usize {
        let s = self.lines.get(line).map(|l| l.as_str()).unwrap_or("");
        let seg = s.get(r.byte_start..r.byte_end).unwrap_or("");
        let mut b = r.byte_start;
        for ch in seg.chars() {
            let w = if ch == '\t' { 4 } else { 1 };
            if cells < w { break; }
            cells -= w;
            b += ch.len_utf8();
        }
        b.min(r.byte_end)
    }

    fn point_to_line_col_byte(&self, point: Point<Pixels>, window: &mut Window, bounds: Bounds<Pixels>) -> Option<(usize, usize)> {
        let local_x: f32 = (point.x - bounds.origin.x).into();
        let local_y: f32 = (point.y - bounds.origin.y).into();
        let y = (local_y + self.scroll_offset).max(0.0);
        let mut vr = (y / self.line_height).floor() as usize;
        vr = vr.min(self.visual_rows.len().saturating_sub(1));
        let (display_row, line) = match self.visual_rows.get(vr) {
            Some(VisualRow::Buffer { display_row }) => {
                let dr = self.display_map.row(*display_row);
                (*display_row, dr.buffer_line.min(self.lines.len().saturating_sub(1)))
            }
            Some(VisualRow::HunkHeader { anchor_line, .. }) => {
                let line = (*anchor_line).min(self.lines.len().saturating_sub(1));
                (self.display_map.buffer_to_display_row(line, 0), line)
            }
            Some(VisualRow::RemovedLine { anchor_line, .. }) => {
                let line = (*anchor_line).min(self.lines.len().saturating_sub(1));
                (self.display_map.buffer_to_display_row(line, 0), line)
            }
            None => (0, 0),
        };
        let dr = self.display_map.row(display_row);
        let cw = self.char_width_px(window);
        let x = (local_x - self.gutter_width).max(0.0);
        let cells = (x / cw).floor().max(0.0) as usize;
        let col = self.cells_to_col_byte_in_row(line, dr, cells);
        Some((line, col))
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let (mut u8o, mut u16) = (0usize, 0usize);
        for ch in self.content.chars() {
            if u16 >= offset { break; }
            u16 += ch.len_utf16();
            u8o += ch.len_utf8();
        }
        u8o
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let (mut u16o, mut u8) = (0usize, 0usize);
        for ch in self.content.chars() {
            if u8 >= offset { break; }
            u8 += ch.len_utf8();
            u16o += ch.len_utf16();
        }
        u16o
    }

    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> { self.offset_to_utf16(r.start)..self.offset_to_utf16(r.end) }
    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> { self.offset_from_utf16(r.start)..self.offset_from_utf16(r.end) }
}

impl Render for CodeEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.visual_rows.len().max(1);
        let first_visible = (self.scroll_offset / self.line_height).floor() as usize;
        let last_visible = (first_visible + self.visible_lines + 1).min(rows);
        let dr = self.display_map.buffer_to_display_row(self.cursor_line, self.cursor_col);
        let cursor_row = self.display_row_to_visual.get(dr).copied().unwrap_or(0);
        let completion_items = self.completion_items.clone();
        let hover_markdown = self.hover_markdown.clone();

        let lines_el: Vec<AnyElement> =
            (first_visible..last_visible).map(|i| self.render_visual_row(i, i == cursor_row, cx)).collect();

        let header = if let Some(path) = &self.path {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("untitled");
            let dirty_marker = if self.is_dirty { " •" } else { "" };
            let changes = self.external_line_marks.len();
            let has_changes = changes > 0;
            let has_tool_diff = self.tool_diff.is_some();
            let hunks = self.tool_diff.as_ref().map(|d| d.hunks.len()).unwrap_or(0);
            Some(
                h_flex()
                    .flex_none()
                    .w_full()
                    .h(px(24.0))
                    .px(Spacing::Base04.px())
                    .items_center()
                    .border_b_1()
                    .border_color(ThemeColors::border())
                    .bg(ThemeColors::bg_secondary())
                    .justify_between()
                    .child(Label::new(format!("{}{}", name, dirty_marker)).size(LabelSize::Small).color(LabelColor::Primary))
                    .child(
                        h_flex()
                            .items_center()
                            .gap(Spacing::Base04.px())
                            .children(has_tool_diff.then(|| Label::new(format!("{hunks} hunks")).size(LabelSize::XSmall).color(LabelColor::Muted).into_any_element()))
                            .children(has_tool_diff.then(|| {
                                let id = self.path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                                h_flex()
                                    .gap(Spacing::Base02.px())
                                    .child(Button::new(format!("tool-replay-{id}"), "Replay").style(ButtonStyle::Icon).on_click(cx.listener(|this, _, _, cx| this.tool_replay(cx))).into_any_element())
                                    .child(Button::new(format!("tool-applyall-{id}"), "Apply All").style(ButtonStyle::Tinted(TintColor::Success)).on_click(cx.listener(|this, _, _, cx| this.tool_apply_all(cx))).into_any_element())
                                    .child(Button::new(format!("tool-discardall-{id}"), "Discard All").style(ButtonStyle::Tinted(TintColor::Warning)).on_click(cx.listener(|this, _, _, cx| this.tool_discard_all(cx))).into_any_element())
                                    .into_any_element()
                            }))
                            .children((!has_tool_diff && has_changes).then(|| Label::new(format!("{} changes", changes)).size(LabelSize::XSmall).color(LabelColor::Muted).into_any_element()))
                            .children((!has_tool_diff && has_changes).then(|| {
                                let id = self.path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                                h_flex()
                                    .gap(Spacing::Base02.px())
                                    .child(Button::new(format!("chg-apply-{id}"), "Apply").style(ButtonStyle::Tinted(TintColor::Success)).on_click(cx.listener(|this, _, _, cx| { this.external_line_marks.clear(); cx.notify(); })).into_any_element())
                                    .child(Button::new(format!("chg-discard-{id}"), "Discard").style(ButtonStyle::Tinted(TintColor::Warning)).on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(p) = this.path.clone() {
                                            let _ = this.ui_tx.send(UiToHost::IdeUndo { path: p.clone() });
                                            let _ = this.ui_tx.send(UiToHost::IdeSaveFile { path: p });
                                        }
                                        this.external_line_marks.clear();
                                        cx.notify();
                                    })).into_any_element())
                                    .child(Button::new(format!("chg-prev-{id}"), "◀").style(ButtonStyle::Icon).on_click(cx.listener(|this, _, _, cx| this.jump_change(-1, cx))).into_any_element())
                                    .child(Button::new(format!("chg-next-{id}"), "▶").style(ButtonStyle::Icon).on_click(cx.listener(|this, _, _, cx| this.jump_change( 1, cx))).into_any_element())
                                    .into_any_element()
                            }))
                    )
            )
        } else { None };

        let scroll_offset = self.scroll_offset;
        let line_height = self.line_height;
        let visible_lines = self.visible_lines as f32;
        let total_height = rows as f32 * line_height;

        v_flex()
            .id("code-editor")
            .key_context("CodeEditor")
            .track_focus(&self.focus_handle)
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::complete))
            .on_action(cx.listener(Self::goto_definition))
            .on_action(cx.listener(Self::hover))
            .on_action(cx.listener(Self::line_up))
            .on_action(cx.listener(Self::line_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                this.ensure_editable(cx);
                match event.keystroke.key.as_str() {
                    "backspace" => this.delete_backward(cx),
                    "enter" => this.insert_newline(cx),
                    "tab" => this.insert_text("    ", window, cx),
                    "up" => {
                        let cur = this.display_map.buffer_to_display_row(this.cursor_line, this.cursor_col);
                        let (l, c) = this.display_map.display_to_buffer(cur.saturating_sub(1));
                        this.move_cursor(l, c, cx);
                    }
                    "down" => {
                        let cur = this.display_map.buffer_to_display_row(this.cursor_line, this.cursor_col);
                        let next = (cur + 1).min(this.display_map.row_count().saturating_sub(1));
                        let (l, c) = this.display_map.display_to_buffer(next);
                        this.move_cursor(l, c, cx);
                    }
                    "left" => {
                        let cur = this.offset_at(this.cursor_line, this.cursor_col);
                        let prev = this.previous_boundary(cur);
                        let (l, c) = this.offset_to_line_col(prev);
                        this.move_cursor(l, c, cx);
                    }
                    "right" => {
                        let cur = this.offset_at(this.cursor_line, this.cursor_col);
                        let next = this.next_boundary(cur);
                        let (l, c) = this.offset_to_line_col(next);
                        this.move_cursor(l, c, cx);
                    }
                    "home" => this.move_cursor(this.cursor_line, 0, cx),
                    "end" => this.move_cursor(this.cursor_line, usize::MAX, cx),
                    _ => {}
                }
            }))
            .on_scroll_wheel(cx.listener(move |this, event: &gpui::ScrollWheelEvent, window, cx| {
                let dy: f32 = event.delta.pixel_delta(px(line_height)).y.into();
                let max_scroll = (total_height - line_height * visible_lines.max(1.0)).max(0.0);
                this.scroll_offset = (this.scroll_offset - dy).max(0.0).min(max_scroll);
                this.rehighlight();
                cx.notify();
            }))
            .flex_1()
            .h_full()
            .bg(ThemeColors::bg_primary())
            .overflow_hidden()
            .children(header)
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h_full()
                            .child(
                                div()
                                    .absolute()
                                    .top(px(0.0))
                                    .right(px(0.0))
                                    .left(px(0.0))
                                    .bottom(px(0.0))
                                    .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                        this.mouse_down_at_point(event.position, event.click_count, window, cx)
                                    }))
                                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                                        this.mouse_move_at_point(event.position, window, cx)
                                    }))
                                    .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                                    .child(CodeEditorInputElement { editor: cx.entity() }),
                            )
                            .children((!completion_items.is_empty()).then(|| {
                                v_flex()
                                    .absolute()
                                    .top(px(30.0))
                                    .right(px(12.0))
                                    .min_w(px(220.0))
                                    .max_w(px(420.0))
                                    .p(Spacing::Base04.px())
                                    .bg(ThemeColors::bg_elevated())
                                    .border_1()
                                    .border_color(ThemeColors::border())
                                    .rounded_sm()
                                    .gap(Spacing::Base02.px())
                                    .children(completion_items.iter().take(12).map(|it| Label::new(it.clone()).size(LabelSize::Small).color(LabelColor::Primary).into_any_element()).collect::<Vec<_>>())
                                    .into_any_element()
                            }))
                            .children(hover_markdown.as_ref().map(|md| {
                                v_flex()
                                    .absolute()
                                    .bottom(px(12.0))
                                    .right(px(12.0))
                                    .min_w(px(240.0))
                                    .max_w(px(520.0))
                                    .p(Spacing::Base04.px())
                                    .bg(ThemeColors::bg_elevated())
                                    .border_1()
                                    .border_color(ThemeColors::border())
                                    .rounded_sm()
                                    .child(Label::new(md.clone()).size(LabelSize::Small).color(LabelColor::Secondary))
                                    .into_any_element()
                            }))
                            .child(v_flex().w_full().gap_0().children(lines_el))
                    )
            )
    }
}

impl CodeEditor {
    fn render_visual_row(&self, visual_row: usize, is_current: bool, cx: &mut Context<Self>) -> AnyElement {
        match self.visual_rows.get(visual_row).cloned().unwrap_or(VisualRow::Buffer { display_row: 0 }) {
            VisualRow::Buffer { display_row } => self.render_row(display_row, is_current, cx),
            VisualRow::HunkHeader { hunk_idx, text, .. } => {
                let id = self.path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                let hunk_idx_apply = hunk_idx;
                let hunk_idx_discard = hunk_idx;
                h_flex()
                    .w_full()
                    .h(px(self.line_height))
                    .bg(ThemeColors::bg_secondary())
                    .child(
                        div()
                            .flex_none()
                            .w(px(self.gutter_width))
                            .h_full()
                            .pr(Spacing::Base04.px())
                            .child(h_flex().w_full().h_full().items_center().justify_end().child(div().w(px(3.0)).h_full().bg(ThemeColors::text_accent()))),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .h_full()
                            .justify_between()
                            .child(div().flex_1().h_full().overflow_x_hidden().text_size(px(UiMetrics::FONT_DEFAULT)).text_color(ThemeColors::text_accent()).child(SharedString::from(text).into_any_element()))
                            .child(
                                h_flex()
                                    .flex_none()
                                    .items_center()
                                    .gap(Spacing::Base02.px())
                                    .pr(Spacing::Base04.px())
                                    .child(Button::new(format!("tool-hunk-apply-{id}-{hunk_idx_apply}"), "Apply").style(ButtonStyle::Tinted(TintColor::Success)).on_click(cx.listener(move |this, _, _, cx| this.tool_apply_hunk(hunk_idx_apply, cx))).into_any_element())
                                    .child(Button::new(format!("tool-hunk-discard-{id}-{hunk_idx_discard}"), "Discard").style(ButtonStyle::Tinted(TintColor::Warning)).on_click(cx.listener(move |this, _, _, cx| this.tool_discard_hunk(hunk_idx_discard, cx))).into_any_element()),
                            ),
                    )
                    .into_any_element()
            }
            VisualRow::RemovedLine { old_line, text, .. } => {
                let line_num = old_line.map(|n| n.to_string()).unwrap_or_default();
                h_flex()
                    .w_full()
                    .h(px(self.line_height))
                    .bg(ThemeColors::diff_removed_bg())
                    .child(
                        div()
                            .flex_none()
                            .w(px(self.gutter_width))
                            .h_full()
                            .pr(Spacing::Base04.px())
                            .text_right()
                            .child(
                                h_flex()
                                    .w_full()
                                    .h_full()
                                    .items_center()
                                    .justify_end()
                                    .gap(Spacing::Base04.px())
                                    .child(div().w(px(3.0)).h_full().bg(ThemeColors::diff_removed_text()))
                                    .child(Label::new(" ").size(LabelSize::XSmall).color(LabelColor::Muted))
                                    .child(Label::new("-").size(LabelSize::Small).color(LabelColor::Muted))
                                    .child(Label::new(line_num).size(LabelSize::Small).color(LabelColor::Muted)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .overflow_x_hidden()
                            .text_size(px(UiMetrics::FONT_DEFAULT))
                            .text_color(ThemeColors::text_primary())
                            .child(SharedString::from(text).into_any_element()),
                    )
                    .into_any_element()
            }
        }
    }
}
