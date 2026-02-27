use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::{Attribute, GeoPrimitive, GeoVertex, Geometry, PolygonPrim, PrimitiveType};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::nodes::{InputStyle, NodeStyle};
use crate::register_node;
use crate::sdf::{SdfHandle, SdfGrid, CHUNK_SIZE};
use bevy::prelude::*;
use std::collections::HashMap;
// Use our new Surface Nets implementation
use crate::libs::algorithms::mc::extract_surface_nets;

use std::sync::Arc;

#[derive(Default)]
pub struct SdfToPolygonsNode;

impl NodeParameters for SdfToPolygonsNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "iso_value",
                "Iso Value",
                "Settings",
                ParameterValue::Float(0.0),
                ParameterUIType::FloatSlider {
                    min: -1.0,
                    max: 1.0,
                },
            ),
            Parameter::new(
                "invert",
                "Invert",
                "Settings",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "hard_surface",
                "Hard Surface",
                "Settings",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for SdfToPolygonsNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let input = inputs
            .first()
            .map(|g| g.materialize())
            .unwrap_or_else(Geometry::new);
        let param_map: HashMap<String, ParameterValue> = params
            .iter()
            .map(|p| (p.name.clone(), p.value.clone()))
            .collect();
        Arc::new(compute_sdf_to_mesh(&input, &param_map))
    }
}

register_node!("SDF To Polygons", "Volume", SdfToPolygonsNode);

pub fn node_style() -> NodeStyle {
    NodeStyle::Normal
}

pub fn input_style() -> InputStyle {
    InputStyle::Individual
}

pub fn compute_sdf_to_mesh(
    input_geo: &Geometry,
    params: &HashMap<String, ParameterValue>,
) -> Geometry {
    if input_geo.sdfs.is_empty() {
        return Geometry::new();
    }

    let iso_value = match params.get("iso_value") {
        Some(ParameterValue::Float(v)) => *v,
        _ => 0.0,
    };
    let invert = match params.get("invert") {
        Some(ParameterValue::Bool(v)) => *v,
        _ => false,
    };
    let hard_surface = match params.get("hard_surface") {
        Some(ParameterValue::Bool(v)) => *v,
        _ => false,
    };

    // If we have multiple volumes, we need to merge the results.
    // For now, we collect geometry from each volume and merge them.
    let geometries: Vec<Geometry> = input_geo
        .sdfs
        .iter()
        .map(|handle| sdf_to_geometry(handle, iso_value, invert, hard_surface))
        .collect();

    crate::libs::algorithms::merge::merge_geometry_slice(&geometries)
}

