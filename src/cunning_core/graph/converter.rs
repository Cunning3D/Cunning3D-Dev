use bevy::prelude::*;
use bevy_egui::egui::Pos2;
use std::collections::HashMap;
use uuid::Uuid;

use crate::cunning_core::graph::asset::{
    ConnectionAssetData, GraphAsset, GraphEditorState, GraphLogic, GraphMeta, NodeAssetData,
};
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::nodes::structs::{Connection, Node, NodeId, NodeType};
use crate::nodes::NodeGraph;

pub struct GraphConverter;

const CDA_PATH_KEY: &str = "__cda_path";

impl GraphConverter {
    /// Convert Runtime NodeGraph to GraphAsset (Sparse Diff)
    pub fn to_asset(graph: &NodeGraph, registry: &NodeRegistry) -> GraphAsset {
        let mut logic = GraphLogic {
            nodes: HashMap::new(),
            connections: Vec::new(),
            inputs: Vec::new(), // TODO: Infer from Graph inputs if applicable
            outputs: Vec::new(),
        };

        let mut editor = GraphEditorState {
            node_positions: HashMap::new(),
        };

        // 1. Convert Nodes (store all params; diffing can be added later)
        let _ = registry; // keep signature stable; registry not required for current NodeType layout
        for (id, node) in &graph.nodes {
            let mut params_diff = HashMap::new();
            let type_id = match &node.node_type {
                NodeType::CDA(cda_data) => {
                    for p in &node.parameters {
                        params_diff.insert(p.name.clone(), p.value.clone());
                    }
                    params_diff.insert(
                        CDA_PATH_KEY.to_string(),
                        crate::nodes::parameter::ParameterValue::String(
                            cda_data.asset_ref.path.clone(),
                        ),
                    );
                    format!("cda://{}", cda_data.asset_ref.uuid)
                }
                _ => {
                    for p in &node.parameters {
                        params_diff.insert(p.name.clone(), p.value.clone());
                    }
                    node.node_type.type_id().to_string()
                }
            };

            logic.nodes.insert(
                *id,
                NodeAssetData {
                    type_id,
                    params: params_diff,
                },
            );

            // Save Editor Position
            editor
                .node_positions
                .insert(*id, Vec2::new(node.position.x, node.position.y));
        }

        // 2. Convert Connections (stable order)
        let mut conns: Vec<&Connection> = graph.connections.values().collect();
        conns.sort_by(|a, b| a.id.cmp(&b.id));
        for conn in conns {
            logic.connections.push(ConnectionAssetData {
                id: conn.id,
                from_node: conn.from_node,
                from_socket: conn.from_port.as_str().to_string(),
                to_node: conn.to_node,
                to_socket: conn.to_port.as_str().to_string(),
                order: conn.order,
            });
        }

        GraphAsset {
            meta: GraphMeta {
                format_version: 2,
                min_engine_version: "0.1.0".to_string(),
                uuid: Uuid::new_v4(),
                name: "Exported".to_string(),
                author: None,
                license: None,
            },
            logic,
            editor: Some(editor),
        }
    }

    /// Restore NodeGraph from GraphAsset
    pub fn from_asset(asset: &GraphAsset, registry: &NodeRegistry) -> NodeGraph {
        let mut graph = NodeGraph::new();
        let _ = registry;

        // 1. Restore Nodes
        for (id, data) in &asset.logic.nodes {
            let mut node = if data.type_id.starts_with("cda://") {
                let uuid = data.type_id.trim_start_matches("cda://");
                let uuid = Uuid::parse_str(uuid).unwrap_or_else(|_| Uuid::new_v4());
                let path = match data.params.get(CDA_PATH_KEY) {
                    Some(crate::nodes::parameter::ParameterValue::String(s)) => s.clone(),
                    _ => String::new(),
                };
                Node::new(
                    *id,
                    "CDA".to_string(),
                    NodeType::CDA(crate::nodes::structs::CDANodeData {
                        asset_ref: crate::cunning_core::cda::CdaAssetRef { uuid, path },
                        name: "CDA".to_string(),
                        coverlay_hud: None,
                        coverlay_units: Vec::new(),
                        inner_param_overrides: Default::default(),
                    }),
                    Pos2::ZERO,
                )
            } else if let Some(n) = resolve_legacy_node_type(&data.type_id, *id) {
                n
            } else {
                Node::new(
                    *id,
                    data.type_id.clone(),
                    NodeType::Generic(data.type_id.clone()),
                    Pos2::ZERO,
                )
            };

            // Apply Params
            for (name, val) in &data.params {
                if name == CDA_PATH_KEY {
                    continue;
                }
                if let Some(p) = node.parameters.iter_mut().find(|p| p.name == *name) {
                    p.value = val.clone();
                }
            }

            // Apply Position
            if let Some(editor) = &asset.editor {
                if let Some(pos) = editor.node_positions.get(id) {
                    node.position = Pos2::new(pos.x, pos.y);
                }
            }

            graph.nodes.insert(*id, node);
        }

        // 2. Restore Connections
        for conn_data in &asset.logic.connections {
            // Simply verify nodes exist
            if graph.nodes.contains_key(&conn_data.from_node)
                && graph.nodes.contains_key(&conn_data.to_node)
            {
                let c = Connection {
                    id: conn_data.id,
                    from_node: conn_data.from_node,
                    from_port: crate::nodes::PortId::from(conn_data.from_socket.as_str()),
                    to_node: conn_data.to_node,
                    to_port: crate::nodes::PortId::from(conn_data.to_socket.as_str()),
                    order: conn_data.order,
                    waypoints: Vec::new(),
                };
                graph.connections.insert(c.id, c);
            }
        }

        graph
    }
}

// Helper to map String -> Node (Legacy)
fn resolve_legacy_node_type(type_name: &str, id: NodeId) -> Option<Node> {
    // This is brittle but necessary until all nodes are in Registry.
    let node_type = match type_name {
        "Create Cube" | "cunning.basic.create_cube" => NodeType::CreateCube,
        "Create Sphere" | "cunning.basic.create_sphere" => NodeType::CreateSphere,
        "Transform" | "cunning.basic.transform" => NodeType::Transform,
        "Merge" | "cunning.utility.merge" => NodeType::Merge,
        "Boolean" | "cunning.modeling.boolean" => NodeType::Boolean,
        "PolyExtrude" | "Poly Extrude" | "cunning.modeling.poly_extrude" => NodeType::PolyExtrude,
        "PolyBevel" | "Poly Bevel" | "cunning.modeling.poly_bevel" => NodeType::PolyBevel,
        "Fuse" | "cunning.modeling.fuse" => NodeType::Fuse,
        "Group Create" | "cunning.group.create" => NodeType::GroupCreate,
        "VDB From Polygons" | "cunning.vdb.from_polygons" => NodeType::VdbFromPolygons,
        "VDB To Polygons" | "cunning.vdb.to_polygons" => NodeType::VdbToPolygons,
        "Voxel Edit" | "VoxelEdit" | "cunning.voxel.edit" => NodeType::VoxelEdit,
        // ... add others
        _ => return None,
    };

    Some(Node::new(id, type_name.to_string(), node_type, Pos2::ZERO))
}
