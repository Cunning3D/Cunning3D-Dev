use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::algorithms::merge::merge_geometry_slice;
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::math::Vec3;
use rayon::prelude::*;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct CopyToPointsNode;

impl NodeParameters for CopyToPointsNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![Parameter::new(
            "scale_mult",
            "Scale Multiplier",
            "General",
            ParameterValue::Float(1.0),
            ParameterUIType::FloatSlider {
                min: 0.0,
                max: 10.0,
            },
        )]
    }
}

impl NodeOp for CopyToPointsNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let source_geo = match mats.get(0) {
            Some(g) => g,
            None => return Arc::new(Geometry::new()),
        };

        let target_points_geo = match mats.get(1) {
            Some(g) => g,
            None => return Arc::new(Geometry::new()),
        };

        if target_points_geo.get_point_count() == 0 {
            return Arc::new(Geometry::new());
        }

        let scale_mult = params
            .iter()
            .find(|p| p.name == "scale_mult")
            .and_then(|p| match p.value {
                ParameterValue::Float(v) => Some(v),
                _ => None,
            })
            .unwrap_or(1.0);

        let positions = target_points_geo
            .get_point_position_attribute()
            .unwrap_or(&[]);
        let scales_opt = target_points_geo
            .get_point_attribute("pscale")
            .and_then(|a| a.as_slice::<f32>());

        // Parallel instantiation
        // We iterate target points in parallel, create (fork) new geometries, transform them
        let result_geos: Vec<Geometry> = positions
            .par_iter()
            .enumerate()
            .map(|(i, &pos)| {
                let scale = if let Some(scales) = scales_opt {
                    scales.get(i).copied().unwrap_or(1.0)
                } else {
                    1.0
                } * scale_mult;

                let mut instance = source_geo.fork();

                if let Some(p_attr) = instance.get_point_attribute_mut(attrs::P) {
                    if let Some(p_data) = p_attr.as_mut_slice::<Vec3>() {
                        for p in p_data.iter_mut() {
                            *p = (*p * scale) + pos;
                        }
                    }
                }
                instance
            })
            .collect();

        Arc::new(merge_geometry_slice(&result_geos))
    }
}

register_node!("CopyToPoints", "Modeling", CopyToPointsNode; inputs: &["Geometry", "Points"], outputs: &["Output"], style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts);
