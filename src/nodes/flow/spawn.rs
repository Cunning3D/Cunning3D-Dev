use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::node_editor_settings::{resolved_node_size, NodeEditorSettings};
use crate::nodes::parameter::ParameterValue;
use crate::nodes::port_key;
use crate::nodes::{
    Connection, ConnectionId, NetworkBox, NetworkBoxId, Node, NodeGraph, NodeId, NodeType,
};
use bevy_egui::egui::{self, Color32, Pos2, Rect, Vec2};
use std::collections::HashSet;
use uuid::Uuid;

pub struct ForEachSpawn {
    pub block_id: String,
    pub nodes: Vec<Node>,
    pub connections: Vec<Connection>,
    pub network_box: NetworkBox,
}

fn next_foreach_block_id(g: &NodeGraph) -> String {
    let mut max_n: i32 = 0;
    for n in g.nodes.values() {
        let Some(p) = n.parameters.iter().find(|p| p.name == "block_id") else {
            continue;
        };
        let ParameterValue::String(s) = &p.value else {
            continue;
        };
        let ss = s.trim();
        if !ss.starts_with("foreach") {
            continue;
        }
        let tail = ss.trim_start_matches("foreach").trim();
        let num: i32 = tail.parse().unwrap_or(0);
        if num > max_n {
            max_n = num;
        }
    }
    format!("foreach{}", max_n + 1)
}

fn fill_generic_params_ports(reg: &NodeRegistry, n: &mut Node, type_name: &str) {
    let Some(desc) = reg.nodes.read().unwrap().get(type_name).cloned() else {
        return;
    };
    n.parameters = (desc.parameters_factory)();
    n.inputs = desc
        .inputs
        .iter()
        .map(|s| (crate::nodes::PortId::from(s.as_str()), ()))
        .collect();
    n.outputs = desc
        .outputs
        .iter()
        .map(|s| (crate::nodes::PortId::from(s.as_str()), ()))
        .collect();
    n.rebuild_ports();
}

