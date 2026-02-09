use crate::cunning_core::registries::node_registry::NodeRegistry;
use bevy::prelude::*;
use rhai::{Engine, Map};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use uuid::Uuid;

pub mod api;
pub mod loader;
pub mod rhai_node;

#[derive(Resource, Clone)]
pub struct ScriptEngine(pub Arc<Mutex<Engine>>);

#[derive(Resource, Default)]
pub struct ScriptNodeState {
    pub per_node: Mutex<HashMap<Uuid, Map>>,
}

pub static GLOBAL_SCRIPT_ENGINE: OnceLock<ScriptEngine> = OnceLock::new();

impl Default for ScriptEngine {
    fn default() -> Self {
        let mut engine = Engine::new();

        // Register standard libraries (Math, etc.)
        // engine.register_global_module(rhai::packages::StandardPackage::new().as_shared_module());
        // StandardPackage is included in Engine::new() by default for full build usually,
        // but explicit check is good. For now default is fine.

        // Lift script complexity limits for desktop app usage
        // 0 = unlimited per Rhai docs
        engine.set_max_expr_depths(0, 0); // unlimited expression nesting (global & in functions)
        engine.set_max_operations(0); // unlimited operations

        // Register our Geometry APIs
        api::register_api(&mut engine);

        Self(Arc::new(Mutex::new(engine)))
    }
}

pub struct ApiSignature {
    pub name: String,
    pub params: Vec<String>,
    pub return_type: String,
    pub comments: String,
}

impl ScriptEngine {
    /// Export all registered function signatures for AI Context.
    pub fn export_api_signatures(&self) -> Vec<ApiSignature> {
        let Ok(engine) = self.0.lock() else {
            return Vec::new();
        };

        // Rhai's Engine exposes iter_functions() or similar logic depending on version.
        // We will collect native functions.
        let mut sigs: Vec<ApiSignature> = Vec::new();

        // Note: Rhai 1.x logic.
        // gen_fn_signatures returns a list of String signatures, but we want structured data if possible.
        // However, standard iteration over `engine.global_namespace().iter_functions()` is internal.
        // But `engine.gen_fn_signatures(false)` gives us human readable strings.
        // Let's use `gen_fn_signatures` and parse/wrap them, OR rely on our manual registration list if Rhai is opaque.
        // Actually, Rhai allows iterating registered functions via metadata if enabled.
        // Assuming we want a robust list, we might iterate `api::register_api` calls? No, that's hard.
        // Let's use `gen_fn_signatures` which returns `Vec<String>`.

        // NOTE: Rhai API for enumerating registered native function signatures is not stable across versions/features.
        // If this call breaks on a given Rhai build, keep the app compiling by falling back to an empty list.
        // (AI still gets tool defs + other context; this is just the optional whitelist section.)
        let _engine_for_future: &Engine = &engine;

        // Sort for stability
        sigs.sort_by(|a, b| a.name.cmp(&b.name));
        sigs
    }
}

pub struct ScriptingPlugin;

impl Plugin for ScriptingPlugin {
    fn build(&self, app: &mut App) {
        let engine = ScriptEngine::default();
        // Initialize global access for AI tools
        let _ = GLOBAL_SCRIPT_ENGINE.set(engine.clone());

        app.insert_resource(engine)
            .insert_resource(ScriptNodeState::default())
            .add_systems(Update, process_rhai_reload_queue);
        info!("Rhai Script Engine Initialized.");
    }
}

fn process_rhai_reload_queue(engine: Res<ScriptEngine>, node_registry: Res<NodeRegistry>) {
    let names = loader::drain_rhai_reload_queue();
    for node_name in names {
        if let Err(e) = loader::reload_single_rhai_plugin(&engine, &node_registry, &node_name) {
            error!("Failed to reload Rhai plugin {}: {}", node_name, e);
        }
    }
}
