use crate::libs::geometry::mesh::{
    Geometry, GeoVertex, GeoPrimitive, PrimitiveType, Attribute, PolygonPrim
};
use crate::libs::geometry::topology::Topology;
use crate::libs::geometry::group::ElementGroupMask;
use crate::libs::geometry::ids::{HalfEdgeId, PrimId, PointId, VertexId, AttributeId};
use crate::libs::algorithms::boolean::{BooleanOperation, BooleanConfig};
use bevy::prelude::{Vec2, Vec3};
use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use manifold_rs::{Manifold, Mesh};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputTopology {
    Polygons,
    Triangles,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NormalStrategy {
    TransferFromInput,
    Recompute,
}

pub struct ManifoldBooleanSettings {
    pub output_topology: OutputTopology,
    pub preserve_hard_edges: bool,
    pub normal_strategy: NormalStrategy,
    pub welding_tolerance: f32,
}

/// Main entry point for Manifold-based boolean operations.
#[cfg(target_arch = "wasm32")]
pub fn run_manifold_boolean(
    target: &Geometry,
    cutter: &Geometry,
    op: BooleanOperation,
    settings: &ManifoldBooleanSettings,
) -> Result<Geometry, String> {
    let _ = (target, cutter, op, settings);
    Err("Manifold boolean is disabled on wasm builds".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_manifold_boolean(
    target: &Geometry,
    cutter: &Geometry,
    op: BooleanOperation,
    settings: &ManifoldBooleanSettings,
) -> Result<Geometry, String> {
    println!("DEBUG: Starting Manifold Boolean...");

    // 1. Convert Target -> Manifold (pure geometry only)
    let m_target = geometry_to_manifold(target)?;

    // 2. Convert Cutter -> Manifold (pure geometry only)
    let m_cutter = geometry_to_manifold(cutter)?;

    println!(
        "DEBUG: Target Manifold is_empty = {}",
        m_target.is_empty()
    );
    println!(
        "DEBUG: Cutter Manifold is_empty = {}",
        m_cutter.is_empty()
    );

    // 3. Perform Boolean
    println!("DEBUG: Executing Boolean Operation {:?}...", op);
    let result_manifold = match op {
        BooleanOperation::Union => m_target.union(&m_cutter),
        BooleanOperation::Intersection => m_target.intersection(&m_cutter),
        BooleanOperation::Difference => m_target.difference(&m_cutter),
        _ => return Err(format!("Unsupported boolean operation: {:?}", op)),
    };

    if result_manifold.is_empty() {
        println!("DEBUG: Boolean Result is Empty! Treating as error to trigger legacy fallback.");
        return Err("Manifold boolean produced empty result".to_string());
    }

    // 4. Convert Manifold -> Geometry (triangulated)
    let (result_geo, _) = manifold_to_geometry(&result_manifold, settings.welding_tolerance);

    println!(
        "DEBUG: [Raw Result] Tris={}",
        result_geo.primitives().len()
    );

    // 5. Classify triangles by Source ID using Plane Matching
    let triangle_source_ids = classify_result_triangles_by_planes(
        &result_geo,
        target,
        cutter,
        op
    );

    let (mut final_geo, final_prim_ids) = if matches!(settings.output_topology, OutputTopology::Polygons) {
        // 6. Reconstruct N-gons by grouping triangles with same Source ID
        reconstruct_polygons_from_ids(&result_geo, &triangle_source_ids, settings.preserve_hard_edges)
    } else {
        (result_geo, triangle_source_ids)
    };

    // 7. Finalize: Transfer Attributes (Interpolated Normals)
    match settings.normal_strategy {
        NormalStrategy::TransferFromInput => {
             transfer_attributes(&mut final_geo, target, cutter, &final_prim_ids, op);
        }
        NormalStrategy::Recompute => {
            // Placeholder for future recompute logic
        }
    }

    Ok(final_geo)
}

// --- Conversion Logic ---

#[cfg(not(target_arch = "wasm32"))]
fn geometry_to_manifold(geo: &Geometry) -> Result<Manifold, String> {
    let positions = geo
        .get_point_position_attribute()
        .ok_or("Geometry missing @P attribute")?;

    let point_count = positions.len();
    if point_count == 0 {
        return Ok(Manifold::empty());
    }

    let mut vert_data: Vec<f32> = Vec::with_capacity(point_count * 3);
    for p in positions {
        vert_data.push(p.x);
        vert_data.push(p.y);
        vert_data.push(p.z);
    }

    let mut indices: Vec<u32> = Vec::new();

    for prim in geo.primitives().values() {
        let vertices = prim.vertices();
        let vcount = vertices.len();
        if vcount < 3 { continue; }
        
        let get_pidx = |vid: VertexId| -> Option<u32> {
             geo.vertices().get(vid.into()).and_then(|v| {
                 geo.points().get_dense_index(v.point_id.into()).map(|i| i as u32)
             })
        };

        let p0_index = get_pidx(vertices[0]).ok_or("Invalid vertex index in primitive (v0)")?;

        for i in 1..vcount - 1 {
            let v1 = vertices[i];
            let v2 = vertices[i + 1];

            let p1_index = get_pidx(v1).ok_or("Invalid vertex index in primitive (v1)")?;
            let p2_index = get_pidx(v2).ok_or("Invalid vertex index in primitive (v2)")?;

            indices.push(p0_index);
            indices.push(p1_index);
            indices.push(p2_index);
        }
    }

    if indices.is_empty() {
        return Ok(Manifold::empty());
    }

    let mesh = Mesh::new(&vert_data, &indices, None);
    Ok(Manifold::from_mesh(mesh))
}

#[cfg(not(target_arch = "wasm32"))]
fn manifold_to_geometry(manifold: &Manifold, tolerance: f32) -> (Geometry, Option<Vec<i32>>) {
    let mesh = manifold.to_mesh();

    let vert_flat = mesh.vertices();
    let idx_flat = mesh.indices();
    let num_props = mesh.num_props() as usize;

    let mut geo = Geometry::new();

    if num_props < 3 {
        return (geo, None);
    }

    let num_verts = vert_flat.len() / num_props;
    if num_verts == 0 || idx_flat.is_empty() {
        return (geo, None);
    }

    // 1. Weld positions
    let mut point_positions: Vec<Vec3> = Vec::new();
    let mut vertex_index_map: Vec<usize> = Vec::with_capacity(num_verts);
    let mut point_lookup: HashMap<(i32, i32, i32), usize> = HashMap::new();

    let scale = if tolerance > 0.0 { 1.0 / tolerance } else { 1e4 };

    for i in 0..num_verts {
        let base = i * num_props;
        let p = Vec3::new(vert_flat[base], vert_flat[base + 1], vert_flat[base + 2]);

        let key = (
            (p.x * scale).round() as i32,
            (p.y * scale).round() as i32,
            (p.z * scale).round() as i32,
        );

        let point_index = if let Some(&idx) = point_lookup.get(&key) {
            idx
        } else {
            let idx = point_positions.len();
            point_positions.push(p);
            point_lookup.insert(key, idx);
            idx
        };

        vertex_index_map.push(point_index);
    }
    
    geo.insert_point_attribute("@P", Attribute::new(point_positions.clone()));
    
    let mut point_ids = Vec::with_capacity(point_positions.len());
    for _ in 0..point_positions.len() {
        point_ids.push(geo.add_point());
    }

    // 2. Primitives
    for chunk in idx_flat.chunks(3) {
        if chunk.len() < 3 { break; }
        
        let i0 = vertex_index_map[chunk[0] as usize];
        let i1 = vertex_index_map[chunk[1] as usize];
        let i2 = vertex_index_map[chunk[2] as usize];

        let v0 = geo.add_vertex(point_ids[i0]);
        let v1 = geo.add_vertex(point_ids[i1]);
        let v2 = geo.add_vertex(point_ids[i2]);

        geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim {
            vertices: vec![v0, v1, v2],
        }));
    }

    (geo, None) 
}

// --- Source ID reconstruction ---

struct PlaneIndex {
    buckets: HashMap<(i32, i32, i32), Vec<(f32, i32)>>,
}

impl PlaneIndex {
    fn new(geo: &Geometry, id_sign: i32) -> Self {
        let mut buckets = HashMap::new();
        let positions = match geo.get_point_position_attribute() {
            Some(p) => p,
            None => return Self { buckets },
        };

        const N_QUANT: f32 = 50.0;

        for (prim_idx, prim) in geo.primitives().values().iter().enumerate() {
            let vertices = prim.vertices();
            if vertices.len() < 3 { continue; }

            let get_pos = |vid: VertexId| -> Vec3 {
                if let Some(v) = geo.vertices().get(vid.into()) {
                    if let Some(dense) = geo.points().get_dense_index(v.point_id.into()) {
                        return positions.get(dense).copied().unwrap_or(Vec3::ZERO);
                    }
                }
                Vec3::ZERO
            };

            let p0 = get_pos(vertices[0]);

            for i in 1..vertices.len() - 1 {
                let p1 = get_pos(vertices[i]);
                let p2 = get_pos(vertices[i + 1]);

                let mut n = (p1 - p0).cross(p2 - p0).normalize_or_zero();
                if n.length_squared() < 0.5 { continue; } 

                let d = n.dot(p0);
                let id = (prim_idx as i32 + 1) * id_sign;

                let key = (
                    (n.x * N_QUANT).round() as i32,
                    (n.y * N_QUANT).round() as i32,
                    (n.z * N_QUANT).round() as i32,
                );

                buckets.entry(key).or_insert_with(Vec::new).push((d, id));
            }
        }

        Self { buckets }
    }

    fn query(&self, n: Vec3, d: f32, invert_normal_match: bool) -> i32 {
        const N_QUANT: f32 = 50.0;
        let search_n = if invert_normal_match { -n } else { n };
        let search_d = if invert_normal_match { -d } else { d }; 

        let key_base = (
            (search_n.x * N_QUANT).round() as i32,
            (search_n.y * N_QUANT).round() as i32,
            (search_n.z * N_QUANT).round() as i32,
        );

        let mut best_id = 0;
        let mut min_dist_err = f32::MAX;
        const DIST_TOLERANCE: f32 = 1e-3; 

        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let key = (key_base.0 + dx, key_base.1 + dy, key_base.2 + dz);
                    if let Some(candidates) = self.buckets.get(&key) {
                        for &(cand_d, cand_id) in candidates {
                            let dist_err = (cand_d - search_d).abs();
                            if dist_err < DIST_TOLERANCE && dist_err < min_dist_err {
                                min_dist_err = dist_err;
                                best_id = cand_id;
                            }
                        }
                    }
                }
            }
        }
        
        if min_dist_err > DIST_TOLERANCE { 0 } else { best_id }
    }
}

