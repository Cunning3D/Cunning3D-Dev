//! Tool system with async execution and cancellation support.
pub mod definitions;
pub mod diff;
pub mod file_ops;
pub mod graph_ops;
pub mod graph_script;
pub mod knowledge_ops;
pub mod rhai_ops;
pub mod rust_nodespec_ops;
pub mod rust_plugin_ops;
pub mod workspace_ops;

use crossbeam_channel::{unbounded, Receiver, Sender};
use serde_json::Value;
use std::sync::Arc;
use crate::cunning_core::registries::node_registry::NodeRegistry;

pub use definitions::{
    canonical_json, tool_call_signature, CancellationToken, Tool, ToolContext, ToolDefinition,
    ToolError, ToolLog, ToolLogLevel, ToolOutput,
};
pub use diff::{compute_file_diff, DiffHunk, DiffLine, DiffLineKind, FileDiff};

/// Tool execution result for async dispatch
#[derive(Debug)]
pub enum ToolResult {
    Success(ToolOutput),
    Error(ToolError),
    Cancelled,
    Progress(ToolLog),
}

/// Async tool execution request
pub struct ToolRequest {
    pub id: u64,
    pub tool_name: String,
    pub args: Value,
    pub cancel_token: CancellationToken,
}

/// Registry to hold all available tools
pub struct ToolRegistry {
    tools: std::collections::HashMap<String, Arc<dyn Tool + Send + Sync>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: std::collections::HashMap::new(),
        }
    }

    pub fn register<T: Tool + Send + Sync + 'static>(&mut self, tool: T) {
        self.tools.insert(tool.name().to_string(), Arc::new(tool));
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool + Send + Sync>> {
        self.tools.get(name).cloned()
    }

    pub fn list_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    /// Execute tool synchronously (for compatibility)
    pub fn execute_sync(&self, name: &str, args: Value) -> Result<ToolOutput, ToolError> {
        let tool = self
            .get(name)
            .ok_or_else(|| ToolError(format!("Tool '{}' not found", name)))?;
        tool.execute(args)
    }

    /// Execute tool with context (cancellation support)
    pub fn execute_with_context(
        &self,
        name: &str,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let tool = self
            .get(name)
            .ok_or_else(|| ToolError(format!("Tool '{}' not found", name)))?;
        tool.execute_with_context(args, ctx)
    }

    /// Check if tool is long-running (should be executed off UI thread)
    pub fn is_long_running(&self, name: &str) -> bool {
        self.get(name).map(|t| t.is_long_running()).unwrap_or(false)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolProfile {
    Full,
    NodeAssistant,
}

fn register_node_assistant_tools(registry: &mut ToolRegistry, node_registry: Arc<NodeRegistry>) {
    registry.register(graph_ops::CreateNodeTool::new(node_registry.clone()));
    registry.register(graph_ops::ConnectNodeTool::new());
    registry.register(graph_ops::SetParameterTool::new());
    registry.register(graph_ops::GetGraphStateTool::new());
    registry.register(graph_ops::GetNodeInfoTool::new(node_registry.clone()));
    registry.register(graph_ops::GetNodeLibraryTool::new(node_registry));
    registry.register(graph_ops::CreateStickyNoteTool::new());
    registry.register(graph_ops::CreateNetworkBoxTool::new());
    registry.register(knowledge_ops::SearchKnowledgeTool);
    registry.register(knowledge_ops::ReadKnowledgeTool);
}

fn register_full_tools(registry: &mut ToolRegistry, node_registry: Arc<NodeRegistry>) {
    registry.register(workspace_ops::ExploreWorkspaceTool);
    registry.register(workspace_ops::SearchWorkspaceTool);
    registry.register(workspace_ops::ReadFileTool);
    registry.register(workspace_ops::PatchFileTool);
    registry.register(workspace_ops::WriteFileTool);
    registry.register(workspace_ops::WebSearchTool);
    registry.register(workspace_ops::GetNodeSpecTemplateTool);
    registry.register(workspace_ops::GetAbiReferenceTool);
    registry.register(workspace_ops::GetInteractionGuideTool);
    registry.register(workspace_ops::GetInteractionTemplateTool);
    registry.register(workspace_ops::GenerateReplayTool);
    registry.register(workspace_ops::ComparePluginsTool);
    registry.register(workspace_ops::TerminalTool);
    registry.register(workspace_ops::DiagnosticsTool);

    registry.register(rust_nodespec_ops::ExtractUserCodeTool);
    registry.register(rust_nodespec_ops::ApplyRustNodeSpecTool::new(node_registry.clone()));

    registry.register(rust_plugin_ops::CreateRustPluginTool);
    registry.register(rust_plugin_ops::CompileRustPluginTool);
    registry.register(rust_plugin_ops::LoadRustPluginsTool::new(node_registry.clone()));
    registry.register(rust_plugin_ops::RevertRustPluginBuildTool::new(node_registry.clone()));

    registry.register(graph_ops::CreateNodeTool::new(node_registry.clone()));
    registry.register(graph_ops::DeleteNodeTool::new());
    registry.register(graph_ops::ConnectNodeTool::new());
    registry.register(graph_ops::SetNodeFlagTool::new());
    registry.register(graph_ops::SetParameterTool::new());
    registry.register(graph_ops::GetGraphStateTool::new());
    registry.register(graph_ops::EditNodeGraphTool::new(node_registry.clone()));
    registry.register(graph_ops::GetGeometryInsightTool::new());
    registry.register(graph_ops::GetNodeInfoTool::new(node_registry.clone()));
    registry.register(graph_ops::GetNodeLibraryTool::new(node_registry.clone()));
    registry.register(graph_ops::ExportNodeSpecTool::new(node_registry.clone()));
    registry.register(graph_ops::CompareGeometryTool::new());
    registry.register(graph_ops::CreateStickyNoteTool::new());
    registry.register(graph_ops::CreateNetworkBoxTool::new());

    registry.register(graph_script::RunGraphScriptTool::new(node_registry));

    registry.register(knowledge_ops::SearchKnowledgeTool);
    registry.register(knowledge_ops::ReadKnowledgeTool);
    registry.register(knowledge_ops::BuildKnowledgePackTool);

    registry.register(file_ops::CreateNodeFolderTool);
    registry.register(file_ops::PatchNodeFileTool);
    registry.register(file_ops::WriteNodeFileTool);

    registry.register(rhai_ops::ReloadPluginTool);
    registry.register(rhai_ops::CheckNodeCompileTool);
}

pub fn build_tool_registry(profile: ToolProfile, node_registry: Arc<NodeRegistry>) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    match profile {
        ToolProfile::Full => register_full_tools(&mut registry, node_registry),
        ToolProfile::NodeAssistant => register_node_assistant_tools(&mut registry, node_registry),
    }
    registry
}

