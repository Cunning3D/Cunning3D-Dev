use bevy::math::{DVec2, DVec3};
use std::collections::{HashMap, HashSet};
use crate::libs::geometry::mesh::{Geometry, GeoPrimitive};
use crate::libs::geometry::ids::{VertexId, PointId};

/// Represents a vertex in the planar graph (local 2D space of a polygon).
#[derive(Debug, Clone)]
pub struct CutVertex {
    pub id: usize,
    pub uv: DVec2, // 2D coordinates in the polygon's projection plane
    
    // Metadata for reconstruction
    pub original_point_idx: Option<PointId>, // If this vertex matches an input point
    pub interpolated_from: Option<(usize, usize, f64)>, // Edge (p0, p1) and t
    pub is_intersection: bool,
}

/// Represents an edge in the planar graph.
#[derive(Debug, Clone)]
pub struct CutEdge {
    pub id: usize,
    pub start: usize, // CutVertex ID
    pub end: usize,   // CutVertex ID
    
    // Connectivity
    pub next: Option<usize>, // Next edge in loop (if computed)
    pub pair: Option<usize>, // Opposite edge (half-edge structure)
    
    // Classification
    pub is_boundary: bool, // Was this an edge of the original polygon?
    pub is_cut: bool,      // Is this a cut line from an intersection?
    
    pub visited: bool,     // For traversal algorithms
}

/// Represents a 2D planar graph overlayed on a 3D polygon.
/// Used to perform clipping operations.
pub struct CutGraph {
    pub prim_idx: usize,
    pub plane_origin: DVec3,
    pub plane_u_axis: DVec3,
    pub plane_v_axis: DVec3,
    pub plane_normal: DVec3,
    
    pub vertices: Vec<CutVertex>,
    pub edges: Vec<CutEdge>,
    
    // Adjacency: Vertex ID -> List of Outgoing Edge IDs
    pub adjacency: HashMap<usize, Vec<usize>>,
}

impl CutGraph {
    pub fn new(prim_idx: usize, geo: &Geometry, normal: DVec3) -> Option<Self> {
        let prim = &geo.primitives().values().get(prim_idx)?;
        let vertices = prim.vertices();
        if vertices.len() < 3 {
            return None;
        }

        // 1. Establish Projection Plane
        // We pick the first vertex as origin, and build a stable orthonormal basis from the normal.
        let v0_id = vertices[0];
        let p0_id = geo.vertices().get(v0_id.into())?.point_id;
        
        let p0_dense_idx = geo.points().get_dense_index(p0_id.into())?;
        let positions = geo.get_point_position_attribute()?;
        let p0_vec3 = positions.get(p0_dense_idx)?;
        
        let origin = DVec3::new(p0_vec3.x as f64, p0_vec3.y as f64, p0_vec3.z as f64);

        // Build basis (u, v) from normal using a robust strategy.
        // Choose an initial axis that is least aligned with the normal, then
        // project it onto the tangent plane and normalize.
        let abs_n = normal.abs();
        let mut u_axis = if abs_n.x <= abs_n.y && abs_n.x <= abs_n.z {
            DVec3::X
        } else if abs_n.y <= abs_n.x && abs_n.y <= abs_n.z {
            DVec3::Y
        } else {
            DVec3::Z
        };
        // Remove normal component from u_axis to make it tangent.
        u_axis = u_axis - normal * u_axis.dot(normal);
        let mut len2 = u_axis.length_squared();
        if len2 < 1e-12 {
            // Fallback: old helper-cross method if projection is degenerate.
            let helper = if normal.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
            u_axis = normal.cross(helper);
            len2 = u_axis.length_squared();
        }
        if len2 < 1e-12 {
            // Degenerate normal / polygon, skip this primitive.
            return None;
        }
        u_axis /= len2.sqrt();

        let mut v_axis = normal.cross(u_axis);
        let len2_v = v_axis.length_squared();
        if len2_v < 1e-12 {
            return None;
        }
        v_axis /= len2_v.sqrt();

        let mut graph = Self {
            prim_idx,
            plane_origin: origin,
            plane_u_axis: u_axis,
            plane_v_axis: v_axis,
            plane_normal: normal,
            vertices: Vec::new(),
            edges: Vec::new(),
            adjacency: HashMap::new(),
        };

        // 2. Add Original Loop
        let mut prev_v_id = 0;
        let start_v_id = 0;
        
        for (i, &v_idx) in vertices.iter().enumerate() {
            let pt_id = geo.vertices().get(v_idx.into())?.point_id;
            let pt_dense = geo.points().get_dense_index(pt_id.into())?;
            let pos = positions.get(pt_dense)?;
            let pos_d = DVec3::new(pos.x as f64, pos.y as f64, pos.z as f64);
            
            // Project to 2D
            let diff = pos_d - origin;
            let uv = DVec2::new(diff.dot(u_axis), diff.dot(v_axis));
            
            let v_id = graph.add_vertex(uv, Some(pt_id), false);
            
            if i > 0 {
                graph.add_edge(prev_v_id, v_id, true, false);
            }
            prev_v_id = v_id;
        }
        // Close loop
        graph.add_edge(prev_v_id, start_v_id, true, false);

        Some(graph)
    }