fn classify_result_triangles_by_planes(
    geo: &Geometry,
    target: &Geometry,
    cutter: &Geometry,
    op: BooleanOperation,
) -> Vec<i32> {
    let positions = match geo.get_point_position_attribute() {
        Some(p) => p,
        None => return vec![0; geo.primitives().len()],
    };

    if positions.is_empty() {
        return vec![0; geo.primitives().len()];
    }

    let target_idx = PlaneIndex::new(target, 1);
    let cutter_idx = PlaneIndex::new(cutter, -1);

    let mut tri_ids = Vec::with_capacity(geo.primitives().len());

    for prim in geo.primitives().values() {
        let vertices = prim.vertices();
        if vertices.len() != 3 {
            tri_ids.push(0);
            continue;
        }

        let get_pos = |vid: VertexId| -> Vec3 {
            if let Some(v) = geo.vertices().get(vid.into()) {
                if let Some(dense) = geo.points().get_dense_index(v.point_id.into()) {
                    return positions.get(dense).copied().unwrap_or(Vec3::ZERO);
                }
            }
            Vec3::ZERO
        };

        let p0 = get_pos(vertices[0]);
        let p1 = get_pos(vertices[1]);
        let p2 = get_pos(vertices[2]);

        let n = (p1 - p0).cross(p2 - p0).normalize_or_zero();
        let d = n.dot(p0); 

        if n.length_squared() < 0.5 {
            tri_ids.push(0);
            continue;
        }

        let mut id = target_idx.query(n, d, false);
        if id == 0 {
            let invert = matches!(op, BooleanOperation::Difference);
            id = cutter_idx.query(n, d, invert);
        }
        tri_ids.push(id);
    }
    tri_ids
}

