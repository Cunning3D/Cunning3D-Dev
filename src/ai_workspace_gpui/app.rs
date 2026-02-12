//! GPUI window launcher and UI components for AI Workspace (Zed-style architecture).

use super::protocol::*;
use super::ui::{h_flex, v_flex, ThemeColors, Label, LabelColor, LabelSize, Button, ButtonStyle, TintColor, Spacing, ResizeDirection, drag_handle};
use super::components::{SessionList, ThreadView, InputComposer, ModelSelector, ModeSelector, ProjectPanel, EditorTabs, QuickOpen, CommandPalette, ProviderSettings, VoiceAssistantSettings};
use crossbeam_channel::{Receiver, Sender, unbounded};
use gpui::{actions, App, Application, AsyncApp, Bounds, Context, DismissEvent, Entity, ExternalPaths, FocusHandle, Focusable, InteractiveElement, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Render, SharedString, TitlebarOptions, Window, WindowBounds, WindowControlArea, WindowHandle, WindowOptions, Styled, anchored, deferred, div, prelude::*, px, relative, size};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use crate::app::window_frame::{WINDOW_CHROME_ICON_FONT_FAMILY, WINDOW_CHROME_GLYPH_CLOSE, WINDOW_CHROME_GLYPH_MAX, WINDOW_CHROME_GLYPH_MIN};

actions!(ai_workspace, [Quit, Refresh, NewSession, ToggleModelSelector, CycleMode, ToggleProjectPanel, ToggleQuickOpen, ToggleCommandPalette, SaveFile, SaveAllFiles]);

// ─────────────────────────────────────────────────────────────────────────────
// GPUI Thread Handle
// ─────────────────────────────────────────────────────────────────────────────

pub struct GpuiWindowHandle {
    pub action_tx: Sender<UiToHost>,
    pub event_rx: Receiver<UiToHost>,
    thread: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
}

impl GpuiWindowHandle {
    pub fn is_running(&self) -> bool { self.thread.as_ref().map(|t| !t.is_finished()).unwrap_or(false) }
    pub fn shutdown(&self) { self.shutdown.store(true, Ordering::SeqCst); let _ = self.action_tx.send(UiToHost::Shutdown); }
}

impl Drop for GpuiWindowHandle {
    fn drop(&mut self) { self.shutdown(); if let Some(t) = self.thread.take() { let _ = t.join(); } }
}

// ─────────────────────────────────────────────────────────────────────────────
// Launch Function
// ─────────────────────────────────────────────────────────────────────────────

pub fn launch_gpui_window(host_to_ui_rx: Receiver<HostToUi>, ui_to_host_tx: Sender<UiToHost>) -> GpuiWindowHandle {
    let (action_tx, action_rx) = unbounded::<UiToHost>();
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();
    let ui_tx = ui_to_host_tx.clone();
    let thread = std::thread::spawn(move || { run_gpui_app(host_to_ui_rx, ui_tx, action_rx, shutdown_clone); });
    GpuiWindowHandle { action_tx, event_rx: unbounded().1, thread: Some(thread), shutdown }
}

// ─────────────────────────────────────────────────────────────────────────────
// GPUI Application
// ─────────────────────────────────────────────────────────────────────────────

fn run_gpui_app(host_rx: Receiver<HostToUi>, ui_tx: Sender<UiToHost>, action_rx: Receiver<UiToHost>, shutdown: Arc<AtomicBool>) {
    bevy::log::info!("[GPUI] AI Workspace window thread started");
    let _ = ui_tx.send(UiToHost::RequestSnapshot);

    Application::new().run(move |cx: &mut App| {
        bind_keys(cx);
        let bounds = Bounds::centered(None, size(px(1100.0), px(800.0)), cx);
        let ui_tx = ui_tx.clone();
        let shutdown = shutdown.clone();

        let window: WindowHandle<AiWorkspaceWindow> = match cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions { title: Some(SharedString::from("AI Workspace")), appears_transparent: true, ..Default::default() }),
                ..Default::default()
            },
            move |window, cx| {
                cx.new(|cx| AiWorkspaceWindow::new(ui_tx.clone(), window, cx))
            },
        ) {
            Ok(w) => w,
            Err(e) => {
                bevy::log::error!("[GPUI] open_window failed: {e}");
                cx.quit();
                return;
            }
        };

        let host_rx = host_rx;
        let action_rx = action_rx;

        cx.spawn(async move |cx: &mut AsyncApp| {
            while !shutdown.load(Ordering::SeqCst) {
                while let Ok(action) = action_rx.try_recv() {
                    if matches!(action, UiToHost::Shutdown) { cx.update(|cx| cx.quit()); return; }
                }
                while let Ok(event) = host_rx.try_recv() {
                    if matches!(event, HostToUi::Shutdown) { cx.update(|cx| cx.quit()); return; }
                    if window
                        .update(cx, |this, window, cx| {
                            this.apply_event(event, window, cx);
                            cx.notify();
                        })
                        .is_err()
                    {
                        // Window was closed / destroyed; stop pumping events to avoid "window not found"
                        // and invalid HWND errors on Windows.
                        cx.update(|cx| cx.quit());
                        return;
                    }
                }
                // Drive editor animations (diff playback) at ~60fps even without host events.
                let _ = window.update(cx, |this, _window, cx| this.tick(cx));
                cx.background_executor().timer(std::time::Duration::from_millis(16)).await;
            }
            cx.update(|cx| cx.quit());
        }).detach();

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.activate(true);
    });
}

