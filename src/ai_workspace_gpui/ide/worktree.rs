//! Worktree: Lazy-loading file tree with watcher (Zed-isomorphic)

use crate::ai_workspace_gpui::protocol::{EntryId, EntryKind, FileEntrySnapshot, FileIcon, IdeEvent};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

static NEXT_ENTRY_ID: AtomicU64 = AtomicU64::new(1);
fn next_entry_id() -> EntryId { EntryId::new(NEXT_ENTRY_ID.fetch_add(1, Ordering::SeqCst)) }

// ─────────────────────────────────────────────────────────────────────────────
// Entry (internal tree node)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: EntryId,
    pub path: PathBuf,
    pub kind: EntryKind,
    pub is_expanded: bool,
    pub children: Vec<EntryId>,
    pub mtime: Option<u64>,
    pub size: Option<u64>,
}

impl Entry {
    fn new_file(path: PathBuf) -> Self {
        let (mtime, size) = Self::read_metadata(&path);
        Self { id: next_entry_id(), path, kind: EntryKind::File, is_expanded: false, children: vec![], mtime, size }
    }
    fn new_dir(path: PathBuf, loaded: bool) -> Self {
        let kind = if loaded { EntryKind::Dir } else { EntryKind::UnloadedDir };
        Self { id: next_entry_id(), path, kind, is_expanded: false, children: vec![], mtime: None, size: None }
    }
    fn read_metadata(path: &Path) -> (Option<u64>, Option<u64>) {
        std::fs::metadata(path).ok().map(|m| {
            let mtime = m.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_secs());
            let size = if m.is_file() { Some(m.len()) } else { None };
            (mtime, size)
        }).unwrap_or((None, None))
    }
    fn name(&self) -> &str { self.path.file_name().and_then(|n| n.to_str()).unwrap_or("") }
    fn depth(&self, root: &Path) -> usize { self.path.strip_prefix(root).map(|p| p.components().count()).unwrap_or(0) }
    pub fn to_snapshot(&self, root: &Path, selected_id: Option<EntryId>) -> FileEntrySnapshot {
        FileEntrySnapshot {
            id: self.id,
            path: self.path.clone(),
            name: self.name().to_string(),
            kind: self.kind,
            depth: self.depth(root),
            is_expanded: self.is_expanded,
            is_selected: selected_id == Some(self.id),
            size: self.size,
            mtime: self.mtime,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Worktree (main tree model)
// ─────────────────────────────────────────────────────────────────────────────

pub struct Worktree {
    root_path: Option<PathBuf>,
    entries: HashMap<EntryId, Entry>,
    root_id: Option<EntryId>,
    selected_id: Option<EntryId>,
    expanded_ids: HashSet<EntryId>,
    ignore_patterns: Vec<String>,
    event_tx: Sender<IdeEvent>,
    scan_rx: Receiver<ScanResult>,
    scan_tx: Sender<ScanRequest>,
    /// Zed-isomorphic: auto-expand first-level children after root loads
    auto_expand_root_children: bool,
}

enum ScanRequest { LoadDir { entry_id: EntryId, path: PathBuf }, Refresh { path: PathBuf } }
enum ScanResult { DirLoaded { entry_id: EntryId, children: Vec<Entry> }, Error { entry_id: EntryId, error: String } }

impl Worktree {
    pub fn new(event_tx: Sender<IdeEvent>) -> Self {
        let (scan_tx, scan_request_rx) = unbounded::<ScanRequest>();
        let (scan_result_tx, scan_rx) = unbounded::<ScanResult>();
        Self::spawn_scanner(scan_request_rx, scan_result_tx);
        Self {
            root_path: None, entries: HashMap::new(), root_id: None, selected_id: None,
            expanded_ids: HashSet::new(),
            ignore_patterns: vec![".git".into(), "target".into(), "node_modules".into(), "__pycache__".into(), ".DS_Store".into()],
            event_tx, scan_rx, scan_tx,
            auto_expand_root_children: false,
        }
    }

    fn spawn_scanner(rx: Receiver<ScanRequest>, tx: Sender<ScanResult>) {
        std::thread::spawn(move || {
            while let Ok(req) = rx.recv() {
                match req {
                    ScanRequest::LoadDir { entry_id, path } => {
                        match Self::scan_dir_sync(&path) {
                            Ok(children) => { let _ = tx.send(ScanResult::DirLoaded { entry_id, children }); }
                            Err(e) => { let _ = tx.send(ScanResult::Error { entry_id, error: e }); }
                        }
                    }
                    ScanRequest::Refresh { .. } => { /* TODO: full refresh */ }
                }
            }
        });
    }

    fn scan_dir_sync(path: &Path) -> Result<Vec<Entry>, String> {
        let rd = std::fs::read_dir(path).map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        for entry in rd.filter_map(|e| e.ok()) {
            let p = entry.path();
            let ft = entry.file_type().ok();
            if let Some(ft) = ft {
                if ft.is_dir() { entries.push(Entry::new_dir(p, false)); }
                else if ft.is_file() { entries.push(Entry::new_file(p)); }
            }
        }
        entries.sort_by(|a, b| {
            let a_dir = matches!(a.kind, EntryKind::Dir | EntryKind::UnloadedDir | EntryKind::PendingDir);
            let b_dir = matches!(b.kind, EntryKind::Dir | EntryKind::UnloadedDir | EntryKind::PendingDir);
            b_dir.cmp(&a_dir).then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
        });
        Ok(entries)
    }

    pub fn set_root(&mut self, path: PathBuf) {
        self.entries.clear();
        self.expanded_ids.clear();
        self.selected_id = None;
        let root = Entry::new_dir(path.clone(), true);
        let root_id = root.id;
        self.entries.insert(root_id, root);
        self.root_id = Some(root_id);
        self.root_path = Some(path.clone());
        self.expanded_ids.insert(root_id);
        let _ = self.scan_tx.send(ScanRequest::LoadDir { entry_id: root_id, path: path.clone() });
        if let Some(e) = self.entries.get_mut(&root_id) { e.kind = EntryKind::PendingDir; }
        let _ = self.event_tx.send(IdeEvent::TreeRootChanged { path });
        let _ = self.event_tx.send(IdeEvent::TreeLoading { entry_id: root_id });
        self.auto_expand_root_children = true;
    }

    pub fn expand(&mut self, entry_id: EntryId) {
        let Some(entry) = self.entries.get_mut(&entry_id) else { return; };
        if !matches!(entry.kind, EntryKind::Dir | EntryKind::UnloadedDir) { return; }
        entry.is_expanded = true;
        self.expanded_ids.insert(entry_id);
        if entry.kind == EntryKind::UnloadedDir {
            entry.kind = EntryKind::PendingDir;
            let path = entry.path.clone();
            let _ = self.scan_tx.send(ScanRequest::LoadDir { entry_id, path });
            let _ = self.event_tx.send(IdeEvent::TreeLoading { entry_id });
        } else {
            self.emit_visible_entries();
        }
    }

    pub fn collapse(&mut self, entry_id: EntryId) {
        let Some(entry) = self.entries.get_mut(&entry_id) else { return; };
        entry.is_expanded = false;
        self.expanded_ids.remove(&entry_id);
        let _ = self.event_tx.send(IdeEvent::TreeEntryCollapsed { entry_id });
        self.emit_visible_entries();
    }

    pub fn select(&mut self, entry_id: Option<EntryId>) {
        self.selected_id = entry_id;
        let _ = self.event_tx.send(IdeEvent::TreeEntrySelected { entry_id });
    }

    pub fn poll(&mut self) {
        while let Ok(result) = self.scan_rx.try_recv() {
            match result {
                ScanResult::DirLoaded { entry_id, children } => {
                    let filtered: Vec<_> = children.into_iter().filter(|e| !self.should_ignore(&e.path)).collect();
                    let child_ids: Vec<EntryId> = filtered.iter().map(|e| e.id).collect();
                    let snapshots: Vec<_> = filtered.iter().map(|e| e.to_snapshot(self.root_path.as_deref().unwrap_or(Path::new("")), self.selected_id)).collect();
                    for e in filtered { self.entries.insert(e.id, e); }
                    if let Some(parent) = self.entries.get_mut(&entry_id) {
                        parent.kind = EntryKind::Dir;
                        parent.children = child_ids.clone();
                    }
                    let _ = self.event_tx.send(IdeEvent::TreeEntryExpanded { entry_id, children: snapshots });

                    // Zed-isomorphic: auto-expand all first-level directories after root loads
                    let is_root = self.root_id == Some(entry_id);
                    if is_root && self.auto_expand_root_children {
                        self.auto_expand_root_children = false;
                        for &child_id in &child_ids {
                            if let Some(child) = self.entries.get(&child_id) {
                                if matches!(child.kind, EntryKind::Dir | EntryKind::UnloadedDir) {
                                    // Queue expand for this child directory
                                    if let Some(e) = self.entries.get_mut(&child_id) {
                                        e.is_expanded = true;
                                        self.expanded_ids.insert(child_id);
                                        if e.kind == EntryKind::UnloadedDir {
                                            e.kind = EntryKind::PendingDir;
                                            let path = e.path.clone();
                                            let _ = self.scan_tx.send(ScanRequest::LoadDir { entry_id: child_id, path });
                                            let _ = self.event_tx.send(IdeEvent::TreeLoading { entry_id: child_id });
                                        }
                                    }
                                }
                            }
                        }
                    }

                    self.emit_visible_entries();
                }
                ScanResult::Error { entry_id, error } => {
                    if let Some(e) = self.entries.get_mut(&entry_id) { e.kind = EntryKind::Dir; e.children.clear(); }
                    let _ = self.event_tx.send(IdeEvent::Error { message: format!("Failed to load {}: {}", entry_id.0, error) });
                }
            }
        }
    }

    fn should_ignore(&self, path: &Path) -> bool {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        name.starts_with('.') || self.ignore_patterns.iter().any(|p| name == p)
    }

    pub fn visible_entries(&self) -> Vec<FileEntrySnapshot> {
        let Some(root_id) = self.root_id else { return vec![]; };
        let root_path = self.root_path.as_deref().unwrap_or(Path::new(""));
        let mut result = Vec::new();
        self.collect_visible(root_id, root_path, &mut result);
        result
    }

    fn collect_visible(&self, entry_id: EntryId, root: &Path, out: &mut Vec<FileEntrySnapshot>) {
        let Some(entry) = self.entries.get(&entry_id) else { return; };
        out.push(entry.to_snapshot(root, self.selected_id));
        if entry.is_expanded {
            for &child_id in &entry.children { self.collect_visible(child_id, root, out); }
        }
    }

    fn emit_visible_entries(&self) {
        let entries = self.visible_entries();
        let _ = self.event_tx.send(IdeEvent::TreeEntriesUpdated { entries, parent_id: self.root_id });
    }

    pub fn root_path(&self) -> Option<&Path> { self.root_path.as_deref() }
    pub fn selected_id(&self) -> Option<EntryId> { self.selected_id }
    pub fn get_entry(&self, id: EntryId) -> Option<&Entry> { self.entries.get(&id) }
}
