use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use bevy::prelude::{IVec2, Vec2, Vec3};
use bevy_egui::egui::Pos2;

use super::definitions::{Tool, ToolDefinition, ToolError, ToolLog, ToolLogLevel, ToolOutput};
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::nodes::flow::spawn::{apply_foreach_block_direct, build_foreach_block};
use crate::nodes::parameter::ParameterValue;
use crate::nodes::PortId;
use crate::nodes::{Node, NodeGraph, NodeType};
use crate::nodes::parameter::ParameterUIType;

fn snapshot() -> Result<Arc<crate::nodes::graph_model::NodeGraphSnapshot>, ToolError> {
    crate::nodes::graph_model::global_graph_snapshot()
        .ok_or_else(|| ToolError("Graph snapshot not available yet".to_string()))
}

fn enqueue(cmd: crate::nodes::graph_model::GraphCommand) -> Result<(), ToolError> {
    crate::nodes::graph_model::enqueue_graph_command(cmd).map_err(ToolError)
}

fn find_node_id(
    graph: &crate::nodes::graph_model::NodeGraphSnapshot,
    identifier: &str,
) -> Option<Uuid> {
    if let Ok(uuid) = Uuid::parse_str(identifier) {
        if graph.nodes.contains_key(&uuid) {
            return Some(uuid);
        }
    }
    graph
        .nodes
        .iter()
        .find(|(_, n)| n.name == identifier)
        .map(|(id, _)| *id)
}

fn resolve_node_type(label: &str) -> NodeType {
    match label {
        "Create Cube" => NodeType::CreateCube,
        "Create Sphere" => NodeType::CreateSphere,
        "Transform" => NodeType::Transform,
        "Merge" => NodeType::Merge,
        "Curve" | "Curve (Plugin)" => NodeType::Generic("Curve".to_string()),
        "FBX Import" | "FBX Importer" => NodeType::FbxImporter,
        "VDB From Polygons" => NodeType::VdbFromPolygons,
        "VDB To Polygons" => NodeType::VdbToPolygons,
        other => NodeType::Generic(other.to_string()),
    }
}

