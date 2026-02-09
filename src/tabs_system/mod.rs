use bevy::ecs::system::{SystemParam, SystemState};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::{
    egui::{self, Ui},
    EguiContext, EguiInput, EguiOutput, EguiRenderOutput, WindowSize,
};
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer as EguiTabViewer};

use crate::cunning_core::profiling::PerformanceMonitor;
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::cunning_core::registries::tab_registry::TabRegistry;
use crate::cunning_core::scripting::ScriptNodeState;
use crate::cunning_core::traits::node_interface::{GizmoState, ServiceProvider};
use crate::libs::ai_service::gemini::copilot_host::GeminiCopilotHost;
use crate::libs::ai_service::native_candle::{AiResultEvent, NativeAiHost, NativeAiInbox};
use crate::libs::ai_service::native_tiny_model::TinyModelHost;
use crate::libs::voice::VoiceService;
use crate::nodes::NodeGraphResource;
use crate::tabs_system::node_editor::connection_hint::service::ConnectionHintState;
use crate::{
    console::ConsoleLog,
    invalidator::GraphRevision,
    theme::ModernTheme,
    ui::{FloatTabToWindowEvent, FloatingTabId, MobileTab, NodeEditorState, UiState},
    viewport_options::{DisplayOptions, OpenNaiveWindowEvent},
    GraphChanged, ViewportInteractionState,
};
use uuid::Uuid;
use std::any::{Any, TypeId};
use std::collections::HashMap;

mod rect_hash;
use rect_hash::mix_rect;

pub use crate::tabs_registry::ai_workspace::pane::AiWorkspacePane;
pub use codex_tab::CodexTab;
pub use console_tab::ConsoleTab;
pub use geometry_spreadsheet_tab::GeometrySpreadsheetTab;
pub use node_editor::NodeEditorTab;
pub use node_properties_tab::NodePropertiesTab;
pub use outliner_tab::OutlinerTab;
pub use pane::settings::SettingsPane;
pub use toolbar::Toolbar;
pub use viewport_3d::Viewport3DTab;

#[path = "./codex_tab.rs"]
pub mod codex_tab;
#[path = "./console_tab.rs"]
pub mod console_tab;
#[path = "./geometry_spreadsheet_tab.rs"]
pub mod geometry_spreadsheet_tab;
pub mod hud_actions;
pub mod node_editor;
#[path = "./node_properties_tab.rs"]
pub mod node_properties_tab;
#[path = "./outliner_tab.rs"]
pub mod outliner_tab;
pub mod pane;
#[path = "./timeline.rs"]
pub mod timeline;
#[path = "./toolbar.rs"]
pub mod toolbar;
pub mod viewport_3d;

/// Context passed to every EditorTab during `ui()`.
/// This aggregates all the Bevy resources/states the tab might need.
/// We use a single context struct to avoid passing 10+ arguments to `ui()`.
pub struct EditorTabContext<'a> {
    pub ui_state: &'a mut UiState,
    pub ui_settings: &'a mut crate::ui_settings::UiSettings,
    pub ui_invalidator: &'a mut crate::invalidator::UiInvalidator,
    pub graph_revision: u64,
    pub node_editor_settings: &'a crate::node_editor_settings::NodeEditorSettings,
    pub settings_registry: &'a crate::settings::SettingsRegistry,
    pub settings_stores: &'a mut crate::settings::SettingsStores,
    pub node_graph_res: &'a mut NodeGraphResource,
    pub viewport_interaction_state: &'a mut ViewportInteractionState,
    pub viewport_layout: &'a mut viewport_3d::ViewportLayout,
    pub node_editor_state: &'a mut NodeEditorState,
    pub theme: &'a ModernTheme,
    pub graph_changed_writer: &'a mut MessageWriter<'a, GraphChanged>,
    /// For UI-only changes (selection, hover, drag preview) - triggers repaint but not scene update
    pub ui_changed_writer: &'a mut MessageWriter<'a, crate::UiChanged>,
    pub open_naive_window_writer: &'a mut MessageWriter<'a, OpenNaiveWindowEvent>,
    pub open_node_info_window_writer: &'a mut MessageWriter<'a, crate::ui::OpenNodeInfoWindowEvent>,
    pub open_settings_window_writer: &'a mut MessageWriter<'a, crate::ui::OpenSettingsWindowEvent>,
    pub open_ai_workspace_window_writer:
        &'a mut MessageWriter<'a, crate::ui::OpenAiWorkspaceWindowEvent>,
    pub set_camera_view_writer:
        &'a mut MessageWriter<'a, crate::viewport_options::SetCameraViewEvent>,
    pub camera_rotate_writer: &'a mut MessageWriter<'a, crate::viewport_options::CameraRotateEvent>,
    pub display_options: &'a mut DisplayOptions,
    pub console_log: &'a ConsoleLog,
    pub timeline_state: &'a mut crate::ui::TimelineState,
    pub tab_registry: &'a TabRegistry,
    pub node_registry: &'a NodeRegistry,
    pub spline_tool_state: &'a crate::nodes::spline::tool_state::SplineToolState,
    pub hud_action_queue: &'a hud_actions::HudActionQueue,
    pub gizmo_state: &'a GizmoState,
    pub perf_monitor: Option<&'a mut PerformanceMonitor>, // Optional mutable access
    pub script_node_state: &'a ScriptNodeState,
    pub native_ai_host: Option<&'a NativeAiHost>,
    pub ai_events: &'a [AiResultEvent],
    pub tiny_model_host: Option<&'a TinyModelHost>,
    pub gemini_copilot_host: Option<&'a GeminiCopilotHost>,
    pub connection_hint_state: Option<&'a mut ConnectionHintState>,
    pub voice_service: Option<&'a VoiceService>,
    pub voxel_tool_state: &'a mut crate::coverlay_bevy_ui::VoxelToolState,
    pub voxel_overlay_settings: &'a mut crate::coverlay_bevy_ui::VoxelOverlaySettings,
    pub voxel_hud_info: &'a crate::coverlay_bevy_ui::VoxelHudInfo,
    pub voxel_selection: &'a crate::voxel_editor::VoxelSelection,
    pub voxel_ai_prompt_stamp_queue: &'a mut crate::voxel_editor::VoxelAiPromptStampQueue,
    pub coverlay_wants_input: &'a mut crate::coverlay_bevy_ui::CoverlayUiWantsInput,
    /// The Bevy window entity this tab is currently rendering into.
    pub window_entity: Entity,
}

