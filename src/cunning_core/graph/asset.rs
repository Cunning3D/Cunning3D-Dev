use bevy::prelude::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::nodes::parameter::ParameterValue;
use crate::nodes::structs::NodeId;

/// The Graph Asset (*.cgraph) Format
///
/// Design Philosophy:
/// 1. Clean Core: Logic separated from UI.
/// 2. Sparse Storage: Only diffs from default values are stored.
/// 3. VFS Paths: Assets referenced via protocols (res://, pkg://).

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphAsset {
    pub meta: GraphMeta,
    pub logic: GraphLogic,

    // Optional Editor state. Can be stripped for Engine runtime builds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<GraphEditorState>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphMeta {
    pub format_version: u32,        // e.g. 1
    pub min_engine_version: String, // e.g. "1.0.0"
    pub uuid: Uuid,
    pub name: String,
    pub author: Option<String>,
    pub license: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphLogic {
    // We use a Vec of structs for determinism and easy diffing, or Map for lookup?
    // Map<NodeId, ...> is better for lookup during loading.
    pub nodes: HashMap<NodeId, NodeAssetData>,
    pub connections: Vec<ConnectionAssetData>,

    // Interface definition (if this graph is used as an Asset/SubGraph)
    #[serde(default)]
    pub inputs: Vec<GraphPortDef>,
    #[serde(default)]
    pub outputs: Vec<GraphPortDef>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NodeAssetData {
    pub type_id: String, // e.g. "cunning.math.add" or "cda://my_asset.cda"

    // Sparse parameter storage.
    // Only stores values that differ from the node's default.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub params: HashMap<String, ParameterValue>,
    // For Subgraph/CDA nodes, we might store internal overrides here?
    // Or just treat them as params.
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConnectionAssetData {
    pub id: Uuid,
    pub from_node: NodeId,
    pub from_socket: String, // Name or ID
    pub to_node: NodeId,
    pub to_socket: String,
    #[serde(default)]
    pub order: i32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphPortDef {
    pub name: String,
    pub data_type: String, // "Geometry", "Float", etc.
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphEditorState {
    pub node_positions: HashMap<NodeId, Vec2>,
    // TODO: Sticky notes, colors, groups, comments
    // pub sticky_notes: ...
}

impl Default for GraphAsset {
    fn default() -> Self {
        Self {
            meta: GraphMeta {
                format_version: 2,
                min_engine_version: "0.1.0".to_string(),
                uuid: Uuid::new_v4(),
                name: "Untitled".to_string(),
                author: None,
                license: None,
            },
            logic: GraphLogic {
                nodes: HashMap::new(),
                connections: Vec::new(),
                inputs: Vec::new(),
                outputs: Vec::new(),
            },
            editor: Some(GraphEditorState {
                node_positions: HashMap::new(),
            }),
        }
    }
}
