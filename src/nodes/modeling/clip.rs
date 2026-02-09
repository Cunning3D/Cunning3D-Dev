use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::math::Vec3;
use rayon::prelude::*;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct ClipNode;

impl NodeParameters for ClipNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "origin",
                "Origin",
                "Plane",
                ParameterValue::Vec3(Vec3::ZERO),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "direction",
                "Direction",
                "Plane",
                ParameterValue::Vec3(Vec3::Y),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "keep_side",
                "Keep Side",
                "Settings",
                ParameterValue::String("Keep Above".to_string()),
                ParameterUIType::String,
            ),
        ]
    }
}

impl NodeOp for ClipNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let input = match mats.get(0) {
            Some(g) => g,
            None => return Arc::new(Geometry::new()),
        };

        let origin = params
            .iter()
            .find(|p| p.name == "origin")
            .and_then(|p| match p.value {
                ParameterValue::Vec3(v) => Some(v),
                _ => None,
            })
            .unwrap_or(Vec3::ZERO);
        let direction = params
            .iter()
            .find(|p| p.name == "direction")
            .and_then(|p| match p.value {
                ParameterValue::Vec3(v) => Some(v),
                _ => None,
            })
            .unwrap_or(Vec3::Y)
            .normalize_or_zero();
        let keep_side = params
            .iter()
            .find(|p| p.name == "keep_side")
            .and_then(|p| match &p.value {
                ParameterValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or("Keep Above".to_string());

        if direction == Vec3::ZERO {
            return input.clone();
        }

        let keep_above = keep_side != "Keep Below";

        let mut out_geo = input.fork();
        let positions = match out_geo.get_point_position_attribute() {
            Some(p) => p,
            None => return Arc::new(out_geo),
        };

        use crate::libs::geometry::ids::PrimId;

        // Parallel scan to identify primitives to remove
        // We get dense indices first
        let to_remove_dense: Vec<usize> = out_geo
            .primitives()
            .values()
            .par_iter()
            .enumerate()
            .filter_map(|(idx, prim)| {
                let mut all_wrong_side = true;

                for &vid in prim.vertices() {
                    let pos = input.get_pos_by_vertex(vid, positions);
                    let dist = (pos - origin).dot(direction);

                    if keep_above {
                        if dist >= 0.0 {
                            all_wrong_side = false;
                        }
                    } else {
                        if dist <= 0.0 {
                            all_wrong_side = false;
                        }
                    }
                }

                if all_wrong_side {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect();

        // Convert dense indices to stable IDs sequentially
        // Note: We must do this before any removal to keep dense indices valid
        let mut to_remove_ids = Vec::with_capacity(to_remove_dense.len());
        let prim_arena = out_geo.primitives();
        for dense_idx in to_remove_dense {
            if let Some(id) = prim_arena.get_id_from_dense(dense_idx) {
                to_remove_ids.push(PrimId::from(id));
            }
        }

        // Remove
        for pid in to_remove_ids {
            out_geo.remove_primitive(pid);
        }

        out_geo.clean();

        Arc::new(out_geo)
    }
}

register_node!("Clip", "Modeling", ClipNode);