impl<'a> ServiceProvider for EditorTabContext<'a> {
    fn get_service(&self, service_type: TypeId) -> Option<&dyn Any> {
        if service_type == TypeId::of::<crate::ui::UiState>() {
            return Some(&*self.ui_state);
        }
        if service_type == TypeId::of::<crate::nodes::spline::tool_state::SplineToolState>() {
            return Some(self.spline_tool_state);
        }
        if service_type == TypeId::of::<hud_actions::HudActionQueue>() {
            return Some(self.hud_action_queue);
        }
        if service_type == TypeId::of::<NodeGraphResource>() {
            return Some(&*self.node_graph_res);
        }
        if service_type == TypeId::of::<GizmoState>() {
            return Some(self.gizmo_state);
        }
        if service_type == TypeId::of::<PerformanceMonitor>() {
            // Added service
            if let Some(perf) = self.perf_monitor.as_deref() {
                return Some(perf as &dyn Any);
            } else {
                return None;
            }
        }
        if service_type == TypeId::of::<ScriptNodeState>() {
            return Some(self.script_node_state);
        }
        None
    }
}

pub trait EditorTab: Send + Sync {
    fn ui(&mut self, ui: &mut egui::Ui, context: &mut EditorTabContext);
    fn title(&self) -> egui::WidgetText;
    fn as_any(&self) -> &dyn std::any::Any;
    fn is_immediate(&self) -> bool {
        false
    }
    fn retained_key(&self, _ui: &egui::Ui, context: &EditorTabContext) -> u64 {
        context.graph_revision
    }
}

/// The UI state for the docking tabs.
pub struct TabUi<'a> {
    pub context: &'a mut EditorTabContext<'a>,
}

impl<'a> EguiTabViewer for TabUi<'a> {
    type Tab = Box<dyn EditorTab>;

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        let id = tab.as_ref() as *const dyn EditorTab as *const () as usize;
        if tab.is_immediate() {
            // Audit: Warn about immediate mode tabs once per session
            let warned_id = ui.make_persistent_id(format!("warned_immediate_{}", id));
            let warned = ui.data_mut(|d| d.get_temp::<bool>(warned_id).unwrap_or(false));
            if !warned {
                let title = tab.title().text().to_string();
                self.context.console_log.warning(format!(
                    "PERF: Tab '{}' is running in IMMEDIATE mode.",
                    title
                ));
                ui.data_mut(|d| d.insert_temp(warned_id, true));
            }
            tab.ui(ui, self.context);
        } else {
            ui.push_id(id, |ui| tab.ui(ui, self.context));
        }
    }

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        tab.title()
    }

    fn on_close(&mut self, _tab: &mut Self::Tab) -> bool {
        // For now, we don't allow closing tabs.
        true // Allow closing now that we can add them!
    }

    fn add_popup(
        &mut self,
        ui: &mut egui::Ui,
        surface: egui_dock::SurfaceIndex,
        node: egui_dock::NodeIndex,
    ) {
        ui.set_min_width(120.0);
        ui.style_mut().visuals.button_frame = false;

        let tabs = vec![
            ("Viewport 3D", crate::ui::PaneTabType::Viewport),
            ("Node Graph", crate::ui::PaneTabType::NodeGraph),
            ("Properties", crate::ui::PaneTabType::Properties),
            ("Spreadsheet", crate::ui::PaneTabType::Spreadsheet),
            ("Outliner", crate::ui::PaneTabType::Outliner),
            ("Coverlay", crate::ui::PaneTabType::Coverlay),
            ("Console (Old)", crate::ui::PaneTabType::Console),
            ("Codex", crate::ui::PaneTabType::Codex),
        ];

        ui.label(egui::RichText::new("Standard Panes").strong());
        ui.separator();
        for (label, type_) in tabs {
            if ui.button(label).clicked() {
                self.context
                    .ui_state
                    .pane_command_queue
                    .push(crate::ui::PaneCommand::Add(type_, surface, node));
                ui.close_menu();
            }
        }

        // --- V5.0 Dynamic Tabs ---
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Plugins (V5)").strong());
        ui.separator();

        let registered_names = self.context.tab_registry.list_names();
        if registered_names.is_empty() {
            ui.label(
                egui::RichText::new("None loaded")
                    .italics()
                    .color(egui::Color32::GRAY),
            );
        } else {
            for name in registered_names {
                if ui.button(&name).clicked() {
                    self.context.ui_state.pane_command_queue.push(
                        crate::ui::PaneCommand::AddByName(name.clone(), surface, node),
                    );
                    ui.close_menu();
                }
            }
        }
    }
}

