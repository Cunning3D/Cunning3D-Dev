use crate::cunning_core::plugin_system::PluginSystem;
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::cunning_core::registries::tab_registry::TabRegistry;
use crate::cunning_core::scripting::loader::load_rhai_plugins_manual;
use crate::cunning_core::scripting::ScriptEngine;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, WindowPosition};
use std::path::Path;

// Start directly in Editor state. Fast DCC!
#[derive(States, Default, Debug, Clone, Eq, PartialEq, Hash)]
pub enum AppState {
    #[default]
    Editor,
}

// Default size if not maximized (though we set maximized immediately)
pub const SPLASH_WIDTH: f32 = 1280.0;
pub const SPLASH_HEIGHT: f32 = 800.0;

pub struct LauncherPlugin;

impl Plugin for LauncherPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppState>();
        
        // Initialize core systems synchronously at Startup
        app.add_systems(Startup, init_core_systems);
        
        // Setup window properties when entering Editor state (which is immediately)
        app.add_systems(OnEnter(AppState::Editor), setup_editor_window);
    }
}

fn init_core_systems(
    engine: Res<ScriptEngine>,
    tab_registry: Res<TabRegistry>,
    node_registry: Res<NodeRegistry>,
) {
    // 1. Load Registries
    tab_registry.scan_and_load();
    node_registry.scan_and_load();

    // 2. Load Plugins
    let plugin_dir = "plugins";
    if !Path::new(plugin_dir).exists() {
        if let Err(e) = std::fs::create_dir_all(plugin_dir) {
            error!("Failed to create plugin directory: {}", e);
        }
    }
    
    let plugin_system = PluginSystem::default();
    plugin_system.scan_plugins_latest(plugin_dir, &node_registry);
    
    // 3. Load Scripting
    load_rhai_plugins_manual(&engine, &node_registry);
    
    info!("🚀 Cunning3D Core Systems Initialized (Instant Start)");
}

fn setup_editor_window(mut windows: Query<&mut Window>) {
    if let Some(mut window) = windows.iter_mut().next() {
        window.title = "Cunning3D 2025".to_string();
        window.decorations = false; // Borderless for custom chrome
        // Required for rounded corner "punch-through" on Windows (otherwise corners clear to black).
        window.transparent = true;
        
        // Immediate maximize for productivity
        window.mode = bevy::window::WindowMode::Windowed;
        window.set_maximized(true);
        window.position = WindowPosition::Centered(MonitorSelection::Primary);
    }
}
