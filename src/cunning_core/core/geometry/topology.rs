use crate::libs::geometry::mesh::{Geometry, GeoPrimitive};
use crate::libs::geometry::ids::{PointId, PrimId, HalfEdgeId};
use crate::libs::geometry::sparse_set::SparseSetArena;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// A Half-Edge represents a directed edge acting as part of a face boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HalfEdge {
    pub origin_point: PointId,
    /// Points to the next half-edge in the ring of half-edges sharing the same physical edge (u,v).
    /// In a 2-manifold mesh, this acts as the 'pair', pointing to the opposite half-edge.
    /// In a boundary case, it points to itself.
    /// In a non-manifold case, it cycles through all incident faces.
    pub next_equivalent: HalfEdgeId, 
    pub next: HalfEdgeId,
    pub primitive_index: PrimId,
}

/// A comprehensive topology structure using the Half-Edge data structure.
/// Upgraded to use SparseSetArena for O(1) dynamic editing and safety.
/// Now supports non-manifold topology via `next_equivalent` ring.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Topology {
    // Core storage: Arena instead of Vec
    pub half_edges: SparseSetArena<HalfEdge>,
    
    /// Map from Primitive ID -> One starting HalfEdge ID
    pub primitive_to_halfedge: HashMap<PrimId, HalfEdgeId>,
    
    /// Map from Point ID -> One outgoing HalfEdge ID (if any)
    pub point_to_halfedge: HashMap<PointId, HalfEdgeId>,
    
    /// Tracks boundary edges (edges where next_equivalent == self).
    pub boundary_halfedges: Vec<HalfEdgeId>,
}

impl Topology {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build topology from geometry.
    /// Supports Polygons. Polylines are ignored in half-edge structure.
    pub fn build_from(geo: &Geometry) -> Self {
        let mut topo = Topology::new();
        
        // Temporary map to group all half-edges sharing a physical edge.
        // Key: Sorted (p_min, p_max). Value: List of HalfEdgeIds on this edge.
        let mut edge_map: HashMap<(PointId, PointId), Vec<HalfEdgeId>> = HashMap::new();
        
        // 1. Create HalfEdges for all primitives
        for (prim_id_idx, prim) in geo.primitives().iter_enumerated() {
            // Only process Polygons
            let vertices = match prim {
                GeoPrimitive::Polygon(poly) => &poly.vertices,
                _ => continue, 
            };

            if vertices.len() < 3 {
                continue;
            }

            let prim_id = PrimId::from(prim_id_idx);
            let indices = vertices; // These are VertexIds
            let count = indices.len();
            
            // We need to look up PointIds for these VertexIds
            let mut face_point_ids = Vec::with_capacity(count);
            for &v_id in indices {
                if let Some(v) = geo.vertices().get(v_id.into()) {
                    face_point_ids.push(v.point_id);
                } else {
                    face_point_ids.clear();
                    break;
                }
            }
            
            if face_point_ids.len() != count {
                continue;
            }

            let mut face_half_edges = Vec::with_capacity(count);

            for i in 0..count {
                let p_curr = face_point_ids[i];
                let p_next = face_point_ids[(i + 1) % count];
                
                // Create HalfEdge placeholder
                let he = HalfEdge {
                    origin_point: p_curr,
                    next_equivalent: HalfEdgeId::INVALID, // Fill later
                    next: HalfEdgeId::INVALID, // Fill later
                    primitive_index: prim_id,
                };

                // Insert into Arena to get ID
                let he_idx_raw = topo.half_edges.insert(he);
                let he_id = HalfEdgeId::from(he_idx_raw);
                
                face_half_edges.push(he_id);
                
                // Record Primitive -> HalfEdge
                if i == 0 {
                    topo.primitive_to_halfedge.insert(prim_id, he_id);
                }
                
                // Record Point -> HalfEdge
                topo.point_to_halfedge.insert(p_curr, he_id);
                
                // Register for equivalent linking
                // Sort keys to group (u,v) and (v,u)
                let key = if p_curr < p_next { (p_curr, p_next) } else { (p_next, p_curr) };
                edge_map.entry(key).or_default().push(he_id);
        }

            // Link 'next' pointers for this face
            for i in 0..count {
                let curr_id = face_half_edges[i];
                let next_id = face_half_edges[(i + 1) % count];
                
                if let Some(he) = topo.half_edges.get_mut(curr_id.into()) {
                    he.next = next_id;
                }
            }
        }

        // 2. Link Equivalent Rings
        for (_, he_list) in edge_map {
            if he_list.is_empty() { continue; }
            
            let count = he_list.len();
            for i in 0..count {
                let curr_id = he_list[i];
                let next_eq_id = he_list[(i + 1) % count]; // Circular link
                
                if let Some(he) = topo.half_edges.get_mut(curr_id.into()) {
                    he.next_equivalent = next_eq_id;
                }
            }
            
            // Collect boundary edges (self-loop)
            if count == 1 {
                topo.boundary_halfedges.push(he_list[0]);
            }
        }

        topo
    }