/// Stores the live editor tab instances that have been floated into native windows.
#[derive(Resource, Default)]
pub struct FloatingEditorTabs {
    pub tabs: HashMap<FloatingTabId, Box<dyn EditorTab>>,
}

#[derive(Resource)]
pub struct TabViewer {
    pub dock_state: DockState<Box<dyn EditorTab>>,
    // Persistent instance for Tablet/Mobile mode
    pub mobile_node_editor: NodeEditorTab,
}

impl FromWorld for TabViewer {
    fn from_world(_world: &mut World) -> Self {
        // The order of tabs here is important for the layout logic below.
        let mut tabs: Vec<Box<dyn EditorTab>> = vec![
            Box::new(Viewport3DTab::default()),
            Box::new(OutlinerTab::default()),
            Box::new(NodePropertiesTab::default()),
            Box::new(GeometrySpreadsheetTab::default()),
            Box::new(ConsoleTab::default()), // Moved Console before NodeEditor for layout logic
            Box::new(CodexTab::default()),
            Box::new(NodeEditorTab::default()),
        ];

        // 1. Start with the 3D Viewport as the central panel.
        let mut dock_state = DockState::new(vec![tabs.remove(0)]);

        // 2. Split Left -> Outliner (20% width)
        // Returns [Original Node (Viewport), New Node (Outliner)]
        let [viewport_node, _ai_node] =
            dock_state
                .main_surface_mut()
                .split_left(NodeIndex::root(), 0.20, vec![tabs.remove(0)]);

        // 3. Split Right (of Viewport) -> Properties + Spreadsheet
        // We want the Viewport (Left) to take about 75% of the remaining width.
        let [viewport_node, properties_node] = dock_state.main_surface_mut().split_right(
            viewport_node,
            0.75,
            vec![tabs.remove(0), tabs.remove(0)],
        );

        // 4. Split Viewport Below -> Console
        // Viewport (Top) keeps 85% of height, bottom group gets the remaining 15%.
        let [_viewport_node, _console_node] = dock_state.main_surface_mut().split_below(
            viewport_node,
            0.85,
            vec![tabs.remove(0), tabs.remove(0)],
        );

        // 5. Split Properties Below -> Node Editor
        // Node Editor takes 50% of the vertical space in the right column
        let [_properties_node, _node_editor_node] =
            dock_state
                .main_surface_mut()
                .split_below(properties_node, 0.5, vec![tabs.remove(0)]);

        Self {
            dock_state,
            mobile_node_editor: NodeEditorTab::default(),
        }
    }
}

#[derive(SystemParam)]
pub struct EditorUiSystemParam<'w> {
    pub tab_registry: Res<'w, TabRegistry>,
    pub node_registry: Res<'w, NodeRegistry>,
    pub settings_registry: Res<'w, crate::settings::SettingsRegistry>,
    pub settings_stores: ResMut<'w, crate::settings::SettingsStores>,
    pub ui_invalidator: ResMut<'w, crate::invalidator::UiInvalidator>,
    pub graph_revision: Res<'w, GraphRevision>,
    pub spline_tool_state: Res<'w, crate::nodes::spline::tool_state::SplineToolState>,
    pub hud_action_queue: Res<'w, hud_actions::HudActionQueue>,
    pub gizmo_state: Res<'w, crate::cunning_core::traits::node_interface::GizmoState>,
    pub perf_monitor: ResMut<'w, crate::cunning_core::profiling::PerformanceMonitor>,
    pub script_node_state: Res<'w, ScriptNodeState>,
    pub native_ai_host: Option<Res<'w, NativeAiHost>>,
    pub native_ai_inbox: Option<Res<'w, NativeAiInbox>>,
    pub tiny_model_host: Option<Res<'w, TinyModelHost>>,
    pub gemini_copilot_host: Option<Res<'w, GeminiCopilotHost>>,
    pub connection_hint_state: Option<ResMut<'w, ConnectionHintState>>,
    pub voice_service: Option<Res<'w, VoiceService>>,

    // Moved to struct to reduce SystemState tuple size
    pub set_camera_view_writer: MessageWriter<'w, crate::viewport_options::SetCameraViewEvent>,
    pub camera_rotate_writer: MessageWriter<'w, crate::viewport_options::CameraRotateEvent>,
    pub display_options: ResMut<'w, DisplayOptions>,
    pub console_log: Res<'w, ConsoleLog>,
    pub timeline_state: ResMut<'w, crate::ui::TimelineState>,
    pub floating_tabs: ResMut<'w, FloatingEditorTabs>,
    pub ui_settings: ResMut<'w, crate::ui_settings::UiSettings>,
    pub node_editor_settings: Res<'w, crate::node_editor_settings::NodeEditorSettings>,
    pub open_settings_window_writer: MessageWriter<'w, crate::ui::OpenSettingsWindowEvent>,
    pub open_ai_workspace_window_writer: MessageWriter<'w, crate::ui::OpenAiWorkspaceWindowEvent>,
    pub ui_changed_writer: MessageWriter<'w, crate::UiChanged>,
    pub voxel_tool_state: ResMut<'w, crate::coverlay_bevy_ui::VoxelToolState>,
    pub voxel_overlay_settings: ResMut<'w, crate::coverlay_bevy_ui::VoxelOverlaySettings>,
    pub voxel_hud_info: Res<'w, crate::coverlay_bevy_ui::VoxelHudInfo>,
    pub voxel_selection: Res<'w, crate::voxel_editor::VoxelSelection>,
    pub voxel_ai_prompt_stamp_queue: ResMut<'w, crate::voxel_editor::VoxelAiPromptStampQueue>,
    pub coverlay_wants_input: ResMut<'w, crate::coverlay_bevy_ui::CoverlayUiWantsInput>,
}

