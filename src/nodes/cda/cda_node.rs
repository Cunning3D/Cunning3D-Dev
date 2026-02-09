//! CDA node compute and parameter building
use crate::cunning_core::cda::{CDAAsset, CDAInterface};
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::Geometry;
use crate::node_editor_settings::{resolved_node_size, NodeEditorSettings};
use crate::nodes::parameter::{Parameter, ParameterValue};
use crate::nodes::port_key;
use crate::nodes::structs::{Connection, Node, NodeGraph, NodeId, NodeType};
use crate::nodes::PortId;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
// bevy Vec types not needed here
use crate::console::{global_console, LogLevel};
use crate::cunning_core::cda::runtime_report::CdaRuntimeReport;
use uuid::Uuid;

/// CDA creation result with asset and rewiring info
pub struct CDACreationResult {
    pub asset: CDAAsset,
    /// External input map: (ExternalNodeId, ExternalPort) -> CDA input port name
    pub external_inputs: HashMap<(NodeId, PortId), PortId>,
    /// External output map: (InnerNodeId, InnerPort) -> CDA output port name
    pub external_outputs: HashMap<(NodeId, PortId), PortId>,
}

#[derive(Clone, Debug)]
pub struct PortRename {
    pub is_input: bool,
    pub old: String,
    pub new: String,
}

/// Build UI parameter list from CDA promoted_params
pub fn build_cda_parameters(asset: &CDAAsset) -> Vec<Parameter> {
    let mut params = Vec::new();
    for promoted in asset.get_promoted_params_sorted() {
        let ui_type = crate::cunning_core::cda::utils::promoted_type_to_ui(&promoted.param_type);
        let value = crate::cunning_core::cda::utils::promoted_channels_to_value(
            &promoted.param_type,
            &promoted.channels,
        );
        let mut p = Parameter::new(
            &promoted.name,
            &promoted.label,
            &promoted.group,
            value,
            ui_type,
        );
        if let Some(cond) = &promoted.ui_config.condition {
            p = p.with_condition(cond);
        }
        params.push(p);
    }
    params
}

/// Compute CDA node
pub fn compute_cda(
    asset: &CDAAsset,
    param_overrides: &HashMap<String, ParameterValue>,
    inputs: &[Arc<dyn GeometryRef>],
    registry: &NodeRegistry,
) -> Arc<Geometry> {
    compute_cda_outputs(None, asset, param_overrides, inputs, registry)
        .into_iter()
        .next()
        .unwrap_or_else(|| Arc::new(Geometry::new()))
}

fn log_report(level: LogLevel, r: CdaRuntimeReport) {
    if let Some(c) = global_console() {
        c.log(level, r.to_string());
    }
}

