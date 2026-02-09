//! Entry view state management (Zed-aligned architecture).
use gpui::{AnyElement, EntityId};
use std::collections::HashMap;
use crate::ai_workspace_gpui::protocol::{EntrySnapshot, MessageStateSnapshot};

// ─────────────────────────────────────────────────────────────────────────────
// Entry
// ─────────────────────────────────────────────────────────────────────────────

pub enum Entry {
    /// User message with editable content
    UserMessage { content: String, is_editing: bool, can_edit: bool },
    /// Assistant message with scroll state
    AssistantMessage { scroll_handle_by_chunk: HashMap<usize, gpui::ScrollHandle> },
    /// Tool call content (terminals, diffs, etc.)
    Content(HashMap<EntityId, AnyElement>),
}

impl Entry {
    pub fn user_message(content: impl Into<String>, can_edit: bool) -> Self {
        Self::UserMessage { content: content.into(), is_editing: false, can_edit }
    }

    pub fn assistant_message() -> Self {
        Self::AssistantMessage { scroll_handle_by_chunk: HashMap::new() }
    }

    pub fn content() -> Self {
        Self::Content(HashMap::new())
    }

    pub fn is_user_message(&self) -> bool { matches!(self, Self::UserMessage { .. }) }
    pub fn is_assistant_message(&self) -> bool { matches!(self, Self::AssistantMessage { .. }) }
    pub fn is_content(&self) -> bool { matches!(self, Self::Content(_)) }

    pub fn set_editing(&mut self, editing: bool) {
        if let Self::UserMessage { is_editing, .. } = self { *is_editing = editing; }
    }

    pub fn scroll_handle_for_chunk(&self, chunk_ix: usize) -> Option<gpui::ScrollHandle> {
        match self {
            Self::AssistantMessage { scroll_handle_by_chunk } => scroll_handle_by_chunk.get(&chunk_ix).cloned(),
            _ => None,
        }
    }

    pub fn ensure_scroll_handle(&mut self, chunk_ix: usize) -> Option<gpui::ScrollHandle> {
        match self {
            Self::AssistantMessage { scroll_handle_by_chunk } => {
                Some(scroll_handle_by_chunk.entry(chunk_ix).or_default().clone())
            }
            _ => None,
        }
    }

    pub fn add_content(&mut self, id: EntityId, element: AnyElement) {
        if let Self::Content(map) = self { map.insert(id, element); }
    }

    pub fn content_count(&self) -> usize {
        match self { Self::Content(map) => map.len(), _ => 0 }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// EntryViewState
// ─────────────────────────────────────────────────────────────────────────────

pub struct EntryViewState {
    entries: Vec<Entry>,
}

impl EntryViewState {
    pub fn new() -> Self { Self { entries: Vec::new() } }

    pub fn entry(&self, index: usize) -> Option<&Entry> { self.entries.get(index) }
    pub fn entry_mut(&mut self, index: usize) -> Option<&mut Entry> { self.entries.get_mut(index) }
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    /// Sync entry from protocol snapshot
    pub fn sync_entry(&mut self, index: usize, snapshot: &EntrySnapshot) {
        match snapshot {
            EntrySnapshot::User { text, .. } => {
                if let Some(Entry::UserMessage { content: c, .. }) = self.entries.get_mut(index) {
                    *c = text.clone();
                } else {
                    self.set_entry(index, Entry::user_message(text, true));
                }
            }
            EntrySnapshot::Assistant { content, state, .. } => {
                if !matches!(self.entries.get(index), Some(Entry::AssistantMessage { .. })) {
                    self.set_entry(index, Entry::assistant_message());
                }
                // Auto-scroll to bottom if streaming
                if *state == MessageStateSnapshot::Streaming {
                    if let Some(Entry::AssistantMessage { scroll_handle_by_chunk }) = self.entries.get_mut(index) {
                        let last_chunk = 0; // Simplified
                        scroll_handle_by_chunk.entry(last_chunk).or_default().scroll_to_bottom();
                    }
                }
            }
            EntrySnapshot::ToolCall(_tc) => {
                if !matches!(self.entries.get(index), Some(Entry::Content(_))) {
                    self.set_entry(index, Entry::content());
                }
            }
        }
    }

    /// Sync all entries from snapshot list
    pub fn sync_all(&mut self, snapshots: &[EntrySnapshot]) {
        // Remove extra entries
        if self.entries.len() > snapshots.len() {
            self.entries.truncate(snapshots.len());
        }
        // Sync existing and add new
        for (i, snap) in snapshots.iter().enumerate() {
            self.sync_entry(i, snap);
        }
    }

    fn set_entry(&mut self, index: usize, entry: Entry) {
        if index == self.entries.len() {
            self.entries.push(entry);
        } else if index < self.entries.len() {
            self.entries[index] = entry;
        }
    }

    pub fn clear(&mut self) { self.entries.clear(); }
}

impl Default for EntryViewState { fn default() -> Self { Self::new() } }
