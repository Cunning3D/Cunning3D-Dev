//! Code block component with copy button and language label.
use gpui::{AnyElement, ClipboardItem, ElementId, IntoElement, ParentElement, Styled, div, px, prelude::*};
use crate::ai_workspace_gpui::ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Button, ButtonStyle, Spacing, UiMetrics};
use crate::ai_workspace_gpui::ide::{HighlightKind, SyntaxSnapshot, Span, kinds, detect_language, highlight};
use std::path::Path;

pub struct CodeBlock {
    id: usize,
    language: Option<String>,
    code: String,
}

impl CodeBlock {
    pub fn new(id: usize, code: impl Into<String>) -> Self {
        Self { id, language: None, code: code.into() }
    }

    pub fn language(mut self, lang: impl Into<String>) -> Self {
        self.language = Some(lang.into());
        self
    }
}

impl IntoElement for CodeBlock {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let code_for_copy = self.code.clone();
        let id = self.id;
        let syntax = self
            .language
            .as_deref()
            .and_then(|h| detect_language(Path::new(""), Some(h)))
            .and_then(|l| highlight(l, &self.code));

        v_flex()
            .id(ElementId::NamedInteger("code-block".into(), id as u64))
            .group("codeblock")
            .w_full()
            .my(Spacing::Base04.px())
            .bg(ThemeColors::bg_elevated())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_sm()
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .px(Spacing::Base06.px())
                    .py(Spacing::Base02.px())
                    .bg(ThemeColors::bg_secondary())
                    .border_b_1()
                    .border_color(ThemeColors::border())
                    .justify_between()
                    .child(Label::new(self.language.unwrap_or_else(|| "code".into())).size(LabelSize::XSmall).color(LabelColor::Muted))
                    .child(
                        Button::new(format!("copy-code-{id}"), "Copy")
                            .style(ButtonStyle::Ghost)
                            .on_click(move |_: &gpui::ClickEvent, _: &mut gpui::Window, cx: &mut gpui::App| cx.write_to_clipboard(ClipboardItem::new_string(code_for_copy.clone())))
                    )
            )
            .child(
                div()
                    .id(ElementId::NamedInteger("code-scroll".into(), id as u64))
                    .w_full()
                    .p(Spacing::Base04.px())
                    .overflow_x_scroll()
                    .text_size(px(UiMetrics::FONT_DEFAULT))
                    .text_color(ThemeColors::text_primary())
                    .child(render_code(&self.code, syntax.as_ref()))
            )
            .into_any_element()
    }
}

fn render_code(code: &str, syn: Option<&SyntaxSnapshot>) -> AnyElement {
    let lines: Vec<&str> = code.split('\n').collect();
    if let Some(syn) = syn {
        return v_flex()
            .w_full()
            .gap_0()
            .children(
                lines
                    .into_iter()
                    .enumerate()
                    .map(|(i, l)| render_line(l, syn.lines.get(i)))
                    .collect::<Vec<_>>(),
            )
            .into_any_element();
    }
    div().child(code.to_string()).into_any_element()
}

fn render_line(line: &str, spans: Option<&Vec<Span>>) -> AnyElement {
    let t = if line.is_empty() { " ".to_string() } else { line.to_string() };
    let Some(spans) = spans else { return div().child(t).into_any_element(); };
    if spans.is_empty() { return div().child(t).into_any_element(); }

    let mut parts: Vec<(String, gpui::Hsla)> = Vec::new();
    let mut pos = 0usize;

    for sp in spans.iter() {
        let mut ss = clamp_back(line, sp.range.start.min(line.len()));
        let mut ee = clamp_fwd(line, sp.range.end.min(line.len()));
        if ee <= ss { continue; }
        if ss > pos {
            let a = clamp_back(line, pos);
            if ss > a { parts.push((line.get(a..ss).unwrap_or("").to_string(), ThemeColors::text_primary())); }
        }
        parts.push((line.get(ss..ee).unwrap_or("").to_string(), color_for_kind(sp.kind)));
        pos = ee.max(pos);
    }
    if pos < line.len() { parts.push((line.get(clamp_back(line, pos)..).unwrap_or("").to_string(), ThemeColors::text_primary())); }

    h_flex()
        .gap_0()
        .children(
            parts
                .into_iter()
                .filter(|(x, _)| !x.is_empty())
                .map(|(x, c)| div().flex_none().text_color(c).child(x).into_any_element())
                .collect::<Vec<_>>(),
        )
        .into_any_element()
}

fn color_for_kind(k: HighlightKind) -> gpui::Hsla {
    if k == kinds::COMMENT { return ThemeColors::text_muted(); }
    if k == kinds::KEYWORD { return ThemeColors::text_accent(); }
    if k == kinds::STRING { return ThemeColors::text_success(); }
    if k == kinds::TYPE { return ThemeColors::text_warning(); }
    if k == kinds::FUNCTION { return ThemeColors::text_primary(); }
    if k == kinds::CONSTANT { return ThemeColors::text_warning(); }
    if k == kinds::NUMBER { return ThemeColors::text_warning(); }
    if k == kinds::OPERATOR { return ThemeColors::text_secondary(); }
    if k == kinds::PUNCTUATION { return ThemeColors::text_secondary(); }
    if k == kinds::VARIABLE { return ThemeColors::text_primary(); }
    ThemeColors::text_primary()
}

fn clamp_back(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) { i -= 1; }
    i
}

fn clamp_fwd(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) { i += 1; }
    i
}