    pub fn add_vertex(&mut self, uv: DVec2, original_point: Option<PointId>, is_cut: bool) -> usize {
        let id = self.vertices.len();
        self.vertices.push(CutVertex {
            id,
            uv,
            original_point_idx: original_point,
            interpolated_from: None,
            is_intersection: is_cut,
        });
        id
    }

    pub fn add_edge(&mut self, start: usize, end: usize, is_boundary: bool, is_cut: bool) -> usize {
        let id = self.edges.len();
        let edge = CutEdge {
            id,
            start,
            end,
            next: None,
            pair: None,
            is_boundary,
            is_cut,
            visited: false,
        };
        self.edges.push(edge);
        self.adjacency.entry(start).or_default().push(id);
        
        // Add twin edge immediately for robustness
        let twin_id = id + 1;
        let twin_edge = CutEdge {
            id: twin_id,
            start: end,
            end: start,
            next: None,
            pair: Some(id),
            is_boundary,
            is_cut,
            visited: false,
        };
        self.edges[id].pair = Some(twin_id); 
        self.edges.push(twin_edge);
        self.adjacency.entry(end).or_default().push(twin_id);
        
        id
    }

    /// Split an edge at a given parameter t (0..1), creating a new vertex and updating topology.
    /// Returns the index of the new vertex.
    pub fn split_edge(&mut self, edge_idx: usize, t: f64, new_uv: DVec2) -> usize {
        // 1. Get original edge data
        let (start, end, is_boundary, is_cut, pair_idx) = {
            let e = &self.edges[edge_idx];
            (e.start, e.end, e.is_boundary, e.is_cut, e.pair)
        };

        // 2. Create new vertex
        // Record that this vertex comes from interpolating between 'start' and 'end' at factor 't'
        let mut new_v_id = self.add_vertex(new_uv, None, true);
        self.vertices[new_v_id].interpolated_from = Some((start, end, t));
        
        // 3. Modify original edge to end at new vertex
        // We reuse the existing edge struct for the first segment (start -> new)
        // so that incoming pointers to 'edge_idx' (like from prev edge) remain valid.
        self.edges[edge_idx].end = new_v_id;
        
        // Remove original connection from adjacency
        // (Optimized out for now as adjacency stores outgoing)
        
        // 4. Create second segment (new -> end)
        let second_seg_id = self.edges.len();
        let second_seg = CutEdge {
            id: second_seg_id,
            start: new_v_id,
            end,
            next: self.edges[edge_idx].next, // Inherit next ptr
            pair: None, // Will fix later
            is_boundary,
            is_cut,
            visited: false,
        };
        self.edges.push(second_seg);
        self.adjacency.entry(new_v_id).or_default().push(second_seg_id);
        
        // Link: first -> second
        self.edges[edge_idx].next = Some(second_seg_id);

        // 5. Handle Twin Edge (pair) if exists
        // If we split A->B, we must also split B->A to maintain topological consistency.
        if let Some(pair_id) = pair_idx {
            // The pair goes End -> Start.
            // We split it into: End -> NewV -> Start.
            // Corresponding to: NewV -> End (second_seg) and Start -> NewV (original modified)
            
            // Pair Segment 1: End -> NewV
            // This is the twin of second_seg (NewV -> End)
            
            // Reuse pair_id for the segment starting at End?
            // pair_id was End -> Start. Now it should be End -> NewV.
            self.edges[pair_id].end = new_v_id;
            // pair_id is now End -> NewV. It pairs with second_seg (NewV -> End).
            
            self.edges[pair_id].pair = Some(second_seg_id);
            self.edges[second_seg_id].pair = Some(pair_id);
            
            // Pair Segment 2: NewV -> Start
            // This is the twin of edge_idx (Start -> NewV)
            let pair_seg_2_id = self.edges.len();
            let pair_seg_2 = CutEdge {
                id: pair_seg_2_id,
                start: new_v_id,
                end: start,
                next: self.edges[pair_id].next, // Inherit next
                pair: Some(edge_idx),
                is_boundary,
                is_cut,
                visited: false,
            };
            self.edges.push(pair_seg_2);
            self.adjacency.entry(new_v_id).or_default().push(pair_seg_2_id);
            
            // Link pair flow: pair_id -> pair_seg_2
            self.edges[pair_id].next = Some(pair_seg_2_id);
            
            // Fix original edge pair
            self.edges[edge_idx].pair = Some(pair_seg_2_id);
        }

        new_v_id
    }
    