pub fn show_editor_ui(world: &mut World) {
    puffin::profile_function!();
    // Ensure the primary window has bevy_egui components, even if InitContexts didn't run yet.
    if let Ok(primary_entity) = world
        .query_filtered::<Entity, (With<PrimaryWindow>, With<Window>)>()
        .single(world)
    {
        let has_ctx = world.entity(primary_entity).contains::<EguiContext>();
        if !has_ctx {
            world.entity_mut(primary_entity).insert((
                EguiContext::default(),
                EguiRenderOutput::default(),
                EguiInput::default(),
                EguiOutput::default(),
                WindowSize::default(),
            ));
            // Reactive winit mode: ensure we get at least one more frame after initializing egui.
            if let Some(mut inv) = world.get_resource_mut::<crate::invalidator::UiInvalidator>() {
                inv.request_repaint_tagged(
                    "egui/init_primary",
                    crate::invalidator::RepaintCause::Layout,
                );
            }
            return;
        }
    }

    // Use SystemState to fetch everything (including the real EguiContext component) safely.
    let mut system_state: SystemState<(
        Query<
            (
                Entity,
                &'static mut EguiContext,
                Option<&'static PrimaryWindow>,
            ),
            With<Window>,
        >,
        ResMut<TabViewer>,
        ResMut<UiState>,
        ResMut<NodeGraphResource>,
        ResMut<ViewportInteractionState>,
        ResMut<viewport_3d::ViewportLayout>,
        ResMut<NodeEditorState>,
        Res<ModernTheme>,
        MessageWriter<GraphChanged>,
        MessageWriter<OpenNaiveWindowEvent>,
        MessageWriter<crate::ui::OpenNodeInfoWindowEvent>,
        MessageWriter<FloatTabToWindowEvent>,
        // Moved to EditorUiSystemParam
        EditorUiSystemParam,
    )> = SystemState::new(world);

    let (
        mut window_ctx_q,
        mut tab_viewer,
        mut ui_state,
        mut node_graph_res,
        mut viewport_interaction_state,
        mut viewport_layout,
        mut node_editor_state,
        theme,
        mut graph_changed_writer,
        mut open_naive_window_writer,
        mut open_node_info_window_writer,
        mut float_tab_to_window_writer,
        params, // Renamed from registries to params
    ) = system_state.get_mut(world);

    let mut chosen: Option<(Entity, Mut<EguiContext>)> = None;
    for (e, ctx, primary) in window_ctx_q.iter_mut() {
        if primary.is_some() {
            chosen = Some((e, ctx));
            break;
        }
        if chosen.is_none() {
            chosen = Some((e, ctx));
        }
    }
    let Some((window_entity, mut egui_context)) = chosen else {
        return;
    };

    // Destructure params to avoid borrow conflicts
    let EditorUiSystemParam {
        tab_registry,
        node_registry,
        settings_registry,
        mut settings_stores,
        mut ui_invalidator,
        graph_revision,
        spline_tool_state,
        hud_action_queue,
        gizmo_state,
        mut perf_monitor,
        script_node_state,
        native_ai_host,
        native_ai_inbox,
        tiny_model_host,
        gemini_copilot_host,
        mut connection_hint_state,
        voice_service,
        mut open_settings_window_writer,
        mut open_ai_workspace_window_writer,
        mut ui_changed_writer,
        mut set_camera_view_writer,
        mut camera_rotate_writer,
        mut display_options,
        console_log,
        mut timeline_state,
        mut floating_tabs,
        mut ui_settings,
        node_editor_settings,
        mut voxel_tool_state,
        mut voxel_overlay_settings,
        voxel_hud_info,
        voxel_selection,
        mut voxel_ai_prompt_stamp_queue,
        mut coverlay_wants_input,
        ..
    } = params;

    let ai_events: &[AiResultEvent] = native_ai_inbox
        .as_deref()
        .map(|i| i.0.as_slice())
        .unwrap_or(&[]);

    {
        let mut cx = EditorTabContext {
            ui_state: &mut ui_state,
            ui_settings: &mut *ui_settings,
            ui_invalidator: &mut *ui_invalidator,
            graph_revision: graph_revision.0,
            node_editor_settings: &*node_editor_settings,
            settings_registry: &*settings_registry,
            settings_stores: &mut *settings_stores,
            node_graph_res: &mut *node_graph_res,
            viewport_interaction_state: &mut viewport_interaction_state,
            viewport_layout: &mut viewport_layout,
            node_editor_state: &mut node_editor_state,
            theme: &theme,
            graph_changed_writer: &mut graph_changed_writer,
            ui_changed_writer: &mut ui_changed_writer,
            open_naive_window_writer: &mut open_naive_window_writer,
            open_node_info_window_writer: &mut open_node_info_window_writer,
            open_settings_window_writer: &mut open_settings_window_writer,
            open_ai_workspace_window_writer: &mut open_ai_workspace_window_writer,
            set_camera_view_writer: &mut set_camera_view_writer,
            camera_rotate_writer: &mut camera_rotate_writer,
            display_options: &mut *display_options,
            console_log: &*console_log,
            timeline_state: &mut *timeline_state,
            tab_registry: &*tab_registry,
            node_registry: &*node_registry,
            spline_tool_state: &*spline_tool_state,
            hud_action_queue: &*hud_action_queue,
            gizmo_state: &*gizmo_state,
            perf_monitor: Some(&mut *perf_monitor),
            script_node_state: &*script_node_state,
            native_ai_host: native_ai_host.as_deref(),
            ai_events: &ai_events,
            tiny_model_host: tiny_model_host.as_deref(),
            gemini_copilot_host: gemini_copilot_host.as_deref(),
            connection_hint_state: connection_hint_state.as_deref_mut(),
            voice_service: voice_service.as_deref(),
            voxel_tool_state: &mut *voxel_tool_state,
            voxel_overlay_settings: &mut *voxel_overlay_settings,
            voxel_hud_info: &*voxel_hud_info,
            voxel_selection: &*voxel_selection,
            voxel_ai_prompt_stamp_queue: &mut *voxel_ai_prompt_stamp_queue,
            coverlay_wants_input: &mut *coverlay_wants_input,
            window_entity,
        };

        // Create and apply the style here
        let mut style = Style::from_egui(egui_context.get_mut().style().as_ref());
        // CRITICAL for Native Viewport: Make dock background transparent so the Bevy hole is visible!
        style.main_surface_border_stroke = egui::Stroke::NONE;
        style.tab.tab_body.bg_fill = egui::Color32::TRANSPARENT; // Use the correct field: bg_fill

        // Interaction sizing is controlled by Settings → General/UI/Interaction.

        match cx.ui_state.layout_mode {
            crate::ui::LayoutMode::Phone => {
                // --- PHONE LAYOUT (Formerly Mobile) ---

                // Bottom Navigation Bar
                egui::TopBottomPanel::bottom("mobile_bottom_nav")
                    .resizable(false)
                    .min_height(50.0)
                    .show(egui_context.get_mut(), |ui| {
                        ui.horizontal_centered(|ui| {
                            let mut tab_button = |ui: &mut Ui, label: &str, tab: MobileTab| {
                                let selected = cx.ui_state.mobile_active_tab == tab;
                                // Increase touch target size
                                if ui
                                    .add(
                                        egui::Button::new(label)
                                            .selected(selected)
                                            .min_size(egui::vec2(80.0, 50.0)),
                                    )
                                    .clicked()
                                {
                                    cx.ui_state.mobile_active_tab = tab;
                                }
                            };

                            ui.columns(4, |cols| {
                                tab_button(&mut cols[0], "Viewport", MobileTab::Viewport);
                                tab_button(&mut cols[1], "Nodes", MobileTab::NodeGraph);
                                tab_button(&mut cols[2], "Properties", MobileTab::Properties);
                                tab_button(&mut cols[3], "Console", MobileTab::Console);
                            });
                        });
                    });

                // Central Content Area
                egui::CentralPanel::default().show(egui_context.get_mut(), |ui| {
                    match cx.ui_state.mobile_active_tab {
                        MobileTab::Viewport => {
                            Viewport3DTab::default().ui(ui, &mut cx);
                        }
                        MobileTab::NodeGraph => {
                            tab_viewer.mobile_node_editor.ui(ui, &mut cx);
                        }
                        MobileTab::Properties => {
                            NodePropertiesTab::default().ui(ui, &mut cx);
                        }
                        MobileTab::Console => {
                            ConsoleTab::default().ui(ui, &mut cx);
                        }
                    }
                });
            }
            crate::ui::LayoutMode::Tablet => {
                // --- TABLET LAYOUT ---

                // Right Panel: Node Graph (Approx 30% width)
                // FINAL ID: Stable ID with relaxed constraints
                let response = egui::SidePanel::right("tablet_node_graph_final")
                    .resizable(true)
                    .default_width(300.0)
                    .width_range(50.0..=2000.0) // Allow it to be very small or very large
                    .show(egui_context.get_mut(), |ui| {
                        // Title
                        ui.label(egui::RichText::new("Node Graph").heading());
                        ui.separator();

                        // CRITICAL FIX: Wrap Node Editor in a restrictive container.
                        // We use a ScrollArea that DOES NOT SCROLL and DOES NOT SHRINK.
                        // This acts as a layout barrier: it takes up all available space in the panel,
                        // but prevents the panel from expanding if the inner content is huge.
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .enable_scrolling(false) // We handle pan/zoom manually in the editor
                            .show(ui, |ui| {
                                tab_viewer.mobile_node_editor.ui(ui, &mut cx);
                            });
                    });

                // VISUAL SPLITTER for Tablet Mode
                // Manually draw a thick separator line on the left edge of the side panel
                let panel_rect = response.response.rect;
                let splitter_center_x = panel_rect.min.x;
                let painter = egui_context.get_mut().layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("tablet_splitter"),
                ));

                // Draw 5px wide line
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(splitter_center_x - 2.5, panel_rect.min.y),
                        egui::vec2(5.0, panel_rect.height()),
                    ),
                    2.0,                                               // Rounding
                    egui::Color32::from_gray(80).linear_multiply(0.5), // Semi-transparent dark grey
                );

                // Draw a "Handle" pill in the middle
                let center_y = panel_rect.center().y;
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(splitter_center_x - 2.0, center_y - 20.0),
                        egui::vec2(4.0, 40.0),
                    ),
                    2.0,
                    egui::Color32::WHITE.linear_multiply(0.8),
                );

                // Central Panel: 3D Viewport (Remaining Left area)
                egui::CentralPanel::default().show(egui_context.get_mut(), |ui| {
                    Viewport3DTab::default().ui(ui, &mut cx);

                    // Overlay: Properties Panel (Floating Window)
                    // Only show if a node is selected
                    if cx.ui_state.last_selected_node_id.is_some() {
                        let sel = cx.ui_state.last_selected_node_id.unwrap();
                        let sel_u128 = sel.as_u128();
                        let sel_key = (sel_u128 as u64) ^ ((sel_u128 >> 64) as u64).rotate_left(17);
                        egui::Window::new("Properties")
                            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-20.0, 20.0))
                            .default_width(320.0)
                            .collapsible(true)
                            .title_bar(true)
                            .frame(
                                egui::Frame::window(&ui.style()).shadow(egui::epaint::Shadow {
                                    offset: [10, 20],
                                    blur: 20,
                                    spread: 5,
                                    color: egui::Color32::from_black_alpha(96),
                                }),
                            )
                            .show(ui.ctx(), |ui| {
                                let key = mix_rect(sel_key, ui.available_rect_before_wrap());
                                ui.push_id(("tablet_properties", key), |ui| {
                                    NodePropertiesTab::default().ui(ui, &mut cx);
                                });
                            });
                    }
                });
            }
            crate::ui::LayoutMode::Desktop => {
                // --- DESKTOP LAYOUT ---
                let _toolbar = Toolbar::default();
                // --- TOPBAR SPACER (Topbar is Bevy UI) ---
                // Use a transparent egui panel to reserve space WITHOUT changing egui's screen_rect size.
                let top_h = 28.0;
                egui::TopBottomPanel::top("bevy_ui_topbar_spacer")
                    .resizable(false)
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(0)))
                    .height_range(top_h..=top_h)
                    .show(egui_context.get_mut(), |ui| {
                        ui.set_enabled(false);
                        ui.allocate_space(egui::vec2(ui.available_width(), top_h));
                    });

                // Shelf is Bevy UI now; keep egui out of this region to avoid overlap/input conflicts.
                let shelf_h = 84.0;
                egui::TopBottomPanel::top("bevy_ui_shelf_spacer")
                    .resizable(false)
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(0)))
                    .height_range(shelf_h..=shelf_h)
                    .show(egui_context.get_mut(), |ui| {
                        ui.set_enabled(false);
                        ui.allocate_space(egui::vec2(ui.available_width(), shelf_h));
                    });

                // --- TIMELINE SPACER (Timeline is Bevy UI) ---
                // Keep egui out of the bottom 60px while preserving egui screen size for SDF/GPUText.
                let bottom_h = 60.0;
                egui::TopBottomPanel::bottom("bevy_ui_timeline_spacer")
                    .resizable(false)
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(0)))
                    .height_range(bottom_h..=bottom_h)
                    .show(egui_context.get_mut(), |ui| {
                        ui.set_enabled(false);
                        ui.allocate_space(egui::vec2(ui.available_width(), bottom_h));
                    });

                let mut tab_ui = TabUi { context: &mut cx };

                DockArea::new(&mut tab_viewer.dock_state)
                    .id(egui::Id::new("main_dock_area"))
                    .style(style)
                    .show_add_buttons(true)
                    .show_add_popup(true)
                    .show(egui_context.get_mut(), &mut tab_ui);

                // Convert egui-dock "window surfaces" into real OS windows (Bevy Window entities).
                // This enables minimizing the main window while keeping detached panels usable.
                let surf_count = tab_viewer.dock_state.surfaces_count();
                if surf_count > 1 {
                    for si in (1..surf_count).rev() {
                        let surface_index = egui_dock::SurfaceIndex(si);
                        let Some(surface) = tab_viewer.dock_state.remove_surface(surface_index) else {
                            continue;
                        };
                        let (mut tree, win_state) = match surface {
                            egui_dock::Surface::Window(tree, win_state) => (tree, win_state),
                            _ => continue,
                        };

                        let base_rect = {
                            let r = win_state.rect();
                            if r == egui::Rect::NOTHING {
                                egui::Rect::from_min_size(egui::pos2(120.0, 120.0), egui::vec2(1100.0, 800.0))
                            } else {
                                r
                            }
                        };

                        // Extract all tabs out of the detached surface.
                        let mut tabs: Vec<Box<dyn EditorTab>> = Vec::new();
                        for n in tree.iter_mut() {
                            if let egui_dock::Node::Leaf { tabs: leaf_tabs, .. } = n {
                                tabs.append(&mut std::mem::take(leaf_tabs));
                            }
                        }

                        // Spawn one OS window per detached tab.
                        for (i, tab) in tabs.into_iter().enumerate() {
                            let id = FloatingTabId(Uuid::new_v4());
                            let title = tab.title().text().to_string();
                            floating_tabs.tabs.insert(id.clone(), tab);
                            let d = (i as f32) * 28.0;
                            let rect = egui::Rect::from_min_size(base_rect.min + egui::vec2(d, d), base_rect.size());
                            float_tab_to_window_writer.write(FloatTabToWindowEvent { title, initial_rect: rect, id });
                        }
                    }
                }
            }
        }
    } // cx dropped here

    // --- Pane Command Processing ---
    fn first_leaf(
        dock_state: &mut DockState<Box<dyn EditorTab>>,
        surface: egui_dock::SurfaceIndex,
        node: egui_dock::NodeIndex,
    ) -> Option<egui_dock::NodeIndex> {
        let len = dock_state[surface].len();
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            if n.0 >= len {
                continue;
            }
            match &dock_state[surface][n] {
                egui_dock::Node::Leaf { .. } => return Some(n),
                egui_dock::Node::Vertical { .. } | egui_dock::Node::Horizontal { .. } => {
                    stack.push(n.right());
                    stack.push(n.left());
                }
                egui_dock::Node::Empty => {}
            }
        }
        None
    }

    let mut should_open_ai_workspace = false;
    let pane_commands: Vec<_> = ui_state.pane_command_queue.drain(..).collect();
    for cmd in pane_commands {
        match cmd {
            crate::ui::PaneCommand::Add(tab_type, surface, node) => {
                let new_tab: Box<dyn EditorTab> = match tab_type {
                    crate::ui::PaneTabType::Viewport => Box::new(Viewport3DTab::default()),
                    crate::ui::PaneTabType::NodeGraph => Box::new(NodeEditorTab::default()),
                    crate::ui::PaneTabType::Properties => Box::new(NodePropertiesTab::default()),
                    crate::ui::PaneTabType::Spreadsheet => {
                        Box::new(GeometrySpreadsheetTab::default())
                    }
                    crate::ui::PaneTabType::Outliner => Box::new(OutlinerTab::default()),
                    crate::ui::PaneTabType::Coverlay => {
                        Box::new(crate::coverlay_bevy_ui::CoverlayDockTab::default())
                    }
                    crate::ui::PaneTabType::Console => Box::new(ConsoleTab::default()),
                    crate::ui::PaneTabType::Codex => Box::new(CodexTab::default()),
                    _ => Box::new(ConsoleTab::default()),
                };

                if let Some(target) = first_leaf(&mut tab_viewer.dock_state, surface, node) {
                    if let egui_dock::Node::Leaf { tabs, .. } =
                        &mut tab_viewer.dock_state[surface][target]
                    {
                        tabs.push(new_tab);
                    }
                    tab_viewer
                        .dock_state
                        .set_focused_node_and_surface((surface, target));
                }
            }
            crate::ui::PaneCommand::AddByName(name, surface, node) => {
                // Special case: AI Workspace opens GPUI window directly
                if name == "AI Workspace" {
                    should_open_ai_workspace = true;
                    continue;
                }
                
                if let Some(new_tab) = tab_registry.create(&name) {
                    if let Some(target) = first_leaf(&mut tab_viewer.dock_state, surface, node) {
                        if let egui_dock::Node::Leaf { tabs, .. } =
                            &mut tab_viewer.dock_state[surface][target]
                        {
                            tabs.push(new_tab);
                        }
                        tab_viewer
                            .dock_state
                            .set_focused_node_and_surface((surface, target));
                    }
                } else {
                    warn!("Failed to create pane: {}", name);
                }
            }
        }
    }
    
    // Set flag for AI Workspace opening (consumed by windowing system)
    if should_open_ai_workspace {
        ui_state.pending_open_ai_workspace = true;
    }
}