pub fn build_foreach_block(
    g: &NodeGraph,
    reg: &NodeRegistry,
    settings: Option<&NodeEditorSettings>,
    pos: Pos2,
    include_meta: bool,
) -> ForEachSpawn {
    let block_id = next_foreach_block_id(g);
    let block_uid = Uuid::new_v4().to_string();
    let sz = settings.map(resolved_node_size);

    let mut begin = Node::new(
        Uuid::new_v4(),
        "ForEach Begin".into(),
        NodeType::Generic("ForEach Begin".into()),
        pos,
    );
    if let Some([w, h]) = sz {
        begin.size = egui::vec2(w, h);
    }
    fill_generic_params_ports(reg, &mut begin, "ForEach Begin");
    if let Some(p) = begin.parameters.iter_mut().find(|p| p.name == "block_id") {
        p.value = ParameterValue::String(block_id.clone());
    }
    if let Some(p) = begin.parameters.iter_mut().find(|p| p.name == "block_uid") {
        p.value = ParameterValue::String(block_uid.clone());
    }

    let end_pos = egui::pos2(pos.x + 260.0, pos.y);
    let mut end = Node::new(
        Uuid::new_v4(),
        "ForEach End".into(),
        NodeType::Generic("ForEach End".into()),
        end_pos,
    );
    if let Some([w, h]) = sz {
        end.size = egui::vec2(w, h);
    }
    fill_generic_params_ports(reg, &mut end, "ForEach End");
    if let Some(p) = end.parameters.iter_mut().find(|p| p.name == "block_id") {
        p.value = ParameterValue::String(block_id.clone());
    }
    if let Some(p) = end.parameters.iter_mut().find(|p| p.name == "block_uid") {
        p.value = ParameterValue::String(block_uid.clone());
    }
    if let Some(p) = end
        .parameters
        .iter_mut()
        .find(|p| p.name == "use_piece_attribute")
    {
        p.value = ParameterValue::Bool(false);
    } // Default: split by primnum/ptnum
    if let Some(p) = end
        .parameters
        .iter_mut()
        .find(|p| p.name == "piece_attribute")
    {
        p.value = ParameterValue::String("class".into());
    }

    let mut meta = None;
    if include_meta {
        let meta_pos = egui::pos2(pos.x, pos.y + 140.0);
        let mut m = Node::new(
            Uuid::new_v4(),
            "ForEach Meta".into(),
            NodeType::Generic("ForEach Meta".into()),
            meta_pos,
        );
        if let Some([w, h]) = sz {
            m.size = egui::vec2(w, h);
        }
        fill_generic_params_ports(reg, &mut m, "ForEach Meta");
        if let Some(p) = m.parameters.iter_mut().find(|p| p.name == "block_id") {
            p.value = ParameterValue::String(block_id.clone());
        }
        if let Some(p) = m.parameters.iter_mut().find(|p| p.name == "block_uid") {
            p.value = ParameterValue::String(block_uid.clone());
        }
        meta = Some(m);
    }

    let mut nodes = vec![begin, end];
    if let Some(m) = meta {
        nodes.push(m);
    }

    let begin_id = nodes[0].id;
    let end_id = nodes[1].id;
    let mut connections = vec![Connection {
        id: ConnectionId::new_v4(),
        from_node: begin_id,
        from_port: port_key::out0(),
        to_node: end_id,
        to_port: port_key::in0(),
        order: 0,
        waypoints: Vec::new(),
    }];
    let _ = include_meta;

    let mut min = Vec2::new(f32::INFINITY, f32::INFINITY);
    let mut max = Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
    for n in &nodes {
        let p = n.position.to_vec2();
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        max.x = max.x.max(p.x + n.size.x);
        max.y = max.y.max(p.y + n.size.y);
    }
    let pad = Vec2::new(30.0, 30.0);
    let rect = Rect::from_min_max((min - pad).to_pos2(), (max + pad).to_pos2());
    let nodes_inside: HashSet<NodeId> = nodes.iter().map(|n| n.id).collect();
    let network_box = NetworkBox {
        id: NetworkBoxId::new_v4(),
        rect,
        title: format!("ForEach {}", block_id),
        color: Color32::from_rgba_unmultiplied(255, 140, 0, 40),
        nodes_inside,
        stickies_inside: HashSet::new(),
    };

    ForEachSpawn {
        block_id,
        nodes,
        connections,
        network_box,
    }
}

pub fn build_foreach_connectivity_block(
    g: &NodeGraph,
    reg: &NodeRegistry,
    settings: Option<&NodeEditorSettings>,
    pos: Pos2,
    include_meta: bool,
) -> ForEachSpawn {
    let mut spec = build_foreach_block(
        g,
        reg,
        settings,
        egui::pos2(pos.x + 260.0, pos.y),
        include_meta,
    );
    let sz = settings.map(resolved_node_size);

    let mut conn = Node::new(
        Uuid::new_v4(),
        "Connectivity".into(),
        NodeType::Generic("Connectivity".into()),
        pos,
    );
    if let Some([w, h]) = sz {
        conn.size = egui::vec2(w, h);
    }
    fill_generic_params_ports(reg, &mut conn, "Connectivity");
    if let Some(p) = conn
        .parameters
        .iter_mut()
        .find(|p| p.name == "connectivity_type")
    {
        p.value = ParameterValue::Int(1);
    }
    if let Some(p) = conn.parameters.iter_mut().find(|p| p.name == "attribute") {
        p.value = ParameterValue::String("class".into());
    }

    // Defaults: primitive-level pieces, attribute-based (class).
    if let Some(end) = spec
        .nodes
        .iter_mut()
        .find(|n| matches!(&n.node_type, NodeType::Generic(s) if s == "ForEach End"))
    {
        if let Some(p) = end.parameters.iter_mut().find(|p| p.name == "piece_domain") {
            p.value = ParameterValue::Int(0);
        }
        if let Some(p) = end
            .parameters
            .iter_mut()
            .find(|p| p.name == "use_piece_attribute")
        {
            p.value = ParameterValue::Bool(true);
        }
        if let Some(p) = end
            .parameters
            .iter_mut()
            .find(|p| p.name == "piece_attribute")
        {
            p.value = ParameterValue::String("class".into());
        }
    }

    let begin_id = spec
        .nodes
        .iter()
        .find(|n| matches!(&n.node_type, NodeType::Generic(s) if s == "ForEach Begin"))
        .map(|n| n.id)
        .unwrap_or_else(Uuid::new_v4);
    spec.nodes.insert(0, conn);
    let conn_id = spec.nodes[0].id;
    spec.connections.insert(
        0,
        Connection {
            id: ConnectionId::new_v4(),
            from_node: conn_id,
            from_port: port_key::out0(),
            to_node: begin_id,
            to_port: port_key::in0(),
            order: 0,
            waypoints: Vec::new(),
        },
    );

    let mut min = Vec2::new(f32::INFINITY, f32::INFINITY);
    let mut max = Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
    for n in &spec.nodes {
        let p = n.position.to_vec2();
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        max.x = max.x.max(p.x + n.size.x);
        max.y = max.y.max(p.y + n.size.y);
    }
    let pad = Vec2::new(30.0, 30.0);
    spec.network_box.rect = Rect::from_min_max((min - pad).to_pos2(), (max + pad).to_pos2());
    spec.network_box.title = format!("ForEach Connectivity {}", spec.block_id);
    spec.network_box.nodes_inside = spec.nodes.iter().map(|n| n.id).collect();
    spec
}

