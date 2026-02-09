//! Workspace exploration, search, read, and web search tools with async/cancellation support.
use super::definitions::{
    Tool, ToolContext, ToolDefinition, ToolError, ToolLog, ToolLogLevel, ToolOutput,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

const ALLOWED_ROOTS: &[&str] = &["plugins", "plugins/extra_node", "crates", "src"];
const ALLOWED_REF_FILES: &[&str] = &[
    "src/cunning_core/plugin_system/c_api.rs",
    "src/cunning_core/plugin_system/mod.rs",
    "src/cunning_core/plugin_system/rust_build.rs",
    "crates/cunning_plugin_sdk/template_rust_node/src/lib.rs",
];
const ZED_MAIN_ROOT: &str = "../zed-main";

// ==========================================
// Tool 1: Explore Workspace
// ==========================================

pub struct ExploreWorkspaceTool;

impl Tool for ExploreWorkspaceTool {
    fn name(&self) -> &str {
        "explore_workspace"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "List directory structure. Default root is 'plugins'. Use sub_path like 'extra_node', 'crates', 'src'.".to_string(),
            parameters: json!({"type":"object","properties":{"sub_path":{"type":"string","description":"Optional path. Examples: '' (plugins root), 'extra_node', 'plugins/extra_node', 'crates', 'src'."}}}),
        }
    }

    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let sub_path = args.get("sub_path").and_then(|v| v.as_str()).unwrap_or("");
        if sub_path.contains("..") {
            return Err(ToolError("Invalid path traversal".into()));
        }
        let sp = sub_path.trim().trim_start_matches("./").trim_start_matches(".\\");
        let sp_norm = sp.replace("\\", "/").trim_matches('/').to_string();
        let root_path = if sp_norm.is_empty() || sp_norm == "plugins" {
            Path::new("plugins").to_path_buf()
        } else if let Some(rest) = sp_norm.strip_prefix("plugins/") {
            if rest.is_empty() { Path::new("plugins").to_path_buf() } else { Path::new("plugins").join(rest) }
        } else if sp_norm == "crates" {
            Path::new("crates").to_path_buf()
        } else if let Some(rest) = sp_norm.strip_prefix("crates/") {
            Path::new("crates").join(rest)
        } else if sp_norm == "src" {
            Path::new("src").to_path_buf()
        } else if let Some(rest) = sp_norm.strip_prefix("src/") {
            Path::new("src").join(rest)
        } else {
            // Backward-compatible default: treat as plugins subdir
            Path::new("plugins").join(sp_norm)
        };
        if !root_path.exists() {
            return Err(ToolError(format!(
                "Path not found: {}. Use sub_path like '', 'extra_node', 'crates', or 'src'.",
                root_path.display()
            )));
        }

        let mut out = format!("📁 Project Structure ({})\n", root_path.display());
        let mut count = 0;
        for entry in WalkDir::new(&root_path)
            .min_depth(1)
            .max_depth(3)
            .sort_by_file_name()
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let name = entry.file_name().to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let indent = "  ".repeat(entry.depth());
            out.push_str(&format!(
                "{}{} {}{}\n",
                indent,
            if entry.file_type().is_dir() {
                    "📂"
            } else {
                    "📄"
                },
                name,
                if entry.file_type().is_dir() { "/" } else { "" }
            ));
            count += 1;
        }
        if count == 0 {
            out.push_str("  (Empty directory)\n");
        }
        Ok(ToolOutput::new(
            out,
            vec![ToolLog {
                    message: format!("Explored {}", root_path.display()),
                level: ToolLogLevel::Info,
            }],
        ))
    }
}

// ==========================================
// Tool 2: Search Workspace
// ==========================================

#[derive(Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    extensions: Option<Vec<String>>,
    #[serde(default)]
    roots: Option<Vec<String>>,
}

pub struct SearchWorkspaceTool;