fn bind_keys(cx: &mut App) {
    use super::components::input_composer;
    use super::components::code_editor;
    use super::ui::{context_menu, picker, text_input};
    cx.bind_keys([
        // TextInput
        KeyBinding::new("backspace", text_input::Backspace, Some("TextInput")),
        KeyBinding::new("delete", text_input::Delete, Some("TextInput")),
        KeyBinding::new("left", text_input::Left, Some("TextInput")),
        KeyBinding::new("right", text_input::Right, Some("TextInput")),
        KeyBinding::new("shift-left", text_input::SelectLeft, Some("TextInput")),
        KeyBinding::new("shift-right", text_input::SelectRight, Some("TextInput")),
        KeyBinding::new("ctrl-a", text_input::SelectAll, Some("TextInput")),
        KeyBinding::new("cmd-a", text_input::SelectAll, Some("TextInput")),
        KeyBinding::new("ctrl-v", text_input::Paste, Some("TextInput")),
        KeyBinding::new("cmd-v", text_input::Paste, Some("TextInput")),
        KeyBinding::new("ctrl-c", text_input::Copy, Some("TextInput")),
        KeyBinding::new("cmd-c", text_input::Copy, Some("TextInput")),
        KeyBinding::new("ctrl-x", text_input::Cut, Some("TextInput")),
        KeyBinding::new("cmd-x", text_input::Cut, Some("TextInput")),
        KeyBinding::new("home", text_input::Home, Some("TextInput")),
        KeyBinding::new("end", text_input::End, Some("TextInput")),
        // InputComposer
        KeyBinding::new("enter", input_composer::Send, Some("InputComposer")),
        KeyBinding::new("shift-enter", input_composer::NewLine, Some("InputComposer")),
        KeyBinding::new("escape", input_composer::Cancel, Some("InputComposer")),
        KeyBinding::new("down", input_composer::AutocompleteNext, Some("InputComposer")),
        KeyBinding::new("up", input_composer::AutocompletePrev, Some("InputComposer")),
        KeyBinding::new("tab", input_composer::AutocompleteConfirm, Some("InputComposer")),
        // ContextMenu
        KeyBinding::new("escape", context_menu::Cancel, Some("ContextMenu")),
        KeyBinding::new("down", context_menu::SelectNext, Some("ContextMenu")),
        KeyBinding::new("up", context_menu::SelectPrev, Some("ContextMenu")),
        KeyBinding::new("enter", context_menu::Confirm, Some("ContextMenu")),
        // Picker
        KeyBinding::new("escape", picker::Cancel, Some("Picker")),
        KeyBinding::new("down", picker::SelectNext, Some("Picker")),
        KeyBinding::new("up", picker::SelectPrev, Some("Picker")),
        KeyBinding::new("enter", picker::Confirm, Some("Picker")),
        // Global
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("ctrl-m", ToggleModelSelector, Some("AiWorkspace")),
        KeyBinding::new("cmd-m", ToggleModelSelector, Some("AiWorkspace")),
        KeyBinding::new("ctrl-shift-m", CycleMode, Some("AiWorkspace")),
        KeyBinding::new("cmd-shift-m", CycleMode, Some("AiWorkspace")),
        // IDE shortcuts
        KeyBinding::new("ctrl-b", ToggleProjectPanel, Some("AiWorkspace")),
        KeyBinding::new("cmd-b", ToggleProjectPanel, Some("AiWorkspace")),
        KeyBinding::new("ctrl-s", SaveFile, Some("AiWorkspace")),
        KeyBinding::new("cmd-s", SaveFile, Some("AiWorkspace")),
        KeyBinding::new("ctrl-shift-s", SaveAllFiles, Some("AiWorkspace")),
        KeyBinding::new("cmd-shift-s", SaveAllFiles, Some("AiWorkspace")),
        // CodeEditor
        KeyBinding::new("ctrl-z", code_editor::Undo, Some("CodeEditor")),
        KeyBinding::new("cmd-z", code_editor::Undo, Some("CodeEditor")),
        KeyBinding::new("ctrl-y", code_editor::Redo, Some("CodeEditor")),
        KeyBinding::new("cmd-y", code_editor::Redo, Some("CodeEditor")),
        KeyBinding::new("ctrl-shift-z", code_editor::Redo, Some("CodeEditor")),
        KeyBinding::new("cmd-shift-z", code_editor::Redo, Some("CodeEditor")),
        KeyBinding::new("ctrl-space", code_editor::Complete, Some("CodeEditor")),
        KeyBinding::new("f12", code_editor::GotoDefinition, Some("CodeEditor")),
        KeyBinding::new("ctrl-alt-h", code_editor::Hover, Some("CodeEditor")),
        KeyBinding::new("ctrl-p", ToggleQuickOpen, Some("AiWorkspace")),
        KeyBinding::new("cmd-p", ToggleQuickOpen, Some("AiWorkspace")),
        KeyBinding::new("ctrl-shift-p", ToggleCommandPalette, Some("AiWorkspace")),
        KeyBinding::new("cmd-shift-p", ToggleCommandPalette, Some("AiWorkspace")),
    ]);
}

// ─────────────────────────────────────────────────────────────────────────────
// UI State
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct AiWorkspaceUiState {
    pub snapshot: WorkspaceSnapshot,
    pub active_session_id: Option<uuid::Uuid>,
    pub expanded_tools: std::collections::HashSet<u64>,
    pub voice_listening: bool,
}

