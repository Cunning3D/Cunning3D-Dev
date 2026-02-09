//! AI Workspace Pane - Minimal GPUI Launcher
//! 
//! This pane only provides a button to launch the GPUI-based AI Workspace window.
//! All session/tool/LLM logic is handled by `ai_workspace_gpui::host::AiWorkspaceHost`.

use super::tools::{
    graph_ops, graph_script, knowledge_ops, rust_nodespec_ops, rust_plugin_ops, workspace_ops, ToolRegistry,
};
use crate::ai_workspace_gpui::app::GpuiWindowHandle;
use crate::ai_workspace_gpui::protocol::UiToHost;
use crate::tabs_system::{EditorTab, EditorTabContext};
use bevy_egui::egui::{self, Ui, WidgetText};
use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

pub struct AiWorkspacePane {
    tool_registry: Arc<ToolRegistry>,
    graph_tools_registered: bool,

    // GPUI Window
    gpui_window_handle: Option<GpuiWindowHandle>,
    gpui_ui_to_host_tx: Option<Sender<UiToHost>>,
    gpui_host_thread: Option<JoinHandle<()>>,
    gpui_host_shutdown: Option<Arc<AtomicBool>>,
}

impl Default for AiWorkspacePane {
    fn default() -> Self {
        let mut registry = ToolRegistry::new();
        // Base tools (no graph access yet)
        registry.register(workspace_ops::ExploreWorkspaceTool);
        registry.register(workspace_ops::SearchWorkspaceTool);
        registry.register(workspace_ops::ReadFileTool);
        registry.register(workspace_ops::WebSearchTool);
        registry.register(workspace_ops::GetNodeSpecTemplateTool);
        registry.register(workspace_ops::GetAbiReferenceTool);
        registry.register(workspace_ops::GetInteractionGuideTool);
        registry.register(workspace_ops::GetInteractionTemplateTool);
        registry.register(workspace_ops::GenerateReplayTool);
        registry.register(workspace_ops::ComparePluginsTool);
        registry.register(workspace_ops::TerminalTool);
        registry.register(workspace_ops::DiagnosticsTool);
        registry.register(rust_nodespec_ops::ExtractUserCodeTool);

        Self {
            tool_registry: Arc::new(registry),
            graph_tools_registered: false,
            gpui_window_handle: None,
            gpui_ui_to_host_tx: None,
            gpui_host_thread: None,
            gpui_host_shutdown: None,
        }
    }
}

impl EditorTab for AiWorkspacePane {
    fn title(&self) -> WidgetText {
        "AI Workspace".into()
    }

    fn ui(&mut self, ui: &mut Ui, context: &mut EditorTabContext) {
        // Lazy Tool Registration with Graph Access
        if !self.graph_tools_registered {
            let registry_arc = Arc::new(context.node_registry.clone());

            let mut registry = ToolRegistry::new();
            // Base tools
            registry.register(workspace_ops::ExploreWorkspaceTool);
            registry.register(workspace_ops::SearchWorkspaceTool);
            registry.register(workspace_ops::ReadFileTool);
            registry.register(workspace_ops::WebSearchTool);
            registry.register(workspace_ops::GetNodeSpecTemplateTool);
            registry.register(workspace_ops::GetAbiReferenceTool);
            registry.register(workspace_ops::GetInteractionGuideTool);
            registry.register(workspace_ops::GetInteractionTemplateTool);
            registry.register(workspace_ops::GenerateReplayTool);
            registry.register(workspace_ops::ComparePluginsTool);
            registry.register(workspace_ops::TerminalTool);
            registry.register(workspace_ops::DiagnosticsTool);
            registry.register(rust_nodespec_ops::ExtractUserCodeTool);
            registry.register(rust_plugin_ops::CreateRustPluginTool);
            registry.register(rust_plugin_ops::CompileRustPluginTool);
            registry.register(rust_plugin_ops::LoadRustPluginsTool::new(registry_arc.clone()));
            registry.register(rust_plugin_ops::RevertRustPluginBuildTool::new(registry_arc.clone()));
            registry.register(rust_nodespec_ops::ApplyRustNodeSpecTool::new(registry_arc.clone()));

            // Graph tools
            registry.register(graph_ops::CreateNodeTool::new(registry_arc.clone()));
            registry.register(graph_ops::DeleteNodeTool::new());
            registry.register(graph_ops::ConnectNodeTool::new());
            registry.register(graph_ops::SetNodeFlagTool::new());
            registry.register(graph_ops::SetParameterTool::new());
            registry.register(graph_ops::GetGraphStateTool::new());
            registry.register(graph_ops::EditNodeGraphTool::new(registry_arc.clone()));
            registry.register(graph_ops::GetGeometryInsightTool::new());
            registry.register(graph_ops::GetNodeInfoTool::new(registry_arc.clone()));
            registry.register(graph_ops::GetNodeLibraryTool::new(registry_arc.clone()));
            registry.register(graph_ops::ExportNodeSpecTool::new(registry_arc.clone()));
            registry.register(graph_ops::CompareGeometryTool::new());
            registry.register(knowledge_ops::SearchKnowledgeTool);
            registry.register(knowledge_ops::ReadKnowledgeTool);
            registry.register(knowledge_ops::BuildKnowledgePackTool);
            registry.register(graph_script::RunGraphScriptTool::new(registry_arc.clone()));

            self.tool_registry = Arc::new(registry);
            self.graph_tools_registered = true;
        }

        // Check GPUI window status
        if let Some(h) = &self.gpui_window_handle {
            if !h.is_running() {
                self.shutdown_gpui_host();
                self.gpui_window_handle = None;
                self.gpui_ui_to_host_tx = None;
            }
        }

        // UI: Single centered button
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() / 3.0);

