//! TextCore: rope-backed text model with transactions.

use crate::ai_workspace_gpui::protocol::TextEdit;
use ropey::Rope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bias { Left, Right }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor { pub byte: usize, pub bias: Bias }

#[derive(Debug, Clone)]
pub struct Transaction { pub edits: Vec<TextEdit>, pub inverse: Vec<TextEdit> }

#[derive(Debug, Clone)]
pub struct TextCore { rope: Rope, version: u64 }

impl TextCore {
    pub fn new(text: &str) -> Self { Self { rope: Rope::from_str(text), version: 1 } }
    pub fn version(&self) -> u64 { self.version }
    pub fn len_bytes(&self) -> usize { self.rope.len_bytes() }
    pub fn to_string(&self) -> String { self.rope.to_string() }
    pub fn line_count(&self) -> usize { self.rope.len_lines().max(1) }

    pub fn byte_to_line_col(&self, byte: usize) -> (usize, usize) {
        let b = byte.min(self.len_bytes());
        let ch = self.rope.byte_to_char(b);
        let line = self.rope.char_to_line(ch);
        let col = ch.saturating_sub(self.rope.line_to_char(line));
        (line, col)
    }

    pub fn line_col_to_byte(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.rope.len_lines().saturating_sub(1));
        let start_ch = self.rope.line_to_char(line);
        let end_ch = self.rope.line_to_char((line + 1).min(self.rope.len_lines()));
        let ch = (start_ch + col).min(end_ch);
        self.rope.char_to_byte(ch)
    }

    pub fn apply_transaction(&mut self, edits: &[TextEdit]) -> Transaction {
        let mut inv: Vec<TextEdit> = Vec::with_capacity(edits.len());
        let mut delta: isize = 0;
        for e in edits {
            let s = ((e.start_offset as isize + delta).max(0) as usize).min(self.len_bytes());
            let t = ((e.end_offset as isize + delta).max(0) as usize).min(self.len_bytes());
            let (s, t) = if s <= t { (s, t) } else { (t, s) };
            let old = self.slice_bytes(s, t);
            self.replace_bytes(s, t, &e.new_text);
            let end = s + e.new_text.as_bytes().len();
            inv.push(TextEdit { start_offset: s, end_offset: end, new_text: old });
            delta += e.new_text.as_bytes().len() as isize - (t - s) as isize;
        }
        inv.reverse();
        self.version = self.version.saturating_add(1);
        Transaction { edits: edits.to_vec(), inverse: inv }
    }

    pub fn apply_inverse(&mut self, inv: &[TextEdit]) {
        for e in inv {
            let s = e.start_offset.min(self.len_bytes());
            let t = e.end_offset.min(self.len_bytes());
            self.replace_bytes(s.min(t), t.max(s), &e.new_text);
        }
        self.version = self.version.saturating_add(1);
    }

    pub fn translate_anchor(a: Anchor, edits: &[TextEdit]) -> Anchor {
        let mut b = a.byte;
        for e in edits {
            let s = e.start_offset;
            let t = e.end_offset.max(s);
            if b < s || (b == s && matches!(a.bias, Bias::Left)) { continue; }
            if b > t || (b == t && matches!(a.bias, Bias::Right)) {
                let d = e.new_text.as_bytes().len() as isize - (t - s) as isize;
                b = ((b as isize) + d).max(0) as usize;
            } else {
                b = s + e.new_text.as_bytes().len();
            }
        }
        Anchor { byte: b, bias: a.bias }
    }

    fn slice_bytes(&self, start: usize, end: usize) -> String {
        if start >= end { return String::new(); }
        let s = self.rope.byte_to_char(start);
        let e = self.rope.byte_to_char(end);
        self.rope.slice(s..e).to_string()
    }

    fn replace_bytes(&mut self, start: usize, end: usize, new_text: &str) {
        let s = self.rope.byte_to_char(start);
        let e = self.rope.byte_to_char(end);
        self.rope.remove(s..e);
        if !new_text.is_empty() { self.rope.insert(s, new_text); }
    }
}

