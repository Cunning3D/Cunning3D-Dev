use std::collections::HashMap;
use std::sync::Arc;

use cunning_kernel::mesh::Geometry;
use cunning_kernel::traits::parameter::ParameterValue;
use smallvec::SmallVec;

use crate::error::{CdaCookError, CdaCookErrorKind};
use crate::{NodeId, OpCode};
use crate::asset::PortId;

pub const OP_INPUT: OpCode = 1;
pub const OP_OUTPUT: OpCode = 2;
pub const OP_BOOLEAN: OpCode = 10;
pub const OP_POLY_EXTRUDE: OpCode = 11;
pub const OP_POLY_BEVEL: OpCode = 12;
pub const OP_GROUP_CREATE: OpCode = 13;
pub const OP_MERGE: OpCode = 20;
pub const OP_VOXEL_EDIT: OpCode = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiWirePolicy { Error, FirstByOrder, LastByOrder, All }

pub trait RuntimeOp: Send + Sync {
    fn compute(&self, node_id: NodeId, params: &HashMap<String, ParameterValue>, inputs: &[PortInputs], external_inputs: &[Arc<Geometry>]) -> Result<Arc<Geometry>, CdaCookError>;
}

#[derive(Clone, Debug)]
pub struct PortInputs { pub port: PortId, pub values: SmallVec<[Arc<Geometry>; 1]> }

#[derive(Clone, Debug)]
struct PortSpec { id: PortId, label: String, policy: MultiWirePolicy }

pub struct RuntimeRegistry {
    by_code: HashMap<OpCode, Arc<dyn RuntimeOp>>,
    type_to_code: HashMap<String, OpCode>,
    in_ports: HashMap<OpCode, HashMap<String, PortSpec>>,  // key -> spec
    out_ports: HashMap<OpCode, HashMap<String, PortSpec>>, // key -> spec
}

impl RuntimeRegistry {
    pub fn new_default() -> Self {
        let mut r = Self { by_code: HashMap::new(), type_to_code: HashMap::new(), in_ports: HashMap::new(), out_ports: HashMap::new() };
        // NOTE: runtime only accepts stable `type_id` strings; UI names must never affect cook/export.
        r.register(OP_INPUT, &["cunning.input"], Arc::new(InputOp));
        r.register(OP_OUTPUT, &["cunning.output"], Arc::new(OutputOp));
        r.register(OP_BOOLEAN, &["cunning.modeling.boolean"], Arc::new(BooleanOp));
        r.register(OP_POLY_EXTRUDE, &["cunning.modeling.poly_extrude"], Arc::new(PolyExtrudeOp));
        r.register(OP_POLY_BEVEL, &["cunning.modeling.poly_bevel"], Arc::new(PolyBevelOp));
        r.register(OP_GROUP_CREATE, &["cunning.group.create"], Arc::new(GroupCreateOp));
        r.register(OP_MERGE, &["cunning.utility.merge"], Arc::new(MergeOp));
        r.register(OP_VOXEL_EDIT, &["cunning.voxel.edit"], Arc::new(VoxelEditOp));

        // Port tables (stable PortId per op). PortId ordering defines VM input ordering.
        // OP_INPUT: no geometry input ports; "index" is a parameter selecting external input.
        r.ports_out(OP_INPUT, &[("out:0", 0, "Output", MultiWirePolicy::Error)]);

        r.ports_in(OP_OUTPUT, &[("in:0", 0, "Input", MultiWirePolicy::Error)]);
        // OP_OUTPUT: sink node; no geometry output ports.

        r.ports_in(OP_BOOLEAN, &[("in:a", 0, "Geometry A", MultiWirePolicy::Error), ("in:b", 1, "Geometry B", MultiWirePolicy::Error)]);
        r.ports_out(OP_BOOLEAN, &[("out:0", 0, "Output", MultiWirePolicy::Error)]);

        r.ports_in(OP_POLY_EXTRUDE, &[("in:0", 0, "Input", MultiWirePolicy::Error)]);
        r.ports_out(OP_POLY_EXTRUDE, &[("out:0", 0, "Output", MultiWirePolicy::Error)]);

        r.ports_in(OP_POLY_BEVEL, &[("in:0", 0, "Input", MultiWirePolicy::Error)]);
        r.ports_out(OP_POLY_BEVEL, &[("out:0", 0, "Output", MultiWirePolicy::Error)]);

        r.ports_in(OP_GROUP_CREATE, &[("in:0", 0, "Input", MultiWirePolicy::Error)]);
        r.ports_out(OP_GROUP_CREATE, &[("out:0", 0, "Output", MultiWirePolicy::Error)]);

        // Merge: Bar/Collection append maps to true array-input semantics at runtime.
        r.ports_in(OP_MERGE, &[("in:0", 0, "Input", MultiWirePolicy::All)]);
        r.ports_out(OP_MERGE, &[("out:0", 0, "Output", MultiWirePolicy::Error)]);

        r.ports_in(OP_VOXEL_EDIT, &[("in:0", 0, "Input", MultiWirePolicy::Error)]);
        r.ports_out(OP_VOXEL_EDIT, &[("out:0", 0, "Output", MultiWirePolicy::Error)]);

        r
    }