impl AiWorkspaceUiState {
    pub fn apply_event(&mut self, event: HostToUi) {
        match event {
            HostToUi::Snapshot(snap) => { self.active_session_id = snap.active_session_id; self.snapshot = snap; }
            HostToUi::ActiveSessionChanged { session_id } => { self.active_session_id = session_id; }
            HostToUi::SessionEvent { session_id, event } => { self.apply_session_event(session_id, event); }
            HostToUi::ProvidersUpdated { profiles, active_idx } => { self.snapshot.providers = profiles; self.snapshot.active_provider_idx = active_idx; }
            HostToUi::VoiceAssistantSettingsUpdated(settings) => {
                self.snapshot.voice_assistant_enabled = settings.enabled;
                self.snapshot.voice_model = if settings.enabled {
                    if settings.use_gemini_live { VoiceModel::GeminiLive } else { VoiceModel::Legacy }
                } else {
                    VoiceModel::Off
                };
            }
            HostToUi::GeminiLiveStateChanged { active } => { self.snapshot.gemini_live_active = active; }
            HostToUi::GeminiLiveTranscript { text: _ } => { /* TODO: display user speech transcript */ }
            HostToUi::GeminiLiveResponse { text: _ } => { /* TODO: display AI response text */ }
            HostToUi::VoiceListeningChanged { listening } => { self.voice_listening = listening; }
            HostToUi::ComposerTextSet { .. } => { /* handled by window (UI-only) */ }
            HostToUi::IdeEvent(ide_event) => { self.apply_ide_event(ide_event); }
            HostToUi::Shutdown => {}
        }
    }

    fn apply_ide_event(&mut self, event: IdeEvent) {
        match event {
            IdeEvent::TreeRootChanged { path } => { self.snapshot.ide.root_path = Some(path); }
            IdeEvent::TreeEntriesUpdated { entries, .. } => { self.snapshot.ide.visible_entries = entries; }
            IdeEvent::TreeEntryExpanded { entry_id, children } => {
                // Update visible entries - find the entry and mark as expanded, insert children
                if let Some(idx) = self.snapshot.ide.visible_entries.iter().position(|e| e.id == entry_id) {
                    if let Some(e) = self.snapshot.ide.visible_entries.get_mut(idx) { e.is_expanded = true; e.kind = EntryKind::Dir; }
                    // Insert children after the parent
                    let insert_idx = idx + 1;
                    for (i, child) in children.into_iter().enumerate() {
                        self.snapshot.ide.visible_entries.insert(insert_idx + i, child);
                    }
                }
            }
            IdeEvent::TreeEntryCollapsed { entry_id } => {
                if let Some(e) = self.snapshot.ide.visible_entries.iter_mut().find(|e| e.id == entry_id) {
                    e.is_expanded = false;
                    let depth = e.depth;
                    // Remove all children (entries with depth > parent depth until next sibling)
                    let parent_idx = self.snapshot.ide.visible_entries.iter().position(|e| e.id == entry_id).unwrap_or(0);
                    let mut remove_end = parent_idx + 1;
                    while remove_end < self.snapshot.ide.visible_entries.len() && self.snapshot.ide.visible_entries[remove_end].depth > depth {
                        remove_end += 1;
                    }
                    self.snapshot.ide.visible_entries.drain(parent_idx + 1..remove_end);
                }
            }
            IdeEvent::TreeEntrySelected { entry_id } => {
                for e in &mut self.snapshot.ide.visible_entries { e.is_selected = Some(e.id) == entry_id; }
            }
            IdeEvent::TreeLoading { entry_id } => {
                if let Some(e) = self.snapshot.ide.visible_entries.iter_mut().find(|e| e.id == entry_id) { e.kind = EntryKind::PendingDir; }
            }
            IdeEvent::FileOpened { path, .. } => {
                if !self.snapshot.ide.open_files.iter().any(|f| f.path == path) {
                    self.snapshot.ide.open_files.push(OpenFileSnapshot { path: path.clone(), is_dirty: false, version: 1, cursor_line: 0, cursor_col: 0 });
                }
                self.snapshot.ide.active_file = Some(path);
            }
            IdeEvent::FileClosed { path } => {
                self.snapshot.ide.open_files.retain(|f| f.path != path);
                if self.snapshot.ide.active_file.as_ref() == Some(&path) {
                    self.snapshot.ide.active_file = self.snapshot.ide.open_files.first().map(|f| f.path.clone());
                }
            }
            IdeEvent::FileSaved { path, version } => {
                if let Some(f) = self.snapshot.ide.open_files.iter_mut().find(|f| f.path == path) { f.is_dirty = false; f.version = version; }
            }
            IdeEvent::FileDirtyChanged { path, is_dirty } => {
                if let Some(f) = self.snapshot.ide.open_files.iter_mut().find(|f| f.path == path) { f.is_dirty = is_dirty; }
            }
            IdeEvent::ActiveFileChanged { path } => { self.snapshot.ide.active_file = path; }
            _ => {}
        }
    }

