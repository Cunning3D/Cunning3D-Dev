// Do not write business logic directly into main.rs: only entry wiring/plugin registration/system scheduling allowed here, business implementation must be moved to corresponding modules
use crate::nodes::parameter::ParameterValue;
use crate::{
    camera::{
        camera_control_system, handle_camera_view_events, CameraController,
        ViewportInteractionState,
    },
    nodes::{NodeGraph, NodeGraphResource},
    render::final_material::{BackfaceTintExt, FinalMaterial},
    render::normal::{CunningNormalPlugin, NormalColor, NormalMarker},
    render::point::{CunningPointPlugin, PointMarker},
    render::primitive_number::{
        CunningPrimitiveNumberPlugin, PrimitiveNumberData, PrimitiveNumberMarker,
    },
    render::uv_material::UvMaterial,
    render::wireframe::{CunningWireframePlugin, WireframeMarker, WireframeTopology},
    tabs_system::{show_editor_ui, show_floating_tabs_ui, TabViewer, Viewport3DTab},
    theme::setup_theme,
    theme::ModernTheme,
    ui::{
        ComponentSelectionMode, FloatTabToWindowEvent, FloatingTabRegistry, FloatingWindowEntry,
        NodeEditorState, PaneTabType, UiState,
    },
    viewport_options::{
        DisplayOptions, OpenNaiveWindowEvent, SetCameraViewEvent, ViewportViewMode,
    },
};
use bevy::asset::AssetPlugin;
use bevy::render::sync_world::SyncToRenderWorld;
#[cfg(feature = "virtual_geometry_meshlet")]
use bevy::pbr::experimental::meshlet::{
    MeshToMeshletMeshConversionError, MeshletMesh, MeshletMesh3d, MeshletPlugin,
    MESHLET_DEFAULT_VERTEX_POSITION_QUANTIZATION_FACTOR,
};
#[cfg(feature = "virtual_geometry_meshlet")]
use bevy::tasks::{futures::future, AsyncComputeTaskPool, Task};
use bevy::{
    camera::{Projection, RenderTarget, ScalingMode},
    pbr::wireframe::WireframePlugin,
    pbr::MaterialPlugin,
    prelude::*,
    render::render_resource::{
        Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
    },
    window::{MonitorSelection, PrimaryWindow, Window, WindowPlugin, WindowPosition, WindowRef},
    winit::{UpdateMode, WinitSettings}, // Import Winit settings
};
use bevy_ecs::system::SystemParam;
use bevy_egui::{egui, EguiContexts, EguiHolesMode, EguiHolesSettings, EguiPlugin, EguiSet};
use bevy_input_focus::InputDispatchPlugin;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};

/// Legacy unified event - kept for backward compatibility during migration.
/// Will be replaced by UiChanged + GeometryChanged.
#[derive(Message, Default, Clone, Copy, Debug)]
pub struct GraphChanged;

/// UI-only change (selection, hover, menu, drag preview) - triggers repaint but NOT scene update.
#[derive(Message, Default, Clone, Copy, Debug)]
pub struct UiChanged;

/// Geometry/cook result changed - triggers 3D scene mesh update.
#[derive(Message, Default, Clone, Copy, Debug)]
pub struct GeometryChanged;

fn dbg_viewport() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("DCC_LOG_VIEWPORT").ok().as_deref() == Some("1"))
}

use crate::cunning_core::graph::async_compute::{
    dispatch_compute_tasks, receive_compute_results, AsyncComputeState, CookThrottleState,
};
use crate::cunning_core::plugin_system::PluginSystem;
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::cunning_core::registries::tab_registry::TabRegistry;
use launcher::plugin::{AppState, LauncherPlugin, SPLASH_HEIGHT, SPLASH_WIDTH};
mod bridge_db_sync;
mod bridge_startup;

