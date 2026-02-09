use super::definitions::{Tool, ToolDefinition, ToolError, ToolLog, ToolLogLevel, ToolOutput};
use super::diff::compute_file_diff;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

// --- Create Node Folder ---

#[derive(Deserialize)]
struct CreateFolderArgs {
    node_name: String,
}

pub struct CreateNodeFolderTool;

impl Tool for CreateNodeFolderTool {
    fn name(&self) -> &str {
        "create_node_folder"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Create a new node folder under `plugins/<NodeName>/` with template node.toml and logic.rhai.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_name": {
                        "type": "string",
                        "description": "PascalCase node name (e.g. 'TestNode')"
                    }
                },
                "required": ["node_name"]
            }),
        }
    }

    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: CreateFolderArgs = serde_json::from_value(args)
            .map_err(|e| ToolError(format!("Invalid arguments: {}", e)))?;

        let node_name = args.node_name;
        let mut logs = Vec::new();

        // Safety Check
        if node_name.contains("..") || node_name.contains("/") || node_name.contains("\\") {
            return Err(ToolError("Invalid node name".to_string()));
        }

        let base_path = Path::new("plugins").join(&node_name);

        logs.push(ToolLog {
            message: format!("Target directory: plugins/{}", node_name),
            level: ToolLogLevel::Info,
        });

        if !base_path.exists() {
            fs::create_dir_all(&base_path)
                .map_err(|e| ToolError(format!("Failed to create dir: {}", e)))?;
            logs.push(ToolLog {
                message: "Created directory".to_string(),
                level: ToolLogLevel::Success,
            });
        } else {
            logs.push(ToolLog {
                message: "Directory already exists".to_string(),
                level: ToolLogLevel::Info,
            });
        }

        // Write Templates
        let toml_code = format!(
            "[meta]\nname = \"{}\"\ncategory = \"Experimental\"\nversion = \"0.1.0\"\n\n[parameters]\n",
            node_name
        );
        fs::write(base_path.join("node.toml"), &toml_code)
            .map_err(|e| ToolError(format!("Failed to write node.toml: {}", e)))?;
        logs.push(ToolLog {
            message: "Wrote node.toml template".to_string(),
            level: ToolLogLevel::Success,
        });

        let logic_code = "// TODO: Implement entry(input)\nfn entry(input) { return input; }\n";
        fs::write(base_path.join("logic.rhai"), logic_code)
            .map_err(|e| ToolError(format!("Failed to write logic.rhai: {}", e)))?;
        logs.push(ToolLog {
            message: "Wrote logic.rhai template".to_string(),
            level: ToolLogLevel::Success,
        });

        Ok(ToolOutput::new(
            format!("Node '{}' scaffold created successfully.", node_name),
            logs,
        ))
    }
}

// --- Patch Node File (Hunk-based) ---

#[derive(Deserialize)]
struct PatchHunk {
    original: String,
    replacement: String,
}

#[derive(Deserialize)]
struct PatchFileArgs {
    node_name: String,
    file_name: String,
    hunks: Vec<PatchHunk>,
}

pub struct PatchNodeFileTool;

