use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

use bevy::prelude::{IVec2, Vec2, Vec3};
use bevy_egui::egui::{Color32, Pos2, Rect, Vec2 as EVec2};

use super::definitions::{Tool, ToolDefinition, ToolError, ToolLog, ToolLogLevel, ToolOutput};
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::nodes::flow::spawn::{apply_foreach_block_direct, build_foreach_block};
use crate::nodes::parameter::ParameterValue;
use crate::nodes::PortId;
use crate::nodes::{Node, NodeGraph, NodeType};

fn snapshot() -> Result<Arc<crate::nodes::graph_model::NodeGraphSnapshot>, ToolError> {
    crate::nodes::graph_model::global_graph_snapshot()
        .ok_or_else(|| ToolError("Graph snapshot not available yet".to_string()))
}

fn enqueue(cmd: crate::nodes::graph_model::GraphCommand) -> Result<(), ToolError> {
    crate::nodes::graph_model::enqueue_graph_command(cmd).map_err(ToolError)
}

fn find_node_id(g: &crate::nodes::graph_model::NodeGraphSnapshot, id_or_name: &str) -> Option<Uuid> {
    if let Ok(id) = Uuid::parse_str(id_or_name) {
        if g.nodes.contains_key(&id) {
            return Some(id);
        }
    }
    g.nodes
        .iter()
        .find(|(_, n)| n.name == id_or_name)
        .map(|(id, _)| *id)
}

fn find_sticky_id(g: &crate::nodes::graph_model::NodeGraphSnapshot, id_or_title: &str) -> Option<Uuid> {
    if let Ok(id) = Uuid::parse_str(id_or_title) {
        if g.sticky_notes.contains_key(&id) {
            return Some(id);
        }
    }
    g.sticky_notes
        .iter()
        .find(|(_, n)| n.title == id_or_title)
        .map(|(id, _)| *id)
}

fn resolve_node_type(label: &str) -> NodeType {
    match label.trim() {
        "Create Cube" | "CreateCube" | "cube" | "Cube" => NodeType::CreateCube,
        "Create Sphere" | "CreateSphere" | "sphere" | "Sphere" => NodeType::CreateSphere,
        "Transform" => NodeType::Transform,
        "Merge" => NodeType::Merge,
        "Boolean" => NodeType::Boolean,
        "Curve" | "Curve (Plugin)" => NodeType::Generic("Curve".to_string()),
        "FBX Import" | "FBX Importer" => NodeType::FbxImporter,
        "SDF From Polygons" => NodeType::VdbFromPolygons,
        "SDF To Polygons" => NodeType::VdbToPolygons,
        other => NodeType::Generic(other.to_string()),
    }
}

fn normalize_port_name(port: Option<&str>, is_output: bool) -> String {
    let p = port.unwrap_or(if is_output { "out:0" } else { "in:0" }).trim();
    let lc = p.to_lowercase();
    match (is_output, lc.as_str()) {
        (true, "output") | (true, "out") | (true, "out0") | (true, "out:0") => "out:0".to_string(),
        (false, "input") | (false, "in") | (false, "in0") | (false, "in:0") => "in:0".to_string(),
        _ => p.to_string(),
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
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
    if n.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, &String)> = candidates
        .iter()
        .map(|c| (levenshtein(&n, &c.to_lowercase()), c))
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().take(k).map(|(_, s)| s.clone()).collect()
}