fn reconstruct_polygons_from_ids(geo: &Geometry, tri_ids: &[i32], preserve_hard_edges: bool) -> (Geometry, Vec<i32>) {
    let mut new_geo = Geometry::new();

    // 1. Copy Points and Map old Point dense index -> new PointId
    let old_positions = geo.get_point_position_attribute().unwrap_or(&[]);
    new_geo.insert_point_attribute("@P", Attribute::new(old_positions.to_vec()));
    
    let mut old_p_dense_to_new_pid = Vec::with_capacity(old_positions.len());
    for _ in 0..old_positions.len() {
        old_p_dense_to_new_pid.push(new_geo.add_point());
    }

    let mut new_prim_ids = Vec::new();
    let mut point_to_vertex_map: HashMap<PointId, VertexId> = HashMap::new();

    // 2. Group triangles by ID
    let mut groups: HashMap<i32, Vec<usize>> = HashMap::new();
    for (tri_idx, &id) in tri_ids.iter().enumerate() {
        groups.entry(id).or_default().push(tri_idx);
    }

    // 3. Process each group
    for (&prim_id, tri_indices) in &groups {
        if prim_id == 0 {
            // ID = 0: New intersection faces, keep as triangles
            for &tri_idx in tri_indices {
                let prim = &geo.primitives().values()[tri_idx];
                let mut new_verts = Vec::new();
                for &old_vid in prim.vertices() {
                    let old_v = geo.vertices().get(old_vid.into()).unwrap();
                    let old_p_dense = geo.points().get_dense_index(old_v.point_id.into()).unwrap();
                    let new_pid = old_p_dense_to_new_pid[old_p_dense];
                    
                    if preserve_hard_edges {
                         new_verts.push(new_geo.add_vertex(new_pid));
                    } else {
                        if let Some(&v_id) = point_to_vertex_map.get(&new_pid) {
                            new_verts.push(v_id);
                        } else {
                            let v_id = new_geo.add_vertex(new_pid);
                            point_to_vertex_map.insert(new_pid, v_id);
                            new_verts.push(v_id);
                        }
                    }
                }
                new_geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: new_verts }));
                new_prim_ids.push(0);
            }
            continue;
        }

        // For valid Source ID: Extract boundary edges and trace loop
        let mut edge_counts: HashMap<(usize, usize), i32> = HashMap::new();

        for &tri_idx in tri_indices {
            let prim = &geo.primitives().values()[tri_idx];
            let verts = prim.vertices();
            if verts.len() != 3 { continue; }
            
            let get_pdense = |vid: VertexId| -> usize {
                let v = geo.vertices().get(vid.into()).unwrap();
                geo.points().get_dense_index(v.point_id.into()).unwrap()
            };
            
            let v0 = get_pdense(verts[0]);
            let v1 = get_pdense(verts[1]);
            let v2 = get_pdense(verts[2]);

            *edge_counts.entry((v0, v1)).or_default() += 1;
            *edge_counts.entry((v1, v2)).or_default() += 1;
            *edge_counts.entry((v2, v0)).or_default() += 1;
        }

        let mut boundary_next: HashMap<usize, usize> = HashMap::new();
        for &(u, v) in edge_counts.keys() {
            let twin_count = edge_counts.get(&(v, u)).cloned().unwrap_or(0);
            let self_count = edge_counts.get(&(u, v)).cloned().unwrap_or(0);
            if self_count > twin_count {
                boundary_next.insert(u, v);
            }
        }

        while !boundary_next.is_empty() {
            let start = *boundary_next.keys().next().unwrap();
            let mut loop_points_dense = Vec::new(); 
            let mut curr = start;
            let max_loop = boundary_next.len() + 1000;
            let mut steps = 0;

            loop {
                loop_points_dense.push(curr);
                steps += 1;
                if let Some(next) = boundary_next.remove(&curr) {
                    curr = next;
                    if curr == start { break; }
                    if steps > max_loop { break; }
                } else { break; }
            }

            if loop_points_dense.len() >= 3 {
                let mut new_verts = Vec::new();
                for &p_dense in &loop_points_dense {
                    let new_pid = old_p_dense_to_new_pid[p_dense];
                    if preserve_hard_edges {
                        new_verts.push(new_geo.add_vertex(new_pid));
                    } else {
                        if let Some(&v_id) = point_to_vertex_map.get(&new_pid) {
                            new_verts.push(v_id);
                        } else {
                            let v_id = new_geo.add_vertex(new_pid);
                            point_to_vertex_map.insert(new_pid, v_id);
                            new_verts.push(v_id);
                        }
                    }
                }
                new_geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: new_verts }));
                new_prim_ids.push(prim_id);
            }
        }
    }
    (new_geo, new_prim_ids)
}

