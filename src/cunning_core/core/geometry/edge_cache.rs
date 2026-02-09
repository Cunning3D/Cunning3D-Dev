use std::collections::HashMap;
use rayon::prelude::*;
use crate::libs::geometry::mesh::{Geometry, Attribute, GeoPrimitive};
use crate::libs::geometry::ids::{AttributeId, PointId, VertexId, HalfEdgeId};
use crate::libs::geometry::topology::Topology;

#[derive(Debug, Clone)]
pub struct EdgeCache {
    /// List of unique edges, sorted by (p0, p1) where p0 < p1
    pub edges: Vec<(PointId, PointId)>,
    /// Map from sorted edge (p0, p1) to index in `edges`
    pub edge_map: HashMap<(PointId, PointId), u32>,
    /// Edge Attributes. Valid only as long as this cache is valid.
    pub attributes: HashMap<AttributeId, Attribute>,
}

impl EdgeCache {
    pub fn new() -> Self {
        Self {
            edges: Vec::new(),
            edge_map: HashMap::new(),
            attributes: HashMap::new(),
        }
    }

    /// Build from Topology.
    /// Supports non-manifold topology by extracting unique edges from all half-edges.
    pub fn from_topology(topo: &Topology) -> Self {
        // 1. Collect (p0, next_id) from all half-edges.
        // We do this in a separate pass to avoid borrowing topo.half_edges twice at once.
        let edge_defs: Vec<(PointId, HalfEdgeId)> = topo.half_edges
            .iter()
            .map(|he| (he.origin_point, he.next))
            .collect();

        // 2. Resolve p1 and normalize
        let mut edges: Vec<(PointId, PointId)> = edge_defs.into_iter().filter_map(|(p0, next_id)| {
             if let Some(next_he) = topo.half_edges.get(next_id.into()) {
                 let p1 = next_he.origin_point;
                 if p0 == p1 { return None; } // degenerate
                 let k0 = (p0.index, p0.generation);
                 let k1 = (p1.index, p1.generation);
                 Some(if k0 <= k1 { (p0, p1) } else { (p1, p0) })
             } else {
                 None
             }
        }).collect();
        
        // 3. Sort
        edges.par_sort_unstable_by(|a, b| {
            (a.0.index, a.0.generation)
                .cmp(&(b.0.index, b.0.generation))
                .then_with(|| (a.1.index, a.1.generation).cmp(&(b.1.index, b.1.generation)))
        });
        
        // 4. Dedup
        edges.dedup();

        let mut edge_map = HashMap::with_capacity(edges.len());
        for (i, edge) in edges.iter().enumerate() {
            edge_map.insert(*edge, i as u32);
        }

        Self {
            edges,
            edge_map,
            attributes: HashMap::new(),
        }
    }

    /// Build the EdgeCache from the geometry's primitives.
    /// This extracts all implicit edges, sorts and deduplicates them.
    pub fn build(geo: &Geometry) -> Self {
        let primitives_arena = geo.primitives();
        let vertices_arena = geo.vertices();

        // 1. Parallel extract raw edges from all primitives
        let raw_edges: Vec<(PointId, PointId)> = primitives_arena
            .par_iter()
            .flat_map(|prim| {
                let (vertices, closed) = match prim {
                    GeoPrimitive::Polygon(p) => (&p.vertices, true),
                    GeoPrimitive::Polyline(p) => (&p.vertices, p.closed),
                    GeoPrimitive::BezierCurve(p) => (&p.vertices, p.closed),
                };

                let count = vertices.len();
                if count < 2 {
                    return Vec::new();
                }

                let mut local_edges = Vec::with_capacity(count);
                
                // Helper to get PointId from VertexId
                let get_pid = |vid: VertexId| -> Option<PointId> {
                    vertices_arena.get(vid.into()).map(|v| v.point_id)
                };

                let limit = if closed { count } else { count - 1 };

                for i in 0..limit {
                    let v0 = vertices[i];
                    let v1 = vertices[(i + 1) % count]; // Cyclic if closed

                    if let (Some(p0), Some(p1)) = (get_pid(v0.into()), get_pid(v1.into())) {
                        if p0 != p1 {
                            // Normalize edge: smaller index first
                            let k0 = (p0.index, p0.generation);
                            let k1 = (p1.index, p1.generation);
                            local_edges.push(if k0 <= k1 { (p0, p1) } else { (p1, p0) });
                        }
                    }
                }
                local_edges
            })
            .collect();

        // 2. Parallel Sort
        let mut edges = raw_edges;
        edges.par_sort_unstable_by(|a, b| {
            (a.0.index, a.0.generation)
                .cmp(&(b.0.index, b.0.generation))
                .then_with(|| (a.1.index, a.1.generation).cmp(&(b.1.index, b.1.generation)))
        });

        // 3. Deduplicate (Sequential but fast on sorted)
        edges.dedup();

        // 4. Build Map (Optional, for O(1) lookup)
        let mut edge_map = HashMap::with_capacity(edges.len());
        for (i, edge) in edges.iter().enumerate() {
            edge_map.insert(*edge, i as u32);
        }

        Self {
            edges,
            edge_map,
            attributes: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.edges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }
}