    fn apply_session_event(&mut self, session_id: uuid::Uuid, event: SessionEventSnapshot) {
        let Some(session) = self.snapshot.sessions.iter_mut().find(|s| s.id == session_id) else { return; };
        match event {
            SessionEventSnapshot::TitleUpdated(title) => { session.title = title; }
            SessionEventSnapshot::BusyChanged { is_busy, reason, stage } => { session.is_busy = is_busy; session.busy_reason = reason; session.busy_stage = stage; }
            SessionEventSnapshot::Text(text) => { if let Some(EntrySnapshot::Assistant { content, state, .. }) = session.entries.last_mut() { content.push_str(&text); *state = MessageStateSnapshot::Streaming; } }
            SessionEventSnapshot::Thinking(text) => { if let Some(EntrySnapshot::Assistant { thinking, .. }) = session.entries.last_mut() { if let Some(t) = thinking { t.content.push_str(&text); } } }
            SessionEventSnapshot::EntryAdded { index, entry } => { if index <= session.entries.len() { session.entries.insert(index, entry); } }
            SessionEventSnapshot::EntryUpdated { index, entry } => { if let Some(e) = session.entries.get_mut(index) { *e = entry; } }
            SessionEventSnapshot::TokenUsageUpdated(usage) => { session.token_usage = usage; }
            SessionEventSnapshot::ToolExecutionStarted { request_id } => { if let Some(EntrySnapshot::ToolCall(c)) = session.entries.iter_mut().find(|e| matches!(e, EntrySnapshot::ToolCall(tc) if tc.id == request_id)) { c.status = ToolCallStatusSnapshot::InProgress; } }
            SessionEventSnapshot::ToolExecutionProgress { request_id, log } => { if let Some(EntrySnapshot::ToolCall(c)) = session.entries.iter_mut().find(|e| matches!(e, EntrySnapshot::ToolCall(tc) if tc.id == request_id)) { c.logs.push(log); } }
            SessionEventSnapshot::ToolExecutionSuccess { request_id, llm_result, raw_output } => { if let Some(EntrySnapshot::ToolCall(c)) = session.entries.iter_mut().find(|e| matches!(e, EntrySnapshot::ToolCall(tc) if tc.id == request_id)) { c.status = ToolCallStatusSnapshot::Completed; c.llm_result = Some(llm_result); c.raw_output = raw_output; } }
            SessionEventSnapshot::ToolExecutionError { request_id, error } => { if let Some(EntrySnapshot::ToolCall(c)) = session.entries.iter_mut().find(|e| matches!(e, EntrySnapshot::ToolCall(tc) if tc.id == request_id)) { c.status = ToolCallStatusSnapshot::Failed(error); } }
            SessionEventSnapshot::ToolExecutionCancelled { request_id } => { if let Some(EntrySnapshot::ToolCall(c)) = session.entries.iter_mut().find(|e| matches!(e, EntrySnapshot::ToolCall(tc) if tc.id == request_id)) { c.status = ToolCallStatusSnapshot::Canceled; } }
            SessionEventSnapshot::ToolRejected { request_id, reason } => { if let Some(EntrySnapshot::ToolCall(c)) = session.entries.iter_mut().find(|e| matches!(e, EntrySnapshot::ToolCall(tc) if tc.id == request_id)) { c.status = ToolCallStatusSnapshot::Rejected(reason); } }
            SessionEventSnapshot::ToolAwaitingApproval { request_id } => { if let Some(EntrySnapshot::ToolCall(c)) = session.entries.iter_mut().find(|e| matches!(e, EntrySnapshot::ToolCall(tc) if tc.id == request_id)) { c.status = ToolCallStatusSnapshot::AwaitingApproval; } }
            SessionEventSnapshot::Error(e) => {
                if let Some(EntrySnapshot::Assistant { content, state, .. }) = session.entries.last_mut() {
                    if !content.is_empty() && !content.ends_with('\n') { content.push('\n'); }
                    content.push_str("[Error] ");
                    content.push_str(&e);
                    content.push('\n');
                    *state = MessageStateSnapshot::Error;
                }
                session.is_busy = false;
                session.busy_reason = None;
                session.busy_stage = BusyStageSnapshot::Idle;
            }
            _ => {}
        }
    }

    pub fn active_session(&self) -> Option<&SessionSnapshot> { self.active_session_id.and_then(|id| self.snapshot.sessions.iter().find(|s| s.id == id)) }
}

// ─────────────────────────────────────────────────────────────────────────────
// GPUI Window View
// ─────────────────────────────────────────────────────────────────────────────

struct AiWorkspaceWindow {
    focus_handle: FocusHandle,
    state: AiWorkspaceUiState,
    thread_view: Entity<ThreadView>,
    input_composer: Entity<InputComposer>,
    session_list: Entity<SessionList>,
    mode_selector: Entity<ModeSelector>,
    model_selector_open: bool,
    model_selector: Option<Entity<ModelSelector>>,
    provider_settings_open: bool,
    provider_settings: Option<Entity<super::ui::popover::Popover>>,
    voice_settings_open: bool,
    voice_settings: Option<Entity<super::ui::popover::Popover>>,
    voice_settings_snapshot: VoiceAssistantSettingsSnapshot,
    project_panel: Entity<ProjectPanel>,
    project_panel_visible: bool,
    editor_tabs: Entity<EditorTabs>,
    quick_open: Option<Entity<QuickOpen>>,
    command_palette: Option<Entity<CommandPalette>>,
    ui_tx: Sender<UiToHost>,
    sidebar_width: f32,
    chat_width_ratio: f32,
    resize: Option<(ResizeTarget, f32, f32, f32)>,
    is_pinned: bool,
    api_setup_gate_opened: bool,
}

#[derive(Clone, Copy)]
enum ResizeTarget { Sidebar, Chat }

