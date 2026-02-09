use super::graph_ops::{EditNodeGraphTool, GraphEditOp, NodePortRef};
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::nodes::structs::NodeGraph;
use crate::tabs_registry::ai_workspace::tools::{
    Tool, ToolDefinition, ToolError, ToolLog, ToolLogLevel, ToolOutput,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// Allows AI to use Rust-like script syntax to manipulate the node graph.
/// This is not a real Rust compiler, but an interpreter for a specific syntax subset.
///
/// Supported syntax paradigms (pseudo-code):
/// ```rust
/// fn build() {
///     // Create node and set alias
///     let cube = create("Cube");
///     // Chain parameter setting
///     cube.set("size", 2.0).set("center_y", 1.0);
///     
///     // Create another node
///     let noise = create("Noise").set("type", "simplex");
///     
///     // Connect
///     // Default connection: cube.output -> noise.input
///     cube.connect(noise);
///     // Explicit connection: cube.out_port -> noise.in_port
///     cube.connect_port(noise, "geometry", "input_geometry");
///     
///     // Set display/Bypass
///     noise.set_display();
/// }
/// ```
pub struct RunGraphScriptTool {
    registry: Arc<NodeRegistry>,
}

impl RunGraphScriptTool {
    pub fn new(registry: Arc<NodeRegistry>) -> Self {
        Self { registry }
    }

    fn split_args_top_level(s: &str) -> Vec<String> {
        let (mut out, mut cur) = (Vec::new(), String::new());
        let (mut dp, mut db, mut in_str, mut esc) = (0i32, 0i32, false, false);
        for ch in s.chars() {
            if in_str {
                cur.push(ch);
                if esc {
                    esc = false;
                } else if ch == '\\' {
                    esc = true;
                } else if ch == '"' {
                    in_str = false;
                }
                continue;
            }
            match ch {
                '"' => {
                    in_str = true;
                    cur.push(ch);
                }
                '(' => {
                    dp += 1;
                    cur.push(ch);
                }
                ')' => {
                    dp = (dp - 1).max(0);
                    cur.push(ch);
                }
                '[' => {
                    db += 1;
                    cur.push(ch);
                }
                ']' => {
                    db = (db - 1).max(0);
                    cur.push(ch);
                }
                ',' if dp == 0 && db == 0 => {
                    let t = cur.trim();
                    if !t.is_empty() {
                        out.push(t.to_string());
                    }
                    cur.clear();
                }
                _ => cur.push(ch),
            }
        }
        let t = cur.trim();
        if !t.is_empty() {
            out.push(t.to_string());
        }
        out
    }

    /// Parse script and convert each line to GraphEditOp
    fn parse_script(&self, script: &str) -> Result<Vec<GraphEditOp>, String> {
        let mut ops = Vec::new();
        let mut var_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new(); // var_name -> alias

        // Simple line parser
        // Note: This is fragile and relies on AI strictly following the format.
        // But for models like Gemini, they can write very standard code if prompted well.

        for line in script.lines() {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with("//")
                || line.starts_with("fn ")
                || line.starts_with("}")
            {
                continue;
            }

            // 1. Handle variable binding: `let var = ...`
            //    - let cube = create("Create Cube");      // Create node and bind alias
            //    - let base = node("Existing Node");      // Bind existing node (by name/UUID)
            //    - let disp = display();                   // Bind current display node
            if line.starts_with("let ") {
                if let Some((var_part, rest)) = line.split_once('=') {
                    let var_name = var_part.trim().trim_start_matches("let ").trim();
                    let rest = rest.trim();

                    // Case A: create("Type")
                    if let Some(type_start) = rest.find("create(\"") {
                        let after_type = &rest[type_start + 8..]; // Skip create("
                        if let Some(type_end) = after_type.find("\")") {
                            let node_type = &after_type[..type_end];

                            // Use var_name as alias, parsed later by EditNodeGraphTool
                            let alias = var_name.to_string();
                            var_map.insert(var_name.to_string(), alias.clone());

                            ops.push(GraphEditOp::CreateNode {
                                node_type: node_type.to_string(),
                                alias: Some(alias.clone()),
                                node_name: None,
                                position: None,
                            });

                            // Handle chained calls after create(...), e.g., .set(...).at(...)
                            let chain_start = type_start + 8 + node_type.len() + 2; // 8: create("  +2: ")
                            if chain_start < rest.len() {
                                self.parse_chain_calls(
                                    var_name,
                                    &rest[chain_start..],
                                    &mut ops,
                                    &var_map,
                                )?;
                            }
                        }
                    }
                    // Case B: node("ExistingName") -> Bind existing node (name/UUID/alias parsed by EditNodeGraphTool)
                    else if let Some(name_start) = rest.find("node(\"") {
                        let after_name = &rest[name_start + 6..]; // Skip node("
                        if let Some(name_end) = after_name.find("\")") {
                            let target_name = &after_name[..name_end];
                            var_map.insert(var_name.to_string(), target_name.to_string());

                            let chain_start = name_start + 6 + target_name.len() + 2; // node(" + name + ")
                            if chain_start < rest.len() {
                                self.parse_chain_calls(
                                    var_name,
                                    &rest[chain_start..],
                                    &mut ops,
                                    &var_map,
                                )?;
                            }
                        }
                    }
                    // Case C: display() -> Bind current display node (special keyword "display")
                    else if let Some(disp_start) = rest.find("display()") {
                        var_map.insert(var_name.to_string(), "display".to_string());

                        let chain_start = disp_start + 9; // "display()" length is 9
                        if chain_start < rest.len() {
                            self.parse_chain_calls(
                                var_name,
                                &rest[chain_start..],
                                &mut ops,
                                &var_map,
                            )?;
                        }
                    }
                }
                continue;
            }

            // 2. Handle standalone calls: `var.connect(other)` or `var.set(...)`
            if let Some(dot_idx) = line.find('.') {
                let var_name = line[..dot_idx].trim();
                if var_map.contains_key(var_name) {
                    self.parse_chain_calls(var_name, &line[dot_idx..], &mut ops, &var_map)?;
                }
            }
        }

        Ok(ops)
    }

    fn parse_chain_calls(
        &self,
        var_name: &str,
        chain_str: &str,
        ops: &mut Vec<GraphEditOp>,
        var_map: &std::collections::HashMap<String, String>,
    ) -> Result<(), String> {
        let mut current = chain_str;
        let alias = var_map
            .get(var_name)
            .ok_or(format!("Unknown variable: {}", var_name))?;

        // Simple loop finding .xxx(...) pattern
        while let Some(dot_idx) = current.find('.') {
            current = &current[dot_idx + 1..]; // skip dot

            if let Some(paren_open) = current.find('(') {
                let method = &current[..paren_open];
                if let Some(paren_close) = current.find(')') {
                    let args_str = &current[paren_open + 1..paren_close];

                    match method {
                        "set" => {
                            // .set("param", val)
                            let args = Self::split_args_top_level(args_str);
                            if args.len() != 2 {
                                return Err(format!("Invalid set() args: expected 2, got {}: {}", args.len(), args_str));
                            }
                            let param = args[0].trim();
                            if !(param.starts_with('"') && param.ends_with('"')) {
                                return Err(format!("Invalid set() param name (must be quoted string): {}", param));
                            }
                            let param_name = param.trim_matches('"');
                            let val_json = self.parse_value(args[1].trim());
                            ops.push(GraphEditOp::SetParam { target: alias.clone(), param: param_name.to_string(), value: val_json });
                        }
                        "connect" => {
                            // .connect(other_var)  -> alias.output -> other.input (default)
                            // .connect(other_var, "from", "to")
                            let args = Self::split_args_top_level(args_str);
                            if args.is_empty() {
                                return Err("Invalid connect() args: missing target".to_string());
                            }
                            let other_var = args[0].trim();
                            let Some(other_alias) = var_map.get(other_var) else {
                                return Err(format!("Unknown variable in connect(): {}", other_var));
                            };
                            let (from_port, to_port) = if args.len() >= 3 {
                                (Some(args[1].trim().trim_matches('"').to_string()), Some(args[2].trim().trim_matches('"').to_string()))
                            } else {
                                (None, None)
                            };
                            ops.push(GraphEditOp::Connect {
                                from: NodePortRef { target: alias.clone(), port: from_port },
                                to: NodePortRef { target: other_alias.clone(), port: to_port },
                            });
                        }
                        "set_display" => {
                            ops.push(GraphEditOp::SetDisplay {
                                target: alias.clone(),
                            });
                        }
                        "bypass" => {
                            // .bypass(true/false) or default true
                            let val = if args_str.is_empty() {
                                true
                            } else {
                                args_str == "true"
                            };
                            ops.push(GraphEditOp::SetFlag {
                                target: alias.clone(),
                                flag: "bypass".to_string(),
                                value: val,
                            });
                        }
                        "at" => {
                            // .at(x, y) -> Explicitly set node position in editor
                            let args: Vec<&str> = args_str.split(',').map(|s| s.trim()).collect();
                            if args.len() == 2 {
                                if let (Ok(x), Ok(y)) =
                                    (args[0].parse::<f32>(), args[1].parse::<f32>())
                                {
                                    ops.push(GraphEditOp::SetPosition {
                                        target: alias.clone(),
                                        position: [x, y],
                                    });
                                }
                            }
                        }
                        _ => {}
                    }

                    current = &current[paren_close + 1..];
                } else {
                    break; // No closing paren
                }
            } else {
                break; // No opening paren
            }
        }
        Ok(())
    }

    fn parse_value(&self, s: &str) -> Value {
        // Try parsing as number
        if let Ok(f) = s.parse::<f32>() {
            return serde_json::json!(f);
        }
        if let Ok(i) = s.parse::<i32>() {
            return serde_json::json!(i);
        }
        if let Ok(b) = s.parse::<bool>() {
            return serde_json::json!(b);
        }
        // Try parsing array [1.0, 2.0]
        if s.starts_with('[') && s.ends_with(']') {
            let content = &s[1..s.len() - 1];
            let nums: Vec<Value> = Self::split_args_top_level(content).iter().map(|p| self.parse_value(p.trim())).collect();
            return serde_json::Value::Array(nums);
        }
        // Try parsing tuple (1, 2, 3) -> [1,2,3]
        if s.starts_with('(') && s.ends_with(')') {
            let content = &s[1..s.len() - 1];
            let nums: Vec<Value> = Self::split_args_top_level(content).iter().map(|p| self.parse_value(p.trim())).collect();
            return serde_json::Value::Array(nums);
        }
        // String
        serde_json::json!(s.trim_matches('"'))
    }
}

impl Tool for RunGraphScriptTool {
    fn name(&self) -> &str {
        "run_graph_script"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description:
                "Executes a batch of graph operations using a concise, Rust-like script syntax. \
                          Syntax examples: \
                          `let cube = create(\"Create Cube\").at(0.0, 100.0);` \
                          `let base = node(\"Existing Node\");` \
                          `let disp = display();` \
                          `cube.set(\"size\", 2.0).connect(base);` \
                          `disp.set_display();`"
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "script": {
                        "type": "string",
                        "description": "The Rust-like script content."
                    }
                },
                "required": ["script"]
            }),
        }
    }

    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let script = args
            .get("script")
            .and_then(|s| s.as_str())
            .ok_or_else(|| ToolError("Missing 'script' argument".to_string()))?;

        let ops = self
            .parse_script(script)
            .map_err(|e| ToolError(format!("Script Parse Error: {}", e)))?;

        if ops.is_empty() {
            let meaningful = script.lines().any(|l| {
                let t = l.trim();
                !(t.is_empty() || t.starts_with("//") || t.starts_with("fn ") || t == "}")
            });
            if meaningful {
                return Err(ToolError("Script produced no operations. Common causes: unsupported syntax or invalid set/connect arguments. Tips: use set(\"divisions\", [1,1,1]) or set(\"divisions\", (1,1,1)).".to_string()));
            }
            return Ok(ToolOutput::new(
                "Script executed but produced no operations (empty or comments only?).",
                vec![ToolLog { message: "RunGraphScriptTool: no operations produced".to_string(), level: ToolLogLevel::Info }],
            ));
        }

        let op_count = ops.len();
        let edit_tool = EditNodeGraphTool::new(self.registry.clone());
        let edit_args = serde_json::json!({ "ops": ops });

        match edit_tool.execute(edit_args) {
            Ok(output) => Ok(ToolOutput::new(
                format!(
                    "Successfully executed Rust Graph Script ({} operations applied).",
                    op_count
                ),
                output.ui_logs,
            )),
            Err(e) => Err(e),
        }
    }
}
