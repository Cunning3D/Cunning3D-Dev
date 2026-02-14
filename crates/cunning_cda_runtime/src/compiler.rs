use std::collections::HashMap;

use cunning_kernel::traits::parameter::ParameterValue;

use crate::asset::{ConnectionDef, NodeDef, RuntimeDefinition};
use crate::error::{CdaCompileError, CdaCompileErrorKind};
use crate::registry::{MultiWirePolicy, RuntimeRegistry};
use crate::asset::PortId;
use crate::{ConnId, NodeId, OpCode};

#[derive(Clone, Debug)]
pub struct InputRef { pub src_node: NodeId, pub src_ix: usize, pub src_port: PortId, pub to_port: PortId, pub order: i32, pub id: ConnId }

#[derive(Clone, Debug)]
pub enum InputSlot {
    One { port: PortId, conn: Option<InputRef> },
    Many { port: PortId, conns: Vec<InputRef> },
}

#[derive(Clone, Debug)]
pub struct CompiledNode {
    pub id: NodeId,
    pub op: OpCode,
    pub params: HashMap<String, ParameterValue>,
    pub inputs: Vec<InputSlot>, // input layout in registry port order
}

#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    pub asset_uuid: uuid::Uuid,
    pub asset_name: String,
    pub nodes: Vec<CompiledNode>, // topo order
    pub node_index: HashMap<NodeId, usize>,
    pub outputs: Vec<NodeId>, // output nodes ids in declared outputs order
}

