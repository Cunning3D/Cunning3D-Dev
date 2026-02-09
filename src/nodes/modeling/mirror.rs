use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::algorithms::merge::merge_geometry;
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::mesh::{GeoPrimitive, Geometry};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::math::Vec3;
use rayon::prelude::*;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct MirrorNode;

impl NodeParameters for MirrorNode {
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
                "Normal",
                "Plane",
                ParameterValue::Vec3(Vec3::X),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "keep_original",
                "Keep Original",
                "Settings",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for MirrorNode {
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
        let normal = params
            .iter()
            .find(|p| p.name == "direction")
            .and_then(|p| match p.value {
                ParameterValue::Vec3(v) => Some(v),
                _ => None,
            })
            .unwrap_or(Vec3::X)
            .normalize_or_zero();
        let keep_original = params
            .iter()
            .find(|p| p.name == "keep_original")
            .and_then(|p| match p.value {
                ParameterValue::Bool(v) => Some(v),
                _ => None,
            })
            .unwrap_or(true);

        if normal == Vec3::ZERO {
            return input.clone();
        }

        let mut mirrored = input.fork();

        // Transform P (Parallel)
        if let Some(p_attr) = mirrored.get_point_attribute_mut(attrs::P) {
            if let Some(p_data) = p_attr.as_mut_slice::<Vec3>() {
                p_data.par_iter_mut().for_each(|p| {
                    let dist = (*p - origin).dot(normal);
                    *p = *p - 2.0 * dist * normal;
                });
            }
        }

        // Transform Vertex N (Parallel)
        if let Some(n_attr) = mirrored.get_vertex_attribute_mut(attrs::N) {
            if let Some(n_data) = n_attr.as_mut_slice::<Vec3>() {
                n_data.par_iter_mut().for_each(|n| {
                    let d = n.dot(normal);
                    *n = *n - 2.0 * d * normal;
                });
            }
        }
        // Transform Point N (Parallel)
        if let Some(n_attr) = mirrored.get_point_attribute_mut(attrs::N) {
            if let Some(n_data) = n_attr.as_mut_slice::<Vec3>() {
                n_data.par_iter_mut().for_each(|n| {
                    let d = n.dot(normal);
                    *n = *n - 2.0 * d * normal;
                });
            }
        }

        // Reverse winding (Sequential for sparse set arena iteration for now,
        // as par_iter on primitives is possible but iterating mutable sparse set is tricky if concurrent modification happens.
        // But SparseSetArena::values_mut() is slice, so par_iter_mut() works!
        // Wait, SparseSetArena doesn't expose values_mut(). I added iter_mut().
        // Let's use it.)

        // Note: iter_mut() returns slice iterator.
        // Wait, iter_mut returns std::slice::IterMut. We need par_iter_mut from rayon.
        // SparseSetArena has `dense` field private.
        // But I saw `pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T>`.
        // To use rayon, I need to be able to turn it into par iter.
        // `dense` is `Vec<T>`.
        // I can't call par_iter_mut unless SparseSetArena exposes it or I can get the slice.
        // `values_mut` isn't in SparseSetArena definition I read.
        // But `iter_mut` returns standard iterator.

        // I will stick to sequential for primitives reverse winding for safety/simplicity unless I add `par_values_mut`.
        // Actually I can just add `par_iter_mut` to SparseSetArena if I could edit it, but I can't edit geometry lib.
        // So sequential is fine for topology changes here (usually fewer primitives than points).

        for prim in mirrored.primitives_mut().iter_mut() {
            match prim {
                GeoPrimitive::Polygon(poly) => poly.vertices.reverse(),
                GeoPrimitive::Polyline(line) => line.vertices.reverse(),
                _ => {}
            }
        }

        if keep_original {
            Arc::new(merge_geometry(vec![input.as_ref(), &mirrored]))
        } else {
            Arc::new(mirrored)
        }
    }
}

register_node!("Mirror", "Modeling", MirrorNode);
