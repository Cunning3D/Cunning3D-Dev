//! Document: Rope-backed text buffer with undo/redo (Zed-isomorphic)

use crate::ai_workspace_gpui::protocol::{IdeEvent, OpenFileSnapshot, TextEdit};
use crate::ai_workspace_gpui::ide::text_core::{TextCore, Transaction};
use crossbeam_channel::Sender;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────────────────────────────────────
// Document (single file buffer)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Document {
    pub path: PathBuf,
    pub core: TextCore,
    pub version: u64,
    pub is_dirty: bool,
    pub cursor_line: usize,
    pub cursor_col: usize,
    undo_stack: Vec<UndoEntry>,
    redo_stack: Vec<UndoEntry>,
    saved_version: u64,
}

#[derive(Debug, Clone)]
struct UndoEntry {
    version: u64,
    tx: Transaction,
}

impl Document {
    pub fn new(path: PathBuf, content: String) -> Self {
        Self {
            path, core: TextCore::new(&content), version: 1, is_dirty: false, cursor_line: 0, cursor_col: 0,
            undo_stack: vec![], redo_stack: vec![], saved_version: 1,
        }
    }

    pub fn apply_edits(&mut self, edits: &[TextEdit]) {
        let tx = self.core.apply_transaction(edits);
        self.version = self.core.version();
        self.undo_stack.push(UndoEntry { version: self.version, tx });
        self.redo_stack.clear();
        self.is_dirty = self.version != self.saved_version;
    }

    pub fn undo(&mut self) -> Option<Vec<TextEdit>> {
        let entry = self.undo_stack.pop()?;
        self.core.apply_inverse(&entry.tx.inverse);
        self.version = self.core.version();
        self.is_dirty = self.version != self.saved_version;
        self.redo_stack.push(entry.clone());
        Some(entry.tx.inverse)
    }

    pub fn redo(&mut self) -> Option<Vec<TextEdit>> {
        let entry = self.redo_stack.pop()?;
        let tx = self.core.apply_transaction(&entry.tx.edits);
        self.version = self.core.version();
        self.is_dirty = self.version != self.saved_version;
        self.undo_stack.push(UndoEntry { version: self.version, tx: tx.clone() });
        Some(tx.edits)
    }

    pub fn mark_saved(&mut self) { self.saved_version = self.version; self.is_dirty = false; }
    pub fn line_count(&self) -> usize { self.core.line_count() }

    pub fn to_string(&self) -> String { self.core.to_string() }
    pub fn len_bytes(&self) -> usize { self.core.len_bytes() }

    pub fn offset_to_line_col(&self, offset: usize) -> (usize, usize) {
        self.core.byte_to_line_col(offset)
    }

    pub fn line_col_to_offset(&self, line: usize, col: usize) -> usize {
        self.core.line_col_to_byte(line, col)
    }

