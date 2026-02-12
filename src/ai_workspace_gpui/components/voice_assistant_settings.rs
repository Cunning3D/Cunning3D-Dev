//! VoiceAssistantSettings: Configure voice assistant wake phrases, commands, timeouts.
use crossbeam_channel::Sender;
use gpui::{App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled, Window, div, prelude::*, px};

use crate::ai_workspace_gpui::{
    protocol::{UiToHost, VoiceAssistantSettingsSnapshot, VoiceModel},
    ui::{h_flex, v_flex, Button, ButtonStyle, Label, LabelColor, LabelSize, Spacing, TextInput, ThemeColors, TintColor},
};

pub struct VoiceAssistantSettings {
    focus_handle: FocusHandle,
    ui_tx: Sender<UiToHost>,

    voice_model: VoiceModel,
    wake_phrases: Entity<TextInput>,
    cmd_input_phrases: Entity<TextInput>,
    cmd_send_phrases: Entity<TextInput>,
    cmd_cancel_phrases: Entity<TextInput>,
    greet_text: Entity<TextInput>,
    sleep_text: Entity<TextInput>,
    idle_timeout_secs: Entity<TextInput>,
    auto_send_pause_secs: Entity<TextInput>,
}

impl EventEmitter<DismissEvent> for VoiceAssistantSettings {}

impl VoiceAssistantSettings {
    pub fn new(
        settings: &VoiceAssistantSettingsSnapshot,
        ui_tx: Sender<UiToHost>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();

        let voice_model = if settings.enabled {
            if settings.use_gemini_live {
                VoiceModel::GeminiLive
            } else {
                VoiceModel::Legacy
            }
        } else {
            VoiceModel::Off
        };

        let wake_phrases = cx.new(|cx| {
            let mut input = TextInput::new(cx, "Wake phrases (separated by |)").multiline(false);
            input.set_text(settings.wake_phrases.clone(), cx);
            input
        });
        let cmd_input_phrases = cx.new(|cx| {
            let mut input = TextInput::new(cx, "Input command phrases").multiline(false);
            input.set_text(settings.cmd_input_phrases.clone(), cx);
            input
        });
        let cmd_send_phrases = cx.new(|cx| {
            let mut input = TextInput::new(cx, "Send command phrases").multiline(false);
            input.set_text(settings.cmd_send_phrases.clone(), cx);
            input
        });
        let cmd_cancel_phrases = cx.new(|cx| {
            let mut input = TextInput::new(cx, "Cancel command phrases").multiline(false);
            input.set_text(settings.cmd_cancel_phrases.clone(), cx);
            input
        });
        let greet_text = cx.new(|cx| {
            let mut input = TextInput::new(cx, "Greeting text (after wake)").multiline(false);
            input.set_text(settings.greet_text.clone(), cx);
            input
        });
        let sleep_text = cx.new(|cx| {
            let mut input = TextInput::new(cx, "Sleep text (on timeout)").multiline(false);
            input.set_text(settings.sleep_text.clone(), cx);
            input
        });
        let idle_timeout_secs = cx.new(|cx| {
            let mut input = TextInput::new(cx, "Idle timeout (seconds)").multiline(false);
            input.set_text(settings.idle_timeout_secs.to_string(), cx);
            input
        });
        let auto_send_pause_secs = cx.new(|cx| {
            let mut input = TextInput::new(cx, "Auto-send pause (seconds)").multiline(false);
            input.set_text(settings.auto_send_pause_secs.to_string(), cx);
            input
        });

        focus_handle.focus(window, cx);
        Self {
            focus_handle,
            ui_tx,
            voice_model,
            wake_phrases,
            cmd_input_phrases,
            cmd_send_phrases,
            cmd_cancel_phrases,
            greet_text,
            sleep_text,
            idle_timeout_secs,
            auto_send_pause_secs,
        }
    }

    fn save(&mut self, _: &gpui::ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let wake_phrases = self.wake_phrases.read(cx).text().trim().to_string();
        let cmd_input_phrases = self.cmd_input_phrases.read(cx).text().trim().to_string();
        let cmd_send_phrases = self.cmd_send_phrases.read(cx).text().trim().to_string();
        let cmd_cancel_phrases = self.cmd_cancel_phrases.read(cx).text().trim().to_string();
        let greet_text = self.greet_text.read(cx).text().trim().to_string();
        let sleep_text = self.sleep_text.read(cx).text().trim().to_string();
        let idle_timeout_secs = self.idle_timeout_secs.read(cx).text().trim().parse().unwrap_or(10);
        let auto_send_pause_secs = self.auto_send_pause_secs.read(cx).text().trim().parse().unwrap_or(3);

        let (enabled, use_gemini_live) = match self.voice_model {
            VoiceModel::Off => (false, false),
            VoiceModel::Legacy => (true, false),
            VoiceModel::GeminiLive => (true, true),
        };

        let _ = self.ui_tx.send(UiToHost::UpdateVoiceAssistantSettings {
            enabled,
            use_gemini_live,
            wake_phrases,
            cmd_input_phrases,
            cmd_send_phrases,
            cmd_cancel_phrases,
            greet_text,
            sleep_text,
            idle_timeout_secs,
            auto_send_pause_secs,
        });

        cx.emit(DismissEvent);
        cx.notify();
    }

