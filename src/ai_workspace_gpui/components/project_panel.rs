//! Project Panel: File tree with virtualized rendering (Zed-isomorphic)

use crossbeam_channel::{bounded, Receiver, Sender};
use gpui::{anchored, deferred, div, list, point, px, prelude::*, AnyElement, App, Context, DismissEvent, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, ListAlignment, ListState, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render, Styled, Window};
use crate::ai_workspace_gpui::{
    protocol::{EntryId, EntryKind, FileEntrySnapshot, FileIcon, UiToHost},
    ui::{h_flex, v_flex, Button, ButtonStyle, ContextMenu, ContextMenuItem, Label, LabelColor, LabelSize, Spacing, TextInput, ThemeColors},
};

// ─────────────────────────────────────────────────────────────────────────────
// ProjectPanel
// ─────────────────────────────────────────────────────────────────────────────

pub struct ProjectPanel {
    entries: Vec<FileEntrySnapshot>,
    list_state: ListState,
    selected_id: Option<EntryId>,
    ui_tx: Sender<UiToHost>,
    focus_handle: FocusHandle,

    context_menu: Option<Entity<ContextMenu>>,
    context_anchor: Point<Pixels>,
    cmd_rx: Receiver<ProjectPanelCmd>,
    cmd_tx: Sender<ProjectPanelCmd>,
    renaming: Option<EntryId>,
    rename_input: Entity<TextInput>,
}

impl ProjectPanel {
    pub fn new(ui_tx: Sender<UiToHost>, cx: &mut Context<Self>) -> Self {
        let (cmd_tx, cmd_rx) = bounded::<ProjectPanelCmd>(64);
        let rename_input = cx.new(|cx| TextInput::new(cx, "Rename").multiline(false));
        Self {
            entries: vec![],
            list_state: ListState::new(0, ListAlignment::Top, px(300.0)),
            selected_id: None,
            ui_tx,
            focus_handle: cx.focus_handle(),

            context_menu: None,
            context_anchor: point(px(0.0), px(0.0)),
            cmd_rx,
            cmd_tx,
            renaming: None,
            rename_input,
        }
    }

    pub fn set_entries(&mut self, entries: Vec<FileEntrySnapshot>, cx: &mut Context<Self>) {
        let count = entries.len();
        self.entries = entries;
        self.list_state.reset(count);
        cx.notify();
    }

    pub fn set_selected(&mut self, id: Option<EntryId>, cx: &mut Context<Self>) {
        self.selected_id = id;
        cx.notify();
    }

    fn entry_by_id(&self, id: EntryId) -> Option<&FileEntrySnapshot> {
        self.entries.iter().find(|e| e.id == id)
    }