    pub fn op_code_for_type(&self, type_id: &str) -> Option<OpCode> { self.type_to_code.get(type_id).copied() }
    pub fn op(&self, op: OpCode) -> Option<Arc<dyn RuntimeOp>> { self.by_code.get(&op).cloned() }
    pub fn port_id(&self, op: OpCode, is_input: bool, name: &str) -> Option<PortId> {
        if is_input { self.in_ports.get(&op).and_then(|m| m.get(name).map(|x| x.id)) }
        else { self.out_ports.get(&op).and_then(|m| m.get(name).map(|x| x.id)) }
    }

    pub fn port_label(&self, op: OpCode, is_input: bool, id: PortId) -> Option<String> {
        let m = if is_input { self.in_ports.get(&op) } else { self.out_ports.get(&op) }?;
        m.values().find(|s| s.id == id).map(|s| s.label.clone())
    }

    pub fn port_key_by_label(&self, op: OpCode, is_input: bool, label: &str) -> Option<String> {
        let m = if is_input { self.in_ports.get(&op) } else { self.out_ports.get(&op) }?;
        m.iter().find(|(_k, s)| s.label == label).map(|(k, _)| k.clone())
    }

    pub fn in_port_specs(&self, op: OpCode) -> Vec<(PortId, String, MultiWirePolicy)> {
        let mut v: Vec<(PortId, String, MultiWirePolicy)> = self.in_ports.get(&op).map(|m| m.values().map(|s| (s.id, s.label.clone(), s.policy)).collect()).unwrap_or_default();
        v.sort_by_key(|x| x.0);
        v.dedup_by_key(|x| x.0);
        v
    }

    pub fn in_port_policy_by_key(&self, op: OpCode, key: &str) -> Option<MultiWirePolicy> { self.in_ports.get(&op).and_then(|m| m.get(key).map(|s| s.policy)) }

    fn register(&mut self, code: OpCode, type_ids: &[&str], op: Arc<dyn RuntimeOp>) {
        self.by_code.insert(code, op);
        for &t in type_ids { self.type_to_code.insert(t.to_string(), code); }
    }

    fn ports_in(&mut self, op: OpCode, ports: &[(&str, PortId, &str, MultiWirePolicy)]) {
        let m = self.in_ports.entry(op).or_default();
        for (k, id, label, policy) in ports { m.insert((*k).to_string(), PortSpec { id: *id, label: (*label).to_string(), policy: *policy }); }
    }
    fn ports_out(&mut self, op: OpCode, ports: &[(&str, PortId, &str, MultiWirePolicy)]) {
        let m = self.out_ports.entry(op).or_default();
        for (k, id, label, policy) in ports { m.insert((*k).to_string(), PortSpec { id: *id, label: (*label).to_string(), policy: *policy }); }
    }
}

struct InputOp;
impl RuntimeOp for InputOp {
    fn compute(&self, _node_id: NodeId, params: &HashMap<String, ParameterValue>, _inputs: &[PortInputs], external_inputs: &[Arc<Geometry>]) -> Result<Arc<Geometry>, CdaCookError> {
        let idx = params.get("index").and_then(|v| match v { ParameterValue::Int(i) => Some(*i as usize), _ => None }).unwrap_or(0);
        Ok(external_inputs.get(idx).cloned().unwrap_or_else(|| Arc::new(Geometry::new())))
    }
}

struct OutputOp;
impl RuntimeOp for OutputOp {
    fn compute(&self, _node_id: NodeId, _params: &HashMap<String, ParameterValue>, inputs: &[PortInputs], _external_inputs: &[Arc<Geometry>]) -> Result<Arc<Geometry>, CdaCookError> {
        Ok(inputs.get(0).and_then(|p| p.values.get(0)).cloned().unwrap_or_else(|| Arc::new(Geometry::new())))
    }
}

struct BooleanOp;
impl RuntimeOp for BooleanOp {
    fn compute(&self, _node_id: NodeId, params: &HashMap<String, ParameterValue>, inputs: &[PortInputs], _external_inputs: &[Arc<Geometry>]) -> Result<Arc<Geometry>, CdaCookError> {
        let a = inputs.get(0).and_then(|p| p.values.get(0)).cloned().unwrap_or_else(|| Arc::new(Geometry::new()));
        let b = inputs.get(1).and_then(|p| p.values.get(0)).cloned().unwrap_or_else(|| Arc::new(Geometry::new()));
        Ok(Arc::new(cunning_kernel::nodes::modeling::boolean::compute_boolean(&[a, b], params)))
    }
}