    pub fn to_snapshot(&self) -> OpenFileSnapshot {
        OpenFileSnapshot { path: self.path.clone(), is_dirty: self.is_dirty, version: self.version, cursor_line: self.cursor_line, cursor_col: self.cursor_col }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DocumentStore (manages all open documents)
// ─────────────────────────────────────────────────────────────────────────────

pub struct DocumentStore {
    documents: HashMap<PathBuf, Document>,
    active_path: Option<PathBuf>,
    recent_files: Vec<PathBuf>,
    event_tx: Sender<IdeEvent>,
}

impl DocumentStore {
    pub fn new(event_tx: Sender<IdeEvent>) -> Self {
        Self { documents: HashMap::new(), active_path: None, recent_files: vec![], event_tx }
    }

    pub fn open(&mut self, path: &Path) -> Result<&Document, String> {
        if self.documents.contains_key(path) {
            self.set_active(path);
            return Ok(self.documents.get(path).unwrap());
        }
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let doc = Document::new(path.to_path_buf(), content.clone());
        self.documents.insert(path.to_path_buf(), doc);
        self.set_active(path);
        self.add_recent(path);
        Ok(self.documents.get(path).unwrap())
    }

    pub fn close(&mut self, path: &Path) -> bool {
        if self.documents.remove(path).is_some() {
            let _ = self.event_tx.send(IdeEvent::FileClosed { path: path.to_path_buf() });
            if self.active_path.as_deref() == Some(path) {
                self.active_path = self.documents.keys().next().cloned();
                let _ = self.event_tx.send(IdeEvent::ActiveFileChanged { path: self.active_path.clone() });
            }
            true
        } else { false }
    }

    pub fn close_all(&mut self) {
        let paths: Vec<_> = self.documents.keys().cloned().collect();
        for p in paths { self.close(&p); }
    }

    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        let doc = self.documents.get_mut(path).ok_or("File not open")?;
        std::fs::write(path, doc.to_string()).map_err(|e| e.to_string())?;
        doc.mark_saved();
        let version = doc.version;
        let _ = self.event_tx.send(IdeEvent::FileSaved { path: path.to_path_buf(), version });
        let _ = self.event_tx.send(IdeEvent::FileDirtyChanged { path: path.to_path_buf(), is_dirty: false });
        Ok(())
    }

    pub fn save_all(&mut self) -> Vec<String> {
        let paths: Vec<_> = self.documents.keys().filter(|p| self.documents.get(*p).map(|d| d.is_dirty).unwrap_or(false)).cloned().collect();
        let mut errors = vec![];
        for p in paths { if let Err(e) = self.save(&p) { errors.push(format!("{}: {}", p.display(), e)); } }
        errors
    }

    pub fn edit(&mut self, path: &Path, edits: Vec<TextEdit>) -> Result<u64, String> {
        let doc = self.documents.get_mut(path).ok_or("File not open")?;
        doc.apply_edits(&edits);
        let version = doc.version;
        let is_dirty = doc.is_dirty;
        let _ = self.event_tx.send(IdeEvent::FileChanged { path: path.to_path_buf(), version, edits });
        let _ = self.event_tx.send(IdeEvent::FileDirtyChanged { path: path.to_path_buf(), is_dirty });
        Ok(version)
    }

    /// Apply external on-disk content change as an undoable edit.
    /// This keeps the editor feeling snappy when tools patch files.
    pub fn sync_from_disk_as_edit(&mut self, path: &Path) -> Result<u64, String> {
        let new_content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let doc = self.documents.get_mut(path).ok_or("File not open")?;
        if doc.to_string() == new_content {
            return Ok(doc.version);
        }
        let old_len = doc.len_bytes();
        let edits = vec![TextEdit { start_offset: 0, end_offset: old_len, new_text: new_content }];
        self.edit(path, edits)
    }

    pub fn undo(&mut self, path: &Path) -> Result<u64, String> {
        let doc = self.documents.get_mut(path).ok_or("File not open")?;
        let edits = doc.undo().ok_or("Nothing to undo")?;
        let version = doc.version;
        let is_dirty = doc.is_dirty;
        let _ = self
            .event_tx
            .send(IdeEvent::FileChanged { path: path.to_path_buf(), version, edits });
        let _ = self
            .event_tx
            .send(IdeEvent::FileDirtyChanged { path: path.to_path_buf(), is_dirty });
        Ok(version)
    }

    pub fn redo(&mut self, path: &Path) -> Result<u64, String> {
        let doc = self.documents.get_mut(path).ok_or("File not open")?;
        let edits = doc.redo().ok_or("Nothing to redo")?;
        let version = doc.version;
        let is_dirty = doc.is_dirty;
        let _ = self
            .event_tx
            .send(IdeEvent::FileChanged { path: path.to_path_buf(), version, edits });
        let _ = self
            .event_tx
            .send(IdeEvent::FileDirtyChanged { path: path.to_path_buf(), is_dirty });
        Ok(version)
    }

    pub fn set_active(&mut self, path: &Path) {
        if self.documents.contains_key(path) {
            self.active_path = Some(path.to_path_buf());
            let _ = self.event_tx.send(IdeEvent::ActiveFileChanged { path: Some(path.to_path_buf()) });
            if let Some(doc) = self.documents.get(path) {
                let _ = self.event_tx.send(IdeEvent::FileOpened {
                    path: path.to_path_buf(),
                    content: doc.to_string(),
                    version: doc.version,
                });
            }
        }
    }

    pub fn set_cursor(&mut self, path: &Path, line: usize, col: usize) {
        if let Some(doc) = self.documents.get_mut(path) {
            doc.cursor_line = line.min(doc.line_count().saturating_sub(1));
            doc.cursor_col = col;
            let _ = self.event_tx.send(IdeEvent::CursorMoved { path: path.to_path_buf(), line: doc.cursor_line as u32, col: doc.cursor_col as u32 });
        }
    }

    pub fn rename_path(&mut self, from: &Path, to: &Path) -> Result<(), String> {
        if from == to {
            return Ok(());
        }
        let mut doc = self.documents.remove(from).ok_or("File not open")?;
        doc.path = to.to_path_buf();
        let content = doc.to_string();
        let version = doc.version;
        self.documents.insert(to.to_path_buf(), doc);

        if self.active_path.as_deref() == Some(from) {
            self.active_path = Some(to.to_path_buf());
        }
        self.recent_files.retain(|p| p != from);
        self.recent_files.retain(|p| p != to);
        self.recent_files.insert(0, to.to_path_buf());

        let _ = self.event_tx.send(IdeEvent::FileClosed { path: from.to_path_buf() });
        let _ = self.event_tx.send(IdeEvent::FileOpened { path: to.to_path_buf(), content, version });
        let _ = self.event_tx.send(IdeEvent::ActiveFileChanged { path: self.active_path.clone() });
        Ok(())
    }

    fn add_recent(&mut self, path: &Path) {
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_path_buf());
        if self.recent_files.len() > 20 { self.recent_files.truncate(20); }
    }