    fn pump_cmds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            match cmd {
                ProjectPanelCmd::EntryClicked(id) => {
                    let Some(entry) = self.entry_by_id(id).cloned() else { continue; };
                    self.on_entry_click(&entry, window, cx);
                }
                ProjectPanelCmd::RightClick { id, anchor } => {
                    self.context_anchor = anchor;
                    self.selected_id = Some(id);
                    self.open_entry_menu(id, window, cx);
                    cx.notify();
                }
                ProjectPanelCmd::RevealInExplorer(id) => {
                    let Some(entry) = self.entry_by_id(id).cloned() else { continue; };
                    let _ = self.ui_tx.send(UiToHost::IdeRevealInExplorer { path: entry.path.clone() });
                }
                ProjectPanelCmd::BeginRename(id) => {
                    let Some(entry) = self.entry_by_id(id).cloned() else { continue; };
                    self.context_menu = None;
                    self.renaming = Some(id);
                    self.rename_input.update(cx, |i, cx| i.set_text(entry.name.clone(), cx));
                    self.rename_input.focus_handle(cx).focus(window, cx);
                    cx.notify();
                }
                ProjectPanelCmd::SubmitRename(id) => {
                    let Some(entry) = self.entry_by_id(id).cloned() else { continue; };
                    let new_name = self.rename_input.read(cx).text().trim().to_string();
                    if new_name.is_empty() {
                        continue;
                    }
                    if let Some(parent) = entry.path.parent() {
                        let to = parent.join(new_name);
                        let _ = self.ui_tx.send(UiToHost::IdeRenamePath { from: entry.path.clone(), to });
                    }
                    self.renaming = None;
                    self.rename_input.update(cx, |i, cx| i.clear(cx));
                    cx.notify();
                }
                ProjectPanelCmd::CancelRename => {
                    self.renaming = None;
                    self.rename_input.update(cx, |i, cx| i.clear(cx));
                    cx.notify();
                }
                ProjectPanelCmd::RequestDelete(id) => {
                    self.open_delete_confirm_menu(id, window, cx);
                }
                ProjectPanelCmd::ConfirmDelete(id) => {
                    let Some(entry) = self.entry_by_id(id).cloned() else { continue; };
                    let _ = self.ui_tx.send(UiToHost::IdeDeletePath { path: entry.path.clone() });
                    self.context_menu = None;
                    cx.notify();
                }
                ProjectPanelCmd::CloseMenu => {
                    self.context_menu = None;
                    cx.notify();
                }
            }
        }
    }

    fn open_menu(&mut self, items: Vec<ContextMenuItem>, window: &mut Window, cx: &mut Context<Self>) {
        let menu = cx.new(|cx| ContextMenu::new(items, cx));
        cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.context_menu = None;
            cx.notify();
        })
        .detach();
        menu.focus_handle(cx).focus(window, cx);
        self.context_menu = Some(menu);
        cx.notify();
    }

    fn open_entry_menu(&mut self, id: EntryId, window: &mut Window, cx: &mut Context<Self>) {
        let tx1 = self.cmd_tx.clone();
        let tx2 = self.cmd_tx.clone();
        let tx3 = self.cmd_tx.clone();
        let tx4 = self.cmd_tx.clone();
        self.open_menu(
            vec![
                ContextMenuItem::header("Entry"),
                ContextMenuItem::entry("Show in Explorer", move |_, _| {
                    let _ = tx1.try_send(ProjectPanelCmd::RevealInExplorer(id));
                }),
                ContextMenuItem::entry("Rename", move |_, _| {
                    let _ = tx2.try_send(ProjectPanelCmd::BeginRename(id));
                }),
                ContextMenuItem::entry("Delete", move |_, _| {
                    let _ = tx3.try_send(ProjectPanelCmd::RequestDelete(id));
                }),
                ContextMenuItem::separator(),
                ContextMenuItem::entry("Cancel", move |_, _| {
                    let _ = tx4.try_send(ProjectPanelCmd::CloseMenu);
                }),
            ],
            window,
            cx,
        );
    }

    fn open_delete_confirm_menu(&mut self, id: EntryId, window: &mut Window, cx: &mut Context<Self>) {
        let tx_yes = self.cmd_tx.clone();
        let tx_no = self.cmd_tx.clone();
        self.open_menu(
            vec![
                ContextMenuItem::header("Delete?"),
                ContextMenuItem::entry("Delete", move |_, _| {
                    let _ = tx_yes.try_send(ProjectPanelCmd::ConfirmDelete(id));
                }),
                ContextMenuItem::separator(),
                ContextMenuItem::entry("Cancel", move |_, _| {
                    let _ = tx_no.try_send(ProjectPanelCmd::CloseMenu);
                }),
            ],
            window,
            cx,
        );
    }

    fn on_entry_click(&mut self, entry: &FileEntrySnapshot, _window: &mut Window, cx: &mut Context<Self>) {
        self.selected_id = Some(entry.id);
        match entry.kind {
            EntryKind::Dir | EntryKind::UnloadedDir | EntryKind::PendingDir => {
                if entry.is_expanded {
                    let _ = self.ui_tx.send(UiToHost::IdeCollapseDir { entry_id: entry.id });
                } else {
                    let _ = self.ui_tx.send(UiToHost::IdeExpandDir { entry_id: entry.id });
                }
            }
            EntryKind::File => {
                let _ = self.ui_tx.send(UiToHost::IdeOpenFile { path: entry.path.clone() });
            }
        }
        cx.notify();
    }

    fn render_entry(&self, entry: &FileEntrySnapshot, selected: bool) -> AnyElement {
        let indent = px((entry.depth as f32) * 16.0);
        let icon = match entry.kind {
            EntryKind::Dir | EntryKind::PendingDir => if entry.is_expanded { "▼" } else { "▶" },
            EntryKind::UnloadedDir => "▶",
            EntryKind::File => Self::file_icon(&entry.path),
        };
        let is_loading = entry.kind == EntryKind::PendingDir;

        h_flex()
            .w_full()
            .pl(indent)
            .px(Spacing::Base04.px())
            .py(Spacing::Base02.px())
            .gap(Spacing::Base04.px())
            .rounded_sm()
            .cursor_pointer()
            .when(selected, |d| d.bg(ThemeColors::bg_selected()))
            .when(!selected, |d| d.hover(|d| d.bg(ThemeColors::bg_hover())))
            .child(Label::new(icon).size(LabelSize::Small).color(if matches!(entry.kind, EntryKind::Dir | EntryKind::UnloadedDir | EntryKind::PendingDir) { LabelColor::Accent } else { LabelColor::Muted }))
            .child(Label::new(&entry.name).size(LabelSize::Small).color(if selected { LabelColor::Primary } else { LabelColor::Secondary }))
            .when(is_loading, |d| d.child(Label::new("...").size(LabelSize::XSmall).color(LabelColor::Muted)))
            .into_any_element()
    }

    fn file_icon(path: &std::path::Path) -> &'static str {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext.to_lowercase().as_str() {
            "rs" => "RS",
            "py" => "PY",
            "js" | "jsx" | "mjs" => "JS",
            "ts" | "tsx" => "TS",
            "json" => "{}",
            "toml" | "yaml" | "yml" => "CFG",
            "md" | "markdown" => "MD",
            "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" => "IMG",
            _ => "F",
        }
    }
}