pub fn compile(def: &RuntimeDefinition, reg: &RuntimeRegistry) -> Result<ExecutionPlan, CdaCompileError> {
    if def.outputs.is_empty() && def.exports.is_empty() {
        return Err(CdaCompileError { asset_uuid: def.meta.uuid, asset_name: def.meta.name.clone(), kind: CdaCompileErrorKind::MissingOutputs });
    }

    let mut nodes_by_id: HashMap<NodeId, &NodeDef> = HashMap::new();
    for n in &def.nodes { nodes_by_id.insert(n.id, n); }

    // Validate connections early (node existence + port validity).
    for c in &def.connections {
        let Some(from_n) = nodes_by_id.get(&c.from_node).copied() else {
            return Err(CdaCompileError { asset_uuid: def.meta.uuid, asset_name: def.meta.name.clone(), kind: CdaCompileErrorKind::MissingNode { node_id: c.from_node } });
        };
        let Some(to_n) = nodes_by_id.get(&c.to_node).copied() else {
            return Err(CdaCompileError { asset_uuid: def.meta.uuid, asset_name: def.meta.name.clone(), kind: CdaCompileErrorKind::MissingNode { node_id: c.to_node } });
        };
        let from_op = reg.op_code_for_type(&from_n.type_id).ok_or_else(|| CdaCompileError { asset_uuid: def.meta.uuid, asset_name: def.meta.name.clone(), kind: CdaCompileErrorKind::UnknownOp { node_id: from_n.id, type_id: from_n.type_id.clone() } })?;
        let to_op = reg.op_code_for_type(&to_n.type_id).ok_or_else(|| CdaCompileError { asset_uuid: def.meta.uuid, asset_name: def.meta.name.clone(), kind: CdaCompileErrorKind::UnknownOp { node_id: to_n.id, type_id: to_n.type_id.clone() } })?;
        if reg.port_label(from_op, false, c.from_port).is_none() {
            return Err(CdaCompileError { asset_uuid: def.meta.uuid, asset_name: def.meta.name.clone(), kind: CdaCompileErrorKind::MissingPort { node_id: from_n.id, port: c.from_port.to_string(), is_input: false } });
        }
        if reg.port_label(to_op, true, c.to_port).is_none() {
            return Err(CdaCompileError { asset_uuid: def.meta.uuid, asset_name: def.meta.name.clone(), kind: CdaCompileErrorKind::MissingPort { node_id: to_n.id, port: c.to_port.to_string(), is_input: true } });
        }
    }

    let mut incoming: HashMap<NodeId, Vec<&ConnectionDef>> = HashMap::new();
    let mut outgoing: HashMap<NodeId, Vec<&ConnectionDef>> = HashMap::new();
    for c in &def.connections {
        incoming.entry(c.to_node).or_default().push(c);
        outgoing.entry(c.from_node).or_default().push(c);
    }

    // Kahn topo sort by NodeId order for determinism.
    let mut indeg: HashMap<NodeId, usize> = HashMap::new();
    for n in &def.nodes { indeg.insert(n.id, incoming.get(&n.id).map(|v| v.len()).unwrap_or(0)); }
    let mut ready: Vec<NodeId> = indeg.iter().filter(|(_, d)| **d == 0).map(|(id, _)| *id).collect();
    ready.sort();

    let mut order: Vec<NodeId> = Vec::with_capacity(def.nodes.len());
    while let Some(id) = ready.first().copied() {
        ready.remove(0);
        order.push(id);
        if let Some(outs) = outgoing.get(&id) {
            for c in outs {
                if let Some(d) = indeg.get_mut(&c.to_node) {
                    *d = d.saturating_sub(1);
                    if *d == 0 { ready.push(c.to_node); }
                }
            }
            ready.sort();
        }
    }

    if order.len() != def.nodes.len() {
        let cycle_nodes: Vec<NodeId> = indeg.into_iter().filter(|(_, d)| *d > 0).map(|(id, _)| id).collect();
        return Err(CdaCompileError { asset_uuid: def.meta.uuid, asset_name: def.meta.name.clone(), kind: CdaCompileErrorKind::CycleDetected { nodes: cycle_nodes } });
    }

    // Compile nodes.
    let mut compiled: Vec<CompiledNode> = Vec::with_capacity(order.len());
    let mut node_index: HashMap<NodeId, usize> = HashMap::new();
    for id in &order { node_index.insert(*id, compiled.len()); 
        let n = nodes_by_id.get(id).copied().ok_or_else(|| CdaCompileError { asset_uuid: def.meta.uuid, asset_name: def.meta.name.clone(), kind: CdaCompileErrorKind::MissingNode { node_id: *id } })?;
        let op = reg.op_code_for_type(&n.type_id).ok_or_else(|| CdaCompileError { asset_uuid: def.meta.uuid, asset_name: def.meta.name.clone(), kind: CdaCompileErrorKind::UnknownOp { node_id: n.id, type_id: n.type_id.clone() } })?;
        // Build inputs in registry port order; missing connections keep placeholders (no shifting).
        let mut by_port: HashMap<PortId, Vec<&ConnectionDef>> = HashMap::new();
        if let Some(cs) = incoming.get(&n.id) { for c in cs { by_port.entry(c.to_port).or_default().push(*c); } }
        let mut inputs: Vec<InputSlot> = Vec::new();
        for (p, label, policy) in reg.in_port_specs(op) {
            let mut xs = by_port.remove(&p).unwrap_or_default();
            xs.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));
            let mk = |c: &ConnectionDef, node_index: &HashMap<NodeId, usize>| -> Result<InputRef, CdaCompileError> {
                let src_ix = node_index.get(&c.from_node).copied().ok_or_else(|| CdaCompileError { asset_uuid: def.meta.uuid, asset_name: def.meta.name.clone(), kind: CdaCompileErrorKind::MissingNode { node_id: c.from_node } })?;
                Ok(InputRef { src_node: c.from_node, src_ix, src_port: c.from_port, to_port: c.to_port, order: c.order, id: c.id })
            };
            match policy {
                MultiWirePolicy::Error => {
                    if xs.len() > 1 {
                        return Err(CdaCompileError { asset_uuid: def.meta.uuid, asset_name: def.meta.name.clone(), kind: CdaCompileErrorKind::MultiWireViolation { node_id: n.id, port: label, count: xs.len() } });
                    }
                    let conn = if let Some(c) = xs.first() { Some(mk(c, &node_index)?) } else { None };
                    inputs.push(InputSlot::One { port: p, conn });
                }
                MultiWirePolicy::FirstByOrder => {
                    let conn = if let Some(c) = xs.first() { Some(mk(c, &node_index)?) } else { None };
                    inputs.push(InputSlot::One { port: p, conn });
                }
                MultiWirePolicy::LastByOrder => {
                    let conn = if let Some(c) = xs.last() { Some(mk(c, &node_index)?) } else { None };
                    inputs.push(InputSlot::One { port: p, conn });
                }
                MultiWirePolicy::All => {
                    let mut conns: Vec<InputRef> = Vec::with_capacity(xs.len());
                    for c in xs { conns.push(mk(c, &node_index)?); }
                    inputs.push(InputSlot::Many { port: p, conns });
                }
            }
        }
        compiled.push(CompiledNode { id: n.id, op, params: n.params.clone(), inputs });
    }

    // Resolve output nodes strictly: find Output node whose params["name"] matches each def.outputs[i].name.
    let mut outputs: Vec<NodeId> = Vec::new();
    for out in &def.outputs {
        let mut found: Option<NodeId> = None;
        for n in &compiled {
            if n.op == crate::registry::OP_OUTPUT {
                if let Some(ParameterValue::String(name)) = n.params.get("name") {
                    if name == &out.name { found = Some(n.id); break; }
                }
            }
        }
        let Some(id) = found else {
            return Err(CdaCompileError { asset_uuid: def.meta.uuid, asset_name: def.meta.name.clone(), kind: CdaCompileErrorKind::OutputBindingMissing { output_name: out.name.clone() } });
        };
        outputs.push(id);
    }

    Ok(ExecutionPlan { asset_uuid: def.meta.uuid, asset_name: def.meta.name.clone(), nodes: compiled, node_index, outputs })
}

