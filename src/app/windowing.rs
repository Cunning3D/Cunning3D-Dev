use bevy::prelude::*;
use bevy::camera::RenderTarget;
use bevy::window::{MonitorSelection, WindowClosed, WindowPosition, WindowRef};
use crossbeam_channel::Receiver;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::{
    tabs_system,
    theme::ModernTheme,
    ui::{self, FloatTabToWindowEvent, FloatingTabRegistry, FloatingWindowEntry},
    viewport_options::OpenNaiveWindowEvent,
    MainCamera,
};

/// Bevy Resource to manage GPUI AI Workspace window lifecycle
#[derive(Resource, Default)]
pub struct GpuiAiWorkspaceState {
    window_handle: Option<crate::ai_workspace_gpui::app::GpuiWindowHandle>,
    ui_to_host_tx: Option<crossbeam_channel::Sender<crate::ai_workspace_gpui::protocol::UiToHost>>,
    host_thread: Option<JoinHandle<()>>,
    host_shutdown: Option<Arc<AtomicBool>>,
    tool_registry: Option<Arc<crate::tabs_registry::ai_workspace::tools::ToolRegistry>>,
    voice_bridge_rx: Option<Receiver<crate::ai_workspace_gpui::protocol::HostToBevy>>,
}

impl GpuiAiWorkspaceState {
    pub fn is_running(&self) -> bool {
        self.window_handle
            .as_ref()
            .map(|h| h.is_running())
            .unwrap_or(false)
    }

    pub fn try_send(&self, action: crate::ai_workspace_gpui::protocol::UiToHost) -> bool {
        self.ui_to_host_tx
            .as_ref()
            .and_then(|tx| tx.try_send(action).ok())
            .is_some()
    }

