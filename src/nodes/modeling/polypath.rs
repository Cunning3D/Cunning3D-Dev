use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::ids::PointId;
use crate::mesh::{Attribute, BezierCurvePrim, GeoPrimitive, Geometry, PolygonPrim, PolylinePrim, VertexId};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::prelude::Vec3;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct PolyPathNode;

#[derive(Clone, Copy)]
struct LineEdge {
    a: PointId,
    b: PointId,
}

impl LineEdge {
    #[inline]
    fn other(self, p: PointId) -> Option<PointId> {
        if self.a == p {
            Some(self.b)
        } else if self.b == p {
            Some(self.a)
        } else {
            None
        }
    }
}

impl NodeParameters for PolyPathNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "close",
                "Close",
                "PolyPath",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "distance",
                "Distance",
                "PolyPath",
                ParameterValue::Float(0.001),
                ParameterUIType::FloatSlider { min: 0.0, max: 10.0 },
            ),
            Parameter::new(
                "make_face",
                "Make Face",
                "PolyPath",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for PolyPathNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let Some(input) = inputs.first().map(|g| g.materialize()) else {
            return Arc::new(Geometry::new());
        };

        let close = get_bool(params, "close", false);
        let close_distance = get_float(params, "distance", 0.001).max(0.0);
        let make_face = get_bool(params, "make_face", false);

        let mut out = input.fork();
        reset_topology_keep_points(&mut out);

        let mut edges: Vec<LineEdge> = Vec::new();
        for prim in input.primitives().iter() {
            match prim {
                GeoPrimitive::Polyline(line) => {
                    let point_ids = point_ids_from_vertices(&input, &line.vertices);
                    if point_ids.len() < 2 {
                        continue;
                    }
                    for i in 0..point_ids.len() - 1 {
                        let a = point_ids[i];
                        let b = point_ids[i + 1];
                        if a != b {
                            edges.push(LineEdge { a, b });
                        }
                    }
                    if line.closed && point_ids.len() > 2 {
                        let a = point_ids[point_ids.len() - 1];
                        let b = point_ids[0];
                        if a != b {
                            edges.push(LineEdge { a, b });
                        }
                    }
                }
                GeoPrimitive::Polygon(poly) => {
                    let verts = remap_vertices_to_points(&input, &mut out, &poly.vertices);
                    if verts.len() >= 3 {
                        out.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: verts }));
                    }
                }
                GeoPrimitive::BezierCurve(curve) => {
                    let verts = remap_vertices_to_points(&input, &mut out, &curve.vertices);
                    if verts.len() >= 2 {
                        out.add_primitive(GeoPrimitive::BezierCurve(BezierCurvePrim {
                            vertices: verts,
                            closed: curve.closed,
                        }));
                    }
                }
            }
        }

        if edges.is_empty() {
            return Arc::new(out);
        }

        let mut adjacency: HashMap<PointId, Vec<usize>> = HashMap::new();
        for (ei, e) in edges.iter().enumerate() {
            adjacency.entry(e.a).or_default().push(ei);
            adjacency.entry(e.b).or_default().push(ei);
        }
        for inc in adjacency.values_mut() {
            inc.sort_unstable();
        }

        let mut point_order: HashMap<PointId, usize> = HashMap::new();
        for (i, (pid, _)) in input.points().iter_enumerated().enumerate() {
            point_order.insert(PointId::from(pid), i);
        }

        let mut starts: Vec<PointId> = adjacency
            .iter()
            .filter_map(|(pid, inc)| (inc.len() != 2).then_some(*pid))
            .collect();
        starts.sort_by_key(|pid| point_order.get(pid).copied().unwrap_or(usize::MAX));

        let mut used = vec![false; edges.len()];
        let mut paths: Vec<Vec<PointId>> = Vec::new();

        for start in starts {
            while let Some(first_edge) = first_unused_edge_at(start, &adjacency, &used) {
                let path = walk_from(start, first_edge, &edges, &adjacency, &mut used);
                if path.len() >= 2 {
                    paths.push(path);
                }
            }
        }
        for ei in 0..edges.len() {
            if used[ei] {
                continue;
            }
            let start = edges[ei].a;
            let path = walk_from(start, ei, &edges, &adjacency, &mut used);
            if path.len() >= 2 {
                paths.push(path);
            }
        }

        let p_attr = input.get_point_attribute(attrs::P);
        for mut path in paths {
            let mut closed_from_graph = false;
            if path.len() >= 2 && path.first() == path.last() {
                closed_from_graph = true;
                path.pop();
            }
            if path.len() < 2 {
                continue;
            }

            let mut closed = closed_from_graph;
            if close && !closed && path.len() >= 3 {
                if let Some(dist) = endpoint_distance(&input, p_attr, path[0], path[path.len() - 1]) {
                    if dist <= close_distance {
                        closed = true;
                    }
                }
            }

            let mut verts = Vec::with_capacity(path.len());
            for pid in &path {
                verts.push(out.add_vertex(*pid));
            }

            if make_face && closed && verts.len() >= 3 {
                out.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: verts }));
            } else {
                out.add_primitive(GeoPrimitive::Polyline(PolylinePrim {
                    vertices: verts,
                    closed,
                }));
            }
        }

        Arc::new(out)
    }
}

