use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::ids::PointId;
use crate::mesh::{GeoPrimitive, Geometry, PolylinePrim, VertexId};
use crate::nodes::parameter::Parameter;
use crate::register_node;
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct ConvertLineNode;

impl NodeParameters for ConvertLineNode {
    fn define_parameters() -> Vec<Parameter> {
        Vec::new()
    }
}

impl NodeOp for ConvertLineNode {
    fn compute(&self, _params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let Some(input) = inputs.first().map(|g| g.materialize()) else {
            return Arc::new(Geometry::new());
        };

        let mut out = input.fork();
        reset_topology_keep_points(&mut out);

        // Avoid duplicate edges when converting polygon meshes (adjacent faces share edges).
        let mut poly_edge_seen: HashSet<(PointId, PointId)> = HashSet::new();

        for prim in input.primitives().iter() {
            match prim {
                GeoPrimitive::Polyline(line) => {
                    let point_ids = point_ids_from_vertices(&input, &line.vertices);
                    add_open_polyline_segments(&mut out, &point_ids, line.closed);
                }
                GeoPrimitive::Polygon(poly) => {
                    // Convert polygon face boundaries to line segments, and do NOT keep polygons,
                    // otherwise the renderer will treat the result as a triangle mesh.
                    let point_ids = point_ids_from_vertices(&input, &poly.vertices);
                    add_polygon_edges_unique(&mut out, &point_ids, &mut poly_edge_seen);
                }
                GeoPrimitive::BezierCurve(curve) => {
                    // Treat curve control points as a polyline (for display / downstream poly tools).
                    let point_ids = point_ids_from_vertices(&input, &curve.vertices);
                    add_open_polyline_segments(&mut out, &point_ids, curve.closed);
                }
            }
        }

        Arc::new(out)
    }
}

#[inline]
fn reset_topology_keep_points(geo: &mut Geometry) {
    geo.vertices_mut().clear();
    geo.primitives_mut().clear();
    geo.edges_mut().clear();

    geo.vertex_attributes.clear();
    geo.primitive_attributes.clear();
    geo.edge_attributes.clear();

    geo.vertex_groups.clear();
    geo.primitive_groups.clear();
    if let Some(groups) = &mut geo.edge_groups {
        groups.clear();
    }
}

#[inline]
fn add_segment_primitive(
    geo: &mut Geometry,
    p0: PointId,
    p1: PointId,
) {
    let v0 = geo.add_vertex(p0);
    let v1 = geo.add_vertex(p1);
    geo.add_primitive(GeoPrimitive::Polyline(PolylinePrim {
        vertices: vec![v0, v1],
        closed: false,
    }));
}

#[inline]
fn add_open_polyline_segments(geo: &mut Geometry, point_ids: &[PointId], closed: bool) {
    if point_ids.len() < 2 {
        return;
    }
    for i in 0..point_ids.len() - 1 {
        let a = point_ids[i];
        let b = point_ids[i + 1];
        if a != b {
            add_segment_primitive(geo, a, b);
        }
    }
    if closed && point_ids.len() > 2 {
        let a = point_ids[point_ids.len() - 1];
        let b = point_ids[0];
        if a != b {
            add_segment_primitive(geo, a, b);
        }
    }
}

#[inline]
fn add_polygon_edges_unique(
    geo: &mut Geometry,
    point_ids: &[PointId],
    seen: &mut HashSet<(PointId, PointId)>,
) {
    let n = point_ids.len();
    if n < 2 {
        return;
    }
    for i in 0..n {
        let a = point_ids[i];
        let b = point_ids[(i + 1) % n];
        if a == b {
            continue;
        }
        let key = if a < b { (a, b) } else { (b, a) };
        if seen.insert(key) {
            add_segment_primitive(geo, a, b);
        }
    }
}

#[inline]
fn point_ids_from_vertices(geo: &Geometry, src_vertices: &[VertexId]) -> Vec<PointId> {
    let mut out = Vec::with_capacity(src_vertices.len());
    for vid in src_vertices {
        let Some(v) = geo.vertices().get((*vid).into()) else {
            continue;
        };
        out.push(v.point_id);
    }
    out
}

register_node!("ConvertLine", "Modeling", ConvertLineNode);