impl Tool for SearchWorkspaceTool {
    fn name(&self) -> &str {
        "search_workspace"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Search text across workspace roots. Default root is 'plugins'. Allowed roots: plugins, src, crates.".to_string(),
            parameters: json!({"type":"object","properties":{"query":{"type":"string","description":"Text to search"},"extensions":{"type":"array","items":{"type":"string"},"description":"File extensions filter"},"roots":{"type":"array","items":{"type":"string","enum":["plugins","src","crates"]},"description":"Roots to search (default: [\"plugins\"])"} },"required":["query"]}),
        }
    }

    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: SearchArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid args: {e}")))?;
        let roots = args
            .roots
            .unwrap_or_else(|| vec!["plugins".to_string()])
            .into_iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if roots.is_empty() {
            return Err(ToolError("roots is empty".into()));
        }
        for r in &roots {
            if r != "plugins" && r != "src" && r != "crates" {
                return Err(ToolError(format!("Invalid root '{}'. Allowed: plugins, src, crates", r)));
            }
            if !Path::new(r).exists() {
                return Err(ToolError(format!("Root directory not found: {}", r)));
            }
        }

        let (mut out, mut count) = (String::new(), 0usize);
        let q = args.query.to_lowercase();
        for root in roots {
            let root_path = Path::new(&root);
            for entry in WalkDir::new(root_path)
                .sort_by_file_name()
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                if let Some(exts) = &args.extensions {
                    let ext = entry
                        .path()
                        .extension()
                        .map(|e| e.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if !exts.contains(&ext) {
                        continue;
                    }
                }
                let Ok(content) = fs::read_to_string(entry.path()) else {
                    continue;
                };
                let mut file_matches = Vec::new();
                for (i, line) in content.lines().enumerate() {
                    if line.to_lowercase().contains(&q) {
                        file_matches.push((i + 1, line.trim()));
                        count += 1;
                        if count >= 50 {
                            break;
                        }
                    }
                }
                if !file_matches.is_empty() {
                    out.push_str(&format!("\n📄 {}:\n", entry.path().display()));
                    for (ln, code) in file_matches {
                        out.push_str(&format!(
                            "  L{:03}: {}\n",
                            ln,
                            if code.len() > 120 { &code[..120] } else { code }
                        ));
                    }
                }
                if count >= 50 {
                    out.push_str("\n... (truncated)");
                    break;
                }
            }
            if count >= 50 {
                break;
            }
        }
        if count == 0 {
            return Ok(ToolOutput::new(
                format!("No matches for '{}'", args.query),
                vec![ToolLog {
                    message: "No matches".into(),
                    level: ToolLogLevel::Info,
                }],
            ));
        }
        Ok(ToolOutput::new(
            out,
            vec![ToolLog {
                message: format!("Found {} matches", count),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

// ==========================================
// Tool 3: Read File
// ==========================================

#[derive(Deserialize)]
struct ReadFileArgs {
    file_path: String,
    #[serde(default = "d1")]
    offset: usize,
    #[serde(default = "d500")]
    limit: usize,
}
fn d1() -> usize {
    1
}
fn d500() -> usize {
    500
}

pub struct ReadFileTool;

impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Read file content with line numbers. Allowed: plugins/*, crates/*, src/*, zed-main/*, and reference files.".to_string(),
            parameters: json!({"type":"object","properties":{"file_path":{"type":"string","description":"Path (e.g. 'extra_node/curve_plugin/src/lib.rs' or 'c_api.rs')"},"offset":{"type":"number","description":"1-indexed start line"},"limit":{"type":"number","description":"Max lines"}},"required":["file_path"]}),
        }
    }

    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let a: ReadFileArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid args: {e}")))?;
        if a.offset == 0 || a.limit == 0 {
            return Err(ToolError("offset/limit must be >= 1".into()));
        }
        if a.file_path.contains("..") {
            return Err(ToolError("Path traversal not allowed".into()));
        }

        let file_path = a
            .file_path
            .trim_start_matches("plugins/")
            .trim_start_matches("plugins\\")
            .trim_start_matches("plugins")
            .trim_start_matches('/')
            .trim_start_matches('\\')
            .to_string();

        let mut resolved: Option<std::path::PathBuf> = None;
        if file_path == "zed-main"
            || file_path.starts_with("zed-main/")
            || file_path.starts_with("zed-main\\")
        {
            let rel = file_path
                .trim_start_matches("zed-main/")
                .trim_start_matches("zed-main\\")
                .trim_start_matches("zed-main");
            let p = Path::new(ZED_MAIN_ROOT).join(rel);
            if p.exists() && p.is_file() {
                resolved = Some(p);
            }
        }
        for rf in ALLOWED_REF_FILES {
            if file_path == *rf || rf.ends_with(&file_path) {
                let p = Path::new(rf);
                if p.exists() {
                    resolved = Some(p.to_path_buf());
                    break;
                }
            }
        }
        if resolved.is_none() {
            for root in ALLOWED_ROOTS {
                let p = Path::new(root).join(&file_path);
                if p.exists() && p.is_file() {
                    resolved = Some(p);
                    break;
                }
            }
        }
        let allowed_msg = format!(
            "Allowed: plugins/*, crates/*, src/*, zed-main/*, {:?}",
            ALLOWED_REF_FILES
        );
        let path = match resolved {
            Some(p) => p,
            None => {
                let direct = Path::new(&file_path);
                if direct.exists() && direct.is_file() {
                    return Err(ToolError(format!("Access denied. {allowed_msg}")));
                }
                return Err(ToolError(format!("File not found. {allowed_msg}")));
            }
        };

        let content =
            fs::read_to_string(&path).map_err(|e| ToolError(format!("Read error: {e}")))?;
        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        if a.offset > total {
            return Err(ToolError(format!(
                "offset {} > file length {}",
                a.offset, total
            )));
        }

        let (start, end) = (a.offset - 1, (a.offset - 1 + a.limit).min(total));
        let mut out = String::new();
        for (i, line) in lines[start..end].iter().enumerate() {
            let ln = start + i + 1;
            out.push_str(&format!(
                "L{}: {}\n",
                ln,
                if line.len() > 200 {
                    &line[..200]
                } else {
                    *line
                }
            ));
        }
        let remaining = total.saturating_sub(end);
        if remaining > 0 {
            out.push_str(&format!(
                "\n... ({} more lines, total {})\n",
                remaining, total
            ));
        }
        // LLM needs a real excerpt (not "OK") to avoid re-requesting read_file; keep it small but useful.
        let mut o =
            ToolOutput::with_summary(out.lines().take(120).collect::<Vec<_>>().join("\n"), out);
        o.ui_logs = vec![ToolLog {
            message: format!("Read {} lines from {}", end - start, path.display()),
            level: ToolLogLevel::Success,
        }];
        Ok(o)
    }
}

// ==========================================
// Tool 4: Web Search (docs.rs priority + multi-source)
// ==========================================

#[derive(Deserialize)]
struct WebSearchArgs {
    query: String,
    #[serde(default = "d5")]
    max_results: usize,
    #[serde(default)]
    source: Option<String>,
}
fn d5() -> usize {
    5
}

pub struct WebSearchTool;

impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }
    fn is_long_running(&self) -> bool {
        true
    } // Network IO

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Search web for Rust docs/APIs. Sources: 'docs' (docs.rs), 'crates' (crates.io), 'ddg' (DuckDuckGo). Default: smart routing.".to_string(),
            parameters: json!({"type":"object","properties":{"query":{"type":"string","description":"Search query (e.g. 'wgpu Buffer')"},"max_results":{"type":"number","description":"Max results (default 5)"},"source":{"type":"string","description":"Source: docs/crates/ddg (optional)"}},"required":["query"]}),
        }
    }

    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let a: WebSearchArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid args: {e}")))?;
        if a.query.trim().is_empty() {
            return Err(ToolError("Query empty".into()));
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| ToolError(format!("HTTP error: {e}")))?;
        let ua = "Cunning3D-AI/1.0";

        // Smart source routing: if query looks like crate name, try crates.io first
        let source = a.source.as_deref().unwrap_or_else(|| {
            let q = a.query.to_lowercase();
            if q.split_whitespace().count() == 1
                && q.chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            {
                "crates"
            } else if q.contains("docs") || q.contains("api") || q.contains("example") {
                "docs"
                    } else {
                "ddg"
            }
        });

        let mut out = format!("🔍 Search: {} (source: {})\n\n", a.query, source);

        match source {
            "crates" => {
                let url = format!(
                    "https://crates.io/api/v1/crates?q={}&per_page={}",
                    urlencoding::encode(&a.query),
                    a.max_results
                );
                if let Ok(resp) = client.get(&url).header("User-Agent", ua).send() {
                    if let Ok(body) = resp.json::<Value>() {
                        if let Some(crates) = body.get("crates").and_then(|v| v.as_array()) {
                            for (i, c) in crates.iter().take(a.max_results).enumerate() {
                                let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                let desc =
                                    c.get("description").and_then(|v| v.as_str()).unwrap_or("");
                                let ver =
                                    c.get("max_version").and_then(|v| v.as_str()).unwrap_or("?");
                                let dl = c.get("downloads").and_then(|v| v.as_u64()).unwrap_or(0);
                                out.push_str(&format!("{}. 📦 **{}** v{} ({} downloads)\n   {}\n   https://docs.rs/{}\n\n", i + 1, name, ver, dl, desc, name));
                            }
                        }
                    }
                }
            }
            "docs" => {
                // docs.rs doesn't have a public search API, redirect to crates.io + construct docs.rs link
                let url = format!(
                    "https://crates.io/api/v1/crates?q={}&per_page={}",
                    urlencoding::encode(&a.query),
                    a.max_results
                );
                if let Ok(resp) = client.get(&url).header("User-Agent", ua).send() {
                    if let Ok(body) = resp.json::<Value>() {
                        if let Some(crates) = body.get("crates").and_then(|v| v.as_array()) {
                            out.push_str("📚 **Docs.rs Links**:\n");
                            for c in crates.iter().take(a.max_results) {
                                let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                let ver = c
                                    .get("max_version")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("latest");
                                out.push_str(&format!(
                                    "  • {} → https://docs.rs/{}/{}\n",
                                    name, name, ver
                                ));
                            }
                            out.push('\n');
                        }
                    }
                }
            }
            _ => {
                // DuckDuckGo Instant Answer
                let url = format!(
                    "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
                    urlencoding::encode(&a.query)
                );
                if let Ok(resp) = client.get(&url).header("User-Agent", ua).send() {
                    if let Ok(body) = resp.json::<Value>() {
                        if let Some(abs) = body
                            .get("Abstract")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                        {
                            out.push_str(&format!("📝 {}\n", abs));
                            if let Some(url) = body.get("AbstractURL").and_then(|v| v.as_str()) {
                                out.push_str(&format!("   {}\n\n", url));
                            }
                        }
                        if let Some(topics) = body.get("RelatedTopics").and_then(|v| v.as_array()) {
                            for (i, t) in topics.iter().take(a.max_results).enumerate() {
                                if let Some(text) = t.get("Text").and_then(|v| v.as_str()) {
                                    let url =
                                        t.get("FirstURL").and_then(|v| v.as_str()).unwrap_or("");
                                    out.push_str(&format!("{}. {}\n   {}\n\n", i + 1, text, url));
                                }
                            }
                        }
                    }
                }
            }
        }

        if out.len() < 60 {
            out.push_str(
                "No detailed results. Try: web_search with source='crates' for Rust crates.\n",
            );
        }
        Ok(ToolOutput::new(
            out,
            vec![ToolLog {
                message: format!("Searched: {}", a.query),
                level: ToolLogLevel::Success,
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
            message: "Starting web search...".into(),
            level: ToolLogLevel::Info,
        });
        let result = self.execute(args);
        if ctx.is_cancelled() {
            return Err(ToolError("Cancelled".into()));
        }
        result
    }
}

// ==========================================
// Tool 5: Get NodeSpec Template (on-demand docs)
// ==========================================

pub struct GetNodeSpecTemplateTool;

impl Tool for GetNodeSpecTemplateTool {
    fn name(&self) -> &str {
        "get_nodespec_template"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Get NodeSpec JSON template for creating new Rust plugin nodes. Call this before apply_rust_nodespec.".to_string(),
            parameters: json!({"type":"object","properties":{"variant":{"type":"string","description":"Template: 'basic', 'with_params', 'with_interaction' (default: basic)"}}})
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let variant = args
            .get("variant")
            .and_then(|v| v.as_str())
            .unwrap_or("basic");
        let template = match variant {
            "with_params" => {
                r#"## NodeSpec with Parameters
{
  "plugin_name": "my_plugin",
  "node": {
    "name": "My Plugin",
    "category": "Experimental",
    "inputs": ["Input"],
    "outputs": ["Output"],
    "params": [
      { "name": "count", "type": "int", "default": 100 },
      { "name": "seed", "type": "int", "default": 0 },
      { "name": "scale", "type": "float", "default": 1.0 }
    ]
  },
  "smoke_test": { "connect_from": "Create Sphere", "expect_point_count_gte": 1 }
}"#
            }
            "with_interaction" => {
                r#"## NodeSpec with Interaction (HUD/Gizmo/Input)
{
  "plugin_name": "my_interactive_plugin",
  "node": {
    "name": "Interactive Node",
    "category": "Experimental",
    "inputs": ["Input"],
    "outputs": ["Output"],
    "params": [{ "name": "offset", "type": "vec3", "default": [0,0,0] }]
  },
  "interaction": {
    "hud_commands": [
      { "tag": "button", "id": 1, "text": "Reset" },
      { "tag": "toggle", "id": 2, "text": "Lock" }
    ],
    "gizmo_primitives": [
      { "pick_id": 1, "primitive": "sphere", "position": [0,0,0], "scale": 0.1, "color": [1,0.5,0,1] }
    ],
    "input_keys": [
      { "key": "r", "action": "Reset offset to origin" }
    ]
  },
  "smoke_test": { "connect_from": "Create Cube" }
}"#
            }
            _ => {
                r#"## Basic NodeSpec Template (Minimal, Ready-to-Run)
{
  "plugin_name": "my_plugin",
  "node": {
    "name": "My Plugin",
    "category": "Experimental",
    "inputs": ["Input"],
    "outputs": ["Output"],
    "params": []
  }
}

## CRITICAL SCHEMA RULES:
- plugin_name: string at TOP LEVEL (required)
- node.name: string (required)
- node.inputs / node.outputs: arrays of STRINGS, e.g. ["Input"], not objects
- node.params: array of {name:string, type:string, default:value}
  - type: "int"|"float"|"bool"|"string"|"vec2"|"vec3"|"vec4"|"color3"|"color4"
- To compile: call apply_rust_nodespec with {"nodespec":{...}, "build":true}
- Without build=true, only code files are written (no cargo build).

## Param Example:
"params": [{"name":"scale","type":"float","default":1.0}]
"#
            }
        };
        let llm = match variant {
            "with_params" => {
                r#"{"plugin_name":"my_plugin","node":{"name":"My Plugin","category":"Experimental","inputs":["Input"],"outputs":["Output"],"params":[{"name":"count","type":"int","default":100},{"name":"seed","type":"int","default":0},{"name":"scale","type":"float","default":1.0}]}}"#
            }
            "with_interaction" => {
                r#"{"plugin_name":"my_interactive_plugin","node":{"name":"Interactive Node","category":"Experimental","inputs":["Input"],"outputs":["Output"],"params":[{"name":"offset","type":"vec3","default":[0,0,0]}]},"interaction":{"hud_commands":[{"tag":"button","id":1,"text":"Reset"}],"gizmo_primitives":[{"pick_id":1,"primitive":"sphere","position":[0,0,0],"scale":[0.1,0.1,0.1],"color":[1,0.5,0,1]}],"input_keys":[{"key":"f","action":"Reset offset to origin"}]}}"#
            }
            _ => {
                r#"{"plugin_name":"my_plugin","node":{"name":"My Plugin","category":"Experimental","inputs":["Input"],"outputs":["Output"],"params":[]}}"#
            }
        };
        let mut o = ToolOutput::with_summary(llm, template.to_string());
        o.ui_logs = vec![ToolLog {
            message: format!("NodeSpec template ({})", variant),
            level: ToolLogLevel::Success,
        }];
        Ok(o)
    }
}

// ==========================================
// Tool 6: Get ABI Reference (on-demand docs)
// ==========================================

pub struct GetAbiReferenceTool;

impl Tool for GetAbiReferenceTool {
    fn name(&self) -> &str {
        "get_abi_reference"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Get C ABI reference for plugin development (CHostApi, CHudCmd, CGizmoCmd, CInputEvent).".to_string(),
            parameters: json!({"type":"object","properties":{"topic":{"type":"string","description":"Topic: 'hostapi', 'hud', 'gizmo', 'input', 'all' (default: all)"}}})
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let topic = args.get("topic").and_then(|v| v.as_str()).unwrap_or("all");
        let mut out = String::new();

        let hostapi = r#"## CHostApi Functions (from c_api.rs)
- node_read_points(geo, out_ptr, out_len): Read point positions [x,y,z,x,y,z,...]
- node_write_points(geo, ptr, len): Write point positions back
- node_point_count(geo) -> usize: Get point count
- node_prim_count(geo) -> usize: Get primitive count
- node_param_int(name) / node_param_float(name) / node_param_bool(name): Read parameters
- node_param_vec3(name, out): Read Vec3 into [f32;3]
- node_state_get(key, out, out_len) / node_state_set(key, data, len): Persistent state per node instance
- node_curve_get(name, out) / node_curve_set(name, data): Curve ramp parameters
- log_info(msg) / log_warn(msg) / log_error(msg): Console logging
"#;
        let hud = r#"## CHudCmd Structure (HUD rendering)
- tag: 0=Label, 1=Button, 2=Toggle, 3=Separator
- id: Unique ID for event routing
- value: Toggle state (0/1) or slider value
- text: Display text (max 64 chars)
Return array of CHudCmd from hud_build callback.
"#;
        let gizmo = r#"## CGizmoCmd Structure (3D gizmos)
- tag: 0=None, 1=Sphere, 2=Cube, 3=Cylinder, 4=Cone, 5=Plane
- pick_id: Non-zero for hit-testing (0 = no picking)
- transform: [f32;16] column-major 4x4 matrix
- color: [f32;4] RGBA
Return array of CGizmoCmd from gizmo_build callback.
"#;
        let input = r#"## CInputEvent Structure (keyboard input)
- tag: 0=None, 1=KeyPressed
- key: ASCII code or special (32=Space, 13=Enter, 27=Escape, 127=Delete, 8=Backspace)
Handle in input_event callback when node is selected.
"#;
        match topic {
            "hostapi" => out.push_str(hostapi),
            "hud" => out.push_str(hud),
            "gizmo" => out.push_str(gizmo),
            "input" => out.push_str(input),
            _ => {
                out.push_str(hostapi);
                out.push_str(hud);
                out.push_str(gizmo);
                out.push_str(input);
            }
        }
        out.push_str("\nFor full source: call read_file(file_path=\"c_api.rs\")\n");
        Ok(ToolOutput::new(
            out,
            vec![ToolLog {
                message: format!("ABI reference ({})", topic),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

// ==========================================
// Tool 7: Get Interaction Guide (on-demand docs)
// ==========================================

pub struct GetInteractionGuideTool;

impl Tool for GetInteractionGuideTool {
    fn name(&self) -> &str {
        "get_interaction_guide"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Get guide for implementing HUD/Gizmo/Input interaction in Rust plugins."
                .to_string(),
            parameters: json!({"type":"object","properties":{}}),
        }
    }
    fn execute(&self, _args: Value) -> Result<ToolOutput, ToolError> {
        let guide = r#"## Interaction Guide for Rust Plugins

### 1. Add interaction to NodeSpec
```json
"interaction": {
  "hud_commands": [{ "tag": "button", "id": 1, "text": "Action" }],
  "gizmo_primitives": [{ "pick_id": 1, "primitive": "sphere", "position": [0,0,0], "scale": 0.1, "color": [1,0,0,1] }],
  "input_keys": [{ "key": "r", "action": "Reset" }]
}
```

### 2. Generated Callbacks
apply_rust_nodespec generates these in lib.rs:
- `hud_build(api) -> Vec<CHudCmd>`: Return HUD elements to render
- `hud_event(api, id, value)`: Called when button clicked / toggle changed
- `gizmo_build(api) -> Vec<CGizmoCmd>`: Return 3D gizmos to render
- `gizmo_event_drag(api, pick_id, world_pos)`: Called when gizmo is dragged
- `gizmo_event_click(api, pick_id)`: Called when gizmo is clicked
- `input_event(api, key)`: Called when key pressed while node selected

### 3. Persisting State
Use node_state_get/node_state_set for data that survives recompute:
```rust
// In gizmo_event_drag:
let pos_bytes = bytemuck::bytes_of(&world_pos);
api.node_state_set("gizmo_pos", pos_bytes);
api.mark_dirty(); // Trigger recompute

// In compute or gizmo_build:
let mut pos = [0f32; 3];
api.node_state_get("gizmo_pos", bytemuck::bytes_of_mut(&mut pos));
```

### 4. Reference Implementation
See: plugins/extra_node/curve_plugin/src/lib.rs (full HUD+Gizmo+Input example)

### 5. Use get_interaction_template for pre-built patterns
Call get_interaction_template with pattern='drag_point'|'bezier_handle'|'transform_gizmo'|'multi_select'.
"#;
        Ok(ToolOutput::new(
            guide.to_string(),
            vec![ToolLog {
                message: "Interaction guide".into(),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

// ==========================================
// Tool 8: Get Interaction Template (common patterns)
// ==========================================

pub struct GetInteractionTemplateTool;

impl Tool for GetInteractionTemplateTool {
    fn name(&self) -> &str {
        "get_interaction_template"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Get pre-built interaction code templates. Patterns: drag_point, bezier_handle, transform_gizmo, multi_select.".to_string(),
            parameters: json!({"type":"object","properties":{"pattern":{"type":"string","description":"Pattern: drag_point|bezier_handle|transform_gizmo|multi_select"}},"required":["pattern"]})
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError("pattern is required".into()))?;
        let template = match pattern {
            "drag_point" => r#"## Drag Point Template
Single draggable sphere that stores position in node_state and applies offset to geometry.

### NodeSpec interaction:
"interaction": {
  "gizmo_primitives": [{ "pick_id": 1, "primitive": "sphere", "position": [0,0,0], "scale": 0.08, "color": [1,0.5,0,1] }],
  "hud_commands": [{ "tag": "button", "id": 1, "text": "Reset Position" }]
}

### USER_CODE snippet (gizmo_event):
match _e.tag {
    CGizmoEventTag::Drag => {
        let pos = _e.world_pos;
        let bytes = unsafe { core::slice::from_raw_parts(pos.as_ptr() as *const u8, 12) };
        (_host.node_state_set)(_host.userdata, node, b"drag_pos\0".as_ptr() as *const i8, bytes.as_ptr(), 12);
        (_host.mark_dirty)(_host.userdata, node);
    }
    _ => {}
}

### USER_CODE snippet (compute):
let mut offset = [0f32; 3];
let offset_bytes = unsafe { core::slice::from_raw_parts_mut(offset.as_mut_ptr() as *mut u8, 12) };
(_host.node_state_get)(_host.userdata, node, b"drag_pos\0".as_ptr() as *const i8, offset_bytes.as_mut_ptr(), 12);
// Apply offset to all points...
"#,
            "bezier_handle" => r#"## Bezier Handle Template
Control point + two tangent handles for curve editing. Uses Curve parameter for storage.

### NodeSpec interaction:
"interaction": {
  "gizmo_primitives": [
    { "pick_id": 1, "primitive": "sphere", "position": [0,0,0], "scale": 0.1, "color": [1,1,1,1] },
    { "pick_id": 2, "primitive": "sphere", "position": [-0.5,0,0], "scale": 0.06, "color": [0.5,0.5,1,1] },
    { "pick_id": 3, "primitive": "sphere", "position": [0.5,0,0], "scale": 0.06, "color": [0.5,0.5,1,1] }
  ],
  "hud_commands": [
    { "tag": "toggle", "id": 1, "text": "Symmetric Handles" },
    { "tag": "button", "id": 2, "text": "Reset Curve" }
  ]
}

### USER_INTERACTION_CODE pattern:
// Read curve points from node parameter, pick_id maps to point index.
// On drag: update point position, optionally mirror symmetric handle.
// Write back via node_curve_set.

match _e.tag {
    CGizmoEventTag::Drag => {
        let idx = (_e.pick_id - 1) as usize;  // 0=main, 1=handle_in, 2=handle_out
        let mut curve_data = [0u8; 256];
        let len = (_host.node_curve_get)(_host.userdata, node, b"curve\0".as_ptr() as *const i8, curve_data.as_mut_ptr(), 256);
        // Parse, modify, write back...
        (_host.node_curve_set)(_host.userdata, node, b"curve\0".as_ptr() as *const i8, curve_data.as_ptr(), len as u32);
        (_host.mark_dirty)(_host.userdata, node);
    }
    _ => {}
}
"#,
            "transform_gizmo" => r#"## Transform Gizmo Template
Translate/Rotate/Scale gizmo with axis constraints. Uses separate pick_ids per axis.

### NodeSpec interaction:
"interaction": {
  "gizmo_primitives": [
    { "pick_id": 1, "primitive": "cylinder", "position": [0.15,0,0], "scale": [0.3,0.02,0.02], "color": [1,0,0,1] },
    { "pick_id": 2, "primitive": "cylinder", "position": [0,0.15,0], "scale": [0.02,0.3,0.02], "color": [0,1,0,1] },
    { "pick_id": 3, "primitive": "cylinder", "position": [0,0,0.15], "scale": [0.02,0.02,0.3], "color": [0,0,1,1] },
    { "pick_id": 4, "primitive": "sphere", "position": [0,0,0], "scale": 0.05, "color": [1,1,0,1] }
  ],
  "hud_commands": [
    { "tag": "button", "id": 1, "text": "Translate" },
    { "tag": "button", "id": 2, "text": "Rotate" },
    { "tag": "button", "id": 3, "text": "Scale" }
  ],
  "input_keys": [
    { "key": "g", "action": "Translate mode" },
    { "key": "r", "action": "Rotate mode" },
    { "key": "s", "action": "Scale mode" }
  ]
}

### USER_INTERACTION_CODE pattern:
// pick_id 1/2/3 = X/Y/Z axis, 4 = free. Calculate delta from drag start.
// Apply transform based on current mode (stored in node_state).
"#,
            "multi_select" => r#"## Multi-Select Template
Click to select points, Shift+Click to add/remove, drag to box-select.

### NodeSpec interaction:
"interaction": {
  "gizmo_primitives": [],  // Dynamically generated per-point
  "hud_commands": [
    { "tag": "label", "id": 0, "text": "Selected: 0" },
    { "tag": "button", "id": 1, "text": "Select All" },
    { "tag": "button", "id": 2, "text": "Invert Selection" },
    { "tag": "button", "id": 3, "text": "Clear Selection" }
  ],
  "input_keys": [
    { "key": "a", "action": "Select All" },
    { "key": "i", "action": "Invert" }
  ]
}

### USER_CODE pattern (gizmo_build):
// Read points from geo, generate CGizmoCmd per point with pick_id = point_index + 1.
// Selected points get different color (e.g. orange vs white).

### USER_INTERACTION_CODE pattern:
// On click: toggle selection for pick_id - 1.
// Store selection as bitfield in node_state.
"#,
            _ => return Err(ToolError(format!("Unknown pattern '{}'. Use: drag_point, bezier_handle, transform_gizmo, multi_select", pattern))),
        };
        Ok(ToolOutput::new(
            template.to_string(),
            vec![ToolLog {
                message: format!("Interaction template: {}", pattern),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

// ==========================================
// Tool 9: Generate Replay Script (for regression testing)
// ==========================================

#[derive(Deserialize)]
struct GenerateReplayArgs {
    plugin_name: String,
    #[serde(default)]
    include_code: bool,
}

pub struct GenerateReplayTool;

impl Tool for GenerateReplayTool {
    fn name(&self) -> &str {
        "generate_replay_script"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Generate a replay script from a plugin's nodespec.json + lib.rs hash for regression testing.".to_string(),
            parameters: json!({"type":"object","properties":{"plugin_name":{"type":"string","description":"Plugin name"},"include_code":{"type":"boolean","description":"Include lib.rs in script (default: false)"}},"required":["plugin_name"]})
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let a: GenerateReplayArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid args: {e}")))?;
        let base = Path::new("plugins/extra_node").join(&a.plugin_name);
        if !base.exists() {
            let mut available: Vec<String> = Vec::new();
            if let Ok(entries) = std::fs::read_dir("plugins/extra_node") {
                for e in entries.filter_map(|e| e.ok()) {
                    if e.path().is_dir() { available.push(e.file_name().to_string_lossy().to_string()); }
                }
            }
            available.sort();
            let hint = if available.is_empty() { "No plugins found.".to_string() } else { format!("Available: {}", available.join(", ")) };
            return Err(ToolError(format!("Plugin '{}' not found. {}", a.plugin_name, hint)));
        }

        let nodespec_path = base.join("nodespec.json");
        let lib_path = base.join("src/lib.rs");

        let nodespec = fs::read_to_string(&nodespec_path).unwrap_or_else(|_| "{}".to_string());
        let lib_code = fs::read_to_string(&lib_path).unwrap_or_else(|_| "".to_string());

        fn fnv1a64(bytes: &[u8]) -> u64 {
            let mut h: u64 = 0xcbf29ce484222325;
            for &b in bytes {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            h
        }
        let lib_hash = fnv1a64(lib_code.as_bytes());

        let mut script = String::new();
        script.push_str(&format!("# Replay Script for '{}'\n", a.plugin_name));
        script.push_str(&format!("# Generated: {}\n", chrono_lite()));
        script.push_str(&format!("# lib.rs hash (FNV-1a64): {:016x}\n\n", lib_hash));
        script.push_str("## NodeSpec:\n```json\n");
        script.push_str(&nodespec);
        script.push_str("\n```\n\n");

        if a.include_code {
            script.push_str("## lib.rs (USER_CODE region):\n```rust\n");
            if let Some(i) = lib_code.find("// === USER_CODE_BEGIN ===") {
                if let Some(j) = lib_code.find("// === USER_CODE_END ===") {
                    script.push_str(&lib_code[i..j + "// === USER_CODE_END ===".len()]);
                }
            }
            script.push_str("\n```\n\n");
        }

        script.push_str("## Replay Steps:\n");
        script.push_str("1. Call apply_rust_nodespec with the NodeSpec above.\n");
        script.push_str("2. Verify build_status=ok and smoke_status=passed.\n");
        script.push_str(&format!(
            "3. Verify lib.rs hash matches {:016x}.\n",
            lib_hash
        ));

        // Save replay script
        let replay_path = base.join("replay.md");
        let _ = fs::write(&replay_path, &script);

        Ok(ToolOutput::new(
            script,
            vec![ToolLog {
                message: format!("Generated replay script: {}", replay_path.display()),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

fn chrono_lite() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", secs)
}

// ==========================================
// Tool 10: Compare Fingerprints (A/B testing)
// ==========================================

#[derive(Deserialize)]
struct ComparePluginsArgs {
    plugin_a: String,
    plugin_b: String,
}

pub struct ComparePluginsTool;

impl Tool for ComparePluginsTool {
    fn name(&self) -> &str {
        "compare_plugins"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Compare two plugin versions (code diff + nodespec diff). For A/B testing after refactoring.".to_string(),
            parameters: json!({"type":"object","properties":{"plugin_a":{"type":"string","description":"First plugin name or path"},"plugin_b":{"type":"string","description":"Second plugin name or path"}},"required":["plugin_a","plugin_b"]})
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let a: ComparePluginsArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid args: {e}")))?;

        let path_a = Path::new("plugins/extra_node")
            .join(&a.plugin_a)
            .join("src/lib.rs");
        let path_b = Path::new("plugins/extra_node")
            .join(&a.plugin_b)
            .join("src/lib.rs");

        let code_a = fs::read_to_string(&path_a).unwrap_or_default();
        let code_b = fs::read_to_string(&path_b).unwrap_or_default();

        fn fnv1a64(bytes: &[u8]) -> u64 {
            let mut h: u64 = 0xcbf29ce484222325;
            for &b in bytes {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            h
        }
        let hash_a = fnv1a64(code_a.as_bytes());
        let hash_b = fnv1a64(code_b.as_bytes());

        let extract_user = |s: &str| -> String {
            let (a, b) = ("// === USER_CODE_BEGIN ===", "// === USER_CODE_END ===");
            if let (Some(i), Some(j)) = (s.find(a), s.find(b)) {
                if j > i {
                    return s[i + a.len()..j].to_string();
                }
            }
            String::new()
        };

        let user_a = extract_user(&code_a);
        let user_b = extract_user(&code_b);

        let mut out = String::new();
        out.push_str(&format!(
            "## Plugin Comparison: {} vs {}\n\n",
            a.plugin_a, a.plugin_b
        ));
        out.push_str(&format!(
            "| Metric | {} | {} |\n|--------|-------|-------|\n",
            a.plugin_a, a.plugin_b
        ));
        out.push_str(&format!(
            "| lib.rs hash | {:016x} | {:016x} |\n",
            hash_a, hash_b
        ));
        out.push_str(&format!(
            "| lib.rs lines | {} | {} |\n",
            code_a.lines().count(),
            code_b.lines().count()
        ));
        out.push_str(&format!(
            "| USER_CODE lines | {} | {} |\n",
            user_a.lines().count(),
            user_b.lines().count()
        ));
        out.push_str(&format!(
            "| Hash match | {} |\n\n",
            if hash_a == hash_b {
                "✅ YES"
        } else {
                "❌ NO"
            }
        ));

        if hash_a != hash_b {
            out.push_str("### USER_CODE Diff:\n");
            let lines_a: Vec<&str> = user_a.lines().collect();
            let lines_b: Vec<&str> = user_b.lines().collect();
            let max_lines = lines_a.len().max(lines_b.len()).min(50);
            for i in 0..max_lines {
                let la = lines_a.get(i).unwrap_or(&"");
                let lb = lines_b.get(i).unwrap_or(&"");
                if la != lb {
                    out.push_str(&format!("L{}: -{}\nL{}: +{}\n", i + 1, la, i + 1, lb));
                }
            }
        }

        Ok(ToolOutput::new(
            out,
            vec![ToolLog {
                message: format!("Compared {} vs {}", a.plugin_a, a.plugin_b),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

// ==========================================
// Tool: Terminal (Zed-style shell execution)
// ==========================================

#[derive(Deserialize)]
struct TerminalArgs {
    command: String,
    #[serde(default)]
    cd: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

pub struct TerminalTool;

fn resolve_cd(cd: &str) -> Result<std::path::PathBuf, ToolError> {
    let root = std::env::current_dir().map_err(|e| ToolError(format!("cwd error: {}", e)))?;
    let p = if cd.trim().is_empty() { root.clone() } else {
        let cand = std::path::PathBuf::from(cd);
        if cand.is_absolute() { cand } else { root.join(cand) }
    };
    let p = p.canonicalize().map_err(|e| ToolError(format!("cd error: {}", e)))?;
    if !p.starts_with(&root) { return Err(ToolError("cd must stay within project root".into())); }
    if !p.exists() { return Err(ToolError(format!("Directory not found: {}", p.display()))); }
    Ok(p)
}

fn run_shell_with_ctx(ctx: Option<&ToolContext>, cwd: &std::path::Path, command: &str, timeout_ms: u64) -> Result<(i32, String, String), ToolError> {
    use std::{io::Read as _, process::{Command, Stdio}, time::{Duration, Instant}};
    let mut child = Command::new(if cfg!(windows) { "cmd" } else { "sh" })
        .args(if cfg!(windows) { vec!["/C", command] } else { vec!["-c", command] })
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ToolError(format!("Failed to execute: {}", e)))?;
    let mut stdout = child.stdout.take().ok_or_else(|| ToolError("stdout unavailable".into()))?;
    let mut stderr = child.stderr.take().ok_or_else(|| ToolError("stderr unavailable".into()))?;
    let out_t = std::thread::spawn(move || { let mut b = Vec::new(); let _ = stdout.read_to_end(&mut b); b });
    let err_t = std::thread::spawn(move || { let mut b = Vec::new(); let _ = stderr.read_to_end(&mut b); b });
    let t0 = Instant::now();
    let code;
    loop {
        if ctx.is_some_and(|c| c.is_cancelled()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ToolError("Cancelled".into()));
        }
        if t0.elapsed() >= Duration::from_millis(timeout_ms) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ToolError(format!("Timeout after {}ms", timeout_ms)));
        }
        match child.try_wait() {
            Ok(Some(status)) => { code = status.code().unwrap_or(-1); break; }
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(e) => return Err(ToolError(format!("Process error: {}", e))),
        }
    }
    let out_b = out_t.join().unwrap_or_default();
    let err_b = err_t.join().unwrap_or_default();
    Ok((code, String::from_utf8_lossy(&out_b).to_string(), String::from_utf8_lossy(&err_b).to_string()))
}

impl Tool for TerminalTool {
    fn name(&self) -> &str {
        "terminal"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Execute a shell command. Use for git, cargo check, etc. Each call is a new shell. Never use for long-running servers.".to_string(),
            parameters: json!({"type":"object","properties":{"command":{"type":"string","description":"The shell command to execute"},"cd":{"type":"string","description":"Working directory (default: project root)"},"timeout_ms":{"type":"integer","description":"Timeout in ms (default: 30000)"}},"required":["command"]}),
        }
    }

    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        self.execute_with_context(args, &ToolContext::default())
    }

    fn execute_with_context(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let a: TerminalArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid args: {}", e)))?;
        let cwd = resolve_cd(&a.cd)?;
        let timeout_ms = a.timeout_ms.unwrap_or(30_000);
        ctx.report_progress(ToolLog { message: format!("$ {} (timeout {}ms)", a.command, timeout_ms), level: ToolLogLevel::Info });
        let (code, stdout, stderr) = run_shell_with_ctx(Some(ctx), &cwd, &a.command, timeout_ms)?;
        let mut combined = format!("[exit {}]\n{}{}", code, stdout, if stderr.is_empty() { String::new() } else { format!("\n[stderr]\n{}", stderr) });
        const UI_LIMIT: usize = 64 * 1024;
        if combined.len() > UI_LIMIT { combined.truncate(UI_LIMIT); combined.push_str("\n... (truncated)\n"); }
        let level = if code == 0 { ToolLogLevel::Success } else { ToolLogLevel::Error };
        Ok(ToolOutput::new(combined, vec![ToolLog { message: format!("$ {} (exit {})", a.command, code), level }]))
    }

    fn is_long_running(&self) -> bool {
        true
    }
}

// ==========================================
// Tool: Diagnostics (Zed-style linter check)
// ==========================================

#[derive(Deserialize)]
struct DiagnosticsArgs {
    #[serde(default)]
    path: Option<String>,
}

pub struct DiagnosticsTool;

impl Tool for DiagnosticsTool {
    fn name(&self) -> &str {
        "diagnostics"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Get compilation errors/warnings. Without path: runs `cargo check` on plugin crate. With path: checks specific file context.".to_string(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"Optional file path to focus diagnostics on"}}}),
        }
    }

    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        self.execute_with_context(args, &ToolContext::default())
    }

    fn execute_with_context(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let a: DiagnosticsArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid args: {}", e)))?;
        let cwd = resolve_cd("")?;
        let timeout_ms = 60_000u64;
        ctx.report_progress(ToolLog { message: format!("cargo check --message-format short (timeout {}ms)", timeout_ms), level: ToolLogLevel::Info });
        let (code, stdout, stderr) = run_shell_with_ctx(Some(ctx), &cwd, "cargo check --message-format short", timeout_ms)?;
        let mut diags = String::new();
        for line in stderr.lines().chain(stdout.lines()) {
            if !(line.contains("error[") || line.contains("warning:") || line.contains("error:")) { continue; }
            if let Some(p) = &a.path { if !line.contains(p) { continue; } }
            diags.push_str(line); diags.push('\n');
        }
        if diags.is_empty() { diags = if code == 0 { "No errors or warnings.".into() } else { format!("Build failed (exit {}), but no parseable diagnostics.", code) }; }
        let level = if code == 0 { ToolLogLevel::Success } else { ToolLogLevel::Warning };
        Ok(ToolOutput::new(diags, vec![ToolLog { message: format!("cargo check (exit {})", code), level }]))
    }

    fn is_long_running(&self) -> bool {
        true
    }
}
