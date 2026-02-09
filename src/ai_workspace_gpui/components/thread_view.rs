//! ThreadView: Chat message list using ListState::Bottom for proper chat alignment.
use gpui::{AnyElement, App, ClipboardItem, Context, Entity, FocusHandle, Focusable, IntoElement, ListAlignment, ListOffset, ListState, ParentElement, Render, SharedString, Styled, Window, div, list, prelude::*, px};
use crossbeam_channel::Sender;
use crate::ai_workspace_gpui::{ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Spacing, ContextMenu, ContextMenuItem}, protocol::{EntrySnapshot, SessionSnapshot, UiToHost, MessageStateSnapshot, BusyStageSnapshot}};
use super::{MessageEntry, MessageRole, ToolCard, EntryViewState};

// ─────────────────────────────────────────────────────────────────────────────
// ThreadView Entity (stateful, uses ListState for efficient scrolling)
// ─────────────────────────────────────────────────────────────────────────────

pub struct ThreadView {
    session: Option<SessionSnapshot>,
    list_state: ListState,
    entry_view_state: EntryViewState,
    ui_tx: Sender<UiToHost>,
    focus_handle: FocusHandle,
    last_entry_count: usize,
    context_menu: Option<(usize, Entity<ContextMenu>)>,
}

impl ThreadView {
    pub fn new(session: Option<SessionSnapshot>, ui_tx: Sender<UiToHost>, cx: &mut Context<Self>) -> Self {
        let entry_count = session.as_ref().map(|s| s.entries.len()).unwrap_or(0);
        let list_state = ListState::new(entry_count, ListAlignment::Bottom, px(512.0));
        let mut entry_view_state = EntryViewState::new();
        if let Some(s) = &session { entry_view_state.sync_all(&s.entries); }
        Self { session, list_state, entry_view_state, ui_tx, focus_handle: cx.focus_handle(), last_entry_count: entry_count, context_menu: None }
    }

    pub fn set_session(&mut self, session: Option<SessionSnapshot>, cx: &mut Context<Self>) {
        let new_count = session.as_ref().map(|s| s.entries.len()).unwrap_or(0);
        if let Some(s) = &session { self.entry_view_state.sync_all(&s.entries); }
        self.session = session;
        if new_count != self.last_entry_count {
            self.list_state.reset(new_count);
            self.last_entry_count = new_count;
        }
        cx.notify();
    }

    pub fn scroll_to_bottom(&self) { self.list_state.scroll_to(gpui::ListOffset { item_ix: usize::MAX, offset_in_item: px(0.0) }); }

    pub fn show_context_menu(&mut self, entry_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = &self.session else { return; };
        let Some(entry) = session.entries.get(entry_ix) else { return; };

        let mut items = vec![ContextMenuItem::header("Message")];

        // Copy action
        let content = match entry {
            EntrySnapshot::User { text, .. } => text.clone(),
            EntrySnapshot::Assistant { content, .. } => content.clone(),
            EntrySnapshot::ToolCall(tc) => tc.title.clone(),
        };
        let content_for_copy = content.clone();
        items.push(ContextMenuItem::entry("Copy", move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(content_for_copy.clone()))));

        // Edit action (user messages only)
        if matches!(entry, EntrySnapshot::User { .. }) {
            items.push(ContextMenuItem::entry("Edit", move |_, _| {
                // TODO: implement edit
            }));
        }

        // Retry action (assistant messages)
        if matches!(entry, EntrySnapshot::Assistant { .. }) {
            items.push(ContextMenuItem::entry("Retry", move |_, _| {
                // TODO: implement retry
            }));
        }

        items.push(ContextMenuItem::separator());
        items.push(ContextMenuItem::entry("Delete", move |_, _| {
            // TODO: implement delete
        }));

        let menu = cx.new(|cx| ContextMenu::new(items, cx));
        self.context_menu = Some((entry_ix, menu));
        cx.notify();
    }

    pub fn hide_context_menu(&mut self, cx: &mut Context<Self>) {
        self.context_menu = None;
        cx.notify();
    }
}

