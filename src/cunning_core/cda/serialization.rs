//! CDA file serialization/deserialization
use super::CDAAsset;
use crate::cunning_core::cda::promoted_param::PromotedParamType;
use crate::cunning_core::graph::asset::{
    ConnectionAssetData, GraphLogic, GraphMeta, GraphPortDef, NodeAssetData,
};
use crate::cunning_core::traits::parameter::ParameterValue;
use crate::nodes::structs::NodeType;
use std::{fs, path::Path};

const CDA_MAGIC: &[u8; 4] = b"CDA\0";
// File container format version (NOT CDAAsset.version).
const CDA_FILE_VERSION: u32 = 1;
const CHUNK_DCC0: [u8; 4] = *b"DCC0";
const CHUNK_GAME: [u8; 4] = *b"GAME";
const CDA_EXTENSION: &str = "cda";

#[derive(Debug)]
pub enum CDAError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Compile(String),
    InvalidMagic,
    VersionMismatch(u32),
    MissingChunk,
}
impl From<std::io::Error> for CDAError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for CDAError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

#[derive(Clone, Debug, Default)]
pub struct CdaSaveReport {
    pub game_ok: bool,
    pub game_error: Option<String>,
}

#[inline]
fn u32le(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}
#[inline]
fn u64le(v: u64) -> [u8; 8] {
    v.to_le_bytes()
}

fn promoted_default(
    pt: &PromotedParamType,
    chans: &[crate::cunning_core::cda::ParamChannel],
) -> ParameterValue {
    crate::cunning_core::cda::utils::promoted_channels_to_value(pt, chans)
}

