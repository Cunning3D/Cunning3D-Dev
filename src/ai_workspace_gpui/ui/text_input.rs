//! Full-featured TextInput component (IME, selection, copy/paste, mouse support).
use gpui::{
    actions, App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, Render, ShapedLine, SharedString, Size, Style,
    TextRun, UTF16Selection, UnderlineStyle, Window, div, fill, point, prelude::*, px, relative, size,
};
use std::ops::Range;
use unicode_segmentation::*;

actions!(text_input, [Backspace, Delete, Left, Right, SelectLeft, SelectRight, SelectAll, Home, End, Paste, Cut, Copy]);

// ─────────────────────────────────────────────────────────────────────────────
// TextInput Entity
// ─────────────────────────────────────────────────────────────────────────────

pub struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    multiline: bool,
    on_change: Option<Box<dyn Fn(&str, &mut Window, &mut App) + 'static>>,
    on_submit: Option<Box<dyn Fn(&str, &mut Window, &mut App) + 'static>>,
}

impl TextInput {
    pub fn new(cx: &mut Context<Self>, placeholder: impl Into<SharedString>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: "".into(),
            placeholder: placeholder.into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            multiline: false,
            on_change: None,
            on_submit: None,
        }
    }

    pub fn multiline(mut self, multiline: bool) -> Self { self.multiline = multiline; self }
    pub fn on_change(mut self, f: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self { self.on_change = Some(Box::new(f)); self }
    pub fn on_submit(mut self, f: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self { self.on_submit = Some(Box::new(f)); self }

    pub fn text(&self) -> &str { &self.content }
    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = text.into();
        self.selected_range = self.content.len()..self.content.len();
        self.marked_range = None;
        cx.notify();
    }
    pub fn clear(&mut self, cx: &mut Context<Self>) { self.set_text("", cx); }
    pub fn is_empty(&self) -> bool { self.content.is_empty() }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() { self.move_to(self.previous_boundary(self.cursor_offset()), cx); }
        else { self.move_to(self.selected_range.start, cx); }
    }
    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() { self.move_to(self.next_boundary(self.selected_range.end), cx); }
        else { self.move_to(self.selected_range.end, cx); }
    }
    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) { self.select_to(self.previous_boundary(self.cursor_offset()), cx); }
    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) { self.select_to(self.next_boundary(self.cursor_offset()), cx); }
    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) { self.move_to(0, cx); self.select_to(self.content.len(), cx); }
    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) { self.move_to(0, cx); }
    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) { self.move_to(self.content.len(), cx); }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() { self.select_to(self.previous_boundary(self.cursor_offset()), cx); }
        self.replace_text_in_range(None, "", window, cx);
    }
    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() { self.select_to(self.next_boundary(self.cursor_offset()), cx); }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = if self.multiline { text } else { text.replace('\n', " ") };
            self.replace_text_in_range(None, &text, window, cx);
        }
    }
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected_range.clone()].to_string()));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected_range.clone()].to_string()));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        if event.modifiers.shift { self.select_to(self.index_for_mouse_position(event.position), cx); }
        else { self.move_to(self.index_for_mouse_position(event.position), cx); }
    }
    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) { self.is_selecting = false; }
    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting { self.select_to(self.index_for_mouse_position(event.position), cx); }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) { self.selected_range = offset..offset; cx.notify(); }
    fn cursor_offset(&self) -> usize { if self.selection_reversed { self.selected_range.start } else { self.selected_range.end } }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() { return 0; }
        let Some(bounds) = self.last_bounds.as_ref() else { return 0; };
        if position.y < bounds.top() { return 0; }
        if position.y > bounds.bottom() { return self.content.len(); }
        // For multiline, calculate which line was clicked and the offset within that line
        let line_height = bounds.size.height / self.content.lines().count().max(1) as f32;
        let line_idx = ((position.y - bounds.top()) / line_height).floor() as usize;
        let lines: Vec<&str> = self.content.lines().collect();
        let lines: Vec<&str> = if lines.is_empty() { vec![""] } else { lines };
        let target_line = lines.get(line_idx).copied().unwrap_or("");
        let mut offset = 0usize;
        for (i, l) in lines.iter().enumerate() {
            if i == line_idx { break; }
            offset += l.len() + 1; // +1 for newline
        }
        // Use the stored layout for x position if available
        if let Some(ref line_layout) = self.last_layout {
            offset + line_layout.closest_index_for_x(position.x - bounds.left()).min(target_line.len())
        } else { offset }
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed { self.selected_range.start = offset; } else { self.selected_range.end = offset; }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let (mut utf8_offset, mut utf16_count) = (0, 0);
        for ch in self.content.chars() { if utf16_count >= offset { break; } utf16_count += ch.len_utf16(); utf8_offset += ch.len_utf8(); }
        utf8_offset
    }
    fn offset_to_utf16(&self, offset: usize) -> usize {
        let (mut utf16_offset, mut utf8_count) = (0, 0);
        for ch in self.content.chars() { if utf8_count >= offset { break; } utf8_count += ch.len_utf8(); utf16_offset += ch.len_utf16(); }
        utf16_offset
    }
    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> { self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end) }
    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> { self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end) }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content.as_ref().grapheme_indices(true).rev().find_map(|(idx, _)| (idx < offset).then_some(idx)).unwrap_or(0)
    }
    fn next_boundary(&self, offset: usize) -> usize {
        self.content.as_ref().grapheme_indices(true).find_map(|(idx, _)| (idx > offset).then_some(idx)).unwrap_or(self.content.len())
    }
}

