use super::definitions::ToolContext;
use super::definitions::{Tool, ToolDefinition, ToolError, ToolLog, ToolLogLevel, ToolOutput};
use super::diff::compute_file_diff;
use crate::cunning_core::plugin_system::{rust_build, PluginSystem};
use crate::cunning_core::registries::node_registry::NodeRegistry;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Deserialize)]
struct CreateRustPluginArgs {
    plugin_name: String,
    #[serde(default)]
    node_name: Option<String>,
}

pub struct CreateRustPluginTool;

impl Tool for CreateRustPluginTool {
    fn name(&self) -> &str {
        "create_rust_plugin"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Create a Rust cdylib plugin crate under plugins/extra_node from the built-in template.".to_string(),
            parameters: json!({"type":"object","properties":{"plugin_name":{"type":"string"},"node_name":{"type":"string"}},"required":["plugin_name"]}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: CreateRustPluginArgs = serde_json::from_value(args)
            .map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        if args.plugin_name.contains("..")
            || args.plugin_name.contains('/')
            || args.plugin_name.contains('\\')
        {
            return Err(ToolError("Invalid plugin_name".to_string()));
        }
        let node_name = args
            .node_name
            .unwrap_or_else(|| format!("{}.Example", args.plugin_name));
        let tpl = Path::new("crates/cunning_plugin_sdk/template_rust_node");
        if !tpl.exists() {
            return Err(ToolError(
                "Template missing: crates/cunning_plugin_sdk/template_rust_node".to_string(),
            ));
        }
        let out_dir = Path::new("plugins/extra_node").join(&args.plugin_name);
        if out_dir.exists() {
            return Err(ToolError(format!(
                "Plugin dir already exists: {}",
                out_dir.display()
            )));
        }
        let _ = std::fs::create_dir_all(out_dir.join("src"))
            .map_err(|e| ToolError(format!("mkdir failed: {e}")))?;
        let cargo = std::fs::read_to_string(tpl.join("Cargo.toml"))
            .map_err(|e| ToolError(format!("read template Cargo.toml failed: {e}")))?;
        let lib = std::fs::read_to_string(tpl.join("src/lib.rs"))
            .map_err(|e| ToolError(format!("read template lib.rs failed: {e}")))?;
        std::fs::write(
            out_dir.join("Cargo.toml"),
            cargo.replace("__PLUGIN_NAME__", &args.plugin_name),
        )
        .map_err(|e| ToolError(format!("write Cargo.toml failed: {e}")))?;
        std::fs::write(
            out_dir.join("src/lib.rs"),
            lib.replace("__PLUGIN_NAME__", &args.plugin_name)
                .replace("__NODE_NAME__", &node_name),
        )
        .map_err(|e| ToolError(format!("write lib.rs failed: {e}")))?;

        let mut out = ToolOutput::new(
            format!(
                "Created Rust plugin crate at {}\nNode: {}",
                out_dir.display(),
                node_name
            ),
            vec![ToolLog {
                message: format!("Created Rust plugin {}", args.plugin_name),
                level: ToolLogLevel::Success,
            }],
        );

        // Show diffs for created files (old is empty)
        let cargo_path = out_dir.join("Cargo.toml");
        let lib_path = out_dir.join("src/lib.rs");
        if let Ok(new_cargo) = std::fs::read_to_string(&cargo_path) {
            if let Some(d) = compute_file_diff(cargo_path.to_string_lossy().to_string(), "", &new_cargo) {
                out.ui_diffs.push(d);
            }
        }
        if let Ok(new_lib) = std::fs::read_to_string(&lib_path) {
            if let Some(d) = compute_file_diff(lib_path.to_string_lossy().to_string(), "", &new_lib) {
                out.ui_diffs.push(d);
            }
        }

        Ok(out)
    }
}

#[derive(Deserialize)]
struct CompileRustPluginArgs {
    plugin_name: String,
    #[serde(default)]
    release: Option<bool>,
    #[serde(default)]
    offline: Option<bool>,
    #[serde(default)]
    locked: Option<bool>,
}

pub struct CompileRustPluginTool;

impl Tool for CompileRustPluginTool {
    fn name(&self) -> &str {
        "compile_rust_plugin"
    }
    fn is_long_running(&self) -> bool {
        true
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Compile a Rust plugin crate and copy a versioned DLL into plugins/extra_node for hot-load on Windows.".to_string(),
            parameters: json!({"type":"object","properties":{"plugin_name":{"type":"string"},"release":{"type":"boolean"},"offline":{"type":"boolean"},"locked":{"type":"boolean"}},"required":["plugin_name"]}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: CompileRustPluginArgs = serde_json::from_value(args)
            .map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        if args.plugin_name.contains("..")
            || args.plugin_name.contains('/')
            || args.plugin_name.contains('\\')
        {
            return Err(ToolError("Invalid plugin_name".to_string()));
        }
        let crate_dir = Path::new("plugins/extra_node").join(&args.plugin_name);
        if !crate_dir.join("Cargo.toml").exists() {
            return Err(ToolError(format!(
                "Missing Cargo.toml: {}",
                crate_dir.display()
            )));
        }
        // No-hitch: enqueue background job instead of blocking cargo build here.
        let mut req = crate::cunning_core::plugin_system::CompileRustPluginRequest::for_extra_node(
            args.plugin_name.clone(),
        );
        req.release = args.release.unwrap_or(true);
        req.offline = args.offline.unwrap_or(false);
        req.locked = args.locked.unwrap_or(false);
        req.hot_reload = true;
        crate::cunning_core::plugin_system::request_compile_rust_plugin(req)
            .map_err(ToolError)?;

        Ok(ToolOutput::new(
            format!(
                "Queued background build for plugin '{}'.\nThe app will hot-reload it when done.",
                args.plugin_name
            ),
            vec![ToolLog {
                message: "Build queued (AppJobs)".into(),
                level: ToolLogLevel::Info,
            }],
        ))
    }

    fn execute_with_context(
        &self,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        if ctx.is_cancelled() {
            return Err(ToolError("Cancelled".into()));
        }
        ctx.report_progress(ToolLog {
            message: "Starting cargo build...".into(),
            level: ToolLogLevel::Info,
        });
        let out = self.execute(args)?;
        if ctx.is_cancelled() {
            return Err(ToolError("Cancelled".into()));
        }
        Ok(out)
    }
}

#[derive(Deserialize)]
struct LoadRustPluginsArgs {
    #[serde(default)]
    dir: Option<String>,
    #[serde(default)]
    latest_only: Option<bool>,
}

pub struct LoadRustPluginsTool {
    registry: Arc<NodeRegistry>,
}

impl LoadRustPluginsTool {
    pub fn new(registry: Arc<NodeRegistry>) -> Self {
        Self { registry }
    }
}

impl Tool for LoadRustPluginsTool {
    fn name(&self) -> &str {
        "load_rust_plugins"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Scan a directory and load all plugin DLLs (register dynamic nodes)."
                .to_string(),
            parameters: json!({"type":"object","properties":{"dir":{"type":"string"},"latest_only":{"type":"boolean"}},"required":[]}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: LoadRustPluginsArgs = serde_json::from_value(args)
            .map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let dir = args.dir.unwrap_or_else(|| "plugins".to_string());
        let latest_only = args.latest_only.unwrap_or(true);
        let ps = PluginSystem::default();
        if latest_only {
            ps.scan_plugins_latest(&dir, &*self.registry);
        } else {
            ps.scan_plugins(&dir, &*self.registry);
        }
        Ok(ToolOutput::new(
            format!(
                "Scanned and loaded DLL plugins from {} (latest_only={})",
                dir, latest_only
            ),
            vec![ToolLog {
                message: format!("Scanned {} (latest_only={})", dir, latest_only),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

#[derive(Deserialize)]
struct RevertRustPluginArgs {
    plugin_name: String,
    #[serde(default)]
    steps_back: Option<usize>,
    #[serde(default)]
    dir: Option<String>,
}

pub struct RevertRustPluginBuildTool {
    registry: Arc<NodeRegistry>,
}
impl RevertRustPluginBuildTool {
    pub fn new(registry: Arc<NodeRegistry>) -> Self {
        Self { registry }
    }
}

impl Tool for RevertRustPluginBuildTool {
    fn name(&self) -> &str {
        "revert_rust_plugin_build"
    }
    fn is_long_running(&self) -> bool {
        true
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Revert a Rust plugin to a previous versioned DLL build, then hot-load latest-only. Copies an older {plugin}_{ts}.dll to a new timestamp to become latest.".to_string(),
            parameters: json!({"type":"object","properties":{"plugin_name":{"type":"string"},"steps_back":{"type":"number","description":"1 = previous build (default)"},"dir":{"type":"string","description":"plugin dll directory (default: plugins)"}},"required":["plugin_name"]}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: RevertRustPluginArgs = serde_json::from_value(args)
            .map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        if args.plugin_name.contains("..")
            || args.plugin_name.contains('/')
            || args.plugin_name.contains('\\')
        {
            return Err(ToolError("Invalid plugin_name".to_string()));
        }
        let steps = args.steps_back.unwrap_or(1).max(1);
        let dir = PathBuf::from(args.dir.unwrap_or_else(|| "plugins".to_string()));
        if !dir.exists() {
            return Err(ToolError(format!("Dir not found: {}", dir.display())));
        }

        let prefix = format!("{}_", args.plugin_name);
        let mut dlls: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(|e| ToolError(format!("read_dir failed: {e}")))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("dll"))
                    .unwrap_or(false)
            })
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(&prefix))
                    .unwrap_or(false)
            })
            .collect();
        // newest first by modified time
        dlls.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        dlls.reverse();
        if dlls.len() <= steps {
            return Err(ToolError(format!(
                "Not enough builds to revert: found {}, need steps_back={}",
                dlls.len(),
                steps
            )));
        }
        let chosen = dlls[steps].clone();

        let out =
            rust_build::copy_versioned(&chosen, &dir, &args.plugin_name).map_err(ToolError)?;
        let ps = PluginSystem::default();
        ps.scan_plugins_latest(dir.to_string_lossy().as_ref(), &*self.registry);
        Ok(ToolOutput::new(format!("Reverted plugin '{}'.\nChosen: {}\nCopied as latest: {}\nReloaded (latest_only=true).", args.plugin_name, chosen.display(), out.display()), vec![
            ToolLog { message: format!("Chosen prior DLL: {}", chosen.display()), level: ToolLogLevel::Info },
            ToolLog { message: format!("Copied as latest: {}", out.display()), level: ToolLogLevel::Success },
            ToolLog { message: "Reloaded plugins (latest_only=true)".to_string(), level: ToolLogLevel::Success },
        ]))
    }
    fn execute_with_context(
        &self,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        if ctx.is_cancelled() {
            return Err(ToolError("Cancelled".into()));
        }
        ctx.report_progress(ToolLog {
            message: "Reverting plugin build...".into(),
            level: ToolLogLevel::Info,
        });
        let out = self.execute(args)?;
        if ctx.is_cancelled() {
            return Err(ToolError("Cancelled".into()));
        }
        Ok(out)
    }
}
