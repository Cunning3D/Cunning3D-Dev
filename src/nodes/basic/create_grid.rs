use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::mesh::{Attribute, GeoPrimitive, Geometry, PolygonPrim};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::prelude::{Vec2, Vec3};
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct CreateGridNode;

impl NodeParameters for CreateGridNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "size",
                "Size",
                "Grid",
                ParameterValue::Vec2(Vec2::new(10.0, 10.0)),
                ParameterUIType::Vec2Drag,
            ),
            Parameter::new(
                "center",
                "Center",
                "Grid",
                ParameterValue::Vec3(Vec3::ZERO),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "rows",
                "Rows",
                "Grid",
                ParameterValue::Int(10),
                ParameterUIType::IntSlider { min: 2, max: 100 },
            ),
            Parameter::new(
                "columns",
                "Columns",
                "Grid",
                ParameterValue::Int(10),
                ParameterUIType::IntSlider { min: 2, max: 100 },
            ),
        ]
    }
}

impl NodeOp for CreateGridNode {
    fn compute(&self, params: &[Parameter], _inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let size = params
            .iter()
            .find(|p| p.name == "size")
            .and_then(|p| match p.value {
                ParameterValue::Vec2(v) => Some(v),
                _ => None,
            })
            .unwrap_or(Vec2::new(10.0, 10.0));
        let center = params
            .iter()
            .find(|p| p.name == "center")
            .and_then(|p| match p.value {
                ParameterValue::Vec3(v) => Some(v),
                _ => None,
            })
            .unwrap_or(Vec3::ZERO);
        let rows = params
            .iter()
            .find(|p| p.name == "rows")
            .and_then(|p| match p.value {
                ParameterValue::Int(v) => Some(v.max(2)),
                _ => None,
            })
            .unwrap_or(10) as usize;
        let cols = params
            .iter()
            .find(|p| p.name == "columns")
            .and_then(|p| match p.value {
                ParameterValue::Int(v) => Some(v.max(2)),
                _ => None,
            })
            .unwrap_or(10) as usize;

        let mut geo = Geometry::new();

        let start_x = -size.x / 2.0;
        let start_z = -size.y / 2.0;
        let step_x = size.x / ((cols - 1) as f32);
        let step_z = size.y / ((rows - 1) as f32);

        let mut p_indices = Vec::with_capacity(rows * cols);
        let mut points = Vec::with_capacity(rows * cols);

        for r in 0..rows {
            for c in 0..cols {
                let x = start_x + (c as f32) * step_x;
                let z = start_z + (r as f32) * step_z;

                let pos = Vec3::new(x, 0.0, z) + center;
                let pid = geo.add_point();
                p_indices.push(pid);
                points.push(pos);
            }
        }

        geo.insert_point_attribute(attrs::P, Attribute::new(points));

        for r in 0..rows - 1 {
            for c in 0..cols - 1 {
                let idx0 = r * cols + c;
                let idx1 = r * cols + (c + 1);
                let idx2 = (r + 1) * cols + c;
                let idx3 = (r + 1) * cols + (c + 1);

                let pid0 = p_indices[idx0];
                let pid1 = p_indices[idx1];
                let pid2 = p_indices[idx2];
                let pid3 = p_indices[idx3];

                let v0 = geo.add_vertex(pid2);
                let v1 = geo.add_vertex(pid3);
                let v2 = geo.add_vertex(pid1);
                let v3 = geo.add_vertex(pid0);

                geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim {
                    vertices: vec![v0, v1, v2, v3],
                }));
            }
        }

        geo.calculate_flat_normals();

        Arc::new(geo)
    }
}

register_node!("Create Grid", "Basic", CreateGridNode);