    /// Insert a segment (from intersection) into the graph.
    /// This handles all splitting logic:
    /// 1. Finds intersections with existing edges.
    /// 2. Splits existing edges at intersection points.
    /// 3. Splits the inserted segment itself.
    /// 4. Adds the resulting sub-segments to the graph.
    pub fn insert_segment(&mut self, p0: DVec2, p1: DVec2) {
        // Queue of segments to process (start_uv, end_uv)
        // We start with the full segment.
        // If we hit an intersection, we split and push the remainder back to queue.
        let mut queue = vec![(p0, p1)];
        
        while let Some((curr_p0, curr_p1)) = queue.pop() {
            if curr_p0.distance_squared(curr_p1) < 1e-12 {
                continue; // Degenerate
            }

            // Find the closest intersection along this segment with ANY existing edge.
            // Brute force for now (O(E)). In production, use a spatial index (Grid/Quadtree) for the graph edges.
            // Since N is small for a single polygon (usually < 100), brute force is fine.
            
            let mut closest_hit: Option<(usize, f64, f64)> = None; // (edge_idx, t_edge, t_seg)
            let mut min_t_seg = 1.0;
            
            // Iterate all edges to find intersection
            for i in 0..self.edges.len() {
                let edge = &self.edges[i];
                if edge.visited { continue; } // Skip if marked (or twin?) No, we need to check all physical edges.
                // We only check "primary" edges (e.g. id even) to avoid double checking twins?
                // Or check all. Twins are geometrically same. Checking one is enough.
                if edge.pair.is_some() && edge.id > edge.pair.unwrap() {
                    continue; // Skip twin
                }
                
                let v_start = self.vertices[edge.start].uv;
                let v_end = self.vertices[edge.end].uv;
                
                // Robust Segment-Segment Intersection
                if let Some((t_edge, t_seg)) = robust_seg_seg_intersection(v_start, v_end, curr_p0, curr_p1) {
                    // Ignore start/end points (epsilon check) to avoid splitting at existing vertices needlessly
                    // unless we want to snap?
                    // For exact boolean, we should snap.
                    if t_seg > 1e-6 && t_seg < min_t_seg - 1e-6 {
                        min_t_seg = t_seg;
                        closest_hit = Some((i, t_edge, t_seg));
                    }
                }
            }
            
            if let Some((hit_edge_idx, t_edge, _t_seg)) = closest_hit {
                // We hit an existing edge!
                
                // 1. Calculate intersection point in UV space, using the EXISTING edge
                // for numerical stability.
                let hit_v_start = self.vertices[self.edges[hit_edge_idx].start].uv;
                let hit_v_end = self.vertices[self.edges[hit_edge_idx].end].uv;
                let intersection_pt = hit_v_start.lerp(hit_v_end, t_edge); // DVec2 lerp

                // 2. Split the existing edge. This updates topology and creates a
                // new vertex in the graph at the intersection location.
                let new_v_id = self.split_edge(hit_edge_idx, t_edge, intersection_pt);

                // 3. The current segment (curr_p0 -> curr_p1) is split into two:
                //    (curr_p0 -> intersection_pt) and (intersection_pt -> curr_p1).
                //    Process the first part and queue the second.

                // 3.1 Handle curr_p0
                let v_start_id = self.find_or_add_vertex(curr_p0);

                // 3.2 Add edge from v_start_id to new_v_id
                if v_start_id != new_v_id {
                    if self.find_edge(v_start_id, new_v_id).is_none() {
                        self.add_edge(v_start_id, new_v_id, false, true); // is_boundary=false, is_cut=true
                    }
                }

                // 3.3 Queue remainder
                queue.push((intersection_pt, curr_p1));
            } else {
                // No intersection. Safe to add segment (curr_p0, curr_p1).
                // Check if vertices exist (snap) or add new.
                let v0 = self.find_or_add_vertex(curr_p0);
                let v1 = self.find_or_add_vertex(curr_p1);
                if v0 != v1 {
                     // Check if edge exists
                     if self.find_edge(v0, v1).is_none() {
                         self.add_edge(v0, v1, false, true); // is_boundary=false, is_cut=true
                     }
                }
            }
        }
    }