mod camera;
pub mod nodes;
pub mod project;
pub mod tabs_system;
use crate::tabs_system::viewport_3d::grid::grid_params::grid_params;
mod console;
mod debug_settings;
mod gizmos;
pub mod mesh;
mod node_editor_settings;
mod public;
mod settings;
mod theme;
mod ui;
mod ui_settings;
mod viewport_options;
pub use cunning_kernel::volume;
mod gpu_text;
mod input;
mod invalidator;
mod render;
mod scene;
mod app;
mod app_jobs;

mod runtime_paths;

pub mod coverlay_bevy_ui; // Bevy UI Coverlay (Viewport Overlay)
pub mod voxel_editor;
mod launcher;
pub mod shelf_bevy_ui;
pub mod timeline_bevy_ui; // Bevy UI Timeline
pub mod topbar_bevy_ui; // Bevy UI Topbar (Desktop) // Bevy UI Shelf (Desktop)
mod voice;
mod voice_assistant;
pub mod libs;

// --- V5.0 Architecture Modules ---
pub mod cunning_core;
use crate::cunning_core::ai_service::native_candle::NativeAiPlugin;
pub mod tabs_registry;
pub mod ai_workspace_gpui;

// mod cunning_shelf; // Not implemented yet

#[allow(dead_code)]
fn auto_switch_mobile_mode(
    mut ui_state: ResMut<UiState>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
) {
    if let Ok(window) = primary_window.single() {
        // Heuristic: If window width < 600.0, enable Phone mode.
        // Tablets will stay in Desktop mode by default, as requested.
        if window.width() < 600.0 {
            if matches!(ui_state.layout_mode, crate::ui::LayoutMode::Desktop) {
                ui_state.layout_mode = crate::ui::LayoutMode::Phone;
                ui_state.mobile_active_tab = crate::ui::MobileTab::Viewport;
                
                #[cfg(target_arch = "wasm32")]
                info!(
                    "Auto-switched to Phone Mode based on window width: {}",
                    window.width()
                );
            }
        }
    }
}

// NOTE: Desktop top/bottom reserved by egui spacer panels instead of modifying EguiInput.screen_rect

// Runtime smoke test removed

// Note: GraphChanged and scene components have been moved to the `scene` module.
// Use `scene::GraphChanged` and `scene::FinalMeshTag` etc. instead.

pub use cunning_viewport::MainCamera;

#[cfg(feature = "virtual_geometry_meshlet")]
fn sync_meshlet_virtual_geometry_system(
    display_options: Res<DisplayOptions>,
    mut commands: Commands,
    mut camera_query: Query<(Entity, &mut Msaa, Option<&OriginalMainCameraMsaa>), With<MainCamera>>,
    mesh_assets: Res<Assets<Mesh>>,
    query_mesh: Query<
        (Entity, &Mesh3d),
        (
            With<FinalMeshTag>,
            Without<MeshletMesh3d>,
            Without<MeshletConversionTask>,
        ),
    >,
    query_meshlet: Query<
        (Entity, &MeshletOriginalMesh),
        (With<FinalMeshTag>, With<MeshletMesh3d>),
    >,
    query_pending: Query<Entity, (With<FinalMeshTag>, With<MeshletConversionTask>)>,
) {
    let enabled = display_options.meshlet_virtual_geometry;

    // Meshlet renderer requirement: cameras rendering meshlets must have MSAA disabled.
    for (cam_entity, mut msaa, orig) in camera_query.iter_mut() {
        if enabled {
            if *msaa != Msaa::Off && orig.is_none() {
                commands
                    .entity(cam_entity)
                    .insert(OriginalMainCameraMsaa(msaa.clone()));
                *msaa = Msaa::Off;
            }
        } else if let Some(orig) = orig {
            if *msaa == Msaa::Off {
                *msaa = orig.0.clone();
            }
            commands.entity(cam_entity).remove::<OriginalMainCameraMsaa>();
        }
    }

    if !enabled {
        // Cancel any in-flight conversions.
        for e in query_pending.iter() {
            commands
                .entity(e)
                .remove::<MeshletConversionTask>()
                .remove::<MeshletOriginalMesh>();
        }
        // Swap back to regular meshes.
        for (e, orig) in query_meshlet.iter() {
            commands
                .entity(e)
                .remove::<MeshletMesh3d>()
                .insert(Mesh3d(orig.0.clone()))
                .remove::<MeshletOriginalMesh>();
        }
        return;
    }

    let pool = AsyncComputeTaskPool::get();
    let mut spawned = 0usize;
    let max_spawn = 2usize;

    for (e, mesh_3d) in query_mesh.iter() {
        if spawned >= max_spawn {
            break;
        }
        let Some(mesh) = mesh_assets.get(&mesh_3d.0) else {
            continue;
        };
        if mesh.primitive_topology()
            != bevy::render::render_resource::PrimitiveTopology::TriangleList
        {
            continue;
        }
        if mesh.indices().is_none() {
            continue;
        }
        if mesh.attribute(Mesh::ATTRIBUTE_POSITION).is_none()
            || mesh.attribute(Mesh::ATTRIBUTE_NORMAL).is_none()
            || mesh.attribute(Mesh::ATTRIBUTE_UV_0).is_none()
        {
            continue;
        }

        // Note: This conversion is expensive; do it asynchronously and keep it opt-in.
        let mesh_clone = mesh.clone();
        let task = pool.spawn(async move {
            MeshletMesh::from_mesh(
                &mesh_clone,
                MESHLET_DEFAULT_VERTEX_POSITION_QUANTIZATION_FACTOR,
            )
        });
        commands
            .entity(e)
            .insert((MeshletOriginalMesh(mesh_3d.0.clone()), MeshletConversionTask(task)));
        spawned += 1;
    }
}

