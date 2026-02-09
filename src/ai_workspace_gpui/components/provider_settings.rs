//! ProviderSettings: Edit Gemini/OpenAI-compat provider settings (persisted to settings/ai/providers.json).
use crossbeam_channel::Sender;
use gpui::{AnyElement, App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled, Window, div, prelude::*, px};

use crate::ai_workspace_gpui::{
    protocol::{ProviderSnapshot, UiToHost},
    ui::{h_flex, v_flex, Button, ButtonStyle, Label, LabelColor, LabelSize, Spacing, TextInput, ThemeColors, TintColor},
};

pub struct ProviderSettings {
    focus_handle: FocusHandle,
    ui_tx: Sender<UiToHost>,

    gemini_key: Entity<TextInput>,
    gemini_pro: Entity<TextInput>,
    gemini_flash: Entity<TextInput>,
    gemini_image: Entity<TextInput>,

    openai_idx: Option<usize>, // index into openai_compat_profiles (0-based)
    openai_name: Entity<TextInput>,
    openai_base_url: Entity<TextInput>,
    openai_key: Entity<TextInput>,
    openai_models: Entity<TextInput>,
    openai_selected_model: Entity<TextInput>,
}

impl EventEmitter<DismissEvent> for ProviderSettings {}

impl ProviderSettings {
    pub fn new(
        providers: &[ProviderSnapshot],
        active_provider_idx: usize,
        ui_tx: Sender<UiToHost>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();

        let gemini = providers.get(0).cloned();
        let active = providers.get(active_provider_idx).cloned();
        let openai_idx = active_provider_idx.checked_sub(1);

        let gemini_key_ph = if gemini.as_ref().is_some_and(|g| g.has_api_key) {
            "Gemini API key (saved, hidden) — paste to replace"
        } else {
            "Gemini API key (stored in providers.json)"
        };
        let gemini_key = cx.new(|cx| TextInput::new(cx, gemini_key_ph).multiline(false));
        let gemini_pro = cx.new(|cx| {
            TextInput::new(cx, "Gemini model_pro (e.g. gemini-3-pro-preview)")
                .multiline(false)
                .on_submit(|_, _, _| {})
        });
        let gemini_flash = cx.new(|cx| TextInput::new(cx, "Gemini model_flash (optional)").multiline(false));
        let gemini_image = cx.new(|cx| TextInput::new(cx, "Gemini model_image (e.g. gemini-3-pro-image-preview)").multiline(false));

        if let Some(g) = gemini {
            gemini_pro.update(cx, |i, cx| i.set_text(g.selected_model.clone(), cx));
            if let Some(first) = g.models.get(0) {
                gemini_pro.update(cx, |i, cx| i.set_text(first.clone(), cx));
            }
            if let Some(second) = g.models.get(1) {
                gemini_flash.update(cx, |i, cx| i.set_text(second.clone(), cx));
            }
            if let Some(im) = g.image_model.clone().filter(|s| !s.trim().is_empty()) {
                gemini_image.update(cx, |i, cx| i.set_text(im, cx));
            }
        }

        let openai_name = cx.new(|cx| TextInput::new(cx, "OpenAI-compat profile name").multiline(false));
        let openai_base_url = cx.new(|cx| TextInput::new(cx, "Base URL (e.g. http://127.0.0.1:1234)").multiline(false));
        let openai_key_ph = if active.as_ref().is_some_and(|p| p.has_api_key) {
            "API key (saved, hidden) — paste to replace"
        } else {
            "API key (empty allowed)"
        };
        let openai_key = cx.new(|cx| TextInput::new(cx, openai_key_ph).multiline(false));
        let openai_models = cx.new(|cx| TextInput::new(cx, "Models (one per line)").multiline(true));
        let openai_selected_model = cx.new(|cx| TextInput::new(cx, "Selected model").multiline(false));

        if let Some(a) = active {
            openai_name.update(cx, |i, cx| i.set_text(a.name.clone(), cx));
            openai_base_url.update(cx, |i, cx| i.set_text(a.base_url.clone(), cx));
            openai_models.update(cx, |i, cx| i.set_text(a.models.join("\n"), cx));
            openai_selected_model.update(cx, |i, cx| i.set_text(a.selected_model.clone(), cx));
        }

        focus_handle.focus(window, cx);
        Self {
            focus_handle,
            ui_tx,
            gemini_key,
            gemini_pro,
            gemini_flash,
            gemini_image,
            openai_idx,
            openai_name,
            openai_base_url,
            openai_key,
            openai_models,
            openai_selected_model,
        }
    }

    fn parse_models(s: &str) -> Vec<String> {
        s.lines()
            .flat_map(|l| l.split(','))
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .map(|x| x.to_string())
            .collect()
    }

    fn save(&mut self, _: &gpui::ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let gemini_api_key = self.gemini_key.read(cx).text().trim().to_string();
        let gemini_pro = self.gemini_pro.read(cx).text().trim().to_string();
        let gemini_flash = self.gemini_flash.read(cx).text().trim().to_string();
        let gemini_image = self.gemini_image.read(cx).text().trim().to_string();
        let _ = self.ui_tx.send(UiToHost::UpdateGeminiSettings {
            api_key: gemini_api_key,
            model_pro: gemini_pro,
            model_flash: gemini_flash,
            model_image: gemini_image,
        });

        if let Some(idx) = self.openai_idx {
            let name = self.openai_name.read(cx).text().trim().to_string();
            let base_url = self.openai_base_url.read(cx).text().trim().to_string();
            let api_key = self.openai_key.read(cx).text().trim().to_string();
            let models = Self::parse_models(self.openai_models.read(cx).text());
            let selected_model = self.openai_selected_model.read(cx).text().trim().to_string();
            let _ = self.ui_tx.send(UiToHost::UpdateOpenAiCompatProfile {
                profile_idx: idx,
                name,
                base_url,
                api_key,
                models,
                selected_model,
            });
        }

        cx.emit(DismissEvent);
        cx.notify();
    }

