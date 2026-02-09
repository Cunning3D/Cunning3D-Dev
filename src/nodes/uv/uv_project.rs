use crate::libs::geometry::geo_ref::GeometryRef;
use crate::{
    cunning_core::traits::node_interface::{NodeOp, NodeParameters},
    mesh::{Attribute, Geometry},
    nodes::parameter::{Parameter, ParameterUIType, ParameterValue},
    register_node,
};
use bevy::prelude::{Mat4, Quat, Vec2, Vec3};
use std::sync::Arc;

#[derive(Default)]
pub struct UvProjectNode;

impl NodeParameters for UvProjectNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "type",
                "Projection Type",
                "Settings",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Planar (XY)".to_string(), 0),
                        ("Planar (YZ)".to_string(), 1),
                        ("Planar (ZX)".to_string(), 2),
                        ("Cylindrical".to_string(), 3),
                        ("Spherical".to_string(), 4),
                    ],
                },
            ),
            Parameter::new(
                "center",
                "Center",
                "Transform",
                ParameterValue::Vec3(Vec3::ZERO),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "rotate",
                "Rotate",
                "Transform",
                ParameterValue::Vec3(Vec3::ZERO),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "scale",
                "Scale",
                "Transform",
                ParameterValue::Vec3(Vec3::ONE),
                ParameterUIType::Vec3Drag,
            ),
        ]
    }
}

impl NodeOp for UvProjectNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let default_input = Arc::new(Geometry::new());
        let input = mats.first().unwrap_or(&default_input);

        // Use fork to create a copy with a new dirty_id, sharing unmodified attributes
        let mut geo = input.fork();

        // 1. Get Parameters
        let proj_type = params
            .iter()
            .find(|p| p.name == "type")
            .and_then(|p| match &p.value {
                ParameterValue::Int(i) => Some(*i),
                _ => None,
            })
            .unwrap_or(0);
        let center = params
            .iter()
            .find(|p| p.name == "center")
            .and_then(|p| match &p.value {
                ParameterValue::Vec3(v) => Some(*v),
                _ => None,
            })
            .unwrap_or(Vec3::ZERO);
        let rotate = params
            .iter()
            .find(|p| p.name == "rotate")
            .and_then(|p| match &p.value {
                ParameterValue::Vec3(v) => Some(*v),
                _ => None,
            })
            .unwrap_or(Vec3::ZERO);
        let scale = params
            .iter()
            .find(|p| p.name == "scale")
            .and_then(|p| match &p.value {
                ParameterValue::Vec3(v) => Some(*v),
                _ => None,
            })
            .unwrap_or(Vec3::ONE);

        // 2. Build Transform Matrix (World -> Projection Space)
        let rotation_quat = Quat::from_euler(
            bevy::prelude::EulerRot::YXZ,
            rotate.y.to_radians(),
            rotate.x.to_radians(),
            rotate.z.to_radians(),
        );
        // Avoid division by zero in scale
        let safe_scale = Vec3::new(
            if scale.x.abs() < 1e-6 { 1.0 } else { scale.x },
            if scale.y.abs() < 1e-6 { 1.0 } else { scale.y },
            if scale.z.abs() < 1e-6 { 1.0 } else { scale.z },
        );
        let transform = Mat4::from_scale_rotation_translation(safe_scale, rotation_quat, center);
        let inv_transform = transform.inverse();

        // 3. Get Positions
        // We use the helper to get @P directly.
        let positions = match geo.get_point_position_attribute() {
            Some(p) => p,
            _ => return input.clone(), // No positions, nothing to project
        };

        // 4. Compute UVs for each Vertex (Face Corner)
        // Since UVs can have seams, they must be Vertex Attributes, not Point Attributes.
        // We iterate over all primitives and their vertices.
        // Geometry stores vertices in a flat list: geo.vertices.
        // However, geo.vertices maps vertex_index -> point_index.
        // We need to generate a UV for each entry in geo.vertices.

        let num_vertices = geo.vertices().len();
        let mut uvs = vec![Vec2::ZERO; num_vertices];

        // Optimization: Process vertices in parallel?
        // PagedBuffer supports parallel iteration, but here we are generating new data.
        // For simple project, sequential is fast enough.

        for (v_idx, v) in geo.vertices().values().iter().enumerate() {
            if let Some(p_idx) = geo.points().get_dense_index(v.point_id.into()) {
                if let Some(pos) = positions.get(p_idx) {
                    // Project point to local space
                    let p_local = inv_transform.transform_point3(*pos);

                    let uv = match proj_type {
                        0 => Vec2::new(p_local.x + 0.5, 1.0 - (p_local.y + 0.5)), // Planar XY
                        1 => Vec2::new(p_local.y + 0.5, 1.0 - (p_local.z + 0.5)), // Planar YZ
                        2 => Vec2::new(p_local.z + 0.5, 1.0 - (p_local.x + 0.5)), // Planar ZX
                        3 => {
                            // Cylindrical
                            // Wrap around Y axis (theta = atan2(x, z))
                            let theta = p_local.x.atan2(p_local.z); // -PI to PI
                            let u = (theta / (std::f32::consts::PI * 2.0)) + 0.5;
                            let v = p_local.y + 0.5;
                            Vec2::new(u, v)
                        }
                        4 => {
                            // Spherical
                            let len = p_local.length();
                            if len < 1e-6 {
                                Vec2::ZERO
                            } else {
                                let theta = p_local.x.atan2(p_local.z);
                                let phi = (p_local.y / len).clamp(-1.0, 1.0).acos();

                                let u = (theta / (std::f32::consts::PI * 2.0)) + 0.5;
                                let v = 1.0 - (phi / std::f32::consts::PI);
                                Vec2::new(u, v)
                            }
                        }
                        _ => Vec2::ZERO,
                    };

                    uvs[v_idx] = uv;
                }
            }
        }

        // 5. Store as Vertex Attribute
        geo.insert_vertex_attribute("@uv", Attribute::new(uvs));

        Arc::new(geo)
    }
}

register_node!("UV Project", "UV", UvProjectNode);
