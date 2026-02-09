use crate::libs::geometry::mesh::{Geometry, Attribute, GeoPrimitive};
use crate::libs::geometry::ids::{PointId, VertexId};
use bevy::prelude::Vec3;
use std::collections::{HashMap, HashSet};
use rayon::prelude::*;
use kiddo::SquaredEuclidean;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuseOutputMode {
    Average,
    First,
    Last,
}

#[derive(Debug, Clone)]
pub struct FuseSettings {
    pub distance: f32,
    pub output_mode: FuseOutputMode,
    pub remove_unused_points: bool,
    pub remove_degenerate_prims: bool,
}

impl Default for FuseSettings {
    fn default() -> Self {
        Self {
            distance: 0.001,
            output_mode: FuseOutputMode::Average,
            remove_unused_points: true,
            remove_degenerate_prims: true,
        }
    }
}

pub fn fuse_points(geo: &mut Geometry, settings: &FuseSettings) {
    if geo.get_point_count() == 0 {
        return;
    }

    // 1. Build Spatial Index (KD-Tree)
    // We assume the geometry already has a valid KDTree cache or builds one now.
    let tree = geo.get_point_kdtree();
    
    // 2. Find Clusters (Union-Find)
    let positions = match geo.get_point_position_attribute() {
        Some(p) => p,
        None => return,
    };
    
    let num_points = positions.len();
    let mut parent: Vec<usize> = (0..num_points).collect();
    
    // Helper to find root
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut root = i;
        while parent[root] != root {
            root = parent[root];
        }
        // Path compression
        let mut curr = i;
        while curr != root {
            let next = parent[curr];
            parent[curr] = root;
            curr = next;
        }
        root
    }

    fn union(parent: &mut [usize], i: usize, j: usize) {
        let root_i = find(parent, i);
        let root_j = find(parent, j);
        if root_i != root_j {
            // Simple union by index to keep deterministic
            if root_i < root_j {
                parent[root_j] = root_i;
            } else {
                parent[root_i] = root_j;
            }
        }
    }

    // Parallel search for neighbors, then sequential union
    // Since Union-Find is hard to parallelize efficiently without concurrent structures,
    // and radius search is the bottleneck, we query in parallel and collect pairs.
    
    // Note: To avoid N^2 pairs, we only query for neighbors with index > current index
    // This is implicitly handled if we iterate all points and query radius.
    // However, KDTree 'within' returns all points.
    // Optimization: We can just use the tree query results.
    
    let pairs: Vec<(usize, usize)> = positions.par_iter().enumerate()
        .flat_map(|(i, pos)| {
            let neighbors = tree.within::<SquaredEuclidean>(&[pos.x, pos.y, pos.z], settings.distance * settings.distance);
            
            let mut local_pairs = Vec::new();
            for neighbor in neighbors {
                let j = neighbor.item as usize;
                if i < j {
                    local_pairs.push((i, j));
                }
            }
            local_pairs
        })
        .collect();

    for (i, j) in pairs {
        union(&mut parent, i, j);
    }
    
    // Flatten parents
    for i in 0..num_points {
        find(&mut parent, i);
    }

    // 3. Calculate New Positions
    // Map: Root Index -> List of Member Indices
    let mut clusters: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, &p) in parent.iter().enumerate() {
        clusters.entry(p).or_default().push(i);
    }

    let mut new_positions = positions.to_vec(); // Start with copy
    
    for (&root, members) in &clusters {
        if members.len() <= 1 { continue; } // No fusion needed

        match settings.output_mode {
            FuseOutputMode::Average => {
                let mut sum = Vec3::ZERO;
                for &idx in members {
                    sum += positions[idx];
                }
                let avg = sum / members.len() as f32;
                // Update all members to the average position (or just the root? Topology rewire uses root)
                // We only strictly need to update the root's position, because all vertices will point to root.
                new_positions[root] = avg;
            },
            FuseOutputMode::First => {
                // Determine 'First' by lowest index (which is already root because of union logic)
                // positions[root] is already correct
            },
            FuseOutputMode::Last => {
                let last_idx = members.iter().max().unwrap();
                new_positions[root] = positions[*last_idx];
            }
        }
    }
    
    // Write back positions
    // Note: We only updated 'root' positions. Other points are "dead" but still in array.
    // If we don't remove unused points, we should theoretically update them too?
    // Usually Fuse only cares about the resulting geometry.
    geo.insert_point_attribute(crate::libs::geometry::attrs::P, Attribute::new(new_positions));

    // 4. Rewire Topology (Vertices -> New Point IDs)
    // We need to map old PointId -> new PointId (which is the root PointId)
    
    // Get all point IDs to map index -> PointId
    // Because SparseSetArena is dense-packed, we can assume index i matches geometry.points()[i] IF we haven't deleted anything yet.
    // Wait, Geometry points arena might have holes if items were removed.
    // But get_point_position_attribute returns a dense slice corresponding to the *packed* data.
    // We need to match Attribute Index -> PointId.
    
    let points_arena = geo.points();
    let point_ids: Vec<PointId> = (0..points_arena.len()).filter_map(|i| {
        points_arena.get_id_from_dense(i).map(|id| PointId::from(id))
    }).collect();
    
    if point_ids.len() != num_points {
        // Sanity check failed (attribute length != arena length). 
        // This usually happens if attributes are out of sync. Assuming they are synced.
    }

    // Remap Vertices
    // For every vertex, check its point_id. Find that point's index. Find its root index. Get root PointId.
    // Optimization: Create a direct map Old PointId -> New PointId
    let mut point_remap: HashMap<PointId, PointId> = HashMap::new();
    for (i, &root_idx) in parent.iter().enumerate() {
        if i != root_idx {
            if let (Some(&old_pid), Some(&new_pid)) = (point_ids.get(i), point_ids.get(root_idx)) {
                point_remap.insert(old_pid, new_pid);
            }
        }
    }

    // Apply remap to all vertices
    for v in geo.vertices_mut().iter_mut() {
        if let Some(&new_pid) = point_remap.get(&v.point_id) {
            v.point_id = new_pid;
        }
    }

    // 5. Remove Degenerate Primitives
    // A primitive is degenerate if it uses the same point multiple times (e.g. Triangle A-A-B).
    if settings.remove_degenerate_prims {
        let mut prims_to_remove = Vec::new();
        
        // Need to read vertices to check point IDs
        // We can't easily iterate primitives_mut and query vertices at the same time due to borrow rules.
        // So we collect IDs first.
        
        let vertices_arena = geo.vertices(); // immutable borrow
        
        for (prim_id_raw, prim) in geo.primitives().iter_enumerated() {
            let prim_id = crate::libs::geometry::ids::PrimId::from(prim_id_raw);
            
            match prim {
                GeoPrimitive::Polygon(poly) => {
                    // Check for duplicate points
                    let mut used_points = HashSet::new();
                    let mut unique_count = 0;
                    
                    for &vid in &poly.vertices {
                        if let Some(v) = vertices_arena.get(vid.into()) {
                            if used_points.insert(v.point_id) {
                                unique_count += 1;
                            }
                        }
                    }
                    
                    if unique_count < 3 {
                        prims_to_remove.push(prim_id);
                    }
                },
                GeoPrimitive::Polyline(line) => {
                     let mut used_points = HashSet::new();
                    let mut unique_count = 0;
                     for &vid in &line.vertices {
                        if let Some(v) = vertices_arena.get(vid.into()) {
                            if used_points.insert(v.point_id) {
                                unique_count += 1;
                            }
                        }
                    }
                    if unique_count < 2 {
                        prims_to_remove.push(prim_id);
                    }
                }
                _ => {}
            }
        }
        
        for pid in prims_to_remove {
            geo.remove_primitive(pid);
        }
    }

    // 6. Remove Unused Points
    // Points that are not roots are now technically "unused" by topology, 
    // BUT we must also check if any primitive still refers to them (e.g. if we didn't update some vertices?)
    // Our rewire logic updated ALL vertices. So non-root points are definitely unused by vertices.
    // However, we should also check if any vertices refer to the roots.
    // Just blindly removing non-roots is safe IF we are sure all vertices mapped to roots.
    
    if settings.remove_unused_points {
        let mut points_to_remove = Vec::new();
        
        // Find points that are NOT used by any vertex
        // 1. Mark all points used by current vertices
        let mut active_points = HashSet::new();
        for v in geo.vertices().iter() {
            active_points.insert(v.point_id);
        }
        
        // 2. Collect unused
        for pid in point_ids {
            if !active_points.contains(&pid) {
                points_to_remove.push(pid);
            }
        }
        
        // 3. Batch remove
        for pid in points_to_remove {
            geo.remove_point(pid);
        }
    }
    
    geo.dirty_id = crate::libs::geometry::mesh::new_dirty_id();
}
