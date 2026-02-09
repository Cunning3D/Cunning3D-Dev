use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::cunning_core::scripting::{rhai_node::RhaiNodeOp, ScriptEngine};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use bevy::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

static RHAI_RELOAD_QUEUE: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn get_reload_queue() -> &'static Mutex<Vec<String>> {
    RHAI_RELOAD_QUEUE.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn enqueue_rhai_reload(node_name: &str) {
    let queue = get_reload_queue();
    if let Ok(mut q) = queue.lock() {
        if !q.iter().any(|n| n == node_name) {
            q.push(node_name.to_string());
        }
    }
}

pub fn drain_rhai_reload_queue() -> Vec<String> {
    let queue = get_reload_queue();
    if let Ok(mut q) = queue.lock() {
        q.drain(..).collect()
    } else {
        Vec::new()
    }
}

#[derive(Deserialize, Debug)]
pub struct NodeConfig {
    meta: NodeMeta,
    implementation: NodeImpl,
    #[serde(default)]
    parameters: HashMap<String, ParamConfig>,
}

#[derive(Deserialize, Debug)]
struct NodeMeta {
    name: String,
    category: String,
    #[serde(default)]
    version: String,
}

#[derive(Deserialize, Debug)]
struct NodeImpl {
    script: String,
    #[serde(default)]
    gizmo_hook: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ParamConfig {
    #[serde(rename = "type")]
    param_type: String,
    default: serde_json::Value, // Use serde_json::Value to handle mixed types (float/int/string)
    #[serde(default)]
    ui: String, // "Slider", "Drag", etc.
    #[serde(default)]
    min: Option<f32>,
    #[serde(default)]
    max: Option<f32>,
    #[serde(default)]
    label: Option<String>,
}

impl NodeConfig {
    pub fn script_file(&self) -> &str {
        &self.implementation.script
    }

