//! CommandPalette: Command search (Ctrl+Shift+P) - Zed-isomorphic

use gpui::{AnyElement, App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render, SharedString, Window, div, prelude::*, px};
use crossbeam_channel::Sender;
use crate::ai_workspace_gpui::{ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Picker, PickerDelegate, Spacing}, protocol::{CommandItem, UiToHost}};

// ─────────────────────────────────────────────────────────────────────────────
// CommandPaletteDelegate
// ─────────────────────────────────────────────────────────────────────────────

pub struct CommandPaletteDelegate {
    items: Vec<CommandItem>,
    selected_ix: usize,
    ui_tx: Sender<UiToHost>,
    on_command: Option<Box<dyn Fn(&str, &mut Window, &mut App) + 'static>>,
}

impl CommandPaletteDelegate {
    pub fn new(ui_tx: Sender<UiToHost>) -> Self {
        Self { items: vec![], selected_ix: 0, ui_tx, on_command: None }
    }

    pub fn on_command(mut self, f: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_command = Some(Box::new(f));
        self
    }

    pub fn set_items(&mut self, items: Vec<CommandItem>) {
        self.items = items;
        self.selected_ix = 0;
    }
}

impl PickerDelegate for CommandPaletteDelegate {
    type ListItem = AnyElement;

    fn match_count(&self) -> usize { self.items.len() }
    fn selected_index(&self) -> usize { self.selected_ix }
    fn set_selected_index(&mut self, ix: usize, cx: &mut Context<Picker<Self>>) { self.selected_ix = ix; cx.notify(); }
    fn placeholder_text(&self, _cx: &App) -> SharedString { "Type a command...".into() }

    fn update_matches(&mut self, query: &str, cx: &mut Context<Picker<Self>>) {
        let _ = self.ui_tx.send(UiToHost::IdeCommandPalette { query: query.to_string() });
    }

    fn confirm(&mut self, _secondary: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        if let Some(item) = self.items.get(self.selected_ix) {
            if let Some(ref cb) = self.on_command {
                cb(&item.id, window, cx);
            }
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
            .justify_between()
            .items_center()
            .rounded_sm()
            .cursor_pointer()
            .when(selected, |d| d.bg(ThemeColors::bg_selected()))
            .when(!selected, |d| d.hover(|d| d.bg(ThemeColors::bg_hover())))
            .child(
                v_flex()
                    .gap(Spacing::Base02.px())
                    .child(Label::new(&item.label).size(LabelSize::Small).color(if selected { LabelColor::Primary } else { LabelColor::Secondary }))
                    .when_some(item.description.as_ref(), |d, desc| d.child(Label::new(desc).size(LabelSize::XSmall).color(LabelColor::Muted)))
            )
            .when_some(item.keybinding.as_ref(), |d, kb| {
                d.child(Label::new(kb).size(LabelSize::XSmall).color(LabelColor::Muted))
            })
            .into_any_element()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CommandPalette (wrapper)
// ─────────────────────────────────────────────────────────────────────────────

pub struct CommandPalette {
    picker: Entity<Picker<CommandPaletteDelegate>>,
}

impl CommandPalette {
    pub fn new(ui_tx: Sender<UiToHost>, on_command: impl Fn(&str, &mut Window, &mut App) + 'static, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let delegate = CommandPaletteDelegate::new(ui_tx).on_command(on_command);
        let picker = cx.new(|cx| Picker::new(delegate, window, cx).width(450.0).max_height(400.0));
        cx.subscribe(&picker, |_, _, _: &DismissEvent, cx| cx.emit(DismissEvent)).detach();
        // Request initial command list
        Self { picker }
    }

    pub fn set_results(&mut self, items: Vec<CommandItem>, cx: &mut Context<Self>) {
        self.picker.update(cx, |p, cx| {
            p.delegate.set_items(items);
            cx.notify();
        });
    }
}

impl EventEmitter<DismissEvent> for CommandPalette {}
impl Focusable for CommandPalette { fn focus_handle(&self, cx: &App) -> FocusHandle { self.picker.read(cx).focus_handle(cx) } }

impl Render for CommandPalette {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(450.0))
            .bg(ThemeColors::bg_secondary())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_md()
            .shadow_lg()
            .child(self.picker.clone())
    }
}
