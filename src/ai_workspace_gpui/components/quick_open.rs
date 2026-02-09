//! QuickOpen: File search picker (Ctrl+P) - Zed-isomorphic

use gpui::{AnyElement, App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render, SharedString, Window, div, prelude::*, px};
use crossbeam_channel::Sender;
use std::path::PathBuf;
use crate::ai_workspace_gpui::{ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Picker, PickerDelegate, Spacing}, protocol::{QuickOpenItem, UiToHost}};

// ─────────────────────────────────────────────────────────────────────────────
// QuickOpenDelegate
// ─────────────────────────────────────────────────────────────────────────────

pub struct QuickOpenDelegate {
    items: Vec<QuickOpenItem>,
    selected_ix: usize,
    ui_tx: Sender<UiToHost>,
}

impl QuickOpenDelegate {
    pub fn new(ui_tx: Sender<UiToHost>) -> Self {
        Self { items: vec![], selected_ix: 0, ui_tx }
    }

    pub fn set_items(&mut self, items: Vec<QuickOpenItem>) {
        self.items = items;
        self.selected_ix = 0;
    }
}

impl PickerDelegate for QuickOpenDelegate {
    type ListItem = AnyElement;

    fn match_count(&self) -> usize { self.items.len() }
    fn selected_index(&self) -> usize { self.selected_ix }
    fn set_selected_index(&mut self, ix: usize, cx: &mut Context<Picker<Self>>) { self.selected_ix = ix; cx.notify(); }
    fn placeholder_text(&self, _cx: &App) -> SharedString { "Type to search files...".into() }

    fn update_matches(&mut self, query: &str, cx: &mut Context<Picker<Self>>) {
        let _ = self.ui_tx.send(UiToHost::IdeQuickOpenQuery { query: query.to_string() });
    }

    fn confirm(&mut self, _secondary: bool, _window: &mut Window, cx: &mut Context<Picker<Self>>) {
        if let Some(item) = self.items.get(self.selected_ix) {
            let _ = self.ui_tx.send(UiToHost::IdeOpenFile { path: item.path.clone() });
        }
    }

    fn dismissed(&mut self, _window: &mut Window, _cx: &mut Context<Picker<Self>>) {}

    fn render_match(&self, ix: usize, selected: bool, _cx: &mut Context<Picker<Self>>) -> Self::ListItem {
        let Some(item) = self.items.get(ix) else {
            return div().h(px(24.0)).into_any_element();
        };

        h_flex()
            .w_full()
            .px(Spacing::Base06.px())
            .py(Spacing::Base04.px())
            .gap(Spacing::Base06.px())
            .rounded_sm()
            .cursor_pointer()
            .when(selected, |d| d.bg(ThemeColors::bg_selected()))
            .when(!selected, |d| d.hover(|d| d.bg(ThemeColors::bg_hover())))
            .child(Label::new(&item.name).size(LabelSize::Small).color(if selected { LabelColor::Primary } else { LabelColor::Secondary }))
            .child(Label::new(item.path.parent().and_then(|p| p.to_str()).map(|s| s.to_string()).unwrap_or_default()).size(LabelSize::XSmall).color(LabelColor::Muted))
            .into_any_element()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// QuickOpen (wrapper)
// ─────────────────────────────────────────────────────────────────────────────

pub struct QuickOpen {
    picker: Entity<Picker<QuickOpenDelegate>>,
}

impl QuickOpen {
    pub fn new(ui_tx: Sender<UiToHost>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let delegate = QuickOpenDelegate::new(ui_tx);
        let picker = cx.new(|cx| Picker::new(delegate, window, cx).width(400.0).max_height(400.0));
        cx.subscribe(&picker, |_, _, _: &DismissEvent, cx| cx.emit(DismissEvent)).detach();
        Self { picker }
    }

    pub fn set_results(&mut self, items: Vec<QuickOpenItem>, cx: &mut Context<Self>) {
        self.picker.update(cx, |p, cx| {
            p.delegate.set_items(items);
            cx.notify();
        });
    }
}

impl EventEmitter<DismissEvent> for QuickOpen {}
impl Focusable for QuickOpen { fn focus_handle(&self, cx: &App) -> FocusHandle { self.picker.read(cx).focus_handle(cx) } }

impl Render for QuickOpen {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(400.0))
            .bg(ThemeColors::bg_secondary())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_md()
            .shadow_lg()
            .child(self.picker.clone())
    }
}