pub fn compute_cda_outputs(
    instance_node_id: Option<NodeId>,
    asset: &CDAAsset,
    param_overrides: &HashMap<String, ParameterValue>,
    inputs: &[Arc<dyn GeometryRef>],
    registry: &NodeRegistry,
) -> Vec<Arc<Geometry>> {
    if let Some(lib) = crate::cunning_core::cda::library::global_cda_library() {
        return lib.cook(
            instance_node_id,
            &crate::cunning_core::cda::CdaAssetRef {
                uuid: asset.id,
                path: String::new(),
            },
            param_overrides,
            inputs,
            registry,
        );
    }

    let _ = registry;
    let Ok(def) = crate::cunning_core::cda::serialization::build_game_chunk(asset) else {
        log_report(
            LogLevel::Error,
            CdaRuntimeReport {
                instance_node_id,
                asset_uuid: asset.id,
                asset_name: asset.name.clone(),
                stage: "export",
                def_node_id: None,
                op: None,
                port: None,
                param: None,
                message: "build_game_chunk failed".to_string(),
            },
        );
        return vec![Arc::new(Geometry::new()); asset.outputs.len().max(1)];
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
    match cunning_cda_runtime::cook(&def, mats.as_slice(), param_overrides, &cancel) {
        Ok(v) => v,
        Err(e) => {
            log_report(
                LogLevel::Error,
                CdaRuntimeReport {
                    instance_node_id,
                    asset_uuid: e.asset_uuid,
                    asset_name: e.asset_name.clone(),
                    stage: "cook",
                    def_node_id: e.node_id,
                    op: e.op,
                    port: e.port.clone(),
                    param: e.param.clone(),
                    message: format!("{:?}", e.kind),
                },
            );
            vec![Arc::new(Geometry::new()); asset.outputs.len().max(1)]
        }
    }
}

/// Create CDA from selected nodes
pub fn create_cda_from_nodes(
    name: &str,
    graph: &NodeGraph,
    selected_nodes: &[NodeId],
    settings: &NodeEditorSettings,
) -> CDACreationResult {
    let mut asset = CDAAsset::new(name);
    let selected_set: HashSet<_> = selected_nodes.iter().copied().collect();
    let default_node_size = resolved_node_size(settings);
    let io_gap_x = settings.cda_io_gap_x.max(0.0);
    let io_margin_y = settings.cda_io_margin_y.max(0.0);
    let cda_input_out = port_key::out0();
    let cda_output_in = port_key::in0();

    // 1. Copy nodes and compute AABB
    let mut min_pos = bevy_egui::egui::Pos2::new(f32::MAX, f32::MAX);
    let mut max_pos = bevy_egui::egui::Pos2::new(f32::MIN, f32::MIN);
    let mut has_nodes = false;

    for &node_id in selected_nodes {
        if let Some(node) = graph.nodes.get(&node_id) {
            asset.inner_graph.nodes.insert(node_id, node.clone());

            min_pos.x = min_pos.x.min(node.position.x);
            min_pos.y = min_pos.y.min(node.position.y);
            max_pos.x = max_pos.x.max(node.position.x + node.size.x);
            max_pos.y = max_pos.y.max(node.position.y + node.size.y);
            has_nodes = true;
        }
    }

    // 2. Copy internal connections
    for conn in graph.connections.values() {
        if selected_set.contains(&conn.from_node) && selected_set.contains(&conn.to_node) {
            asset.inner_graph.connections.insert(conn.id, conn.clone());
        }
    }

    // 3. Set view center
    let center_x = if has_nodes {
        (min_pos.x + max_pos.x) * 0.5
    } else {
        0.0
    };
    let center_y = if has_nodes {
        (min_pos.y + max_pos.y) * 0.5
    } else {
        0.0
    };

    if has_nodes {
        asset.view_center = Some([center_x, center_y]);
        asset.view_zoom = Some(1.0);
    } else {
        asset.view_center = Some([0.0, 0.0]);
        asset.view_zoom = Some(1.0);
        min_pos = bevy_egui::egui::Pos2::ZERO;
        max_pos = bevy_egui::egui::Pos2::new(default_node_size[0], default_node_size[1]);
    }

    // 4. Analyze external connections and generate Input nodes (above AABB, horizontal)
    let mut external_inputs_map: HashMap<(NodeId, PortId), PortId> = HashMap::new();
    let mut next_input_idx = 0;

    // Collect and sort incoming edges (deterministic)
    let mut incoming_edges: Vec<&Connection> = graph
        .connections
        .values()
        .filter(|c| !selected_set.contains(&c.from_node) && selected_set.contains(&c.to_node))
        .collect();
    incoming_edges.sort_by_key(|c| c.id);

    // Temp map: (ExternalNode, ExternalPort) -> InputIndex
    let mut key_to_input_idx: HashMap<(NodeId, PortId), i32> = HashMap::new();

    // Precompute horizontal layout, collect unique inputs
    let mut unique_inputs = Vec::new();
    for conn in &incoming_edges {
        let key = (conn.from_node, conn.from_port);
        if !key_to_input_idx.contains_key(&key) {
            key_to_input_idx.insert(key.clone(), next_input_idx);
            unique_inputs.push(key);
            next_input_idx += 1;
        }
    }

    // Layout: Inputs at top
    let input_pitch_x = default_node_size[0] + io_gap_x;
    let input_start_x = center_x - (unique_inputs.len() as f32 - 1.0) * input_pitch_x * 0.5;
    let input_y = min_pos.y - io_margin_y;
    let mut input_node_by_idx: HashMap<i32, NodeId> = HashMap::new();

    for (i, key) in unique_inputs.iter().enumerate() {
        let idx = key_to_input_idx[key];
        let iface_name = format!("input_{}", idx);
        let node_id = Uuid::new_v4();
        let iface = CDAInterface::new(&iface_name, node_id).with_order(idx);
        external_inputs_map.insert(key.clone(), iface.port_key());
        asset.add_input(iface);

        // Create internal Input node
        let node_name = format!("Input_{}", idx);
        let pos_x = input_start_x + (i as f32) * input_pitch_x;

        let mut input_node = Node::new(
            node_id,
            node_name.clone(),
            NodeType::CDAInput(node_name),
            bevy_egui::egui::Pos2::new(pos_x, input_y),
        );
        input_node.size = bevy_egui::egui::vec2(default_node_size[0], default_node_size[1]);
        asset.inner_graph.nodes.insert(node_id, input_node);
        input_node_by_idx.insert(idx, node_id);
    }

    // Establish Input connections
    for conn in incoming_edges {
        let key = (conn.from_node, conn.from_port);
        let idx = key_to_input_idx[&key];
        let Some(&input_node_id) = input_node_by_idx.get(&idx) else {
            continue;
        };
        let id = Uuid::new_v4();
        asset.inner_graph.connections.insert(
            id,
            Connection {
                id,
                from_node: input_node_id,
                from_port: cda_input_out.clone(),
                to_node: conn.to_node,
                to_port: conn.to_port.clone(),
                order: 0,
                waypoints: Vec::new(),
            },
        );
    }

    // 5. Analyze external connections and generate Output nodes (below AABB, horizontal)
    let mut external_outputs_map: HashMap<(NodeId, PortId), PortId> = HashMap::new();
    let mut next_output_idx = 0;

    // Collect and sort outgoing edges
    let mut outgoing_edges: Vec<&Connection> = graph
        .connections
        .values()
        .filter(|c| selected_set.contains(&c.from_node) && !selected_set.contains(&c.to_node))
        .collect();
    outgoing_edges.sort_by_key(|c| c.id);

    let mut key_to_output_idx: HashMap<(NodeId, PortId), i32> = HashMap::new();
    let mut unique_outputs = Vec::new();

    for conn in &outgoing_edges {
        let key = (conn.from_node, conn.from_port);
        if !key_to_output_idx.contains_key(&key) {
            key_to_output_idx.insert(key.clone(), next_output_idx);
            unique_outputs.push(key);
            next_output_idx += 1;
        }
    }

    // Layout: Outputs at bottom
    let output_pitch_x = default_node_size[0] + io_gap_x;
    let output_start_x = center_x - (unique_outputs.len() as f32 - 1.0) * output_pitch_x * 0.5;
    let output_y = max_pos.y + io_margin_y;
    let mut output_node_by_idx: HashMap<i32, NodeId> = HashMap::new();

    for (i, key) in unique_outputs.iter().enumerate() {
        let idx = key_to_output_idx[key];

        let iface_name = format!("output_{}", idx);
        let node_id = Uuid::new_v4();
        let iface = CDAInterface::new(&iface_name, node_id).with_order(idx);
        external_outputs_map.insert(key.clone(), iface.port_key());
        asset.add_output(iface);

        // Create internal Output node
        let node_name = format!("Output_{}", idx);
        let pos_x = output_start_x + (i as f32) * output_pitch_x;

        let mut output_node = Node::new(
            node_id,
            node_name.clone(),
            NodeType::CDAOutput(node_name),
            bevy_egui::egui::Pos2::new(pos_x, output_y),
        );
        output_node.size = bevy_egui::egui::vec2(default_node_size[0], default_node_size[1]);
        asset.inner_graph.nodes.insert(node_id, output_node);
        output_node_by_idx.insert(idx, node_id);
    }

    // Establish Output connections
    for conn in outgoing_edges {
        let key = (conn.from_node, conn.from_port);
        let idx = key_to_output_idx[&key];
        let Some(&output_node_id) = output_node_by_idx.get(&idx) else {
            continue;
        };
        let id = Uuid::new_v4();
        asset.inner_graph.connections.insert(
            id,
            Connection {
                id,
                from_node: conn.from_node,
                from_port: conn.from_port.clone(),
                to_node: output_node_id,
                to_port: cda_output_in.clone(),
                order: conn.order,
                waypoints: Vec::new(),
            },
        );
    }

    // Default output handling
    if asset.outputs.is_empty() && !selected_nodes.is_empty() {
        if let Some(&last_node_id) = selected_nodes.last() {
            let idx = 0;
            let iface_name = "output_0".to_string();
            let node_id = Uuid::new_v4();
            asset.add_output(CDAInterface::new(&iface_name, node_id).with_order(idx));
            let node_name = "Output_0".to_string();
            let pos_x = center_x;
            let pos_y = max_pos.y + io_margin_y;

            let mut output_node = Node::new(
                node_id,
                node_name.clone(),
                NodeType::CDAOutput(node_name),
                bevy_egui::egui::Pos2::new(pos_x, pos_y),
            );
            output_node.size = bevy_egui::egui::vec2(default_node_size[0], default_node_size[1]);
            asset.inner_graph.nodes.insert(node_id, output_node);

            let last_port = graph
                .nodes
                .get(&last_node_id)
                .map(|n| {
                    if n.outputs.contains_key(&port_key::out0()) {
                        return port_key::out0();
                    }
                    let mut ks: Vec<PortId> = n.outputs.keys().copied().collect();
                    ks.sort_by(|a, b| a.as_str().cmp(b.as_str()));
                    ks.into_iter().next().unwrap_or_else(|| port_key::out0())
                })
                .unwrap_or_else(|| port_key::out0());
            let id = Uuid::new_v4();
            asset.inner_graph.connections.insert(
                id,
                Connection {
                    id,
                    from_node: last_node_id,
                    from_port: last_port,
                    to_node: node_id,
                    to_port: cda_output_in.clone(),
                    order: 0,
                    waypoints: Vec::new(),
                },
            );
        }
    }

    CDACreationResult {
        asset,
        external_inputs: external_inputs_map,
        external_outputs: external_outputs_map,
    }
}

pub fn sync_asset_io_nodes(
    asset: &mut CDAAsset,
    settings: &NodeEditorSettings,
) -> (bool, Vec<PortRename>) {
    let default_node_size = resolved_node_size(settings);
    let io_gap_x = settings.cda_io_gap_x.max(0.0);
    let io_margin_y = settings.cda_io_margin_y.max(0.0);
    let mut changed = false;
    let renames: Vec<PortRename> = Vec::new(); // deprecated: CDA external ports are keyed by iface.id now

    // Ensure unique iface names + internal nodes
    let mut name_used: HashSet<String> = HashSet::new();
    let mut node_used: HashSet<NodeId> = HashSet::new();
    for (idx, iface) in asset.inputs.iter_mut().enumerate() {
        let old = iface.name.clone();
        if iface.name.trim().is_empty() {
            iface.name = format!("input_{}", idx);
            changed = true;
        }
        let base = iface.name.clone();
        if !name_used.insert(base.clone()) {
            let mut k = 1;
            while !name_used.insert(format!("{}_{}", base, k)) {
                k += 1;
            }
            iface.name = format!("{}_{}", base, k);
            changed = true;
        }
        if !node_used.insert(iface.internal_node) {
            iface.internal_node = Uuid::new_v4();
            node_used.insert(iface.internal_node);
            changed = true;
        }
        let _ = old;
    }
    for (idx, iface) in asset.outputs.iter_mut().enumerate() {
        let old = iface.name.clone();
        if iface.name.trim().is_empty() {
            iface.name = format!("output_{}", idx);
            changed = true;
        }
        let base = iface.name.clone();
        if !name_used.insert(base.clone()) {
            let mut k = 1;
            while !name_used.insert(format!("{}_{}", base, k)) {
                k += 1;
            }
            iface.name = format!("{}_{}", base, k);
            changed = true;
        }
        if !node_used.insert(iface.internal_node) {
            iface.internal_node = Uuid::new_v4();
            node_used.insert(iface.internal_node);
            changed = true;
        }
        let _ = old;
    }

    // Track IO node ids that must exist
    let mut keep: HashSet<NodeId> = HashSet::new();
    for i in &asset.inputs {
        keep.insert(i.internal_node);
    }
    for o in &asset.outputs {
        keep.insert(o.internal_node);
    }

    // Ensure IO nodes exist for each interface (create with stable id = internal_node) + correct type
    for (idx, i) in asset.inputs.iter().enumerate() {
        let id = i.internal_node;
        let want_name = format!("Input_{}", idx);
        let want_ty = NodeType::CDAInput(want_name.clone());
        match asset.inner_graph.nodes.get_mut(&id) {
            Some(n) => {
                if !matches!(n.node_type, NodeType::CDAInput(_)) {
                    n.node_type = want_ty;
                    changed = true;
                }
                if n.name != want_name {
                    n.name = want_name;
                    changed = true;
                }
                n.rebuild_ports();
            }
            None => {
                let mut n = Node::new(id, want_name.clone(), want_ty, bevy_egui::egui::Pos2::ZERO);
                n.size = bevy_egui::egui::vec2(default_node_size[0], default_node_size[1]);
                asset.inner_graph.nodes.insert(id, n);
                changed = true;
            }
        }
    }
    for (idx, o) in asset.outputs.iter().enumerate() {
        let id = o.internal_node;
        let want_name = format!("Output_{}", idx);
        let want_ty = NodeType::CDAOutput(want_name.clone());
        match asset.inner_graph.nodes.get_mut(&id) {
            Some(n) => {
                if !matches!(n.node_type, NodeType::CDAOutput(_)) {
                    n.node_type = want_ty;
                    changed = true;
                }
                if n.name != want_name {
                    n.name = want_name;
                    changed = true;
                }
                n.rebuild_ports();
            }
            None => {
                let mut n = Node::new(id, want_name.clone(), want_ty, bevy_egui::egui::Pos2::ZERO);
                n.size = bevy_egui::egui::vec2(default_node_size[0], default_node_size[1]);
                asset.inner_graph.nodes.insert(id, n);
                changed = true;
            }
        }
    }

    // Remove orphan IO nodes (generated) that are no longer referenced by interfaces
    let mut remove_ids: Vec<NodeId> = Vec::new();
    for (id, n) in &asset.inner_graph.nodes {
        if matches!(n.node_type, NodeType::CDAInput(_) | NodeType::CDAOutput(_))
            && !keep.contains(id)
        {
            remove_ids.push(*id);
        }
    }
    if !remove_ids.is_empty() {
        changed = true;
    }
    for id in &remove_ids {
        asset.inner_graph.nodes.remove(id);
    }
    if !remove_ids.is_empty() {
        asset
            .inner_graph
            .connections
            .retain(|_, c| !remove_ids.contains(&c.from_node) && !remove_ids.contains(&c.to_node));
    }

    // Layout IO nodes around non-IO AABB
    let mut min_pos = bevy_egui::egui::Pos2::new(f32::MAX, f32::MAX);
    let mut max_pos = bevy_egui::egui::Pos2::new(f32::MIN, f32::MIN);
    let mut has_body = false;
    for (id, n) in &asset.inner_graph.nodes {
        if keep.contains(id) {
            continue;
        }
        min_pos.x = min_pos.x.min(n.position.x);
        min_pos.y = min_pos.y.min(n.position.y);
        max_pos.x = max_pos.x.max(n.position.x + n.size.x);
        max_pos.y = max_pos.y.max(n.position.y + n.size.y);
        has_body = true;
    }
    if !has_body {
        min_pos = bevy_egui::egui::Pos2::ZERO;
        max_pos = bevy_egui::egui::Pos2::new(default_node_size[0], default_node_size[1]);
    }
    let center_x = (min_pos.x + max_pos.x) * 0.5;

    let in_pitch_x = default_node_size[0] + io_gap_x;
    let in_start_x = center_x - (asset.inputs.len() as f32 - 1.0) * in_pitch_x * 0.5;
    let in_y = min_pos.y - io_margin_y;
    for (i, iface) in asset.inputs.iter().enumerate() {
        if let Some(n) = asset.inner_graph.nodes.get_mut(&iface.internal_node) {
            let nx = in_start_x + i as f32 * in_pitch_x;
            let p = bevy_egui::egui::Pos2::new(nx, in_y);
            if n.position != p
                || n.size.x != default_node_size[0]
                || n.size.y != default_node_size[1]
            {
                n.position = p;
                n.size = bevy_egui::egui::vec2(default_node_size[0], default_node_size[1]);
                changed = true;
            }
        }
    }

    let out_pitch_x = default_node_size[0] + io_gap_x;
    let out_start_x = center_x - (asset.outputs.len() as f32 - 1.0) * out_pitch_x * 0.5;
    let out_y = max_pos.y + io_margin_y;
    for (i, iface) in asset.outputs.iter().enumerate() {
        if let Some(n) = asset.inner_graph.nodes.get_mut(&iface.internal_node) {
            let nx = out_start_x + i as f32 * out_pitch_x;
            let p = bevy_egui::egui::Pos2::new(nx, out_y);
            if n.position != p
                || n.size.x != default_node_size[0]
                || n.size.y != default_node_size[1]
            {
                n.position = p;
                n.size = bevy_egui::egui::vec2(default_node_size[0], default_node_size[1]);
                changed = true;
            }
        }
    }

    (changed, renames)
}
