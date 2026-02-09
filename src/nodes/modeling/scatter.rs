use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::mesh::{Attribute, GeoPrimitive, Geometry};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::math::Vec3;
use rayon::prelude::*;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct ScatterNode;

impl NodeParameters for ScatterNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "count",
                "Count",
                "General",
                ParameterValue::Int(1000),
                ParameterUIType::IntSlider {
                    min: 0,
                    max: 100000,
                },
            ),
            Parameter::new(
                "seed",
                "Seed",
                "General",
                ParameterValue::Int(0),
                ParameterUIType::IntSlider { min: 0, max: 10000 },
            ),
        ]
    }
}

// Stateless random helper for parallel execution
fn pcg_hash(input: u64) -> u32 {
    let state = input.wrapping_mul(747796405).wrapping_add(2891336453);
    let word = ((state >> ((state >> 28).wrapping_add(4))) ^ state).wrapping_mul(277803737);
    ((word >> 22) ^ word) as u32
}

fn next_f32(state: &mut u64) -> f32 {
    *state = state.wrapping_add(1);
    let val = pcg_hash(*state);
    (val as f32) / (u32::MAX as f32)
}

#[derive(Clone)]
struct PrimSample {
    verts: Vec<crate::libs::geometry::ids::VertexId>,
    tri_cdf: Vec<f32>,
    total: f32,
}

impl NodeOp for ScatterNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let Some(input) = mats.get(0) else {
            return Arc::new(Geometry::new());
        };

        let count = params
            .iter()
            .find(|p| p.name == "count")
            .and_then(|p| match p.value {
                ParameterValue::Int(v) => Some(v),
                _ => None,
            })
            .unwrap_or(1000) as usize;
        let seed = params
            .iter()
            .find(|p| p.name == "seed")
            .and_then(|p| match p.value {
                ParameterValue::Int(v) => Some(v),
                _ => None,
            })
            .unwrap_or(0) as u64;

        let Some(positions) = input.get_point_position_attribute() else {
            return Arc::new(Geometry::new());
        };

        // 1) Precompute valid polygon samples (no fallback to Vec3::ZERO)
        let mut prim_cdf = Vec::new();
        let mut prims = Vec::new();
        let mut total_area = 0.0;

        for prim in input.primitives().values() {
            let GeoPrimitive::Polygon(poly) = prim else {
                continue;
            };
            if poly.vertices.len() < 3 {
                continue;
            }
            let p0 = input.get_pos_by_vertex(poly.vertices[0], positions);
            let mut tri_cdf = Vec::with_capacity(poly.vertices.len().saturating_sub(2));
            let mut sub_total = 0.0;
            for i in 1..poly.vertices.len() - 1 {
                let p1 = input.get_pos_by_vertex(poly.vertices[i], positions);
                let p2 = input.get_pos_by_vertex(poly.vertices[i + 1], positions);
                sub_total += (p1 - p0).cross(p2 - p0).length() * 0.5;
                tri_cdf.push(sub_total);
            }
            if sub_total <= 0.0 {
                continue;
            }
            total_area += sub_total;
            prim_cdf.push(total_area);
            prims.push(PrimSample {
                verts: poly.vertices.clone(),
                tri_cdf,
                total: sub_total,
            });
        }

        if total_area <= 0.0 {
            return Arc::new(Geometry::new());
        }

        // 2) Parallel point generation (always from valid polygon triangles)
        let new_points: Vec<Vec3> = (0..count)
            .into_par_iter()
            .map(|i| {
                let mut s = seed.wrapping_add(i as u64).wrapping_mul(0x9e3779b97f4a7c15);
                let r = next_f32(&mut s) * total_area;
                let pidx = prim_cdf
                    .binary_search_by(|v| v.partial_cmp(&r).unwrap_or(std::cmp::Ordering::Equal))
                    .unwrap_or_else(|e| e)
                    .min(prim_cdf.len() - 1);
                let prim = &prims[pidx];
                let r2 = next_f32(&mut s) * prim.total;
                let tidx = prim
                    .tri_cdf
                    .binary_search_by(|v| v.partial_cmp(&r2).unwrap_or(std::cmp::Ordering::Equal))
                    .unwrap_or_else(|e| e)
                    .min(prim.tri_cdf.len() - 1);
                let v0 = prim.verts[0];
                let v1 = prim.verts[tidx + 1];
                let v2 = prim.verts[tidx + 2];
                let p0 = input.get_pos_by_vertex(v0, positions);
                let p1 = input.get_pos_by_vertex(v1, positions);
                let p2 = input.get_pos_by_vertex(v2, positions);
                let u = next_f32(&mut s);
                let v = next_f32(&mut s);
                let (u, v) = if u + v > 1.0 {
                    (1.0 - u, 1.0 - v)
                } else {
                    (u, v)
                };
                p0 + (p1 - p0) * u + (p2 - p0) * v
            })
            .collect();

        // 3. Assemble Output
        let mut out_geo = Geometry::new();
        // Batch add points (topology IDs)
        for _ in 0..count {
            out_geo.add_point();
        }
        // Bulk insert attribute
        out_geo.insert_point_attribute(attrs::P, Attribute::new(new_points));

        Arc::new(out_geo)
    }
}

register_node!("Scatter", "Modeling", ScatterNode);