/// Async tool executor that runs tools off the UI thread
pub struct AsyncToolExecutor {
    request_tx: Sender<(ToolRequest, Sender<(u64, ToolResult)>)>,
    _worker_handles: Vec<std::thread::JoinHandle<()>>,
}

impl AsyncToolExecutor {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        let (request_tx, request_rx): (
            Sender<(ToolRequest, Sender<(u64, ToolResult)>)>,
            Receiver<_>,
        ) = unbounded();
        let worker_count = std::thread::available_parallelism()
            .map(|n| n.get().min(4))
            .unwrap_or(2)
            .max(1);
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let registry = registry.clone();
            let rx = request_rx.clone();
            handles.push(std::thread::spawn(move || {
                while let Ok((req, result_tx)) = rx.recv() {
                    let tool = match registry.get(&req.tool_name) {
                        Some(t) => t,
                        None => {
                            let _ = result_tx.send((
                                req.id,
                                ToolResult::Error(ToolError(format!(
                                    "Tool '{}' not found",
                                    req.tool_name
                                ))),
                            ));
                            continue;
                        }
                    };
                    let ctx = ToolContext {
                        cancel_token: req.cancel_token.clone(),
                        progress_callback: Some(Box::new({
                            let tx = result_tx.clone();
                            let id = req.id;
                            move |log| {
                                let _ = tx.send((id, ToolResult::Progress(log)));
                            }
                        })),
                    };
                    if ctx.is_cancelled() {
                        let _ = result_tx.send((req.id, ToolResult::Cancelled));
                        continue;
                    }
                    match tool.execute_with_context(req.args, &ctx) {
                        Ok(output) => {
                            let _ = result_tx.send((req.id, ToolResult::Success(output)));
                        }
                        Err(e) if e.0 == "Cancelled" => {
                            let _ = result_tx.send((req.id, ToolResult::Cancelled));
                        }
                        Err(e) => {
                            let _ = result_tx.send((req.id, ToolResult::Error(e)));
                        }
                    }
                }
            }));
        }
        Self {
            request_tx,
            _worker_handles: handles,
        }
    }

    /// Submit a tool execution request, returns channel for results
    pub fn submit(
        &self,
        id: u64,
        tool_name: String,
        args: Value,
        cancel_token: CancellationToken,
    ) -> Receiver<(u64, ToolResult)> {
        let (result_tx, result_rx) = unbounded();
        let req = ToolRequest {
            id,
            tool_name,
            args,
            cancel_token,
        };
        let _ = self.request_tx.send((req, result_tx));
        result_rx
    }
}