pub fn build_foreach_point_block(
    g: &NodeGraph,
    reg: &NodeRegistry,
    settings: Option<&NodeEditorSettings>,
    pos: Pos2,
    include_meta: bool,
) -> ForEachSpawn {
    let mut spec = build_foreach_block(g, reg, settings, pos, include_meta);
    if let Some(end) = spec
        .nodes
        .iter_mut()
        .find(|n| matches!(&n.node_type, NodeType::Generic(s) if s == "ForEach End"))
    {
        if let Some(p) = end.parameters.iter_mut().find(|p| p.name == "piece_domain") {
            p.value = ParameterValue::Int(1);
        }
        if let Some(p) = end
            .parameters
            .iter_mut()
            .find(|p| p.name == "use_piece_attribute")
        {
            p.value = ParameterValue::Bool(false);
        }
        if let Some(p) = end
            .parameters
            .iter_mut()
            .find(|p| p.name == "piece_attribute")
        {
            p.value = ParameterValue::String("class".into());
        }
    }
    spec.network_box.title = format!("ForEach Point {}", spec.block_id);
    spec
}

pub fn build_foreach_primitive_block(
    g: &NodeGraph,
    reg: &NodeRegistry,
    settings: Option<&NodeEditorSettings>,
    pos: Pos2,
    include_meta: bool,
) -> ForEachSpawn {
    let mut spec = build_foreach_block(g, reg, settings, pos, include_meta);
    if let Some(end) = spec
        .nodes
        .iter_mut()
        .find(|n| matches!(&n.node_type, NodeType::Generic(s) if s == "ForEach End"))
    {
        if let Some(p) = end.parameters.iter_mut().find(|p| p.name == "piece_domain") {
            p.value = ParameterValue::Int(0);
        }
        if let Some(p) = end
            .parameters
            .iter_mut()
            .find(|p| p.name == "use_piece_attribute")
        {
            p.value = ParameterValue::Bool(false);
        }
        if let Some(p) = end
            .parameters
            .iter_mut()
            .find(|p| p.name == "piece_attribute")
        {
            p.value = ParameterValue::String("class".into());
        }
    }
    spec.network_box.title = format!("ForEach Primitive {}", spec.block_id);
    spec
}

pub fn apply_foreach_block_direct(g: &mut NodeGraph, spec: ForEachSpawn) -> Vec<NodeId> {
    let ids: Vec<NodeId> = spec.nodes.iter().map(|n| n.id).collect();
    for n in spec.nodes {
        g.nodes.insert(n.id, n);
    }
    for c in spec.connections {
        let to = c.to_node;
        g.connections.insert(c.id, c);
        g.mark_dirty(to);
    }
    let bid = spec.network_box.id;
    g.network_boxes.insert(bid, spec.network_box);
    if !g.network_box_draw_order.contains(&bid) {
        g.network_box_draw_order.push(bid);
    }
    for id in &ids {
        g.mark_dirty(*id);
    }
    ids
}