fn normalize_port_name(port: Option<&str>, is_output: bool) -> String {
    let p = port.unwrap_or(if is_output { "out:0" } else { "in:0" }).trim();
    let p_lc = p.to_lowercase();
    match (is_output, p_lc.as_str()) {
        (true, "output") | (true, "out") | (true, "out0") | (true, "out:0") => "out:0".to_string(),
        (false, "input") | (false, "in") | (false, "in0") | (false, "in:0") => "in:0".to_string(),
        _ => p.to_string(),
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.is_empty() { return b.len(); }
    if b.is_empty() { return a.len(); }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur: Vec<usize> = vec![0; b.len() + 1];
    for (i, &ac) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &bc) in b.iter().enumerate() {
            let cost = (ac != bc) as usize;
            cur[j + 1] = (prev[j + 1] + 1).min((cur[j] + 1).min(prev[j] + cost));
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn suggest_names(needle: &str, candidates: &[String], k: usize) -> Vec<String> {
    let n = needle.trim().to_lowercase();
    if n.is_empty() { return Vec::new(); }
    let mut scored: Vec<(usize, &String)> = candidates
        .iter()
        .map(|c| (levenshtein(&n, &c.to_lowercase()), c))
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().take(k).map(|(_, s)| s.clone()).collect()
}

// -----------------------------------------------------------------------------
// create_node
// -----------------------------------------------------------------------------

pub struct CreateNodeTool {
    registry: Arc<NodeRegistry>,
}

impl CreateNodeTool {
    pub fn new(registry: Arc<NodeRegistry>) -> Self {
        Self { registry }
    }
}

#[derive(Serialize, Deserialize)]
struct CreateNodeArgs {
    node_type: String,
    node_name: Option<String>,
}

impl Tool for CreateNodeTool {
    fn name(&self) -> &str {
        "create_node"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Create a new graph node with default parameters.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_type": { "type": "string" },
                    "node_name": { "type": "string" }
                },
                "required": ["node_type"]
            }),
        }
    }

    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: CreateNodeArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;

        let node_type = resolve_node_type(&args.node_type);
        let node_id = Uuid::new_v4();
        let name = args.node_name.unwrap_or_else(|| node_type.name().to_string());
        let name_for_closure = name.clone();
        let registry = self.registry.clone();
        enqueue(Box::new(move |graph: &mut NodeGraph| {
            // Foreach macro block
            if matches!(&node_type, NodeType::Generic(s) if s == "ForEach Begin" || s == "ForEach End") {
                let spec = build_foreach_block(graph, &registry, None, Pos2::new(0.0, 0.0), false);
                let ids = apply_foreach_block_direct(graph, spec);
                for id in ids {
                    graph.mark_dirty(id);
                }
                return crate::nodes::graph_model::GraphCommandEffect { graph_changed: true, geometry_changed: true };
            }

            let mut node = Node::new(
                node_id,
                name_for_closure.clone(),
                node_type.clone(),
                Pos2::new(0.0, 0.0),
            );
            if let NodeType::Generic(type_name) = &node.node_type {
                if let Some(descriptor) = registry.nodes.read().unwrap().get(type_name) {
                    node.parameters = (descriptor.parameters_factory)();
                    node.inputs = descriptor
                        .inputs
                        .iter()
                        .map(|s| (PortId::from(s.as_str()), ()))
                        .collect();
                    node.outputs = descriptor
                        .outputs
                        .iter()
                        .map(|s| (PortId::from(s.as_str()), ()))
                        .collect();
                    node.rebuild_ports();
                }
            }
            graph.nodes.insert(node_id, node);
            graph.mark_dirty(node_id);
            crate::nodes::graph_model::GraphCommandEffect { graph_changed: true, geometry_changed: true }
        }))?;

        Ok(ToolOutput::new(
            format!("Queued create_node '{}' ({node_id}).", name),
            vec![ToolLog {
                message: format!("create_node queued: {} ({})", name, node_id),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

// -----------------------------------------------------------------------------
// delete_node
// -----------------------------------------------------------------------------

pub struct DeleteNodeTool;
impl DeleteNodeTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Serialize, Deserialize)]
struct DeleteNodeArgs {
    node_name_or_id: String,
}

impl Tool for DeleteNodeTool {
    fn name(&self) -> &str {
        "delete_node"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Delete a node from the graph by Name or ID.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "node_name_or_id": { "type": "string" } },
                "required": ["node_name_or_id"]
            }),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: DeleteNodeArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let graph = snapshot()?;
        let target_id = find_node_id(&graph, &args.node_name_or_id)
            .ok_or_else(|| ToolError(format!("Node not found: {}", args.node_name_or_id)))?;
        enqueue(Box::new(move |g: &mut NodeGraph| {
            g.mark_dirty(target_id);
            g.nodes.remove(&target_id);
            g.connections
                .retain(|_, c| c.from_node != target_id && c.to_node != target_id);
            g.geometry_cache.remove(&target_id);
            if g.display_node == Some(target_id) {
                g.display_node = None;
            }
            crate::nodes::graph_model::GraphCommandEffect { graph_changed: true, geometry_changed: true }
        }))?;
        Ok(ToolOutput::new(
            format!("Queued delete_node ({target_id})."),
            vec![ToolLog {
                message: format!("delete_node queued: {target_id}"),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

// -----------------------------------------------------------------------------
// connect_nodes
// -----------------------------------------------------------------------------

pub struct ConnectNodeTool;
impl ConnectNodeTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Serialize, Deserialize)]
struct ConnectNodeArgs {
    from_node: String,
    from_port: Option<String>,
    to_node: String,
    to_port: Option<String>,
}

impl Tool for ConnectNodeTool {
    fn name(&self) -> &str {
        "connect_nodes"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Connect two nodes (from output -> to input).".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "from_node": { "type": "string" },
                    "from_port": { "type": "string" },
                    "to_node": { "type": "string" },
                    "to_port": { "type": "string" }
                },
                "required": ["from_node","to_node"]
            }),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: ConnectNodeArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let graph = snapshot()?;
        let from_id = find_node_id(&graph, &args.from_node)
            .ok_or_else(|| ToolError(format!("Source node not found: {}", args.from_node)))?;
        let to_id = find_node_id(&graph, &args.to_node)
            .ok_or_else(|| ToolError(format!("Target node not found: {}", args.to_node)))?;
        let from_port = normalize_port_name(args.from_port.as_deref(), true);
        let to_port = normalize_port_name(args.to_port.as_deref(), false);
        let connection_id = Uuid::new_v4();
        enqueue(Box::new(move |g: &mut NodeGraph| {
            g.connections.insert(
                connection_id,
                crate::nodes::Connection {
                    id: connection_id,
                    from_node: from_id,
                    from_port: PortId::from(from_port.as_str()),
                    to_node: to_id,
                    to_port: PortId::from(to_port.as_str()),
                    order: 0,
                    waypoints: Vec::new(),
                },
            );
            g.mark_dirty(to_id);
            crate::nodes::graph_model::GraphCommandEffect { graph_changed: true, geometry_changed: true }
        }))?;
        Ok(ToolOutput::new(
            format!("Queued connect (id={connection_id})."),
            vec![ToolLog {
                message: format!("connect_nodes queued: {connection_id}"),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

// -----------------------------------------------------------------------------
// set_node_flag
// -----------------------------------------------------------------------------

pub struct SetNodeFlagTool;
impl SetNodeFlagTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Serialize, Deserialize)]
struct SetNodeFlagArgs {
    node_name: String,
    flag: String,
    active: bool,
}

impl Tool for SetNodeFlagTool {
    fn name(&self) -> &str {
        "set_node_flag"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Set a state flag on a node (bypass, display, template, lock).".to_string(),
            parameters: json!({
                "type":"object",
                "properties":{
                    "node_name":{"type":"string"},
                    "flag":{"type":"string","enum":["bypass","display","template","lock"]},
                    "active":{"type":"boolean"}
                },
                "required":["node_name","flag","active"]
            }),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: SetNodeFlagArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let graph = snapshot()?;
        let node_id = find_node_id(&graph, &args.node_name)
            .ok_or_else(|| ToolError(format!("Node not found: {}", args.node_name)))?;
        let flag = args.flag.clone();
        let active = args.active;
        enqueue(Box::new(move |g: &mut NodeGraph| {
            if let Some(n) = g.nodes.get_mut(&node_id) {
                match flag.as_str() {
                    "bypass" => {
                        n.is_bypassed = active;
                        g.mark_dirty(node_id);
                    }
                    "lock" => n.is_locked = active,
                    "template" => n.is_template = active,
                    "display" => {
                        if active {
                            g.display_node = Some(node_id);
                            for (id, nn) in g.nodes.iter_mut() {
                                nn.is_display_node = *id == node_id;
                            }
                            g.mark_dirty(node_id);
                        } else {
                            if g.display_node == Some(node_id) {
                                g.display_node = None;
                            }
                            n.is_display_node = false;
                        }
                    }
                    _ => {}
                }
            }
            crate::nodes::graph_model::GraphCommandEffect { graph_changed: true, geometry_changed: true }
        }))?;
        Ok(ToolOutput::new(
            "Queued set_node_flag.".to_string(),
            vec![ToolLog {
                message: "set_node_flag queued".into(),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

// -----------------------------------------------------------------------------
// set_parameter
// -----------------------------------------------------------------------------

pub struct SetParameterTool;
impl SetParameterTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Serialize, Deserialize)]
struct SetParameterArgs {
    node_name: String,
    param_name: String,
    value: Value,
}

impl Tool for SetParameterTool {
    fn name(&self) -> &str {
        "set_parameter"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Set the value of a node parameter (simple types).".to_string(),
            parameters: json!({
                "type":"object",
                "properties":{
                    "node_name":{"type":"string"},
                    "param_name":{"type":"string"},
                    "value":{}
                },
                "required":["node_name","param_name","value"]
            }),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: SetParameterArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let graph = snapshot()?;
        let node_id = find_node_id(&graph, &args.node_name)
            .ok_or_else(|| ToolError(format!("Node not found: {}", args.node_name)))?;
        let node = graph
            .nodes
            .get(&node_id)
            .ok_or_else(|| ToolError("Node not found in snapshot".to_string()))?;
        let current_val = node
            .parameters
            .iter()
            .find(|p| p.name == args.param_name)
            .map(|p| &p.value)
            .ok_or_else(|| ToolError(format!("Parameter not found: {}", args.param_name)))?;

        let new_val = match current_val {
            ParameterValue::Float(_) => {
                let v: f64 = serde_json::from_value(args.value.clone())
                    .map_err(|_| ToolError("Expected float value".to_string()))?;
                ParameterValue::Float(v as f32)
            }
            ParameterValue::Int(_) => {
                let v: i64 = serde_json::from_value(args.value.clone())
                    .map_err(|_| ToolError("Expected integer value".to_string()))?;
                ParameterValue::Int(v as i32)
            }
            ParameterValue::Bool(_) => {
                let v: bool = serde_json::from_value(args.value.clone())
                    .map_err(|_| ToolError("Expected boolean value".to_string()))?;
                ParameterValue::Bool(v)
            }
            ParameterValue::String(_) => {
                let v: String = serde_json::from_value(args.value.clone())
                    .map_err(|_| ToolError("Expected string value".to_string()))?;
                ParameterValue::String(v)
            }
            ParameterValue::Vec2(_) => {
                let v: [f32; 2] = serde_json::from_value(args.value.clone())
                    .map_err(|_| ToolError("Expected Vec2 [x,y]".to_string()))?;
                ParameterValue::Vec2(Vec2::new(v[0], v[1]))
            }
            ParameterValue::Vec3(_) => {
                let v: [f32; 3] = serde_json::from_value(args.value.clone())
                    .map_err(|_| ToolError("Expected Vec3 [x,y,z]".to_string()))?;
                ParameterValue::Vec3(Vec3::new(v[0], v[1], v[2]))
            }
            ParameterValue::IVec2(_) => {
                let v: [i32; 2] = serde_json::from_value(args.value.clone())
                    .map_err(|_| ToolError("Expected IVec2 [x,y]".to_string()))?;
                ParameterValue::IVec2(IVec2::new(v[0], v[1]))
            }
            _ => {
                return Err(ToolError(format!(
                    "Unsupported parameter type for tools: {:?}",
                    current_val
                )))
            }
        };

        let pname = args.param_name.clone();
        enqueue(Box::new(move |g: &mut NodeGraph| {
            if let Some(n) = g.nodes.get_mut(&node_id) {
                if let Some(p) = n.parameters.iter_mut().find(|p| p.name == pname) {
                    p.value = new_val;
                }
                g.mark_dirty(node_id);
            }
            crate::nodes::graph_model::GraphCommandEffect { graph_changed: true, geometry_changed: true }
        }))?;

        Ok(ToolOutput::new(
            "Queued set_parameter.".to_string(),
            vec![ToolLog {
                message: "set_parameter queued".into(),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

// -----------------------------------------------------------------------------
// get_graph_state
// -----------------------------------------------------------------------------

pub struct GetGraphStateTool;
impl GetGraphStateTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for GetGraphStateTool {
    fn name(&self) -> &str {
        "get_graph_state"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Returns a summary of the current node graph.".to_string(),
            parameters: json!({"type":"object","properties":{},"required":[]}),
        }
    }
    fn execute(&self, _args: Value) -> Result<ToolOutput, ToolError> {
        let graph = snapshot()?;
        let nodes: Vec<_> = graph
            .nodes
            .values()
            .map(|n| {
                json!({
                    "id": n.id.to_string(),
                    "name": n.name,
                    "type": n.node_type.name(),
                    "bypassed": n.is_bypassed,
                    "display": n.is_display_node
                })
            })
            .collect();
        let connections: Vec<_> = graph
            .connections
            .values()
            .map(|c| {
                json!({
                    "from": c.from_node.to_string(),
                    "from_port": c.from_port.to_string(),
                    "to": c.to_node.to_string(),
                    "to_port": c.to_port.to_string()
                })
            })
            .collect();
        let state = json!({
            "node_count": graph.nodes.len(),
            "connection_count": graph.connections.len(),
            "display_node": graph.display_node.map(|id| id.to_string()),
            "nodes": nodes,
            "connections": connections
        });
        Ok(ToolOutput::new(
            serde_json::to_string_pretty(&state).unwrap_or_else(|_| "{}".to_string()),
            vec![ToolLog {
                message: "Read graph snapshot".into(),
                level: ToolLogLevel::Info,
            }],
        ))
    }
}

// -----------------------------------------------------------------------------
// edit_node_graph (batch) - queued best-effort
// -----------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct EditNodeGraphArgs {
    #[serde(default)]
    description: Option<String>,
    ops: Vec<GraphEditOp>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GraphEditOp {
    CreateNode {
        node_type: String,
        #[serde(default)]
        alias: Option<String>,
        #[serde(default)]
        node_name: Option<String>,
        #[serde(default)]
        position: Option<[f32; 2]>,
    },
    DeleteNode { target: String },
    Connect { from: NodePortRef, to: NodePortRef },
    Disconnect { from: NodePortRef, to: NodePortRef },
    SetParam { target: String, param: String, value: Value },
    SetFlag { target: String, flag: String, value: bool },
    SetDisplay { target: String },
    SetPosition { target: String, position: [f32; 2] },
    RenameNode { target: String, new_name: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodePortRef {
    pub target: String,
    #[serde(default)]
    pub port: Option<String>,
}

pub struct EditNodeGraphTool {
    registry: Arc<NodeRegistry>,
}

impl EditNodeGraphTool {
    pub fn new(registry: Arc<NodeRegistry>) -> Self {
        Self { registry }
    }
}

impl Tool for EditNodeGraphTool {
    fn name(&self) -> &str {
        "edit_node_graph"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Apply a batch of graph edit operations (queued).".to_string(),
            parameters: json!({"type":"object","properties":{"description":{"type":"string"},"ops":{"type":"array"}},"required":["ops"]}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: EditNodeGraphArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let ops = args.ops;
        let ops_len = ops.len();

        // Pre-validate ops against current snapshot to avoid silent no-ops.
        let snap = snapshot()?;
        let mut virtual_aliases: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut errors: Vec<String> = Vec::new();
        let resolve_snap = |va: &std::collections::HashSet<String>, t: &str| -> Option<Uuid> {
            if va.contains(t) { None } else { find_node_id(&snap, t) }
        };
        let ports_of = |id: &Uuid| -> (Vec<String>, Vec<String>) {
            let Some(n) = snap.nodes.get(id) else { return (Vec::new(), Vec::new()); };
            let mut ins: Vec<String> = n.inputs.keys().map(|p| p.to_string()).collect();
            let mut outs: Vec<String> = n.outputs.keys().map(|p| p.to_string()).collect();
            ins.sort();
            outs.sort();
            (ins, outs)
        };
        for op in &ops {
            match op {
                GraphEditOp::CreateNode { alias: Some(a), .. } => {
                    virtual_aliases.insert(a.clone());
                }
                GraphEditOp::SetParam { target, param, .. } => {
                    if let Some(id) = resolve_snap(&virtual_aliases, target) {
                        let Some(n) = snap.nodes.get(&id) else { continue; };
                        if !n.parameters.iter().any(|p| p.name == *param) {
                            let mut avail: Vec<String> = n.parameters.iter().map(|p| p.name.clone()).collect();
                            avail.sort();
                            let sug = suggest_names(param, &avail, 3);
                            let mut msg = format!(
                                "SetParam: parameter '{}' not found on node '{}' (type {}). Available parameters: {:?}.",
                                param, n.name, n.node_type.name(), avail
                            );
                            if !sug.is_empty() { msg.push_str(&format!(" Did you mean: {:?}?", sug)); }
                            msg.push_str(" Tip: for per-axis edits, set the whole vec param (e.g. divisions=[x,y,z]).");
                            errors.push(msg);
                        }
                    } else if !virtual_aliases.contains(target) {
                        errors.push(format!("SetParam: target node not found: {}", target));
                    }
                }
                GraphEditOp::Connect { from, to } | GraphEditOp::Disconnect { from, to } => {
                    if let Some(from_id) = resolve_snap(&virtual_aliases, &from.target) {
                        let (_ins, outs) = ports_of(&from_id);
                        let port = normalize_port_name(from.port.as_deref(), true);
                        if !outs.iter().any(|p| p == &port) {
                            errors.push(format!(
                                "Connect: source port '{}' not found on '{}'. Available outputs: {:?}",
                                port, from.target, outs
                            ));
                        }
                    } else if !virtual_aliases.contains(&from.target) {
                        errors.push(format!("Connect: source node not found: {}", from.target));
                    }
                    if let Some(to_id) = resolve_snap(&virtual_aliases, &to.target) {
                        let (ins, _outs) = ports_of(&to_id);
                        let port = normalize_port_name(to.port.as_deref(), false);
                        if !ins.iter().any(|p| p == &port) {
                            errors.push(format!(
                                "Connect: target port '{}' not found on '{}'. Available inputs: {:?}",
                                port, to.target, ins
                            ));
                        }
                    } else if !virtual_aliases.contains(&to.target) {
                        errors.push(format!("Connect: target node not found: {}", to.target));
                    }
                }
                GraphEditOp::SetFlag { target, .. }
                | GraphEditOp::SetDisplay { target }
                | GraphEditOp::SetPosition { target, .. }
                | GraphEditOp::RenameNode { target, .. }
                | GraphEditOp::DeleteNode { target } => {
                    if target != "display"
                        && resolve_snap(&virtual_aliases, target).is_none()
                        && !virtual_aliases.contains(target)
                    {
                        errors.push(format!("Op: target node not found: {}", target));
                    }
                }
                _ => {}
            }
        }
        if !errors.is_empty() {
            return Err(ToolError(errors.join("\n")));
        }

        let registry = self.registry.clone();
        enqueue(Box::new(move |graph: &mut NodeGraph| {
            let mut aliases: HashMap<String, Uuid> = HashMap::new();
            fn resolve_target(graph: &NodeGraph, aliases: &HashMap<String, Uuid>, t: &str) -> Option<Uuid> {
                if let Some(id) = aliases.get(t) {
                    return Some(*id);
                }
                if t == "display" {
                    return graph.display_node;
                }
                if let Ok(uuid) = Uuid::parse_str(t) {
                    if graph.nodes.contains_key(&uuid) {
                        return Some(uuid);
                    }
                }
                graph
                    .nodes
                    .iter()
                    .find(|(_, n)| n.name == t)
                    .map(|(id, _)| *id)
            }

            for op in ops {
                match op {
                    GraphEditOp::CreateNode { node_type, alias, node_name, position } => {
                        let node_type_resolved = resolve_node_type(&node_type);
                        let pos = position.map(|[x, y]| Pos2::new(x, y)).unwrap_or(Pos2::new(0.0, 0.0));
                        if matches!(&node_type_resolved, NodeType::Generic(s) if s == "ForEach Begin" || s == "ForEach End") {
                            let spec = build_foreach_block(graph, &registry, None, pos, false);
                            let ids = apply_foreach_block_direct(graph, spec);
                            if let Some(a) = alias {
                                if let Some(id) = ids.first().copied() { aliases.insert(a, id); }
                            }
                            for id in ids { graph.mark_dirty(id); }
                            continue;
                        }
                        let id = Uuid::new_v4();
                        let name = node_name.unwrap_or_else(|| node_type_resolved.name().to_string());
                        let mut node = Node::new(id, name, node_type_resolved, pos);
                        if let NodeType::Generic(type_name) = &node.node_type {
                            if let Some(descriptor) = registry.nodes.read().unwrap().get(type_name) {
                                node.parameters = (descriptor.parameters_factory)();
                                node.inputs = descriptor.inputs.iter().map(|s| (PortId::from(s.as_str()), ())).collect();
                                node.outputs = descriptor.outputs.iter().map(|s| (PortId::from(s.as_str()), ())).collect();
                                node.rebuild_ports();
                            }
                        }
                        graph.nodes.insert(id, node);
                        graph.mark_dirty(id);
                        if let Some(a) = alias { aliases.insert(a, id); }
                    }
                    GraphEditOp::DeleteNode { target } => {
                        if let Some(id) = resolve_target(graph, &aliases, &target) {
                            graph.mark_dirty(id);
                            graph.nodes.remove(&id);
                            graph.connections.retain(|_, c| c.from_node != id && c.to_node != id);
                            if graph.display_node == Some(id) { graph.display_node = None; }
                        }
                    }
                    GraphEditOp::Connect { from, to } => {
                        let (Some(from_id), Some(to_id)) = (resolve_target(graph, &aliases, &from.target), resolve_target(graph, &aliases, &to.target)) else { continue; };
                        let from_port = PortId::from(normalize_port_name(from.port.as_deref(), true).as_str());
                        let to_port = PortId::from(normalize_port_name(to.port.as_deref(), false).as_str());
                        let connection_id = Uuid::new_v4();
                        graph.connections.insert(connection_id, crate::nodes::Connection { id: connection_id, from_node: from_id, from_port, to_node: to_id, to_port, order: 0, waypoints: Vec::new() });
                        graph.mark_dirty(to_id);
                    }
                    GraphEditOp::Disconnect { from, to } => {
                        let (Some(from_id), Some(to_id)) = (resolve_target(graph, &aliases, &from.target), resolve_target(graph, &aliases, &to.target)) else { continue; };
                        let from_port = PortId::from(normalize_port_name(from.port.as_deref(), true).as_str());
                        let to_port = PortId::from(normalize_port_name(to.port.as_deref(), false).as_str());
                        graph.connections.retain(|_, c| !(c.from_node == from_id && c.from_port == from_port && c.to_node == to_id && c.to_port == to_port));
                        graph.mark_dirty(to_id);
                    }
                    GraphEditOp::SetParam { target, param, value } => {
                        let Some(id) = resolve_target(graph, &aliases, &target) else { continue; };
                        if let Some(n) = graph.nodes.get_mut(&id) {
                            if let Some(p) = n.parameters.iter_mut().find(|p| p.name == param) {
                                match &p.value {
                                    ParameterValue::Float(_) => if let Ok(v) = serde_json::from_value::<f64>(value.clone()) { p.value = ParameterValue::Float(v as f32); },
                                    ParameterValue::Int(_) => if let Ok(v) = serde_json::from_value::<i64>(value.clone()) { p.value = ParameterValue::Int(v as i32); },
                                    ParameterValue::Bool(_) => if let Ok(v) = serde_json::from_value::<bool>(value.clone()) { p.value = ParameterValue::Bool(v); },
                                    ParameterValue::String(_) => if let Ok(v) = serde_json::from_value::<String>(value.clone()) { p.value = ParameterValue::String(v); },
                                    ParameterValue::Vec3(_) => if let Ok(v) = serde_json::from_value::<[f32;3]>(value.clone()) { p.value = ParameterValue::Vec3(Vec3::new(v[0], v[1], v[2])); },
                                    ParameterValue::Vec2(_) => if let Ok(v) = serde_json::from_value::<[f32;2]>(value.clone()) { p.value = ParameterValue::Vec2(Vec2::new(v[0], v[1])); },
                                    _ => {}
                                }
                            }
                        }
                        graph.mark_dirty(id);
                    }
                    GraphEditOp::SetFlag { target, flag, value } => {
                        let Some(id) = resolve_target(graph, &aliases, &target) else { continue; };
                        if let Some(n) = graph.nodes.get_mut(&id) {
                            match flag.as_str() {
                                "bypass" => { n.is_bypassed = value; graph.mark_dirty(id); }
                                "lock" => n.is_locked = value,
                                "template" => n.is_template = value,
                                "display" => if value {
                                    graph.display_node = Some(id);
                                    for (nid, nn) in graph.nodes.iter_mut() { nn.is_display_node = *nid == id; }
                                    graph.mark_dirty(id);
                                },
                                _ => {}
                            }
                        }
                    }
                    GraphEditOp::SetDisplay { target } => {
                        if let Some(id) = resolve_target(graph, &aliases, &target) {
                            graph.display_node = Some(id);
                            for (nid, nn) in graph.nodes.iter_mut() { nn.is_display_node = *nid == id; }
                            graph.mark_dirty(id);
                        }
                    }
                    GraphEditOp::SetPosition { target, position } => {
                        if let Some(id) = resolve_target(graph, &aliases, &target) {
                            if let Some(n) = graph.nodes.get_mut(&id) { n.position = Pos2::new(position[0], position[1]); }
                        }
                    }
                    GraphEditOp::RenameNode { target, new_name } => {
                        if let Some(id) = resolve_target(graph, &aliases, &target) {
                            if let Some(n) = graph.nodes.get_mut(&id) { n.name = new_name; }
                        }
                    }
                }
            }
            crate::nodes::graph_model::GraphCommandEffect { graph_changed: true, geometry_changed: true }
        }))?;

        let summary = args
            .description
            .map(|d| format!("Queued edit_node_graph: {d} ({} ops).", ops_len))
            .unwrap_or_else(|| format!("Queued edit_node_graph ({} ops).", ops_len));
        Ok(ToolOutput::new(
            summary,
            vec![ToolLog {
                message: "edit_node_graph queued".into(),
                level: ToolLogLevel::Info,
            }],
        ))
    }
}

// -----------------------------------------------------------------------------
// get_geometry_insight
// -----------------------------------------------------------------------------

pub struct GetGeometryInsightTool;
impl GetGeometryInsightTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Serialize, Deserialize)]
struct GetGeometryInsightArgs {
    #[serde(default)]
    node_ref: Option<String>,
}

impl Tool for GetGeometryInsightTool {
    fn name(&self) -> &str {
        "get_geometry_insight"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Return a compact geometry fingerprint for a node's cached geometry (snapshot).".to_string(),
            parameters: json!({"type":"object","properties":{"node_ref":{"type":"string"}},"required":[]}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: GetGeometryInsightArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let graph = snapshot()?;
        let node_id = if let Some(node_ref) = args.node_ref {
            if node_ref == "display" {
                graph
                    .display_node
                    .ok_or_else(|| ToolError("No display node is currently set".to_string()))?
            } else {
                find_node_id(&graph, &node_ref).ok_or_else(|| ToolError(format!("Node not found: {node_ref}")))?
            }
        } else {
            graph.display_node.ok_or_else(|| ToolError("No display node is currently set; please specify node_ref".to_string()))?
        };
        let geo_arc = graph.geometry_cache.get(&node_id).cloned().ok_or_else(|| ToolError("No cached geometry found for the requested node.".to_string()))?;
        let fingerprint = geo_arc.compute_fingerprint();
        let text = serde_json::to_string_pretty(&fingerprint).unwrap_or_else(|_| "{}".to_string());
        Ok(ToolOutput::new(
            text,
            vec![ToolLog {
                message: format!(
                    "Read geometry insight for node {} (points: {}, primitives: {})",
                    node_id, fingerprint.point_count, fingerprint.primitive_count
                ),
                level: ToolLogLevel::Info,
            }],
        ))
    }
}

// -----------------------------------------------------------------------------
// compare_geometry
// -----------------------------------------------------------------------------

pub struct CompareGeometryTool;
impl CompareGeometryTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Serialize, Deserialize)]
struct CompareGeometryArgs {
    node_a_ref: String,
    node_b_ref: String,
}

impl Tool for CompareGeometryTool {
    fn name(&self) -> &str {
        "compare_geometry"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Compare geometry fingerprints between two nodes (by name/uuid/display).".to_string(),
            parameters: json!({"type":"object","properties":{"node_a_ref":{"type":"string"},"node_b_ref":{"type":"string"}},"required":["node_a_ref","node_b_ref"]}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let a: CompareGeometryArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let snap = snapshot()?;
        let resolve = |r: &str| -> Result<Uuid, ToolError> {
            if r == "display" {
                return snap
                    .display_node
                    .ok_or_else(|| ToolError("No display node is currently set".to_string()));
            }
            find_node_id(&snap, r).ok_or_else(|| ToolError(format!("Node not found: {r}")))
        };
        let id_a = resolve(&a.node_a_ref)?;
        let id_b = resolve(&a.node_b_ref)?;
        let geo_a = snap
            .geometry_cache
            .get(&id_a)
            .cloned()
            .ok_or_else(|| ToolError("No cached geometry for node_a_ref".to_string()))?;
        let geo_b = snap
            .geometry_cache
            .get(&id_b)
            .cloned()
            .ok_or_else(|| ToolError("No cached geometry for node_b_ref".to_string()))?;
        let fp_a = geo_a.compute_fingerprint();
        let fp_b = geo_b.compute_fingerprint();

        let diff = json!({
            "node_a": {"ref": a.node_a_ref, "id": id_a.to_string(), "fingerprint": fp_a},
            "node_b": {"ref": a.node_b_ref, "id": id_b.to_string(), "fingerprint": fp_b},
            "equal_core": {
                "point_count": fp_a.point_count == fp_b.point_count,
                "primitive_count": fp_a.primitive_count == fp_b.primitive_count,
                "bbox_min": fp_a.bbox_min == fp_b.bbox_min,
                "bbox_max": fp_a.bbox_max == fp_b.bbox_max,
                "topology": fp_a.topology == fp_b.topology
            }
        });
        let summary = format!(
            "CompareGeometry: points {} vs {}, prims {} vs {}.",
            fp_a.point_count, fp_b.point_count, fp_a.primitive_count, fp_b.primitive_count
        );
        Ok(ToolOutput::with_summary(
            summary,
            serde_json::to_string_pretty(&diff).unwrap_or_else(|_| "{}".to_string()),
        ))
    }
}

// -----------------------------------------------------------------------------
// get_node_info (instance + type)
// -----------------------------------------------------------------------------

pub struct GetNodeInfoTool {
    registry: Arc<NodeRegistry>,
}

impl GetNodeInfoTool {
    pub fn new(registry: Arc<NodeRegistry>) -> Self {
        Self { registry }
    }
}

#[derive(Serialize, Deserialize)]
struct GetNodeInfoArgs {
    #[serde(default)]
    node_ref: Option<String>,
    #[serde(default)]
    node_type: Option<String>,
}

impl Tool for GetNodeInfoTool {
    fn name(&self) -> &str {
        "get_node_info"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Query node info. Use node_ref (graph instance: name/uuid/display) or node_type (registry key).".to_string(),
            parameters: json!({"type":"object","properties":{"node_ref":{"type":"string","description":"Node name/uuid or 'display'"},"node_type":{"type":"string","description":"Registry key (e.g. 'Create Cube')"}}}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: GetNodeInfoArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let value_kind = |v: &ParameterValue| -> &'static str {
            match v {
                ParameterValue::Float(_) => "float",
                ParameterValue::Int(_) => "int",
                ParameterValue::Vec2(_) => "vec2",
                ParameterValue::Vec3(_) => "vec3",
                ParameterValue::Vec4(_) => "vec4",
                ParameterValue::IVec2(_) => "ivec2",
                ParameterValue::String(_) => "string",
                ParameterValue::Color(_) => "color3",
                ParameterValue::Color4(_) => "color4",
                ParameterValue::Bool(_) => "bool",
                ParameterValue::Curve(_) => "curve",
                ParameterValue::UnitySpline(_) => "unity_spline",
                ParameterValue::Volume(_) => "volume",
            }
        };
        let value_json = |v: &ParameterValue| -> Value {
            serde_json::to_value(v).unwrap_or_else(|_| json!(format!("{:?}", v)))
        };

        if let Some(node_ref) = args.node_ref.as_deref() {
            let graph = snapshot()?;
            let id = if node_ref == "display" {
                graph
                    .display_node
                    .ok_or_else(|| ToolError("No display node is currently set".to_string()))?
            } else {
                find_node_id(&graph, node_ref)
                    .ok_or_else(|| ToolError(format!("Node not found: {}", node_ref)))?
            };
            let node = graph
                .nodes
                .get(&id)
                .ok_or_else(|| ToolError("Node not found in snapshot".to_string()))?;
            let mut inputs: Vec<String> = node.inputs.keys().map(|p| p.to_string()).collect();
            let mut outputs: Vec<String> = node.outputs.keys().map(|p| p.to_string()).collect();
            inputs.sort();
            outputs.sort();
            let params: Vec<Value> = node
                .parameters
                .iter()
                .map(|p| {
                    json!({
                        "name": p.name,
                        "label": p.label,
                        "group": p.group,
                        "kind": value_kind(&p.value),
                        "value": value_json(&p.value),
                        "ui": format!("{:?}", p.ui_type),
                    })
                })
                .collect();
            let info = json!({
                "scope": "instance",
                "id": id.to_string(),
                "name": node.name,
                "type": node.node_type.name(),
                "inputs": inputs,
                "outputs": outputs,
                "parameters": params,
            });
            return Ok(ToolOutput::new(
                serde_json::to_string_pretty(&info).unwrap_or_else(|_| "{}".to_string()),
                vec![ToolLog { message: "Read node instance".into(), level: ToolLogLevel::Info }],
            ));
        }

        let Some(node_type) = args.node_type.as_deref().filter(|s| !s.trim().is_empty()) else {
            return Err(ToolError("Missing argument: provide node_ref or node_type".to_string()));
        };
        let desc = self
            .registry
            .nodes
            .read()
            .unwrap()
            .get(node_type)
            .cloned()
            .ok_or_else(|| ToolError(format!("Node type not found: {}", node_type)))?;
        let params = (desc.parameters_factory)();
        let params: Vec<Value> = params
            .iter()
            .map(|p| {
                json!({
                    "name": p.name,
                    "label": p.label,
                    "group": p.group,
                    "kind": value_kind(&p.value),
                    "default": value_json(&p.value),
                    "ui": format!("{:?}", p.ui_type),
                })
            })
            .collect();
        let info = json!({
            "scope": "type",
            "name": desc.name,
            "display_name": desc.display_name,
            "category": desc.category,
            "inputs": desc.inputs,
            "outputs": desc.outputs,
            "origin": format!("{:?}", desc.origin),
            "parameters": params,
        });
        Ok(ToolOutput::new(
            serde_json::to_string_pretty(&info).unwrap_or_else(|_| "{}".to_string()),
            vec![ToolLog { message: "Read node type info".into(), level: ToolLogLevel::Info }],
        ))
    }
}

// -----------------------------------------------------------------------------
// get_node_library
// -----------------------------------------------------------------------------

pub struct GetNodeLibraryTool {
    registry: Arc<NodeRegistry>,
}

// -----------------------------------------------------------------------------
// export_nodespec (from instance/type)
// -----------------------------------------------------------------------------

pub struct ExportNodeSpecTool {
    registry: Arc<NodeRegistry>,
}

impl ExportNodeSpecTool {
    pub fn new(registry: Arc<NodeRegistry>) -> Self {
        Self { registry }
    }
}

#[derive(Serialize, Deserialize)]
struct ExportNodeSpecArgs {
    #[serde(default)]
    node_ref: Option<String>,
    #[serde(default)]
    node_type: Option<String>,
    #[serde(default)]
    plugin_name: Option<String>,
}

fn nodespec_type_of(v: &ParameterValue) -> Option<&'static str> {
    match v {
        ParameterValue::Int(_) => Some("int"),
        ParameterValue::Float(_) => Some("float"),
        ParameterValue::Bool(_) => Some("bool"),
        ParameterValue::String(_) => Some("string"),
        ParameterValue::Vec2(_) => Some("vec2"),
        ParameterValue::Vec3(_) => Some("vec3"),
        ParameterValue::Vec4(_) => Some("vec4"),
        ParameterValue::Color(_) => Some("color3"),
        ParameterValue::Color4(_) => Some("color4"),
        _ => None,
    }
}

fn nodespec_ui_of(ui: &ParameterUIType) -> Option<Value> {
    match ui {
        ParameterUIType::FloatSlider { min, max } => Some(json!({"kind":"float_slider","min":min,"max":max})),
        ParameterUIType::IntSlider { min, max } => Some(json!({"kind":"int_slider","min":*min as f32,"max":*max as f32})),
        ParameterUIType::Vec2Drag => Some(json!({"kind":"vec2_drag"})),
        ParameterUIType::Vec3Drag => Some(json!({"kind":"vec3_drag"})),
        ParameterUIType::Vec4Drag => Some(json!({"kind":"vec4_drag"})),
        ParameterUIType::String => Some(json!({"kind":"string"})),
        ParameterUIType::Toggle => Some(json!({"kind":"toggle"})),
        ParameterUIType::Dropdown { choices } => Some(json!({"kind":"dropdown","choices":choices})),
        ParameterUIType::Color { show_alpha } => Some(json!({"kind":"color","show_alpha":show_alpha})),
        ParameterUIType::Code => Some(json!({"kind":"code"})),
        _ => None,
    }
}

impl Tool for ExportNodeSpecTool {
    fn name(&self) -> &str {
        "export_nodespec"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Export a NodeSpec skeleton from a node instance (node_ref) or node type (node_type).".to_string(),
            parameters: json!({"type":"object","properties":{"node_ref":{"type":"string","description":"Node name/uuid or 'display'"},"node_type":{"type":"string","description":"Registry key (e.g. 'Create Cube')"},"plugin_name":{"type":"string","description":"Optional plugin_name to include"} } }),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let a: ExportNodeSpecArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let plugin_name = a.plugin_name.unwrap_or_else(|| "my_plugin".to_string());

        let (node_name, category, inputs, outputs, params) = if let Some(node_ref) = a.node_ref.as_deref() {
            let graph = snapshot()?;
            let id = if node_ref == "display" {
                graph.display_node.ok_or_else(|| ToolError("No display node is currently set".to_string()))?
            } else {
                find_node_id(&graph, node_ref).ok_or_else(|| ToolError(format!("Node not found: {}", node_ref)))?
            };
            let node = graph.nodes.get(&id).ok_or_else(|| ToolError("Node not found in snapshot".to_string()))?;
            let node_name = node.node_type.name().to_string();
            let mut inputs: Vec<String> = node.inputs.keys().map(|p| p.to_string()).collect();
            let mut outputs: Vec<String> = node.outputs.keys().map(|p| p.to_string()).collect();
            inputs.sort();
            outputs.sort();
            let params = node.parameters.clone();
            (node_name, Some("Exported".to_string()), inputs, outputs, params)
        } else {
            let Some(node_type) = a.node_type.as_deref().filter(|s| !s.trim().is_empty()) else {
                return Err(ToolError("Missing argument: provide node_ref or node_type".to_string()));
            };
            let desc = self.registry.nodes.read().unwrap().get(node_type).cloned().ok_or_else(|| ToolError(format!("Node type not found: {}", node_type)))?;
            let params = (desc.parameters_factory)();
            (desc.display_name, Some(desc.category), desc.inputs, desc.outputs, params)
        };

        let params_v: Vec<Value> = params
            .iter()
            .filter_map(|p| {
                let ty = nodespec_type_of(&p.value)?;
                let mut o = json!({
                    "name": p.name,
                    "type": ty,
                    "default": serde_json::to_value(&p.value).unwrap_or(json!(null)),
                });
                if !p.label.is_empty() { o["label"] = json!(p.label.clone()); }
                if !p.group.is_empty() { o["group"] = json!(p.group.clone()); }
                if let Some(ui) = nodespec_ui_of(&p.ui_type) { o["ui"] = ui; }
                Some(o)
            })
            .collect();

        let nodespec = json!({
            "plugin_name": plugin_name,
            "node": {
                "name": node_name,
                "category": category.unwrap_or_else(|| "Experimental".to_string()),
                "inputs": inputs,
                "outputs": outputs,
                "params": params_v
            }
        });
        Ok(ToolOutput::new(
            serde_json::to_string_pretty(&nodespec).unwrap_or_else(|_| "{}".to_string()),
            vec![ToolLog { message: "Exported nodespec skeleton".into(), level: ToolLogLevel::Success }],
        ))
    }
}

impl GetNodeLibraryTool {
    pub fn new(registry: Arc<NodeRegistry>) -> Self {
        Self { registry }
    }
}

#[derive(Serialize, Deserialize)]
struct GetNodeLibraryArgs {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default = "d200")]
    limit: usize,
}

fn d200() -> usize {
    200
}

impl Tool for GetNodeLibraryTool {
    fn name(&self) -> &str {
        "get_node_library"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "List node types available in the runtime registry (built-in + plugins).".to_string(),
            parameters: json!({"type":"object","properties":{"query":{"type":"string","description":"Optional fuzzy filter on display name"},"category":{"type":"string","description":"Optional category filter"},"limit":{"type":"integer","description":"Max returned items (default 200)"}}}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let a: GetNodeLibraryArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let map = self.registry.nodes.read().unwrap();
        let mut items: Vec<Value> = map
            .values()
            .filter(|d| {
                a.category.as_ref().is_none_or(|c| d.category == *c)
                    && a.query.as_ref().is_none_or(|q| d.display_name_lc.contains(&q.trim().to_lowercase()))
            })
            .map(|d| {
                json!({
                    "display_name": d.display_name,
                    "name": d.name,
                    "category": d.category,
                    "origin": format!("{:?}", d.origin),
                })
            })
            .collect();
        items.sort_by(|a, b| a.get("display_name").and_then(|v| v.as_str()).cmp(&b.get("display_name").and_then(|v| v.as_str())));
        if items.len() > a.limit {
            items.truncate(a.limit);
        }
        Ok(ToolOutput::new(
            serde_json::to_string_pretty(&json!({ "count": items.len(), "items": items }))
                .unwrap_or_else(|_| "{}".to_string()),
            vec![ToolLog { message: "Listed node registry".into(), level: ToolLogLevel::Info }],
        ))
    }
}