#[cfg(feature = "virtual_geometry_meshlet")]
fn poll_meshlet_conversion_tasks_system(
    mut commands: Commands,
    mut tasks: Query<(Entity, &mut MeshletConversionTask, &MeshletOriginalMesh), With<FinalMeshTag>>,
    mut meshlet_mesh_assets: ResMut<Assets<MeshletMesh>>,
) {
    for (e, mut task, _orig) in tasks.iter_mut() {
        let Some(res) = future::block_on(future::poll_once(&mut task.0)) else {
            continue;
        };
        match res {
            Ok(meshlet_mesh) => {
                let handle = meshlet_mesh_assets.add(meshlet_mesh);
                commands
                    .entity(e)
                    .remove::<Mesh3d>()
                    .insert(MeshletMesh3d(handle))
                    .remove::<MeshletConversionTask>();
            }
            Err(err) => {
                warn!("[Meshlets] Mesh->MeshletMesh conversion failed: {err:?}");
                commands
                    .entity(e)
                    .remove::<MeshletConversionTask>()
                    .remove::<MeshletOriginalMesh>();
            }
        }
    }
}

// REPLACED BY ASYNC COMPUTE
// /// The system that re-computes the node graph whenever it changes.
// pub fn compute_node_system(
//     mut graph_changed: EventReader<GraphChanged>,
//     mut node_graph_res: ResMut<NodeGraphResource>,
//     node_registry: Res<NodeRegistry>,
//     mut perf_monitor: ResMut<crate::cunning_core::profiling::PerformanceMonitor>,
// ) {
//     if graph_changed.is_empty() {
//         return;
//     }
//     // DO NOT consume the event here. Let the update_3d_scene_from_node_graph system do it.
//     // graph_changed.clear();
//
//     let mut node_graph = node_graph_res.0.lock().unwrap();
//
//     let mut compute_targets = HashSet::new();
//     if let Some(display_id) = node_graph.display_node {
//         compute_targets.insert(display_id);
//     }
//
//     node_graph.compute(&compute_targets, &node_registry, Some(&mut *perf_monitor));
// }

fn profiler_tick() {
    if puffin::are_scopes_on() {
        puffin::GlobalProfiler::lock().new_frame();
    }
}

fn install_egui_image_loaders(mut egui_contexts: EguiContexts) {
    egui_extras::install_image_loaders(egui_contexts.ctx_mut());
}