// --- Attribute Transfer ---

fn transfer_attributes(
    final_geo: &mut Geometry,
    target: &Geometry,
    cutter: &Geometry,
    source_prim_ids: &[i32],
    op: BooleanOperation,
) {
    let mut all_attrs = std::collections::HashSet::new();
    for k in target.point_attributes.keys() { all_attrs.insert(*k); }
    for k in target.vertex_attributes.keys() { all_attrs.insert(*k); }
    for k in target.primitive_attributes.keys() { all_attrs.insert(*k); }
    for k in cutter.point_attributes.keys() { all_attrs.insert(*k); }
    for k in cutter.vertex_attributes.keys() { all_attrs.insert(*k); }
    for k in cutter.primitive_attributes.keys() { all_attrs.insert(*k); }

    let final_pos = final_geo.get_point_position_attribute().unwrap().to_vec();

    for attr_id in all_attrs {
        if attr_id.as_str() == "@P" { continue; }

        let proto_attr = target.get_vertex_attribute(attr_id)
            .or_else(|| target.get_point_attribute(attr_id))
            .or_else(|| target.get_primitive_attribute(attr_id))
            .or_else(|| cutter.get_vertex_attribute(attr_id));
        
        let proto_attr = match proto_attr {
            Some(a) => a,
            None => continue,
        };

        let is_normal = attr_id.as_str() == "@N";
        let normalize = is_normal;
        let invert_on_diff = is_normal;

        interpolate_and_insert_attribute(
            final_geo, 
            &final_pos,
            target, 
            cutter, 
            source_prim_ids, 
            op, 
            attr_id, 
            proto_attr, 
            invert_on_diff, 
            normalize
        );
    }
}

