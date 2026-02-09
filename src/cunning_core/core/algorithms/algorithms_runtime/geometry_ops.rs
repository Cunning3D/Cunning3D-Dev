use crate::libs::geometry::ids::{EdgeId, PointId, PrimId, VertexId};
use crate::libs::geometry::mesh::{Geometry, GeoPrimitive};
#[allow(unused_imports)]
use crate::libs::geometry::topology::Topology;

impl Geometry {
    pub fn split_edge(&mut self, p0: PointId, p1: PointId, t: f32) -> PointId { self.structural_edit(|g| g.split_edge_internal(p0, p1, t)) }

    fn split_edge_internal(&mut self, p0: PointId, p1: PointId, t: f32) -> PointId {
        let topo = self.get_topology();
        let mut prims_to_update = Vec::new();

        // Helper to collect all primitives adjacent to the edge (u, v)
        // by finding one half-edge and iterating its equivalent ring.
        let mut collect_prims = |u: PointId, v: PointId| {
            if let Some(he_start) = topo.iter_spoke_edges(u).find(|&he| topo.dest_point(he) == v) {
                let mut curr = he_start;
                loop {
                     if let Some(he) = topo.half_edges.get(curr.into()) {
                        prims_to_update.push(he.primitive_index);
                    }
                    curr = topo.next_equivalent(curr);
                    if curr == he_start || !curr.is_valid() { break; }
                }
            }
        };

        // Collect from both directions (p0->p1 and p1->p0) to catch all faces
        collect_prims(p0, p1);
        collect_prims(p1, p0);

        prims_to_update.sort_by_key(|k| (k.index, k.generation));
        prims_to_update.dedup();

        let new_p = self.add_point_from_mix(
            self.points().get_dense_index(p0.into()).expect("Invalid p0"),
            self.points().get_dense_index(p1.into()).expect("Invalid p1"),
            t,
        );
        for prim_id in prims_to_update {
            if self.primitives().get_dense_index(prim_id.into()).is_none() { continue; }
            
            // Extract vertices if it is a Polygon
            let mut verts = if let Some(GeoPrimitive::Polygon(p)) = self.primitives().get(prim_id.into()) {
                p.vertices.clone()
            } else {
                continue;
            };

            let (count, mut insert_indices) = (verts.len(), Vec::new());
            for i in 0..count {
                let (v_curr_id, v_next_id) = (verts[i], verts[(i + 1) % count]);
                let p_curr = self.vertices().get(v_curr_id.into()).map(|v| v.point_id);
                let p_next = self.vertices().get(v_next_id.into()).map(|v| v.point_id);
                if (p_curr == Some(p0) && p_next == Some(p1)) || (p_curr == Some(p1) && p_next == Some(p0)) {
                    insert_indices.push((i + 1, v_curr_id, v_next_id));
                }
            }
            insert_indices.sort_by(|a, b| b.0.cmp(&a.0));
            for (insert_pos, v_a, v_b) in insert_indices {
                let new_v = self.add_vertex_from_mix(
                    self.vertices().get_dense_index(v_a.into()).unwrap(),
                    self.vertices().get_dense_index(v_b.into()).unwrap(),
                    t,
                    new_p,
                );
                if insert_pos >= verts.len() { verts.push(new_v); } else { verts.insert(insert_pos, new_v); }
            }
            self.set_primitive_vertices_no_invalidate(prim_id, verts);
        }
        new_p
    }

    pub fn collapse_edge(&mut self, keep: PointId, remove: PointId) {
        if keep == remove { return; }
        self.structural_edit(|g| g.collapse_edge_internal_no_invalidate(keep, remove));
    }

    fn collapse_edge_internal_no_invalidate(&mut self, keep: PointId, remove: PointId) {
        let prims_using_remove: Vec<PrimId> = self.primitives().iter_enumerated().filter_map(|(idx, prim)| {
            for &vid in prim.vertices() {
                if let Some(v) = self.vertices().get(vid.into()) { if v.point_id == remove { return Some(PrimId::from(idx)); } }
            }
            None
        }).collect();
        let edges_using_remove: Vec<EdgeId> = self.edges().iter_enumerated().filter_map(|(idx, edge)| {
            if edge.p0 == remove || edge.p1 == remove { Some(EdgeId::from(idx)) } else { None }
        }).collect();
        let vertices_to_update: Vec<VertexId> = self.vertices().iter_enumerated().filter_map(|(idx, v)| {
            if v.point_id == remove { Some(VertexId::from(idx)) } else { None }
        }).collect();
        for vid in vertices_to_update { self.set_vertex_point_no_invalidate(vid, keep); }
        for eid in edges_using_remove {
            if let Some(edge) = self.edges_mut().get_mut(eid.into()) {
                if edge.p0 == remove { edge.p0 = keep; }
                if edge.p1 == remove { edge.p1 = keep; }
            }
        }
        let edges_to_kill: Vec<EdgeId> = self.edges().iter_enumerated().filter_map(|(idx, edge)| {
            if edge.p0 == edge.p1 { Some(EdgeId::from(idx)) } else { None }
        }).collect();
        for e in edges_to_kill { self.remove_edge(e); }
        let (mut prims_to_kill, mut prim_updates) = (Vec::new(), Vec::new()); // (PrimId, Vec<VertexId>)
        for pid in prims_using_remove {
            let Some(prim) = self.primitives().get(pid.into()) else { continue; };
            let vertices = prim.vertices();
            
            let mut v_p_list = Vec::with_capacity(vertices.len());
            for &vid in vertices { if let Some(v) = self.vertices().get(vid.into()) { v_p_list.push((vid, v.point_id)); } }
            if v_p_list.is_empty() { prims_to_kill.push(pid); continue; }
            
            let (mut deduped, mut last_pt) = (Vec::new(), PointId::INVALID);
            for (vid, pid2) in v_p_list { if pid2 != last_pt { deduped.push(vid); last_pt = pid2; } }
            
            let is_polygon = matches!(prim, GeoPrimitive::Polygon(_));
            
            if is_polygon && deduped.len() > 1 {
                let (first_vid, last_vid) = (deduped[0], deduped[deduped.len() - 1]);
                if self.vertices().get(first_vid.into()).unwrap().point_id == self.vertices().get(last_vid.into()).unwrap().point_id { deduped.pop(); }
            }
            let min_count = if is_polygon { 3 } else { 2 };
            if deduped.len() < min_count { prims_to_kill.push(pid); } else { prim_updates.push((pid, deduped)); }
        }
        for (pid, new_verts) in prim_updates { self.set_primitive_vertices_no_invalidate(pid, new_verts); }
        for pid in prims_to_kill { self.remove_primitive(pid); }
        self.remove_point(remove);
    }
}