fn main() {
    let assets_dir = runtime_paths::assets_dir();
    let assets_dir = assets_dir.to_string_lossy().into_owned();
    let mut bridge_path: Option<String> = None;
    let mut bridge_ephemeral = false;
    let mut bridge_db_path: Option<String> = None;
    {
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            if a == "--bridge" {
                bridge_path = it.next();
                continue;
            }
            if a == "--bridge-db" {
                bridge_db_path = it.next();
                continue;
            }
            if a == "--ephemeral" {
                bridge_ephemeral = true;
                continue;
            }
        }
    }
    puffin::set_scopes_on(false); // Puffin off by default (enable via UI)
    App::new()
        .insert_resource(bridge_startup::BridgeStartup {
            path: bridge_path,
            ephemeral: bridge_ephemeral,
        })
        .insert_resource(bridge_db_sync::BridgeDb {
            path: bridge_db_path,
        })
        // Configure Winit for DCC/Editor Application.
        // Use Continuous when focused to avoid visible stutter/flash during camera interaction.
        .insert_resource(WinitSettings {
            focused_mode: UpdateMode::Continuous,
            unfocused_mode: UpdateMode::reactive_low_power(std::time::Duration::from_secs(60)),
        })
        // Use small splash window: borderless, centered (non-transparent to avoid white flash)
        .add_plugins((
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: assets_dir,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Cunning3D Splash".into(),
                        resolution: (SPLASH_WIDTH as u32, SPLASH_HEIGHT as u32).into(),
                        decorations: false,
                        transparent: false,
                        position: WindowPosition::Centered(MonitorSelection::Primary),
                        ..default()
                    }),
                    ..default()
                }),
            #[cfg(feature = "virtual_geometry_meshlet")]
            MeshletPlugin {
                // Default is tuned for dense scenes; keep conservative for editor usage.
                cluster_buffer_slots: 1 << 14,
            },
        ))
        .add_plugins(LauncherPlugin)
        .add_plugins(bridge_db_sync::BridgeDbSyncPlugin)
        .add_plugins(app_jobs::AppJobsPlugin)
        .add_plugins(project::ProjectIoPlugin)
        .add_plugins(crate::cunning_core::plugin_system::PluginBuildJobsPlugin)
        .add_plugins(crate::nodes::graph_model::GraphModelPlugin)
        .add_plugins(crate::nodes::ai_texture::AiTexturePlugin)
        .add_plugins(EguiPlugin)
        // Best-practice: avoid AutoAll hole punching (can incorrectly block egui input, e.g. node editor menus).
        // We explicitly mark only the cgui regions that should "punch through" egui.
        .insert_resource(EguiHolesSettings {
            enabled: true,
            mode: EguiHolesMode::ManualOnly,
        })
        // CGUI (fork of bevy_ui): required for Timeline/Topbar interaction + layout.
        .add_plugins(bevy_cgui::UiPlugin)
        .add_plugins(bevy_cgui_render::UiPlugin)
        .add_plugins(bevy_cgui_widgets::UiWidgetsPlugins)
        .add_plugins(InputDispatchPlugin)
        .add_plugins(timeline_bevy_ui::TimelineUiPlugin) // Bevy UI Timeline
        .add_plugins(topbar_bevy_ui::TopbarUiPlugin) // Bevy UI Topbar (Desktop)
        .add_plugins(coverlay_bevy_ui::CoverlayUiPlugin) // Bevy UI Coverlay (Viewport Overlay)
        .add_plugins(voxel_editor::VoxelEditorPlugin) // Voxel editor input (CPU MVP)
        .add_plugins(shelf_bevy_ui::ShelfUiPlugin) // Bevy UI Shelf (Desktop)
        .add_plugins(WireframePlugin::default())
        .add_plugins((MaterialPlugin::<crate::gizmos::renderer::GizmoMaterial>::default(),))
        .add_plugins(MaterialPlugin::<crate::render::grid_plane::GridPlaneMaterial>::default())
        .add_plugins(MaterialPlugin::<FinalMaterial>::default())
        .add_plugins(gizmos::GizmoPlugin)
        // Bevy 0.18: custom gizmo groups must be registered, otherwise `config_mut::<T>()` will panic.
        .init_gizmo_group::<crate::gizmos::GridGizmos>()
        .init_gizmo_group::<crate::gizmos::GridMajorGizmos>()
        .init_gizmo_group::<crate::gizmos::GridAxisGizmos>()
        .init_gizmo_group::<crate::gizmos::TransformGizmoLines>()
        .init_gizmo_group::<crate::gizmos::SelectedCurveGizmos>()
        .init_gizmo_group::<crate::gizmos::SelectedCurveXrayGizmos>()
        .init_gizmo_group::<crate::tabs_system::viewport_3d::UvBoundaryGizmos>()
        .add_plugins(CunningWireframePlugin)
        .add_plugins(MaterialPlugin::<UvMaterial>::default())
        .add_plugins(MaterialPlugin::<
            crate::tabs_system::viewport_3d::group_highlight::GroupHighlightMaterial,
        >::default())
        .add_plugins(MaterialPlugin::<
            crate::tabs_system::viewport_3d::group_highlight::GroupHighlightWireMaterial,
        >::default())
        .add_plugins(CunningPointPlugin)
        .add_plugins(CunningNormalPlugin)
        .add_plugins(CunningPrimitiveNumberPlugin)
        .add_plugins(render::voxel_faces::CunningVoxelFacesPlugin)
        // Must run after UI selection + voxel cmd edits are applied, otherwise root stays None.
        .add_systems(
            Update,
            render::voxel_faces_desktop_sync::sync_voxel_faces_root_from_final_geo_system
                .run_if(in_state(AppState::Editor))
                .after(show_editor_ui),
        )
        .add_plugins(render::group_visualization::GroupVisualizationPlugin) // [FIX] Add Group Visualization Plugin
        .add_plugins(cunning_core::scripting::ScriptingPlugin)
        .add_plugins(cunning_core::profiling::ProfilingPlugin)
        .add_plugins(invalidator::InvalidatorPlugin)
        .add_plugins(NativeAiPlugin)
        .add_plugins(crate::cunning_core::ai_service::native_tiny_model::NativeTinyModelPlugin)
        .insert_resource(crate::cunning_core::ai_service::gemini::copilot_host::GeminiCopilotHost::new())
        // Connection hint removed (feature disabled).
        .add_plugins(crate::voice::VoiceServicePlugin)
        .add_plugins(crate::voice_assistant::AiVoiceAssistantPlugin)
        // WebGPU runtime init (wasm): async, non-blocking, required for GPU-resident cooking.
        .add_systems(
            Startup,
            crate::nodes::gpu::runtime::init_gpu_runtime_startup_system,
        )
        .add_message::<GraphChanged>()
        .add_message::<UiChanged>()
        .add_message::<GeometryChanged>()
        .add_message::<OpenNaiveWindowEvent>()
        .add_message::<FloatTabToWindowEvent>()
        .add_message::<ui::OpenSettingsWindowEvent>()
        .add_message::<ui::OpenFilePickerEvent>()
        .add_message::<ui::FilePickerChosenEvent>()
        .add_message::<crate::cunning_core::plugin_system::CompileRustPluginRequest>()
        .add_message::<ui::OpenAiWorkspaceWindowEvent>()
        .add_message::<ui::OpenNodeInfoWindowEvent>()
        .add_message::<SetCameraViewEvent>()
        .add_message::<crate::viewport_options::CameraRotateEvent>()
        .init_resource::<NodeGraphResource>()
        .init_resource::<UiState>()
        .init_resource::<ui::FilePickerState>()
        .init_resource::<NodeEditorState>()
        .init_resource::<TabViewer>()
        .init_resource::<tabs_system::FloatingEditorTabs>()
        .init_resource::<tabs_system::viewport_3d::ViewportLayout>()
        .init_resource::<ViewportInteractionState>()
        .init_resource::<DisplayOptions>()
        .init_resource::<crate::camera::TurntableRuntimeState>()
        .init_resource::<FloatingTabRegistry>()
        .init_resource::<console::ConsoleLog>()
        .init_resource::<crate::cunning_core::cda::CdaLibrary>()
        .init_resource::<input::NavigationInput>()
        .init_resource::<ui::TimelineState>()
        .init_resource::<crate::nodes::spline::tool_state::SplineToolState>() // Unity Spline State
        .init_resource::<crate::tabs_system::hud_actions::HudActionQueue>()
        .init_resource::<TabRegistry>() // V5 Architecture
        .init_resource::<NodeRegistry>() // V5 Architecture
        .init_resource::<settings::SettingsRegistry>()
        .init_resource::<settings::SettingsStores>()
        .init_resource::<crate::app::windowing::PendingNaiveWindows>()
        .init_resource::<crate::app::windowing::GpuiAiWorkspaceState>()
        .init_resource::<AsyncComputeState>() // Phase 3: Async Compute
        .init_resource::<CookThrottleState>() // Cook throttle for interaction (10-15Hz during drag)
        .insert_resource(theme::ModernTheme::dark()) // Use dark theme by default
        .init_resource::<ui_settings::UiSettings>()
        .init_resource::<node_editor_settings::NodeEditorSettings>()
        .init_resource::<debug_settings::DebugSettings>()
        // 3D scene and theme init on Editor entry; registries and plugins loaded by Launcher
        .add_systems(
            OnEnter(AppState::Editor),
            (
                crate::app::startup::setup_3d_scene,
                setup_theme,
                console::init_global_console,
                crate::cunning_core::cda::library::init_global_cda_library,
                crate::app::startup::register_voxel_coverlay_pressure_test_cda,
                bridge_startup::import_bridge_on_enter,
                tabs_system::viewport_3d::gizmo_systems::configure_gizmos_system,
                crate::render::grid_plane::setup_grid_plane_system,
            ),
        )
        .add_systems(Update, bridge_startup::cleanup_ephemeral_on_exit)
        .add_systems(
            Update,
            crate::cunning_core::plugin_system::ensure_curve_reference_plugin_system
                .run_if(in_state(AppState::Editor))
                .before(crate::cunning_core::plugin_system::auto_reload_latest_plugins_system),
        )
        .add_systems(
            Update,
            crate::cunning_core::plugin_system::auto_reload_latest_plugins_system
                .run_if(in_state(AppState::Editor))
                .before(show_editor_ui),
        )
        .add_systems(
            Update,
            ui_settings::sync_from_settings_stores
                .run_if(in_state(AppState::Editor))
                .before(theme::apply_ui_settings),
        )
        .add_systems(
            Update,
            node_editor_settings::sync_from_settings_stores
                .run_if(in_state(AppState::Editor))
                .before(show_editor_ui),
        )
        .add_systems(
            Update,
            node_editor_settings::apply_to_all_nodes
                .run_if(in_state(AppState::Editor))
                .before(show_editor_ui)
                .after(node_editor_settings::sync_from_settings_stores),
        )
        .add_systems(
            Update,
            debug_settings::sync_from_settings_stores
                .run_if(in_state(AppState::Editor))
                .before(show_editor_ui),
        )
        .add_systems(
            Update,
            theme::apply_ui_settings
                .run_if(in_state(AppState::Editor))
                .before(show_editor_ui),
        )
        .add_systems(
            Update,
            tabs_system::timeline::timeline_playback_system
                .run_if(in_state(AppState::Editor))
                .before(show_editor_ui),
        )
        .add_systems(
            Update,
            crate::tabs_system::hud_actions::apply_hud_actions_system
                .run_if(in_state(AppState::Editor))
                .before(show_editor_ui),
        )
        // Core update loop: split into smaller groups to satisfy IntoSystemConfigs limits.
        .add_systems(Update, profiler_tick) // Puffin Tick
        // Phase 3: Non-blocking Compute Pipeline
        .add_systems(Update, dispatch_compute_tasks)
        .add_systems(Update, receive_compute_results)
        .add_systems(Update, crate::scene::systems::update_3d_scene_from_node_graph)
        .add_systems(Update, sync_active_group_toggle_system) // [NEW] Sync active group when toggle changes
        .add_systems(
            Update,
            tabs_system::viewport_3d::group_highlight::update_group_highlight_system,
            )
        // Viewport render texture and grid (Editor only)
        .add_systems(
            Update,
            (crate::render::grid_plane::update_grid_plane_system,)
                .run_if(in_state(AppState::Editor)),
        )
        // Camera input and control (must remain chained)
        .add_systems(
            Update,
            (input::input_mapping_system, camera_control_system).chain(),
        )
        // Handle camera view change events from viewport gizmo
        .add_systems(Startup, crate::app::startup::setup_registries)
        .add_systems(Startup, install_egui_image_loaders) // [FIX] Install svg loaders
        // Initialize fonts for newly spawned windows at the correct egui stage boundary (prevents "No fonts loaded" crash).
        .add_systems(
            PreUpdate,
            theme::init_new_window_egui_fonts_system
                .run_if(in_state(AppState::Editor))
                .after(EguiSet::InitContexts)
                .before(EguiSet::BeginFrame),
        )
        .add_systems(
            Update,
            crate::gpu_text::install_gpu_text_renderer_system.before(show_editor_ui),
        )
        // Ensure CDA library global is available even before entering Editor state (topbar may exist earlier).
        .add_systems(
            Startup,
            crate::cunning_core::cda::library::init_global_cda_library,
        )
        .add_systems(Startup, crate::render::uv_material::setup_uv_material)
        .add_systems(Startup, crate::app::startup::setup_3d_scene)
        .add_systems(Update, handle_camera_view_events)
        .add_systems(Update, crate::camera::handle_camera_rotate_events)
        .add_systems(Update, crate::camera::camera_transition_system)
        .add_systems(Update, crate::camera::turntable_camera_system.run_if(in_state(AppState::Editor)).after(crate::camera::camera_transition_system))
        // Camera parameters and multi-window management
        .add_systems(
            Update,
            (
                crate::app::windowing::sync_naive_camera_system,
                crate::camera::update_camera_speed_system,
                #[cfg(feature = "virtual_geometry_meshlet")]
                poll_meshlet_conversion_tasks_system,
            ),
        )
        // Must run before Bevy's `camera_system` (CameraUpdateSystems is in PostUpdate).
        .add_systems(PostUpdate, crate::camera::sanitize_camera_window_targets_system)
        .add_systems(
            Update,
            (
                crate::app::windowing::spawn_naive_window_system,
                crate::app::windowing::handle_float_tab_window_system,
                crate::app::windowing::handle_open_settings_window_system,
                crate::app::windowing::handle_open_ai_workspace_window_system,
                crate::app::windowing::gpui_ai_workspace_voice_bridge_system,
                crate::app::windowing::handle_open_node_info_window_system,
                crate::app::windowing::cleanup_floating_windows_on_close_system,
            ),
        )
        // Spawn cameras for newly-created windows only after the Window component is applied.
        .add_systems(PostUpdate, crate::app::windowing::spawn_naive_camera_after_window_ready_system)
        // NOTE: Desktop top/bottom reserved by egui spacer panels in tabs_system/mod.rs
        // DisplayOptions-dependent visibility/material systems
        .add_systems(
            Update,
            (
                crate::scene::systems::update_final_mesh_visibility_system,
                crate::scene::systems::update_final_mesh_material_system,
                crate::render::overlay_visibility::update_point_visibility_system,
                crate::render::overlay_visibility::update_normal_visibility_system,
                crate::scene::systems::sync_uv_view_mode,
                #[cfg(feature = "virtual_geometry_meshlet")]
                sync_meshlet_virtual_geometry_system,
            )
                .run_if(resource_changed::<DisplayOptions>),
        )
        .add_systems(
            PostUpdate,
            crate::render::overlay_visibility::debug_normal_entity_state
                .after(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
        )
        // Core editor UI + floating tabs + gizmo overlay + native viewport sync
        .add_systems(Update, show_editor_ui.run_if(in_state(AppState::Editor)))
        .add_systems(
            Update,
            show_floating_tabs_ui.run_if(in_state(AppState::Editor)),
        )
        .add_systems(
            Update,
            ui::file_picker::file_picker_ui_system
                .run_if(in_state(AppState::Editor))
                .after(show_editor_ui),
        )
        .add_systems(
            Update,
            crate::gpu_text::debug_log_gpu_text_stats
                .run_if(in_state(AppState::Editor))
                .after(show_editor_ui),
        )
        .add_systems(
            Update,
            tabs_system::viewport_3d::gizmo_systems::draw_interactive_gizmos_system,
        )
        .add_systems(Update, tabs_system::viewport_3d::draw_uv_grid_system)
        // Ensure camera viewport is synced BEFORE any viewport-bound interactions (Curve tool, gizmos)
        // and BEFORE we dispatch compute/update the scene this frame (reactive mode needs correct ordering).
        .add_systems(
            Update,
            tabs_system::viewport_3d::camera_sync::sync_main_camera_viewport
                .before(dispatch_compute_tasks),
        )
        // Freeze layout mode to Desktop by default. Tablet/Phone are still available via topbar,
        // but we do not auto-switch on window resize/minimize (window width can transiently become 0).
        // .add_systems(Update, auto_switch_mobile_mode.run_if(in_state(AppState::Editor)))
        .add_systems(
            Update,
            (
                render::lighting::update_headlight_transform_system,
                render::lighting::viewport_lighting_control_system,
                render::lighting::viewport_lighting_update_system,
                render::lighting::update_scene_point_lights,
                render::lighting::update_scene_dir_lights,
                render::lighting::update_scene_spot_lights,
            ),
        )
        .add_systems(
            Update,
            (
                crate::render::viewport_draw::highlight_selected_components,
                crate::render::viewport_draw::draw_point_numbers_system,
                crate::render::viewport_draw::draw_primitive_numbers_system,
                crate::render::viewport_draw::draw_vertex_numbers_system,
                // draw_vertex_normals_system, // Replaced by CunningNormalPlugin
                // draw_primitive_normals_system, // Replaced by CunningNormalPlugin
                crate::render::viewport_draw::draw_template_wireframes_system,
            )
                .in_set(EguiSet::ProcessOutput),
        )
        .run();
}