fn interpolate_and_insert_attribute(
    final_geo: &mut Geometry,
    final_pos: &[Vec3],
    target: &Geometry,
    cutter: &Geometry,
    source_prim_ids: &[i32],
    op: BooleanOperation,
    attr_id: AttributeId,
    proto_type: &Attribute,
    invert_on_diff: bool,
    normalize: bool,
) {
    // Vec3 Implementation
    if let Some(_) = proto_type.as_slice::<Vec3>() {
        process_typed_attribute(
            final_geo, final_pos, target, cutter, source_prim_ids, op, attr_id, invert_on_diff,
            |v: Vec3| if normalize { v.normalize_or_zero() } else { v },
            |a, b, c, u, v, w| a * w + b * u + c * v
        );
        return;
    }

    // Vec2 Implementation (@uv)
    if let Some(_) = proto_type.as_slice::<Vec2>() {
        process_typed_attribute(
            final_geo, final_pos, target, cutter, source_prim_ids, op, attr_id, false,
            |v: Vec2| v,
            |a, b, c, u, v, w| a * w + b * u + c * v
        );
        return;
    }

    // F32 Implementation
    if let Some(_) = proto_type.as_slice::<f32>() {
        process_typed_attribute(
            final_geo, final_pos, target, cutter, source_prim_ids, op, attr_id, invert_on_diff,
            |v: f32| v,
            |a, b, c, u, v, w| a * w + b * u + c * v
        );
        return;
    }
}

fn process_typed_attribute<T>(
    final_geo: &mut Geometry,
    final_pos: &[Vec3],
    target: &Geometry,
    cutter: &Geometry,
    source_prim_ids: &[i32],
    op: BooleanOperation,
    attr_id: AttributeId,
    invert_on_diff: bool,
    norm_fn: impl Fn(T) -> T,
    mix_fn: impl Fn(T, T, T, f32, f32, f32) -> T + Copy,
) 
where T: Copy + Send + Sync + 'static + Default + std::fmt::Debug + std::ops::Neg<Output=T> + PartialEq
{
    let num_verts = final_geo.vertices().len();
    let mut final_values = vec![T::default(); num_verts];
    
    let t_attr = target.get_vertex_attribute(attr_id)
        .or_else(|| target.get_point_attribute(attr_id))
        .or_else(|| target.get_primitive_attribute(attr_id))
        .and_then(|a| a.as_slice::<T>());
        
    let c_attr = cutter.get_vertex_attribute(attr_id)
        .or_else(|| cutter.get_point_attribute(attr_id))
        .or_else(|| cutter.get_primitive_attribute(attr_id))
        .and_then(|a| a.as_slice::<T>());

    for (prim_idx, primitive) in final_geo.primitives().values().iter().enumerate() {
        if prim_idx >= source_prim_ids.len() { break; }
        let source_id = source_prim_ids[prim_idx];
        let vertices = primitive.vertices();

        if source_id == 0 {
            continue;
        }

        let (source_geo, s_attr, s_pos) = if source_id > 0 {
            (target, t_attr, target.get_point_position_attribute())
        } else {
            (cutter, c_attr, cutter.get_point_position_attribute())
        };

        let src_prim_idx = (source_id.abs() - 1) as usize;
        if src_prim_idx >= source_geo.primitives().len() { continue; }
        let src_prim = &source_geo.primitives().values()[src_prim_idx];
        let s_pos = s_pos.unwrap();

        for &v_idx in vertices {
            if let Some(v_dense) = final_geo.vertices().get_dense_index(v_idx.into()) {
                let pt_idx = final_geo.vertices().get(v_idx.into()).unwrap().point_id;
                let pt_dense = final_geo.points().get_dense_index(pt_idx.into()).unwrap();
                let pos = final_pos[pt_dense];

                if let Some(mut val) = interpolate_value_on_primitive(
                    pos, src_prim, src_prim_idx, source_geo, s_pos, s_attr, T::default(), mix_fn
                ) {
                    if invert_on_diff && matches!(op, BooleanOperation::Difference) && source_id < 0 {
                        val = -val;
                    }
                    val = norm_fn(val);
                    final_values[v_dense] = val;
                }
            }
        }
    }
    
    final_geo.insert_vertex_attribute(attr_id, Attribute::new(final_values));
}