    pub fn gizmo_hook(&self) -> Option<&str> {
        self.implementation.gizmo_hook.as_deref()
    }
}

pub fn load_rhai_plugins(engine: Res<ScriptEngine>, node_registry: Res<NodeRegistry>) {
    load_rhai_plugins_manual(engine.as_ref(), node_registry.as_ref());
}

pub fn load_rhai_plugins_manual(engine: &ScriptEngine, node_registry: &NodeRegistry) {
    let plugin_dir = Path::new("plugins");
    if !plugin_dir.exists() {
        let _ = fs::create_dir_all(plugin_dir);
        return;
    }

    // 1. Scan Directory
    if let Ok(entries) = fs::read_dir(plugin_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let config_path = path.join("node.toml");
                let dir_name = path.file_name().unwrap().to_string_lossy().to_string();

                if !config_path.exists() {
                    continue;
                }

                let config_str = match fs::read_to_string(&config_path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                let config = match toml::from_str::<NodeConfig>(&config_str) {
                    Ok(c) => c,
                    Err(e) => {
                        error!("Failed to parse node.toml for {}: {}", dir_name, e);
                        continue;
                    }
                };

                let logic_path = path.join(&config.implementation.script);
                if !logic_path.exists() {
                    continue;
                }

                let script_content = match fs::read_to_string(&logic_path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                info!("Loading Rhai Plugin: {} ({})", config.meta.name, dir_name);

                let ast = {
                    let engine_lock = engine.0.lock().unwrap();
                    match engine_lock.compile(&script_content) {
                        Ok(a) => a,
                        Err(e) => {
                            error!("Failed to compile script {}: {}", config.meta.name, e);
                            continue;
                        }
                    }
                };

                let mut ui_ast = None;
                if let Some(ui_script_name) = &config.implementation.gizmo_hook {
                    let ui_path = path.join(ui_script_name);
                    if ui_path.exists() {
                        if let Ok(ui_content) = fs::read_to_string(&ui_path) {
                            let ui_res = {
                                let engine_lock = engine.0.lock().unwrap();
                                engine_lock.compile(&ui_content)
                            };
                            match ui_res {
                                Ok(u_ast) => ui_ast = Some(Arc::new(u_ast)),
                                Err(e) => {
                                    error!("Failed to compile UI script {}: {}", ui_script_name, e)
                                }
                            }
                        }
                    }
                }

                let engine_clone = engine.clone();
                let op = RhaiNodeOp::new(&config.meta.name, ast, ui_ast, engine_clone);

                let mut parameters = Vec::new();
                let mut param_keys: Vec<_> = config.parameters.keys().collect();
                param_keys.sort();

                for key in param_keys {
                    let p_conf = &config.parameters[key];
                    let label = p_conf.label.clone().unwrap_or_else(|| key.clone());

                    let (val, ui) = match p_conf.param_type.as_str() {
                        "Float" => {
                            let v = p_conf.default.as_f64().unwrap_or(0.0) as f32;
                            let ui = if p_conf.ui == "Slider" {
                                ParameterUIType::FloatSlider {
                                    min: p_conf.min.unwrap_or(0.0),
                                    max: p_conf.max.unwrap_or(1.0),
                                }
                            } else {
                                ParameterUIType::FloatSlider {
                                    min: 0.0,
                                    max: 100.0,
                                }
                            };
                            (ParameterValue::Float(v), ui)
                        }
                        "Int" => {
                            let v = p_conf.default.as_i64().unwrap_or(0) as i32;
                            (
                                ParameterValue::Int(v),
                                ParameterUIType::IntSlider { min: 0, max: 10 },
                            )
                        }
                        "Vector3" => {
                            // Parse default as [x, y, z] if provided; fallback to ZERO
                            let (x, y, z) = if let Some(arr) = p_conf.default.as_array() {
                                let x = arr.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                                let y = arr.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                                let z = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                                (x, y, z)
                            } else {
                                (0.0, 0.0, 0.0)
                            };
                            (
                                ParameterValue::Vec3(Vec3::new(x, y, z)),
                                ParameterUIType::Vec3Drag,
                            )
                        }
                        _ => {
                            warn!("Unknown param type: {}", p_conf.param_type);
                            continue;
                        }
                    };

                    parameters.push(Parameter::new(key, &label, "General", val, ui));
                }

                use crate::cunning_core::registries::node_registry::RuntimeNodeDescriptor;
                use crate::cunning_core::traits::node_interface::{NodeInteraction, NodeOp};

                let op_clone_1 = op.clone();
                let op_factory =
                    Arc::new(move || -> Box<dyn NodeOp> { Box::new(op_clone_1.clone()) });

                let op_clone_2 = op.clone();
                let interaction_factory =
                    Arc::new(move || -> Box<dyn NodeInteraction> { Box::new(op_clone_2.clone()) });

                let params_clone = parameters.clone();
                let parameters_factory =
                    Arc::new(move || -> Vec<Parameter> { params_clone.clone() });

                node_registry.register_dynamic(RuntimeNodeDescriptor {
                    name: config.meta.name.clone(),
                    display_name: config.meta.name.clone(),
                    display_name_lc: config.meta.name.to_lowercase(),
                    category: config.meta.category.clone(),
                    op_factory,
                    interaction_factory: Some(interaction_factory),
                    coverlay_kinds: Vec::new(),
                    parameters_factory,
                    inputs: vec!["Input".to_string()],
                    outputs: vec!["Output".to_string()],
                    input_style: crate::cunning_core::registries::node_registry::InputStyle::Single,
                    node_style: crate::cunning_core::registries::node_registry::NodeStyle::Normal,
                    origin: crate::cunning_core::registries::node_registry::NodeOrigin::BuiltIn,
                });
            }
        }
    }
}

pub fn reload_single_rhai_plugin(
    engine: &ScriptEngine,
    node_registry: &NodeRegistry,
    node_name: &str,
) -> Result<(), String> {
    let base_path = Path::new("plugins").join(node_name);
    if !base_path.exists() {
        return Err(format!(
            "Plugin directory for node '{}' does not exist",
            node_name
        ));
    }

    let config_path = base_path.join("node.toml");
    let dir_name = base_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let config_str = fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read node.toml for {}: {}", dir_name, e))?;

    let config = toml::from_str::<NodeConfig>(&config_str)
        .map_err(|e| format!("Failed to parse node.toml for {}: {}", dir_name, e))?;

    let logic_path = base_path.join(config.script_file());
    if !logic_path.exists() {
        return Err(format!(
            "Logic script '{}' not found for {}",
            config.script_file(),
            dir_name
        ));
    }

    let script_content = fs::read_to_string(&logic_path)
        .map_err(|e| format!("Failed to read logic script for {}: {}", dir_name, e))?;

    info!("Reloading Rhai Plugin: {} ({})", config.meta.name, dir_name);

    let ast = {
        let engine_lock = engine
            .0
            .lock()
            .map_err(|e| format!("Rhai engine lock poisoned: {}", e))?;
        match engine_lock.compile(&script_content) {
            Ok(a) => a,
            Err(e) => {
                return Err(format!(
                    "Failed to compile script {}: {}",
                    config.meta.name, e
                ));
            }
        }
    };

    let mut ui_ast = None;
    if let Some(ui_script_name) = config.gizmo_hook() {
        let ui_path = base_path.join(ui_script_name);
        if ui_path.exists() {
            if let Ok(ui_content) = fs::read_to_string(&ui_path) {
                let ui_res = {
                    let engine_lock = engine
                        .0
                        .lock()
                        .map_err(|e| format!("Rhai engine lock poisoned: {}", e))?;
                    engine_lock.compile(&ui_content)
                };
                match ui_res {
                    Ok(u_ast) => ui_ast = Some(Arc::new(u_ast)),
                    Err(e) => {
                        return Err(format!(
                            "Failed to compile UI script {}: {}",
                            ui_script_name, e
                        ));
                    }
                }
            }
        }
    }

    let engine_clone = engine.clone();
    let op = RhaiNodeOp::new(&config.meta.name, ast, ui_ast, engine_clone);

    let mut parameters = Vec::new();
    let mut param_keys: Vec<_> = config.parameters.keys().collect();
    param_keys.sort();

    for key in param_keys {
        let p_conf = &config.parameters[key];
        let label = p_conf.label.clone().unwrap_or_else(|| key.clone());

        let (val, ui) = match p_conf.param_type.as_str() {
            "Float" => {
                let v = p_conf.default.as_f64().unwrap_or(0.0) as f32;
                let ui = if p_conf.ui == "Slider" {
                    ParameterUIType::FloatSlider {
                        min: p_conf.min.unwrap_or(0.0),
                        max: p_conf.max.unwrap_or(1.0),
                    }
                } else {
                    ParameterUIType::FloatSlider {
                        min: 0.0,
                        max: 100.0,
                    }
                };
                (ParameterValue::Float(v), ui)
            }
            "Int" => {
                let v = p_conf.default.as_i64().unwrap_or(0) as i32;
                (
                    ParameterValue::Int(v),
                    ParameterUIType::IntSlider { min: 0, max: 10 },
                )
            }
            "Vector3" => {
                let (x, y, z) = if let Some(arr) = p_conf.default.as_array() {
                    let x = arr.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                    let y = arr.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                    let z = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                    (x, y, z)
                } else {
                    (0.0, 0.0, 0.0)
                };
                (
                    ParameterValue::Vec3(Vec3::new(x, y, z)),
                    ParameterUIType::Vec3Drag,
                )
            }
            _ => {
                return Err(format!("Unknown param type: {}", p_conf.param_type));
            }
        };

        parameters.push(Parameter::new(key, &label, "General", val, ui));
    }

    use crate::cunning_core::registries::node_registry::RuntimeNodeDescriptor;
    use crate::cunning_core::traits::node_interface::{NodeInteraction, NodeOp};

    let op_clone_1 = op.clone();
    let op_factory = Arc::new(move || -> Box<dyn NodeOp> { Box::new(op_clone_1.clone()) });

    let op_clone_2 = op.clone();
    let interaction_factory =
        Arc::new(move || -> Box<dyn NodeInteraction> { Box::new(op_clone_2.clone()) });

    let params_clone = parameters.clone();
    let parameters_factory = Arc::new(move || -> Vec<Parameter> { params_clone.clone() });

    node_registry.register_dynamic(RuntimeNodeDescriptor {
        name: config.meta.name.clone(),
        display_name: config.meta.name.clone(),
        display_name_lc: config.meta.name.to_lowercase(),
        category: config.meta.category.clone(),
        op_factory,
        interaction_factory: Some(interaction_factory),
        coverlay_kinds: Vec::new(),
        parameters_factory,
        inputs: vec!["Input".to_string()],
        outputs: vec!["Output".to_string()],
        input_style: crate::cunning_core::registries::node_registry::InputStyle::Single,
        node_style: crate::cunning_core::registries::node_registry::NodeStyle::Normal,
        origin: crate::cunning_core::registries::node_registry::NodeOrigin::BuiltIn,
    });

    Ok(())
}
