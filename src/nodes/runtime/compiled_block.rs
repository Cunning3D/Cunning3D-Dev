use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::cunning_core::traits::node_interface::NodeOp;
use crate::libs::algorithms::{merge, transform};
use crate::libs::geometry::geo_ref::{ForEachMeta, GeometryRef};
use crate::mesh::{Attribute, Geometry};
use crate::nodes::attribute::attribute_promote::AttributePromoteParams;
use crate::nodes::runtime::execution_context::CompiledExecutionContext;
use crate::nodes::structs::GeoCacheRef;
use crate::nodes::{port_key, ConnectionId, Node, NodeGraph, NodeId, NodeType, PortId};
use bevy::prelude::{EulerRot, Quat, Vec3};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum InputSource {
    Slot(usize),
    External(usize),
    Empty,
}

#[derive(Clone)]
enum OpKind {
    NodeOp {
        op: Arc<dyn NodeOp>,
    },
    Cda {
        asset_ref: crate::cunning_core::cda::CdaAssetRef,
        input_ports: Vec<PortId>,
        output_ports: Vec<PortId>,
        params_map: Arc<HashMap<String, crate::nodes::parameter::ParameterValue>>,
    },
    ForEachMeta,
    Transform {
        params: TransformParams,
    },
    Merge,
    Boolean {
        params_map: Arc<HashMap<String, crate::nodes::parameter::ParameterValue>>,
    },
    AttributePromote {
        params: Arc<AttributePromoteParams>,
    },
    GroupCreate {
        params_map: Arc<HashMap<String, crate::nodes::parameter::ParameterValue>>,
    },
    GroupCombine {
        params_map: Arc<HashMap<String, crate::nodes::parameter::ParameterValue>>,
    },
    GroupPromote {
        params_map: Arc<HashMap<String, crate::nodes::parameter::ParameterValue>>,
    },
    GroupManage {
        params_map: Arc<HashMap<String, crate::nodes::parameter::ParameterValue>>,
    },
    GroupNormalize {
        params_map: Arc<HashMap<String, crate::nodes::parameter::ParameterValue>>,
    },
}

#[derive(Clone)]
pub struct CompiledOp {
    pub node_id: NodeId,
    pub kind: OpKind,
    pub params: Vec<crate::nodes::parameter::Parameter>,
    pub inputs: Vec<InputSource>,
    pub outputs: Vec<usize>,
}

#[derive(Clone, Copy)]
struct TransformParams {
    translate: Vec3,
    rotation: Quat,
    scale: Vec3,
}

#[derive(Clone)]
pub struct CompiledBlock {
    pub block_id: String,
    pub begin_id: NodeId,
    pub end_id: NodeId,
    pub ops: Vec<CompiledOp>,
    pub sink: InputSource, // where End.in0 comes from (already resolved)
    pub sink_fb: Option<InputSource>, // where End.in1 comes from (already resolved)
    pub inside: HashSet<NodeId>,
    pub external_keys: Vec<(NodeId, PortId)>,
    pub loop_in_slot: usize,
    pub slot_count: usize,
}

impl std::fmt::Debug for CompiledBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledBlock")
            .field("block_id", &self.block_id)
            .field("begin_id", &self.begin_id)
            .field("end_id", &self.end_id)
            .field("op_count", &self.ops.len())
            .field("inside_count", &self.inside.len())
            .finish()
    }
}

fn sorted_conns_to(g: &NodeGraph, to: NodeId) -> Vec<(PortId, i32, NodeId, PortId, ConnectionId)> {
    let mut conns: Vec<(PortId, i32, NodeId, PortId, ConnectionId)> = g
        .connections
        .values()
        .filter(|c| c.to_node == to)
        .map(|c| (c.to_port, c.order, c.from_node, c.from_port, c.id))
        .collect();
    conns.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.4.cmp(&b.4)));
    conns
}

fn select_ports_for_order(n: &Node) -> Option<Vec<PortId>> {
    if matches!(n.node_type, NodeType::Boolean) {
        return Some(vec![port_key::in_a(), port_key::in_b()]);
    }
    None
}

