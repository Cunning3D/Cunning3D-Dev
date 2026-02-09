use crate::cunning_core::profiling::ComputeRecord;
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::libs::geometry::geo_ref::{ForEachMeta, GeometryRef, GeometryView};
use crate::libs::geometry::group::ElementGroupMask;
use crate::mesh::Geometry;
use crate::nodes::parameter::Parameter;
use crate::nodes::port_key;
use crate::nodes::structs::{GeoCacheRef, NodeGraph, NodeId, NodeParamOverrides, PortId};
use bevy::prelude::Vec3;
use rayon::prelude::*;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[inline]
fn p_str(params: &[Parameter], n: &str, d: &str) -> String {
    params
        .iter()
        .find(|p| p.name == n)
        .and_then(|p| {
            if let crate::nodes::parameter::ParameterValue::String(s) = &p.value {
                Some(s.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| d.to_string())
}

#[inline]
fn p_int(params: &[Parameter], n: &str, d: i32) -> i32 {
    params
        .iter()
        .find(|p| p.name == n)
        .and_then(|p| {
            if let crate::nodes::parameter::ParameterValue::Int(v) = &p.value {
                Some(*v)
            } else {
                None
            }
        })
        .unwrap_or(d)
}

#[inline]
fn p_bool(params: &[Parameter], n: &str, d: bool) -> bool {
    params
        .iter()
        .find(|p| p.name == n)
        .and_then(|p| {
            if let crate::nodes::parameter::ParameterValue::Bool(v) = &p.value {
                Some(*v)
            } else {
                None
            }
        })
        .unwrap_or(d)
}

#[inline]
fn block_key_from_params(ps: &[Parameter]) -> String {
    let uid = p_str(ps, "block_uid", "");
    if !uid.trim().is_empty() {
        return uid;
    }
    p_str(ps, "block_id", "")
}

#[inline]
fn clear_scope_caches(g: &mut NodeGraph, inside: &std::collections::hash_set::HashSet<NodeId>) {
    for nid in inside {
        g.geometry_cache.remove(nid);
    }
    g.geometry_cache_lru.retain(|nid| !inside.contains(nid));
    g.port_geometry_cache
        .retain(|(nid, _), _| !inside.contains(nid));
    g.port_ref_cache.retain(|(nid, _), _| !inside.contains(nid));
}

#[inline]
fn flush_warnings(warns: &[String]) {
    if let Some(c) = crate::console::global_console() {
        for w in warns {
            c.warning(w.clone());
        }
    }
}

#[inline]
fn empty_ref() -> GeoCacheRef {
    GeoCacheRef::empty()
}

#[inline]
fn params_hash(params: &[Parameter]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in params {
        p.name.hash(&mut h);
        match &p.value {
            crate::nodes::parameter::ParameterValue::Int(v) => v.hash(&mut h),
            crate::nodes::parameter::ParameterValue::Bool(v) => v.hash(&mut h),
            crate::nodes::parameter::ParameterValue::Float(v) => v.to_bits().hash(&mut h),
            crate::nodes::parameter::ParameterValue::String(s) => s.hash(&mut h),
            _ => {}
        }
    }
    h.finish()
}

#[inline]
fn piece_key_hash(base_dirty: u64, params: &[Parameter]) -> u64 {
    let (domain, method, attr, count) = (
        p_int(params, "piece_domain", 0),
        p_int(params, "iteration_method", 0),
        p_str(params, "piece_attribute", "class"),
        p_int(params, "count", 1),
    );
    let mut h = std::collections::hash_map::DefaultHasher::new();
    base_dirty.hash(&mut h);
    domain.hash(&mut h);
    method.hash(&mut h);
    attr.hash(&mut h);
    count.hash(&mut h);
    h.finish()
}

#[inline]
fn geo_fingerprint_hash(g: &Geometry) -> u64 {
    let fp = g.compute_fingerprint();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    fp.point_count.hash(&mut h);
    fp.primitive_count.hash(&mut h);
    if let Some(v) = fp.bbox_min {
        for x in v {
            x.to_bits().hash(&mut h);
        }
    }
    if let Some(v) = fp.bbox_max {
        for x in v {
            x.to_bits().hash(&mut h);
        }
    }
    h.finish()
}

#[inline]
fn foreach_block_nodes(
    g: &NodeGraph,
    block_key: &str,
) -> (Option<NodeId>, Option<NodeId>, Option<NodeId>) {
    if let Some(entry) = g.block_id_index.get(block_key) {
        return *entry;
    }
    let mut begin = None;
    let mut end = None;
    let mut meta = None;
    for (id, n) in &g.nodes {
        let uid = n
            .parameters
            .iter()
            .find(|p| p.name == "block_uid")
            .and_then(|p| {
                if let crate::nodes::parameter::ParameterValue::String(s) = &p.value {
                    Some(s.as_str())
                } else {
                    None
                }
            });
        let bid = n
            .parameters
            .iter()
            .find(|p| p.name == "block_id")
            .and_then(|p| {
                if let crate::nodes::parameter::ParameterValue::String(s) = &p.value {
                    Some(s.as_str())
                } else {
                    None
                }
            });
        if uid != Some(block_key) && bid != Some(block_key) {
            continue;
        }
        match &n.node_type {
            crate::nodes::structs::NodeType::Generic(s) if s == "ForEach Begin" => {
                if begin.is_none() {
                    begin = Some(*id);
                }
            }
            crate::nodes::structs::NodeType::Generic(s) if s == "ForEach End" => {
                if end.is_none() {
                    end = Some(*id);
                }
            }
            crate::nodes::structs::NodeType::Generic(s) if s == "ForEach Meta" => {
                if meta.is_none() {
                    meta = Some(*id);
                }
            }
            _ => {}
        }
    }
    (begin, end, meta)
}

#[inline]
fn foreach_reachable(
    g: &NodeGraph,
    begin: NodeId,
    end: NodeId,
    meta: Option<NodeId>,
) -> HashSet<NodeId> {
    let mut out: HashMap<NodeId, Vec<NodeId>> = HashMap::with_capacity(g.connections.len());
    for c in g.connections.values() {
        out.entry(c.from_node).or_default().push(c.to_node);
    }
    let mut reach: HashSet<NodeId> = HashSet::new();
    let mut q: VecDeque<NodeId> = VecDeque::new();
    q.push_back(begin);
    while let Some(nid) = q.pop_front() {
        if !reach.insert(nid) {
            continue;
        }
        if nid == end || Some(nid) == meta {
            continue;
        }
        if let Some(v) = out.get(&nid) {
            for &to in v {
                q.push_back(to);
            }
        }
    }
    reach.insert(begin);
    reach.insert(end);
    if let Some(m) = meta {
        reach.insert(m);
    }
    reach
}

#[inline]
fn reach_cache_key(
    g: &NodeGraph,
    block_key: &str,
    begin: NodeId,
    end: NodeId,
    meta: Option<NodeId>,
) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    g.graph_revision.hash(&mut h);
    block_key.hash(&mut h);
    begin.hash(&mut h);
    end.hash(&mut h);
    meta.hash(&mut h);
    h.finish()
}

#[inline]
fn foreach_reachable_cached(
    g: &mut NodeGraph,
    block_key: &str,
    begin: NodeId,
    end: NodeId,
    meta: Option<NodeId>,
) -> Arc<HashSet<NodeId>> {
    let key = reach_cache_key(g, block_key, begin, end, meta);
    if let Some((k, reach)) = g.foreach_reach_cache.get(&end) {
        if *k == key {
            return reach.clone();
        }
    }
    let reach = Arc::new(foreach_reachable(g, begin, end, meta));
    g.foreach_reach_cache.insert(end, (key, reach.clone()));
    reach
}

pub struct GraphExecutor<'a> {
    g: &'a mut NodeGraph,
}

impl<'a> GraphExecutor<'a> {
    #[inline]
    pub fn new(g: &'a mut NodeGraph) -> Self {
        Self { g }
    }

    pub fn foreach_run_end(
        &mut self,
        end_id: NodeId,
        registry: &NodeRegistry,
        perf_stats: &mut Option<&mut HashMap<NodeId, ComputeRecord>>,
        overrides: Option<&NodeParamOverrides>,
        visiting: &mut std::collections::hash_set::HashSet<NodeId>,
    ) -> (GeoCacheRef, GeoCacheRef) {
        let g = &mut *self.g;
        let end = match g.nodes.get(&end_id) {
            Some(n) => n.clone(),
            None => return (GeoCacheRef::empty(), GeoCacheRef::empty()),
        };
        let block_key = block_key_from_params(&end.parameters);
        if block_key.is_empty() {
            return (GeoCacheRef::empty(), GeoCacheRef::empty());
        }

        let begin_id = g.nodes.iter().find_map(|(id, n)| {
            let ok = matches!(&n.node_type, crate::nodes::structs::NodeType::Generic(s) if s == "ForEach Begin") && {
                let uid = p_str(&n.parameters, "block_uid", "");
                let bid = p_str(&n.parameters, "block_id", "");
                (!uid.is_empty() && uid == block_key) || (!bid.is_empty() && bid == block_key)
            };
            if ok { Some(*id) } else { None }
        });
        let Some(begin_id) = begin_id else {
            return (GeoCacheRef::empty(), GeoCacheRef::empty());
        };

        let base_src = g
            .connections
            .values()
            .filter(|c| c.to_node == begin_id && c.to_port == port_key::in0())
            .min_by(|a, b| a.id.cmp(&b.id))
            .map(|c| (c.from_node, c.from_port.clone()));
        let base_geo = base_src
            .map(|(n, p)| g.compute_output(n, &p, registry, perf_stats, overrides, visiting))
            .unwrap_or_else(|| GeoCacheRef::empty().as_geo());
        let base_dirty = base_geo.dirty_id;
        let begin_method = g
            .nodes
            .get(&begin_id)
            .map(|n| p_int(&n.parameters, "method", 0))
            .unwrap_or(0);

        let (iter_method, gather, max_it, stop_empty, stop_hash, single_pass) = (
            p_int(&end.parameters, "iteration_method", 0),
            p_int(&end.parameters, "gather_method", 0),
            p_int(&end.parameters, "max_iterations", 100).max(1) as usize,
            p_bool(&end.parameters, "stop_when_empty", true),
            p_bool(&end.parameters, "stop_when_unchanged_hash", true),
            p_bool(&end.parameters, "single_pass", false),
        );

        // Cache fast-path before any piece planning/compilation work.
        let bkey = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            base_dirty.hash(&mut h);
            params_hash(&end.parameters).hash(&mut h);
            h.finish()
        };
        if let Some((k, o, f)) = g.foreach_block_cache_ref.get(&end_id) {
            if *k == bkey {
                return (o.clone(), f.clone());
            }
        }

        // --- GPU Loop Fusion (10000x-class optimization) ---
        // If the loop body is a linear chain of "Attribute Kernel (GPU)" nodes and the foreach is by-pieces (no feedback/metadata/early-stop),
        // then running the fused kernel once on the full input is equivalent to running it per-piece and merging.
        let fb_count = g
            .connections
            .values()
            .filter(|c| c.to_node == end_id && c.to_port == port_key::in1())
            .count();
        let body_src = g
            .connections
            .values()
            .filter(|c| c.to_node == end_id && c.to_port == port_key::in0())
            .min_by(|a, b| a.id.cmp(&b.id))
            .map(|c| (c.from_node, c.from_port.clone()));
        if iter_method == 0
            && gather == 0
            && fb_count == 0
            && !single_pass
            && !stop_empty
            && !stop_hash
            && begin_method != 1
            && begin_method != 2
        {
            if let Some((mut cur_id, cur_port)) = body_src.clone() {
                if port_key::is_out0(&cur_port) {
                    let mut chain_rev: Vec<NodeId> = Vec::new();
                    let mut ops_rev: Vec<crate::nodes::gpu::ops::GpuOp> = Vec::new();
                    loop {
                        let Some(cur) = g.nodes.get(&cur_id) else {
                            break;
                        };
                        let is_gpu = matches!(&cur.node_type, crate::nodes::structs::NodeType::Generic(s) if s == "Attribute Kernel (GPU)");
                        if !is_gpu {
                            break;
                        }

                        // Ensure single input from either Begin.out0 or previous GPU.out0, and no extra inputs.
                        let mut in0 = g
                            .connections
                            .values()
                            .filter(|c| c.to_node == cur_id && c.to_port == port_key::in0())
                            .min_by(|a, b| a.id.cmp(&b.id));
                        if in0.is_none() {
                            break;
                        }
                        if g.connections
                            .values()
                            .any(|c| c.to_node == cur_id && c.to_port != port_key::in0())
                        {
                            break;
                        }

                        // Ensure out0 only fans out to either End.in0 or the next node in chain (we check while walking).
                        let out_fanout = g
                            .connections
                            .values()
                            .filter(|c| c.from_node == cur_id && port_key::is_out0(&c.from_port))
                            .count();
                        if out_fanout == 0 {
                            break;
                        }

                        // Lower to GpuOp (must be domain=Points and same attr across chain to be fusable).
                        let op = crate::nodes::gpu::ops::lower_attribute_kernel(&cur.parameters);
                        let crate::nodes::gpu::ops::GpuOp::AffineVec3 { domain, .. } = &op else {
                            break;
                        };
                        if *domain != 0 {
                            break;
                        }

                        chain_rev.push(cur_id);
                        ops_rev.push(op);
                        let Some(conn) = in0.take() else {
                            break;
                        };
                        if conn.from_node == begin_id && port_key::is_out0(&conn.from_port) {
                            break;
                        }
                        // Must come from previous GPU node out0.
                        if !port_key::is_out0(&conn.from_port) {
                            break;
                        }
                        cur_id = conn.from_node;
                    }

                    if !chain_rev.is_empty() {
                        // Validate that the chain is exactly the inside of the foreach block (no extra nodes), and that every GPU node's out0 only goes to End or other nodes in chain.
                        let meta_id = foreach_block_nodes(g, &block_key).2;
                        let reach = foreach_reachable(g, begin_id, end_id, meta_id);
                        let chain: std::collections::HashSet<NodeId> =
                            chain_rev.iter().copied().collect();
                        let mut inside_ok = true;
                        for nid in &reach {
                            if *nid == begin_id || *nid == end_id || Some(*nid) == meta_id {
                                continue;
                            }
                            if !chain.contains(nid) {
                                inside_ok = false;
                                break;
                            }
                        }
                        if inside_ok {
                            for &nid in &chain_rev {
                                for c in g.connections.values().filter(|c| {
                                    c.from_node == nid && port_key::is_out0(&c.from_port)
                                }) {
                                    if c.to_node != end_id && !chain.contains(&c.to_node) {
                                        inside_ok = false;
                                        break;
                                    }
                                }
                                if !inside_ok {
                                    break;
                                }
                            }
                        }
                        if inside_ok {
                            let mut ops = ops_rev;
                            ops.reverse();
                            if let Some(crate::nodes::gpu::ops::GpuOp::AffineVec3 {
                                domain: _,
                                attr,
                                mul,
                                add,
                            }) = crate::nodes::gpu::ops::fold_affine_chain(&ops)
                            {
                                let h = crate::nodes::gpu::runtime::GpuGeoHandle::from_cpu(
                                    base_geo.clone(),
                                )
                                .apply_affine_vec3(
                                    attr.as_str(),
                                    mul,
                                    add,
                                );
                                let out = GeoCacheRef::Gpu(h);
                                g.foreach_block_cache_ref
                                    .insert(end_id, (bkey, out.clone(), out.clone()));
                                return (out.clone(), out);
                            }
                        }
                    }
                }
            }
        }

        let mut warns: Vec<String> = Vec::new();
        let pkey = piece_key_hash(base_dirty, &end.parameters);
        let pieces = if let Some((k, v)) = g.foreach_piece_cache.get(&end_id) {
            if *k == pkey {
                v.clone()
            } else {
                let v: Vec<GeoCacheRef> = {
                    use crate::libs::algorithms::algorithms_runtime::foreach_pieces::{
                        plan_foreach_pieces, ForeachPiecePlanItem as I, ForeachPiecePlanParams as P,
                    };
                    let (domain, method, use_attr, mut attr, count) = (
                        p_int(&end.parameters, "piece_domain", 0),
                        p_int(&end.parameters, "iteration_method", 0),
                        p_bool(&end.parameters, "use_piece_attribute", true),
                        p_str(&end.parameters, "piece_attribute", "class")
                            .trim()
                            .to_string(),
                        p_int(&end.parameters, "count", 1).max(1) as usize,
                    );
                    let a_norm = attr.trim_start_matches('@');
                    let by_index = if domain == 1 { "ptnum" } else { "primnum" };
                    if !use_attr || attr.is_empty() {
                        attr = by_index.into();
                    } else if !matches!(a_norm, "ptnum" | "pointnum" | "primnum" | "primitivenum") {
                        let ok = if domain == 1 {
                            base_geo.get_point_attribute(a_norm).is_some()
                        } else {
                            base_geo.get_primitive_attribute(a_norm).is_some()
                        };
                        if !ok {
                            warns.push(format!(
                                "ForEachEnd: piece attribute '{}' not found; falling back to {}",
                                attr, by_index
                            ));
                            attr = by_index.into();
                        }
                    }
                    plan_foreach_pieces(
                        base_geo.clone(),
                        P {
                            domain,
                            method,
                            attr,
                            count,
                        },
                    )
                    .into_iter()
                    .map(|it| match it {
                        I::FullInput => GeoCacheRef::Geo(base_geo.clone()),
                        I::View(v) => GeoCacheRef::View(v),
                    })
                    .collect()
                };
                g.foreach_piece_cache.insert(end_id, (pkey, v.clone()));
                v
            }
        } else {
            let v: Vec<GeoCacheRef> = {
                use crate::libs::algorithms::algorithms_runtime::foreach_pieces::{
                    plan_foreach_pieces, ForeachPiecePlanItem as I, ForeachPiecePlanParams as P,
                };
                let (domain, method, use_attr, mut attr, count) = (
                    p_int(&end.parameters, "piece_domain", 0),
                    p_int(&end.parameters, "iteration_method", 0),
                    p_bool(&end.parameters, "use_piece_attribute", true),
                    p_str(&end.parameters, "piece_attribute", "class")
                        .trim()
                        .to_string(),
                    p_int(&end.parameters, "count", 1).max(1) as usize,
                );
                let a_norm = attr.trim_start_matches('@');
                let by_index = if domain == 1 { "ptnum" } else { "primnum" };
                if !use_attr || attr.is_empty() {
                    attr = by_index.into();
                } else if !matches!(a_norm, "ptnum" | "pointnum" | "primnum" | "primitivenum") {
                    let ok = if domain == 1 {
                        base_geo.get_point_attribute(a_norm).is_some()
                    } else {
                        base_geo.get_primitive_attribute(a_norm).is_some()
                    };
                    if !ok {
                        warns.push(format!(
                            "ForEachEnd: piece attribute '{}' not found; falling back to {}",
                            attr, by_index
                        ));
                        attr = by_index.into();
                    }
                }
                plan_foreach_pieces(
                    base_geo.clone(),
                    P {
                        domain,
                        method,
                        attr,
                        count,
                    },
                )
                .into_iter()
                .map(|it| match it {
                    I::FullInput => GeoCacheRef::Geo(base_geo.clone()),
                    I::View(v) => GeoCacheRef::View(v),
                })
                .collect()
            };
            g.foreach_piece_cache.insert(end_id, (pkey, v.clone()));
            v
        };
        if gather == 1 && fb_count == 0 {
            warns.push(format!(
                "ForEachEnd feedback mode: Feedback input is not connected (end_id={})",
                end_id
            ));
        }
        if fb_count > 1 {
            warns.push(format!(
                "ForEachEnd feedback input has {} wires (only the first is used) (end_id={})",
                fb_count, end_id
            ));
        }

        let fb_src = g
            .connections
            .values()
            .filter(|c| c.to_node == end_id && c.to_port == port_key::in1())
            .min_by(|a, b| a.id.cmp(&b.id))
            .map(|c| (c.from_node, c.from_port.clone()));

        let mut compiled_opt: Option<Arc<crate::nodes::runtime::compiled_block::CompiledBlock>> =
            if single_pass {
                None
            } else {
                g.foreach_compiled_cache
                    .get(&end_id)
                    .and_then(|(gr, pr, b)| {
                        if *gr == g.graph_revision && *pr == g.param_revision {
                            Some(b.clone())
                        } else {
                            None
                        }
                    })
            };
        let mut reach: HashSet<NodeId> = HashSet::new();
        if compiled_opt.is_none() {
            let meta_id = foreach_block_nodes(g, &block_key).2;
            let reach_set = foreach_reachable_cached(g, &block_key, begin_id, end_id, meta_id);
            reach = reach_set.iter().copied().collect();
            if !single_pass {
                if let Some(b) = crate::nodes::runtime::compiled_block::CompiledBlock::compile(
                    g,
                    registry,
                    end_id,
                    begin_id,
                    block_key.clone(),
                    reach.clone(),
                ) {
                    let b = Arc::new(b);
                    g.foreach_compiled_cache
                        .insert(end_id, (g.graph_revision, g.param_revision, b.clone()));
                    compiled_opt = Some(b);
                }
            }
        }

        let externals_vec: Arc<Vec<GeoCacheRef>> = if let Some(compiled) = &compiled_opt {
            if let Some((gr, pr, v)) = g.foreach_externals_cache.get(&end_id) {
                if *gr == g.graph_revision && *pr == g.param_revision {
                    v.clone()
                } else {
                    let mut v: Vec<GeoCacheRef> =
                        Vec::with_capacity(compiled.external_keys().len());
                    for &(n, port) in compiled.external_keys().iter() {
                        v.push(g.compute_output_ref(
                            n, &port, registry, perf_stats, overrides, visiting,
                        ));
                    }
                    let v = Arc::new(v);
                    g.foreach_externals_cache
                        .insert(end_id, (g.graph_revision, g.param_revision, v.clone()));
                    v
                }
            } else {
                let mut v: Vec<GeoCacheRef> = Vec::with_capacity(compiled.external_keys().len());
                for &(n, port) in compiled.external_keys().iter() {
                    v.push(
                        g.compute_output_ref(n, &port, registry, perf_stats, overrides, visiting),
                    );
                }
                let v = Arc::new(v);
                g.foreach_externals_cache
                    .insert(end_id, (g.graph_revision, g.param_revision, v.clone()));
                v
            }
        } else {
            Arc::new(Vec::new())
        };
        let mut cctx = crate::nodes::runtime::execution_context::CompiledExecutionContext {
            externals_vec: externals_vec.clone(),
            slot_cache: Vec::new(),
            scratch_inputs: Vec::new(),
            scratch_geos: Vec::new(),
            scratch_gpu: Vec::new(),
            warnings: Vec::new(),
            current_meta: ForEachMeta::default(),
        };
        if let Some(c) = &compiled_opt {
            cctx.slot_cache.resize(c.slot_count, GeoCacheRef::empty());
        }

        let mut last_fb: Arc<Geometry> = if let Some(compiled) = &compiled_opt {
            match compiled.sink_fb.as_ref() {
                Some(crate::nodes::runtime::compiled_block::InputSource::External(idx)) => {
                    externals_vec
                        .get(*idx)
                        .map(|g| g.as_geo())
                        .unwrap_or_else(|| GeoCacheRef::empty().as_geo())
                }
                _ => GeoCacheRef::empty().as_geo(),
            }
        } else {
            fb_src
                .as_ref()
                .map(|(n, p)| {
                    g.compute_output_ref(*n, p, registry, perf_stats, overrides, visiting)
                        .as_geo()
                })
                .unwrap_or_else(|| GeoCacheRef::empty().as_geo())
        };

        let (sp_mode, sp_idx, sp_val) = (
            p_int(&end.parameters, "single_pass_mode", 0),
            p_int(&end.parameters, "single_pass_index", 0).max(0) as usize,
            p_str(&end.parameters, "single_pass_value", "")
                .trim()
                .to_string(),
        );
        let total_iters = pieces.len().max(1).min(max_it);
        let sp_sel = if single_pass && !pieces.is_empty() {
            if sp_mode == 1 && !sp_val.is_empty() {
                let iv = sp_val.parse::<i32>().ok();
                pieces
                    .iter()
                    .enumerate()
                    .find_map(|(i, p)| match p {
                        GeoCacheRef::View(v) => v.foreach_meta().and_then(|m| {
                            if m.value == sp_val || iv == Some(m.ivalue) {
                                Some(i)
                            } else {
                                None
                            }
                        }),
                        _ => None,
                    })
                    .unwrap_or_else(|| sp_idx.min(pieces.len().saturating_sub(1)))
            } else {
                sp_idx.min(pieces.len().saturating_sub(1))
            }
        } else {
            0
        };
        let sp_piece = if single_pass {
            pieces.get(sp_sel).cloned().unwrap_or_else(empty_ref)
        } else {
            empty_ref()
        };
        let iters = if single_pass { 1 } else { total_iters };

        let pre_masks = if begin_method == 2 {
            let pm = Arc::new(ElementGroupMask::new(base_geo.primitives().len()));
            let ptm = Arc::new(ElementGroupMask::new(base_geo.points().len()));
            Some((pm, ptm))
        } else {
            None
        };
        let can_parallel = compiled_opt.as_ref().is_some_and(|c| {
            gather == 0
                && begin_method != 1
                && c.sink_fb.is_none()
                && !stop_hash
                && !stop_empty
                && !single_pass
                && iters > 1
        });
        if can_parallel {
            let compiled = compiled_opt.as_ref().unwrap().clone();
            let (out_geo, mut par_warns) = (0..iters)
                .into_par_iter()
                .map_init(
                    || {
                        crate::nodes::runtime::execution_context::compiled_ctx_tls_init(
                            externals_vec.clone(),
                            compiled.slot_count,
                        );
                        ()
                    },
                    |_, i| {
                        crate::nodes::runtime::execution_context::with_compiled_ctx_tls(|lctx| {
                            lctx.warnings.clear();
                            let piece_in = pieces.get(i).cloned().unwrap_or_else(empty_ref);
                            let mut m = match &piece_in {
                                GeoCacheRef::View(v) => {
                                    v.foreach_meta().cloned().unwrap_or_default()
                                }
                                _ => ForEachMeta::default(),
                            };
                            m.iteration = i as i32;
                            m.numiterations = iters as i32;
                            lctx.current_meta = m;
                            let begin_out = match begin_method {
                                2 => {
                                    let (pm, ptm) = pre_masks.as_ref().unwrap();
                                    GeoCacheRef::View(Arc::new(GeometryView::from_masks(
                                        base_geo.clone(),
                                        Some(pm),
                                        Some(ptm),
                                        Some(lctx.current_meta.clone()),
                                    )))
                                }
                                3 => GeoCacheRef::Geo(base_geo.clone()),
                                _ => piece_in,
                            };
                            let out = compiled.run_iter(registry, lctx, begin_out).0.as_geo();
                            let warns = lctx.take_warnings();
                            (out, warns)
                        })
                    },
                )
                .reduce(
                    || (GeoCacheRef::empty().as_geo(), Vec::new()),
                    |(a, mut wa), (b, mut wb)| {
                        wa.append(&mut wb);
                        if GeometryRef::point_len(a.as_ref()) == 0
                            && GeometryRef::prim_len(a.as_ref()) == 0
                        {
                            return (b, wa);
                        }
                        if GeometryRef::point_len(b.as_ref()) == 0
                            && GeometryRef::prim_len(b.as_ref()) == 0
                        {
                            return (a, wa);
                        }
                        (
                            Arc::new(crate::libs::algorithms::merge::binary_merge(&a, &b)),
                            wa,
                        )
                    },
                );
            let mut all_warns = warns;
            all_warns.extend(par_warns.drain(..));
            let out = GeoCacheRef::Geo(out_geo.clone());
            g.foreach_block_cache_ref
                .insert(end_id, (bkey, out.clone(), out.clone()));
            flush_warnings(&all_warns);
            return (out.clone(), out);
        }

        let mut outs: Vec<Arc<Geometry>> = Vec::new();
        let mut last_out: Arc<Geometry> = GeoCacheRef::empty().as_geo();
        let mut last_hash: Option<u64> = None;

        if compiled_opt.is_none() {
            g.foreach_scope_begin(&reach);
        }
        for i in 0..iters {
            if compiled_opt.is_none() && i > 0 {
                g.foreach_scope_bump();
            }
            let piece_in = if single_pass {
                sp_piece.clone()
            } else {
                pieces.get(i).cloned().unwrap_or_else(empty_ref)
            };
            if stop_empty && piece_in.is_empty_geo() {
                break;
            }
            let fb_in: Arc<Geometry> = if i == 0
                && GeometryRef::point_len(last_fb.as_ref()) == 0
                && GeometryRef::prim_len(last_fb.as_ref()) == 0
            {
                piece_in.clone().as_geo()
            } else {
                last_fb.clone()
            };
            let mut m = match &piece_in {
                GeoCacheRef::View(v) => v.foreach_meta().cloned().unwrap_or_default(),
                _ => ForEachMeta::default(),
            };
            m.iteration = if single_pass { sp_sel as i32 } else { i as i32 };
            m.numiterations = if single_pass {
                total_iters as i32
            } else {
                iters as i32
            };

            if compiled_opt.is_some() {
                cctx.current_meta = m;
            } else {
                crate::nodes::runtime::foreach_tls::push(block_key.clone(), m.clone());
            }

            let begin_out = match begin_method {
                1 => GeoCacheRef::Geo(fb_in.clone()),
                2 => {
                    let (pm, ptm) = pre_masks.as_ref().unwrap();
                    GeoCacheRef::View(Arc::new(GeometryView::from_masks(
                        base_geo.clone(),
                        Some(pm),
                        Some(ptm),
                        if compiled_opt.is_some() {
                            Some(cctx.current_meta.clone())
                        } else {
                            crate::nodes::runtime::foreach_tls::last().map(|(_, m)| m)
                        },
                    )))
                }
                3 => GeoCacheRef::Geo(base_geo.clone()),
                _ => piece_in.clone(),
            };

            let (out, fb) = if let Some(compiled) = &compiled_opt {
                let (out_r, fb_r) = compiled.run_iter(registry, &mut cctx, begin_out);
                (out_r.as_geo(), fb_r.as_geo())
            } else {
                g.port_ref_cache
                    .insert((begin_id, port_key::out0()), begin_out);
                if g.foreach_scope_nodes.contains(&begin_id) {
                    g.foreach_port_epoch
                        .insert((begin_id, port_key::out0()), g.foreach_scope_epoch);
                }
                let out = body_src
                    .as_ref()
                    .map(|(n, p)| {
                        g.compute_output_ref(*n, p, registry, perf_stats, overrides, visiting)
                            .as_geo()
                    })
                    .unwrap_or_else(|| GeoCacheRef::empty().as_geo());
                let fb = fb_src
                    .as_ref()
                    .map(|(n, p)| {
                        g.compute_output_ref(*n, p, registry, perf_stats, overrides, visiting)
                            .as_geo()
                    })
                    .unwrap_or_else(|| out.clone());
                (out, fb)
            };

            last_out = out.clone();
            last_fb = fb.clone();
            if compiled_opt.is_none() {
                crate::nodes::runtime::foreach_tls::pop();
            }

            if gather == 0 {
                outs.push(out);
            }
            if stop_hash {
                let h = geo_fingerprint_hash(if gather == 1 { &last_fb } else { &last_out });
                if last_hash == Some(h) {
                    break;
                }
                last_hash = Some(h);
            }
        }

        let out_geo: Arc<Geometry> = if gather == 0 {
            Arc::new(crate::libs::algorithms::merge::merge_geometry_arcs(&outs))
        } else {
            last_out.clone()
        };
        let fb_geo: Arc<Geometry> = last_fb.clone();
        let out_ref = GeoCacheRef::Geo(out_geo.clone());
        let fb_ref = GeoCacheRef::Geo(fb_geo.clone());
        g.foreach_block_cache_ref
            .insert(end_id, (bkey, out_ref.clone(), fb_ref.clone()));
        flush_warnings(&warns);
        if compiled_opt.is_none() {
            g.foreach_scope_end();
        }
        (out_ref, fb_ref)
    }
}