    /// Iterates all points known to this topology.
    pub fn iter_points(&self) -> impl Iterator<Item = PointId> + '_ {
        self.point_to_halfedge.keys().copied()
    }
    
    // --- Traversal Helpers ---

    /// Get the next half-edge around the face.
    pub fn next(&self, he: HalfEdgeId) -> HalfEdgeId {
        if !he.is_valid() { return HalfEdgeId::INVALID; }
        self.half_edges.get(he.into())
            .map(|e| e.next)
            .unwrap_or(HalfEdgeId::INVALID)
    }

    /// Get the previous half-edge around the face.
    pub fn prev(&self, he_id: HalfEdgeId) -> HalfEdgeId {
        if !he_id.is_valid() { return HalfEdgeId::INVALID; }
        
        let start = he_id;
        let mut curr = self.next(he_id);
        let mut prev = he_id;
        
        let mut iterations = 0;
        while curr != start && curr.is_valid() && iterations < 100 {
            prev = curr;
            curr = self.next(curr);
            iterations += 1;
        }
        
        if curr == start {
            prev
        } else {
            HalfEdgeId::INVALID
        }
    }

    /// Get the next equivalent half-edge in the ring.
    /// Replaces `pair()`. In a 2-manifold mesh, this returns the opposite edge.
    /// In a boundary, it returns self.
    pub fn next_equivalent(&self, he: HalfEdgeId) -> HalfEdgeId {
        if !he.is_valid() { return HalfEdgeId::INVALID; }
        self.half_edges.get(he.into())
            .map(|e| e.next_equivalent)
            .unwrap_or(HalfEdgeId::INVALID)
    }

    /// Helper for backward compatibility / manifold assumption.
    /// Returns the first equivalent edge that is "opposite" (starts at dest).
    /// If none found (boundary or weird config), returns INVALID (or self? old pair returned INVALID for boundary).
    pub fn pair(&self, he: HalfEdgeId) -> HalfEdgeId {
        if !he.is_valid() { return HalfEdgeId::INVALID; }
        
        let start_node = self.half_edges.get(he.into());
        if start_node.is_none() { return HalfEdgeId::INVALID; }
        let start_dest = self.dest_point(he);

        let mut curr = self.next_equivalent(he);
        let start = he;
        
        // Loop through the ring to find an opposite edge
        let mut iterations = 0;
        while curr != start && curr.is_valid() && iterations < 100 {
            if let Some(curr_node) = self.half_edges.get(curr.into()) {
                // Check if opposite: origin == start_dest
                if curr_node.origin_point == start_dest {
                    return curr;
                }
            }
            curr = self.next_equivalent(curr);
            iterations += 1;
        }
        
        // If not found (e.g. boundary, or only parallel edges), return INVALID to match old pair() behavior for boundaries
        HalfEdgeId::INVALID
    }

    /// Get the destination point of a half-edge.
    pub fn dest_point(&self, he: HalfEdgeId) -> PointId {
        let next_id = self.next(he);
        if next_id.is_valid() {
            self.half_edges.get(next_id.into())
                .map(|e| e.origin_point)
                .unwrap_or(PointId::INVALID)
        } else {
            PointId::INVALID
        }
    }
    
    /// Iterate all half-edges incident to a point (outgoing).
    pub fn iter_spoke_edges(&self, point_id: PointId) -> SpokeIterator {
        let start_he = self.point_to_halfedge.get(&point_id).copied().unwrap_or(HalfEdgeId::INVALID);
        SpokeIterator {
            topo: self,
            start_he,
            current_he: start_he,
            just_started: true,
        }
    }
    
    pub fn get_boundary_edges(&self) -> &[HalfEdgeId] {
        &self.boundary_halfedges
    }
    
    // --- Incremental Update API ---
    
    /// Insert a single half-edge. Returns the new HalfEdgeId.
    /// Call `link_equivalent_edges` after batch insertions to complete topology.
    pub fn insert_half_edge(&mut self, he: HalfEdge) -> HalfEdgeId {
        let idx = self.half_edges.insert(he);
        HalfEdgeId::from(idx)
    }
    
    /// Remove a half-edge by ID. O(1) with memory compaction.
    pub fn remove_half_edge(&mut self, he_id: HalfEdgeId) -> Option<HalfEdge> {
        // Update maps before removal
        if let Some(he) = self.half_edges.get(he_id.into()) {
            let origin = he.origin_point;
            let prim = he.primitive_index;
            
            // Remove from point map if this was the recorded spoke
            if self.point_to_halfedge.get(&origin) == Some(&he_id) {
                // Try to find another spoke, or remove entry
                let alt = self.iter_spoke_edges(origin).find(|&e| e != he_id);
                if let Some(alt_he) = alt { self.point_to_halfedge.insert(origin, alt_he); }
                else { self.point_to_halfedge.remove(&origin); }
            }
            
            // Remove from primitive map if this was the start
            if self.primitive_to_halfedge.get(&prim) == Some(&he_id) {
                self.primitive_to_halfedge.remove(&prim);
            }
            
            // Remove from boundary list if present
            self.boundary_halfedges.retain(|&e| e != he_id);
        }
        
        self.half_edges.remove(he_id.into())
    }
    
    /// 批量移除指定 primitive 的所有 half-edges，O(k) 复杂度
    pub fn remove_primitive(&mut self, prim_id: PrimId) {
        let Some(start) = self.primitive_to_halfedge.remove(&prim_id) else { return; };
        if !start.is_valid() { return; }
        
        // 收集这个面的所有 half-edges
        let mut to_remove = Vec::new();
        let mut curr = start;
        let mut iter = 0;
        loop {
            to_remove.push(curr);
            curr = self.next(curr);
            iter += 1;
            if curr == start || !curr.is_valid() || iter > 100 { break; }
        }
        
        // 批量删除
        for he_id in to_remove { let _ = self.remove_half_edge(he_id); }
    }
    
    /// 增量添加一个 primitive 的拓扑，返回起始 HalfEdgeId
    pub fn insert_primitive(&mut self, prim_id: PrimId, point_ids: &[PointId]) -> Option<HalfEdgeId> {
        if point_ids.len() < 3 { return None; }
        let count = point_ids.len();
        let mut face_hes = Vec::with_capacity(count);
        
        // 创建 half-edges
        for i in 0..count {
            let he = HalfEdge {
                origin_point: point_ids[i],
                next_equivalent: HalfEdgeId::INVALID,
                next: HalfEdgeId::INVALID,
                primitive_index: prim_id,
            };
            let he_id = self.insert_half_edge(he);
            face_hes.push(he_id);
            self.point_to_halfedge.insert(point_ids[i], he_id);
        }
        
        // 链接 next 指针
        for i in 0..count {
            let next_id = face_hes[(i + 1) % count];
            if let Some(he) = self.half_edges.get_mut(face_hes[i].into()) { he.next = next_id; }
        }
        
        let start = face_hes[0];
        self.primitive_to_halfedge.insert(prim_id, start);
        Some(start)
    }
    
    /// 链接等价 half-edges（pair 关系），用于批量插入后建立完整拓扑
    pub fn link_equivalent_edges(&mut self, edge_map: &mut HashMap<(PointId, PointId), Vec<HalfEdgeId>>) {
        for (_, he_list) in edge_map.iter() {
            if he_list.is_empty() { continue; }
            let count = he_list.len();
            for i in 0..count {
                let curr_id = he_list[i];
                let next_eq_id = he_list[(i + 1) % count];
                if let Some(he) = self.half_edges.get_mut(curr_id.into()) { he.next_equivalent = next_eq_id; }
            }
            if count == 1 { self.boundary_halfedges.push(he_list[0]); }
        }
    }
    
    /// 检查是否需要完全重建
    pub fn needs_rebuild(&self) -> bool { false }

    /// Get all neighbor points connected to the given point by an edge.
    pub fn get_point_neighbors(&self, point_id: PointId) -> Vec<PointId> {
        let mut neighbors = Vec::new();
        for he in self.iter_spoke_edges(point_id) {
            let dest = self.dest_point(he);
            if dest.is_valid() {
                neighbors.push(dest);
            }
        }
        neighbors.sort_by_key(|k| (k.index, k.generation));
        neighbors.dedup();
        neighbors
    }

    pub fn get_primitive_neighbors(&self, prim_id: PrimId) -> Vec<PrimId> {
        let start_he = self.primitive_to_halfedge.get(&prim_id).copied().unwrap_or(HalfEdgeId::INVALID);

        if !start_he.is_valid() {
            return Vec::new();
        }

        let mut neighbors = Vec::new();
        let mut curr = start_he;

        // Circulate around the face
        loop {
            // Check all equivalent edges for neighbors
            let mut eq = self.next_equivalent(curr);
            while eq != curr && eq.is_valid() {
                 if let Some(neighbor_he) = self.half_edges.get(eq.into()) {
                     let neighbor_prim = neighbor_he.primitive_index;
                     if neighbor_prim != prim_id {
                    neighbors.push(neighbor_prim);
                }
                }
                eq = self.next_equivalent(eq);
            }
            
            curr = self.next(curr);
            if curr == start_he || !curr.is_valid() {
                break;
            }
        }
        
        neighbors.sort_by_key(|k| (k.index, k.generation));
        neighbors.dedup();
        neighbors
    }
}

pub struct SpokeIterator<'a> {
    topo: &'a Topology,
    start_he: HalfEdgeId,
    current_he: HalfEdgeId,
    just_started: bool,
}

impl<'a> Iterator for SpokeIterator<'a> {
    type Item = HalfEdgeId;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.current_he.is_valid() {
            return None;
        }

        if !self.just_started && self.current_he == self.start_he {
            return None;
        }

        let yield_he = self.current_he;
        self.just_started = false;

        // Move to next spoke:
        // In manifold: pair -> next.
        // In non-manifold: we need to find an "incoming" edge in the ring (dest == origin)
        // and take its next.
        
        // Find 'pair' (an equivalent edge that is opposite)
        let pair_he = self.topo.pair(self.current_he);
        
        if pair_he.is_valid() {
            self.current_he = self.topo.next(pair_he);
        } else {
            // Boundary or weird. 
            // If boundary, maybe we stop? Or try to find another path?
            // For standard boundary iteration:
            self.current_he = HalfEdgeId::INVALID;
        }

        Some(yield_he)
    }
}