#[derive(Clone)]
enum RawInputSource {
    LoopIn,
    External((NodeId, PortId)),
    NodeOut { node: NodeId, port: PortId },
    Empty,
}

fn input_sources_for_node(
    g: &NodeGraph,
    n: &Node,
    begin_id: NodeId,
    inside: &HashSet<NodeId>,
) -> Vec<RawInputSource> {
    if let Some(ports) = select_ports_for_order(n) {
        return ports
            .into_iter()
            .map(|to_port| {
                let src = g
                    .connections
                    .values()
                    .filter(|c| c.to_node == n.id && c.to_port == to_port)
                    .min_by(|a, b| a.id.cmp(&b.id))
                    .map(|c| (c.from_node, c.from_port));
                match src {
                    Some((sn, sp)) if sn == begin_id && port_key::is_out0(&sp) => {
                        RawInputSource::LoopIn
                    }
                    Some((sn, sp)) if inside.contains(&sn) => {
                        RawInputSource::NodeOut { node: sn, port: sp }
                    }
                    Some((sn, sp)) => RawInputSource::External((sn, sp)),
                    None => RawInputSource::Empty,
                }
            })
            .collect();
    }
    sorted_conns_to(g, n.id)
        .into_iter()
        .map(|(_tp, _ord, sn, sp, _id)| {
            if sn == begin_id && port_key::is_out0(&sp) {
                RawInputSource::LoopIn
            } else if inside.contains(&sn) {
                RawInputSource::NodeOut { node: sn, port: sp }
            } else {
                RawInputSource::External((sn, sp))
            }
        })
        .collect()
}

fn foreach_meta_geo(m: ForEachMeta) -> Arc<Geometry> {
    let mut g = Geometry::new();
    g.insert_detail_attribute("iteration", Attribute::new(vec![m.iteration]));
    g.insert_detail_attribute("numiterations", Attribute::new(vec![m.numiterations]));
    g.insert_detail_attribute("value", Attribute::new(vec![m.value]));
    g.insert_detail_attribute("ivalue", Attribute::new(vec![m.ivalue]));
    Arc::new(g)
}

fn params_to_map(
    params: &[crate::nodes::parameter::Parameter],
) -> HashMap<String, crate::nodes::parameter::ParameterValue> {
    use crate::nodes::parameter::ParameterValue;
    let mut map = HashMap::new();
    for p in params {
        match &p.value {
            ParameterValue::Float(v) => {
                map.insert(p.name.clone(), ParameterValue::Float(*v));
            }
            ParameterValue::Int(v) => {
                map.insert(p.name.clone(), ParameterValue::Int(*v));
            }
            ParameterValue::Bool(v) => {
                map.insert(p.name.clone(), ParameterValue::Bool(*v));
            }
            ParameterValue::String(s) => {
                map.insert(p.name.clone(), ParameterValue::String(s.clone()));
            }
            ParameterValue::Vec3(v) => {
                map.insert(p.name.clone(), ParameterValue::Vec3(*v));
            }
            ParameterValue::Color(v) => {
                map.insert(p.name.clone(), ParameterValue::Color(*v));
            }
            ParameterValue::Curve(c) => {
                map.insert(p.name.clone(), ParameterValue::Curve(c.clone()));
            }
            _ => {}
        }
    }
    map
}

fn transform_params(params: &[crate::nodes::parameter::Parameter]) -> TransformParams {
    use crate::nodes::parameter::ParameterValue;
    let mut translate = Vec3::ZERO;
    let mut rotate_deg = Vec3::ZERO;
    let mut scale = Vec3::ONE;
    let mut uniform_scale = 1.0f32;
    for p in params {
        match (&*p.name, &p.value) {
            ("translate", ParameterValue::Vec3(v)) => translate = *v,
            ("rotate", ParameterValue::Vec3(v)) => rotate_deg = *v,
            ("scale", ParameterValue::Vec3(v)) => scale = *v,
            ("uniform_scale", ParameterValue::Float(v)) => uniform_scale = *v,
            _ => {}
        }
    }
    let rotation = Quat::from_euler(
        EulerRot::XYZ,
        rotate_deg.x.to_radians(),
        rotate_deg.y.to_radians(),
        rotate_deg.z.to_radians(),
    );
    let final_scale = scale * uniform_scale;
    TransformParams {
        translate,
        rotation,
        scale: final_scale,
    }
}

