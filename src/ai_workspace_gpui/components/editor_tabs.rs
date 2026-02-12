//! Editor Tabs: Multi-tab editor with split support (Zed-isomorphic)

use gpui::{AnyElement, App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, Window, div, prelude::*, px};
use crossbeam_channel::Sender;
use std::path::PathBuf;
use crate::ai_workspace_gpui::{ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Button, ButtonStyle, Spacing}, protocol::{DiagnosticSnapshot, OpenFileSnapshot, TextEdit, UiToHost}, components::CodeEditor};

// ─────────────────────────────────────────────────────────────────────────────
// EditorTabs
// ─────────────────────────────────────────────────────────────────────────────

pub struct EditorTabs {
    focus_handle: FocusHandle,
    tabs: Vec<TabInfo>,
    active_path: Option<PathBuf>,
    editor: Entity<CodeEditor>,
    ui_tx: Sender<UiToHost>,
}

#[derive(Clone)]
struct TabInfo { path: PathBuf, name: String, is_dirty: bool }

impl EditorTabs {
    pub fn new(ui_tx: Sender<UiToHost>, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| CodeEditor::new(ui_tx.clone(), cx));
        Self { focus_handle: cx.focus_handle(), tabs: vec![], active_path: None, editor, ui_tx }
    }

    pub fn set_open_files(&mut self, files: Vec<OpenFileSnapshot>, active: Option<PathBuf>, cx: &mut Context<Self>) {
        self.tabs = files.iter().map(|f| TabInfo {
            path: f.path.clone(),
            name: f.path.file_name().and_then(|n| n.to_str()).unwrap_or("untitled").to_string(),
            is_dirty: f.is_dirty,
        }).collect();
        self.active_path = active;
        cx.notify();
    }

    pub fn set_file_content(&mut self, path: PathBuf, content: String, version: u64, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.set_content(path, content, version, cx));
    }

    pub fn apply_file_changed(&mut self, path: PathBuf, version: u64, edits: Vec<TextEdit>, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |e, cx| e.apply_file_changed(path, version, edits, cx));
    }

    pub fn tick(&mut self, cx: &mut Context<Self>) -> bool {
        self.editor.update(cx, |e, cx| e.tick_external_playback(cx))
    }

    pub fn set_cursor(&mut self, path: PathBuf, line: u32, col: u32, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.set_cursor_from_host(path, line as usize, col as usize, cx));
    }

    pub fn set_diagnostics(&mut self, path: PathBuf, diagnostics: Vec<DiagnosticSnapshot>, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.set_diagnostics(path, diagnostics, cx));
    }

    pub fn set_completions(&mut self, path: PathBuf, items: Vec<String>, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.set_completions(path, items, cx));
    }

    pub fn set_hover(&mut self, path: PathBuf, markdown: String, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| e.set_hover(path, markdown, cx));
    }

    pub fn mark_dirty(&mut self, path: &PathBuf, is_dirty: bool, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| &t.path == path) { tab.is_dirty = is_dirty; }
        if self.active_path.as_ref() == Some(path) {
            self.editor.update(cx, |e, cx| e.set_dirty(is_dirty, cx));
        }
        cx.notify();
    }

    fn select_tab(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.active_path = Some(path.clone());
        let _ = self.ui_tx.send(UiToHost::IdeSetActiveFile { path });
        cx.notify();
    }

    fn close_tab(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let _ = self.ui_tx.send(UiToHost::IdeCloseFile { path });
        cx.notify();
    }

    fn render_tab(&self, tab: &TabInfo, is_active: bool) -> AnyElement {
        let path = tab.path.clone();
        let path_close = tab.path.clone();
        let dirty = if tab.is_dirty { " •" } else { "" };

        h_flex()
            .id(format!("tab-{}", tab.name))
            .h(px(28.0))
            .px(Spacing::Base04.px())
            .gap(Spacing::Base04.px())
            .items_center()
            .cursor_pointer()
            .border_b_2()
            .when(is_active, |d| d.border_color(ThemeColors::text_accent()).bg(ThemeColors::bg_primary()))
            .when(!is_active, |d| d.border_color(gpui::transparent_black()).bg(ThemeColors::bg_secondary()).hover(|d| d.bg(ThemeColors::bg_hover())))
            .child(Label::new(format!("{}{}", tab.name, dirty)).size(LabelSize::Small).color(if is_active { LabelColor::Primary } else { LabelColor::Secondary }))
            .child(
                div()
                    .id(format!("close-{}", tab.name))
                    .w(px(14.0))
                    .h(px(14.0))
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|d| d.bg(ThemeColors::bg_hover()))
                    .child(Label::new("×").size(LabelSize::Small).color(LabelColor::Muted))
            )
            .into_any_element()
    }
}

impl Focusable for EditorTabs { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for EditorTabs {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = self.tabs.clone();
        let active_path = self.active_path.clone();
        let tx = self.ui_tx.clone();

        // Tab bar
        let tab_bar = h_flex()
            .flex_none()
            .w_full()
            .h(px(28.0))
            .bg(ThemeColors::bg_secondary())
            .border_b_1()
            .border_color(ThemeColors::border())
            .overflow_x_hidden()
            .children(tabs.iter().map(|tab| {
                let is_active = active_path.as_ref() == Some(&tab.path);
                let path_click = tab.path.clone();
                let path_close = tab.path.clone();
                let tx_click = tx.clone();
                let tx_close = tx.clone();

                h_flex()
                    .id(format!("tab-{}", tab.name))
                    .h(px(28.0))
                    .px(Spacing::Base04.px())
                    .gap(Spacing::Base04.px())
                    .items_center()
                    .cursor_pointer()
                    .border_b_2()
                    .when(is_active, |d| d.border_color(ThemeColors::text_accent()).bg(ThemeColors::bg_primary()))
                    .when(!is_active, |d| d.border_color(gpui::transparent_black()).bg(ThemeColors::bg_secondary()).hover(|d| d.bg(ThemeColors::bg_hover())))
                    .on_click(move |_, _, _| { let _ = tx_click.send(UiToHost::IdeSetActiveFile { path: path_click.clone() }); })
                    .child(Label::new(format!("{}{}", tab.name, if tab.is_dirty { " •" } else { "" })).size(LabelSize::Small).color(if is_active { LabelColor::Primary } else { LabelColor::Secondary }))
                    .child(
                        div()
                            .id(format!("close-{}", tab.name))
                            .w(px(14.0))
                            .h(px(14.0))
                            .rounded_sm()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|d| d.bg(ThemeColors::bg_hover()))
                            .on_click(move |_, _, _| {
                                let _ = tx_close.send(UiToHost::IdeCloseFile { path: path_close.clone() });
                            })
                            .child(Label::new("×").size(LabelSize::Small).color(LabelColor::Muted))
                    )
            }));

        // Editor area or empty state
        let editor_area = if self.tabs.is_empty() {
            div()
                .flex_1()
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .child(Label::new("No files open").size(LabelSize::Large).color(LabelColor::Muted))
                .into_any_element()
        } else {
            self.editor.clone().into_any_element()
        };

        v_flex()
            .id("editor-tabs")
            .track_focus(&self.focus_handle)
            .flex_1()
            .h_full()
            .bg(ThemeColors::bg_primary())
            .overflow_hidden()
            .child(tab_bar)
            .child(editor_area)
    }
}
