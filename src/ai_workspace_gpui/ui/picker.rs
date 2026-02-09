//! Generic Picker component (Zed-style searchable list picker).
use gpui::{actions, AnyElement, App, Context, DismissEvent, ElementId, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, prelude::*, px};
use super::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, TextInput, Spacing};

actions!(picker, [SelectNext, SelectPrev, Confirm, Cancel]);

// ─────────────────────────────────────────────────────────────────────────────
// PickerDelegate trait
// ─────────────────────────────────────────────────────────────────────────────

pub trait PickerDelegate: Sized + 'static {
    type ListItem: IntoElement;
    fn match_count(&self) -> usize;
    fn selected_index(&self) -> usize;
    fn set_selected_index(&mut self, ix: usize, cx: &mut Context<Picker<Self>>);
    fn placeholder_text(&self, cx: &App) -> SharedString;
    fn update_matches(&mut self, query: &str, cx: &mut Context<Picker<Self>>);
    fn confirm(&mut self, secondary: bool, window: &mut Window, cx: &mut Context<Picker<Self>>);
    fn dismissed(&mut self, window: &mut Window, cx: &mut Context<Picker<Self>>);
    fn render_match(&self, ix: usize, selected: bool, cx: &mut Context<Picker<Self>>) -> Self::ListItem;
    fn separators_after(&self, ix: usize) -> bool { false }
}

// ─────────────────────────────────────────────────────────────────────────────
// Picker Entity
// ─────────────────────────────────────────────────────────────────────────────

pub struct Picker<D: PickerDelegate> {
    pub delegate: D,
    query_editor: Entity<TextInput>,
    focus_handle: FocusHandle,
    max_height: Option<f32>,
    width: Option<f32>,
}

impl<D: PickerDelegate> EventEmitter<DismissEvent> for Picker<D> {}

impl<D: PickerDelegate> Picker<D> {
    pub fn new(delegate: D, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = delegate.placeholder_text(cx);
        let query_editor = cx.new(|cx| TextInput::new(cx, placeholder));
        let focus_handle = cx.focus_handle();
        Self { delegate, query_editor, focus_handle, max_height: Some(300.0), width: Some(280.0) }
    }

    pub fn max_height(mut self, h: f32) -> Self { self.max_height = Some(h); self }
    pub fn width(mut self, w: f32) -> Self { self.width = Some(w); self }

    pub fn query(&self, cx: &App) -> String { self.query_editor.read(cx).text().to_string() }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.delegate.match_count();
        if count > 0 {
            let ix = (self.delegate.selected_index() + 1) % count;
            self.delegate.set_selected_index(ix, cx);
        }
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.delegate.match_count();
        if count > 0 {
            let ix = if self.delegate.selected_index() == 0 { count - 1 } else { self.delegate.selected_index() - 1 };
            self.delegate.set_selected_index(ix, cx);
        }
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        self.delegate.confirm(false, window, cx);
        cx.emit(DismissEvent);
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.delegate.dismissed(window, cx);
        cx.emit(DismissEvent);
    }

    fn on_query_change(&mut self, query: &str, cx: &mut Context<Self>) {
        self.delegate.update_matches(query, cx);
        cx.notify();
    }
}

impl<D: PickerDelegate> Focusable for Picker<D> {
    fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() }
}

impl<D: PickerDelegate> Render for Picker<D> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.query(cx);
        self.delegate.update_matches(&query, cx);

        let count = self.delegate.match_count();
        let selected = self.delegate.selected_index();

        let items: Vec<AnyElement> = (0..count).flat_map(|ix| {
            let mut elements = vec![
                div()
                    .id(ElementId::NamedInteger("picker-item".into(), ix as u64))
                    .w_full()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.delegate.set_selected_index(ix, cx);
                        this.delegate.confirm(false, window, cx);
                        cx.emit(DismissEvent);
                    }))
                    .on_hover(cx.listener(move |this, hovered, _, cx| {
                        if *hovered { this.delegate.set_selected_index(ix, cx); }
                    }))
                    .child(self.delegate.render_match(ix, ix == selected, cx))
                    .into_any_element()
            ];
            if self.delegate.separators_after(ix) {
                elements.push(div().w_full().h(px(1.0)).my(Spacing::Base02.px()).bg(ThemeColors::border()).into_any_element());
            }
            elements
        }).collect();

        v_flex()
            .id("picker")
            .key_context("Picker")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .when_some(self.width, |d, w| d.w(px(w)))
            .bg(ThemeColors::bg_elevated())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_md()
            .shadow_lg()
            .overflow_hidden()
            .child(
                div()
                    .p(Spacing::Base06.px())
                    .border_b_1()
                    .border_color(ThemeColors::border())
                    .child(self.query_editor.clone())
            )
            .child(
                {
                    let base = div()
                        .id("picker-scroll")
                        .flex_1()
                        .when_some(self.max_height, |d, h| d.max_h(px(h)))
                        .overflow_y_scroll()
                        .p(Spacing::Base04.px());
                    let base = if items.is_empty() {
                        base.child(div().p(Spacing::Base08.px()).child(Label::new("No matches").color(LabelColor::Muted)))
                    } else {
                        base
                    };
                    base.children(items)
                }
            )
    }
}
