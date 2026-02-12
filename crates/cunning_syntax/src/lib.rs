//! Shared syntax highlighting (tree-sitter, compiled-in languages).

use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::{collections::hash_map::DefaultHasher, hash::Hasher, ops::Range, path::Path};
use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LanguageId {
    Rust,
    Wgsl,
    Markdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct HighlightKind(pub u16);

pub mod kinds {
    use super::HighlightKind;
    pub const COMMENT: HighlightKind = HighlightKind(1);
    pub const KEYWORD: HighlightKind = HighlightKind(2);
    pub const STRING: HighlightKind = HighlightKind(3);
    pub const TYPE: HighlightKind = HighlightKind(4);
    pub const FUNCTION: HighlightKind = HighlightKind(5);
    pub const CONSTANT: HighlightKind = HighlightKind(6);
    pub const NUMBER: HighlightKind = HighlightKind(7);
    pub const OPERATOR: HighlightKind = HighlightKind(8);
    pub const PUNCTUATION: HighlightKind = HighlightKind(9);
    pub const VARIABLE: HighlightKind = HighlightKind(10);
}

#[derive(Debug, Clone)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: HighlightKind,
}

#[derive(Debug, Clone)]
pub struct SyntaxSnapshot {
    pub lines: Vec<Vec<Span>>,
}

#[derive(Debug, Clone)]
pub struct SyntaxViewport {
    pub start_line: usize,
    pub lines: Vec<Vec<Span>>,
}

pub fn detect_language(path: &Path, hint: Option<&str>) -> Option<LanguageId> {
    let hint = hint.unwrap_or("").trim().to_lowercase();
    if !hint.is_empty() {
        return match hint.as_str() {
            "rs" | "rust" => Some(LanguageId::Rust),
            "wgsl" => Some(LanguageId::Wgsl),
            "md" | "markdown" | "mdx" => Some(LanguageId::Markdown),
            _ => None,
        };
    }
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
        "rs" => Some(LanguageId::Rust),
        "wgsl" => Some(LanguageId::Wgsl),
        "md" | "markdown" | "mdx" => Some(LanguageId::Markdown),
        _ => None,
    }
}

pub fn highlight_for_path(path: &Path, source: &str) -> Option<SyntaxSnapshot> {
    const MAX_HL_BYTES: usize = 1_000_000;
    const MAX_HL_LINES: usize = 80_000;
    if source.len() > MAX_HL_BYTES
        || source
            .as_bytes()
            .iter()
            .take(4_000_000)
            .filter(|&&b| b == b'\n')
            .count()
            > MAX_HL_LINES
    {
        return None;
    }
    highlight(detect_language(path, None)?, source)
}

pub fn highlight(lang: LanguageId, source: &str) -> Option<SyntaxSnapshot> {
    static CACHE: Lazy<DashMap<(LanguageId, u64), SyntaxSnapshot>> = Lazy::new(DashMap::new);
    let mut h = DefaultHasher::new();
    h.write(source.as_bytes());
    let key = (lang, h.finish());
    if let Some(x) = CACHE.get(&key) {
        return Some(x.clone());
    }
    let out = highlight_uncached(lang, source)?;
    if CACHE.len() > 96 {
        CACHE.clear();
    }
    CACHE.insert(key, out.clone());
    Some(out)
}

pub fn highlight_viewport(lang: LanguageId, source: &str, start_line: usize, end_line_exclusive: usize) -> Option<SyntaxViewport> {
    static CACHE: Lazy<DashMap<(LanguageId, u64, usize, usize), SyntaxViewport>> = Lazy::new(DashMap::new);
    let mut h = DefaultHasher::new();
    h.write(source.as_bytes());
    let key = (lang, h.finish(), start_line, end_line_exclusive);
    if let Some(x) = CACHE.get(&key) {
        return Some(x.clone());
    }
    let out = highlight_viewport_uncached(lang, source, start_line, end_line_exclusive)?;
    if CACHE.len() > 128 { CACHE.clear(); }
    CACHE.insert(key, out.clone());
    Some(out)
}