    fn find_or_add_vertex(&mut self, uv: DVec2) -> usize {
        for v in &self.vertices {
            if v.uv.distance_squared(uv) < 1e-10 {
                return v.id;
            }
        }
        self.add_vertex(uv, None, true)
    }

    /// Extract closed regions (faces) from the graph.
    /// Returns a list of vertex indices for each region.
    /// Distinguishes between Holes (CW) and Shells (CCW).
    pub fn extract_regions(&mut self) -> Vec<Vec<usize>> {
        let mut regions = Vec::new();
        
        // 1. Sort outgoing edges angularly around each vertex
        // This is crucial for "Left Turn" traversal.
        for (v_id, edges) in self.adjacency.iter_mut() {
            let center = self.vertices[*v_id].uv;
            // Sort edges by angle using atan2
            edges.sort_by(|&a_id, &b_id| {
                let a = &self.edges[a_id];
                let b = &self.edges[b_id];
                let pa = self.vertices[a.end].uv - center;
                let pb = self.vertices[b.end].uv - center;
                let angle_a = pa.y.atan2(pa.x);
                let angle_b = pb.y.atan2(pb.x);
                angle_a.partial_cmp(&angle_b).unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        // 2. Mark all half-edges as unvisited
        // We reuse the 'visited' flag on edges.
        // Note: Edge ID 2k and 2k+1 are pairs.
        for e in &mut self.edges {
            e.visited = false;
        }

        // 3. Traverse cycles
        // Iterate every half-edge. If not visited, trace its cycle.
        for start_edge_id in 0..self.edges.len() {
            if self.edges[start_edge_id].visited {
                continue;
            }

            let mut cycle = Vec::new();
            let mut curr_edge_id = start_edge_id;
            
            loop {
                let curr_edge = &mut self.edges[curr_edge_id];
                if curr_edge.visited {
                    break; // Should match start_edge_id if logic is correct
                }
                curr_edge.visited = true;
                
                // Add destination vertex to region (standard polygon format)
                cycle.push(curr_edge.end);
                
                let v_end = curr_edge.end;
                
                // Find next edge: The one that is "most left" relative to incoming edge.
                // In a sorted adjacency list, this is the edge immediately BEFORE the twin of current edge.
                // Or immediately AFTER?
                // Let's trace: Incoming is 'curr_edge'. Twin is 'pair'. Twin points OUT from v_end.
                // We want the next edge in CCW order around v_end.
                
                if let Some(pair_id) = curr_edge.pair {
                    if let Some(outgoing) = self.adjacency.get(&v_end) {
                        // Find position of pair_id in outgoing list
                        if let Some(pos) = outgoing.iter().position(|&id| id == pair_id) {
                            // The "Next" edge in CCW face traversal is the one *after* the pair in the CCW sorted list around vertex.
                            // Wait, atan2 sorts -PI to PI.
                            // Let's assume standard CCW sort.
                            // If we come in on edge E, we want to leave on the edge that makes the sharpest Left Turn.
                            // Which is the next edge in the sorted radial list after E's twin.
                            
                            let next_idx = (pos + 1) % outgoing.len();
                            curr_edge_id = outgoing[next_idx];
                        } else {
                            break; // Broken topology?
                        }
                    } else {
                        break;
                    }
                } else {
                    break; // Dead end
                }

                if curr_edge_id == start_edge_id {
                    break;
                }
            }
            
            if !cycle.is_empty() {
                // Filter out non-simple cycles that revisit vertices (self-intersections / bow-ties).
                if !is_simple_cycle(&cycle) {
                    continue;
                }

                // Check winding / area
                // Filter out degenerate cycles (area ~ 0) or holes if we only want solids for now?
                if calculate_signed_area(&cycle, &self.vertices) > 1e-6 {
                    regions.push(cycle);
                }
            }
        }
        
        regions
    }
    pub fn find_edge(&self, start: usize, end: usize) -> Option<usize> {
        if let Some(outgoing) = self.adjacency.get(&start) {
            for &edge_id in outgoing {
                if self.edges[edge_id].end == end {
                    return Some(edge_id);
                }
            }
        }
        None
    }
}

fn calculate_signed_area(indices: &[usize], vertices: &[CutVertex]) -> f64 {
    let mut area = 0.0;
    for i in 0..indices.len() {
        let j = (i + 1) % indices.len();
        let p0 = vertices[indices[i]].uv;
        let p1 = vertices[indices[j]].uv;
        area += p0.x * p1.y - p1.x * p0.y;
    }
    area * 0.5
}

/// Returns true if the cycle does not revisit any vertex (i.e. simple polygon).
fn is_simple_cycle(indices: &[usize]) -> bool {
    let mut seen = HashSet::new();
    for &idx in indices {
        if !seen.insert(idx) {
            return false;
        }
    }
    true
}

// Helper: 2D Segment-Segment Intersection
// Returns (t_a, t_b) if they intersect strictly within (0..1)
// or slightly loosely for robustness.
fn robust_seg_seg_intersection(a0: DVec2, a1: DVec2, b0: DVec2, b1: DVec2) -> Option<(f64, f64)> {
    let da = a1 - a0;
    let db = b1 - b0;
    let dc = b0 - a0;
    
    let cross_ab = da.x * db.y - da.y * db.x;
    
    if cross_ab.abs() < 1e-12 {
        return None; // Parallel
    }
    
    let t_a = (dc.x * db.y - dc.y * db.x) / cross_ab;
    let t_b = (dc.x * da.y - dc.y * da.x) / cross_ab;
    
    // Check bounds with epsilon
    if t_a >= 0.0 && t_a <= 1.0 && t_b >= 0.0 && t_b <= 1.0 {
        Some((t_a, t_b))
    } else {
        None
    }
}
