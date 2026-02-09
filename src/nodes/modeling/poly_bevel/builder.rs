use super::structures::*;
use crate::libs::geometry::ids::{EdgeId, HalfEdgeId, PointId};
use crate::libs::geometry::mesh::Geometry;
use crate::libs::geometry::topology::Topology;
use bevy::prelude::*;
use std::collections::HashMap;

pub struct BevelBuilder<'a> {
    geo: &'a Geometry,
    topo: &'a Topology,
    graph: BevelGraph,
    he_to_idx: HashMap<HalfEdgeId, usize>,
    pt_to_idx: HashMap<PointId, usize>,
}

impl<'a> BevelBuilder<'a> {
    pub fn new(geo: &'a Geometry, topo: &'a Topology) -> Self {
        // Pre-allocate capacity optimization
        let est_pts = geo.points().len();
        let est_edges = topo.half_edges.len();
        Self {
            geo,
            topo,
            graph: BevelGraph::with_capacity(est_pts, est_edges),
            he_to_idx: HashMap::with_capacity(est_edges),
            pt_to_idx: HashMap::with_capacity(est_pts),
        }
    }

    pub fn build(
        mut self,
        edge_selection: &[bool],
        divisions: usize,
        offset: f32,
        point_weights: Option<&[f32]>,
        edge_weights: Option<&HashMap<(PointId, PointId), f32>>,
    ) -> BevelGraph {
        let Some(_positions) = self.geo.get_point_position_attribute() else {
            return self.graph;
        };

        for (pid_idx, _) in self.geo.points().iter_enumerated() {
            let p_id = PointId::from(pid_idx);
            let mut is_involved = false;
            for he in self.topo.iter_spoke_edges(p_id) {
                if let Some(di) = self.topo.half_edges.get_dense_index(he.into()) {
                    if di < edge_selection.len() && edge_selection[di] {
                        is_involved = true;
                        break;
                    }
                }
            }

            if is_involved {
                let w = point_weights
                    .and_then(|ws| {
                        self.geo
                            .points()
                            .get_dense_index(pid_idx)
                            .and_then(|di| ws.get(di))
                    })
                    .copied()
                    .unwrap_or(1.0f32)
                    .clamp(0.0f32, 1.0f32);
                self.create_bev_vert(p_id, divisions, offset * w, edge_selection, edge_weights);
            }
        }

        let num_edges = self.graph.edges.len();
        for i in 0..num_edges {
            let he_id = self.graph.edges[i].he_id;
            let pair_he = self.topo.pair(he_id);

            if let Some(pair_idx) = self.he_to_idx.get(&pair_he) {
                self.graph.edges[i].pair_index = *pair_idx;
            } else {
                self.graph.edges[i].pair_index = usize::MAX;
            }
        }

        // Sync SoA arrays for cache-friendly parallel access
        self.graph.sync_soa();
        self.graph
    }

    fn create_bev_vert(
        &mut self,
        p_id: PointId,
        divisions: usize,
        offset: f32,
        edge_selection: &[bool],
        edge_weights: Option<&HashMap<(PointId, PointId), f32>>,
    ) {
        let v_idx = self.graph.verts.len();
        self.pt_to_idx.insert(p_id, v_idx);

        let mut edges_indices = Vec::new();

        for he_id in self.topo.iter_spoke_edges(p_id) {
            let is_selected = self
                .topo
                .half_edges
                .get_dense_index(he_id.into())
                .map(|di| di < edge_selection.len() && edge_selection[di])
                .unwrap_or(false);
            let ew = edge_weights
                .and_then(|m| {
                    let q = self.topo.dest_point(he_id);
                    if !q.is_valid() {
                        return None;
                    }
                    let key = if p_id < q { (p_id, q) } else { (q, p_id) };
                    m.get(&key).copied()
                })
                .unwrap_or(1.0f32)
                .clamp(0.0f32, 1.0f32);
            let off = offset * ew;

            let eh_idx = self.graph.edges.len();
            edges_indices.push(eh_idx);
            self.he_to_idx.insert(he_id, eh_idx);

            let eh = EdgeHalf {
                e_id: EdgeId::INVALID,
                he_id,
                pair_index: usize::MAX,
                next_index: usize::MAX,
                prev_index: usize::MAX,
                origin_bev_vert: v_idx,

                offset_l: if is_selected { off } else { 0.0 },
                offset_r: if is_selected { off } else { 0.0 },
                seg: if is_selected { divisions } else { 0 },
                is_bev: is_selected,
                is_seam: false,

                left_v: None,
                right_v: None,
            };
            self.graph.edges.push(eh);
        }

        let n = edges_indices.len();
        if n > 0 {
            for i in 0..n {
                let curr = edges_indices[i];
                let next = edges_indices[(i + 1) % n];
                let prev = edges_indices[(i + n - 1) % n];

                self.graph.edges[curr].next_index = next;
                self.graph.edges[curr].prev_index = prev;
            }
        }

        let bv = BevVert {
            p_id,
            edge_count: n,
            edges: edges_indices,
            vmesh: None,
            offset: 0.0,
        };
        self.graph.verts.push(bv);
    }
}