impl AiWorkspaceWindow {
    fn new(ui_tx: Sender<UiToHost>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let tx = ui_tx.clone();
        let thread_view = cx.new(|cx| ThreadView::new(None, tx.clone(), cx));
        let input_composer = cx.new(|cx| InputComposer::new(None, tx.clone(), window, cx));
        let session_list = cx.new(|cx| SessionList::new(Vec::new(), None, tx.clone(), window, cx));
        let mode_selector = cx.new(|cx| ModeSelector::new(cx));
        let project_panel = cx.new(|cx| ProjectPanel::new(tx.clone(), cx));
        let editor_tabs = cx.new(|cx| EditorTabs::new(tx.clone(), cx));
        focus_handle.focus(window, cx);
        // Set initial root to plugins directory
        if let Ok(cwd) = std::env::current_dir() {
            let plugins = cwd.join("plugins");
            let _ = tx.send(UiToHost::IdeSetRoot { path: if plugins.exists() { plugins } else { cwd } });
        }
        Self { focus_handle, state: AiWorkspaceUiState::default(), thread_view, input_composer, session_list, mode_selector, model_selector_open: false, model_selector: None, provider_settings_open: false, provider_settings: None, voice_settings_open: false, voice_settings: None, voice_settings_snapshot: VoiceAssistantSettingsSnapshot::default(), project_panel, project_panel_visible: true, editor_tabs, quick_open: None, command_palette: None, ui_tx, sidebar_width: 240.0, chat_width_ratio: 0.35, resize: None, is_pinned: false, api_setup_gate_opened: false }
    }

    fn resize_start(&mut self, target: ResizeTarget, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.resize = Some((target, f32::from(event.position.x), self.sidebar_width, self.chat_width_ratio));
        cx.notify();
    }
    fn resize_sidebar_start(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) { self.resize_start(ResizeTarget::Sidebar, event, window, cx); }
    fn resize_chat_start(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) { self.resize_start(ResizeTarget::Chat, event, window, cx); }