impl Focusable for ThreadView { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for ThreadView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(session) = &self.session else {
            return v_flex().flex_1().size_full().items_center().justify_center()
                .child(Label::new("No session selected").color(LabelColor::Muted)).into_any_element();
        };
        if session.entries.is_empty() {
            return v_flex().flex_1().size_full().items_center().justify_center()
                .child(v_flex().gap(Spacing::Base08.px()).items_center()
                    .child(Label::new("AI").size(LabelSize::Large))
                    .child(Label::new("Start a conversation...").color(LabelColor::Muted))
                    .child(Label::new("Type a message below and press Enter to send").size(LabelSize::XSmall).color(LabelColor::Muted))
                ).into_any_element();
        }

        let session_clone = session.clone();
        let tx = self.ui_tx.clone();

        div()
            .id("thread-view")
            .track_focus(&self.focus_handle)
            .flex_1()
            .size_full()
            .overflow_hidden()
            .child(
                list(self.list_state.clone(), move |ix, _window, _cx| {
                    let Some(entry) = session_clone.entries.get(ix) else { return div().h(px(20.0)).into_any_element(); };
                    let sid = session_clone.id;
                    let tx = tx.clone();
                    let prev_role = if ix > 0 {
                        session_clone.entries.get(ix - 1).map(|e| match e {
                            EntrySnapshot::User { .. } => MessageRole::User,
                            EntrySnapshot::Assistant { .. } => MessageRole::Assistant,
                            EntrySnapshot::ToolCall(_) => MessageRole::Tool,
                        })
                    } else { None };
                    match entry {
                        EntrySnapshot::User { text, timestamp, .. } => MessageEntry::user(ix, text).with_prev_role(prev_role).with_timestamp(*timestamp).into_any_element(),
                        EntrySnapshot::Assistant { content, state, thinking, timestamp, .. } => MessageEntry::assistant(ix, content, *state, thinking.as_ref()).with_prev_role(prev_role).with_timestamp(*timestamp).into_any_element(),
                        EntrySnapshot::ToolCall(c) => ToolCard::new(ix, c.clone(), sid, tx).into_any_element(),
                    }
                })
                .flex_1()
                .size_full()
                .py(Spacing::Base12.px())
            )
            .child(self.render_busy_indicator(session, cx))
            .into_any_element()
    }
}

impl ThreadView {
    fn render_busy_indicator(&self, session: &SessionSnapshot, _cx: &Context<Self>) -> AnyElement {
        if !session.is_busy { return div().into_any_element(); }
        let reason = session.busy_reason.clone().unwrap_or_else(|| "Processing...".into());
        let stage_text: Option<SharedString> = match session.busy_stage {
            BusyStageSnapshot::Idle => None,
            BusyStageSnapshot::ToolRunning => Some("Running tool...".into()),
            BusyStageSnapshot::ToolFeedback => Some("Awaiting feedback...".into()),
            BusyStageSnapshot::WaitingModel => Some("Waiting for model...".into()),
            BusyStageSnapshot::Generating => Some("Generating...".into()),
            BusyStageSnapshot::AutoHeal { current, max } => Some(format!("Auto-healing ({}/{})", current, max).into()),
            BusyStageSnapshot::NetworkRetry { attempt } => Some(format!("Retry #{}", attempt).into()),
        };

        h_flex()
            .flex_none()
            .w_full()
            .px(Spacing::Base08.px())
            .py(Spacing::Base04.px())
            .gap(Spacing::Base08.px())
            .bg(ThemeColors::bg_secondary())
            .border_t_1()
            .border_color(ThemeColors::border())
            .child(div().w(px(16.0)).h(px(16.0)).rounded_full().bg(ThemeColors::text_accent()))
            .child(Label::new(reason).size(LabelSize::Small).color(LabelColor::Secondary))
            .children(stage_text.as_ref().map(|s| Label::new(s.clone()).size(LabelSize::XSmall).color(LabelColor::Muted)))
            .into_any_element()
    }
}
