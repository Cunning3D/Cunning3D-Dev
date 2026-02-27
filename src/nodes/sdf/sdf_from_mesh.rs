use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::ids::VertexId;
use crate::mesh::{Attribute, Geometry};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::nodes::{InputStyle, NodeStyle};
use crate::register_node;
use crate::sdf::{SdfHandle, SdfGrid, CHUNK_SIZE};
use bevy::prelude::*;
use dashmap::DashSet;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::collections::HashMap;

// Parry3d Imports
use parry3d::math::{Point, Vector};
use parry3d::na::{Isometry3, Point3, Vector3};
use parry3d::query::PointQuery;
use parry3d::shape::TriMesh;

// Import Chunk from volume
use crate::sdf::SdfChunk;
use std::sync::Arc;

#[derive(Default)]
pub struct SdfFromPolygonsNode;

impl NodeParameters for SdfFromPolygonsNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "voxel_size",
                "Voxel Size",
                "Settings",
                ParameterValue::Float(0.1),
                ParameterUIType::FloatSlider {
                    min: 0.001,
                    max: 1000.0,
                },
            ),
            Parameter::new(
                "bandwidth",
                "Bandwidth",
                "Settings",
                ParameterValue::Int(3),
                ParameterUIType::IntSlider { min: 1, max: 10 },
            ),
            Parameter::new(
                "display_points",
                "Show Points",
                "Visualization",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for SdfFromPolygonsNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let input = inputs
            .first()
            .map(|g| g.materialize())
            .unwrap_or_else(Geometry::new);
        let param_map: HashMap<String, ParameterValue> = params
            .iter()
            .map(|p| (p.name.clone(), p.value.clone()))
            .collect();
        Arc::new(compute_sdf_from_mesh(&input, &param_map))
    }
}

register_node!("SDF From Polygons", "Volume", SdfFromPolygonsNode);

pub fn node_style() -> NodeStyle {
    NodeStyle::Normal
}

pub fn input_style() -> InputStyle {
    InputStyle::Individual
}

pub fn compute_sdf_from_mesh(
    input_geo: &Geometry,
    params: &HashMap<String, ParameterValue>,
) -> Geometry {
    puffin::profile_function!();

    let voxel_size = match params.get("voxel_size") {
        Some(ParameterValue::Float(v)) => *v,
        _ => 0.1,
    };
    // Clamp voxel size
    let voxel_size = voxel_size.max(0.01);

    let bandwidth = match params.get("bandwidth") {
        Some(ParameterValue::Int(v)) => *v,
        _ => 3,
    };

    let display_points = match params.get("display_points") {
        Some(ParameterValue::Bool(v)) => *v,
        _ => false,
    };

    // 1. Extract Vertices
    let Some(positions) = input_geo
        .get_point_attribute("@P")
        .and_then(|a| a.as_slice::<Vec3>())
    else {
        return Geometry::new();
    };

    let vertices: Vec<Point3<f32>> = positions
        .iter()
        .map(|p| Point3::new(p.x, p.y, p.z))
        .collect();

    // 2. Extract Indices
    let mut indices: Vec<[u32; 3]> = Vec::new();

    // Helper to get dense point index from VertexId
    let get_dense = |vid: VertexId| -> Option<u32> {
        let v = input_geo.vertices().get(vid.into())?;
        input_geo
            .points()
            .get_dense_index(v.point_id.into())
            .map(|i| i as u32)
    };

    for prim in input_geo.primitives().values() {
        let verts = prim.vertices();
        if verts.len() < 3 {
            continue;
        }

        let Some(v0) = get_dense(verts[0]) else {
            continue;
        };
        for i in 1..verts.len() - 1 {
            let Some(v1) = get_dense(verts[i]) else {
                continue;
            };
            let Some(v2) = get_dense(verts[i + 1]) else {
                continue;
            };
            indices.push([v0, v1, v2]);
        }
    }

    if indices.is_empty() {
        return Geometry::new();
    }

    // Build Parry TriMesh (Accelerated Structure)
    let trimesh = TriMesh::new(vertices.clone(), indices.clone());

    // 3. Sparse Block Collection (Level 4 Optimization)
    // Instead of collecting voxels, we collect CHUNKS (Blocks).
    // This reduces memory and contention by 4096x (16^3).
    let candidate_chunks = DashSet::new();
    let inv_voxel_size = 1.0 / voxel_size;
    let band_width_units = (bandwidth as f32 + 1.0) * voxel_size;
    let chunk_size_f32 = CHUNK_SIZE as f32;

    // Parallel iteration over triangles to identify surface chunks
    #[cfg(not(target_arch = "wasm32"))]
    indices.par_chunks(100).for_each(|chunk| {
        process_chunk(
            chunk,
            &vertices,
            band_width_units,
            inv_voxel_size,
            chunk_size_f32,
            &candidate_chunks,
        );
    });

    #[cfg(target_arch = "wasm32")]
    indices.chunks(100).for_each(|chunk| {
        process_chunk(
            chunk,
            &vertices,
            band_width_units,
            inv_voxel_size,
            chunk_size_f32,
            &candidate_chunks,
        );
    });

    // 4. Parallel Chunk Filling (Global BVH Query)
    let active_chunks: Vec<IVec3> = candidate_chunks.iter().map(|k| *k.key()).collect();
    let identity = Isometry3::identity();
    let limit = bandwidth as f32 * voxel_size;

    // Process each chunk in parallel using Global Parry Query (Robust)
    #[cfg(not(target_arch = "wasm32"))]
    let chunks_map: FxHashMap<IVec3, SdfChunk> = active_chunks
        .par_iter()
        .map(|&chunk_idx| process_active_chunk(chunk_idx, limit, voxel_size, &trimesh, &identity))
        .collect();

    #[cfg(target_arch = "wasm32")]
    let chunks_map: FxHashMap<IVec3, SdfChunk> = active_chunks
        .iter()
        .map(|&chunk_idx| process_active_chunk(chunk_idx, limit, voxel_size, &trimesh, &identity))
        .collect();

    // 5. Construct Grid
    let mut grid = SdfGrid::new(voxel_size, limit);
    grid.chunks = chunks_map;

    let mut output = Geometry::new();
    output.sdfs.push(SdfHandle::new(grid));

    // Store visualization preference
    let viz_val = if display_points { 1.0 } else { 0.0 };
    output.insert_detail_attribute(
        "display_points",
        Attribute::new(vec![viz_val]),
    );

    output
}

