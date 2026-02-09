use crate::libs::geometry::geo_ref::GeometryRef;
use crate::{
    cunning_core::traits::node_interface::{NodeOp, NodeParameters},
    libs::geometry::ids::{PointId, VertexId},
    mesh::{Attribute, Geometry},
    nodes::parameter::{Parameter, ParameterUIType, ParameterValue},
    register_node,
};
use bevy::prelude::Vec2;
use std::sync::Arc;
use xatlas_rs as xatlas;

#[derive(Default)]
pub struct AutoUvNode;

impl NodeParameters for AutoUvNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![Parameter::new(
            "padding",
            "Padding",
            "Settings",
            ParameterValue::Float(0.02),
            ParameterUIType::FloatSlider { min: 0.0, max: 0.2 },
        )]
    }
}

impl NodeOp for AutoUvNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let default_input = Arc::new(Geometry::new());
        let input = mats.first().unwrap_or(&default_input);

        let padding = params
            .iter()
            .find(|p| p.name == "padding")
            .and_then(|p| match p.value {
                ParameterValue::Float(f) => Some(f),
                _ => None,
            })
            .unwrap_or(0.02);

        // 1. Prepare Input for xatlas
        let positions = match input.get_point_position_attribute() {
            Some(p) => p,
            _ => return input.clone(), // No positions
        };

        let mut indices: Vec<u32> = Vec::new();

        // Triangulate
        for prim in input.primitives().values() {
            let verts = prim.vertices();
            if verts.len() < 3 {
                continue;
            }

            // Fan triangulation
            let v0_idx = verts[0];
            for i in 1..verts.len() - 1 {
                let v1_idx = verts[i];
                let v2_idx = verts[i + 1];

                let get_p = |vid: VertexId| -> Option<u32> {
                    let v = input.vertices().get(vid.into())?;
                    input
                        .points()
                        .get_dense_index(v.point_id.into())
                        .map(|i| i as u32)
                };

                if let (Some(p0), Some(p1), Some(p2)) =
                    (get_p(v0_idx), get_p(v1_idx), get_p(v2_idx))
                {
                    indices.push(p0);
                    indices.push(p1);
                    indices.push(p2);
                }
            }
        }

        // Convert positions to flat slice [x, y, z, x, y, z...]
        let mut pos_data: Vec<f32> = Vec::with_capacity(positions.len() * 3);
        for p in positions.iter() {
            pos_data.push(p.x);
            pos_data.push(p.y);
            pos_data.push(p.z);
        }

        // 2. Run xatlas
        let mut atlas = xatlas::Xatlas::new();

        let mesh_decl = xatlas::MeshDecl {
            vertex_count: positions.len() as u32,
            vertex_position_data: unsafe {
                std::slice::from_raw_parts(pos_data.as_ptr() as *const u8, pos_data.len() * 4)
            },
            vertex_position_stride: 12,
            index_count: indices.len() as u32,
            index_data: unsafe {
                std::slice::from_raw_parts(indices.as_ptr() as *const u8, indices.len() * 4)
            },
            index_format: xatlas::IndexFormat::Uint32,
            ..Default::default()
        };

        atlas.add_mesh(&mesh_decl);

        // Parameterize & Pack
        let mut chart_options = xatlas::ChartOptions::default();
        let mut pack_options = xatlas::PackOptions::default();

        let atlas_size = 1024;
        pack_options.padding = (padding * atlas_size as f32) as u32;
        pack_options.resolution = atlas_size;

        atlas.generate(chart_options, pack_options, |_, _| {});

        // 3. Read back results
        let meshes = atlas.meshes();
        if meshes.is_empty() {
            return input.clone();
        }

        let output_mesh = &meshes[0];
        let out_indices = &output_mesh.indices;
        let out_vertices = &output_mesh.vertices;

        // Re-calculate input mapping (needed for remapping)
        // Store VertexId
        let mut input_tri_vertex_indices: Vec<VertexId> = Vec::new();
        for prim in input.primitives().values() {
            let verts = prim.vertices();
            if verts.len() < 3 {
                continue;
            }
            let v0_idx = verts[0];
            for i in 1..verts.len() - 1 {
                let v1_idx = verts[i];
                let v2_idx = verts[i + 1];
                input_tri_vertex_indices.push(v0_idx);
                input_tri_vertex_indices.push(v1_idx);
                input_tri_vertex_indices.push(v2_idx);
            }
        }

        let mut uvs = vec![Vec2::ZERO; input.vertices().len()];
        let mut uv_set = vec![false; input.vertices().len()];

        // Iterate xatlas triangles
        for (tri_idx, chunk) in out_indices.chunks(3).enumerate() {
            if tri_idx * 3 >= input_tri_vertex_indices.len() {
                break;
            }

            // Get original vertex indices for this triangle
            let orig_v0 = input_tri_vertex_indices[tri_idx * 3];
            let orig_v1 = input_tri_vertex_indices[tri_idx * 3 + 1];
            let orig_v2 = input_tri_vertex_indices[tri_idx * 3 + 2];

            // Get UVs from xatlas vertices
            let x_v0 = &out_vertices[chunk[0] as usize];
            let x_v1 = &out_vertices[chunk[1] as usize];
            let x_v2 = &out_vertices[chunk[2] as usize];

            let uv0 = Vec2::new(
                x_v0.uv[0] as f32 / atlas_size as f32,
                x_v0.uv[1] as f32 / atlas_size as f32,
            );
            let uv1 = Vec2::new(
                x_v1.uv[0] as f32 / atlas_size as f32,
                x_v1.uv[1] as f32 / atlas_size as f32,
            );
            let uv2 = Vec2::new(
                x_v2.uv[0] as f32 / atlas_size as f32,
                x_v2.uv[1] as f32 / atlas_size as f32,
            );

            let set_uv = |vid: VertexId, val: Vec2, uvs: &mut [Vec2], uv_set: &mut [bool]| {
                if let Some(dense) = input.vertices().get_dense_index(vid.into()) {
                    uvs[dense] = val;
                    uv_set[dense] = true;
                }
            };

            set_uv(orig_v0, uv0, &mut uvs, &mut uv_set);
            set_uv(orig_v1, uv1, &mut uvs, &mut uv_set);
            set_uv(orig_v2, uv2, &mut uvs, &mut uv_set);
        }

        let mut geo = input.fork();
        geo.insert_vertex_attribute("@uv", Attribute::new(uvs));

        Arc::new(geo)
    }
}

register_node!("Auto UV", "UV", AutoUvNode);
