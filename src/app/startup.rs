use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy::text::CosmicFontSystem;

use crate::{
    camera::CameraController,
    console,
    cunning_core::plugin_system::PluginSystem,
    debug_settings, node_editor_settings, settings, ui_settings,
    MainCamera,
};
use crate::cunning_core::registries::{node_registry::NodeRegistry, tab_registry::TabRegistry};

/// One-time init: load OS fonts into bevy_text (fix missing glyph/tofu on Windows).
pub(crate) fn init_bevy_text_system_fonts(
    mut done: Local<bool>,
    mut font_system: ResMut<CosmicFontSystem>,
) {
    if *done {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        font_system.0.db_mut().load_system_fonts();
    }
    *done = true;
}

pub(crate) fn auto_open_ai_workspace_if_missing_api(mut ui_state: ResMut<crate::ui::UiState>) {
    if !crate::cunning_core::ai_service::gemini::api_key::read_gemini_api_key_env().trim().is_empty() {
        return;
    }
    let raw = std::fs::read_to_string(crate::runtime_paths::ai_providers_path()).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
    let k = v.get("gemini").and_then(|g| g.get("api_key")).and_then(|x| x.as_str()).unwrap_or("").trim();
    if k.is_empty() {
        ui_state.pending_open_ai_workspace = true;
    }
}

pub(crate) fn setup_registries(
    mut commands: Commands,
    tab_registry: Res<TabRegistry>,
    node_registry: Res<NodeRegistry>,
    mut settings_registry: ResMut<settings::SettingsRegistry>,
    mut settings_stores: ResMut<settings::SettingsStores>,
    mut ui_settings: ResMut<ui_settings::UiSettings>,
    mut node_editor_settings: ResMut<node_editor_settings::NodeEditorSettings>,
    mut debug_settings: ResMut<debug_settings::DebugSettings>,
) {
    tab_registry.scan_and_load();
    node_registry.scan_and_load();
    settings_registry.scan_and_load();
    settings_stores.load();
    ui_settings::apply_from_settings(&*settings_registry, &*settings_stores, &mut *ui_settings);
    node_editor_settings::apply_from_settings(&*settings_registry, &*settings_stores, &mut *node_editor_settings);
    debug_settings::apply_from_settings(&*settings_registry, &*settings_stores, &mut *debug_settings);
    let plugin_system = PluginSystem::default();
    let plugin_dir = "plugins";
    if !std::path::Path::new(plugin_dir).exists() {
        let _ = std::fs::create_dir_all(plugin_dir);
        bevy::prelude::info!("Created plugin directory: {}", plugin_dir);
    }
    plugin_system.scan_plugins_latest(plugin_dir, &*node_registry);
    commands.insert_resource(plugin_system);
}

pub(crate) fn setup_3d_scene(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    console_log: Res<console::ConsoleLog>,
    existing_cameras: Query<Entity, With<MainCamera>>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
) {
    if !existing_cameras.is_empty() {
        return;
    }
    if let Some(w) = primary_window.iter().next() {
        println!("setup_3d_scene: Found PrimaryWindow entity: {:?}", w);
    } else {
        println!("setup_3d_scene: WARNING - No PrimaryWindow found!");
    }
    console_log.info("Cunning3d Modeling Software initialized!");
    console_log.info("Console system ready.");
    let _ = &mut images;
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 100.0,
        affects_lightmapped_meshes: true,
    });
    commands.spawn((
        Camera::default(),
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection::default()),
        Transform::from_xyz(-2.0, 2.5, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        CameraController::default(),
        MainCamera,
        Msaa::Sample4,
        bevy::core_pipeline::prepass::DepthPrepass,
    ));
}

pub(crate) fn register_voxel_coverlay_pressure_test_cda(
    cda_lib: Res<crate::cunning_core::cda::CdaLibrary>,
) {
    let name = "VoxelEdit PressureTest";
    if cda_lib.list_defs().iter().any(|(_id, n)| n == name) {
        return;
    }
    let mut a = crate::cunning_core::cda::CDAAsset::new(name);
    let (in_id, voxel_id, out_id) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    let mut g = crate::nodes::structs::NodeGraph::default();
    g.nodes.insert(
        in_id,
        crate::nodes::structs::Node::new(
            in_id,
            "Input".to_string(),
            crate::nodes::structs::NodeType::CDAInput("Input".to_string()),
            bevy_egui::egui::Pos2::new(-220.0, 0.0),
        ),
    );
    g.nodes.insert(
        voxel_id,
        crate::nodes::structs::Node::new(
            voxel_id,
            "Voxel Edit".to_string(),
            crate::nodes::structs::NodeType::VoxelEdit,
            bevy_egui::egui::Pos2::new(0.0, 0.0),
        ),
    );
    g.nodes.insert(
        out_id,
        crate::nodes::structs::Node::new(
            out_id,
            "Output".to_string(),
            crate::nodes::structs::NodeType::CDAOutput("Output".to_string()),
            bevy_egui::egui::Pos2::new(220.0, 0.0),
        ),
    );
    let mk = |from: uuid::Uuid,
              to: uuid::Uuid,
              from_port: crate::nodes::PortId,
              to_port: crate::nodes::PortId,
              order: i32| crate::nodes::structs::Connection {
        id: uuid::Uuid::new_v4(),
        from_node: from,
        from_port,
        to_node: to,
        to_port,
        order,
        waypoints: Vec::new(),
    };
    g.connections.insert(uuid::Uuid::new_v4(), mk(in_id, voxel_id, crate::nodes::port_key::out0(), crate::nodes::port_key::in0(), 0));
    g.connections.insert(uuid::Uuid::new_v4(), mk(voxel_id, out_id, crate::nodes::port_key::out0(), crate::nodes::port_key::in0(), 1));
    a.inner_graph = g;
    a.inputs = vec![crate::cunning_core::cda::CDAInterface::new("Input", in_id)];
    a.outputs = vec![crate::cunning_core::cda::CDAInterface::new("Output", out_id)];
    a.coverlay_units.push(crate::cunning_core::cda::asset::CdaCoverlayUnit {
        node_id: voxel_id,
        label: "VoxelEdit".to_string(),
        icon: Some("🧊".to_string()),
        order: 0,
        default_on: true,
    });
    let _ = cda_lib.insert_in_memory(a);
}