impl EntityInputHandler for TextInput {
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
        self.content = (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..]).into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        if let Some(ref cb) = self.on_change { cb(&self.content, window, cx); }
        cx.notify();
    }
    fn replace_and_mark_text_in_range(&mut self, range_utf16: Option<Range<usize>>, new_text: &str, new_selected_range_utf16: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16.as_ref().map(|r| self.range_from_utf16(r)).or(self.marked_range.clone()).unwrap_or(self.selected_range.clone());
        self.content = (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..]).into();
        self.marked_range = if new_text.is_empty() { None } else { Some(range.start..range.start + new_text.len()) };
        self.selected_range = new_selected_range_utf16.as_ref().map(|r| self.range_from_utf16(r)).map(|r| r.start + range.start..r.end + range.end).unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        cx.notify();
    }
    fn bounds_for_range(&mut self, range_utf16: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(point(bounds.left() + layout.x_for_index(range.start), bounds.top()), point(bounds.left() + layout.x_for_index(range.end), bounds.bottom())))
    }
    fn character_index_for_point(&mut self, pt: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let line_pt = self.last_bounds?.localize(&pt)?;
        let layout = self.last_layout.as_ref()?;
        Some(self.offset_to_utf16(layout.index_for_x(pt.x - line_pt.x)?))
    }
}

impl Focusable for TextInput { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use super::ThemeColors;
        div()
            .id("text-input")
            .key_context("TextInput")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .size_full()
            .min_h(px(24.0))
            .child(TextInputElement { input: cx.entity() })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TextInputElement (low-level rendering)
// ─────────────────────────────────────────────────────────────────────────────

struct TextInputElement { input: Entity<TextInput> }
struct PrepaintState { lines: Vec<ShapedLine>, cursor: Option<PaintQuad>, selection: Option<PaintQuad>, line_height: Pixels }

impl IntoElement for TextInputElement { type Element = Self; fn into_element(self) -> Self { self } }

impl Element for TextInputElement {
    type RequestLayoutState = usize; // line count
    type PrepaintState = PrepaintState;
    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, usize) {
        let input = self.input.read(cx);
        let line_count = if input.multiline { input.content.lines().count().max(1) } else { 1 };
        let line_height = window.line_height();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (line_height * line_count as f32).into();
        (window.request_layout(style, [], cx), line_count)
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, line_count: &mut usize, window: &mut Window, cx: &mut App) -> PrepaintState {
        use super::ThemeColors;
        let input = self.input.read(cx);
        let content = input.content.clone();
        let selected_range = input.selected_range.clone();
        let cursor_offset = input.cursor_offset();
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let (display_text, text_color) = if content.is_empty() { (input.placeholder.to_string(), ThemeColors::text_muted()) } else { (content.to_string(), style.color) };
        
        // Split into lines for multiline mode
        let text_lines: Vec<&str> = if input.multiline { display_text.lines().collect() } else { vec![&display_text] };
        let text_lines: Vec<&str> = if text_lines.is_empty() { vec![""] } else { text_lines };
        
        let mut shaped_lines = Vec::new();
        let mut cursor_quad = None;
        let mut selection_quad = None;
        let mut char_offset = 0usize;
        
        for (line_idx, line_text) in text_lines.iter().enumerate() {
            let run = TextRun { len: line_text.len(), font: style.font(), color: text_color, background_color: None, underline: None, strikethrough: None };
            let shaped = window.text_system().shape_line(SharedString::from(line_text.to_string()), font_size, &[run], None);
            let line_y = bounds.top() + line_height * line_idx as f32;
            let line_start = char_offset;
            let line_end = char_offset + line_text.len();
            
            // Cursor on this line
            if cursor_offset >= line_start && cursor_offset <= line_end && selected_range.is_empty() {
                let x = shaped.x_for_index(cursor_offset - line_start);
                cursor_quad = Some(fill(Bounds::new(point(bounds.left() + x, line_y), size(px(2.), line_height)), ThemeColors::text_accent()));
            }
            
            // Selection on this line (simplified: only show selection on cursor's line)
            if !selected_range.is_empty() && selected_range.start < line_end && selected_range.end > line_start {
                let sel_start = selected_range.start.saturating_sub(line_start).min(line_text.len());
                let sel_end = (selected_range.end - line_start).min(line_text.len());
                let x1 = shaped.x_for_index(sel_start);
                let x2 = shaped.x_for_index(sel_end);
                selection_quad = Some(fill(Bounds::from_corners(point(bounds.left() + x1, line_y), point(bounds.left() + x2, line_y + line_height)), ThemeColors::bg_selected()));
            }
            
            shaped_lines.push(shaped);
            char_offset = line_end + 1; // +1 for newline
        }
        
        PrepaintState { lines: shaped_lines, cursor: cursor_quad, selection: selection_quad, line_height }
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut usize, prepaint: &mut PrepaintState, window: &mut Window, cx: &mut App) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.input.clone()), cx);
        if let Some(sel) = prepaint.selection.take() { window.paint_quad(sel); }
        for (i, line) in prepaint.lines.iter().enumerate() {
            let y = bounds.top() + prepaint.line_height * i as f32;
            line.paint(point(bounds.left(), y), prepaint.line_height, gpui::TextAlign::Left, None, window, cx).ok();
        }
        if focus_handle.is_focused(window) { if let Some(cur) = prepaint.cursor.take() { window.paint_quad(cur); } }
        // Store first line for IME compatibility
        self.input.update(cx, |input, _| { 
            input.last_layout = prepaint.lines.first().cloned(); 
            input.last_bounds = Some(bounds); 
        });
    }
}
