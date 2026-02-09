//! Small line-diff model for UI rendering (clamped for performance).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffLineKind { Context, Added, Removed }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub line_num_old: Option<usize>,
    pub line_num_new: Option<usize>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub file_path: String,
    pub hunks: Vec<DiffHunk>,
}

pub fn compute_file_diff(file_path: impl Into<String>, old: &str, new: &str) -> Option<FileDiff> {
    if old == new {
        return None;
    }
    use similar::{ChangeTag, TextDiff};
    const CTX: usize = 3;
    const MAX_HUNKS: usize = 6;
    const MAX_LINES: usize = 320;

    let diff = TextDiff::from_lines(old, new);
    let mut hunks = Vec::new();
    let mut total = 0usize;
    for group in diff.grouped_ops(CTX).into_iter().take(MAX_HUNKS) {
        let Some(first) = group.first() else { continue; };
        let Some(last) = group.last() else { continue; };
        let old_start0 = first.old_range().start;
        let old_end0 = last.old_range().end;
        let new_start0 = first.new_range().start;
        let new_end0 = last.new_range().end;
        let (mut o, mut n) = (old_start0 + 1, new_start0 + 1);
        let mut lines = Vec::new();
        for op in group {
            for ch in diff.iter_changes(&op) {
                if total >= MAX_LINES {
                    lines.push(DiffLine { kind: DiffLineKind::Context, line_num_old: None, line_num_new: None, content: "... diff truncated ...".into() });
                    break;
                }
                let s = ch.to_string_lossy().trim_end_matches('\n').to_string();
                match ch.tag() {
                    ChangeTag::Equal => { lines.push(DiffLine { kind: DiffLineKind::Context, line_num_old: Some(o), line_num_new: Some(n), content: s }); o += 1; n += 1; }
                    ChangeTag::Delete => { lines.push(DiffLine { kind: DiffLineKind::Removed, line_num_old: Some(o), line_num_new: None, content: s }); o += 1; }
                    ChangeTag::Insert => { lines.push(DiffLine { kind: DiffLineKind::Added, line_num_old: None, line_num_new: Some(n), content: s }); n += 1; }
                }
                total += 1;
            }
            if total >= MAX_LINES { break; }
        }
        hunks.push(DiffHunk {
            old_start: old_start0 + 1,
            old_count: old_end0.saturating_sub(old_start0),
            new_start: new_start0 + 1,
            new_count: new_end0.saturating_sub(new_start0),
            lines,
        });
        if total >= MAX_LINES { break; }
    }
    Some(FileDiff { file_path: file_path.into(), hunks })
}