    fn cancel(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn set_voice_off(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.voice_model = VoiceModel::Off;
        cx.notify();
    }

    fn set_voice_legacy(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.voice_model = VoiceModel::Legacy;
        cx.notify();
    }

    fn set_voice_gemini_live(&mut self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.voice_model = VoiceModel::GeminiLive;
        cx.notify();
    }

    fn field_row(label: &str, input: Entity<TextInput>) -> impl IntoElement {
        v_flex()
            .gap(Spacing::Base02.px())
            .child(Label::new(label.to_string()).size(LabelSize::XSmall).color(LabelColor::Muted))
            .child(
                div()
                    .p(Spacing::Base06.px())
                    .bg(ThemeColors::bg_primary())
                    .border_1()
                    .border_color(ThemeColors::border())
                    .rounded_sm()
                    .child(input),
            )
    }
}

impl Focusable for VoiceAssistantSettings {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for VoiceAssistantSettings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let off_style = if matches!(self.voice_model, VoiceModel::Off) {
            ButtonStyle::Tinted(TintColor::Accent)
        } else {
            ButtonStyle::Ghost
        };
        let legacy_style = if matches!(self.voice_model, VoiceModel::Legacy) {
            ButtonStyle::Tinted(TintColor::Accent)
        } else {
            ButtonStyle::Ghost
        };
        let gemini_live_style = if matches!(self.voice_model, VoiceModel::GeminiLive) {
            ButtonStyle::Tinted(TintColor::Success)
        } else {
            ButtonStyle::Ghost
        };

        v_flex()
            .id("voice-assistant-settings")
            .w(px(480.0))
            .gap(Spacing::Base08.px())
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(Label::new("Voice Assistant Settings").size(LabelSize::Large).color(LabelColor::Primary))
                    .child(
                        h_flex()
                            .gap(Spacing::Base04.px())
                            .child(
                                Button::new("voice-off", "Off")
                                    .style(off_style)
                                    .on_click(cx.listener(Self::set_voice_off)),
                            )
                            .child(
                                Button::new("voice-legacy", "Legacy (Whisper + TTS)")
                                    .style(legacy_style)
                                    .on_click(cx.listener(Self::set_voice_legacy)),
                            )
                            .child(
                                Button::new("voice-gemini-live", "Gemini Live")
                                    .style(gemini_live_style)
                                    .on_click(cx.listener(Self::set_voice_gemini_live)),
                            ),
                    ),
            )
            .child(Self::field_row("Wake Phrases (e.g. hello gemini|hi gemini|你好)", self.wake_phrases.clone()))
            .child(Self::field_row("Input Command (e.g. start dictation|输入)", self.cmd_input_phrases.clone()))
            .child(Self::field_row("Send Command (e.g. send|发送)", self.cmd_send_phrases.clone()))
            .child(Self::field_row("Cancel Command (e.g. cancel|取消)", self.cmd_cancel_phrases.clone()))
            .child(Self::field_row("Greeting Text", self.greet_text.clone()))
            .child(Self::field_row("Sleep Text", self.sleep_text.clone()))
            .child(
                h_flex()
                    .gap(Spacing::Base08.px())
                    .child(
                        v_flex()
                            .flex_1()
                            .gap(Spacing::Base02.px())
                            .child(Label::new("Idle Timeout (sec)").size(LabelSize::XSmall).color(LabelColor::Muted))
                            .child(
                                div()
                                    .p(Spacing::Base06.px())
                                    .bg(ThemeColors::bg_primary())
                                    .border_1()
                                    .border_color(ThemeColors::border())
                                    .rounded_sm()
                                    .child(self.idle_timeout_secs.clone()),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .gap(Spacing::Base02.px())
                            .child(Label::new("Auto-send Pause (sec)").size(LabelSize::XSmall).color(LabelColor::Muted))
                            .child(
                                div()
                                    .p(Spacing::Base06.px())
                                    .bg(ThemeColors::bg_primary())
                                    .border_1()
                                    .border_color(ThemeColors::border())
                                    .rounded_sm()
                                    .child(self.auto_send_pause_secs.clone()),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(
                        Button::new("cancel-voice-settings", "Cancel")
                            .style(ButtonStyle::Ghost)
                            .on_click(cx.listener(Self::cancel)),
                    )
                    .child(
                        Button::new("save-voice-settings", "Save")
                            .style(ButtonStyle::Tinted(TintColor::Accent))
                            .on_click(cx.listener(Self::save)),
                    ),
            )
    }
}