impl CompiledBlock {
    pub fn compile(
        g: &NodeGraph,
        registry: &NodeRegistry,
        end_id: NodeId,
        begin_id: NodeId,
        block_id: String,
        inside: HashSet<NodeId>,
    ) -> Option<Self> {
        let mut slot_map: HashMap<(NodeId, PortId), usize> = HashMap::new();
        let mut external_keys: Vec<(NodeId, PortId)> = Vec::new();
        let mut external_index: HashMap<(NodeId, PortId), usize> = HashMap::new();
        let mut next_slot = 0usize;
        let mut slot_for = |node: NodeId,
                            port: PortId,
                            slot_map: &mut HashMap<(NodeId, PortId), usize>,
                            next_slot: &mut usize|
         -> usize {
            if let Some(&s) = slot_map.get(&(node, port)) {
                return s;
            }
            let s = *next_slot;
            *next_slot += 1;
            slot_map.insert((node, port), s);
            s
        };
        let loop_in_slot = slot_for(begin_id, port_key::out0(), &mut slot_map, &mut next_slot);

        let sink_src = g
            .connections
            .values()
            .filter(|c| c.to_node == end_id && c.to_port == port_key::in0())
            .min_by(|a, b| a.id.cmp(&b.id))
            .map(|c| (c.from_node, c.from_port));
        let sink_raw = match sink_src {
            Some((sn, sp)) if sn == begin_id && port_key::is_out0(&sp) => RawInputSource::LoopIn,
            Some((sn, sp)) if inside.contains(&sn) => {
                RawInputSource::NodeOut { node: sn, port: sp }
            }
            Some((sn, sp)) => RawInputSource::External((sn, sp)),
            None => RawInputSource::Empty,
        };
        let sink_fb_src = g
            .connections
            .values()
            .filter(|c| c.to_node == end_id && c.to_port == port_key::in1())
            .min_by(|a, b| a.id.cmp(&b.id))
            .map(|c| (c.from_node, c.from_port));
        let sink_fb_raw = sink_fb_src.map(|(sn, sp)| {
            if sn == begin_id && port_key::is_out0(&sp) {
                RawInputSource::LoopIn
            } else if inside.contains(&sn) {
                RawInputSource::NodeOut { node: sn, port: sp }
            } else {
                RawInputSource::External((sn, sp))
            }
        });

        // Backward collect only nodes needed for sink, limited to inside.
        let mut need: HashSet<NodeId> = HashSet::new();
        let mut stack: Vec<NodeId> = Vec::new();
        if let RawInputSource::NodeOut { node, .. } = &sink_raw {
            stack.push(*node);
        }
        if let Some(RawInputSource::NodeOut { node, .. }) = &sink_fb_raw {
            stack.push(*node);
        }
        while let Some(nid) = stack.pop() {
            if !inside.contains(&nid) || !need.insert(nid) {
                continue;
            }
            let Some(n) = g.nodes.get(&nid) else {
                continue;
            };
            for src in input_sources_for_node(g, n, begin_id, &inside) {
                if let RawInputSource::NodeOut { node, .. } = src {
                    stack.push(node);
                }
            }
        }

        // Toposort needed nodes (Kahn) based on inside dependencies.
        let mut indeg: HashMap<NodeId, usize> = HashMap::new();
        let mut deps: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for &nid in &need {
            indeg.insert(nid, 0);
        }
        for &nid in &need {
            let Some(n) = g.nodes.get(&nid) else {
                continue;
            };
            for src in input_sources_for_node(g, n, begin_id, &inside) {
                if let RawInputSource::NodeOut { node: sn, .. } = src {
                    if need.contains(&sn) {
                        *indeg.get_mut(&nid).unwrap() += 1;
                        deps.entry(sn).or_default().push(nid);
                    }
                }
            }
        }
        let mut q: std::collections::VecDeque<NodeId> = indeg
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(&n, _)| n)
            .collect();
        let mut order: Vec<NodeId> = Vec::with_capacity(need.len());
        while let Some(nid) = q.pop_front() {
            order.push(nid);
            if let Some(ds) = deps.get(&nid) {
                for &m in ds {
                    if let Some(d) = indeg.get_mut(&m) {
                        *d -= 1;
                        if *d == 0 {
                            q.push_back(m);
                        }
                    }
                }
            }
        }
        if order.len() != need.len() {
            return None;
        }

