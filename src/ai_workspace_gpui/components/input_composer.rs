//! InputComposer: Message editor with TextInput, @mentions, /commands, and keyboard shortcuts.
use gpui::{actions, AnyElement, App, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Point, Render, Styled, Window, anchored, deferred, div, prelude::*, px};
use crossbeam_channel::Sender;
use std::path::PathBuf;
use crate::ai_workspace_gpui::{ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Button, ButtonStyle, TintColor, Spacing, TextInput, AutocompleteMenu, AutocompleteItem, AutocompleteMenuEvent, MentionKind}, protocol::{UiToHost, MentionSnapshot}};

actions!(input_composer, [Send, SendImmediate, Cancel, NewLine, AutocompleteNext, AutocompletePrev, AutocompleteConfirm]);

pub struct InputComposer {
    text_input: Entity<TextInput>,
    autocomplete: Option<Entity<AutocompleteMenu>>,
    mentions: Vec<MentionKind>,
    active_file: Option<String>,
    session_id: Option<uuid::Uuid>,
    ui_tx: Sender<UiToHost>,
    focus_handle: FocusHandle,
    is_busy: bool,
    is_voice_active: bool,
    voice_available: bool,
}

impl InputComposer {
    pub fn new(session_id: Option<uuid::Uuid>, ui_tx: Sender<UiToHost>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text_input = cx.new(|cx| {
            TextInput::new(cx, "Type @ to mention, / for commands... (Enter to send)")
                .multiline(true)
                .on_change(|text, window, cx| { /* Trigger autocomplete check in parent */ })
        });
        let focus_handle = cx.focus_handle();
        Self { text_input, autocomplete: None, mentions: Vec::new(), active_file: None, session_id, ui_tx, focus_handle, is_busy: false, is_voice_active: false, voice_available: false }
    }

