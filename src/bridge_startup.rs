use std::collections::HashMap;
use std::path::Path;

use bevy::app::AppExit;
use bevy::ecs::prelude::MessageReader;
use bevy::log::{info, warn};
use bevy::prelude::*;
use uuid::Uuid;

use crate::cunning_core::cda::library::{global_cda_library, CdaAssetRef};
use crate::cunning_core::cda::{CDAAsset, CDAInterface};
use crate::nodes::parameter::ParameterValue;
use crate::nodes::{CDANodeData, Node, NodeGraph, NodeGraphResource, NodeType};

#[derive(Resource, Default, Clone)]
pub struct BridgeStartup {
    pub path: Option<String>,
    pub ephemeral: bool,
}

#[derive(serde::Deserialize)]
struct BridgeFileHeader {
    version: String,
}

#[derive(serde::Deserialize)]
struct BridgeFile {
    header: BridgeFileHeader,
    bridge_records: Vec<CdaBridgeRecord>,
}

#[derive(serde::Deserialize)]
struct CdaBridgeInput {
    kind: String,
    #[serde(default)]
    handle: u64,
    #[serde(default)]
    dirty: u64,
    #[serde(default)]
    source_basis: u32,
    #[serde(default)]
    blob_key: String,
    #[serde(default)]
    blob_id: u64,
}

#[derive(serde::Deserialize)]
struct CdaBridgeRecord {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    time_utc: String,
    #[serde(default)]
    instance_id: u64,
    #[serde(default)]
    asset_name: String,
    #[serde(default)]
    asset_source_path: String,
    #[serde(default)]
    asset_source_json: String,
    #[serde(default)]
    params_json: HashMap<String, ParameterValue>,
    #[serde(default)]
    inputs: Vec<CdaBridgeInput>,
}

fn parse_uuid_from_asset_json(s: &str) -> Option<Uuid> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    let u = v
        .get("meta")
        .and_then(|m| m.get("uuid"))
        .and_then(|x| x.as_str())?;
    Uuid::parse_str(u).ok()
}

fn ensure_bridge_asset_in_memory(asset_ref: &CdaAssetRef, asset_name: &str, asset_source_json: &str) {
    // Unity "empty bridge CDA" may have no on-disk .cda. In that case, reconstruct a minimal
    // editor-side CDAAsset from the runtime (GAME) json so we can rebuild ports and connect inputs.
    let Some(lib) = global_cda_library() else { return };
    if lib.get(asset_ref.uuid).is_some() { return };
    if asset_source_json.is_empty() { return };
    let def: cunning_cda_runtime::asset::RuntimeDefinition = match serde_json::from_str(asset_source_json) {
        Ok(v) => v,
        Err(e) => {
            warn!("bridge: asset_source_json parse failed (runtime def): {e}");
            return;
        }
    };
    let mut asset = CDAAsset::default();
    asset.name = if !asset_name.is_empty() { asset_name.to_string() } else { def.meta.name.clone() };
    asset.id = asset_ref.uuid;
    asset.inputs = def
        .inputs
        .iter()
        .enumerate()
        .map(|(i, p)| CDAInterface::new(&p.name, Uuid::new_v4()).with_label(&p.name).with_order(i as i32))
        .collect();
    asset.outputs = def
        .outputs
        .iter()
        .enumerate()
        .map(|(i, p)| CDAInterface::new(&p.name, Uuid::new_v4()).with_label(&p.name).with_order(i as i32))
        .collect();
    lib.put(asset);
    info!("bridge: created in-memory CDAAsset from asset_source_json (uuid={})", asset_ref.uuid);
}

