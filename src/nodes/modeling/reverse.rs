use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::mesh::{GeoPrimitive, Geometry};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::math::Vec3;
use rayon::prelude::*;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct ReverseNode;

impl NodeParameters for ReverseNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "reverse_vertices",
                "Reverse Vertices",
                "General",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "reverse_normals",
                "Reverse Normals",
                "General",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for ReverseNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let input = match mats.get(0) {
            Some(g) => g,
            None => return Arc::new(Geometry::new()),
        };

        let rev_verts = params
            .iter()
            .find(|p| p.name == "reverse_vertices")
            .and_then(|p| match p.value {
                ParameterValue::Bool(v) => Some(v),
                _ => None,
            })
            .unwrap_or(true);
        let rev_norms = params
            .iter()
            .find(|p| p.name == "reverse_normals")
            .and_then(|p| match p.value {
                ParameterValue::Bool(v) => Some(v),
                _ => None,
            })
            .unwrap_or(true);

        let mut out_geo = input.fork();

        if rev_verts {
            for prim in out_geo.primitives_mut().iter_mut() {
                match prim {
                    GeoPrimitive::Polygon(poly) => {
                        poly.vertices.reverse();
                    }
                    GeoPrimitive::Polyline(line) => {
                        line.vertices.reverse();
                    }
                    _ => {}
                }
            }
        }

        if rev_norms {
            if let Some(n_attr) = out_geo.get_vertex_attribute_mut(attrs::N) {
                if let Some(normals) = n_attr.as_mut_slice::<Vec3>() {
                    normals.par_iter_mut().for_each(|n| *n = -*n);
                }
            }

            if let Some(n_attr) = out_geo.get_point_attribute_mut(attrs::N) {
                if let Some(normals) = n_attr.as_mut_slice::<Vec3>() {
                    normals.par_iter_mut().for_each(|n| *n = -*n);
                }
            }
        }

        Arc::new(out_geo)
    }
}

register_node!("Reverse", "Modeling", ReverseNode);
