use bevy::math::Vec3;
use crate::libs::geometry::mesh::Geometry;
use std::collections::HashMap;
use crate::libs::geometry::ids::VertexId;

/// Represents a source "Truth" point derived from the input geometry.
/// Unlike topology vertices which are welded, these are "Split Points" (Face Corners).
/// A single welded vertex might generate multiple TruthPoints if it sits on a hard edge 
/// (different normals) or is shared by multiple primitives.
#[derive(Debug, Clone, Copy)]
pub struct TruthPoint {
    pub position: Vec3,
    pub normal: Vec3,
    pub original_prim_id: i32,
    // Future expansion: UVs, Groups, Weights, etc.
}

/// A spatial index for quick attribute restoration.
pub struct TruthCloud {
    points: Vec<TruthPoint>,
    // Simple spatial hash grid for acceleration
    // Key: (x, y, z) quantized cell coordinate
    // Value: List of indices into `points`
    grid: HashMap<(i32, i32, i32), Vec<usize>>,
    cell_size: f32,
}

impl TruthCloud {
    /// Build a TruthCloud from a geometry.
    /// 
    /// * `geo`: The source geometry.
    /// * `id_offset`: ID offset to distinguish Target vs Cutter (e.g., 1 for Target, -1 for Cutter).
    ///                Or simply pass pure PrimID and handle type externally.
    ///                Let's stick to signed IDs for simplicity: +ve for Target, -ve for Cutter.
    pub fn build(geo: &Geometry, id_sign: i32) -> Self {
        let positions = match geo.get_point_position_attribute() {
            Some(p) => p,
            None => return Self::empty(),
        };

        let mut truth_points = Vec::new();
        
        // 1. Calculate Face Normals on the fly or use existing ones.
        //    Since we need "Split Normals" (Face Normals), computing per-primitive is safest.
        
        for (prim_idx, prim) in geo.primitives().values().iter().enumerate() {
            let verts = prim.vertices();
            if verts.len() < 3 { continue; }

            let get_p = |vid: VertexId| {
                let v = geo.vertices().get(vid.into())?;
                let pid = geo.points().get_dense_index(v.point_id.into())?;
                positions.get(pid)
            };

            let p0 = match get_p(verts[0]) { Some(p) => p, None => continue };
            let p1 = match get_p(verts[1]) { Some(p) => p, None => continue };
            let p2 = match get_p(verts[2]) { Some(p) => p, None => continue };
            
            let n = (*p1 - *p0).cross(*p2 - *p0).normalize_or_zero();

            // Signed ID: + (idx+1) or - (idx+1)
            let raw_id = (prim_idx as i32) + 1;
            let final_id = raw_id * id_sign.signum();

            // Create a TruthPoint for each vertex in this primitive
            for &v_idx in verts {
                if let Some(pt_pos) = get_p(v_idx) {
                    truth_points.push(TruthPoint {
                        position: *pt_pos,
                        normal: n,
                        original_prim_id: final_id,
                    });
                }
            }
        }

        // 2. Build Spatial Grid
        // Heuristic cell size: average bounding box / 10? 
        // Or fixed reasonable size? Let's use 0.1 for now, or adapt to bounds.
        // Adaptive is better.
        let cell_size = if truth_points.is_empty() { 1.0 } else {
            // Compute bounds
            let mut min = Vec3::splat(f32::MAX);
            let mut max = Vec3::splat(f32::MIN);
            for p in &truth_points {
                min = min.min(p.position);
                max = max.max(p.position);
            }
            let extent = max - min;
            let avg_dim = (extent.x + extent.y + extent.z) / 3.0;
            // Aim for roughly 10-20 cells across
            (avg_dim / 20.0).max(0.001) 
        };

        let mut grid = HashMap::new();
        for (i, tp) in truth_points.iter().enumerate() {
            let key = Self::quantize(tp.position, cell_size);
            grid.entry(key).or_insert(Vec::new()).push(i);
        }

        Self {
            points: truth_points,
            grid,
            cell_size,
        }
    }

    fn empty() -> Self {
        Self { points: Vec::new(), grid: HashMap::new(), cell_size: 1.0 }
    }

    fn quantize(p: Vec3, cell_size: f32) -> (i32, i32, i32) {
        (
            (p.x / cell_size).floor() as i32,
            (p.y / cell_size).floor() as i32,
            (p.z / cell_size).floor() as i32,
        )
    }

    /// Query the closest matching attribute.
    /// 
    /// * `pos`: Query position.
    /// * `normal`: Query normal (from result mesh).
    /// * `pos_tolerance`: Max distance to consider a match.
    /// * `normal_tolerance`: Min dot product to consider a match (e.g. 0.99).
    pub fn query(&self, pos: Vec3, normal: Vec3, pos_tolerance: f32, normal_tolerance: f32) -> Option<i32> {
        if self.points.is_empty() { return None; }

        let center_key = Self::quantize(pos, self.cell_size);
        
        let mut best_id = None;
        let mut min_dist_sq = pos_tolerance * pos_tolerance;

        // Search 3x3x3 neighborhood
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let key = (center_key.0 + dx, center_key.1 + dy, center_key.2 + dz);
                    if let Some(indices) = self.grid.get(&key) {
                        for &idx in indices {
                            let tp = &self.points[idx];
                            
                            // 1. Distance Check
                            let d2 = tp.position.distance_squared(pos);
                            if d2 > min_dist_sq { continue; }

                            // 2. Normal Check
                            // We only care if normals are roughly aligned.
                            if tp.normal.dot(normal) < normal_tolerance { continue; }

                            // Found a better match
                            min_dist_sq = d2;
                            best_id = Some(tp.original_prim_id);
                        }
                    }
                }
            }
        }

        best_id
    }
}
