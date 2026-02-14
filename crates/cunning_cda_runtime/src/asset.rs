use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use cunning_kernel::traits::parameter::ParameterValue;

pub type NodeId = Uuid;
pub type ConnId = Uuid;
pub type PortId = u32;
pub type IfaceId = Uuid;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ExportsMode {
    #[default]
    BlackBox,
    Advanced,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ExportKind {
    NodeOutput { node_id: NodeId },
    NodeParam { node_id: NodeId, param: String, #[serde(default)] channel: Option<u32> },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ExportDef {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub order: i32,
    pub kind: ExportKind,
}

/// Runtime-facing HUD exposure unit (single-select).
/// Mirrors `CDAAsset.hud_units` in editor-side `src/cunning_core/cda/asset.rs`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HudUnit {
    pub node_id: NodeId,
    pub label: String,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub is_default: bool,
}

/// Runtime-facing Coverlay exposure unit (multi-select).
/// Mirrors `CDAAsset.coverlay_units` in editor-side `src/cunning_core/cda/asset.rs`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoverlayUnit {
    pub node_id: NodeId,
    pub label: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub default_on: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RuntimeMeta {
    pub format_version: u32,
    pub min_engine_version: String,
    pub uuid: Uuid,
    pub name: String,
    pub author: Option<String>,
    pub license: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PortDef {
    pub id: IfaceId,
    pub name: String,
    #[serde(default)] pub data_type: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NodeDef {
    pub id: NodeId,
    pub type_id: String,
    #[serde(default)] pub params: HashMap<String, ParameterValue>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConnectionDef {
    pub id: ConnId,
    pub from_node: NodeId,
    pub from_port: PortId,
    pub to_node: NodeId,
    pub to_port: PortId,
    #[serde(default)] pub order: i32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PromotedBinding { pub node: NodeId, pub param: String, #[serde(default)] pub channel: Option<u32> }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PromotedParam {
    pub name: String,
    #[serde(default)] pub label: String,
    #[serde(default)] pub group: String,
    #[serde(default)] pub order: i32,
    pub param_type: String,
    #[serde(default = "default_param_value")] pub default_value: ParameterValue,
    #[serde(default)] pub bindings: Vec<PromotedBinding>,
}

fn default_param_value() -> ParameterValue { ParameterValue::Float(0.0) }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RuntimeDefinition {
    pub meta: RuntimeMeta,
    #[serde(default)] pub inputs: Vec<PortDef>,
    #[serde(default)] pub outputs: Vec<PortDef>,
    pub nodes: Vec<NodeDef>,
    pub connections: Vec<ConnectionDef>,
    #[serde(default)] pub promoted_params: Vec<PromotedParam>,
    /// Overlay exposure (authoring-time): HUD is single-select, Coverlay is multi-select.
    #[serde(default)]
    pub hud_units: Vec<HudUnit>,
    #[serde(default)]
    pub coverlay_units: Vec<CoverlayUnit>,
    /// Host-side selectable exports (advanced mode).
    #[serde(default)]
    pub exports_mode: ExportsMode,
    #[serde(default)]
    pub exports: Vec<ExportDef>,
}