impl Focusable for ProjectPanel { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for ProjectPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.pump_cmds(_window, cx);
        let entries = self.entries.clone();
        let selected_id = self.selected_id;
        let tx = self.ui_tx.clone();
        let cmd_tx = self.cmd_tx.clone();
        let renaming = self.renaming;
        let rename_input = self.rename_input.clone();
        let menu = self.context_menu.as_ref().map(|m| {
            deferred(anchored().position(self.context_anchor).child(m.clone())).with_priority(2)
        });

        v_flex()
            .id("project-panel")
            .track_focus(&self.focus_handle)
            .flex_1()
            .w_full()
            .min_h(px(200.0))
            .overflow_hidden()
            .border_b_1()
            .border_color(ThemeColors::border())
            .child(
                h_flex()
                    .flex_none()
                    .w_full()
                    .h(px(28.0))
                    .px(Spacing::Base04.px())
                    .justify_between()
                    .items_center()
                    .border_b_1()
                    .border_color(ThemeColors::border())
                    .child(Label::new("Plugins").size(LabelSize::Small).color(LabelColor::Muted))
                    .child(
                        Button::new("refresh-tree", "↻")
                            .style(ButtonStyle::Ghost)
                            .on_click({
                                let tx = tx.clone();
                                move |_, _, _| { let _ = tx.send(UiToHost::IdeRefreshTree); }
                            })
                    )
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        list(self.list_state.clone(), move |ix, _window, _cx| {
                            let Some(entry) = entries.get(ix) else { return div().h(px(24.0)).into_any_element(); };
                            let selected = selected_id == Some(entry.id);
                            let entry_id = entry.id;
                            let cmd_click = cmd_tx.clone();
                            let cmd_rc = cmd_tx.clone();

                            // Build indent guides (tree lines)
                            let depth = entry.depth;
                            let mut row = h_flex()
                                .id(format!("entry-{}", entry.id.0))
                                .w_full()
                                .h(px(24.0))
                                .items_center()
                                .cursor_pointer()
                                .when(selected, |d| d.bg(ThemeColors::bg_selected()))
                                .when(!selected, |d| d.hover(|d| d.bg(ThemeColors::bg_hover())));

                            // Indent guides: vertical lines for each level
                            for _ in 0..depth {
                                row = row.child(
                                    div()
                                        .w(px(16.0))
                                        .h_full()
                                        .flex_none()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            div()
                                                .w(px(1.0))
                                                .h_full()
                                                .bg(ThemeColors::border().opacity(0.4))
                                        )
                                );
                            }

                            // Arrow/icon + name
                            row = row
                                .child(
                                    div()
                                        .w(px(16.0))
                                        .flex_none()
                                        .items_center()
                                        .justify_center()
                                        .child(Label::new(match entry.kind {
                                            EntryKind::Dir | EntryKind::PendingDir => if entry.is_expanded { "▼" } else { "▶" },
                                            EntryKind::UnloadedDir => "▶",
                                            EntryKind::File => "  ",
                                        }).size(LabelSize::XSmall).color(LabelColor::Muted))
                                )
                                .child(
                                    {
                                        let mut name_row = h_flex()
                                        .gap(Spacing::Base04.px())
                                        .items_center()
                                        .child(Label::new(Self::file_icon(&entry.path)).size(LabelSize::Small).color(if matches!(entry.kind, EntryKind::Dir | EntryKind::UnloadedDir | EntryKind::PendingDir) { LabelColor::Accent } else { LabelColor::Muted }))
                                        .when(entry.kind == EntryKind::PendingDir, |d| d.child(Label::new("...").size(LabelSize::XSmall).color(LabelColor::Muted)));

                                        if renaming == Some(entry_id) {
                                            let tx_submit = cmd_tx.clone();
                                            let tx_cancel = cmd_tx.clone();
                                            name_row = name_row.child(
                                                div()
                                                    .flex_1()
                                                    .on_key_down(move |ev: &KeyDownEvent, _, _| {
                                                        match ev.keystroke.key.as_str() {
                                                            "enter" => {
                                                                let _ = tx_submit.try_send(ProjectPanelCmd::SubmitRename(entry_id));
                                                            }
                                                            "escape" => {
                                                                let _ = tx_cancel.try_send(ProjectPanelCmd::CancelRename);
                                                            }
                                                            _ => {}
                                                        }
                                                    })
                                                    .child(rename_input.clone()),
                                            );
                                        } else {
                                            name_row = name_row.child(
                                                Label::new(&entry.name)
                                                    .size(LabelSize::Small)
                                                    .color(if selected { LabelColor::Primary } else { LabelColor::Secondary }),
                                            );
                                        }
                                        name_row
                                    }
                                )
                                .on_click(move |_, _, _| {
                                    let _ = cmd_click.try_send(ProjectPanelCmd::EntryClicked(entry_id));
                                })
                                .on_mouse_down(MouseButton::Right, move |ev: &MouseDownEvent, _, _| {
                                    let _ = cmd_rc.try_send(ProjectPanelCmd::RightClick { id: entry_id, anchor: ev.position });
                                });

                            row.into_any_element()
                        })
                        .flex_1()
                        .size_full()
                    )
            )
            .children(menu)
    }
}

enum ProjectPanelCmd {
    EntryClicked(EntryId),
    RightClick { id: EntryId, anchor: Point<Pixels> },
    RevealInExplorer(EntryId),
    BeginRename(EntryId),
    SubmitRename(EntryId),
    CancelRename,
    RequestDelete(EntryId),
    ConfirmDelete(EntryId),
    CloseMenu,
}
