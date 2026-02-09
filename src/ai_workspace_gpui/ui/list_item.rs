//! ListItem component (Zed-style API)

use gpui::{AnyElement, App, ClickEvent, ElementId, IntoElement, ParentElement, Styled, Window, div, prelude::*};
use super::{ThemeColors, h_flex, Spacing};

#[derive(Clone, Copy, Default, PartialEq)]
pub enum ListItemSpacing { #[default] Dense, Sparse }

pub struct ListItem {
    id: ElementId,
    selected: bool,
    disabled: bool,
    spacing: ListItemSpacing,
    start_slot: Option<AnyElement>,
    end_slot: Option<AnyElement>,
    on_click: Option<Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>>,
    children: Vec<AnyElement>,
}

impl ListItem {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), selected: false, disabled: false, spacing: ListItemSpacing::default(), start_slot: None, end_slot: None, on_click: None, children: Vec::new() }
    }
    pub fn toggle_state(mut self, selected: bool) -> Self { self.selected = selected; self }
    pub fn disabled(mut self, disabled: bool) -> Self { self.disabled = disabled; self }
    pub fn spacing(mut self, spacing: ListItemSpacing) -> Self { self.spacing = spacing; self }
    pub fn start_slot<E: IntoElement>(mut self, el: impl Into<Option<E>>) -> Self { self.start_slot = el.into().map(|e| e.into_any_element()); self }
    pub fn end_slot<E: IntoElement>(mut self, el: impl Into<Option<E>>) -> Self { self.end_slot = el.into().map(|e| e.into_any_element()); self }
    pub fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self { self.on_click = Some(Box::new(handler)); self }
    pub fn child<E: IntoElement>(mut self, child: E) -> Self { self.children.push(child.into_any_element()); self }
    pub fn children<E: IntoElement>(mut self, children: impl IntoIterator<Item = E>) -> Self { self.children.extend(children.into_iter().map(|c| c.into_any_element())); self }
}

impl IntoElement for ListItem {
    type Element = AnyElement;
    fn into_element(self) -> Self::Element {
        let py = match self.spacing { ListItemSpacing::Dense => Spacing::Base02.px(), ListItemSpacing::Sparse => Spacing::Base04.px() };
        let base = div().id(self.id).w_full().px(Spacing::Base06.px()).py(py).rounded_sm().cursor_pointer()
            .when(!self.disabled, |d| d.hover(|s| s.bg(ThemeColors::bg_hover())).active(|s| s.bg(ThemeColors::bg_active())))
            .when(self.selected, |d| d.bg(ThemeColors::bg_selected()))
            .when(self.disabled, |d| d.opacity(0.5).cursor_default());
        let base = if let Some(handler) = self.on_click { base.on_click(handler) } else { base };
        let inner = h_flex().gap(Spacing::Base06.px()).flex_1().children(self.start_slot).children(self.children);
        let row = h_flex().w_full().justify_between().child(inner).children(self.end_slot);
        base.child(row).into_any_element()
    }
}