pub fn sdf_to_geometry(
    volume_handle: &SdfHandle,
    iso_value: f32,
    invert: bool,
    hard_surface: bool,
) -> Geometry {
    puffin::profile_function!();

    let grid = volume_handle.grid.read().unwrap();

    // 1. Calculate bounds of active voxels
    if grid.chunks.is_empty() {
        return Geometry::new();
    }

    // Bounds in INDEX space
    let mut min_idx = IVec3::splat(i32::MAX);
    let mut max_idx = IVec3::splat(i32::MIN);

    for chunk_pos in grid.chunks.keys() {
        let chunk_min = *chunk_pos * CHUNK_SIZE;
        let chunk_max = chunk_min + IVec3::splat(CHUNK_SIZE);
        min_idx = min_idx.min(chunk_min);
        max_idx = max_idx.max(chunk_max);
    }

    // Add padding for Surface Nets to close the surface (requires neighbors)
    min_idx -= IVec3::ONE;
    max_idx += IVec3::ONE;

    let size = (max_idx - min_idx).as_uvec3();
    let width = size.x as usize;
    let height = size.y as usize;
    let depth = size.z as usize;

    // Safety check for OOM
    if width * height * depth > 100_000_000 {
        println!("Warning: SDF to Mesh bounds too large: {:?}", size);
        return Geometry::new();
    }

    // 2. Build dense buffer
    puffin::profile_scope!("densify_grid");
    // TODO: Optimize this to be sparse/chunk-based in future
    let mut data = vec![grid.background_value; width * height * depth];

    let mut chunk_processed_count = 0;
    for (chunk_pos, chunk) in &grid.chunks {
        // Yield every 10 chunks to keep profiler responsive
        chunk_processed_count += 1;
        if chunk_processed_count % 10 == 0 {
            puffin::yield_now();
        }

        let chunk_origin = *chunk_pos * CHUNK_SIZE;

        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    let global_pos = chunk_origin + IVec3::new(x, y, z);
                    if global_pos.x >= min_idx.x
                        && global_pos.x < max_idx.x
                        && global_pos.y >= min_idx.y
                        && global_pos.y < max_idx.y
                        && global_pos.z >= min_idx.z
                        && global_pos.z < max_idx.z
                    {
                        let local_x = (global_pos.x - min_idx.x) as usize;
                        let local_y = (global_pos.y - min_idx.y) as usize;
                        let local_z = (global_pos.z - min_idx.z) as usize;

                        let idx = local_z * width * height + local_y * width + local_x;
                        let mut val = chunk.get(x, y, z);
                        if invert {
                            val = -val;
                        }
                        data[idx] = val;
                    }
                }
            }
        }
    }

    // 3. Run Surface Nets
    puffin::profile_scope!("surface_nets");
    // Returns (vertices: Vec<f32> flat XYZ, indices: Vec<usize> quad indices)
    let dims = [width, height, depth];
    let (mesh_vertices, mesh_indices) = extract_surface_nets(&data, dims, iso_value, hard_surface);

    // 4. Convert to Geometry
    puffin::profile_scope!("convert_to_geometry");
    let mut output = Geometry::new();

    let min_idx_vec3 = min_idx.as_vec3(); // Index offset

    let mut point_attributes_p = Vec::new();

    let num_points = mesh_vertices.len() / 3;

    // We need to keep track of VertexIds created for each point index (0..num_points)
    // because primitives refer to point indices from surface nets.
    // In our new system, Primitives refer to VertexIds.
    // And each Point has exactly one Vertex in this simple case.
    let mut point_to_vertex_id = Vec::with_capacity(num_points);

    for i in 0..num_points {
        if i % 50000 == 0 {
            puffin::yield_now();
        }

        let x = mesh_vertices[i * 3];
        let y = mesh_vertices[i * 3 + 1];
        let z = mesh_vertices[i * 3 + 2];

        // Coords relative to min_idx in index space
        let local_index_pos = Vec3::new(x, y, z) + min_idx_vec3;

        // Convert Index Space -> Local Metric Space
        let local_metric_pos = local_index_pos * grid.voxel_size;

        // Apply Volume Transform
        let world_pos = volume_handle.transform.transform_point3(local_metric_pos);

        point_attributes_p.push(world_pos);

        // New Geometry Creation: Add Point + Vertex
        let pid = output.add_point();
        let vid = output.add_vertex(pid);
        point_to_vertex_id.push(vid);
    }

    // Create primitives (Quads)
    // Each quad has 4 indices referring to the points array
    for i in 0..mesh_indices.len() / 4 {
        let idx0 = mesh_indices[i * 4];
        let idx1 = mesh_indices[i * 4 + 1];
        let idx2 = mesh_indices[i * 4 + 2];
        let idx3 = mesh_indices[i * 4 + 3];

        if idx0 >= point_to_vertex_id.len()
            || idx1 >= point_to_vertex_id.len()
            || idx2 >= point_to_vertex_id.len()
            || idx3 >= point_to_vertex_id.len()
        {
            continue;
        }

        let v0 = point_to_vertex_id[idx0];
        let v1 = point_to_vertex_id[idx1];
        let v2 = point_to_vertex_id[idx2];
        let v3 = point_to_vertex_id[idx3];

        output.add_primitive(GeoPrimitive::Polygon(PolygonPrim {
            vertices: vec![v0, v1, v2, v3],
        }));
    }

    output.insert_point_attribute(
        "@P",
        Attribute::new(crate::libs::algorithms::algorithms_dcc::PagedBuffer::from(
            point_attributes_p,
        )),
    );

    // Calculate Normals
    output.calculate_flat_normals();

    output
}

#[allow(dead_code)]
pub fn vdb_to_geometry(
    volume_handle: &SdfHandle,
    iso_value: f32,
    invert: bool,
    hard_surface: bool,
) -> Geometry {
    sdf_to_geometry(volume_handle, iso_value, invert, hard_surface)
}
