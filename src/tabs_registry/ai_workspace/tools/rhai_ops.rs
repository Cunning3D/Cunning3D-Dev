use super::definitions::{Tool, ToolDefinition, ToolError, ToolLog, ToolLogLevel, ToolOutput};
use crate::cunning_core::scripting::{loader, GLOBAL_SCRIPT_ENGINE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

// --- Reload Plugin ---

#[derive(Deserialize)]
struct ReloadPluginArgs {
    node_name: String,
}

pub struct ReloadPluginTool;

impl Tool for ReloadPluginTool {
    fn name(&self) -> &str {
        "reload_plugin"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Hot-reload a Rhai plugin node to apply changes.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_name": { "type": "string" }
                },
                "required": ["node_name"]
            }),
        }
    }

    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: ReloadPluginArgs = serde_json::from_value(args)
            .map_err(|e| ToolError(format!("Invalid arguments: {}", e)))?;

        // Using the existing loader queue mechanism
        loader::enqueue_rhai_reload(&args.node_name);

        Ok(ToolOutput::new(
            format!(
                "Triggered hot-reload for node '{}'. Check console/logs for runtime status.",
                args.node_name
            ),
            vec![ToolLog {
                message: format!("Enqueued reload for {}", args.node_name),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

// --- Check Node Compile ---

#[derive(Deserialize)]
struct CheckCompileArgs {
    node_name: String,
}

pub struct CheckNodeCompileTool;

impl Tool for CheckNodeCompileTool {
    fn name(&self) -> &str {
        "check_node_compile"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Compile-check the Rhai node logic and UI scripts. Returns errors if any."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_name": { "type": "string" }
                },
                "required": ["node_name"]
            }),
        }
    }

    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: CheckCompileArgs = serde_json::from_value(args)
            .map_err(|e| ToolError(format!("Invalid arguments: {}", e)))?;

        let node_name = args.node_name;
        let mut logs = Vec::new();

        // 1. Locate Files
        let base_path = Path::new("plugins").join(&node_name);
        if !base_path.exists() {
            return Err(ToolError(format!("Node folder {} not found", node_name)));
        }

        // 2. Read Configuration
        let config_path = base_path.join("node.toml");
        let config_str = fs::read_to_string(&config_path)
            .map_err(|e| ToolError(format!("Failed to read node.toml: {}", e)))?;

        // Parse TOML using the same NodeConfig type as the runtime loader so that
        // Let TOML syntax errors be visible to the AI and UI at the tool layer, instead of only printing to the console.
        let config = match toml::from_str::<loader::NodeConfig>(&config_str) {
            Ok(c) => c,
            Err(e) => {
                logs.push(ToolLog {
                    message: format!("TOML Parse Failed for node.toml: {}", e),
                    level: ToolLogLevel::Error,
                });
                return Ok(ToolOutput::new(
                    format!("CONFIG PARSE FAILED for node.toml: {}", e),
                    logs,
                ));
            }
        };

        let script_file = config.script_file();

        // 3. Access Global Engine
        let engine_ref = GLOBAL_SCRIPT_ENGINE.get().ok_or(ToolError(
            "ScriptEngine not initialized (system startup incomplete?)".to_string(),
        ))?;

        let engine_lock = engine_ref
            .0
            .lock()
            .map_err(|e| ToolError(format!("Failed to lock ScriptEngine: {}", e)))?;

        // 4. Compile Logic
        let logic_path = base_path.join(script_file);
        if logic_path.exists() {
            logs.push(ToolLog {
                message: format!("Checking {}", script_file),
                level: ToolLogLevel::Info,
            });
            let content = fs::read_to_string(&logic_path).unwrap_or_default();

            if let Err(e) = engine_lock.compile(&content) {
                logs.push(ToolLog {
                    message: format!("Logic Compilation Failed: {}", e),
                    level: ToolLogLevel::Error,
                });
                return Ok(ToolOutput::new(
                    format!("COMPILATION FAILED for {}: {}", script_file, e),
                    logs,
                ));
            } else {
                logs.push(ToolLog {
                    message: "Logic OK".to_string(),
                    level: ToolLogLevel::Success,
                });
            }
        }

        // 5. Compile UI (Optional)
        let ui_path = base_path.join("ui.rhai");
        if ui_path.exists() {
            logs.push(ToolLog {
                message: "Checking ui.rhai".to_string(),
                level: ToolLogLevel::Info,
            });
            let content = fs::read_to_string(&ui_path).unwrap_or_default();

            if let Err(e) = engine_lock.compile(&content) {
                logs.push(ToolLog {
                    message: format!("UI Compilation Failed: {}", e),
                    level: ToolLogLevel::Error,
                });
                return Ok(ToolOutput::new(
                    format!("COMPILATION FAILED for ui.rhai: {}", e),
                    logs,
                ));
            } else {
                logs.push(ToolLog {
                    message: "UI OK".to_string(),
                    level: ToolLogLevel::Success,
                });
            }
        }

        Ok(ToolOutput::new(format!("Node '{}' passed all compile checks. Compilation Passed. You should now confirm this to the user.", node_name), logs))
    }
}