/// Renders UI for all floating (native) windows based on FloatingTabRegistry.
pub fn show_floating_tabs_ui(world: &mut World) {
    // Collect window entities first to avoid holding long-lived borrows across iterations.
    let window_entities: Vec<Entity> = world
        .query_filtered::<
            Entity,
            (
                With<EguiContext>,
                Without<PrimaryWindow>,
                Without<crate::ui::NeedsEguiFontsInit>,
            ),
        >()
        .iter(world)
        .collect();
    if window_entities.is_empty() {
        return;
    }

    // Fetch shared editor resources (same context as main editor UI) + per-window EguiContext.
    let mut system_state: SystemState<(
        Query<&'static mut EguiContext, Without<PrimaryWindow>>,
        ResMut<UiState>,
        ResMut<NodeGraphResource>,
        ResMut<ViewportInteractionState>,
        ResMut<viewport_3d::ViewportLayout>,
        ResMut<NodeEditorState>,
        Res<ModernTheme>,
        MessageWriter<GraphChanged>,
        MessageWriter<OpenNaiveWindowEvent>,
        MessageWriter<crate::ui::OpenNodeInfoWindowEvent>,
        MessageWriter<FloatTabToWindowEvent>,
        Res<crate::ui::FloatingTabRegistry>,
        EditorUiSystemParam,
    )> = SystemState::new(world);

    for entity in window_entities {
        let (
            mut window_query,
            mut ui_state,
            mut node_graph_res,
            mut viewport_interaction_state,
            mut viewport_layout,
            mut node_editor_state,
            theme,
            mut graph_changed_writer,
            mut open_naive_window_writer,
            mut open_node_info_window_writer,
            mut _float_tab_writer,
            floating_registry,
            params,
        ) = system_state.get_mut(world);

        let Ok(mut egui_ctx) = window_query.get_mut(entity) else {
            continue;
        };

        // Destructure params per-iteration (fresh borrows each loop).
        let EditorUiSystemParam {
            tab_registry,
            node_registry,
            settings_registry,
            mut settings_stores,
            mut ui_invalidator,
            graph_revision,
            spline_tool_state,
            hud_action_queue,
            gizmo_state,
            perf_monitor: _perf_monitor,
            script_node_state,
            native_ai_host,
            native_ai_inbox,
            tiny_model_host,
            gemini_copilot_host,
            mut connection_hint_state,
            voice_service,
            mut open_settings_window_writer,
            mut open_ai_workspace_window_writer,
            mut ui_changed_writer,
            mut set_camera_view_writer,
            mut camera_rotate_writer,
            mut display_options,
            console_log,
            mut timeline_state,
            mut floating_tabs,
            mut ui_settings,
            node_editor_settings,
            mut voxel_tool_state,
            mut voxel_overlay_settings,
            voxel_hud_info,
            voxel_selection,
            mut voxel_ai_prompt_stamp_queue,
            mut coverlay_wants_input,
            ..
        } = params;

        let ai_events: &[AiResultEvent] = native_ai_inbox
            .as_deref()
            .map(|i| i.0.as_slice())
            .unwrap_or(&[]);

        if let Some(window_entry) = floating_registry.floating_windows.get(&entity) {
            if let Some(tab) = floating_tabs.tabs.get_mut(&window_entry.id) {
                let ctx = egui_ctx.get_mut();

                let mut cx = EditorTabContext {
                    ui_state: &mut ui_state,
                    ui_settings: &mut *ui_settings,
                    ui_invalidator: &mut *ui_invalidator,
                    graph_revision: graph_revision.0,
                    node_editor_settings: &*node_editor_settings,
                    settings_registry: &*settings_registry,
                    settings_stores: &mut *settings_stores,
                    node_graph_res: &mut *node_graph_res,
                    viewport_interaction_state: &mut viewport_interaction_state,
                    viewport_layout: &mut viewport_layout,
                    node_editor_state: &mut node_editor_state,
                    theme: &theme,
                    graph_changed_writer: &mut graph_changed_writer,
                    ui_changed_writer: &mut ui_changed_writer,
                    open_naive_window_writer: &mut open_naive_window_writer,
                    open_node_info_window_writer: &mut open_node_info_window_writer,
                    open_settings_window_writer: &mut open_settings_window_writer,
                    open_ai_workspace_window_writer: &mut open_ai_workspace_window_writer,
                    set_camera_view_writer: &mut set_camera_view_writer,
                    camera_rotate_writer: &mut camera_rotate_writer,
                    display_options: &mut *display_options,
                    console_log: &*console_log,
                    timeline_state: &mut *timeline_state,
                    tab_registry: &*tab_registry,
                    node_registry: &*node_registry,
                    spline_tool_state: &*spline_tool_state,
                    hud_action_queue: &*hud_action_queue,
                    gizmo_state: &*gizmo_state,
                    perf_monitor: None,
                    script_node_state: &*script_node_state,
                    native_ai_host: native_ai_host.as_deref(),
                    ai_events,
                    tiny_model_host: tiny_model_host.as_deref(),
                    gemini_copilot_host: gemini_copilot_host.as_deref(),
                    connection_hint_state: connection_hint_state.as_deref_mut(),
                    voice_service: voice_service.as_deref(),
                    voxel_tool_state: &mut *voxel_tool_state,
                    voxel_overlay_settings: &mut *voxel_overlay_settings,
                    voxel_hud_info: &*voxel_hud_info,
                    voxel_selection: &*voxel_selection,
                    voxel_ai_prompt_stamp_queue: &mut *voxel_ai_prompt_stamp_queue,
                    coverlay_wants_input: &mut *coverlay_wants_input,
                    window_entity: entity,
                };

                // IMPORTANT: Floating windows are rendered in immediate mode to avoid long-lived borrows
                // (retained closures can capture &mut references across loop iterations).
                egui::CentralPanel::default().show(ctx, |ui| {
                    tab.ui(ui, &mut cx);
                });

                // NOTE: Do not force repaint every frame; winit reactive mode + egui repaint requests
                // should drive updates only when needed (input/animations/data changes).
            }
        }
    }
}

/// Apply settings edits staged by UI without forcing a per-frame `ResMut<SettingsStores>`.
pub fn apply_settings_edits_system(world: &mut World) {
    let has_edits = world
        .get_resource::<crate::ui::UiState>()
        .map_or(false, |s| !s.settings_edits.is_empty());
    if !has_edits {
        return;
    }

    let edits = {
        let mut s = world.resource_mut::<crate::ui::UiState>();
        std::mem::take(&mut s.settings_edits)
    };
    if edits.is_empty() {
        return;
    }

    {
        let mut stores = world.resource_mut::<crate::settings::SettingsStores>();
        for e in edits {
            match e {
                crate::ui::SettingsEdit::Set(id, v) => stores.user.set(id, v),
                crate::ui::SettingsEdit::Remove(id) => stores.user.remove(&id),
            }
        }
    }

    // Ensure the next frame runs so egui picks up the store changes (centralized invalidator).
    if let Some(mut inv) = world.get_resource_mut::<crate::invalidator::UiInvalidator>() {
        inv.request_repaint_tagged(
            "settings/apply_edits",
            crate::invalidator::RepaintCause::DataChanged,
        );
    }
}