fn auto_place_rect_snap(
    g: &crate::nodes::graph_model::NodeGraphSnapshot,
    desired: Rect,
    pad: f32,
) -> Rect {
    let blocked = |r: Rect| -> bool {
        for n in g.nodes.values() {
            if Rect::from_min_size(n.position, n.size).expand(pad).intersects(r) {
                return true;
            }
        }
        for s in g.sticky_notes.values() {
            if s.rect.expand(pad).intersects(r) {
                return true;
            }
        }
        for b in g.network_boxes.values() {
            if b.rect.expand(pad).intersects(r) {
                return true;
            }
        }
        false
    };
    if !blocked(desired) {
        return desired;
    }
    let step = 72.0;
    let c0 = desired.center();
    let sz = desired.size();
    for r in 1i32..=16 {
        for ox in -r..=r {
            for oy in -r..=r {
                if ox.abs() != r && oy.abs() != r {
                    continue;
                }
                let c = c0 + EVec2::new(ox as f32 * step, oy as f32 * step);
                let cand = Rect::from_center_size(c, sz);
                if !blocked(cand) {
                    return cand;
                }
            }
        }
    }
    desired
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
    #[serde(default)]
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
                "type":"object",
                "properties":{
                    "node_type":{"type":"string"},
                    "node_name":{"type":"string"}
                },
                "required":["node_type"]
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
            if matches!(&node_type, NodeType::Generic(s) if s == "ForEach Begin" || s == "ForEach End") {
                let spec = build_foreach_block(graph, &registry, None, Pos2::new(0.0, 0.0), false);
                let ids = apply_foreach_block_direct(graph, spec);
                for id in ids {
                    graph.mark_dirty(id);
                }
                return crate::nodes::graph_model::GraphCommandEffect { graph_changed: true, geometry_changed: true };
            }

            let mut node = Node::new(node_id, name_for_closure.clone(), node_type.clone(), Pos2::new(0.0, 0.0));
            if let NodeType::Generic(type_name) = &node.node_type {
                if let Some(desc) = registry.nodes.read().unwrap().get(type_name) {
                    node.parameters = (desc.parameters_factory)();
                    node.inputs = desc
                        .inputs
                        .iter()
                        .map(|s| (PortId::from(s.as_str()), ()))
                        .collect();
                    node.outputs = desc
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
// create_sticky_note
// -----------------------------------------------------------------------------

pub struct CreateStickyNoteTool;
impl CreateStickyNoteTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Serialize, Deserialize)]
struct CreateStickyNoteArgs {
    #[serde(default)]
    title: Option<String>,
    content: String,
    #[serde(default)]
    anchor_nodes: Option<Vec<String>>,
    #[serde(default)]
    size: Option<[f32; 2]>,
    #[serde(default)]
    color_rgba: Option<[u8; 4]>,
}

impl Tool for CreateStickyNoteTool {
    fn name(&self) -> &str {
        "create_sticky_note"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Create a sticky note annotation in the node graph.".to_string(),
            parameters: json!({
                "type":"object",
                "properties":{
                    "title":{"type":"string"},
                    "content":{"type":"string"},
                    "anchor_nodes":{"type":"array","items":{"type":"string"}},
                    "size":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},
                    "color_rgba":{"type":"array","items":{"type":"integer"},"minItems":4,"maxItems":4}
                },
                "required":["content"]
            }),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: CreateStickyNoteArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let snap = snapshot()?;
        let id = Uuid::new_v4();
        let title = args.title.unwrap_or_else(|| "Sticky Note".to_string());
        let content = args.content;
        let sz = args.size.unwrap_or([320.0, 180.0]);
        let pad = 18.0;
        let c = args.color_rgba.unwrap_or([255, 243, 138, 255]);
        let color = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);

        let anchor = args.anchor_nodes.unwrap_or_default();
        let mut anchor_center: Option<Pos2> = None;
        if !anchor.is_empty() {
            let mut min = Pos2::new(f32::MAX, f32::MAX);
            let mut max = Pos2::new(f32::MIN, f32::MIN);
            let mut any = false;
            for a in &anchor {
                if let Some(nid) = find_node_id(&snap, a) {
                    if let Some(n) = snap.nodes.get(&nid) {
                        let r = Rect::from_min_size(n.position, n.size);
                        min.x = min.x.min(r.min.x);
                        min.y = min.y.min(r.min.y);
                        max.x = max.x.max(r.max.x);
                        max.y = max.y.max(r.max.y);
                        any = true;
                    }
                }
            }
            if any {
                anchor_center = Some(Rect::from_min_max(min, max).center());
            }
        }
        let desired_center = anchor_center.unwrap_or(Pos2::new(0.0, 0.0));
        let desired = Rect::from_center_size(
            Pos2::new(desired_center.x, desired_center.y - (sz[1] * 0.5 + 56.0)),
            EVec2::new(sz[0], sz[1]),
        );
        let placed = auto_place_rect_snap(&snap, desired, pad);

        enqueue(Box::new(move |g: &mut NodeGraph| {
            g.sticky_notes.insert(
                id,
                crate::nodes::StickyNote {
                    id,
                    rect: placed,
                    title,
                    content,
                    color,
                },
            );
            if !g.sticky_note_draw_order.contains(&id) {
                g.sticky_note_draw_order.push(id);
            }
            crate::nodes::graph_model::GraphCommandEffect { graph_changed: true, geometry_changed: true }
        }))?;

        Ok(ToolOutput::new(
            format!("Queued create_sticky_note ({id})."),
            vec![ToolLog {
                message: format!("create_sticky_note queued: {id}"),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

// -----------------------------------------------------------------------------
// create_network_box
// -----------------------------------------------------------------------------

pub struct CreateNetworkBoxTool;
impl CreateNetworkBoxTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Serialize, Deserialize)]
struct CreateNetworkBoxArgs {
    title: String,
    #[serde(default)]
    nodes_inside: Option<Vec<String>>,
    #[serde(default)]
    stickies_inside: Option<Vec<String>>,
    #[serde(default)]
    pad: Option<f32>,
    #[serde(default)]
    color_rgba: Option<[u8; 4]>,
}

impl Tool for CreateNetworkBoxTool {
    fn name(&self) -> &str {
        "create_network_box"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Create a network box to group nodes/stickies.".to_string(),
            parameters: json!({
                "type":"object",
                "properties":{
                    "title":{"type":"string"},
                    "nodes_inside":{"type":"array","items":{"type":"string"}},
                    "stickies_inside":{"type":"array","items":{"type":"string"}},
                    "pad":{"type":"number"},
                    "color_rgba":{"type":"array","items":{"type":"integer"},"minItems":4,"maxItems":4}
                },
                "required":["title"]
            }),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: CreateNetworkBoxArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let snap = snapshot()?;

        let id = Uuid::new_v4();
        let pad = args.pad.unwrap_or(40.0).max(0.0);
        let c = args.color_rgba.unwrap_or([50, 50, 80, 100]);
        let color = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);

        let mut nodes_set: HashSet<Uuid> = HashSet::new();
        for n in args.nodes_inside.unwrap_or_default() {
            if let Some(id) = find_node_id(&snap, &n) {
                nodes_set.insert(id);
            }
        }
        let mut stickies_set: HashSet<Uuid> = HashSet::new();
        for s in args.stickies_inside.unwrap_or_default() {
            if let Some(id) = find_sticky_id(&snap, &s) {
                stickies_set.insert(id);
            }
        }

        let mut min = Pos2::new(f32::MAX, f32::MAX);
        let mut max = Pos2::new(f32::MIN, f32::MIN);
        let mut any = false;
        for nid in &nodes_set {
            if let Some(n) = snap.nodes.get(nid) {
                let r = Rect::from_min_size(n.position, n.size);
                min.x = min.x.min(r.min.x);
                min.y = min.y.min(r.min.y);
                max.x = max.x.max(r.max.x);
                max.y = max.y.max(r.max.y);
                any = true;
            }
        }
        for sid in &stickies_set {
            if let Some(s) = snap.sticky_notes.get(sid) {
                let r = s.rect;
                min.x = min.x.min(r.min.x);
                min.y = min.y.min(r.min.y);
                max.x = max.x.max(r.max.x);
                max.y = max.y.max(r.max.y);
                any = true;
            }
        }

        let desired = if any {
            Rect::from_min_max(min, max).expand(pad)
        } else {
            Rect::from_min_size(Pos2::new(0.0, 0.0), EVec2::new(360.0, 220.0))
        };
        let placed = auto_place_rect_snap(&snap, desired, 18.0);
        let title = args.title;

        enqueue(Box::new(move |g: &mut NodeGraph| {
            g.network_boxes.insert(
                id,
                crate::nodes::NetworkBox {
                    id,
                    rect: placed,
                    title,
                    color,
                    nodes_inside: nodes_set,
                    stickies_inside: stickies_set,
                },
            );
            if !g.network_box_draw_order.contains(&id) {
                g.network_box_draw_order.push(id);
            }
            crate::nodes::graph_model::GraphCommandEffect { graph_changed: true, geometry_changed: true }
        }))?;

        Ok(ToolOutput::new(
            format!("Queued create_network_box ({id})."),
            vec![ToolLog {
                message: format!("create_network_box queued: {id}"),
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
                "type":"object",
                "properties":{"node_name_or_id":{"type":"string"}},
                "required":["node_name_or_id"]
            }),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: DeleteNodeArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let graph = snapshot()?;
        let all: Vec<String> = graph.nodes.values().map(|n| n.name.clone()).collect();
        let target_id = find_node_id(&graph, &args.node_name_or_id).ok_or_else(|| {
            let sug = suggest_names(&args.node_name_or_id, &all, 4);
            if sug.is_empty() {
                ToolError(format!("Node not found: {}", args.node_name_or_id))
            } else {
                ToolError(format!(
                    "Node not found: {}. Suggestions: {:?}",
                    args.node_name_or_id, sug
                ))
            }
        })?;
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
// connect_nodes (self-healing: validate + ensure visible)
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
    #[serde(default)]
    from_port: Option<String>,
    to_node: String,
    #[serde(default)]
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
                "type":"object",
                "properties":{
                    "from_node":{"type":"string"},
                    "from_port":{"type":"string"},
                    "to_node":{"type":"string"},
                    "to_port":{"type":"string"}
                },
                "required":["from_node","to_node"]
            }),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: ConnectNodeArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let graph = snapshot()?;
        let all_node_names: Vec<String> = graph.nodes.values().map(|n| n.name.clone()).collect();

        let from_id = find_node_id(&graph, &args.from_node).ok_or_else(|| {
            let sug = suggest_names(&args.from_node, &all_node_names, 4);
            if sug.is_empty() {
                ToolError(format!("Source node not found: {}", args.from_node))
            } else {
                ToolError(format!(
                    "Source node not found: {}. Suggestions: {:?}",
                    args.from_node, sug
                ))
            }
        })?;
        let to_id = find_node_id(&graph, &args.to_node).ok_or_else(|| {
            let sug = suggest_names(&args.to_node, &all_node_names, 4);
            if sug.is_empty() {
                ToolError(format!("Target node not found: {}", args.to_node))
            } else {
                ToolError(format!(
                    "Target node not found: {}. Suggestions: {:?}",
                    args.to_node, sug
                ))
            }
        })?;

        let from_node = graph
            .nodes
            .get(&from_id)
            .ok_or_else(|| ToolError("Source node missing in snapshot".to_string()))?;
        let to_node = graph
            .nodes
            .get(&to_id)
            .ok_or_else(|| ToolError("Target node missing in snapshot".to_string()))?;

        let from_port = normalize_port_name(args.from_port.as_deref(), true);
        let to_port = normalize_port_name(args.to_port.as_deref(), false);
        let from_pid = PortId::from(from_port.as_str());
        let to_pid = PortId::from(to_port.as_str());

        if !from_node.outputs.contains_key(&from_pid) {
            let mut avail: Vec<String> = from_node.outputs.keys().map(|p| p.to_string()).collect();
            avail.sort();
            let sug = suggest_names(&from_port, &avail, 4);
            return Err(ToolError(format!(
                "Invalid from_port '{}' for node '{}'. Available outputs: {:?}. Suggestions: {:?}",
                from_port, from_node.name, avail, sug
            )));
        }
        if !to_node.inputs.contains_key(&to_pid) {
            let mut avail: Vec<String> = to_node.inputs.keys().map(|p| p.to_string()).collect();
            avail.sort();
            let sug = suggest_names(&to_port, &avail, 4);
            return Err(ToolError(format!(
                "Invalid to_port '{}' for node '{}'. Available inputs: {:?}. Suggestions: {:?}",
                to_port, to_node.name, avail, sug
            )));
        }

        if graph.connections.values().any(|c| {
            c.from_node == from_id && c.to_node == to_id && c.from_port == from_pid && c.to_port == to_pid
        }) {
            return Ok(ToolOutput::new(
                "Already connected.".to_string(),
                vec![ToolLog {
                    message: "connect_nodes skipped (already connected)".into(),
                    level: ToolLogLevel::Info,
                }],
            ));
        }

        let connection_id = Uuid::new_v4();
        enqueue(Box::new(move |g: &mut NodeGraph| {
            g.connections.insert(
                connection_id,
                crate::nodes::Connection {
                    id: connection_id,
                    from_node: from_id,
                    from_port: from_pid,
                    to_node: to_id,
                    to_port: to_pid,
                    order: 0,
                    waypoints: Vec::new(),
                },
            );
            g.mark_dirty(to_id);
            crate::nodes::graph_model::GraphCommandEffect { graph_changed: true, geometry_changed: true }
        }))?;

        // Ensure: wait until snapshot shows the connection, or return an actionable error.
        const ENSURE_TIMEOUT_MS: u64 = 1200;
        let deadline = Instant::now() + Duration::from_millis(ENSURE_TIMEOUT_MS);
        loop {
            if let Ok(snap) = snapshot() {
                if snap.connections.values().any(|c| {
                    c.from_node == from_id
                        && c.to_node == to_id
                        && c.from_port == from_pid
                        && c.to_port == to_pid
                }) {
                    break;
                }
            }
            if Instant::now() >= deadline {
                return Err(ToolError(format!(
                    "connect_nodes queued but not yet visible in snapshot after {}ms; call get_graph_state to verify, then retry connect_nodes if missing.",
                    ENSURE_TIMEOUT_MS
                )));
            }
            std::thread::sleep(Duration::from_millis(12));
        }

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
        let all_node_names: Vec<String> = graph.nodes.values().map(|n| n.name.clone()).collect();
        let node_id = find_node_id(&graph, &args.node_name).ok_or_else(|| {
            let sug = suggest_names(&args.node_name, &all_node_names, 4);
            if sug.is_empty() {
                ToolError(format!("Node not found: {}", args.node_name))
            } else {
                ToolError(format!("Node not found: {}. Suggestions: {:?}", args.node_name, sug))
            }
        })?;
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
// set_parameter (self-healing errors)
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
        let all_node_names: Vec<String> = graph.nodes.values().map(|n| n.name.clone()).collect();
        let node_id = find_node_id(&graph, &args.node_name).ok_or_else(|| {
            let sug = suggest_names(&args.node_name, &all_node_names, 4);
            if sug.is_empty() {
                ToolError(format!("Node not found: {}", args.node_name))
            } else {
                ToolError(format!("Node not found: {}. Suggestions: {:?}", args.node_name, sug))
            }
        })?;
        let node = graph
            .nodes
            .get(&node_id)
            .ok_or_else(|| ToolError("Node not found in snapshot".to_string()))?;

        let current_val = node.parameters.iter().find(|p| p.name == args.param_name).map(|p| &p.value);
        let current_val = match current_val {
            Some(v) => v,
            None => {
                let mut avail: Vec<String> = node.parameters.iter().map(|p| p.name.clone()).collect();
                avail.sort();
                let sug = suggest_names(&args.param_name, &avail, 4);
                return Err(ToolError(format!(
                    "Parameter not found: '{}' on node '{}'. Available parameters: {:?}. Suggestions: {:?}",
                    args.param_name, node.name, avail, sug
                )));
            }
        };

        let new_val = match current_val {
            ParameterValue::Float(_) => {
                let v: f64 = serde_json::from_value(args.value.clone()).map_err(|_| {
                    ToolError(format!("Expected float value for '{}.{}'", node.name, args.param_name))
                })?;
                ParameterValue::Float(v as f32)
            }
            ParameterValue::Int(_) => {
                let v: i64 = serde_json::from_value(args.value.clone()).map_err(|_| {
                    ToolError(format!(
                        "Expected integer value for '{}.{}'",
                        node.name, args.param_name
                    ))
                })?;
                ParameterValue::Int(v as i32)
            }
            ParameterValue::Bool(_) => {
                let v: bool = serde_json::from_value(args.value.clone()).map_err(|_| {
                    ToolError(format!(
                        "Expected boolean value for '{}.{}'",
                        node.name, args.param_name
                    ))
                })?;
                ParameterValue::Bool(v)
            }
            ParameterValue::String(_) => {
                let v: String = serde_json::from_value(args.value.clone()).map_err(|_| {
                    ToolError(format!(
                        "Expected string value for '{}.{}'",
                        node.name, args.param_name
                    ))
                })?;
                ParameterValue::String(v)
            }
            ParameterValue::Vec2(_) => {
                let v: [f32; 2] = serde_json::from_value(args.value.clone()).map_err(|_| {
                    ToolError(format!("Expected Vec2 [x,y] for '{}.{}'", node.name, args.param_name))
                })?;
                ParameterValue::Vec2(Vec2::new(v[0], v[1]))
            }
            ParameterValue::Vec3(_) => {
                let v: [f32; 3] = serde_json::from_value(args.value.clone()).map_err(|_| {
                    ToolError(format!(
                        "Expected Vec3 [x,y,z] for '{}.{}'",
                        node.name, args.param_name
                    ))
                })?;
                ParameterValue::Vec3(Vec3::new(v[0], v[1], v[2]))
            }
            ParameterValue::IVec2(_) => {
                let v: [i32; 2] = serde_json::from_value(args.value.clone()).map_err(|_| {
                    ToolError(format!(
                        "Expected IVec2 [x,y] for '{}.{}'",
                        node.name, args.param_name
                    ))
                })?;
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
        let g = snapshot()?;
        let nodes: Vec<_> = g
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
        let connections: Vec<_> = g
            .connections
            .values()
            .map(|c| {
                json!({
                    "id": c.id.to_string(),
                    "from": c.from_node.to_string(),
                    "from_port": c.from_port.to_string(),
                    "to": c.to_node.to_string(),
                    "to_port": c.to_port.to_string()
                })
            })
            .collect();
        let state = json!({
            "node_count": g.nodes.len(),
            "connection_count": g.connections.len(),
            "graph_revision": g.graph_revision,
            "param_revision": g.param_revision,
            "display_node": g.display_node.map(|id| id.to_string()),
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
    node_ref: String,
}

impl Tool for GetNodeInfoTool {
    fn name(&self) -> &str {
        "get_node_info"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Get node info (instance if exists in graph, otherwise type info from registry).".to_string(),
            parameters: json!({"type":"object","properties":{"node_ref":{"type":"string"}},"required":["node_ref"]}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: GetNodeInfoArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let snap = snapshot()?;
        if let Some(id) = find_node_id(&snap, &args.node_ref) {
            let Some(n) = snap.nodes.get(&id) else {
                return Err(ToolError("Node missing in snapshot".to_string()));
            };
            let mut params: Vec<String> = n.parameters.iter().map(|p| p.name.clone()).collect();
            params.sort();
            let mut ins: Vec<String> = n.inputs.keys().map(|p| p.to_string()).collect();
            let mut outs: Vec<String> = n.outputs.keys().map(|p| p.to_string()).collect();
            ins.sort();
            outs.sort();
            let out = json!({
                "scope":"instance",
                "id": n.id.to_string(),
                "name": n.name,
                "type": n.node_type.name(),
                "inputs": ins,
                "outputs": outs,
                "parameters": params
            });
            return Ok(ToolOutput::new(
                serde_json::to_string_pretty(&out).unwrap_or_else(|_| "{}".to_string()),
                vec![ToolLog { message: "Read node instance".into(), level: ToolLogLevel::Info }],
            ));
        }

        let ty = resolve_node_type(&args.node_ref);
        let type_name = ty.name().to_string();
        if let NodeType::Generic(s) = &ty {
            if let Some(desc) = self.registry.nodes.read().unwrap().get(s) {
                let mut inputs = desc.inputs.clone();
                let mut outputs = desc.outputs.clone();
                inputs.sort();
                outputs.sort();
                let params = (desc.parameters_factory)()
                    .into_iter()
                    .map(|p| p.name)
                    .collect::<Vec<_>>();
                let out = json!({
                    "scope":"type",
                    "name": desc.name,
                    "display_name": desc.display_name,
                    "category": desc.category,
                    "origin": format!("{:?}", desc.origin),
                    "inputs": inputs,
                    "outputs": outputs,
                    "parameters": params
                });
                return Ok(ToolOutput::new(
                    serde_json::to_string_pretty(&out).unwrap_or_else(|_| "{}".to_string()),
                    vec![ToolLog { message: "Read node type".into(), level: ToolLogLevel::Info }],
                ));
            }
        }
        let out = json!({
            "scope":"type",
            "name": type_name,
            "display_name": type_name,
            "category": "BuiltIn",
            "inputs": [],
            "outputs": [],
            "parameters": []
        });
        Ok(ToolOutput::new(
            serde_json::to_string_pretty(&out).unwrap_or_else(|_| "{}".to_string()),
            vec![ToolLog { message: "Read node type (fallback)".into(), level: ToolLogLevel::Info }],
        ))
    }
}

// -----------------------------------------------------------------------------
// get_node_library
// -----------------------------------------------------------------------------

pub struct GetNodeLibraryTool {
    registry: Arc<NodeRegistry>,
}
impl GetNodeLibraryTool {
    pub fn new(registry: Arc<NodeRegistry>) -> Self {
        Self { registry }
    }
}

impl Tool for GetNodeLibraryTool {
    fn name(&self) -> &str {
        "get_node_library"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "List node library categories and nodes.".to_string(),
            parameters: json!({"type":"object","properties":{},"required":[]}),
        }
    }
    fn execute(&self, _args: Value) -> Result<ToolOutput, ToolError> {
        let map = self.registry.nodes.read().unwrap();
        let mut cats: HashMap<String, Vec<String>> = HashMap::new();
        for d in map.values() {
            cats.entry(d.category.clone()).or_default().push(d.display_name.clone());
        }
        for v in cats.values_mut() {
            v.sort();
            v.dedup();
        }
        let mut keys: Vec<String> = cats.keys().cloned().collect();
        keys.sort();
        let out = keys
            .into_iter()
            .map(|k| (k.clone(), cats.remove(&k).unwrap_or_default()))
            .collect::<HashMap<_, _>>();
        Ok(ToolOutput::new(
            serde_json::to_string_pretty(&out).unwrap_or_else(|_| "{}".to_string()),
            vec![ToolLog { message: "Listed node library".into(), level: ToolLogLevel::Info }],
        ))
    }
}

// -----------------------------------------------------------------------------
// edit_node_graph (batch, minimal)
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
        let reg = self.registry.clone();
        enqueue(Box::new(move |g: &mut NodeGraph| {
            let mut aliases: HashMap<String, Uuid> = HashMap::new();
            let resolve_live = |g: &NodeGraph, aliases: &HashMap<String, Uuid>, t: &str| -> Option<Uuid> {
                if t == "display" { return g.display_node; }
                if let Some(id) = aliases.get(t).copied() { return Some(id); }
                if let Ok(id) = Uuid::parse_str(t) { return g.nodes.contains_key(&id).then_some(id); }
                g.nodes.iter().find(|(_, n)| n.name == t).map(|(id, _)| *id)
            };
            for op in ops {
                match op {
                    GraphEditOp::CreateNode { node_type, alias, node_name, position } => {
                        let ty = resolve_node_type(&node_type);
                        let id = Uuid::new_v4();
                        let name = node_name.unwrap_or_else(|| ty.name().to_string());
                        let pos = position
                            .map(|p| Pos2::new(p[0], p[1]))
                            .unwrap_or_else(|| Pos2::new(0.0, 0.0));
                        let mut node = Node::new(id, name, ty.clone(), pos);
                        if let NodeType::Generic(type_name) = &node.node_type {
                            if let Some(desc) = reg.nodes.read().unwrap().get(type_name) {
                                node.parameters = (desc.parameters_factory)();
                                node.inputs = desc.inputs.iter().map(|s| (PortId::from(s.as_str()), ())).collect();
                                node.outputs = desc.outputs.iter().map(|s| (PortId::from(s.as_str()), ())).collect();
                                node.rebuild_ports();
                            }
                        }
                        g.nodes.insert(id, node);
                        g.mark_dirty(id);
                        if let Some(a) = alias {
                            aliases.insert(a, id);
                        }
                    }
                    GraphEditOp::DeleteNode { target } => {
                        if let Some(id) = resolve_live(&*g, &aliases, &target) {
                            g.nodes.remove(&id);
                            g.connections.retain(|_, c| c.from_node != id && c.to_node != id);
                            if g.display_node == Some(id) {
                                g.display_node = None;
                            }
                        }
                    }
                    GraphEditOp::Connect { from, to } => {
                        let Some(from_id) = resolve_live(&*g, &aliases, &from.target) else { continue; };
                        let Some(to_id) = resolve_live(&*g, &aliases, &to.target) else { continue; };
                        let from_port = normalize_port_name(from.port.as_deref(), true);
                        let to_port = normalize_port_name(to.port.as_deref(), false);
                        let from_pid = PortId::from(from_port.as_str());
                        let to_pid = PortId::from(to_port.as_str());
                        if g.connections.values().any(|c| {
                            c.from_node == from_id && c.to_node == to_id && c.from_port == from_pid && c.to_port == to_pid
                        }) {
                            continue;
                        }
                        let conn_id = Uuid::new_v4();
                        g.connections.insert(conn_id, crate::nodes::Connection {
                            id: conn_id,
                            from_node: from_id,
                            from_port: from_pid,
                            to_node: to_id,
                            to_port: to_pid,
                            order: 0,
                            waypoints: Vec::new(),
                        });
                        g.mark_dirty(to_id);
                    }
                    GraphEditOp::Disconnect { from, to } => {
                        let Some(from_id) = resolve_live(&*g, &aliases, &from.target) else { continue; };
                        let Some(to_id) = resolve_live(&*g, &aliases, &to.target) else { continue; };
                        let from_port = normalize_port_name(from.port.as_deref(), true);
                        let to_port = normalize_port_name(to.port.as_deref(), false);
                        let from_pid = PortId::from(from_port.as_str());
                        let to_pid = PortId::from(to_port.as_str());
                        g.connections.retain(|_, c| {
                            !(c.from_node == from_id && c.to_node == to_id && c.from_port == from_pid && c.to_port == to_pid)
                        });
                        g.mark_dirty(to_id);
                    }
                    GraphEditOp::SetParam { target, param, value } => {
                        let Some(id) = resolve_live(&*g, &aliases, &target) else { continue; };
                        if let Some(n) = g.nodes.get_mut(&id) {
                            if let Some(p) = n.parameters.iter_mut().find(|p| p.name == param) {
                                // Best-effort: reuse SetParameterTool semantics by matching current type.
                                match &p.value {
                                    ParameterValue::Float(_) => {
                                        if let Ok(v) = serde_json::from_value::<f64>(value.clone()) {
                                            p.value = ParameterValue::Float(v as f32);
                                        }
                                    }
                                    ParameterValue::Int(_) => {
                                        if let Ok(v) = serde_json::from_value::<i64>(value.clone()) {
                                            p.value = ParameterValue::Int(v as i32);
                                        }
                                    }
                                    ParameterValue::Bool(_) => {
                                        if let Ok(v) = serde_json::from_value::<bool>(value.clone()) {
                                            p.value = ParameterValue::Bool(v);
                                        }
                                    }
                                    ParameterValue::String(_) => {
                                        if let Ok(v) = serde_json::from_value::<String>(value.clone()) {
                                            p.value = ParameterValue::String(v);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            g.mark_dirty(id);
                        }
                    }
                    GraphEditOp::SetFlag { target, flag, value } => {
                        let Some(id) = resolve_live(&*g, &aliases, &target) else { continue; };
                        match flag.as_str() {
                            "display" => {
                                if value {
                                    g.display_node = Some(id);
                                    for (nid, nn) in g.nodes.iter_mut() { nn.is_display_node = *nid == id; }
                                    g.mark_dirty(id);
                                } else {
                                    if g.display_node == Some(id) { g.display_node = None; }
                                    if let Some(n) = g.nodes.get_mut(&id) { n.is_display_node = false; }
                                    g.mark_dirty(id);
                                }
                            }
                            "bypass" | "lock" | "template" => {
                                if let Some(n) = g.nodes.get_mut(&id) {
                                    match flag.as_str() {
                                        "bypass" => n.is_bypassed = value,
                                        "lock" => n.is_locked = value,
                                        "template" => n.is_template = value,
                                        _ => {}
                                    }
                                    g.mark_dirty(id);
                                }
                            }
                            _ => {}
                        }
                    }
                    GraphEditOp::SetDisplay { target } => {
                        let Some(id) = resolve_live(&*g, &aliases, &target) else { continue; };
                        g.display_node = Some(id);
                        for (nid, n) in g.nodes.iter_mut() {
                            n.is_display_node = *nid == id;
                        }
                        g.mark_dirty(id);
                    }
                    GraphEditOp::SetPosition { target, position } => {
                        let Some(id) = resolve_live(&*g, &aliases, &target) else { continue; };
                        if let Some(n) = g.nodes.get_mut(&id) {
                            n.position = Pos2::new(position[0], position[1]);
                            g.mark_dirty(id);
                        }
                    }
                    GraphEditOp::RenameNode { target, new_name } => {
                        let Some(id) = resolve_live(&*g, &aliases, &target) else { continue; };
                        if let Some(n) = g.nodes.get_mut(&id) {
                            n.name = new_name;
                            g.mark_dirty(id);
                        }
                    }
                }
            }
            crate::nodes::graph_model::GraphCommandEffect { graph_changed: true, geometry_changed: true }
        }))?;
        Ok(ToolOutput::new(
            format!("Queued edit_node_graph ops={ops_len}."),
            vec![ToolLog { message: "edit_node_graph queued".into(), level: ToolLogLevel::Success }],
        ))
    }
}

// -----------------------------------------------------------------------------
// Placeholders for full-profile tools (kept lightweight; expand as needed)
// -----------------------------------------------------------------------------

pub struct GetGeometryInsightTool;
impl GetGeometryInsightTool {
    pub fn new() -> Self { Self }
}
impl Tool for GetGeometryInsightTool {
    fn name(&self) -> &str { "get_geometry_insight" }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition { name: self.name().to_string(), description: "Get a compact geometry insight (placeholder).".to_string(), parameters: json!({"type":"object","properties":{},"required":[]}) }
    }
    fn execute(&self, _args: Value) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::simple("Geometry insight not implemented in this build.".to_string()))
    }
}

pub struct ExportNodeSpecTool { registry: Arc<NodeRegistry> }
impl ExportNodeSpecTool { pub fn new(registry: Arc<NodeRegistry>) -> Self { Self { registry } } }
#[derive(Serialize, Deserialize)]
struct ExportNodeSpecArgs { node_type: String }
impl Tool for ExportNodeSpecTool {
    fn name(&self) -> &str { "export_node_spec" }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Export a minimal node spec for a node type.".to_string(),
            parameters: json!({"type":"object","properties":{"node_type":{"type":"string"}},"required":["node_type"]}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: ExportNodeSpecArgs = serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let ty = resolve_node_type(&args.node_type);
        if let NodeType::Generic(s) = &ty {
            if let Some(desc) = self.registry.nodes.read().unwrap().get(s) {
                let spec = json!({
                    "name": desc.name,
                    "display_name": desc.display_name,
                    "category": desc.category,
                    "inputs": desc.inputs,
                    "outputs": desc.outputs,
                    "parameters": (desc.parameters_factory)().into_iter().map(|p| p.name).collect::<Vec<_>>(),
                    "origin": format!("{:?}", desc.origin),
                });
                return Ok(ToolOutput::new(serde_json::to_string_pretty(&spec).unwrap_or_else(|_| "{}".to_string()), vec![]));
            }
        }
        Ok(ToolOutput::simple(format!("No registry spec for type '{}'.", args.node_type)))
    }
}

pub struct CompareGeometryTool;
impl CompareGeometryTool { pub fn new() -> Self { Self } }
#[derive(Serialize, Deserialize)]
struct CompareGeometryArgs { a: String, b: String }
impl Tool for CompareGeometryTool {
    fn name(&self) -> &str { "compare_geometry" }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Compare cached geometry stats between two nodes (best-effort).".to_string(),
            parameters: json!({"type":"object","properties":{"a":{"type":"string"},"b":{"type":"string"}},"required":["a","b"]}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let args: CompareGeometryArgs = serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid arguments: {e}")))?;
        let snap = snapshot()?;
        let ida = find_node_id(&snap, &args.a).ok_or_else(|| ToolError(format!("Node not found: {}", args.a)))?;
        let idb = find_node_id(&snap, &args.b).ok_or_else(|| ToolError(format!("Node not found: {}", args.b)))?;
        let ga = snap.geometry_cache.get(&ida).map(|g| (g.get_point_count(), g.primitives().len(), g.vertices().len(), g.edges().len()));
        let gb = snap.geometry_cache.get(&idb).map(|g| (g.get_point_count(), g.primitives().len(), g.vertices().len(), g.edges().len()));
        let out = json!({ "a": args.a, "b": args.b, "a_stats": ga, "b_stats": gb });
        Ok(ToolOutput::new(serde_json::to_string_pretty(&out).unwrap_or_else(|_| "{}".to_string()), vec![]))
    }
}