pub fn build_game_chunk(
    a: &CDAAsset,
) -> Result<cunning_cda_runtime::asset::RuntimeDefinition, CDAError> {
    // NOTE: runtime schema is owned by `cunning_cda_runtime`; we output that schema (no fallback).
    use cunning_cda_runtime::asset as rt;
    use cunning_cda_runtime::registry::RuntimeRegistry;

    let mut nodes: std::collections::HashMap<crate::nodes::structs::NodeId, NodeAssetData> =
        std::collections::HashMap::new();
    // IO index mapping: sort by order then name for determinism
    let mut ins = a.inputs.clone();
    ins.sort_by(|x, y| x.order.cmp(&y.order).then(x.name.cmp(&y.name)));
    let mut outs = a.outputs.clone();
    outs.sort_by(|x, y| x.order.cmp(&y.order).then(x.name.cmp(&y.name)));
    let in_idx: std::collections::HashMap<crate::nodes::structs::NodeId, i32> = ins
        .iter()
        .enumerate()
        .map(|(i, it)| (it.internal_node, i as i32))
        .collect();
    let out_name: std::collections::HashMap<crate::nodes::structs::NodeId, String> = outs
        .iter()
        .map(|o| (o.internal_node, o.name.clone()))
        .collect();

    for (id, n) in &a.inner_graph.nodes {
        let mut params: std::collections::HashMap<String, ParameterValue> =
            std::collections::HashMap::new();
        for p in &n.parameters {
            params.insert(p.name.clone(), p.value.clone());
        }
        let (type_id, params) = match &n.node_type {
            // NOTE: runtime node kinds use stable `type_id`; UI names must never reach GAME export.
            NodeType::CDAInput(_) => ("cunning.input".to_string(), {
                let mut p = params;
                if let Some(i) = in_idx.get(id) {
                    p.insert("index".to_string(), ParameterValue::Int(*i));
                }
                p
            }),
            NodeType::CDAOutput(_) => ("cunning.output".to_string(), {
                let mut p = params;
                if let Some(nm) = out_name.get(id) {
                    p.insert("name".to_string(), ParameterValue::String(nm.clone()));
                }
                p
            }),
            _ => (n.node_type.type_id().to_string(), params),
        };
        nodes.insert(*id, NodeAssetData { type_id, params });
    }

    let mut connections: Vec<ConnectionAssetData> = Vec::new();
    for c in a.inner_graph.connections.values() {
        connections.push(ConnectionAssetData {
            id: c.id,
            from_node: c.from_node,
            from_socket: c.from_port.as_str().to_string(),
            to_node: c.to_node,
            to_socket: c.to_port.as_str().to_string(),
            order: c.order,
        });
    }

    let promoted_params: Vec<rt::PromotedParam> = a
        .promoted_params
        .iter()
        .map(|p| {
            let mut bindings: Vec<rt::PromotedBinding> = Vec::new();
            for ch in &p.channels {
                for b in &ch.bindings {
                    bindings.push(rt::PromotedBinding {
                        node: b.target_node,
                        param: b.target_param.clone(),
                        channel: b.target_channel.map(|v| v as u32),
                    });
                }
            }
            rt::PromotedParam {
                name: p.name.clone(),
                label: p.label.clone(),
                group: p.group.clone(),
                order: p.order,
                param_type: p.param_type.display_name().to_string(),
                default_value: promoted_default(&p.param_type, &p.channels),
                bindings,
            }
        })
        .collect();

    let meta = GraphMeta {
        format_version: 1,
        min_engine_version: "0.1.0".to_string(),
        uuid: a.id,
        name: a.name.clone(),
        author: a.author.clone(),
        license: None,
    };
    let nodes_len = nodes.len();
    let logic = GraphLogic {
        nodes,
        connections,
        inputs: Vec::new(),
        outputs: Vec::new(),
    };
    let inputs: Vec<GraphPortDef> = ins
        .iter()
        .map(|p| GraphPortDef {
            name: p.name.clone(),
            data_type: "Geometry".to_string(),
        })
        .collect();
    let outputs: Vec<GraphPortDef> = outs
        .iter()
        .map(|p| GraphPortDef {
            name: p.name.clone(),
            data_type: "Geometry".to_string(),
        })
        .collect();

    // Convert to strict runtime schema deterministically (NO random ids, NO string ports in exec path).
    let reg = RuntimeRegistry::new_default();
    let mut type_by_id: std::collections::HashMap<crate::nodes::structs::NodeId, String> =
        std::collections::HashMap::new();
    for (id, n) in &logic.nodes {
        type_by_id.insert(*id, n.type_id.clone());
    }

    let mut rt_nodes: Vec<rt::NodeDef> = Vec::with_capacity(nodes_len);
    for (id, n) in &logic.nodes {
        rt_nodes.push(rt::NodeDef {
            id: *id,
            type_id: n.type_id.clone(),
            params: n.params.clone(),
        });
    }

    let mut rt_conns: Vec<rt::ConnectionDef> = Vec::with_capacity(a.inner_graph.connections.len());
    for c in a.inner_graph.connections.values() {
        let from_type = type_by_id.get(&c.from_node).cloned().unwrap_or_default();
        let to_type = type_by_id.get(&c.to_node).cloned().unwrap_or_default();
        let Some(from_op) = reg.op_code_for_type(&from_type) else {
            return Err(CDAError::Compile(format!("Unknown op type: {}", from_type)));
        };
        let Some(to_op) = reg.op_code_for_type(&to_type) else {
            return Err(CDAError::Compile(format!("Unknown op type: {}", to_type)));
        };
        let from_port = reg
            .port_key_by_label(from_op, false, c.from_port.as_str())
            .unwrap_or_else(|| c.from_port.as_str().to_string());
        let to_port = reg
            .port_key_by_label(to_op, true, c.to_port.as_str())
            .unwrap_or_else(|| c.to_port.as_str().to_string());
        // Ports are stable keys in NodeGraph (in:0/out:0/in:a/...) and RuntimeRegistry maps them to PortId.
        let Some(from_pid) = reg.port_id(from_op, false, &from_port) else {
            return Err(CDAError::Compile(format!(
                "Unknown from_port key: {}.{}",
                from_type, from_port
            )));
        };
        let Some(to_pid) = reg.port_id(to_op, true, &to_port) else {
            return Err(CDAError::Compile(format!(
                "Unknown to_port key: {}.{}",
                to_type, to_port
            )));
        };
        rt_conns.push(rt::ConnectionDef {
            id: c.id,
            from_node: c.from_node,
            from_port: from_pid,
            to_node: c.to_node,
            to_port: to_pid,
            order: c.order,
        });
    }

    // NOTE: IO ids must be stable across edits/reorders; use CDAInterface.id (uuid) instead of enumerate().
    let rt_inputs: Vec<rt::PortDef> = ins
        .into_iter()
        .zip(inputs.into_iter())
        .map(|(iface, p)| rt::PortDef {
            id: iface.id,
            name: p.name,
            data_type: p.data_type,
        })
        .collect();
    let rt_outputs: Vec<rt::PortDef> = outs
        .into_iter()
        .zip(outputs.into_iter())
        .map(|(iface, p)| rt::PortDef {
            id: iface.id,
            name: p.name,
            data_type: p.data_type,
        })
        .collect();
    let def = rt::RuntimeDefinition {
        meta: rt::RuntimeMeta {
            format_version: 1,
            min_engine_version: "0.1.0".to_string(),
            uuid: meta.uuid,
            name: meta.name,
            author: meta.author,
            license: meta.license,
        },
        inputs: rt_inputs,
        outputs: rt_outputs,
        nodes: rt_nodes,
        connections: rt_conns,
        promoted_params,
        hud_units: a
            .hud_units
            .iter()
            .map(|u| rt::HudUnit {
                node_id: u.node_id,
                label: u.label.clone(),
                order: u.order,
                is_default: u.is_default,
            })
            .collect(),
        coverlay_units: a
            .coverlay_units
            .iter()
            .map(|u| rt::CoverlayUnit {
                node_id: u.node_id,
                label: u.label.clone(),
                icon: u.icon.clone(),
                order: u.order,
                default_on: u.default_on,
            })
            .collect(),
    };
    Ok(def)
}