    pub fn set_session(&mut self, session_id: Option<uuid::Uuid>) { self.session_id = session_id; }
    pub fn set_active_file(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        self.active_file = path.and_then(|p| p.to_str().map(|s| s.to_string()));
        cx.notify();
    }
    pub fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) { self.is_busy = busy; cx.notify(); }
    pub fn set_voice_active(&mut self, active: bool, cx: &mut Context<Self>) { self.is_voice_active = active; cx.notify(); }
    pub fn set_voice_available(&mut self, available: bool, cx: &mut Context<Self>) { self.voice_available = available; cx.notify(); }

    fn toggle_voice(&mut self, _: &gpui::ClickEvent, _: &mut Window, _cx: &mut Context<Self>) {
        if !self.voice_available {
            return;
        }
        if self.is_voice_active {
            let _ = self.ui_tx.send(UiToHost::StopVoice);
        } else {
            let _ = self.ui_tx.send(UiToHost::StartVoice);
        }
    }
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) { self.text_input.focus_handle(cx).focus(window, cx); }
    pub fn text(&self, cx: &App) -> String { self.text_input.read(cx).text().to_string() }
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) { self.text_input.update(cx, |i, cx| i.set_text(text.into(), cx)); }
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.text_input.update(cx, |input, cx| input.clear(cx));
        self.mentions.clear();
        self.autocomplete = None;
    }
    pub fn is_empty(&self, cx: &App) -> bool { self.text_input.read(cx).is_empty() }

    fn check_autocomplete(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let trigger = Self::find_trigger(&text);
        if let Some(query) = trigger {
            if self.autocomplete.is_none() {
                let menu = cx.new(|cx| AutocompleteMenu::new(&query, cx));
                cx.subscribe(&menu, |this, _, ev: &AutocompleteMenuEvent, cx| {
                    match ev {
                        AutocompleteMenuEvent::Confirm(item) => { this.insert_autocomplete_item(item, cx); this.autocomplete = None; }
                        AutocompleteMenuEvent::Dismiss => { this.autocomplete = None; }
                    }
                    cx.notify();
                }).detach();
                self.autocomplete = Some(menu);
            } else if let Some(ref menu) = self.autocomplete {
                menu.update(cx, |m, cx| m.update_query(&query, cx));
            }
        } else {
            self.autocomplete = None;
        }
        cx.notify();
    }

    fn find_trigger(text: &str) -> Option<String> {
        let chars: Vec<char> = text.chars().collect();
        for (i, &c) in chars.iter().enumerate().rev() {
            if c == '@' || c == '/' {
                if i == 0 || chars[i - 1].is_whitespace() {
                    return Some(chars[i..].iter().collect());
                }
            }
            if c.is_whitespace() { break; }
        }
        None
    }

    fn send(&mut self, _: &Send, _window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy || self.autocomplete.is_some() { return; }
        let Some(sid) = self.session_id else { return; };
        let text = self.text(cx);
        if text.trim().is_empty() { return; }
        let mentions = self.mentions.iter().map(|m| Self::to_snapshot(m)).collect();
        let _ = self.ui_tx.send(UiToHost::SendMessage { session_id: sid, text, mentions, images: Vec::new() });
        self.clear(cx);
    }

    pub fn push_mentions(&mut self, mentions: Vec<MentionKind>, cx: &mut Context<Self>) {
        for m in mentions {
            if !self.mentions.contains(&m) {
                self.mentions.push(m);
            }
        }
        cx.notify();
    }

    fn attach_active_file(&mut self, _: &(), _: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.active_file.clone() else { return; };
        self.push_mentions(vec![MentionKind::File { path }], cx);
    }

    fn attach_selection(&mut self, _: &(), _: &mut Window, cx: &mut Context<Self>) {
        self.push_mentions(vec![MentionKind::Selection], cx);
    }

    fn to_snapshot(mention: &MentionKind) -> MentionSnapshot {
        match mention {
            MentionKind::File { path } => MentionSnapshot::File { path: path.clone() },
            MentionKind::Directory { path } => MentionSnapshot::Directory { path: path.clone() },
            MentionKind::Symbol { name, path } => MentionSnapshot::Symbol { path: path.clone(), name: name.clone(), start_line: 0, end_line: 0 },
            MentionKind::Selection => MentionSnapshot::Selection { path: None, start_line: 0, end_line: 0 },
            MentionKind::Diagnostics { errors, warnings } => MentionSnapshot::Diagnostics { errors: *errors, warnings: *warnings },
            MentionKind::Thread { .. } => MentionSnapshot::Selection { path: None, start_line: 0, end_line: 0 },
            MentionKind::Image { id } => MentionSnapshot::PastedImage { id: *id },
            MentionKind::Url { url } => MentionSnapshot::Fetch { url: url.clone() },
        }
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.autocomplete.is_some() { self.autocomplete = None; cx.notify(); return; }
        if let Some(sid) = self.session_id { let _ = self.ui_tx.send(UiToHost::AbortSession { session_id: sid }); }
    }

    fn new_line(&mut self, _: &NewLine, _: &mut Window, cx: &mut Context<Self>) {
        self.text_input.update(cx, |input, cx| {
            let cur = input.text().to_string() + "\n";
            input.set_text(cur, cx);
        });
    }

    fn autocomplete_next(&mut self, _: &AutocompleteNext, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ref menu) = self.autocomplete { menu.update(cx, |m, cx| m.select_next(cx)); }
    }

    fn autocomplete_prev(&mut self, _: &AutocompletePrev, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ref menu) = self.autocomplete { menu.update(cx, |m, cx| m.select_prev(cx)); }
    }

    fn autocomplete_confirm(&mut self, _: &AutocompleteConfirm, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ref menu) = self.autocomplete else { return; };
        let item = menu.read(cx).selected_item().cloned();
        if let Some(item) = item {
            self.insert_autocomplete_item(&item, cx);
        }
        self.autocomplete = None;
        cx.notify();
    }

    fn insert_autocomplete_item(&mut self, item: &AutocompleteItem, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let trigger_pos = text.rfind(|c| c == '@' || c == '/').unwrap_or(text.len());
        let prefix = &text[..trigger_pos];
        let insertion = match item {
            AutocompleteItem::Mention(m) => { self.mentions.push(m.clone()); format!("{} ", m.label()) }
            AutocompleteItem::Command(c) => format!("/{} ", c.name),
        };
        let new_text = format!("{}{}", prefix, insertion);
        self.text_input.update(cx, |input, cx| input.set_text(new_text, cx));
    }
}

