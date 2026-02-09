//! Code block component with copy button and language label.
use gpui::{AnyElement, ClipboardItem, ElementId, IntoElement, ParentElement, Styled, div, px, prelude::*};
use crate::ai_workspace_gpui::ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Button, ButtonStyle, Spacing};

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

        v_flex()
            .id(ElementId::NamedInteger("code-block".into(), id as u64))
            .group("codeblock")
            .w_full()
            .my(Spacing::Base04.px())
            .bg(ThemeColors::bg_elevated())
            .border_1()
            .border_color(ThemeColors::border())
            .rounded_md()
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
                    .p(Spacing::Base06.px())
                    .overflow_x_scroll()
                    .text_size(px(12.0))
                    .text_color(ThemeColors::text_primary())
                    .child(self.code)
            )
            .into_any_element()
    }
}