pub fn import_bridge_on_enter(mut ng_res: ResMut<NodeGraphResource>, bridge: Res<BridgeStartup>) {
    let Some(path) = bridge.path.as_deref() else {
        info!("bridge: no --bridge path provided");
        return;
    };
    if path.is_empty() {
        warn!("bridge: --bridge path is empty");
        return;
    }
    let txt = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(e) => {
            warn!("bridge: read failed: {e} ({path})");
            return;
        }
    };
    let file = match serde_json::from_str::<BridgeFile>(&txt) {
        Ok(v) => v,
        Err(e) => {
            warn!("bridge: json parse failed: {e} ({path})");
            return;
        }
    };
    if !file.header.version.starts_with("bridge-") {
        warn!(
            "bridge: unsupported header version: {}",
            file.header.version
        );
        return;
    }
    let Some(rec) = file.bridge_records.first() else {
        warn!("bridge: bridge_records is empty");
        return;
    };
    let asset_uuid =
        parse_uuid_from_asset_json(&rec.asset_source_json).unwrap_or_else(Uuid::new_v4);
    let asset_ref = CdaAssetRef {
        uuid: asset_uuid,
        path: rec.asset_source_path.clone(),
    };
    info!(
        "bridge: importing CDA asset: uuid={} path={}",
        asset_ref.uuid, asset_ref.path
    );
    if let Some(lib) = global_cda_library() {
        if let Err(e) = lib.ensure_loaded(&asset_ref) {
            warn!("bridge: ensure_loaded failed: {e:?}");
        }
    } else {
        warn!("bridge: global_cda_library() is None (not initialized yet?)");
    }
    if asset_ref.path.is_empty() {
        ensure_bridge_asset_in_memory(&asset_ref, &rec.asset_name, &rec.asset_source_json);
    }

    let mut graph = NodeGraph::new();
    let cda_id = Uuid::new_v4();
    let mut cda_node = Node::new(
        cda_id,
        if rec.asset_name.is_empty() {
            "CDA".to_string()
        } else {
            rec.asset_name.clone()
        },
        NodeType::CDA(CDANodeData {
            asset_ref: asset_ref.clone(),
            name: if rec.asset_name.is_empty() {
                "CDA".to_string()
            } else {
                rec.asset_name.clone()
            },
            coverlay_hud: None,
            coverlay_units: Vec::new(),
            inner_param_overrides: Default::default(),
        }),
        bevy_egui::egui::Pos2::new(300.0, 200.0),
    );
    cda_node.rebuild_ports();
    cda_node.rebuild_parameters();
    for p in cda_node.parameters.iter_mut() {
        if let Some(v) = rec.params_json.get(&p.name) {
            p.value = v.clone();
        }
    }
    graph.nodes.insert(cda_id, cda_node);

    // Default inputs: create placeholder nodes by kind and connect by index (deterministic).
    if let Some(lib) = global_cda_library() {
        if let Some(asset) = lib.get(asset_ref.uuid) {
            for (i, in_port) in asset.inputs.iter().enumerate() {
                let inp = rec.inputs.get(i);
                let kind = inp.map(|x| x.kind.as_str()).unwrap_or("unknown");
                let node_type = if kind == "spline" {
                    NodeType::Spline
                } else {
                    NodeType::Generic("Input".to_string())
                };
                let nid = Uuid::new_v4();
                let mut n = Node::new(
                    nid,
                    format!("Input_{}", i),
                    node_type,
                    bevy_egui::egui::Pos2::new(40.0, 120.0 + i as f32 * 90.0),
                );
                n.rebuild_parameters();
                if kind == "spline" {
                    if let Some(inp) = inp {
                        if let Some(p) = n
                            .parameters
                            .iter_mut()
                            .find(|p| p.name == "spline_blob_key")
                        {
                            p.value = ParameterValue::String(inp.blob_key.clone());
                        }
                        if let Some(p) = n
                            .parameters
                            .iter_mut()
                            .find(|p| p.name == "spline_source_basis")
                        {
                            p.value = ParameterValue::Int(inp.source_basis as i32);
                        }
                    }
                }
                n.rebuild_ports();
                graph.nodes.insert(nid, n);
                let cid = Uuid::new_v4();
                graph.connections.insert(
                    cid,
                    crate::nodes::Connection {
                        id: cid,
                        from_node: nid,
                        from_port: crate::nodes::port_key::out0(),
                        to_node: cda_id,
                        to_port: in_port.port_key(),
                        order: i as i32,
                        waypoints: Vec::new(),
                    },
                );
            }
        }
    }

    graph.display_node = Some(cda_id);
    graph.ensure_display_node_default();
    ng_res.0 = graph;
    info!("bridge: imported graph ok (cda_node={cda_id})");
}

pub fn cleanup_ephemeral_on_exit(mut exit: MessageReader<AppExit>, bridge: Res<BridgeStartup>) {
    if exit.is_empty() {
        return;
    }
    exit.clear();
    if !bridge.ephemeral {
        return;
    }
    let Some(p) = bridge.path.as_deref() else {
        return;
    };
    let _ = std::fs::remove_file(Path::new(p));
}
