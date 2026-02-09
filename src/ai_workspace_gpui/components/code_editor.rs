//! Code Editor: Virtualized multi-line editor with line numbers (Zed-isomorphic)

use gpui::{actions, AnyElement, App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render, SharedString, Size, Style, TextRun, UTF16Selection, Window, div, fill, point, prelude::*, px, relative, size};
use crossbeam_channel::Sender;
use std::collections::VecDeque;
use std::ops::Range;
use std::path::PathBuf;
use crate::ai_workspace_gpui::{ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Spacing}, protocol::{TextEdit, UiToHost}};

actions!(code_editor, [Undo, Redo, LineUp, LineDown, PageUp, PageDown, GotoLine]);

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
    cursor_line: usize,
    cursor_col: usize,
    selection: Option<Selection>,
    is_selecting: bool,
    selection_anchor_line: Option<usize>,
    scroll_offset: f32,
    visible_lines: usize,
    line_height: f32,
    gutter_width: f32,
    ui_tx: Sender<UiToHost>,

    pending_edits: VecDeque<Vec<TextEdit>>,
}

#[derive(Clone, Copy)]
struct Selection { start_line: usize, start_col: usize, end_line: usize, end_col: usize }

impl CodeEditor {
    pub fn new(ui_tx: Sender<UiToHost>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            path: None,
            content: String::new(),
            version: 0,
            is_dirty: false,
            lines: vec![String::new()],
            cursor_line: 0,
            cursor_col: 0,
            selection: None,
            is_selecting: false,
            selection_anchor_line: None,
            scroll_offset: 0.0,
            visible_lines: 30,
            line_height: 20.0,
            gutter_width: 50.0,
            ui_tx,

            pending_edits: VecDeque::new(),
        }
    }

    pub fn set_content(&mut self, path: PathBuf, content: String, version: u64, cx: &mut Context<Self>) {
        self.path = Some(path);
        self.lines = content.lines().map(|l| l.to_string()).collect();
        if self.lines.is_empty() { self.lines.push(String::new()); }
        self.content = content;
        self.version = version;
        self.is_dirty = false;
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.selection = None;
        self.is_selecting = false;
        self.selection_anchor_line = None;
        self.scroll_offset = 0.0;
        self.pending_edits.clear();
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
        self.version = 0;
        self.is_dirty = false;
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.selection = None;
        self.is_selecting = false;
        self.selection_anchor_line = None;
        self.pending_edits.clear();
        cx.notify();
    }

    pub fn set_dirty(&mut self, is_dirty: bool, cx: &mut Context<Self>) {
        self.is_dirty = is_dirty;
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
            Self::apply_edits_to_string(&mut self.content, &edits);
            self.lines = self.content.lines().map(|l| l.to_string()).collect();
            if self.lines.is_empty() {
                self.lines.push(String::new());
            }
            self.cursor_line = self.cursor_line.min(self.lines.len().saturating_sub(1));
            self.cursor_col = self
                .cursor_col
                .min(self.lines.get(self.cursor_line).map(|l| l.len()).unwrap_or(0));
        }

        self.version = version;
        cx.notify();
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
    pub fn line_count(&self) -> usize { self.lines.len() }

    // ─────────────────────────────────────────────────────────────────────────
    // Navigation
    // ─────────────────────────────────────────────────────────────────────────

    fn move_cursor(&mut self, line: usize, col: usize, cx: &mut Context<Self>) {
        self.cursor_line = line.min(self.lines.len().saturating_sub(1));
        self.cursor_col = col.min(self.lines.get(self.cursor_line).map(|l| l.len()).unwrap_or(0));
        self.selection = None;
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
            self.selection = None;
            let _ = self.ui_tx.send(UiToHost::IdeSelectionCleared { path });
            cx.notify();
            return;
        }
        self.selection = Some(Selection {
            start_line: start,
            start_col: 0,
            end_line: end,
            end_col: self.lines.get(end).map(|l| l.len()).unwrap_or(0),
        });
        let _ = self.ui_tx.send(UiToHost::IdeSelectionChanged {
            path,
            start_line: start as u32,
            end_line: end as u32,
        });
        cx.notify();
    }

    fn mouse_down_on_line(
        &mut self,
        line_idx: usize,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.is_focusing() {
            return;
        }
        self.is_selecting = true;
        self.selection_anchor_line = Some(line_idx);
        self.cursor_line = line_idx.min(self.lines.len().saturating_sub(1));
        self.cursor_col = 0;
        self.selection = None;
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

    fn mouse_move_on_line(
        &mut self,
        line_idx: usize,
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
        self.set_line_selection(anchor, line_idx, cx);
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
        let cursor_y = self.cursor_line as f32;
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
        let Some(path) = self.path.clone() else { return; };
        let offset = self.offset_at(self.cursor_line, self.cursor_col);
        let edit = TextEdit { start_offset: offset, end_offset: offset, new_text: ch.to_string() };
        let _ = self.ui_tx.send(UiToHost::IdeEditFile { path, version: self.version, edits: vec![edit] });
        self.pending_edits.push_back(vec![TextEdit { start_offset: offset, end_offset: offset, new_text: ch.to_string() }]);
        // Optimistically update local state
        if let Some(line) = self.lines.get_mut(self.cursor_line) {
            line.insert(self.cursor_col, ch);
            self.cursor_col += 1;
        }
        self.is_dirty = true;
        self.version = self.version.saturating_add(1);
        self.content = self.lines.join("\n");
        cx.notify();
    }

    fn delete_backward(&mut self, cx: &mut Context<Self>) {
        if self.cursor_col == 0 && self.cursor_line == 0 { return; }
        let Some(path) = self.path.clone() else { return; };
        let offset = self.offset_at(self.cursor_line, self.cursor_col);
        let start = if self.cursor_col > 0 { offset - 1 } else { offset - 1 }; // newline
        let edit = TextEdit { start_offset: start, end_offset: offset, new_text: String::new() };
        let _ = self.ui_tx.send(UiToHost::IdeEditFile { path, version: self.version, edits: vec![edit] });
        self.pending_edits.push_back(vec![TextEdit { start_offset: start, end_offset: offset, new_text: String::new() }]);
        // Optimistically update
        if self.cursor_col > 0 {
            if let Some(line) = self.lines.get_mut(self.cursor_line) {
                line.remove(self.cursor_col - 1);
                self.cursor_col -= 1;
            }
        } else if self.cursor_line > 0 {
            let current = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            self.cursor_col = self.lines[self.cursor_line].len();
            self.lines[self.cursor_line].push_str(&current);
        }
        self.is_dirty = true;
        self.version = self.version.saturating_add(1);
        self.content = self.lines.join("\n");
        cx.notify();
    }

    fn insert_newline(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else { return; };
        let offset = self.offset_at(self.cursor_line, self.cursor_col);
        let edit = TextEdit { start_offset: offset, end_offset: offset, new_text: "\n".to_string() };
        let _ = self.ui_tx.send(UiToHost::IdeEditFile { path, version: self.version, edits: vec![edit] });
        self.pending_edits.push_back(vec![TextEdit { start_offset: offset, end_offset: offset, new_text: "\n".to_string() }]);
        // Optimistically update
        if let Some(line) = self.lines.get_mut(self.cursor_line) {
            let rest = line.split_off(self.cursor_col);
            self.lines.insert(self.cursor_line + 1, rest);
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
        self.is_dirty = true;
        self.version = self.version.saturating_add(1);
        self.content = self.lines.join("\n");
        self.ensure_cursor_visible();
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

    // ─────────────────────────────────────────────────────────────────────────
    // Rendering
    // ─────────────────────────────────────────────────────────────────────────

    fn render_line(&self, line_idx: usize, is_current: bool, cx: &mut Context<Self>) -> AnyElement {
        let line_num = (line_idx + 1).to_string();
        let line_text = self.lines.get(line_idx).cloned().unwrap_or_default();
        let line_idx_down = line_idx;
        let line_idx_move = line_idx;

        h_flex()
            .w_full()
            .h(px(self.line_height))
            .when(is_current, |d| d.bg(ThemeColors::bg_selected()))
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, event, window, cx| {
                this.mouse_down_on_line(line_idx_down, event, window, cx)
            }))
            .on_mouse_move(cx.listener(move |this, event, window, cx| {
                this.mouse_move_on_line(line_idx_move, event, window, cx)
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(self.gutter_width))
                    .h_full()
                    .pr(Spacing::Base04.px())
                    .text_right()
                    .child(Label::new(line_num).size(LabelSize::Small).color(LabelColor::Muted))
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .overflow_x_hidden()
                    .text_sm()
                    .text_color(ThemeColors::text_primary())
                    .child(SharedString::from(line_text))
            )
            .into_any_element()
    }
}

impl Focusable for CodeEditor { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for CodeEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let first_visible = (self.scroll_offset / self.line_height).floor() as usize;
        let last_visible = (first_visible + self.visible_lines + 1).min(self.lines.len());

        let lines_el: Vec<AnyElement> = (first_visible..last_visible)
            .map(|i| self.render_line(i, i == self.cursor_line, cx))
            .collect();

        let header = if let Some(path) = &self.path {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("untitled");
            let dirty_marker = if self.is_dirty { " •" } else { "" };
            Some(
                h_flex()
                    .flex_none()
                    .w_full()
                    .h(px(28.0))
                    .px(Spacing::Base06.px())
                    .items_center()
                    .border_b_1()
                    .border_color(ThemeColors::border())
                    .bg(ThemeColors::bg_secondary())
                    .child(Label::new(format!("{}{}", name, dirty_marker)).size(LabelSize::Small).color(LabelColor::Primary))
            )
        } else { None };

        let scroll_offset = self.scroll_offset;
        let line_height = self.line_height;
        let total_height = self.lines.len() as f32 * line_height;

        v_flex()
            .id("code-editor")
            .key_context("CodeEditor")
            .track_focus(&self.focus_handle)
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::line_up))
            .on_action(cx.listener(Self::line_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                if let Some(ch) = event.keystroke.key.chars().next() {
                    if ch.is_ascii_graphic() || ch == ' ' {
                        this.insert_char(ch, cx);
                    }
                }
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "backspace" => this.delete_backward(cx),
                    "enter" => this.insert_newline(cx),
                    "up" => this.move_cursor(this.cursor_line.saturating_sub(1), this.cursor_col, cx),
                    "down" => this.move_cursor(this.cursor_line + 1, this.cursor_col, cx),
                    "left" => this.move_cursor(this.cursor_line, this.cursor_col.saturating_sub(1), cx),
                    "right" => this.move_cursor(this.cursor_line, this.cursor_col + 1, cx),
                    "home" => this.move_cursor(this.cursor_line, 0, cx),
                    "end" => this.move_cursor(this.cursor_line, usize::MAX, cx),
                    _ => {}
                }
            }))
            .on_scroll_wheel(cx.listener(move |this, event: &gpui::ScrollWheelEvent, window, cx| {
                let dy: f32 = event.delta.pixel_delta(px(line_height)).y.into();
                this.scroll_offset = (this.scroll_offset - dy).max(0.0).min(total_height - line_height * 10.0);
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
                        v_flex()
                            .w_full()
                            .gap_0()
                            .children(lines_el)
                    )
            )
    }
}
