//! Face Rebuild: port of Blender bev_rebuild_polygon / bevel_rebuild_existing_polygons (6744–6957).
//! Reconstructs original faces with new bevel boundary vertices, preserving topology.
use super::Poly;
use crate::libs::geometry::mesh::{GeoPrimitive, Geometry};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

/// Mapping from original vertex to its bevel boundary vertices.
#[derive(Clone, Debug, Default)]
pub struct BevVertMapping {
    /// original_point_id -> vec of (new_point_id, profile_k index)
    pub boundary_verts: HashMap<usize, Vec<(usize, usize)>>,
    /// For each original point, the segment count.
    pub seg_count: HashMap<usize, usize>,
}

/// Result of face rebuild operation.
#[derive(Clone, Debug, Default)]
pub struct FaceRebuildResult {
    /// New primitives to replace the original.
    pub new_polys: Vec<Poly>,
    /// Original face indices that were rebuilt (for deletion).
    pub rebuilt_faces: HashSet<usize>,
    /// Mapping: (new_vertex_idx) -> (original_bevel_vertex_idx) for attribute interpolation.
    pub nv_bv_map: HashMap<usize, usize>,
}

/// Blender bev_rebuild_polygon (6744-6937): rebuild a single face with bevel boundary verts.
///
/// # Algorithm (Blender 6753-6878):
/// For each loop in the face:
/// - If the loop vertex is beveled (has boundary verts):
///   - Find the direction (CCW or CW) to traverse boundary verts
///   - Collect all boundary verts from eprev.rightv to e.leftv (or reverse)
/// - Else: keep the original vertex
pub fn rebuild_polygon(
    _orig_face_idx: usize,
    orig_verts: &[usize], // Original vertex indices of the face
    bev_mapping: &BevVertMapping,
    edge_half_map: &HashMap<(usize, usize), (usize, usize)>, // (v0,v1) -> (bev_vert_idx, edge_half_idx)
) -> Option<(Vec<usize>, HashMap<usize, usize>)> {
    let mut new_verts: Vec<usize> = Vec::new();
    let mut nv_bv_map: HashMap<usize, usize> = HashMap::new();
    let n = orig_verts.len();
    let mut do_rebuild = false;

    for i in 0..n {
        let v = orig_verts[i];
        let v_prev = orig_verts[if i == 0 { n - 1 } else { i - 1 }];
        let v_next = orig_verts[(i + 1) % n];

        if let Some(boundary) = bev_mapping.boundary_verts.get(&v) {
            if boundary.is_empty() {
                new_verts.push(v);
                nv_bv_map.insert(v, v);
                continue;
            }

            let seg = bev_mapping.seg_count.get(&v).copied().unwrap_or(1);

            // Find edge halves for prev and next edges
            let e_prev = edge_half_map.get(&(v_prev.min(v), v_prev.max(v)));
            let e_next = edge_half_map.get(&(v.min(v_next), v.max(v_next)));

            // Determine traversal direction: CCW or CW around bevel vertex
            // Blender 6766-6789: based on edge ordering in bevel vertex
            let go_ccw = match (e_prev, e_next) {
                (Some(&(_, ep_idx)), Some(&(_, en_idx))) => ep_idx < en_idx,
                _ => true, // Default CCW
            };

            // Collect boundary verts in order
            // Blender 6833-6840 (CCW) or 6860-6867 (CW)
            if go_ccw {
                for k in 0..=seg {
                    if let Some(&(new_v, _)) = boundary.iter().find(|(_, pk)| *pk == k) {
                        new_verts.push(new_v);
                        nv_bv_map.insert(new_v, v);
                    }
                }
            } else {
                for k in (0..=seg).rev() {
                    if let Some(&(new_v, _)) = boundary.iter().find(|(_, pk)| *pk == k) {
                        new_verts.push(new_v);
                        nv_bv_map.insert(new_v, v);
                    }
                }
            }
            do_rebuild = true;
        } else {
            new_verts.push(v);
            nv_bv_map.insert(v, v);
        }
    }

    if do_rebuild && new_verts.len() >= 3 {
        Some((new_verts, nv_bv_map))
    } else {
        None
    }
}

/// Blender bevel_rebuild_existing_polygons (6940-6957): rebuild all faces touching beveled verts.
pub fn rebuild_existing_polygons(
    geo: &Geometry,
    beveled_points: &HashSet<usize>,
    bev_mapping: &BevVertMapping,
    edge_half_map: &HashMap<(usize, usize), (usize, usize)>,
) -> FaceRebuildResult {
    let mut result = FaceRebuildResult::default();

    for (face_idx, prim) in geo.primitives().iter().enumerate() {
        let GeoPrimitive::Polygon(poly) = prim else {
            continue;
        };
        let touches_beveled = poly.vertices.iter().any(|&vid| {
            geo.vertices()
                .get(vid.into())
                .and_then(|v| geo.points().get_dense_index(v.point_id.into()))
                .map(|pi| beveled_points.contains(&pi))
                .unwrap_or(false)
        });

        if !touches_beveled {
            continue;
        }
        if result.rebuilt_faces.contains(&face_idx) {
            continue;
        }

        let orig_points: Vec<usize> = poly
            .vertices
            .iter()
            .filter_map(|&vid| {
                geo.vertices()
                    .get(vid.into())
                    .and_then(|v| geo.points().get_dense_index(v.point_id.into()))
            })
            .collect();

        if let Some((new_verts, nv_map)) =
            rebuild_polygon(face_idx, &orig_points, bev_mapping, edge_half_map)
        {
            result.new_polys.push(new_verts.clone());
            result.rebuilt_faces.insert(face_idx);
            result.nv_bv_map.extend(nv_map);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rebuild_simple() {
        let mut mapping = BevVertMapping::default();
        // Vertex 0 has 3 boundary verts (seg=2)
        mapping
            .boundary_verts
            .insert(0, vec![(10, 0), (11, 1), (12, 2)]);
        mapping.seg_count.insert(0, 2);

        let orig = vec![0, 1, 2];
        let edge_map = HashMap::new();

        let result = rebuild_polygon(0, &orig, &mapping, &edge_map);
        assert!(result.is_some());
        let (verts, _) = result.unwrap();
        // Should have 3 boundary verts + 2 original = 5 total
        assert_eq!(verts.len(), 5);
    }
}
