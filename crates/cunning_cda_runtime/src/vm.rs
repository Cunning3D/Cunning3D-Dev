use std::collections::HashMap;
use std::sync::Arc;

use cunning_kernel::mesh::Geometry;
use cunning_kernel::traits::parameter::ParameterValue;
use smallvec::SmallVec;
use smallvec::smallvec;

use crate::asset::RuntimeDefinition;
use crate::compiler::{ExecutionPlan, InputSlot};
use crate::error::{CdaCookError, CdaCookErrorKind};
use crate::registry::{PortInputs, RuntimeRegistry};

fn apply_channel(dst: &mut ParameterValue, src: &ParameterValue, channel: Option<u32>) {
    let Some(ch) = channel else { *dst = src.clone(); return; };
    let ch = ch as usize;
    let v = match src {
        ParameterValue::Float(f) => *f as f64,
        ParameterValue::Int(i) => *i as f64,
        ParameterValue::Bool(b) => if *b { 1.0 } else { 0.0 },
        ParameterValue::Vec2(v) => [v.x as f64, v.y as f64].get(ch).copied().unwrap_or(0.0),
        ParameterValue::Vec3(v) | ParameterValue::Color(v) => [v.x as f64, v.y as f64, v.z as f64].get(ch).copied().unwrap_or(0.0),
        ParameterValue::Vec4(v) | ParameterValue::Color4(v) => [v.x as f64, v.y as f64, v.z as f64, v.w as f64].get(ch).copied().unwrap_or(0.0),
        _ => 0.0,
    };
    set_channel(dst, ch, v);
}

fn set_channel(v: &mut ParameterValue, ch: usize, val: f64) {
    let f = val as f32;
    match v {
        ParameterValue::Float(x) => *x = f,
        ParameterValue::Int(x) => *x = val as i32,
        ParameterValue::Bool(x) => *x = val != 0.0,
        ParameterValue::Vec2(p) => match ch { 0 => p.x = f, 1 => p.y = f, _ => {} },
        ParameterValue::Vec3(p) | ParameterValue::Color(p) => match ch { 0 => p.x = f, 1 => p.y = f, 2 => p.z = f, _ => {} },
        ParameterValue::Vec4(p) | ParameterValue::Color4(p) => match ch { 0 => p.x = f, 1 => p.y = f, 2 => p.z = f, 3 => p.w = f, _ => {} },
        _ => {}
    }
}

pub fn execute(plan: &ExecutionPlan, def: &RuntimeDefinition, reg: &RuntimeRegistry, inputs: &[Arc<Geometry>], overrides: &HashMap<String, ParameterValue>, cancel: &std::sync::atomic::AtomicBool) -> Result<Vec<Arc<Geometry>>, CdaCookError> {
    let empty = Arc::new(Geometry::new());
    let mut cache: Vec<Option<Arc<Geometry>>> = vec![None; plan.nodes.len()];

    // Build per-node param maps.
    //
    // IMPORTANT: params must come from the *current* RuntimeDefinition (def), not from the compiled plan.
    // The plan's params are only a baseline/default; def is allowed to change between cooks
    // (e.g. promoted params, interactive internal overrides like VoxelEdit cmds_json, etc.).
    let mut params_by_ix: Vec<HashMap<String, ParameterValue>> =
        plan.nodes.iter().map(|n| n.params.clone()).collect();
    for n in &def.nodes {
        if let Some(&ix) = plan.node_index.get(&n.id) {
            params_by_ix[ix] = n.params.clone();
        }
    }
    for pp in &def.promoted_params {
        let v = overrides.get(&pp.name).cloned().unwrap_or_else(|| pp.default_value.clone());
        for b in &pp.bindings {
            if let Some(&ix) = plan.node_index.get(&b.node) {
                let e = params_by_ix[ix].entry(b.param.clone()).or_insert_with(|| v.clone());
                apply_channel(e, &v, b.channel);
            }
        }
    }

    for (ix, n) in plan.nodes.iter().enumerate() {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(CdaCookError { asset_uuid: plan.asset_uuid, asset_name: plan.asset_name.clone(), node_id: Some(n.id), op: Some(n.op), port: None, param: None, kind: CdaCookErrorKind::Cancelled });
        }
        if cache[ix].is_some() { continue; }

        // Build structured inputs per port (no flattening; Many stays grouped by port).
        let mut ins: Vec<PortInputs> = Vec::with_capacity(n.inputs.len());
        for slot in &n.inputs {
            match slot {
                InputSlot::One { port, conn } => {
                    if let Some(ir) = conn {
                        let g = cache.get(ir.src_ix).and_then(|x| x.clone()).ok_or_else(|| CdaCookError {
                            asset_uuid: plan.asset_uuid,
                            asset_name: plan.asset_name.clone(),
                            node_id: Some(n.id),
                            op: Some(n.op),
                            port: reg.port_label(n.op, true, ir.to_port).or_else(|| Some(ir.to_port.to_string())),
                            param: None,
                            kind: CdaCookErrorKind::MissingDependency { src_node: ir.src_node },
                        })?;
                        ins.push(PortInputs { port: *port, values: smallvec![g] });
                    } else {
                        ins.push(PortInputs { port: *port, values: smallvec![empty.clone()] });
                    }
                }
                InputSlot::Many { port, conns } => {
                    let mut values: SmallVec<[Arc<Geometry>; 1]> = SmallVec::new();
                    values.reserve(conns.len());
                    for ir in conns {
                        let g = cache.get(ir.src_ix).and_then(|x| x.clone()).ok_or_else(|| CdaCookError {
                            asset_uuid: plan.asset_uuid,
                            asset_name: plan.asset_name.clone(),
                            node_id: Some(n.id),
                            op: Some(n.op),
                            port: reg.port_label(n.op, true, ir.to_port).or_else(|| Some(ir.to_port.to_string())),
                            param: None,
                            kind: CdaCookErrorKind::MissingDependency { src_node: ir.src_node },
                        })?;
                        values.push(g);
                    }
                    ins.push(PortInputs { port: *port, values });
                }
            }
        }

        let p = &params_by_ix[ix];
        let op = reg.op(n.op).ok_or_else(|| CdaCookError { asset_uuid: plan.asset_uuid, asset_name: plan.asset_name.clone(), node_id: Some(n.id), op: Some(n.op), port: None, param: None, kind: CdaCookErrorKind::Internal { message: "missing op".to_string() } })?;
        let out = op.compute(n.id, p, &ins, inputs)?;
        cache[ix] = Some(out);
    }

    let mut outs: Vec<Arc<Geometry>> = Vec::with_capacity(plan.outputs.len());
    for id in &plan.outputs {
        if let Some(&ix) = plan.node_index.get(id) { outs.push(cache.get(ix).and_then(|x| x.clone()).unwrap_or_else(|| empty.clone())); }
        else { outs.push(empty.clone()); }
    }
    Ok(outs)
}