        let to_slot = |src: RawInputSource,
                       loop_in_slot: usize,
                       slot_map: &mut HashMap<(NodeId, PortId), usize>,
                       next_slot: &mut usize,
                       external_index: &mut HashMap<(NodeId, PortId), usize>,
                       external_keys: &mut Vec<(NodeId, PortId)>|
         -> InputSource {
            match src {
                RawInputSource::LoopIn => InputSource::Slot(loop_in_slot),
                RawInputSource::NodeOut { node, port } => {
                    InputSource::Slot(slot_for(node, port, slot_map, next_slot))
                }
                RawInputSource::External((node, port)) => {
                    let idx = *external_index.entry((node, port)).or_insert_with(|| {
                        let i = external_keys.len();
                        external_keys.push((node, port));
                        i
                    });
                    InputSource::External(idx)
                }
                RawInputSource::Empty => InputSource::Empty,
            }
        };

        let sink = to_slot(
            sink_raw,
            loop_in_slot,
            &mut slot_map,
            &mut next_slot,
            &mut external_index,
            &mut external_keys,
        );
        let sink_fb = sink_fb_raw.map(|s| {
            to_slot(
                s,
                loop_in_slot,
                &mut slot_map,
                &mut next_slot,
                &mut external_index,
                &mut external_keys,
            )
        });