    pub fn get(&self, path: &Path) -> Option<&Document> { self.documents.get(path) }
    pub fn get_mut(&mut self, path: &Path) -> Option<&mut Document> { self.documents.get_mut(path) }
    pub fn active(&self) -> Option<&Document> { self.active_path.as_ref().and_then(|p| self.documents.get(p)) }
    pub fn active_path(&self) -> Option<&Path> { self.active_path.as_deref() }
    pub fn open_files(&self) -> Vec<OpenFileSnapshot> { self.documents.values().map(|d| d.to_snapshot()).collect() }
    pub fn recent_files(&self) -> &[PathBuf] { &self.recent_files }
    pub fn is_open(&self, path: &Path) -> bool { self.documents.contains_key(path) }

    /// Reload an already-open document from disk and emit IdeEvent::FileChanged.
    /// This makes tool-based file edits immediately visible in the editor and undoable.
    pub fn reload_from_disk_if_open(&mut self, path: &Path) -> Result<(), String> {
        if !self.documents.contains_key(path) {
            return Ok(());
        }
        let old = self.documents.get(path).map(|d| d.to_string()).unwrap_or_default();
        let new = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        if old == new {
            return Ok(());
        }
        let doc = self.documents.get_mut(path).ok_or("File not open")?;
        let edits = vec![TextEdit { start_offset: 0, end_offset: old.len(), new_text: new }];
        doc.apply_edits(&edits);
        // Disk is already updated (tool wrote it). Treat this as clean while keeping undo available.
        doc.mark_saved();
        let version = doc.version;
        let _ = self.event_tx.send(IdeEvent::FileChanged { path: path.to_path_buf(), version, edits });
        let _ = self.event_tx.send(IdeEvent::FileDirtyChanged { path: path.to_path_buf(), is_dirty: false });
        Ok(())
    }
}