fn highlight_uncached(lang: LanguageId, source: &str) -> Option<SyntaxSnapshot> {
    let cfgs = &*CONFIGS;
    let (cfg, inj) = match lang {
        LanguageId::Rust => (cfgs.rust.as_ref()?, InjectionMode::Rust),
        LanguageId::Wgsl => (cfgs.wgsl.as_ref()?, InjectionMode::Wgsl),
        LanguageId::Markdown => (cfgs.markdown.as_ref()?, InjectionMode::Markdown),
    };

    let line_starts = build_line_starts(source);
    let mut lines: Vec<Vec<Span>> = (0..line_starts.len()).map(|_| Vec::new()).collect();
    let mut hi = Highlighter::default();
    let mut stack: Vec<Highlight> = Vec::new();

    let iter = hi
        .highlight(cfg, source.as_bytes(), None, |s| inj.resolve(cfgs, s))
        .ok()?;
    for ev in iter {
        let Ok(ev) = ev else { continue; };
        match ev {
            HighlightEvent::HighlightStart(h) => stack.push(h),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let Some(top) = stack.last().copied() else { continue; };
                let Some(name) = cfgs.names.get(top.0) else { continue; };
                let Some(kind) = kind_from_capture(name) else { continue; };
                push_span(source, &line_starts, &mut lines, start, end, kind);
            }
        }
    }
    Some(SyntaxSnapshot { lines })
}

fn highlight_viewport_uncached(lang: LanguageId, source: &str, start_line: usize, end_line_exclusive: usize) -> Option<SyntaxViewport> {
    let cfgs = &*CONFIGS;
    let (cfg, inj) = match lang {
        LanguageId::Rust => (cfgs.rust.as_ref()?, InjectionMode::Rust),
        LanguageId::Wgsl => (cfgs.wgsl.as_ref()?, InjectionMode::Wgsl),
        LanguageId::Markdown => (cfgs.markdown.as_ref()?, InjectionMode::Markdown),
    };

    let line_starts = build_line_starts(source);
    let max_line = line_starts.len();
    let start_line = start_line.min(max_line);
    let end_line_exclusive = end_line_exclusive.min(max_line).max(start_line);
    let mut lines: Vec<Vec<Span>> = (0..end_line_exclusive.saturating_sub(start_line)).map(|_| Vec::new()).collect();

    let mut hi = Highlighter::default();
    let mut stack: Vec<Highlight> = Vec::new();
    let iter = hi
        .highlight(cfg, source.as_bytes(), None, |s| inj.resolve(cfgs, s))
        .ok()?;
    for ev in iter {
        let Ok(ev) = ev else { continue; };
        match ev {
            HighlightEvent::HighlightStart(h) => stack.push(h),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let Some(top) = stack.last().copied() else { continue; };
                let Some(name) = cfgs.names.get(top.0) else { continue; };
                let Some(kind) = kind_from_capture(name) else { continue; };
                push_span_viewport(source, &line_starts, start_line, end_line_exclusive, &mut lines, start, end, kind);
            }
        }
    }

    Some(SyntaxViewport {
        start_line,
        lines,
    })
}

struct Configs {
    names: [&'static str; 10],
    rust: Option<HighlightConfiguration>,
    wgsl: Option<HighlightConfiguration>,
    markdown: Option<HighlightConfiguration>,
    markdown_inline: Option<HighlightConfiguration>,
}

static CONFIGS: Lazy<Configs> = Lazy::new(|| {
    let names = [
        "comment",
        "keyword",
        "string",
        "type",
        "function",
        "constant",
        "number",
        "operator",
        "punctuation",
        "variable",
    ];

    let rust = match HighlightConfiguration::new(
        tree_sitter_rust::LANGUAGE.into(),
        "rust",
        tree_sitter_rust::HIGHLIGHTS_QUERY,
        tree_sitter_rust::INJECTIONS_QUERY,
        "",
    ) {
        Ok(mut cfg) => {
            cfg.configure(&names);
            Some(cfg)
        }
        Err(e) => {
            eprintln!("[cunning_syntax] rust highlight disabled: {e:?}");
            None
        }
    };

    let wgsl = match HighlightConfiguration::new(
        tree_sitter_wgsl::LANGUAGE.into(),
        "wgsl",
        tree_sitter_wgsl::HIGHLIGHTS_QUERY,
        "",
        "",
    ) {
        Ok(mut cfg) => {
            cfg.configure(&names);
            Some(cfg)
        }
        Err(e) => {
            eprintln!("[cunning_syntax] wgsl highlight disabled: {e:?}");
            None
        }
    };

    let markdown = match HighlightConfiguration::new(
        tree_sitter_markdown::LANGUAGE.into(),
        "markdown",
        tree_sitter_markdown::HIGHLIGHTS_QUERY,
        tree_sitter_markdown::INJECTIONS_QUERY,
        "",
    ) {
        Ok(mut cfg) => {
            cfg.configure(&names);
            Some(cfg)
        }
        Err(e) => {
            eprintln!("[cunning_syntax] markdown highlight disabled: {e:?}");
            None
        }
    };

    let markdown_inline = match HighlightConfiguration::new(
        tree_sitter_markdown::LANGUAGE_INLINE.into(),
        "markdown_inline",
        tree_sitter_markdown::HIGHLIGHTS_QUERY_INLINE,
        tree_sitter_markdown::INJECTIONS_QUERY_INLINE,
        "",
    ) {
        Ok(mut cfg) => {
            cfg.configure(&names);
            Some(cfg)
        }
        Err(e) => {
            eprintln!("[cunning_syntax] markdown_inline highlight disabled: {e:?}");
            None
        }
    };

    Configs {
        names,
        rust,
        wgsl,
        markdown,
        markdown_inline,
    }
});

#[derive(Clone, Copy)]
enum InjectionMode {
    Rust,
    Wgsl,
    Markdown,
}

impl InjectionMode {
    fn resolve<'a>(&self, cfgs: &'a Configs, lang: &str) -> Option<&'a HighlightConfiguration> {
        let l = lang.trim().to_lowercase();
        let l = l.as_str();
        match l {
            "rust" | "rs" => cfgs.rust.as_ref(),
            "wgsl" => cfgs.wgsl.as_ref(),
            "markdown" | "md" | "mdx" => cfgs.markdown.as_ref(),
            "markdown_inline" | "md_inline" | "markdown-inline" => cfgs.markdown_inline.as_ref(),
            _ => match self {
                InjectionMode::Markdown => None,
                _ => None,
            },
        }
    }
}