        let mut ops: Vec<CompiledOp> = Vec::new();
        for nid in order {
            let n = g.nodes.get(&nid)?.clone();
            // Safety: nested foreach blocks are not compiled in Phase1 (fallback to interpreter).
            if matches!(&n.node_type, NodeType::Generic(s) if s == "ForEach Begin" || s == "ForEach End")
            {
                return None;
            }
            let inputs_raw = input_sources_for_node(g, &n, begin_id, &inside);
            let inputs = inputs_raw
                .into_iter()
                .map(|src| {
                    to_slot(
                        src,
                        loop_in_slot,
                        &mut slot_map,
                        &mut next_slot,
                        &mut external_index,
                        &mut external_keys,
                    )
                })
                .collect();
            let params = n.parameters.clone();
            let (kind, outputs) = match &n.node_type {
                NodeType::CDA(data) => {
                    let (ins, outs) = crate::cunning_core::cda::library::global_cda_library()
                        .and_then(|lib| lib.get(data.asset_ref.uuid))
                        .map(|a| {
                            (
                                a.inputs
                                    .iter()
                                    .map(|p| PortId::from(p.port_key().as_str()))
                                    .collect(),
                                a.outputs
                                    .iter()
                                    .map(|p| PortId::from(p.port_key().as_str()))
                                    .collect(),
                            )
                        })
                        .unwrap_or_else(|| (vec![port_key::in0()], vec![port_key::out0()]));
                    let pm = Arc::new(params_to_map(&params));
                    let out_slots: Vec<usize> = outs
                        .iter()
                        .map(|p| slot_for(nid, *p, &mut slot_map, &mut next_slot))
                        .collect();
                    (
                        OpKind::Cda {
                            asset_ref: data.asset_ref.clone(),
                            input_ports: ins.clone(),
                            output_ports: outs.clone(),
                            params_map: pm,
                        },
                        out_slots,
                    )
                }
                NodeType::Generic(s) if s == "ForEach Meta" => (
                    OpKind::ForEachMeta,
                    vec![slot_for(
                        nid,
                        port_key::out0(),
                        &mut slot_map,
                        &mut next_slot,
                    )],
                ),
                NodeType::Transform => {
                    let tp = transform_params(&params);
                    (
                        OpKind::Transform { params: tp },
                        vec![slot_for(
                            nid,
                            port_key::out0(),
                            &mut slot_map,
                            &mut next_slot,
                        )],
                    )
                }
                NodeType::Merge => (
                    OpKind::Merge,
                    vec![slot_for(
                        nid,
                        port_key::out0(),
                        &mut slot_map,
                        &mut next_slot,
                    )],
                ),
                NodeType::Boolean => {
                    let pm = Arc::new(params_to_map(&params));
                    (
                        OpKind::Boolean { params_map: pm },
                        vec![slot_for(
                            nid,
                            port_key::out0(),
                            &mut slot_map,
                            &mut next_slot,
                        )],
                    )
                }
                NodeType::AttributePromote(_) => {
                    let pm = params_to_map(&params);
                    let pp = Arc::new(
                        crate::nodes::attribute::attribute_promote::parse_promote_params(&pm),
                    );
                    (
                        OpKind::AttributePromote { params: pp },
                        vec![slot_for(
                            nid,
                            port_key::out0(),
                            &mut slot_map,
                            &mut next_slot,
                        )],
                    )
                }
                NodeType::GroupCreate => {
                    let pm = Arc::new(params_to_map(&params));
                    (
                        OpKind::GroupCreate { params_map: pm },
                        vec![slot_for(
                            nid,
                            port_key::out0(),
                            &mut slot_map,
                            &mut next_slot,
                        )],
                    )
                }
                NodeType::GroupCombine => {
                    let pm = Arc::new(params_to_map(&params));
                    (
                        OpKind::GroupCombine { params_map: pm },
                        vec![slot_for(
                            nid,
                            port_key::out0(),
                            &mut slot_map,
                            &mut next_slot,
                        )],
                    )
                }
                NodeType::GroupPromote => {
                    let pm = Arc::new(params_to_map(&params));
                    (
                        OpKind::GroupPromote { params_map: pm },
                        vec![slot_for(
                            nid,
                            port_key::out0(),
                            &mut slot_map,
                            &mut next_slot,
                        )],
                    )
                }
                NodeType::GroupManage => {
                    let pm = Arc::new(params_to_map(&params));
                    (
                        OpKind::GroupManage { params_map: pm },
                        vec![slot_for(
                            nid,
                            port_key::out0(),
                            &mut slot_map,
                            &mut next_slot,
                        )],
                    )
                }
                NodeType::GroupNormalize => {
                    let pm = Arc::new(params_to_map(&params));
                    (
                        OpKind::GroupNormalize { params_map: pm },
                        vec![slot_for(
                            nid,
                            port_key::out0(),
                            &mut slot_map,
                            &mut next_slot,
                        )],
                    )
                }
                _ => {
                    let name = n.node_type.name();
                    let op = registry.create_op(name).map(Arc::from)?;
                    (
                        OpKind::NodeOp { op },
                        vec![slot_for(
                            nid,
                            port_key::out0(),
                            &mut slot_map,
                            &mut next_slot,
                        )],
                    )
                }
            };
            ops.push(CompiledOp {
                node_id: nid,
                kind,
                params,
                inputs,
                outputs,
            });
        }

