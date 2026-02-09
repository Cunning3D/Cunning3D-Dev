use rayon::prelude::*;
use bevy::math::{DVec3, DVec2, Vec3};
use crate::libs::geometry::mesh::{Geometry, GeoPrimitive, PolygonPrim, Attribute};
use crate::libs::geometry::ids::{AttributeId, VertexId};
use crate::libs::algorithms::boolean::attributes::BooleanConfig;
use crate::libs::algorithms::boolean::spatial::SpatialIndex;
use crate::libs::algorithms::boolean::kernel::GeoKernel;
use crate::libs::algorithms::boolean::graph::CutGraph;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanOperation {
    Union,
    Intersection,
    Difference,
}

#[inline]
fn build_vertex_to_point_dense(geo: &Geometry) -> Vec<usize> {
    let ps = geo.points();
    geo.vertices()
        .values()
        .iter()
        .map(|v| ps.get_dense_index(v.point_id.into()).unwrap_or(usize::MAX))
        .collect()
}

/// The main entry point for the high-performance boolean engine.
/// 
/// Pipeline:
/// 1. Build BVH for acceleration.
/// 2. Find intersections in parallel.
/// 3. Build planar cut-graphs for each primitive.
/// 4. Classify regions (Inside/Outside).
/// 5. Reconstruct final geometry.
pub fn mesh_boolean(
    geo_a: &Geometry,
    geo_b: &Geometry,
    op: BooleanOperation,
    _config: &BooleanConfig
) -> Result<Geometry, String> {
    puffin::profile_function!();
    // 1. Spatial Indexing
    let (bvh_a, bvh_b) = {
        puffin::profile_scope!("boolean_build_bvh");
        (
            SpatialIndex::build(geo_a).ok_or("Failed to build BVH for Geometry A")?,
            SpatialIndex::build(geo_b).ok_or("Failed to build BVH for Geometry B")?,
        )
    };

    // Helper to process one side (Target cut by Cutter)
    // Returns list of Geometry chunks, where each chunk has a "temp_inside" group indicating classification
    let process_half = |target_geo: &Geometry, _target_bvh: &SpatialIndex, cutter_geo: &Geometry, cutter_bvh: &SpatialIndex| -> Vec<Geometry> {
        let (Some(positions_t), Some(positions_c)) = (target_geo.get_point_position_attribute(), cutter_geo.get_point_position_attribute()) else { return Vec::new(); };
        let t_v2p = build_vertex_to_point_dense(target_geo);
        let c_v2p = build_vertex_to_point_dense(cutter_geo);
        let t_vertices = target_geo.vertices();
        let c_vertices = cutter_geo.vertices();
        
        // 2.1 Intersection & CutGraph
        // Note: par_iter on SparseSetArena values returns reference to GeoPrimitive
        // We use enumerate() to get the dense index, which CutGraph expects.
        let cut_graphs: Vec<Option<CutGraph>> = target_geo.primitives().values().par_iter().enumerate()
            .map(|(idx_t, prim_t)| {
                // Only support polygons for now
                let vertices_t = prim_t.vertices();
                if vertices_t.len() < 3 { return None; }
                
                let get_pos = |vid: VertexId| -> Option<Vec3> {
                    let vdi = t_vertices.get_dense_index(vid.into())?;
                    let pdi = *t_v2p.get(vdi)?;
                    if pdi == usize::MAX { return None; }
                    positions_t.get(pdi).copied()
                };

                let p0 = get_pos(vertices_t[0])?;
                let p1 = get_pos(vertices_t[1])?;
                let p2 = get_pos(vertices_t[2])?;
                
                let normal_t = (DVec3::from(p1) - DVec3::from(p0)).cross(DVec3::from(p2) - DVec3::from(p0)).normalize();
                
                let mut graph = CutGraph::new(idx_t, target_geo, normal_t)?;

                // Broad phase
                if let Some(bounds) = Bounds3D::from_prim(target_geo, idx_t) {
                    let hits = cutter_bvh.query_aabb(bounds.min, bounds.max);
                    
                    // Narrow phase: Edge-Plane intersection
                    for &idx_c in &hits {
                        // Access via slice for dense index
                        let prim_c = &cutter_geo.primitives().values().get(idx_c)?; 
                        let vertices_c = prim_c.vertices();
                        let num_verts_c = vertices_c.len();
                        
                        let plane_n = graph.plane_normal;
                        let plane_o = graph.plane_origin;
                        
                        let mut hit_points = Vec::new();
                        
                        // Helper to get position for cutter vertex
                        let get_pos_c = |vid: VertexId| -> Option<Vec3> {
                            let vdi = c_vertices.get_dense_index(vid.into())?;
                            let pdi = *c_v2p.get(vdi)?;
                            if pdi == usize::MAX { return None; }
                            positions_c.get(pdi).copied()
                        };

                        for i in 0..num_verts_c {
                            let j = (i + 1) % num_verts_c;
                            let v0 = vertices_c[i];
                            let v1 = vertices_c[j];
                            
                            let p0 = DVec3::from(get_pos_c(v0)?);
                            let p1 = DVec3::from(get_pos_c(v1)?);
                             
                            let d0 = (p0 - plane_o).dot(plane_n);
                            let d1 = (p1 - plane_o).dot(plane_n);
                             
                            if d0.signum() != d1.signum() {
                                let t = d0 / (d0 - d1);
                                let pt = p0.lerp(p1, t);
                                let diff = pt - plane_o;
                                let uv = DVec2::new(diff.dot(graph.plane_u_axis), diff.dot(graph.plane_v_axis));
                                hit_points.push(uv);
                            }
                        }
                        
                        // Form segments
                        if hit_points.len() >= 2 {
                            let c0 = get_pos_c(vertices_c[0])?;
                            let c1 = get_pos_c(vertices_c[1])?;
                            let c2 = get_pos_c(vertices_c[2])?;
                            let normal_c = (DVec3::from(c1) - DVec3::from(c0)).cross(DVec3::from(c2) - DVec3::from(c0)).normalize();
                             
                            let dir = plane_n.cross(normal_c);
                            if dir.length_squared() > 1e-12 {
                                let dir_2d = DVec2::new(dir.dot(graph.plane_u_axis), dir.dot(graph.plane_v_axis));
                                hit_points.sort_by(|a, b| a.dot(dir_2d).partial_cmp(&b.dot(dir_2d)).unwrap_or(std::cmp::Ordering::Equal));
                                 
                                for k in (0..hit_points.len()).step_by(2) {
                                    if k+1 < hit_points.len() {
                                        graph.insert_segment(hit_points[k], hit_points[k+1]);
                                    }
                                }
                            }
                        }
                    }
                }
                Some(graph)
            })
            .collect();

        // 2.2 Reconstruction
        cut_graphs.into_par_iter().filter_map(|g_opt| {
            let mut graph = g_opt?;
            let regions = graph.extract_regions();
            if regions.is_empty() { return None; }
            
            let mut geo_acc = Geometry::new();
            let mut pos_acc = Vec::new();
            let mut inside_group_mask = Vec::new();
            
            // CORRECT LOOP:
            for region in regions {
                 // Classification
                let mut center_uv = DVec2::ZERO;
                for &v in &region { center_uv += graph.vertices[v].uv; }
                center_uv /= region.len() as f64;
                let center_3d = graph.plane_origin + graph.plane_u_axis * center_uv.x + graph.plane_v_axis * center_uv.y;
                let query = center_3d + graph.plane_normal * 1e-5;
                let is_inside = is_point_inside_geometry(query, DVec3::Y, cutter_geo, cutter_bvh);
                inside_group_mask.push(is_inside);
                
                // Build vertices for this polygon
                let mut poly_verts = Vec::new();
                for &v_id in &region {
                    let pos = graph.plane_origin + graph.plane_u_axis * graph.vertices[v_id].uv.x + graph.plane_v_axis * graph.vertices[v_id].uv.y;
                    
                    // Add Point
                    let pid = geo_acc.add_point();
                    // Store position (we'll batch insert later? No, `add_point` adds default. We can set it now or collect.)
                    // Efficient way: collect to Vec<Vec3> and insert Attribute at end.
                    // Map pid -> index in pos_vec
                    pos_acc.push(pos); 
                    
                    // Add Vertex
                    let vid = geo_acc.add_vertex(pid);
                    poly_verts.push(vid);
                }
                
                // Add Primitive
                geo_acc.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: poly_verts }));
            }
            
            // Write positions
             let vec3_pos: Vec<Vec3> = pos_acc.iter().map(|p| Vec3::new(p.x as f32, p.y as f32, p.z as f32)).collect();
            geo_acc.insert_point_attribute(AttributeId::from("@P"), Attribute::new(vec3_pos));
            
            // Write Group
            let len = geo_acc.primitives().len();
            let mut mask = geo_acc.primitive_groups.entry(AttributeId::from("temp_inside")).or_insert_with(|| {
                let mut m = crate::libs::geometry::group::ElementGroupMask::new(len);
                m
            });
            // Ensure mask has correct size (push defaults)
             while mask.len() < len { mask.push(false); }
             
             // Set bits
             for (i, &is_in) in inside_group_mask.iter().enumerate() {
                 if is_in { mask.set(i, true); }
             }
            
            Some(geo_acc)
        }).collect()
    };

    // 3. Run Processing
    let (chunks_a, chunks_b) = {
        puffin::profile_scope!("boolean_process_halves");
        (
            process_half(geo_a, &bvh_a, geo_b, &bvh_b),
            process_half(geo_b, &bvh_b, geo_a, &bvh_a),
        )
    };

    // 4. Merge and Filter
    puffin::profile_scope!("boolean_merge_filter");
    let mut final_geo = Geometry::new();
    let mut all_positions = Vec::new();
    let mut all_point_ids = Vec::new(); // Dense map: Index -> PointId
    
    // Create groups
    final_geo.primitive_groups.insert(AttributeId::from("a_inside_b"), Default::default());
    final_geo.primitive_groups.insert(AttributeId::from("a_outside_b"), Default::default());
    final_geo.primitive_groups.insert(AttributeId::from("b_inside_a"), Default::default());
    final_geo.primitive_groups.insert(AttributeId::from("b_outside_a"), Default::default());
    
    let mut merge_chunks = |chunks: Vec<Geometry>, is_a: bool| {
        for chunk in chunks {
            let inside_mask = chunk.get_primitive_group("temp_inside").cloned().unwrap_or_default();
            
            // 1. Copy positions and create points
            let chunk_positions = chunk.get_point_position_attribute().unwrap_or(&[]);
            let start_p_idx = all_point_ids.len();
            for &pos in chunk_positions {
                let pid = final_geo.add_point();
                all_point_ids.push(pid);
                all_positions.push(pos);
            }
            let chunk_p_map = &all_point_ids[start_p_idx..]; 

            for (prim_idx, prim) in chunk.primitives().values().iter().enumerate() {
                let is_inside = inside_mask.get(prim_idx);
                
                let (keep, reverse) = match op {
                    BooleanOperation::Union => (!is_inside, false),
                    BooleanOperation::Intersection => (is_inside, false),
                    BooleanOperation::Difference => {
                        if is_a { (!is_inside, false) } // Keep A Outside
                        else { (is_inside, true) }      // Keep B Inside (Reverse)
                    }
                };
                
                if keep {
                    // Create new vertices for the primitive
                    let mut new_verts = Vec::new();
                    
                    let vertices = prim.vertices();
                    let ids: Vec<VertexId> = if reverse { vertices.iter().rev().copied().collect() } else { vertices.to_vec() };
                    
                    for &old_vid in &ids {
                        if let Some(old_v) = chunk.vertices().get(old_vid.into()) {
                            if let Some(old_p_dense) = chunk.points().get_dense_index(old_v.point_id.into()) {
                                if old_p_dense < chunk_p_map.len() {
                                    let new_pid = chunk_p_map[old_p_dense];
                                    let new_vid = final_geo.add_vertex(new_pid);
                                    new_verts.push(new_vid);
                                }
                            }
                        }
                    }
                    
                    let new_prim_id = final_geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: new_verts }));
                    
                    // Groups (dense index of new primitive)
                    let new_prim_dense = final_geo.primitives().get_dense_index(new_prim_id.into()).unwrap_or(0);
                    
                    let group_name = if is_a {
                        if is_inside { "a_inside_b" } else { "a_outside_b" }
                    } else {
                        if is_inside { "b_inside_a" } else { "b_outside_a" }
                    };
                    
                    // Helper to set group safely
                    if let Some(mask) = final_geo.primitive_groups.get_mut(&AttributeId::from(group_name)) {
                        while mask.len() <= new_prim_dense { mask.push(false); }
                        mask.set(new_prim_dense, true);
                    }
                }
            }
        }
    };
    
    merge_chunks(chunks_a, true);
    merge_chunks(chunks_b, false);
    
    // Add P attribute
    let p_id = AttributeId::from("@P");
    final_geo.insert_point_attribute(p_id, Attribute::new(all_positions));

    Ok(final_geo)
}
    
