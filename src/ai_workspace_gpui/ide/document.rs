//! Document: Rope-based text buffer with undo/redo (Zed-isomorphic)

use crate::ai_workspace_gpui::protocol::{IdeEvent, OpenFileSnapshot, TextEdit};
use crossbeam_channel::Sender;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────────────────────────────────────
// Document (single file buffer)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Document {
    pub path: PathBuf,
    pub content: String,
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
    edits: Vec<TextEdit>,
    inverse: Vec<TextEdit>,
}

impl Document {
    pub fn new(path: PathBuf, content: String) -> Self {
        Self {
            path, content, version: 1, is_dirty: false, cursor_line: 0, cursor_col: 0,
            undo_stack: vec![], redo_stack: vec![], saved_version: 1,
        }
    }

    pub fn apply_edit(&mut self, edit: &TextEdit) -> TextEdit {
        let old_text = self.content[edit.start_offset..edit.end_offset].to_string();
        let inverse = TextEdit { start_offset: edit.start_offset, end_offset: edit.start_offset + edit.new_text.len(), new_text: old_text };
        self.content = format!("{}{}{}", &self.content[..edit.start_offset], &edit.new_text, &self.content[edit.end_offset..]);
        self.version += 1;
        self.is_dirty = self.version != self.saved_version;
        inverse
    }

    pub fn apply_edits(&mut self, edits: &[TextEdit]) {
        let mut inverses = Vec::new();
        let mut offset_delta: isize = 0;
        for edit in edits {
            let adjusted = TextEdit {
                start_offset: (edit.start_offset as isize + offset_delta) as usize,
                end_offset: (edit.end_offset as isize + offset_delta) as usize,
                new_text: edit.new_text.clone(),
            };
            let inv = self.apply_edit(&adjusted);
            offset_delta += edit.new_text.len() as isize - (edit.end_offset - edit.start_offset) as isize;
            inverses.push(inv);
        }
        inverses.reverse();
        self.undo_stack.push(UndoEntry { version: self.version, edits: edits.to_vec(), inverse: inverses });
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) -> Option<Vec<TextEdit>> {
        let entry = self.undo_stack.pop()?;
        for inv in &entry.inverse { self.apply_edit(inv); }
        self.redo_stack.push(entry.clone());
        Some(entry.inverse)
    }

    pub fn redo(&mut self) -> Option<Vec<TextEdit>> {
        let entry = self.redo_stack.pop()?;
        for edit in &entry.edits { self.apply_edit(edit); }
        self.undo_stack.push(entry.clone());
        Some(entry.edits)
    }

    pub fn mark_saved(&mut self) { self.saved_version = self.version; self.is_dirty = false; }
    pub fn line_count(&self) -> usize { self.content.lines().count().max(1) }

    pub fn line_at(&self, line: usize) -> &str {
        self.content.lines().nth(line).unwrap_or("")
    }

    pub fn offset_to_line_col(&self, offset: usize) -> (usize, usize) {
        let mut line = 0;
        let mut col = 0;
        for (i, ch) in self.content.char_indices() {
            if i >= offset { break; }
            if ch == '\n' { line += 1; col = 0; } else { col += 1; }
        }
        (line, col)
    }

    pub fn line_col_to_offset(&self, line: usize, col: usize) -> usize {
        let mut cur_line = 0;
        let mut cur_col = 0;
        for (i, ch) in self.content.char_indices() {
            if cur_line == line && cur_col == col { return i; }
            if ch == '\n' { cur_line += 1; cur_col = 0; if cur_line > line { return i; } }
            else { cur_col += 1; }
        }
        self.content.len()
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
        std::fs::write(path, &doc.content).map_err(|e| e.to_string())?;
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
                    content: doc.content.clone(),
                    version: doc.version,
                });
            }
        }
    }

    pub fn rename_path(&mut self, from: &Path, to: &Path) -> Result<(), String> {
        if from == to {
            return Ok(());
        }
        let mut doc = self.documents.remove(from).ok_or("File not open")?;
        doc.path = to.to_path_buf();
        let content = doc.content.clone();
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
}