fn kind_from_capture(name: &str) -> Option<HighlightKind> {
    let head = name.split('.').next().unwrap_or(name);
    Some(match head {
        "comment" => kinds::COMMENT,
        "keyword" => kinds::KEYWORD,
        "string" => kinds::STRING,
        "type" => kinds::TYPE,
        "function" => kinds::FUNCTION,
        "constant" => kinds::CONSTANT,
        "number" => kinds::NUMBER,
        "operator" => kinds::OPERATOR,
        "punctuation" => kinds::PUNCTUATION,
        "variable" => kinds::VARIABLE,
        _ => return None,
    })
}

fn build_line_starts(source: &str) -> Vec<usize> {
    let b = source.as_bytes();
    let mut out = vec![0usize];
    for (i, x) in b.iter().enumerate() {
        if *x == b'\n' {
            out.push(i + 1);
        }
    }
    if out.is_empty() {
        out.push(0);
    }
    out
}

fn line_for_offset(starts: &[usize], off: usize) -> usize {
    match starts.binary_search(&off) {
        Ok(i) => i,
        Err(0) => 0,
        Err(i) => i - 1,
    }
}

fn push_span(
    source: &str,
    starts: &[usize],
    lines: &mut [Vec<Span>],
    start: usize,
    end: usize,
    kind: HighlightKind,
) {
    let bytes = source.as_bytes();
    let mut s = start.min(bytes.len());
    let end = end.min(bytes.len());
    while s < end {
        let li = line_for_offset(starts, s).min(lines.len().saturating_sub(1));
        let ls = *starts.get(li).unwrap_or(&0);
        let le = if li + 1 < starts.len() {
            starts[li + 1].saturating_sub(1)
        } else {
            bytes.len()
        };
        let e = end.min(le);
        if e > s && e >= ls {
            lines[li].push(Span {
                range: (s - ls)..(e - ls),
                kind,
            });
        }
        if e >= le && le < bytes.len() && bytes[le] == b'\n' {
            s = le + 1;
        } else {
            s = e;
        }
    }
}

fn push_span_viewport(
    source: &str,
    starts: &[usize],
    start_line: usize,
    end_line_exclusive: usize,
    lines: &mut [Vec<Span>],
    start: usize,
    end: usize,
    kind: HighlightKind,
) {
    let bytes = source.as_bytes();
    let mut s = start.min(bytes.len());
    let end = end.min(bytes.len());
    while s < end {
        let li = line_for_offset(starts, s);
        let ls = *starts.get(li).unwrap_or(&0);
        let le = if li + 1 < starts.len() {
            starts[li + 1].saturating_sub(1)
        } else {
            bytes.len()
        };
        let e = end.min(le);
        if li >= start_line && li < end_line_exclusive {
            let out_i = li - start_line;
            if let Some(out) = lines.get_mut(out_i) {
                if e > s && e >= ls {
                    out.push(Span {
                        range: (s - ls)..(e - ls),
                        kind,
                    });
                }
            }
        }
        if e >= le && le < bytes.len() && bytes[le] == b'\n' {
            s = le + 1;
        } else {
            s = e;
        }
    }
}