impl Tool for PatchNodeFileTool {
    fn name(&self) -> &str {
        "patch_node_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Apply precise text patches to a file. Use this for fixes instead of rewriting the whole file.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_name": { "type": "string" },
                    "file_name": { "type": "string" },
                    "hunks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "original": { "type": "string", "description": "Exact text to find and replace (must be unique)" },
                                "replacement": { "type": "string", "description": "New text" }
                            },
                            "required": ["original", "replacement"]
                        }
                    }
                },
                "required": ["node_name", "file_name", "hunks"]
            }),
        }
    }

    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: PatchFileArgs = serde_json::from_value(args)
            .map_err(|e| ToolError(format!("Invalid arguments: {}", e)))?;

        let base_path = Path::new("plugins").join(&args.node_name);
        let file_path = base_path.join(&args.file_name);

        if !file_path.exists() {
            return Err(ToolError(format!(
                "File not found: plugins/{}/{}",
                args.node_name, args.file_name
            )));
        }

        let old = fs::read_to_string(&file_path)
            .map_err(|e| ToolError(format!("Failed to read file: {}", e)))?;
        let mut content = old.clone();

        let mut logs = Vec::new();
        let mut applied_count = 0;

        for (i, hunk) in args.hunks.iter().enumerate() {
            // Simple string find-and-replace
            // Safety: Ensure uniqueness
            let matches: Vec<_> = content.match_indices(&hunk.original).collect();

            if matches.is_empty() {
                logs.push(ToolLog {
                    message: format!("Hunk {} failed: 'original' text not found", i),
                    level: ToolLogLevel::Error,
                });
                return Err(ToolError(format!(
                    "Hunk {} failed: original text not found. Please read file and try again.",
                    i
                )));
            }
            if matches.len() > 1 {
                logs.push(ToolLog {
                    message: format!(
                        "Hunk {} failed: 'original' text matches {} times (must be unique)",
                        i,
                        matches.len()
                    ),
                    level: ToolLogLevel::Error,
                });
                return Err(ToolError(format!(
                    "Hunk {} failed: original text is not unique.",
                    i
                )));
            }

            content = content.replace(&hunk.original, &hunk.replacement);
            applied_count += 1;
        }

        fs::write(&file_path, &content)
            .map_err(|e| ToolError(format!("Failed to write patched file: {}", e)))?;

        logs.push(ToolLog {
            message: format!("Applied {} patches to {}", applied_count, args.file_name),
            level: ToolLogLevel::Success,
        });

        let mut out = ToolOutput::new(
            format!(
                "Successfully patched {}/{} with {} hunks.",
                args.node_name, args.file_name, applied_count
            ),
            logs,
        );
        if let Some(d) = compute_file_diff(file_path.to_string_lossy().to_string(), &old, &content) {
            out.ui_diffs.push(d);
        }
        Ok(out)
    }
}

// --- Write Node File ---

#[derive(Deserialize)]
struct WriteFileArgs {
    node_name: String,
    file_name: String,
    content: String,
}

pub struct WriteNodeFileTool;

impl Tool for WriteNodeFileTool {
    fn name(&self) -> &str {
        "write_node_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Write content to a specific file in a node folder.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_name": { "type": "string" },
                    "file_name": { "type": "string", "enum": ["node.toml", "logic.rhai", "ui.rhai"] },
                    "content": { "type": "string" }
                },
                "required": ["node_name", "file_name", "content"]
            }),
        }
    }

    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: WriteFileArgs = serde_json::from_value(args)
            .map_err(|e| ToolError(format!("Invalid arguments: {}", e)))?;

        let base_path = Path::new("plugins").join(&args.node_name);
        if !base_path.exists() {
            return Err(ToolError(format!(
                "Node folder plugins/{} does not exist",
                args.node_name
            )));
        }

        let file_path = base_path.join(&args.file_name);
        let old = fs::read_to_string(&file_path).unwrap_or_default();
        fs::write(&file_path, &args.content)
            .map_err(|e| ToolError(format!("Failed to write file: {}", e)))?;

        let len = args.content.len();
        let mut logs = vec![ToolLog {
            level: ToolLogLevel::Success,
            message: format!("Wrote {} ({} bytes)", args.file_name, len),
        }];

        // Hint for the AI to check compilation
        if args.file_name.ends_with(".rhai") {
            logs.push(ToolLog {
                level: ToolLogLevel::Info,
                message: "IMPORTANT: You MUST now call `check_node_compile` to verify this change."
                    .to_string(),
            });
        }

        let mut out = ToolOutput::new(
            format!("Successfully wrote {} bytes to {}.", len, args.file_name),
            logs,
        );
        if let Some(d) = compute_file_diff(file_path.to_string_lossy().to_string(), &old, &args.content) {
            out.ui_diffs.push(d);
        }
        Ok(out)
    }
}

// ... (rest of the code remains the same)