impl CDAAsset {
    pub fn save_with_report(&self, path: impl AsRef<Path>) -> Result<CdaSaveReport, CDAError> {
        let dcc = serde_json::to_string(self)?;
        let dcc_bytes = dcc.as_bytes();

        // Strict: .cda must contain a valid runtime GAME chunk; no stub fallback.
        let game = build_game_chunk(self)?;
        let report = CdaSaveReport {
            game_ok: true,
            game_error: None,
        };

        let game_bytes = serde_json::to_vec(&game)?;
        // header: magic + version + chunk_count(2) + table(2 entries) + payloads
        let table_off = 4 + 4 + 4;
        let table_sz = 2 * (4 + 8 + 8);
        let dcc_off = (table_off + table_sz) as u64;
        let game_off = dcc_off + dcc_bytes.len() as u64;
        let mut out = Vec::with_capacity(game_off as usize + game_bytes.len());
        out.extend_from_slice(CDA_MAGIC);
        out.extend_from_slice(&u32le(CDA_FILE_VERSION));
        out.extend_from_slice(&u32le(2));
        out.extend_from_slice(&CHUNK_DCC0);
        out.extend_from_slice(&u64le(dcc_off));
        out.extend_from_slice(&u64le(dcc_bytes.len() as u64));
        out.extend_from_slice(&CHUNK_GAME);
        out.extend_from_slice(&u64le(game_off));
        out.extend_from_slice(&u64le(game_bytes.len() as u64));
        out.extend_from_slice(dcc_bytes);
        out.extend_from_slice(&game_bytes);
        fs::write(path, out)?;
        Ok(report)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), CDAError> {
        let _ = self.save_with_report(path)?;
        Ok(())
    }

    #[inline]
    fn chunk_bytes<'a>(data: &'a [u8], chunk: [u8; 4]) -> Result<&'a [u8], CDAError> {
        if data.get(0..4) != Some(CDA_MAGIC) {
            return Err(CDAError::InvalidMagic);
        }
        let ver = u32::from_le_bytes(
            data.get(4..8)
                .ok_or(CDAError::InvalidMagic)?
                .try_into()
                .map_err(|_| CDAError::InvalidMagic)?,
        );
        if ver != CDA_FILE_VERSION {
            return Err(CDAError::VersionMismatch(ver));
        }
        let n = u32::from_le_bytes(
            data.get(8..12)
                .ok_or(CDAError::InvalidMagic)?
                .try_into()
                .map_err(|_| CDAError::InvalidMagic)?,
        ) as usize;
        let mut at = 12usize;
        for _ in 0..n {
            let id: [u8; 4] = data
                .get(at..at + 4)
                .ok_or(CDAError::InvalidMagic)?
                .try_into()
                .map_err(|_| CDAError::InvalidMagic)?;
            at += 4;
            let off = u64::from_le_bytes(
                data.get(at..at + 8)
                    .ok_or(CDAError::InvalidMagic)?
                    .try_into()
                    .map_err(|_| CDAError::InvalidMagic)?,
            );
            at += 8;
            let sz = u64::from_le_bytes(
                data.get(at..at + 8)
                    .ok_or(CDAError::InvalidMagic)?
                    .try_into()
                    .map_err(|_| CDAError::InvalidMagic)?,
            );
            at += 8;
            if id == chunk {
                let s = off as usize;
                let e = s.checked_add(sz as usize).ok_or(CDAError::InvalidMagic)?;
                return Ok(data.get(s..e).ok_or(CDAError::InvalidMagic)?);
            }
        }
        Err(CDAError::MissingChunk)
    }

    pub fn load_dcc(path: impl AsRef<Path>) -> Result<Self, CDAError> {
        let data = fs::read(path)?;
        let bytes = Self::chunk_bytes(&data, CHUNK_DCC0)?;
        let json = std::str::from_utf8(bytes).map_err(|_| CDAError::InvalidMagic)?;
        Ok(serde_json::from_str(json)?)
    }

    pub fn load_game_json(path: impl AsRef<Path>) -> Result<String, CDAError> {
        let data = fs::read(path)?;
        let bytes = Self::chunk_bytes(&data, CHUNK_GAME)?;
        Ok(std::str::from_utf8(bytes)
            .map_err(|_| CDAError::InvalidMagic)?
            .to_string())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, CDAError> {
        Self::load_dcc(path)
    }

    pub fn extension() -> &'static str {
        CDA_EXTENSION
    }
}
