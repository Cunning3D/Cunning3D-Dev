//! Project Panel: File tree with virtualized rendering (Zed-isomorphic)

use gpui::{AnyElement, App, Context, Entity, FocusHandle, Focusable, IntoElement, ListAlignment, ListState, ParentElement, Render, Styled, Window, div, list, prelude::*, px};
use crossbeam_channel::Sender;
use crate::ai_workspace_gpui::{ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Button, ButtonStyle, Spacing}, protocol::{EntryId, EntryKind, FileEntrySnapshot, FileIcon, UiToHost}};

// ─────────────────────────────────────────────────────────────────────────────
// ProjectPanel
// ─────────────────────────────────────────────────────────────────────────────

pub struct ProjectPanel {
    entries: Vec<FileEntrySnapshot>,
    list_state: ListState,
    selected_id: Option<EntryId>,
    ui_tx: Sender<UiToHost>,
    focus_handle: FocusHandle,
}

impl ProjectPanel {
    pub fn new(ui_tx: Sender<UiToHost>, cx: &mut Context<Self>) -> Self {
        Self {
            entries: vec![],
            list_state: ListState::new(0, ListAlignment::Top, px(300.0)),
            selected_id: None,
            ui_tx,
            focus_handle: cx.focus_handle(),
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
        let entries = self.entries.clone();
        let selected_id = self.selected_id;
        let tx = self.ui_tx.clone();

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
                    .h(px(32.0))
                    .px(Spacing::Base06.px())
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
                            let entry_clone = entry.clone();
                            let tx_click = tx.clone();

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
                                    h_flex()
                                        .gap(Spacing::Base04.px())
                                        .items_center()
                                        .child(Label::new(Self::file_icon(&entry.path)).size(LabelSize::Small).color(if matches!(entry.kind, EntryKind::Dir | EntryKind::UnloadedDir | EntryKind::PendingDir) { LabelColor::Accent } else { LabelColor::Muted }))
                                        .child(Label::new(&entry.name).size(LabelSize::Small).color(if selected { LabelColor::Primary } else { LabelColor::Secondary }))
                                        .when(entry.kind == EntryKind::PendingDir, |d| d.child(Label::new("...").size(LabelSize::XSmall).color(LabelColor::Muted)))
                                )
                                .on_click(move |_, window, cx| {
                                    let e = entry_clone.clone();
                                    match e.kind {
                                        EntryKind::Dir | EntryKind::UnloadedDir | EntryKind::PendingDir => {
                                            if e.is_expanded { let _ = tx_click.send(UiToHost::IdeCollapseDir { entry_id: e.id }); }
                                            else { let _ = tx_click.send(UiToHost::IdeExpandDir { entry_id: e.id }); }
                                        }
                                        EntryKind::File => { let _ = tx_click.send(UiToHost::IdeOpenFile { path: e.path.clone() }); }
                                    }
                                });

                            row.into_any_element()
                        })
                        .flex_1()
                        .size_full()
                    )
            )
    }
}