fn interpolate_value_on_primitive<T: Copy + Send + Sync, F>(
    pos: Vec3,
    prim: &GeoPrimitive,
    prim_idx: usize,
    geo: &Geometry,
    positions: &[Vec3],
    attrs: Option<&[T]>,
    default: T,
    mix_fn: F,
) -> Option<T> 
where F: Fn(T, T, T, f32, f32, f32) -> T
{
    let vertices = prim.vertices();
    if vertices.len() < 3 { return None; }

    let get_pos = |vid: VertexId| -> Vec3 {
        let p_idx = geo.vertices().get(vid.into()).unwrap().point_id;
        let dense = geo.points().get_dense_index(p_idx.into()).unwrap();
        positions.get(dense).copied().unwrap_or(Vec3::ZERO)
    };

    let p0 = get_pos(vertices[0]);

    for i in 1..vertices.len() - 1 {
        let v1_idx = vertices[i];
        let v2_idx = vertices[i + 1];

        let p1 = get_pos(v1_idx);
        let p2 = get_pos(v2_idx);

        let v0v1 = p1 - p0;
        let v0v2 = p2 - p0;
        let v0p = pos - p0;

        let dot00 = v0v1.dot(v0v1);
        let dot01 = v0v1.dot(v0v2);
        let dot02 = v0v1.dot(v0p);
        let dot11 = v0v2.dot(v0v2);
        let dot12 = v0v2.dot(v0p);

        let inv_denom = 1.0 / (dot00 * dot11 - dot01 * dot01 + 1e-8);
        let u = (dot11 * dot02 - dot01 * dot12) * inv_denom;
        let v = (dot00 * dot12 - dot01 * dot02) * inv_denom;

        const TOL: f32 = -0.01; 
        if u >= TOL && v >= TOL && (u + v) <= (1.0 - TOL) {
            let get_val = |v_idx: VertexId| -> T {
                if let Some(buf) = attrs {
                    if buf.len() == geo.primitives().len() {
                        *buf.get(prim_idx).unwrap_or(&default)
                    } else if buf.len() == geo.vertices().len() {
                        let dense_v = geo.vertices().get_dense_index(v_idx.into()).unwrap();
                        *buf.get(dense_v).unwrap_or(&default)
                    } else {
                        let p_idx = geo.vertices().get(v_idx.into()).unwrap().point_id;
                        let dense_p = geo.points().get_dense_index(p_idx.into()).unwrap();
                        if buf.len() > dense_p {
                            *buf.get(dense_p).unwrap_or(&default)
                        } else {
                            default
                        }
                    }
                } else {
                    default
                }
            };

            let n0 = get_val(vertices[0]);
            let n1 = get_val(v1_idx);
            let n2 = get_val(v2_idx);

            let u_c = u.max(0.0).min(1.0);
            let v_c = v.max(0.0).min(1.0);
            let w_c = (1.0 - u_c - v_c).max(0.0);
            let sum = u_c + v_c + w_c;
            
            return Some(mix_fn(n0, n1, n2, u_c/sum, v_c/sum, w_c/sum));
        }
    }
    None
}
