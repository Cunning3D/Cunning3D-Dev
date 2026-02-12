//! Context menu component (Zed-style right-click menus with keyboard navigation).
use gpui::{actions, AnyElement, App, ClickEvent, Context, DismissEvent, ElementId, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, anchored, deferred, div, prelude::*, px};
use std::sync::Arc;
use super::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Spacing};

actions!(context_menu, [SelectNext, SelectPrev, Confirm, Cancel]);

// ─────────────────────────────────────────────────────────────────────────────
// ContextMenuItem
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub enum ContextMenuItem {
    Entry { label: SharedString, icon: Option<SharedString>, shortcut: Option<SharedString>, disabled: bool, handler: Arc<dyn Fn(&mut Window, &mut App) + 'static> },
    Separator,
    Header(SharedString),
}

impl ContextMenuItem {
    pub fn entry(label: impl Into<SharedString>, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self::Entry { label: label.into(), icon: None, shortcut: None, disabled: false, handler: Arc::new(handler) }
    }
    pub fn with_icon(mut self, icon: impl Into<SharedString>) -> Self { if let Self::Entry { icon: ref mut i, .. } = self { *i = Some(icon.into()); } self }
    pub fn with_shortcut(mut self, shortcut: impl Into<SharedString>) -> Self { if let Self::Entry { shortcut: ref mut s, .. } = self { *s = Some(shortcut.into()); } self }
    pub fn disabled(mut self, disabled: bool) -> Self { if let Self::Entry { disabled: ref mut d, .. } = self { *d = disabled; } self }
    pub fn separator() -> Self { Self::Separator }
    pub fn header(label: impl Into<SharedString>) -> Self { Self::Header(label.into()) }
}

// ─────────────────────────────────────────────────────────────────────────────
// ContextMenu Entity
// ─────────────────────────────────────────────────────────────────────────────

pub struct ContextMenu {
    items: Vec<ContextMenuItem>,
    selected_idx: Option<usize>,
    focus_handle: FocusHandle,
    focus_out_subscription: Option<Subscription>,
}

impl EventEmitter<DismissEvent> for ContextMenu {}

impl ContextMenu {
    pub fn new(items: Vec<ContextMenuItem>, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let selected = items.iter().position(|i| matches!(i, ContextMenuItem::Entry { disabled: false, .. }));
        Self { items, selected_idx: selected, focus_handle, focus_out_subscription: None }
    }

    fn entry_count(&self) -> usize { self.items.iter().filter(|i| matches!(i, ContextMenuItem::Entry { .. })).count() }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        let entries: Vec<usize> = self.items.iter().enumerate().filter_map(|(i, item)| matches!(item, ContextMenuItem::Entry { disabled: false, .. }).then_some(i)).collect();
        if entries.is_empty() { return; }
        let current = self.selected_idx.unwrap_or(0);
        self.selected_idx = entries.iter().find(|&&i| i > current).or(entries.first()).copied();
        cx.notify();
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        let entries: Vec<usize> = self.items.iter().enumerate().filter_map(|(i, item)| matches!(item, ContextMenuItem::Entry { disabled: false, .. }).then_some(i)).collect();
        if entries.is_empty() { return; }
        let current = self.selected_idx.unwrap_or(entries.len());
        self.selected_idx = entries.iter().rev().find(|&&i| i < current).or(entries.last()).copied();
        cx.notify();
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(idx) = self.selected_idx {
            if let Some(ContextMenuItem::Entry { handler, disabled: false, .. }) = self.items.get(idx) {
                handler(window, cx);
            }
        }
        cx.emit(DismissEvent);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) { cx.emit(DismissEvent); }

    fn on_item_click(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ContextMenuItem::Entry { handler, disabled: false, .. }) = self.items.get(idx) {
            handler(window, cx);
            cx.emit(DismissEvent);
        }
    }
}

impl Focusable for ContextMenu { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for ContextMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_out_subscription.is_none() {
            let sub = cx.on_focus_out(&self.focus_handle, window, |_this, _evt, _window, cx| {
                cx.emit(DismissEvent);
            });
            self.focus_out_subscription = Some(sub);
        }

        let items: Vec<AnyElement> = self.items.iter().enumerate().map(|(idx, item)| {
            match item {
                ContextMenuItem::Entry { label, icon, shortcut, disabled, .. } => {
                    let is_selected = self.selected_idx == Some(idx);
                    let disabled = *disabled;
                    h_flex()
                        .id(ElementId::NamedInteger("menu-item".into(), idx as u64))
                        .w_full()
                        .px(Spacing::Base06.px())
                        .py(Spacing::Base02.px())
                        .gap(Spacing::Base06.px())
                        .rounded_sm()
                        .cursor_pointer()
                        .when(is_selected && !disabled, |d| d.bg(ThemeColors::bg_selected()))
                        .when(!is_selected && !disabled, |d| d.hover(|d| d.bg(ThemeColors::bg_hover())))
                        .when(disabled, |d| d.opacity(0.5).cursor_default())
                        .children(icon.as_ref().map(|i| Label::new(i.clone()).size(LabelSize::Small).color(LabelColor::Secondary)))
                        .child(Label::new(label.clone()).size(LabelSize::Small).color(if disabled { LabelColor::Muted } else { LabelColor::Primary }))
                        .child(div().flex_1())
                        .children(shortcut.as_ref().map(|s| Label::new(s.clone()).size(LabelSize::XSmall).color(LabelColor::Muted)))
                        .on_click(cx.listener(move |this, _, window, cx| { if !disabled { this.on_item_click(idx, window, cx); } }))
                        .on_hover(cx.listener(move |this, hovered, _, cx| { if *hovered && !disabled { this.selected_idx = Some(idx); cx.notify(); } }))
                        .into_any_element()
                }
                ContextMenuItem::Separator => div().w_full().h(px(1.0)).my(Spacing::Base04.px()).bg(ThemeColors::border()).into_any_element(),
                ContextMenuItem::Header(label) => h_flex().w_full().px(Spacing::Base08.px()).py(Spacing::Base02.px())
                    .child(Label::new(label.clone()).size(LabelSize::XSmall).color(LabelColor::Muted))
                    .into_any_element(),
            }
        }).collect();

        v_flex()
            .id("context-menu")
            .key_context("ContextMenu")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .min_w(px(180.0))
            .max_w(px(300.0))
            .p(Spacing::Base04.px())
            .bg(ThemeColors::bg_elevated())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_sm()
            .shadow_lg()
            .children(items)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Context Menu Builder / Opener
// ─────────────────────────────────────────────────────────────────────────────

pub struct ContextMenuHandle { pub menu: Option<Entity<ContextMenu>> }
impl Default for ContextMenuHandle { fn default() -> Self { Self { menu: None } } }

impl ContextMenuHandle {
    pub fn open(&mut self, items: Vec<ContextMenuItem>, window: &mut Window, cx: &mut App) {
        let menu = cx.new(|cx| ContextMenu::new(items, cx));
        menu.focus_handle(cx).focus(window, cx);
        self.menu = Some(menu);
    }
    pub fn close(&mut self) { self.menu = None; }
    pub fn is_open(&self) -> bool { self.menu.is_some() }

    pub fn render(&self, anchor: gpui::Point<gpui::Pixels>) -> Option<impl IntoElement> {
        self.menu.as_ref().map(|menu| {
            deferred(anchored().position(anchor).child(menu.clone())).with_priority(1)
        })
    }
}