    pub fn launch_if_not_running(
        &mut self,
        node_registry: crate::cunning_core::registries::node_registry::NodeRegistry,
    ) {
        // Check if already running
        if let Some(h) = &self.window_handle {
            if h.is_running() {
                return;
            }
            self.shutdown();
        }

        use crate::ai_workspace_gpui::app::launch_gpui_window;
        use crate::ai_workspace_gpui::protocol::{HostToUi, UiToHost};
        use crate::ai_workspace_gpui::AiWorkspaceHost;
        use crate::tabs_registry::ai_workspace::tools::{
            graph_ops, graph_script, rust_nodespec_ops, rust_plugin_ops, workspace_ops, ToolRegistry,
        };
        use crossbeam_channel::unbounded;

        // Build tool registry with graph access
        let registry_arc = Arc::new(node_registry);
        let mut registry = ToolRegistry::new();
        registry.register(workspace_ops::ExploreWorkspaceTool);
        registry.register(workspace_ops::SearchWorkspaceTool);
        registry.register(workspace_ops::ReadFileTool);
        registry.register(workspace_ops::PatchFileTool);
        registry.register(workspace_ops::WriteFileTool);
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
        registry.register(graph_ops::CreateNodeTool::new(registry_arc.clone()));
        registry.register(graph_ops::DeleteNodeTool::new());
        registry.register(graph_ops::ConnectNodeTool::new());
        registry.register(graph_ops::SetNodeFlagTool::new());
        registry.register(graph_ops::SetParameterTool::new());
        registry.register(graph_ops::GetGraphStateTool::new());
        registry.register(graph_ops::EditNodeGraphTool::new(registry_arc.clone()));
        registry.register(graph_ops::GetGeometryInsightTool::new());
        registry.register(graph_script::RunGraphScriptTool::new(registry_arc.clone()));

        let tool_registry = Arc::new(registry);
        self.tool_registry = Some(tool_registry.clone());

        // Create channels
        let (host_to_ui_tx, host_to_ui_rx) = unbounded::<HostToUi>();
        let (ui_to_host_tx, ui_to_host_rx) = unbounded::<UiToHost>();
        let (voice_bridge_tx, voice_bridge_rx) = unbounded::<crate::ai_workspace_gpui::protocol::HostToBevy>();

        // Start Host actor thread
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        let tool_reg = tool_registry.clone();
        let host_thread = std::thread::spawn(move || {
            let mut host = AiWorkspaceHost::new(tool_reg, host_to_ui_tx, voice_bridge_tx.clone());
            let tick = std::time::Duration::from_millis(16);
            loop {
                if shutdown_clone.load(Ordering::SeqCst) {
                    host.handle_action(UiToHost::Shutdown);
                    host.poll();
                    break;
                }
                loop {
                    match ui_to_host_rx.try_recv() {
                        Ok(action) => {
                            if matches!(action, UiToHost::Shutdown) {
                                host.handle_action(UiToHost::Shutdown);
                                host.poll();
                                return;
                            }
                            if let UiToHost::SetVoiceAssistantEnabled { enabled } = action {
                                let _ = voice_bridge_tx.try_send(crate::ai_workspace_gpui::protocol::HostToBevy::VoiceSetAssistantEnabled { enabled });
                                host.handle_action(UiToHost::SetVoiceAssistantEnabled { enabled });
                                continue;
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
        let handle = launch_gpui_window(host_to_ui_rx, ui_to_host_tx.clone());

        self.window_handle = Some(handle);
        self.ui_to_host_tx = Some(ui_to_host_tx);
        self.host_thread = Some(host_thread);
        self.host_shutdown = Some(shutdown);
        self.voice_bridge_rx = Some(voice_bridge_rx);
        bevy::log::info!("[AI Workspace] GPUI window launched directly from dock");
    }

    fn shutdown(&mut self) {
        if let Some(tx) = self.ui_to_host_tx.as_ref() {
            let _ = tx.send(crate::ai_workspace_gpui::protocol::UiToHost::Shutdown);
        }
        if let Some(flag) = self.host_shutdown.as_ref() {
            flag.store(true, Ordering::SeqCst);
        }
        if let Some(t) = self.host_thread.take() {
            let _ = t.join();
        }
        self.window_handle = None;
        self.ui_to_host_tx = None;
        self.host_shutdown = None;
        self.tool_registry = None;
        self.voice_bridge_rx = None;
    }
}

pub fn gpui_ai_workspace_voice_bridge_system(
    mut gpui: ResMut<GpuiAiWorkspaceState>,
    mut stores: ResMut<crate::settings::SettingsStores>,
    mut cfg: Option<ResMut<crate::voice_assistant::AiVoiceAssistantConfig>>,
    voice: Option<Res<crate::voice::VoiceService>>,
) {
    let Some(rx) = gpui.voice_bridge_rx.as_ref() else { return; };
    while let Ok(ev) = rx.try_recv() {
        match ev {
            crate::ai_workspace_gpui::protocol::HostToBevy::VoiceSetAssistantEnabled { enabled } => {
                stores.user.set("voice.assistant.enabled".into(), crate::settings::SettingValue::Bool(enabled));
                stores.save_user();
                if let Some(mut c) = cfg.take() { c.enabled = enabled; }
                if !enabled {
                    if let Some(v) = voice.as_ref() {
                        v.send(crate::voice::VoiceCommand::StopSpeaking);
                        v.send(crate::voice::VoiceCommand::SetMode(crate::voice::VoiceMode::Off));
                    }
                }
            }
            crate::ai_workspace_gpui::protocol::HostToBevy::VoiceSpeak { text } => {
                let can_speak = gpui.is_running();
                if can_speak {
                    if let Some(v) = voice.as_ref() {
                        v.send(crate::voice::VoiceCommand::Speak(text));
                    }
                }
            }
            crate::ai_workspace_gpui::protocol::HostToBevy::VoiceStopSpeaking => {
                if let Some(v) = voice.as_ref() {
                    v.send(crate::voice::VoiceCommand::StopSpeaking);
                }
            }
            crate::ai_workspace_gpui::protocol::HostToBevy::VoiceStartListening => {
                if let Some(v) = voice.as_ref() {
                    v.send(crate::voice::VoiceCommand::StartListening);
                }
            }
            crate::ai_workspace_gpui::protocol::HostToBevy::VoiceStopListening => {
                if let Some(v) = voice.as_ref() {
                    v.send(crate::voice::VoiceCommand::StopListening);
                }
            }
            crate::ai_workspace_gpui::protocol::HostToBevy::StartGeminiLive { api_key, system_instruction, tools } => {
                // Bridge GPUI → Bevy: start Gemini Live session.
                // NOTE: The actual streaming is implemented in `crate::voice::gemini_live`.
                if let Some(v) = voice.as_ref() {
                    v.send(crate::voice::VoiceCommand::SetMode(crate::voice::VoiceMode::GeminiLive));
                    v.send(crate::voice::VoiceCommand::StartGeminiLive { api_key, system_instruction, tools });
                }
            }
            crate::ai_workspace_gpui::protocol::HostToBevy::StopGeminiLive => {
                if let Some(v) = voice.as_ref() {
                    v.send(crate::voice::VoiceCommand::StopGeminiLive);
                    v.send(crate::voice::VoiceCommand::SetMode(crate::voice::VoiceMode::Off));
                }
            }
            crate::ai_workspace_gpui::protocol::HostToBevy::SendGeminiLiveText { text } => {
                if let Some(v) = voice.as_ref() {
                    v.send(crate::voice::VoiceCommand::SendGeminiLiveText { text });
                }
            }
            crate::ai_workspace_gpui::protocol::HostToBevy::SendGeminiLiveToolResponse { id, name, response } => {
                if let Some(v) = voice.as_ref() {
                    v.send(crate::voice::VoiceCommand::SendGeminiLiveToolResponse { id, name, response });
                }
            }
        }
    }
}

#[derive(Component)]
pub struct NaiveCamera;

#[derive(Resource, Default)]
pub struct PendingNaiveWindows(pub Vec<Entity>);

pub fn spawn_naive_window_system(
    mut commands: Commands,
    mut events: MessageReader<OpenNaiveWindowEvent>,
    mut pending: ResMut<PendingNaiveWindows>,
) {
    for _ in events.read() {
        let window_entity = commands
            .spawn(Window {
                title: "N Polygon To Unity Test Scene".to_string(),
                resolution: bevy::window::WindowResolution::new(800, 600),
                ..default()
            })
            .id();
        pending.0.push(window_entity);
    }
}

pub fn spawn_naive_camera_after_window_ready_system(
    mut commands: Commands,
    windows: Query<(), With<Window>>,
    mut pending: ResMut<PendingNaiveWindows>,
) {
    pending.0.retain(|&window_entity| {
        if windows.get(window_entity).is_err() {
            return true;
        }
        commands.spawn((
            Camera::default(),
            Camera3d::default(),
            Projection::Perspective(PerspectiveProjection::default()),
            Transform::from_xyz(-2.0, 2.5, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
            RenderTarget::Window(WindowRef::Entity(window_entity)),
            NaiveCamera,
            Msaa::Sample4,
            bevy::core_pipeline::prepass::DepthPrepass,
        ));
        false
    });
}

pub fn sync_naive_camera_system(
    main_cam_query: Query<&Transform, (With<MainCamera>, Without<NaiveCamera>)>,
    mut naive_cam_query: Query<&mut Transform, With<NaiveCamera>>,
) {
    if let Ok(main_transform) = main_cam_query.single() {
        for mut naive_transform in naive_cam_query.iter_mut() {
            *naive_transform = *main_transform;
        }
    }
}

pub fn handle_float_tab_window_system(
    mut commands: Commands,
    mut events: MessageReader<FloatTabToWindowEvent>,
    mut registry: ResMut<FloatingTabRegistry>,
    theme: Res<ModernTheme>,
) {
    for event in events.read() {
        let size = event.initial_rect.size();
        let window_entity = commands
            .spawn((
                Window {
                    title: event.title.clone(),
                    resolution: bevy::window::WindowResolution::new(size.x.max(1000.0) as u32, size.y.max(1000.0) as u32),
                    position: WindowPosition::At(bevy::math::IVec2::new(event.initial_rect.min.x as i32, event.initial_rect.min.y as i32)),
                    ..default()
                },
                bevy_egui::EguiContext::default(),
                bevy_egui::EguiRenderOutput::default(),
                bevy_egui::EguiInput::default(),
                bevy_egui::EguiOutput::default(),
                bevy_egui::WindowSize::default(),
                crate::ui::NeedsEguiFontsInit,
            ))
            .id();
        let _ = &theme;
        registry.floating_windows.insert(
            window_entity,
            FloatingWindowEntry {
                title: event.title.clone(),
                id: event.id.clone(),
            },
        );
    }
}

pub fn handle_open_settings_window_system(
    mut commands: Commands,
    mut events: MessageReader<ui::OpenSettingsWindowEvent>,
    mut registry: ResMut<FloatingTabRegistry>,
    mut floating_tabs: ResMut<tabs_system::FloatingEditorTabs>,
    theme: Res<ModernTheme>,
) {
    if events.is_empty() {
        return;
    }
    for _ in events.read() {
        if registry.floating_windows.values().any(|e| e.title == "Settings") {
            continue;
        }
        let id = crate::ui::FloatingTabId(uuid::Uuid::from_u128(0x5E7714A0D6A5457B9E6C4D8C6D8B5E77));
        floating_tabs
            .tabs
            .entry(id.clone())
            .or_insert_with(|| Box::new(tabs_system::pane::settings::SettingsPane::default()));
        let window_entity = commands
            .spawn((
                Window {
                    title: "Settings".into(),
                    resolution: bevy::window::WindowResolution::new(980, 720),
                    position: WindowPosition::Centered(MonitorSelection::Primary),
                    ..default()
                },
                bevy_egui::EguiContext::default(),
                bevy_egui::EguiRenderOutput::default(),
                bevy_egui::EguiInput::default(),
                bevy_egui::EguiOutput::default(),
                bevy_egui::WindowSize::default(),
                crate::ui::NeedsEguiFontsInit,
            ))
            .id();
        let _ = &theme;
        registry.floating_windows.insert(
            window_entity,
            FloatingWindowEntry {
                title: "Settings".into(),
                id,
            },
        );
    }
}

pub fn handle_open_ai_workspace_window_system(
    mut events: MessageReader<ui::OpenAiWorkspaceWindowEvent>,
    mut ui_state: ResMut<ui::UiState>,
    mut gpui_state: ResMut<GpuiAiWorkspaceState>,
    node_graph_res: Res<crate::nodes::NodeGraphResource>,
    node_registry: Res<crate::cunning_core::registries::node_registry::NodeRegistry>,
) {
    // Check pending flag from pane command
    let should_open = ui_state.pending_open_ai_workspace || !events.is_empty();
    if ui_state.pending_open_ai_workspace {
        ui_state.pending_open_ai_workspace = false;
    }
    for _ in events.read() {}
    
    if should_open {
        // Launch GPUI window directly (no egui tab)
        let _ = &node_graph_res;
        gpui_state.launch_if_not_running(node_registry.clone());
    }
}

/// Spawn the Hot Reload popup window (like UE's compile progress window).
pub fn handle_open_hot_reload_window_system(
    mut commands: Commands,
    mut events: MessageReader<ui::OpenHotReloadWindowEvent>,
    mut registry: ResMut<FloatingTabRegistry>,
    mut floating_tabs: ResMut<tabs_system::FloatingEditorTabs>,
    hot_log: Res<crate::tabs_system::pane::hot_reload::HotReloadLog>,
    jobs_snap: Res<crate::tabs_system::pane::hot_reload::HotReloadJobsSnapshot>,
    theme: Res<ModernTheme>,
) {
    if events.is_empty() { return; }
    for _ in events.read() {
        // Prevent duplicate windows
        if registry.floating_windows.values().any(|e| e.title == "Hot Reload") { continue; }
        let id = crate::ui::FloatingTabId(uuid::Uuid::from_u128(0xC3D_407_0E10AD_0000_0000_0001u128));
        floating_tabs.tabs.entry(id.clone()).or_insert_with(|| {
            Box::new(crate::tabs_system::pane::hot_reload::HotReloadTab::new(hot_log.clone(), jobs_snap.clone()))
        });
        let window_entity = commands.spawn((
            Window {
                title: "Hot Reload".into(),
                resolution: bevy::window::WindowResolution::new(720, 480),
                position: WindowPosition::Centered(MonitorSelection::Primary),
                ..default()
            },
            bevy_egui::EguiContext::default(),
            bevy_egui::EguiRenderOutput::default(),
            bevy_egui::EguiInput::default(),
            bevy_egui::EguiOutput::default(),
            bevy_egui::WindowSize::default(),
            crate::ui::NeedsEguiFontsInit,
        )).id();
        let _ = &theme;
        registry.floating_windows.insert(window_entity, FloatingWindowEntry { title: "Hot Reload".into(), id });
    }
}

pub fn handle_open_node_info_window_system(
    mut events: MessageReader<ui::OpenNodeInfoWindowEvent>,
    mut float_writer: MessageWriter<FloatTabToWindowEvent>,
    mut floating_tabs: ResMut<tabs_system::FloatingEditorTabs>,
) {
    for e in events.read() {
        let id = crate::ui::FloatingTabId(uuid::Uuid::new_v4());
        floating_tabs
            .tabs
            .insert(id.clone(), Box::new(crate::tabs_system::node_editor::node_info_tab::NodeInfoTab::new(e.node_id)));
        float_writer.write(FloatTabToWindowEvent {
            title: "Node Info".into(),
            initial_rect: e.initial_rect,
            id,
        });
    }
}

pub fn cleanup_floating_windows_on_close_system(
    mut registry: ResMut<FloatingTabRegistry>,
    mut closed_events: MessageReader<WindowClosed>,
    mut floating_tabs: ResMut<tabs_system::FloatingEditorTabs>,
) {
    for evt in closed_events.read() {
        if let Some(entry) = registry.floating_windows.remove(&evt.window) {
            floating_tabs.tabs.remove(&entry.id);
        }
    }
}

