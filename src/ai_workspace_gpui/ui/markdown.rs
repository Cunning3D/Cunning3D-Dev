//! Simple Markdown renderer for chat messages (code blocks, bold, italic, links).
use gpui::{AnyElement, IntoElement, ParentElement, SharedString, Styled, div, prelude::*, px};
use super::{v_flex, ThemeColors, Label, LabelColor, LabelSize, Spacing};

// ─────────────────────────────────────────────────────────────────────────────
// Markdown Parser
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum MarkdownBlock {
    Paragraph(Vec<MarkdownSpan>),
    CodeBlock { language: Option<String>, code: String },
    Heading { level: u8, text: String },
    List { ordered: bool, items: Vec<String> },
    Quote(String),
    HorizontalRule,
}

#[derive(Debug, Clone)]
pub enum MarkdownSpan {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
    Link { text: String, url: String },
}

pub fn parse_markdown(text: &str) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        // Code block
        if line.starts_with("```") {
            let language = line.strip_prefix("```").map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            let mut code = String::new();
            while let Some(code_line) = lines.next() {
                if code_line.starts_with("```") { break; }
                if !code.is_empty() { code.push('\n'); }
                code.push_str(code_line);
            }
            blocks.push(MarkdownBlock::CodeBlock { language, code });
            continue;
        }

        // Heading
        if line.starts_with('#') {
            let level = line.chars().take_while(|&c| c == '#').count() as u8;
            let text = line.trim_start_matches('#').trim().to_string();
            blocks.push(MarkdownBlock::Heading { level: level.min(6), text });
            continue;
        }

        // Horizontal rule
        if line.chars().all(|c| c == '-' || c == '*' || c == '_' || c.is_whitespace()) && line.chars().filter(|&c| c == '-' || c == '*' || c == '_').count() >= 3 {
            blocks.push(MarkdownBlock::HorizontalRule);
            continue;
        }

        // Quote
        if line.starts_with('>') {
            let quote_text = line.strip_prefix('>').unwrap_or(line).trim().to_string();
            blocks.push(MarkdownBlock::Quote(quote_text));
            continue;
        }

        // List item
        if line.trim_start().starts_with("- ") || line.trim_start().starts_with("* ") {
            let item = line.trim_start().strip_prefix("- ").or_else(|| line.trim_start().strip_prefix("* ")).unwrap_or(line).to_string();
            if let Some(MarkdownBlock::List { ordered: false, items }) = blocks.last_mut() {
                items.push(item);
            } else {
                blocks.push(MarkdownBlock::List { ordered: false, items: vec![item] });
            }
            continue;
        }

        // Numbered list
        if let Some(rest) = line.trim_start().strip_prefix(|c: char| c.is_ascii_digit()).and_then(|s| s.strip_prefix(". ")) {
            let item = rest.to_string();
            if let Some(MarkdownBlock::List { ordered: true, items }) = blocks.last_mut() {
                items.push(item);
            } else {
                blocks.push(MarkdownBlock::List { ordered: true, items: vec![item] });
            }
            continue;
        }

        // Regular paragraph
        if !line.trim().is_empty() {
            let spans = parse_inline(line);
            blocks.push(MarkdownBlock::Paragraph(spans));
        }
    }

    blocks
}

fn parse_inline(text: &str) -> Vec<MarkdownSpan> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '`' => {
                if !current.is_empty() { spans.push(MarkdownSpan::Text(std::mem::take(&mut current))); }
                let mut code = String::new();
                while let Some(&next) = chars.peek() {
                    if next == '`' { chars.next(); break; }
                    code.push(chars.next().unwrap());
                }
                spans.push(MarkdownSpan::Code(code));
            }
            '*' | '_' => {
                let is_double = chars.peek() == Some(&c);
                if is_double { chars.next(); }
                if !current.is_empty() { spans.push(MarkdownSpan::Text(std::mem::take(&mut current))); }
                let mut inner = String::new();
                let end_pattern = if is_double { format!("{}{}", c, c) } else { c.to_string() };
                while let Some(next) = chars.next() {
                    if inner.ends_with(&end_pattern.chars().next().unwrap().to_string()) && (end_pattern.len() == 1 || chars.peek() == Some(&c)) {
                        if end_pattern.len() > 1 { chars.next(); }
                        inner.pop();
                        break;
                    }
                    inner.push(next);
                }
                spans.push(if is_double { MarkdownSpan::Bold(inner) } else { MarkdownSpan::Italic(inner) });
            }
            '[' => {
                if !current.is_empty() { spans.push(MarkdownSpan::Text(std::mem::take(&mut current))); }
                let mut link_text = String::new();
                while let Some(next) = chars.next() {
                    if next == ']' { break; }
                    link_text.push(next);
                }
                if chars.peek() == Some(&'(') {
                    chars.next();
                    let mut url = String::new();
                    while let Some(next) = chars.next() {
                        if next == ')' { break; }
                        url.push(next);
                    }
                    spans.push(MarkdownSpan::Link { text: link_text, url });
                } else {
                    current.push('[');
                    current.push_str(&link_text);
                    current.push(']');
                }
            }
            _ => current.push(c),
        }
    }

    if !current.is_empty() { spans.push(MarkdownSpan::Text(current)); }
    spans
}