impl Focusable for InputComposer { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for InputComposer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_busy = self.is_busy;
        let is_empty = self.is_empty(cx);
        let has_autocomplete = self.autocomplete.is_some();
        let has_active_file = self.active_file.is_some();
        let voice_available = self.voice_available;

        // Check for autocomplete trigger
        self.check_autocomplete(cx);

        v_flex()
            .id("input-composer")
            .key_context("InputComposer")
            .track_focus(&self.focus_handle(cx))
            .on_action(cx.listener(Self::send))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::new_line))
            .on_action(cx.listener(Self::autocomplete_next))
            .on_action(cx.listener(Self::autocomplete_prev))
            .on_action(cx.listener(Self::autocomplete_confirm))
            .w_full()
            .gap(Spacing::Base02.px())
            .p(Spacing::Base06.px())
            .bg(ThemeColors::bg_secondary())
            .border_t_1()
            .border_color(ThemeColors::border())
            // Autocomplete popup
            .children(self.autocomplete.as_ref().map(|menu| {
                deferred(anchored().snap_to_window().child(menu.clone())).with_priority(1)
            }))
            // Mentions display
            .when(!self.mentions.is_empty(), |this| {
                this.child(
                    h_flex().w_full().gap(Spacing::Base04.px()).flex_wrap()
                        .children(self.mentions.iter().map(|m| {
                            h_flex().px(Spacing::Base02.px()).py(px(1.0)).bg(ThemeColors::bg_selected()).rounded_sm().gap(Spacing::Base02.px())
                                .child(Label::new(m.icon()).size(LabelSize::XSmall))
                                .child(Label::new(m.label()).size(LabelSize::XSmall).color(LabelColor::Accent))
                        }))
                )
            })
            .child(
                div()
                    .id("input-composer-scroll")
                    .flex_1()
                    .min_h(px(48.0))
                    .max_h(px(160.0))
                    .overflow_y_scroll()
                    .p(Spacing::Base04.px())
                    .bg(ThemeColors::bg_primary())
                    .border_1()
                    .border_color(if has_autocomplete { ThemeColors::border_focus() } else { ThemeColors::border() })
                    .rounded_sm()
                    .child(self.text_input.clone())
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap(Spacing::Base04.px())
                            .child(
                                Button::new("attach-active-file", "Attach File")
                                    .style(ButtonStyle::Ghost)
                                    .disabled(!has_active_file)
                                    .on_click(cx.listener(|this, _, window, cx| this.attach_active_file(&(), window, cx)))
                            )
                            .child(
                                Button::new("attach-selection", "Attach Selection")
                                    .style(ButtonStyle::Ghost)
                                    .on_click(cx.listener(|this, _, window, cx| this.attach_selection(&(), window, cx)))
                            )
                            .child(
                                Button::new("voice-toggle", if self.is_voice_active { "🔴 Stop" } else { "🎙️ Voice" })
                                    .style(if self.is_voice_active { ButtonStyle::Tinted(TintColor::Error) } else { ButtonStyle::Ghost })
                                    .disabled(!voice_available)
                                    .on_click(cx.listener(Self::toggle_voice))
                            )
                            .child(Label::new("@ mention | / command | Enter send").size(LabelSize::XSmall).color(LabelColor::Muted))
                    )
                    .child(
                        h_flex().gap(Spacing::Base04.px())
                            .when(is_busy, |this| this.child(
                                Button::new("abort", "Stop").style(ButtonStyle::Tinted(TintColor::Error))
                                    .on_click(cx.listener(|this, _, _, cx| { if let Some(sid) = this.session_id { let _ = this.ui_tx.send(UiToHost::AbortSession { session_id: sid }); } }))
                            ))
                            .child(
                                Button::new("send", if is_busy { "Sending..." } else { "Send ↵" })
                                    .style(if is_empty || is_busy { ButtonStyle::Ghost } else { ButtonStyle::Tinted(TintColor::Accent) })
                                    .disabled(is_empty || is_busy)
                                    .on_click(cx.listener(|this, _, window, cx| this.send(&Send, window, cx)))
                            )
                    )
            )
    }
}
