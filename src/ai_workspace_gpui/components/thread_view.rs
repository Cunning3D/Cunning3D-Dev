//! ThreadView: Chat message list using ListState::Bottom for proper chat alignment.
use gpui::{AnyElement, App, ClipboardItem, Context, Entity, FocusHandle, Focusable, IntoElement, ListAlignment, ListOffset, ListState, ParentElement, Render, SharedString, Styled, Window, div, list, prelude::*, px};
use crossbeam_channel::Sender;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use crate::ai_workspace_gpui::{ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Spacing, ContextMenu, ContextMenuItem}, protocol::{EntrySnapshot, SessionSnapshot, UiToHost, MessageStateSnapshot, BusyStageSnapshot}};
use super::{MessageEntry, MessageRole, ToolCard, ToolCardView, ToolGroupCardView, EntryViewState};

#[derive(Clone, Debug)]
enum DisplayEntry {
    Single { entry_ix: usize },
    ToolGroup { group_id: u64, entry_ix: usize, tools: Vec<u64> },
}

// ─────────────────────────────────────────────────────────────────────────────
// ThreadView Entity (stateful, uses ListState for efficient scrolling)
// ─────────────────────────────────────────────────────────────────────────────

pub struct ThreadView {
    session: Option<SessionSnapshot>,
    session_shared: Arc<RwLock<Option<SessionSnapshot>>>,
    display_shared: Arc<RwLock<Vec<DisplayEntry>>>,
    list_state: ListState,
    entry_view_state: EntryViewState,
    tool_cards: Arc<RwLock<HashMap<u64, Entity<ToolCardView>>>>,
    tool_groups: Arc<RwLock<HashMap<u64, Entity<ToolGroupCardView>>>>,
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
        let session_shared = Arc::new(RwLock::new(session.clone()));
        let display_shared = Arc::new(RwLock::new(Vec::new()));
        let tool_cards = Arc::new(RwLock::new(HashMap::new()));
        let tool_groups = Arc::new(RwLock::new(HashMap::new()));
        let mut this = Self { session, session_shared, display_shared, list_state, entry_view_state, tool_cards, tool_groups, ui_tx, focus_handle: cx.focus_handle(), last_entry_count: entry_count, context_menu: None };
        this.sync_tool_cards(cx);
        this.rebuild_display_entries(cx);
        let disp_len = this.display_shared.read().ok().map(|d| d.len()).unwrap_or(entry_count);
        this.list_state.reset(disp_len);
        this.last_entry_count = disp_len;
        this
    }

    pub fn set_session(&mut self, session: Option<SessionSnapshot>, cx: &mut Context<Self>) {
        let new_count_raw = session.as_ref().map(|s| s.entries.len()).unwrap_or(0);
        if let Some(s) = &session { self.entry_view_state.sync_all(&s.entries); }
        self.session = session;
        if let Ok(mut g) = self.session_shared.write() { *g = self.session.clone(); }
        self.sync_tool_cards(cx);
        self.rebuild_display_entries(cx);
        let new_count = self.display_shared.read().ok().map(|d| d.len()).unwrap_or(new_count_raw);
        // Force refresh even when entry count is unchanged (tool status/logs update).
        self.list_state.reset(new_count);
        self.last_entry_count = new_count;
        cx.notify();
    }

    fn sync_tool_cards(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.session else { return; };
        let mut present: HashSet<u64> = HashSet::new();
        for (ix, entry) in session.entries.iter().enumerate() {
            let EntrySnapshot::ToolCall(tc) = entry else { continue; };
            let id = tc.id;
            present.insert(id);
            let mut map = self.tool_cards.write().ok();
            let Some(map) = map.as_mut() else { continue; };
            if let Some(ent) = map.get(&id) {
                let tc = tc.clone();
                ent.update(cx, |v, cx| v.set_tool(tc, cx));
            } else {
                let tx = self.ui_tx.clone();
                let sid = session.id;
                let tc2 = tc.clone();
                let ent = cx.new(|_cx| ToolCardView::new(ix, tc2, sid, tx));
                map.insert(id, ent);
            }
        }
        if let Ok(mut map) = self.tool_cards.write() {
            map.retain(|id, _| present.contains(id));
        }
    }

    fn rebuild_display_entries(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            if let Ok(mut d) = self.display_shared.write() { d.clear(); }
            return;
        };
        let mut display: Vec<DisplayEntry> = Vec::with_capacity(session.entries.len());
        let mut i = 0usize;
        while i < session.entries.len() {
            match session.entries.get(i) {
                Some(EntrySnapshot::ToolCall(tc)) => {
                    let mut tools = vec![tc.id];
                    let group_id = tc.id;
                    let entry_ix = i;
                    i += 1;
                    while i < session.entries.len() {
                        match session.entries.get(i) {
                            Some(EntrySnapshot::ToolCall(tc2)) => { tools.push(tc2.id); i += 1; }
                            _ => break,
                        }
                    }
                    display.push(DisplayEntry::ToolGroup { group_id, entry_ix, tools });
                }
                Some(_) => {
                    display.push(DisplayEntry::Single { entry_ix: i });
                    i += 1;
                }
                None => break,
            }
        }
        if let Ok(mut d) = self.display_shared.write() { *d = display; }
        self.sync_tool_groups(cx);
    }

    fn sync_tool_groups(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.session else { return; };
        let Ok(display) = self.display_shared.read() else { return; };
        let mut by_id: HashMap<u64, crate::ai_workspace_gpui::protocol::ToolCallSnapshot> = HashMap::new();
        for e in &session.entries {
            if let EntrySnapshot::ToolCall(tc) = e { by_id.insert(tc.id, tc.clone()); }
        }
        let mut present: HashSet<u64> = HashSet::new();
        for de in display.iter() {
            let DisplayEntry::ToolGroup { group_id, entry_ix: _, tools } = de else { continue; };
            present.insert(*group_id);
            let snapshots: Vec<_> = tools.iter().filter_map(|id| by_id.get(id).cloned()).collect();
            if let Ok(mut map) = self.tool_groups.write() {
                if let Some(ent) = map.get(group_id) {
                    let snaps = snapshots.clone();
                    ent.update(cx, |v, cx| v.set_tools(snaps, cx));
                } else {
                    let tx = self.ui_tx.clone();
                    let sid = session.id;
                    let tool_cards = self.tool_cards.clone();
                    let snaps = snapshots.clone();
                    let gid = *group_id;
                    let ent = cx.new(|_cx| ToolGroupCardView::new(gid, snaps, sid, tx, tool_cards));
                    map.insert(*group_id, ent);
                }
            }
        }
        if let Ok(mut map) = self.tool_groups.write() {
            map.retain(|gid, _| present.contains(gid));
        }
    }

    pub fn scroll_to_bottom(&self) { self.list_state.scroll_to(gpui::ListOffset { item_ix: usize::MAX, offset_in_item: px(0.0) }); }

    pub fn show_context_menu(&mut self, entry_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = &self.session else { return; };
        let entry_ix = self.display_shared.read().ok().and_then(|d| d.get(entry_ix).map(|de| match de {
            DisplayEntry::Single { entry_ix } => *entry_ix,
            DisplayEntry::ToolGroup { entry_ix, .. } => *entry_ix,
        })).unwrap_or(entry_ix);
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
            let sid = session.id;
            let tx = self.ui_tx.clone();
            let presets = [
                ("Create a Cube node", "Create a 'Create Cube' node in the current node graph, name it 'cube', and set it as display."),
                ("Create node network", "Create: Create Cube -> Transform -> Merge (name them cube/xform/merge), connect them, set merge as display."),
                ("Inspect display geometry", "Get geometry insight for the current display node and summarize point/prim counts and bbox."),
                ("Write a Rust plugin node", "Create a new Rust plugin/custom node via NodeSpec. Ask me 2 clarification questions first: node name + inputs/outputs/params."),
            ];
            return v_flex().flex_1().size_full().items_center().justify_center()
                .child(v_flex().gap(Spacing::Base08.px()).items_center()
                    .child(Label::new("AI").size(LabelSize::Large))
                    .child(Label::new("Start a conversation...").color(LabelColor::Muted))
                    .child(Label::new("Type a message below and press Enter to send").size(LabelSize::XSmall).color(LabelColor::Muted))
                    .child(
                        v_flex()
                            .gap(Spacing::Base04.px())
                            .pt(Spacing::Base06.px())
                            .child(Label::new("Presets").size(LabelSize::Small).color(LabelColor::Secondary))
                            .child(
                                h_flex()
                                    .gap(Spacing::Base04.px())
                                    .flex_wrap()
                                    .children(presets.into_iter().map(|(title, text)| {
                                        let tx = tx.clone();
                                        let text = text.to_string();
                                        let key = title.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>();
                                        let id = format!("preset-{}-{}", sid, key);
                                        crate::ai_workspace_gpui::ui::Button::new(id, title)
                                            .style(crate::ai_workspace_gpui::ui::ButtonStyle::Subtle)
                                            .on_click(move |_, _, _| { let _ = tx.send(UiToHost::SetComposerText { session_id: sid, text: text.clone() }); })
                                            .into_any_element()
                                    }).collect::<Vec<_>>())
                            )
                    )
                ).into_any_element();
        }

        // IMPORTANT: don't capture a cloned snapshot in the list closure.
        // Using a clone here makes tool / streaming updates appear "stale" until a full resync.
        let tx = self.ui_tx.clone();
        let session_shared = self.session_shared.clone();
        let display_shared = self.display_shared.clone();
        let tool_cards = self.tool_cards.clone();
        let tool_groups = self.tool_groups.clone();

        div()
            .id("thread-view")
            .track_focus(&self.focus_handle)
            .flex_1()
            .size_full()
            .overflow_hidden()
            .child(
                list(self.list_state.clone(), {
                    let tx = tx.clone();
                    let session_shared = session_shared.clone();
                    let display_shared = display_shared.clone();
                    let tool_cards = tool_cards.clone();
                    let tool_groups = tool_groups.clone();
                    move |ix, _window, _cx| {
                        let session = match session_shared.read() {
                            Ok(g) => g.clone(),
                            Err(_) => None,
                        };
                        let Some(session) = session else { return div().h(px(20.0)).into_any_element(); };
                        let display = match display_shared.read() {
                            Ok(d) => d.clone(),
                            Err(_) => Vec::new(),
                        };
                        let Some(de) = display.get(ix) else { return div().h(px(20.0)).into_any_element(); };
                        let sid = session.id;
                        let tx = tx.clone();
                        let prev_role = if ix > 0 {
                            display.get(ix - 1).map(|p| match p {
                                DisplayEntry::Single { entry_ix } => session.entries.get(*entry_ix).map(|e| match e {
                                    EntrySnapshot::User { .. } => MessageRole::User,
                                    EntrySnapshot::Assistant { .. } => MessageRole::Assistant,
                                    EntrySnapshot::ToolCall(_) => MessageRole::Tool,
                                }),
                                DisplayEntry::ToolGroup { .. } => Some(MessageRole::Tool),
                            }).flatten()
                        } else { None };

                        match de {
                            DisplayEntry::Single { entry_ix } => {
                                let Some(entry) = session.entries.get(*entry_ix) else { return div().h(px(20.0)).into_any_element(); };
                                match entry {
                                    EntrySnapshot::User { text, timestamp, .. } => MessageEntry::user(*entry_ix, text).with_prev_role(prev_role).with_timestamp(*timestamp).into_any_element(),
                                    EntrySnapshot::Assistant { content, state, thinking, timestamp, .. } => MessageEntry::assistant(*entry_ix, content, *state, thinking.as_ref()).with_ui_tx(tx.clone()).with_prev_role(prev_role).with_timestamp(*timestamp).into_any_element(),
                                    EntrySnapshot::ToolCall(c) => {
                                        if let Ok(map) = tool_cards.read() {
                                            if let Some(ent) = map.get(&c.id) {
                                                return ent.clone().into_any_element();
                                            }
                                        }
                                        ToolCard::new(*entry_ix, c.clone(), sid, tx).into_any_element()
                                    }
                                }
                            }
                            DisplayEntry::ToolGroup { group_id, .. } => {
                                if let Ok(map) = tool_groups.read() {
                                    if let Some(ent) = map.get(group_id) {
                                        return ent.clone().into_any_element();
                                    }
                                }
                                div().h(px(20.0)).into_any_element()
                            }
                        }
                    }
                })
                .flex_1()
                .size_full()
                .py(Spacing::Base06.px())
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
            BusyStageSnapshot::ToolFeedback => Some("Processing tool result...".into()),
            BusyStageSnapshot::WaitingModel => Some("Waiting for model...".into()),
            BusyStageSnapshot::Generating => Some("Generating...".into()),
            BusyStageSnapshot::AutoHeal { current, max } => Some(format!("Auto-healing ({}/{})", current, max).into()),
            BusyStageSnapshot::NetworkRetry { attempt } => Some(format!("Retry #{}", attempt).into()),
        };

        h_flex()
            .flex_none()
            .w_full()
            .px(Spacing::Base06.px())
            .py(Spacing::Base02.px())
            .gap(Spacing::Base06.px())
            .bg(ThemeColors::bg_secondary())
            .border_t_1()
            .border_color(ThemeColors::border())
            .child(div().w(px(16.0)).h(px(16.0)).rounded_full().bg(ThemeColors::text_accent()))
            .child(Label::new(reason).size(LabelSize::Small).color(LabelColor::Secondary))
            .children(stage_text.as_ref().map(|s| Label::new(s.clone()).size(LabelSize::XSmall).color(LabelColor::Muted)))
            .into_any_element()
    }
}