            let gpui_running = self
                .gpui_window_handle
                .as_ref()
                .map(|h| h.is_running())
                .unwrap_or(false);

            if gpui_running {
                ui.label("GPUI AI Workspace is running");
                if ui.button("⬚ Focus Window").clicked() {
                    // TODO: bring window to front
                }
            } else {
                ui.heading("AI Workspace");
                ui.add_space(16.0);
                if ui
                    .button("⬚ Open GPUI Window")
                    .on_hover_text("Launch AI Workspace in separate GPUI window (Zed-like)")
                    .clicked()
                {
                    self.launch_gpui_window();
                }
            }
        });
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn retained_key(&self, _ui: &egui::Ui, _context: &EditorTabContext) -> u64 {
        0
    }
}

impl AiWorkspacePane {
    /// Launch GPUI AI Workspace window in separate thread
    fn launch_gpui_window(&mut self) {
        use crate::ai_workspace_gpui::app::launch_gpui_window;
        use crate::ai_workspace_gpui::protocol::{HostToBevy, HostToUi, UiToHost};
        use crate::ai_workspace_gpui::AiWorkspaceHost;
        use crossbeam_channel::unbounded;

        // Create channels for Host↔UI communication
        let (host_to_ui_tx, host_to_ui_rx) = unbounded::<HostToUi>();
        let (ui_to_host_tx, ui_to_host_rx) = unbounded::<UiToHost>();

        // Start Host actor thread
        let tool_registry = self.tool_registry.clone();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        let host_thread = std::thread::spawn(move || {
            let (bevy_tx, _bevy_rx) = unbounded::<HostToBevy>();
            let mut host = AiWorkspaceHost::new(tool_registry, host_to_ui_tx, bevy_tx);
            let tick = std::time::Duration::from_millis(16);
            loop {
                if shutdown_clone.load(Ordering::SeqCst) {
                    host.handle_action(UiToHost::Shutdown);
                    host.poll();
                    break;
                }

                // Drain UI actions
                loop {
                    match ui_to_host_rx.try_recv() {
                        Ok(action) => {
                            if matches!(action, UiToHost::Shutdown) {
                                host.handle_action(UiToHost::Shutdown);
                                host.poll();
                                return;
                            }
                            host.handle_action(action);
                        }
                        Err(crossbeam_channel::TryRecvError::Empty) => break,
                        Err(crossbeam_channel::TryRecvError::Disconnected) => {
                            host.handle_action(UiToHost::Shutdown);
                            host.poll();
                            return;
                        }
                    }
                }

                host.poll();
                std::thread::sleep(tick);
            }
        });

        // Launch GPUI window
        let ui_to_host_tx_for_window = ui_to_host_tx.clone();
        let handle = launch_gpui_window(host_to_ui_rx, ui_to_host_tx_for_window);

        self.gpui_window_handle = Some(handle);
        self.gpui_ui_to_host_tx = Some(ui_to_host_tx);
        self.gpui_host_thread = Some(host_thread);
        self.gpui_host_shutdown = Some(shutdown);
        bevy::log::info!("[AI Workspace] GPUI window launched");
    }

    fn shutdown_gpui_host(&mut self) {
        if let Some(tx) = self.gpui_ui_to_host_tx.as_ref() {
            let _ = tx.send(UiToHost::Shutdown);
        }
        if let Some(flag) = self.gpui_host_shutdown.as_ref() {
            flag.store(true, Ordering::SeqCst);
        }
        if let Some(t) = self.gpui_host_thread.take() {
            let _ = t.join();
        }
        self.gpui_host_shutdown = None;
    }
}