fn process_chunk(
    chunk: &[[u32; 3]],
    vertices: &[Point3<f32>],
    band_width_units: f32,
    inv_voxel_size: f32,
    chunk_size_f32: f32,
    candidate_chunks: &DashSet<IVec3>,
) {
    for tri in chunk {
        let v0 = vertices[tri[0] as usize];
        let v1 = vertices[tri[1] as usize];
        let v2 = vertices[tri[2] as usize];

        // AABB of triangle expanded by bandwidth
        let min_x = v0.x.min(v1.x).min(v2.x) - band_width_units;
        let min_y = v0.y.min(v1.y).min(v2.y) - band_width_units;
        let min_z = v0.z.min(v1.z).min(v2.z) - band_width_units;

        let max_x = v0.x.max(v1.x).max(v2.x) + band_width_units;
        let max_y = v0.y.max(v1.y).max(v2.y) + band_width_units;
        let max_z = v0.z.max(v1.z).max(v2.z) + band_width_units;

        // Convert to Chunk Coordinates
        let min_chunk_x = (min_x * inv_voxel_size / chunk_size_f32).floor() as i32;
        let min_chunk_y = (min_y * inv_voxel_size / chunk_size_f32).floor() as i32;
        let min_chunk_z = (min_z * inv_voxel_size / chunk_size_f32).floor() as i32;

        let max_chunk_x = (max_x * inv_voxel_size / chunk_size_f32).floor() as i32;
        let max_chunk_y = (max_y * inv_voxel_size / chunk_size_f32).floor() as i32;
        let max_chunk_z = (max_z * inv_voxel_size / chunk_size_f32).floor() as i32;

        for z in min_chunk_z..=max_chunk_z {
            for y in min_chunk_y..=max_chunk_y {
                for x in min_chunk_x..=max_chunk_x {
                    candidate_chunks.insert(IVec3::new(x, y, z));
                }
            }
        }
    }
}

fn process_active_chunk(
    chunk_idx: IVec3,
    limit: f32,
    voxel_size: f32,
    trimesh: &TriMesh,
    identity: &Isometry3<f32>,
) -> (IVec3, SdfChunk) {
    let mut chunk = SdfChunk::new(limit);
    let chunk_origin = chunk_idx * CHUNK_SIZE;

    for z in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let global_idx = chunk_origin + IVec3::new(x, y, z);
                let center = global_idx.as_vec3() * voxel_size;
                let pt = Point3::new(center.x, center.y, center.z);

                // Global Closest Point Query (Robust)
                let projection = trimesh.project_point(identity, &pt, false);
                let dist = parry3d::na::distance(&pt, &projection.point);

                // Global Sign Check (Robust)
                let is_inside = trimesh.contains_point(identity, &pt);

                // Shell Logic: Dilate the surface by 1.25 * voxel_size
                // A thickness > 1.0 is required to prevent "aliasing holes" where the shell falls between voxels.
                let thickness = voxel_size * 1.0;
                let signed_dist = if is_inside {
                    -dist - thickness
                } else {
                    dist - thickness
                };

                // Clamp
                let clamped_dist = signed_dist.clamp(-limit, limit);
                chunk.set(x, y, z, clamped_dist);
            }
        }
    }
    (chunk_idx, chunk)
}