    fn resize_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some((target, start_x, start_sidebar, start_ratio)) = self.resize else { return; };
        let dx = f32::from(event.position.x) - start_x;
        match target {
            ResizeTarget::Sidebar => self.sidebar_width = (start_sidebar + dx).clamp(180.0, 400.0),
            ResizeTarget::Chat => {
                let win_w = f32::from(window.bounds().size.width);
                let content_w = (win_w - self.sidebar_width - 8.0).max(1.0);
                self.chat_width_ratio = (start_ratio - dx / content_w).clamp(0.30, 0.50);
            }
        }
        cx.notify();
    }

    fn resize_end(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) { if self.resize.take().is_some() { cx.notify(); } }

    fn apply_event(&mut self, event: HostToUi, window: &mut Window, cx: &mut Context<Self>) {
        // First apply event to state snapshot
        self.state.apply_event(event.clone());
        // Handle voice settings updates
        if let HostToUi::VoiceAssistantSettingsUpdated(ref settings) = event {
            self.voice_settings_snapshot = settings.clone();
        }
        // Handle IDE events for editor tabs, quick open, command palette
        if let HostToUi::IdeEvent(ref ide_event) = event {
            match ide_event {
                IdeEvent::FileOpened { path, content, version } => {
                    self.editor_tabs.update(cx, |tabs, cx| tabs.set_file_content(path.clone(), content.clone(), *version, cx));
                }
                IdeEvent::FileChanged { path, version, edits } => {
                    self.editor_tabs.update(cx, |tabs, cx| {
                        tabs.apply_file_changed(path.clone(), *version, edits.clone(), cx)
                    });
                }
                IdeEvent::CursorMoved { path, line, col } => {
                    self.editor_tabs.update(cx, |tabs, cx| tabs.set_cursor(path.clone(), *line, *col, cx));
                }
                IdeEvent::FileDirtyChanged { path, is_dirty } => {
                    self.editor_tabs.update(cx, |tabs, cx| tabs.mark_dirty(path, *is_dirty, cx));
                }
                IdeEvent::DiagnosticsUpdated { path, diagnostics } => {
                    self.editor_tabs.update(cx, |tabs, cx| tabs.set_diagnostics(path.clone(), diagnostics.clone(), cx));
                }
                IdeEvent::CompletionItems { path, items } => {
                    self.editor_tabs.update(cx, |tabs, cx| tabs.set_completions(path.clone(), items.clone(), cx));
                }
                IdeEvent::HoverText { path, markdown } => {
                    self.editor_tabs.update(cx, |tabs, cx| tabs.set_hover(path.clone(), markdown.clone(), cx));
                }
                IdeEvent::DefinitionLocation { from: _, to, line, col } => {
                    let _ = self.ui_tx.send(UiToHost::IdeGotoLine { path: to.clone(), line: *line, col: *col });
                }
                IdeEvent::QuickOpenResults { items } => {
                    if let Some(ref qo) = self.quick_open { qo.update(cx, |qo, cx| qo.set_results(items.clone(), cx)); }
                }
                IdeEvent::CommandPaletteResults { commands } => {
                    if let Some(ref cp) = self.command_palette { cp.update(cx, |cp, cx| cp.set_results(commands.clone(), cx)); }
                }
                _ => {}
            }
        }
        if let HostToUi::ComposerTextSet { session_id, text } = &event {
            if self.state.active_session_id == Some(*session_id) {
                let text = text.clone();
                self.input_composer.update(cx, |comp, cx| comp.set_text(text, cx));
                self.input_composer.update(cx, |comp, cx| comp.focus(window, cx));
            }
        }
        // Update project panel with IDE snapshot
        let ide_entries = self.state.snapshot.ide.visible_entries.clone();
        self.project_panel_visible = self.state.snapshot.ide.project_panel_visible;
        self.project_panel.update(cx, |panel, cx| panel.set_entries(ide_entries, cx));
        // Update editor tabs with open files
        let open_files = self.state.snapshot.ide.open_files.clone();
        let active_file = self.state.snapshot.ide.active_file.clone();
        self.editor_tabs.update(cx, |tabs, cx| tabs.set_open_files(open_files, active_file, cx));
        // Update chat components
        let active_session = self.state.active_session().cloned();
        let active_id = self.state.active_session_id;
        let is_busy = active_session.as_ref().map(|s| s.is_busy).unwrap_or(false);
        let active_file = self.state.snapshot.ide.active_file.clone();
        let voice_available = !matches!(self.state.snapshot.voice_model, VoiceModel::Off);
        let voice_active = !matches!(self.state.snapshot.voice_model, VoiceModel::Off)
            && (self.state.snapshot.gemini_live_active || self.state.voice_listening);
        self.thread_view.update(cx, |view, cx| view.set_session(active_session, cx));
        self.input_composer.update(cx, |comp, cx| {
            comp.set_session(active_id);
            comp.set_busy(is_busy, cx);
            comp.set_active_file(active_file.clone(), cx);
            comp.set_voice_active(voice_active, cx);
            comp.set_voice_available(voice_available, cx);
        });
        self.session_list.update(cx, |list, cx| list.set_sessions(self.state.snapshot.sessions.clone(), active_id, cx));

        let missing_gemini_key = self
            .state
            .snapshot
            .providers
            .first()
            .is_some_and(|p| p.backend_type == BackendType::Gemini && !p.has_api_key);
        if !missing_gemini_key {
            self.api_setup_gate_opened = false;
        }
    }

    fn toggle_model_selector(&mut self, _: &ToggleModelSelector, window: &mut Window, cx: &mut Context<Self>) {
        if self.model_selector_open {
            self.model_selector = None;
            self.model_selector_open = false;
        } else {
            let providers = self.state.snapshot.providers.clone();
            let active_idx = self.state.snapshot.active_provider_idx;
            let voice_model = self.state.snapshot.voice_model;
            let tx = self.ui_tx.clone();
            let selector = cx.new(|cx| ModelSelector::new(&providers, active_idx, voice_model, tx, window, cx));
            cx.subscribe(&selector, |this, _, _: &DismissEvent, cx| {
                this.model_selector = None;
                this.model_selector_open = false;
                cx.notify();
            }).detach();
            self.model_selector = Some(selector);
            self.model_selector_open = true;
        }
        cx.notify();
    }

    fn toggle_provider_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.provider_settings_open {
            self.provider_settings = None;
            self.provider_settings_open = false;
            cx.notify();
            return;
        }
        let providers = self.state.snapshot.providers.clone();
        let active_idx = self.state.snapshot.active_provider_idx;
        let tx = self.ui_tx.clone();
        let panel = cx.new(|cx| ProviderSettings::new(&providers, active_idx, tx, window, cx));
        let pop = cx.new(|cx| super::ui::popover::Popover::new(cx, move |_, _| panel.clone().into_any_element()));
        cx.subscribe(&pop, |this, _, _: &DismissEvent, cx| {
            this.provider_settings = None;
            this.provider_settings_open = false;
            cx.notify();
        }).detach();
        self.provider_settings = Some(pop);
        self.provider_settings_open = true;
        cx.notify();
    }

    fn toggle_voice_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.voice_settings_open {
            self.voice_settings = None;
            self.voice_settings_open = false;
            cx.notify();
            return;
        }
        // Request latest settings from host
        let _ = self.ui_tx.send(UiToHost::RequestVoiceAssistantSettings);
        let settings = self.voice_settings_snapshot.clone();
        let tx = self.ui_tx.clone();
        let panel = cx.new(|cx| VoiceAssistantSettings::new(&settings, tx, window, cx));
        let pop = cx.new(|cx| super::ui::popover::Popover::new(cx, move |_, _| panel.clone().into_any_element()));
        cx.subscribe(&pop, |this, _, _: &DismissEvent, cx| {
            this.voice_settings = None;
            this.voice_settings_open = false;
            cx.notify();
        }).detach();
        self.voice_settings = Some(pop);
        self.voice_settings_open = true;
        cx.notify();
    }

    fn cycle_mode(&mut self, _: &CycleMode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode_selector.update(cx, |m, cx| m.cycle(window, cx));
    }

    fn toggle_project_panel(&mut self, _: &ToggleProjectPanel, _window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.ui_tx.send(UiToHost::IdeToggleProjectPanel);
        cx.notify();
    }

    fn save_file(&mut self, _: &SaveFile, _window: &mut Window, _cx: &mut Context<Self>) {
        if let Some(path) = self.state.snapshot.ide.active_file.clone() {
            let _ = self.ui_tx.send(UiToHost::IdeSaveFile { path });
        }
    }

    fn save_all_files(&mut self, _: &SaveAllFiles, _window: &mut Window, _cx: &mut Context<Self>) {
        let _ = self.ui_tx.send(UiToHost::IdeSaveAllFiles);
    }

    fn toggle_quick_open(&mut self, _: &ToggleQuickOpen, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_open.is_some() {
            self.quick_open = None;
        } else {
            let tx = self.ui_tx.clone();
            let qo = cx.new(|cx| super::components::QuickOpen::new(tx, window, cx));
            cx.subscribe(&qo, |this, _, _: &DismissEvent, cx| {
                this.quick_open = None;
                cx.notify();
            }).detach();
            self.quick_open = Some(qo);
            // Trigger initial search
            let _ = self.ui_tx.send(UiToHost::IdeQuickOpenQuery { query: String::new() });
        }
        cx.notify();
    }

    fn toggle_command_palette(&mut self, _: &ToggleCommandPalette, window: &mut Window, cx: &mut Context<Self>) {
        if self.command_palette.is_some() {
            self.command_palette = None;
        } else {
            let tx = self.ui_tx.clone();
            let tx_cmd = self.ui_tx.clone();
            let cp = cx.new(|cx| super::components::CommandPalette::new(tx, move |cmd_id, window, cx| {
                // Handle command execution
                match cmd_id {
                    "new_session" => { let _ = tx_cmd.send(UiToHost::NewSession); }
                    "toggle_project_panel" => { let _ = tx_cmd.send(UiToHost::IdeToggleProjectPanel); }
                    "save_file" => { /* handled by active file */ }
                    "save_all" => { let _ = tx_cmd.send(UiToHost::IdeSaveAllFiles); }
                    _ => {}
                }
            }, window, cx));
            cx.subscribe(&cp, |this, _, _: &DismissEvent, cx| {
                this.command_palette = None;
                cx.notify();
            }).detach();
            self.command_palette = Some(cp);
            // Trigger initial command list
            let _ = self.ui_tx.send(UiToHost::IdeCommandPalette { query: String::new() });
        }
        cx.notify();
    }

    fn toggle_pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.is_pinned = !self.is_pinned;
        window.set_always_on_top(self.is_pinned);
        cx.notify();
    }

    fn current_model_name(&self) -> String {
        let providers = &self.state.snapshot.providers;
        let idx = self.state.snapshot.active_provider_idx;
        providers.get(idx).map(|p| p.selected_model.clone()).unwrap_or_else(|| "Select Model".into())
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        self.editor_tabs.update(cx, |tabs, cx| { let _ = tabs.tick(cx); });
    }
}

