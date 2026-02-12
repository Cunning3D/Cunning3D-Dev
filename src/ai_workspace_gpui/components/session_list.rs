//! Session list sidebar with Zed-style right-click menu (rename/delete).
use crossbeam_channel::{bounded, Receiver, Sender};
use gpui::{
    anchored, deferred, div, point, px, prelude::*, AnyElement, App, Context, DismissEvent,
    ElementId, Entity, FocusHandle, Focusable, InteractiveElement, KeyDownEvent, MouseButton,
    MouseDownEvent, ParentElement, Pixels, Point, Render, Styled, Window,
};
use uuid::Uuid;
use crate::ai_workspace_gpui::{
    protocol::{SessionSnapshot, UiToHost},
    ui::{
        h_flex, v_flex, Button, ButtonStyle, ContextMenu, ContextMenuItem, Label, LabelColor,
        LabelSize, Spacing, TextInput, ThemeColors,
    },
};

pub struct SessionList {
    sessions: Vec<SessionSnapshot>,
    active_id: Option<uuid::Uuid>,
    ui_tx: Sender<UiToHost>,
    focus_handle: FocusHandle,
    context_menu: Option<Entity<ContextMenu>>,
    context_anchor: Point<Pixels>,
    cmd_rx: Receiver<SessionListCmd>,
    cmd_tx: Sender<SessionListCmd>,
    renaming: Option<Uuid>,
    rename_input: Entity<TextInput>,
}

impl SessionList {
    pub fn new(sessions: Vec<SessionSnapshot>, active_id: Option<uuid::Uuid>, ui_tx: Sender<UiToHost>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (cmd_tx, cmd_rx) = bounded::<SessionListCmd>(64);
        let rename_input = cx.new(|cx| TextInput::new(cx, "Rename session").multiline(false));
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            sessions,
            active_id,
            ui_tx,
            focus_handle,
            context_menu: None,
            context_anchor: point(px(0.0), px(0.0)),
            cmd_rx,
            cmd_tx,
            renaming: None,
            rename_input,
        }
    }

    pub fn set_sessions(&mut self, sessions: Vec<SessionSnapshot>, active_id: Option<uuid::Uuid>, cx: &mut Context<Self>) {
        self.sessions = sessions;
        self.active_id = active_id;
        if let Some(sid) = self.renaming {
            if !self.sessions.iter().any(|s| s.id == sid) {
                self.renaming = None;
            }
        }
        cx.notify();
    }

    fn pump_cmds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            match cmd {
                SessionListCmd::BeginRename(sid) => {
                    self.context_menu = None;
                    self.renaming = Some(sid);
                    let title = self.sessions.iter().find(|s| s.id == sid).map(|s| s.title.clone()).unwrap_or_default();
                    self.rename_input.update(cx, |i, cx| i.set_text(title, cx));
                    self.rename_input.focus_handle(cx).focus(window, cx);
                }
                SessionListCmd::SubmitRename(sid) => {
                    let mut title = self.rename_input.read(cx).text().trim().to_string();
                    if title.is_empty() {
                        title = "New Session".into();
                    }
                    let _ = self.ui_tx.send(UiToHost::RenameSession { session_id: sid, title });
                    self.renaming = None;
                    self.rename_input.update(cx, |i, cx| i.clear(cx));
                }
                SessionListCmd::CancelRename => {
                    self.renaming = None;
                    self.rename_input.update(cx, |i, cx| i.clear(cx));
                }
                SessionListCmd::RequestDelete(sid) => {
                    let non_empty = self.sessions.iter().find(|s| s.id == sid).map(|s| !s.entries.is_empty()).unwrap_or(false);
                    if non_empty {
                        self.open_delete_confirm_menu(sid, window, cx);
                    } else {
                        let _ = self.ui_tx.send(UiToHost::DeleteSession { session_id: sid });
                        self.context_menu = None;
                    }
                }
                SessionListCmd::ConfirmDelete(sid) => {
                    let _ = self.ui_tx.send(UiToHost::DeleteSession { session_id: sid });
                    self.context_menu = None;
                }
                SessionListCmd::CloseMenu => {
                    self.context_menu = None;
                }
            }
        }
    }

    fn open_menu(&mut self, items: Vec<ContextMenuItem>, window: &mut Window, cx: &mut Context<Self>) {
        let menu = cx.new(|cx| ContextMenu::new(items, cx));
        cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.context_menu = None;
            cx.notify();
        }).detach();
        menu.focus_handle(cx).focus(window, cx);
        self.context_menu = Some(menu);
        cx.notify();
    }

    fn open_session_menu(&mut self, sid: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let tx1 = self.cmd_tx.clone();
        let tx2 = self.cmd_tx.clone();
        let tx3 = self.cmd_tx.clone();
        self.open_menu(
            vec![
                ContextMenuItem::header("Session"),
                ContextMenuItem::entry("Rename", move |_, _| { let _ = tx1.try_send(SessionListCmd::BeginRename(sid)); }),
                ContextMenuItem::entry("Delete", move |_, _| { let _ = tx2.try_send(SessionListCmd::RequestDelete(sid)); }),
                ContextMenuItem::separator(),
                ContextMenuItem::entry("Cancel", move |_, _| { let _ = tx3.try_send(SessionListCmd::CloseMenu); }),
            ],
            window,
            cx,
        );
    }

    fn open_delete_confirm_menu(&mut self, sid: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let tx_yes = self.cmd_tx.clone();
        let tx_no = self.cmd_tx.clone();
        self.open_menu(
            vec![
                ContextMenuItem::header("Delete session?"),
                ContextMenuItem::entry("Delete", move |_, _| { let _ = tx_yes.try_send(SessionListCmd::ConfirmDelete(sid)); }),
                ContextMenuItem::separator(),
                ContextMenuItem::entry("Cancel", move |_, _| { let _ = tx_no.try_send(SessionListCmd::CloseMenu); }),
            ],
            window,
            cx,
        );
    }

    fn on_session_right_click(&mut self, sid: Uuid, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.context_anchor = ev.position;
        let _ = self.ui_tx.send(UiToHost::SelectSession { session_id: sid });
        self.open_session_menu(sid, window, cx);
    }

    fn rename_key_down(&mut self, sid: Uuid, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        match ev.keystroke.key.as_str() {
            "enter" => { let _ = self.cmd_tx.try_send(SessionListCmd::SubmitRename(sid)); }
            "escape" => { let _ = self.cmd_tx.try_send(SessionListCmd::CancelRename); }
            _ => {}
        }
        cx.notify();
    }

    fn render_session_item(&mut self, session: &SessionSnapshot, cx: &mut Context<Self>) -> AnyElement {
        let is_active = self.active_id == Some(session.id);
        let sid = session.id;
        let tx = self.ui_tx.clone();
        let title = if session.title.is_empty() { "New Session".to_string() } else { session.title.clone() };
        let title_display = if title.chars().count() > 25 { format!("{}…", title.chars().take(24).collect::<String>()) } else { title };
        let status = session.is_busy.then_some(("BUSY", LabelColor::Accent));
        let entry_count = session.entries.len();
        let info_text = if entry_count > 0 { format!("{} messages", entry_count) } else { "Empty".to_string() };
        let is_renaming = self.renaming == Some(sid);
        let rename_row = is_renaming.then_some(
            div()
                .key_context("SessionRename")
                .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| this.rename_key_down(sid, ev, window, cx)))
                .child(self.rename_input.clone())
                .into_any_element(),
        );
        let tx_rc = self.ui_tx.clone();
        div()
            .id(ElementId::Name(format!("session-{sid}").into()))
            .w_full()
            .px(Spacing::Base06.px())
            .py(Spacing::Base02.px())
            .rounded_sm()
            .cursor_pointer()
            .when(is_active, |d| d.bg(ThemeColors::bg_selected()).border_l_2().border_color(ThemeColors::text_accent()))
            .when(!is_active, |d| d.hover(|d| d.bg(ThemeColors::bg_elevated())))
            .on_click(move |_, _, _| { let _ = tx.send(UiToHost::SelectSession { session_id: sid }); })
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                let _ = tx_rc.send(UiToHost::SelectSession { session_id: sid });
                this.on_session_right_click(sid, ev, window, cx);
            }))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(
                        v_flex()
                            .flex_1()
                            .overflow_hidden()
                            .gap(Spacing::Base02.px())
                            .child(
                                h_flex().gap(Spacing::Base04.px())
                                    .child(Label::new(title_display).size(LabelSize::Small).color(if is_active { LabelColor::Primary } else { LabelColor::Secondary }))
                                    .children(status.map(|(icon, color)| Label::new(icon).size(LabelSize::XSmall).color(color)))
                            )
                            .child(Label::new(info_text).size(LabelSize::XSmall).color(LabelColor::Muted))
                            .children(rename_row)
                    )
            )
            .into_any_element()
    }
}