// Classification Helper
fn is_point_inside_geometry(pt: DVec3, dir: DVec3, geo: &Geometry, _bvh: &SpatialIndex) -> bool {
    let mut intersections = 0;
    let positions = if let Some(p) = geo.get_point_position_attribute() { p } else { return false; };
    
    // Iterate ALL primitives (Slow!)
    // TODO: Use BVH Ray Cast
    for prim in geo.primitives().values() {
        let vertices = prim.vertices();
        if vertices.len() < 3 { continue; }
        
        let get_pos = |vid: VertexId| -> Option<Vec3> {
            let p_idx = geo.vertices().get(vid.into())?.point_id;
            let dense_idx = geo.points().get_dense_index(p_idx.into())?;
            positions.get(dense_idx).copied()
        };

        if let Some(p0_vec) = get_pos(vertices[0]) {
            let p0 = DVec3::from(p0_vec);
            for i in 1..vertices.len()-1 {
                if let (Some(p1_vec), Some(p2_vec)) = (get_pos(vertices[i]), get_pos(vertices[i+1])) {
                    let p1 = DVec3::from(p1_vec);
                    let p2 = DVec3::from(p2_vec);
                    
                    if let Some(_) = GeoKernel::intersect_ray_triangle(pt, dir, p0, p1, p2) {
                        intersections += 1;
                    }
                }
            }
        }
    }
    
    intersections % 2 == 1
}

struct Bounds3D { min: Vec3, max: Vec3 }

impl Bounds3D {
    fn from_prim(geo: &Geometry, prim_idx: usize) -> Option<Self> {
        let positions = geo.get_point_position_attribute()?;
        let prim = geo.primitives().values().get(prim_idx)?;
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        
        let vertices = prim.vertices();
        if vertices.is_empty() { return None; }

        for &v_idx in vertices {
            if let Some(v) = geo.vertices().get(v_idx.into()) {
                if let Some(dense_idx) = geo.points().get_dense_index(v.point_id.into()) {
                    if let Some(pos) = positions.get(dense_idx) {
                        min = min.min(*pos);
                        max = max.max(*pos);
                    }
                }
            }
        }
        Some(Self { min, max })
    }
}