// ─────────────────────────────────────────────────────────────────────────────
// Markdown Renderer
// ─────────────────────────────────────────────────────────────────────────────

pub struct Markdown { blocks: Vec<MarkdownBlock> }

impl Markdown {
    pub fn new(text: &str) -> Self { Self { blocks: parse_markdown(text) } }
}

impl IntoElement for Markdown {
    type Element = <gpui::Div as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let children: Vec<AnyElement> = self.blocks.into_iter().map(|block| {
            match block {
                MarkdownBlock::Paragraph(spans) => {
                    let span_elements: Vec<AnyElement> = spans.into_iter().map(|span| {
                        match span {
                            MarkdownSpan::Text(t) => div().text_color(ThemeColors::text_primary()).child(t).into_any_element(),
                            MarkdownSpan::Bold(t) => div().font_weight(gpui::FontWeight::BOLD).text_color(ThemeColors::text_primary()).child(t).into_any_element(),
                            MarkdownSpan::Italic(t) => div().italic().text_color(ThemeColors::text_primary()).child(t).into_any_element(),
                            MarkdownSpan::Code(t) => div().px(Spacing::Base02.px()).py(px(1.0)).bg(ThemeColors::bg_elevated()).rounded_sm().text_color(ThemeColors::text_accent()).child(t).into_any_element(),
                            MarkdownSpan::Link { text, url: _ } => div().text_color(ThemeColors::text_accent()).cursor_pointer().child(text).into_any_element(),
                        }
                    }).collect();
                    div().flex().flex_wrap().gap(px(2.0)).children(span_elements).into_any_element()
                }
                MarkdownBlock::CodeBlock { language, code } => {
                    v_flex()
                        .w_full()
                        .my(Spacing::Base04.px())
                        .bg(ThemeColors::bg_elevated())
                        .border_1()
                        .border_color(ThemeColors::border())
                        .rounded_md()
                        .overflow_hidden()
                        .child(
                            div().w_full().px(Spacing::Base06.px()).py(Spacing::Base02.px()).bg(ThemeColors::bg_secondary()).border_b_1().border_color(ThemeColors::border())
                                .child(Label::new(language.unwrap_or_else(|| "code".into())).size(LabelSize::XSmall).color(LabelColor::Muted))
                        )
                        .child(
                            div().id("markdown-code-scroll").w_full().p(Spacing::Base06.px()).overflow_x_scroll()
                                .text_size(px(12.0))
                                .text_color(ThemeColors::text_primary())
                                .child(code)
                        )
                        .into_any_element()
                }
                MarkdownBlock::Heading { level, text } => {
                    let size = match level { 1 => px(24.0), 2 => px(20.0), 3 => px(18.0), _ => px(16.0) };
                    div().w_full().my(Spacing::Base04.px()).text_size(size).font_weight(gpui::FontWeight::BOLD).text_color(ThemeColors::text_primary()).child(text).into_any_element()
                }
                MarkdownBlock::List { ordered, items } => {
                    let list_items: Vec<AnyElement> = items.into_iter().enumerate().map(|(i, item)| {
                        div().flex().gap(Spacing::Base04.px())
                            .child(Label::new(if ordered { format!("{}.", i + 1) } else { "-".into() }).color(LabelColor::Muted))
                            .child(Label::new(item).color(LabelColor::Primary))
                            .into_any_element()
                    }).collect();
                    v_flex().w_full().my(Spacing::Base02.px()).gap(Spacing::Base02.px()).children(list_items).into_any_element()
                }
                MarkdownBlock::Quote(text) => {
                    div().w_full().my(Spacing::Base04.px()).pl(Spacing::Base08.px()).border_l_2().border_color(ThemeColors::border())
                        .text_color(ThemeColors::text_secondary())
                        .italic()
                        .child(text)
                        .into_any_element()
                }
                MarkdownBlock::HorizontalRule => {
                    div().w_full().h(px(1.0)).my(Spacing::Base08.px()).bg(ThemeColors::border()).into_any_element()
                }
            }
        }).collect();

        v_flex().w_full().gap(Spacing::Base02.px()).children(children).into_element()
    }
}