enum SessionListCmd {
    BeginRename(Uuid),
    SubmitRename(Uuid),
    CancelRename,
    RequestDelete(Uuid),
    ConfirmDelete(Uuid),
    CloseMenu,
}

impl Focusable for SessionList { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for SessionList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.pump_cmds(window, cx);
        let tx_new = self.ui_tx.clone();
        let header = h_flex()
            .flex_none()
            .w_full()
            .h(px(28.0))
            .px(Spacing::Base04.px())
            .justify_between()
            .items_center()
            .border_b_1()
            .border_color(ThemeColors::border())
            .child(Label::new("Sessions").size(LabelSize::Small).color(LabelColor::Primary))
            .child(Button::new("new-session", "+").style(ButtonStyle::Ghost).on_click(move |_, _, _| { let _ = tx_new.send(UiToHost::NewSession); }));

        let sessions = self.sessions.clone();
        let session_items: Vec<AnyElement> = sessions.iter().map(|s| self.render_session_item(s, cx)).collect();
        let list_content = if session_items.is_empty() {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .p(Spacing::Base06.px())
                .child(Label::new("No sessions yet").size(LabelSize::Small).color(LabelColor::Muted))
                .child(Label::new("Click + to create one").size(LabelSize::XSmall).color(LabelColor::Muted))
                .into_any_element()
        } else {
            div()
                .id("session-list-scroll")
                .flex_1()
                .overflow_y_scroll()
                .p(Spacing::Base04.px())
                .children(session_items)
                .into_any_element()
        };

        let menu = self.context_menu.as_ref().map(|m| deferred(anchored().position(self.context_anchor).child(m.clone())).with_priority(2));

        v_flex()
            .id("session-list")
            .track_focus(&self.focus_handle)
            .flex_1()
            .w_full()
            .min_h(px(150.0))
            .overflow_hidden()
            .child(header)
            .child(list_content)
            .children(menu)
    }
}