// Scene update systems moved to `crate::scene::systems`.

/// System to sync active group visualization when the toggle changes
fn sync_active_group_toggle_system(
    mut display_options: ResMut<DisplayOptions>,
    node_graph_res: Res<NodeGraphResource>,
) {
    // Only run if DisplayOptions changed and active group highlight is enabled
    if !display_options.is_changed() {
        return;
    }
    
    if !display_options.overlays.highlight_active_group {
        return;
    }

    let node_graph = &node_graph_res.0;
    let mut point_group = None;
    let mut edge_group = None;
    let mut vertex_group = None;

    if let Some(display_node_id) = node_graph.display_node {
        if let Some(node) = node_graph.nodes.get(&display_node_id) {
            let find_param = |name: &str| -> Option<&ParameterValue> {
                node.parameters
                    .iter()
                    .find(|p| p.name == name)
                    .map(|p| &p.value)
            };

            if let Some(ParameterValue::String(name)) = find_param("group_name") {
                if !name.is_empty() {
                     if let Some(ParameterValue::Int(type_idx)) = find_param("group_type") {
                         match type_idx {
                             0 => point_group = Some(name.clone()),
                            2 => vertex_group = Some(name.clone()),
                            3 => edge_group = Some(name.clone()),
                             _ => {}
                         }
                     }
                }
            }
        }
    }
    
    // Update if different
    if display_options.overlays.point_group_viz != point_group {
        display_options.overlays.point_group_viz = point_group;
    }
    if display_options.overlays.edge_group_viz != edge_group {
        display_options.overlays.edge_group_viz = edge_group;
    }
    if display_options.overlays.vertex_group_viz != vertex_group {
        display_options.overlays.vertex_group_viz = vertex_group;
    }
}