    fn cancel(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn add_openai_profile(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let _ = self.ui_tx.send(UiToHost::AddOpenAiCompatProfile);
        cx.emit(DismissEvent);
    }

    fn delete_openai_profile(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(idx) = self.openai_idx else { return; };
        let _ = self.ui_tx.send(UiToHost::DeleteOpenAiCompatProfile { profile_idx: idx });
        cx.emit(DismissEvent);
    }

    fn section_header(title: &str) -> AnyElement {
        h_flex()
            .w_full()
            .justify_between()
            .child(Label::new(title.to_string()).size(LabelSize::Small).color(LabelColor::Primary))
            .into_any_element()
    }
}

impl Focusable for ProviderSettings {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ProviderSettings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_openai = self.openai_idx.is_some();
        v_flex()
            .id("provider-settings")
            .w(px(520.0))
            .gap(Spacing::Base08.px())
            .child(Self::section_header("Gemini"))
            .child(
                v_flex()
                    .gap(Spacing::Base06.px())
                    .child(Label::new("api_key").size(LabelSize::XSmall).color(LabelColor::Muted))
                    .child(div().p(Spacing::Base06.px()).bg(ThemeColors::bg_primary()).border_1().border_color(ThemeColors::border()).rounded_sm().child(self.gemini_key.clone()))
                    .child(Label::new("model_pro").size(LabelSize::XSmall).color(LabelColor::Muted))
                    .child(div().p(Spacing::Base06.px()).bg(ThemeColors::bg_primary()).border_1().border_color(ThemeColors::border()).rounded_sm().child(self.gemini_pro.clone()))
                    .child(Label::new("model_flash").size(LabelSize::XSmall).color(LabelColor::Muted))
                    .child(div().p(Spacing::Base06.px()).bg(ThemeColors::bg_primary()).border_1().border_color(ThemeColors::border()).rounded_sm().child(self.gemini_flash.clone()))
                    .child(Label::new("model_image").size(LabelSize::XSmall).color(LabelColor::Muted))
                    .child(div().p(Spacing::Base06.px()).bg(ThemeColors::bg_primary()).border_1().border_color(ThemeColors::border()).rounded_sm().child(self.gemini_image.clone()))
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(Label::new("OpenAI-compat (profiles)").size(LabelSize::Small).color(LabelColor::Primary))
                    .child(
                        h_flex()
                            .gap(Spacing::Base04.px())
                            .child(
                                Button::new("add-openai-profile", "+ Add")
                                    .style(ButtonStyle::Ghost)
                                    .on_click(cx.listener(Self::add_openai_profile)),
                            )
                            .when(has_openai, |d| {
                                d.child(
                                    Button::new("delete-openai-profile", "Delete")
                                        .style(ButtonStyle::Tinted(TintColor::Error))
                                        .on_click(cx.listener(Self::delete_openai_profile)),
                                )
                            }),
                    )
                    .into_any_element(),
            )
            .when(!has_openai, |d| {
                d.child(Label::new("No OpenAI-compat profile selected.").size(LabelSize::XSmall).color(LabelColor::Muted))
            })
            .when(has_openai, |d| {
                d.child(
                    v_flex()
                        .gap(Spacing::Base06.px())
                        .child(Label::new("name").size(LabelSize::XSmall).color(LabelColor::Muted))
                        .child(div().p(Spacing::Base06.px()).bg(ThemeColors::bg_primary()).border_1().border_color(ThemeColors::border()).rounded_sm().child(self.openai_name.clone()))
                        .child(Label::new("base_url").size(LabelSize::XSmall).color(LabelColor::Muted))
                        .child(div().p(Spacing::Base06.px()).bg(ThemeColors::bg_primary()).border_1().border_color(ThemeColors::border()).rounded_sm().child(self.openai_base_url.clone()))
                        .child(Label::new("api_key").size(LabelSize::XSmall).color(LabelColor::Muted))
                        .child(div().p(Spacing::Base06.px()).bg(ThemeColors::bg_primary()).border_1().border_color(ThemeColors::border()).rounded_sm().child(self.openai_key.clone()))
                        .child(Label::new("models").size(LabelSize::XSmall).color(LabelColor::Muted))
                        .child(div().h(px(110.0)).p(Spacing::Base06.px()).bg(ThemeColors::bg_primary()).border_1().border_color(ThemeColors::border()).rounded_sm().child(self.openai_models.clone()))
                        .child(Label::new("selected_model").size(LabelSize::XSmall).color(LabelColor::Muted))
                        .child(div().p(Spacing::Base06.px()).bg(ThemeColors::bg_primary()).border_1().border_color(ThemeColors::border()).rounded_sm().child(self.openai_selected_model.clone()))
                )
            })
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(Button::new("cancel-provider-settings", "Cancel").style(ButtonStyle::Ghost).on_click(cx.listener(Self::cancel)))
                    .child(Button::new("save-provider-settings", "Save").style(ButtonStyle::Tinted(TintColor::Accent)).on_click(cx.listener(Self::save)))
            )
    }
}