impl Focusable for AiWorkspaceWindow { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() } }

impl Render for AiWorkspaceWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_id = self.state.active_session_id.or(self.state.snapshot.active_session_id);
        let active_session = self.state.active_session().cloned();
        let model_name = self.current_model_name();

        let missing_gemini_key = self
            .state
            .snapshot
            .providers
            .first()
            .is_some_and(|p| p.backend_type == BackendType::Gemini && !p.has_api_key);
        if missing_gemini_key && !self.provider_settings_open && !self.api_setup_gate_opened {
            self.api_setup_gate_opened = true;
            self.toggle_provider_settings(window, cx);
        }

        // Left sidebar: plugins file tree (top) + sessions (bottom)
        let sidebar_w = self.sidebar_width;
        let sidebar = v_flex()
            .flex_none()
            .w(px(sidebar_w))
            .min_w(px(180.0))
            .max_w(px(400.0))
            .h_full()
            .bg(ThemeColors::bg_secondary())
            .overflow_hidden()
            .child(self.project_panel.clone())
            .child(self.session_list.clone());

        // Sidebar drag handle
        let sidebar_handle = drag_handle("sidebar-handle", ResizeDirection::Horizontal).on_mouse_down(MouseButton::Left, cx.listener(Self::resize_sidebar_start));

        // Header bar
        let tx_snap = self.ui_tx.clone();
        let tx_shut = self.ui_tx.clone();
        let tx_new = self.ui_tx.clone();
        let pin_icon = if self.is_pinned { "📌" } else { "📍" };
        let voice_on = self.state.snapshot.voice_assistant_enabled;
        let tx_voice = self.ui_tx.clone();
        let header = h_flex()
            .flex_none()
            .h(px(32.0))
            .px(Spacing::Base08.px())
            .gap(Spacing::Base06.px())
            .border_b_1()
            .border_color(ThemeColors::border())
            .bg(ThemeColors::bg_secondary())
            .child(Label::new("AI Workspace").size(LabelSize::Small).color(LabelColor::Primary))
            .child(div().flex_1().window_control_area(WindowControlArea::Drag))
            .child(self.mode_selector.clone())
            .child(Button::new("model-btn", model_name).style(ButtonStyle::Subtle).on_click(cx.listener(|this, _, window, cx| this.toggle_model_selector(&ToggleModelSelector, window, cx))))
            .child(Button::new("provider-settings", "⚙").style(ButtonStyle::Icon).on_click(cx.listener(|this, _, window, cx| this.toggle_provider_settings(window, cx))))
            .child(
                Button::new("voice-assistant", "🎙")
                    .style(ButtonStyle::Icon)
                    .toggle_state(voice_on)
                    .on_click(move |_, _, _| { let _ = tx_voice.send(UiToHost::SetVoiceAssistantEnabled { enabled: !voice_on }); })
            )
            .child(Button::new("voice-settings", "🔊").style(ButtonStyle::Icon).on_click(cx.listener(|this, _, window, cx| this.toggle_voice_settings(window, cx))))
            .child(Button::new("new-session", "+ New").style(ButtonStyle::Tinted(TintColor::Accent)).on_click(move |_, _, _| { let _ = tx_new.send(UiToHost::NewSession); }))
            .child(Button::new("pin-window", pin_icon).style(ButtonStyle::Icon).toggle_state(self.is_pinned).on_click(cx.listener(|this, _, window, cx| this.toggle_pin(window, cx))))
            .child(Button::new("refresh", "↻").style(ButtonStyle::Icon).on_click(move |_, _, _| { let _ = tx_snap.send(UiToHost::RequestSnapshot); }))
            .child(div().font_family(WINDOW_CHROME_ICON_FONT_FAMILY).window_control_area(WindowControlArea::Min).child(Button::new("minimize", WINDOW_CHROME_GLYPH_MIN).style(ButtonStyle::Icon).on_click(move |_, window, _| window.minimize_window())))
            .child(div().font_family(WINDOW_CHROME_ICON_FONT_FAMILY).window_control_area(WindowControlArea::Max).child(Button::new("maximize", WINDOW_CHROME_GLYPH_MAX).style(ButtonStyle::Icon).on_click(move |_, window, _| window.titlebar_double_click())))
            .child(div().font_family(WINDOW_CHROME_ICON_FONT_FAMILY).window_control_area(WindowControlArea::Close).child(Button::new("close", WINDOW_CHROME_GLYPH_CLOSE).style(ButtonStyle::Icon).on_click(move |_, _, _| { let _ = tx_shut.send(UiToHost::Shutdown); })));

        // Token usage bar
        let token_info = active_session.as_ref().map(|s| {
            let usage = &s.token_usage;
            let ratio = usage.ratio();
            h_flex()
                .flex_none()
                .w_full()
                .px(Spacing::Base06.px())
                .py(Spacing::Base02.px())
                .gap(Spacing::Base06.px())
                .bg(ThemeColors::bg_secondary())
                .border_t_1()
                .border_color(ThemeColors::border())
                .child(Label::new(format!("Tokens: {} in / {} out ({})", usage.input_tokens, usage.output_tokens, usage.total_tokens)).size(LabelSize::XSmall).color(LabelColor::Muted))
                .child(div().flex_1())
                .child(
                    div().w(px(100.0)).h(px(4.0)).bg(ThemeColors::bg_primary()).rounded_sm().overflow_hidden()
                        .child(div().h_full().w(px(100.0 * ratio)).bg(if usage.is_warning() { ThemeColors::text_warning() } else { ThemeColors::text_accent() }))
                )
        });

        // Model selector popover
        let model_selector_popover = self.model_selector.as_ref().map(|s| {
            deferred(anchored().snap_to_window().child(s.clone())).with_priority(2)
        });

        let provider_settings_popover = self.provider_settings.as_ref().map(|p| {
            deferred(anchored().snap_to_window().child(p.clone())).with_priority(3)
        });

        let voice_settings_popover = self.voice_settings.as_ref().map(|p| {
            deferred(anchored().snap_to_window().child(p.clone())).with_priority(3)
        });

        // Quick open popover
        let quick_open_popover = self.quick_open.as_ref().map(|qo| {
            deferred(anchored().snap_to_window().child(qo.clone())).with_priority(3)
        });

        // Command palette popover
        let command_palette_popover = self.command_palette.as_ref().map(|cp| {
            deferred(anchored().snap_to_window().child(cp.clone())).with_priority(3)
        });

        // Editor area (tabs + code editor)
        let editor_area = div()
            .flex_1()
            .h_full()
            .bg(ThemeColors::bg_primary())
            .overflow_hidden()
            .child(self.editor_tabs.clone());

        // Chat drag handle
        let chat_handle = drag_handle("chat-handle", ResizeDirection::Horizontal).on_mouse_down(MouseButton::Left, cx.listener(Self::resize_chat_start));

        // Chat content area (thread + input) - flexible 30%-50% width with horizontal padding
        let chat_ratio = self.chat_width_ratio;
        let chat_content = v_flex()
            .flex_none()
            .w(relative(chat_ratio))
            .min_w(px(360.0))
            .max_w(relative(0.5))
            .h_full()
            .px(Spacing::Base08.px())
            .bg(ThemeColors::bg_primary())
            .border_l_1()
            .border_color(ThemeColors::border())
            .overflow_hidden()
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(ThemeColors::bg_selected().opacity(0.25)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _window, cx| {
                let mut mentions = Vec::new();
                for p in paths.paths() {
                    let is_dir = std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false);
                    let path = p.to_string_lossy().to_string();
                    if is_dir {
                        mentions.push(super::ui::MentionKind::Directory { path });
                    } else {
                        mentions.push(super::ui::MentionKind::File { path });
                    }
                }
                this.input_composer.update(cx, |comp, cx| comp.push_mentions(mentions, cx));
            }))
            .child(div().flex_1().overflow_hidden().child(self.thread_view.clone()))
            .children(token_info)
            .child(self.input_composer.clone());

        // Main content area (editor + handle + chat)
        let main_content = h_flex()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .child(editor_area)
            .child(chat_handle)
            .child(chat_content);

        // Full layout with header
        let content_with_header = v_flex()
            .flex_1()
            .h_full()
            .bg(ThemeColors::bg_primary())
            .overflow_hidden()
            .child(header)
            .child(main_content);

        // Main layout: Sidebar | Handle | (Header + Editor + Chat)
        div()
            .id("ai-workspace-window")
            .key_context("AiWorkspace")
            .track_focus(&self.focus_handle)
            .on_mouse_move(cx.listener(Self::resize_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::resize_end))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::resize_end))
            .on_action(cx.listener(Self::toggle_model_selector))
            .on_action(cx.listener(Self::cycle_mode))
            .on_action(cx.listener(Self::toggle_project_panel))
            .on_action(cx.listener(Self::save_file))
            .on_action(cx.listener(Self::save_all_files))
            .on_action(cx.listener(Self::toggle_quick_open))
            .on_action(cx.listener(Self::toggle_command_palette))
            .flex()
            .flex_row()
            .size_full()
            .text_color(ThemeColors::text_primary())
            .bg(ThemeColors::bg_primary())
            .child(sidebar)
            .child(sidebar_handle)
            .child(content_with_header)
            .children(model_selector_popover)
            .children(provider_settings_popover)
            .children(voice_settings_popover)
            .children(quick_open_popover)
            .children(command_palette_popover)
    }
}