#[inline]
fn get_bool(params: &[Parameter], name: &str, default: bool) -> bool {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match p.value {
            ParameterValue::Bool(v) => Some(v),
            _ => None,
        })
        .unwrap_or(default)
}

#[inline]
fn get_float(params: &[Parameter], name: &str, default: f32) -> f32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match p.value {
            ParameterValue::Float(v) => Some(v),
            _ => None,
        })
        .unwrap_or(default)
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
fn remap_vertices_to_points(
    input: &Geometry,
    out: &mut Geometry,
    src_vertices: &[VertexId],
) -> Vec<VertexId> {
    let mut out_vertices = Vec::with_capacity(src_vertices.len());
    for vid in src_vertices {
        let Some(v) = input.vertices().get((*vid).into()) else {
            continue;
        };
        out_vertices.push(out.add_vertex(v.point_id));
    }
    out_vertices
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

#[inline]
fn first_unused_edge_at(
    point: PointId,
    adjacency: &HashMap<PointId, Vec<usize>>,
    used: &[bool],
) -> Option<usize> {
    let edges = adjacency.get(&point)?;
    edges.iter().copied().find(|&ei| !used[ei])
}

#[inline]
fn choose_next_unused_edge(
    point: PointId,
    prev_edge: Option<usize>,
    adjacency: &HashMap<PointId, Vec<usize>>,
    used: &[bool],
) -> Option<usize> {
    let edges = adjacency.get(&point)?;
    let mut fallback = None;
    for &ei in edges {
        if used[ei] {
            continue;
        }
        if Some(ei) != prev_edge {
            return Some(ei);
        }
        fallback = Some(ei);
    }
    fallback
}

fn walk_from(
    start: PointId,
    first_edge: usize,
    edges: &[LineEdge],
    adjacency: &HashMap<PointId, Vec<usize>>,
    used: &mut [bool],
) -> Vec<PointId> {
    let mut path = Vec::new();
    path.push(start);

    let mut curr = start;
    let mut prev_edge = None;
    let mut edge = Some(first_edge);
    while let Some(ei) = edge {
        if used[ei] {
            break;
        }
        used[ei] = true;
        let Some(next) = edges[ei].other(curr) else {
            break;
        };
        path.push(next);
        prev_edge = Some(ei);
        curr = next;
        edge = choose_next_unused_edge(curr, prev_edge, adjacency, used);
        if curr == start {
            break;
        }
    }

    path
}

#[inline]
fn sample_vec3_attr(attr: Option<&Attribute>, index: usize) -> Option<Vec3> {
    let a = attr?;
    if let Some(s) = a.as_slice::<Vec3>() {
        return s.get(index).copied();
    }
    if let Some(pb) = a.as_paged::<Vec3>() {
        return pb.get(index);
    }
    None
}

#[inline]
fn point_position(geo: &Geometry, p_attr: Option<&Attribute>, pid: PointId) -> Option<Vec3> {
    let dense = geo.points().get_dense_index(pid.into())?;
    sample_vec3_attr(p_attr, dense)
}

#[inline]
fn endpoint_distance(
    geo: &Geometry,
    p_attr: Option<&Attribute>,
    a: PointId,
    b: PointId,
) -> Option<f32> {
    let pa = point_position(geo, p_attr, a)?;
    let pb = point_position(geo, p_attr, b)?;
    Some((pa - pb).length())
}

register_node!("PolyPath", "Modeling", PolyPathNode);