struct PolyExtrudeOp;
impl RuntimeOp for PolyExtrudeOp {
    fn compute(&self, _node_id: NodeId, params: &HashMap<String, ParameterValue>, inputs: &[PortInputs], _external_inputs: &[Arc<Geometry>]) -> Result<Arc<Geometry>, CdaCookError> {
        use cunning_kernel::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
        use cunning_kernel::nodes::modeling::poly_extrude::PolyExtrudeNode;
        let mut p = <PolyExtrudeNode as NodeParameters>::define_parameters();
        for (k, v) in params { if let Some(pp) = p.iter_mut().find(|pp| pp.name == *k) { pp.value = v.clone(); } }
        let in0 = inputs.get(0).and_then(|p| p.values.get(0)).cloned().unwrap_or_else(|| Arc::new(Geometry::new()));
        Ok(PolyExtrudeNode::default().compute(&p, &[in0]))
    }
}

struct PolyBevelOp;
impl RuntimeOp for PolyBevelOp {
    fn compute(&self, _node_id: NodeId, params: &HashMap<String, ParameterValue>, inputs: &[PortInputs], _external_inputs: &[Arc<Geometry>]) -> Result<Arc<Geometry>, CdaCookError> {
        use cunning_kernel::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
        use cunning_kernel::nodes::modeling::poly_bevel::PolyBevelNode;
        let mut p = <PolyBevelNode as NodeParameters>::define_parameters();
        for (k, v) in params { if let Some(pp) = p.iter_mut().find(|pp| pp.name == *k) { pp.value = v.clone(); } }
        let in0 = inputs.get(0).and_then(|p| p.values.get(0)).cloned().unwrap_or_else(|| Arc::new(Geometry::new()));
        Ok(PolyBevelNode::default().compute(&p, &[in0]))
    }
}

struct GroupCreateOp;
impl RuntimeOp for GroupCreateOp {
    fn compute(&self, _node_id: NodeId, params: &HashMap<String, ParameterValue>, inputs: &[PortInputs], _external_inputs: &[Arc<Geometry>]) -> Result<Arc<Geometry>, CdaCookError> {
        use cunning_kernel::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
        use cunning_kernel::nodes::group::group_create::GroupCreateNode;
        let mut p = <GroupCreateNode as NodeParameters>::define_parameters();
        for (k, v) in params { if let Some(pp) = p.iter_mut().find(|pp| pp.name == *k) { pp.value = v.clone(); } }
        let in0 = inputs.get(0).and_then(|p| p.values.get(0)).cloned().unwrap_or_else(|| Arc::new(Geometry::new()));
        Ok(GroupCreateNode::default().compute(&p, &[in0]))
    }
}

struct MergeOp;
impl RuntimeOp for MergeOp {
    fn compute(&self, _node_id: NodeId, _params: &HashMap<String, ParameterValue>, inputs: &[PortInputs], _external_inputs: &[Arc<Geometry>]) -> Result<Arc<Geometry>, CdaCookError> {
        let ins = inputs.get(0).map(|p| p.values.as_slice()).unwrap_or(&[]);
        let refs: Vec<&Geometry> = ins.iter().map(|g| g.as_ref()).collect();
        Ok(Arc::new(cunning_kernel::libs::algorithms::merge::merge_geometry(refs)))
    }
}

struct VoxelEditOp;
impl RuntimeOp for VoxelEditOp {
    fn compute(
        &self,
        _node_id: NodeId,
        params: &HashMap<String, ParameterValue>,
        inputs: &[PortInputs],
        _external_inputs: &[Arc<Geometry>],
    ) -> Result<Arc<Geometry>, CdaCookError> {
        let in0 = inputs
            .get(0)
            .and_then(|p| p.values.get(0))
            .cloned()
            .unwrap_or_else(|| Arc::new(Geometry::new()));
        Ok(Arc::new(
            cunning_kernel::nodes::voxel::voxel_edit::compute_voxel_edit(None, &in0, params),
        ))
    }
}

pub fn op_failed(asset_uuid: uuid::Uuid, asset_name: &str, node_id: NodeId, op: OpCode, message: String) -> CdaCookError {
    CdaCookError { asset_uuid, asset_name: asset_name.to_string(), node_id: Some(node_id), op: Some(op), port: None, param: None, kind: CdaCookErrorKind::OpFailed { message } }
}