        let slot_count = next_slot;
        Some(Self {
            block_id,
            begin_id,
            end_id,
            ops,
            sink,
            sink_fb,
            inside,
            external_keys,
            loop_in_slot,
            slot_count,
        })
    }

    #[inline]
    pub fn external_keys(&self) -> &[(NodeId, PortId)] {
        &self.external_keys
    }

    pub fn run_iter(
        &self,
        registry: &NodeRegistry,
        ctx: &mut CompiledExecutionContext,
        loop_in: GeoCacheRef,
    ) -> (GeoCacheRef, GeoCacheRef) {
        if ctx.slot_cache.len() != self.slot_count {
            ctx.slot_cache
                .resize(self.slot_count, CompiledExecutionContext::empty_geo());
        }
        ctx.slot_cache[self.loop_in_slot] = loop_in;
        for cop in &self.ops {
            let out = match &cop.kind {
                OpKind::ForEachMeta => GeoCacheRef::Geo(foreach_meta_geo(ctx.current_meta.clone())),
                OpKind::Cda {
                    asset_ref,
                    input_ports,
                    output_ports,
                    params_map,
                } => {
                    ctx.scratch_inputs.clear();
                    ctx.scratch_inputs.reserve(input_ports.len());
                    for (i, _port) in input_ports.iter().enumerate() {
                        let src = cop.inputs.get(i).cloned().unwrap_or(InputSource::Empty);
                        let gref = self.eval_src(ctx, src).as_georef();
                        ctx.scratch_inputs.push(gref);
                    }
                    let outs = crate::cunning_core::cda::library::global_cda_library()
                        .map(|lib| {
                            lib.cook(
                                Some(cop.node_id),
                                asset_ref,
                                params_map,
                                &ctx.scratch_inputs,
                                registry,
                            )
                        })
                        .unwrap_or_default();
                    for (i, port) in output_ports.iter().enumerate() {
                        let g = outs
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| GeoCacheRef::empty().as_geo());
                        let slot = cop.outputs.get(i).copied().unwrap_or(0);
                        ctx.slot_cache[slot] = GeoCacheRef::Geo(g);
                    }
                    ctx.slot_cache
                        .get(*cop.outputs.get(0).unwrap_or(&0))
                        .cloned()
                        .unwrap_or_else(CompiledExecutionContext::empty_geo)
                }
                OpKind::NodeOp { op: nodeop } => {
                    ctx.scratch_inputs.clear();
                    ctx.scratch_inputs.reserve(cop.inputs.len());
                    for src in cop.inputs.iter().cloned() {
                        let gref = self.eval_src(ctx, src).as_georef();
                        ctx.scratch_inputs.push(gref);
                    }
                    GeoCacheRef::Geo(nodeop.compute(&cop.params, &ctx.scratch_inputs))
                }
                OpKind::Transform { params } => {
                    let input = cop
                        .inputs
                        .get(0)
                        .cloned()
                        .map(|s| self.eval_src(ctx, s).as_geo())
                        .unwrap_or_else(|| GeoCacheRef::empty().as_geo());
                    GeoCacheRef::Geo(Arc::new(transform::transform_geometry_quat(
                        &input,
                        params.translate,
                        params.rotation,
                        params.scale,
                    )))
                }
                OpKind::Merge => {
                    ctx.scratch_gpu.clear();
                    ctx.scratch_geos.clear();
                    let mut all_gpu = true;
                    for src in cop.inputs.iter().cloned() {
                        match self.eval_src(ctx, src) {
                            GeoCacheRef::Gpu(h) => ctx.scratch_gpu.push(h),
                            other => {
                                all_gpu = false;
                                ctx.scratch_geos.push(other.as_geo());
                            }
                        }
                    }
                    if all_gpu {
                        if let Some(h) =
                            crate::nodes::gpu::runtime::merge_appendable_points(&ctx.scratch_gpu)
                        {
                            GeoCacheRef::Gpu(h)
                        } else {
                            GeoCacheRef::Geo(Arc::new(merge::merge_geometry_arcs(
                                &ctx.scratch_geos,
                            )))
                        }
                    } else {
                        GeoCacheRef::Geo(Arc::new(merge::merge_geometry_arcs(&ctx.scratch_geos)))
                    }
                }
                OpKind::Boolean { params_map } => {
                    let a = cop
                        .inputs
                        .get(0)
                        .cloned()
                        .map(|s| self.eval_src(ctx, s).as_geo())
                        .unwrap_or_else(|| GeoCacheRef::empty().as_geo());
                    let b = cop
                        .inputs
                        .get(1)
                        .cloned()
                        .map(|s| self.eval_src(ctx, s).as_geo())
                        .unwrap_or_else(|| GeoCacheRef::empty().as_geo());
                    ctx.scratch_geos.clear();
                    ctx.scratch_geos.push(a);
                    ctx.scratch_geos.push(b);
                    GeoCacheRef::Geo(Arc::new(
                        crate::nodes::modeling::boolean::boolean_node::compute_boolean(
                            &ctx.scratch_geos,
                            params_map,
                        ),
                    ))
                }
                OpKind::AttributePromote { params } => {
                    let input = cop
                        .inputs
                        .get(0)
                        .cloned()
                        .map(|s| self.eval_src(ctx, s).as_geo())
                        .unwrap_or_else(|| GeoCacheRef::empty().as_geo());
                    match crate::nodes::attribute::attribute_promote::promote_with_params(
                        &input, params,
                    ) {
                        Ok(res) => GeoCacheRef::Geo(Arc::new(res)),
                        Err(_) => GeoCacheRef::Geo(input),
                    }
                }
                OpKind::GroupCreate { params_map } => {
                    let input = cop
                        .inputs
                        .get(0)
                        .cloned()
                        .map(|s| self.eval_src(ctx, s).as_geo())
                        .unwrap_or_else(|| GeoCacheRef::empty().as_geo());
                    GeoCacheRef::Geo(
                        crate::nodes::group::group_create::GroupCreateNode.compute_params_map(
                            &input,
                            params_map,
                            cop.inputs
                                .get(1)
                                .cloned()
                                .map(|s| self.eval_src(ctx, s).as_geo()),
                        ),
                    )
                }
                OpKind::GroupCombine { params_map } => {
                    let input = cop
                        .inputs
                        .get(0)
                        .cloned()
                        .map(|s| self.eval_src(ctx, s).as_geo())
                        .unwrap_or_else(|| GeoCacheRef::empty().as_geo());
                    GeoCacheRef::Geo(
                        crate::nodes::group::group_combine::GroupCombineNode
                            .compute_params_map(&input, params_map),
                    )
                }
                OpKind::GroupPromote { params_map } => {
                    let input = cop
                        .inputs
                        .get(0)
                        .cloned()
                        .map(|s| self.eval_src(ctx, s).as_geo())
                        .unwrap_or_else(|| GeoCacheRef::empty().as_geo());
                    GeoCacheRef::Geo(
                        crate::nodes::group::group_promote::GroupPromoteNode
                            .compute_params_map(&input, params_map),
                    )
                }
                OpKind::GroupManage { params_map } => {
                    let input = cop
                        .inputs
                        .get(0)
                        .cloned()
                        .map(|s| self.eval_src(ctx, s).as_geo())
                        .unwrap_or_else(|| GeoCacheRef::empty().as_geo());
                    GeoCacheRef::Geo(
                        crate::nodes::group::group_manage::GroupManageNode
                            .compute_params_map(&input, params_map),
                    )
                }
                OpKind::GroupNormalize { params_map } => {
                    let input = cop
                        .inputs
                        .get(0)
                        .cloned()
                        .map(|s| self.eval_src(ctx, s).as_geo())
                        .unwrap_or_else(|| GeoCacheRef::empty().as_geo());
                    GeoCacheRef::Geo(
                        crate::nodes::group::group_normalize::GroupNormalizeNode
                            .compute_params_map(&input, params_map),
                    )
                }
            };
            if cop.outputs.len() == 1 {
                ctx.slot_cache[cop.outputs[0]] = out;
            } else if matches!(cop.kind, OpKind::NodeOp { .. } | OpKind::ForEachMeta) {
                for &p in &cop.outputs {
                    ctx.slot_cache[p] = out.clone();
                }
            }
        }
        let main = self.eval_src(ctx, self.sink.clone());
        let fb = self
            .sink_fb
            .clone()
            .map(|s| self.eval_src(ctx, s))
            .unwrap_or_else(|| main.clone());
        (main, fb)
    }

    fn eval_src(&self, ctx: &mut CompiledExecutionContext, src: InputSource) -> GeoCacheRef {
        match src {
            InputSource::Slot(idx) => ctx
                .slot_cache
                .get(idx)
                .cloned()
                .unwrap_or_else(CompiledExecutionContext::empty_geo),
            InputSource::External(idx) => ctx
                .externals_vec
                .get(idx)
                .cloned()
                .unwrap_or_else(CompiledExecutionContext::empty_geo),
            InputSource::Empty => CompiledExecutionContext::empty_geo(),
        }
    }
}
