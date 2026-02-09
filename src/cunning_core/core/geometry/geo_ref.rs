//! GeometryRef: read-only geometry abstraction to enable zero-copy views (ForEach, out-of-core).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;
use crate::libs::algorithms::algorithms_dcc::PagedBuffer;
use crate::libs::geometry::group::ElementGroupMask;
use crate::libs::geometry::ids::{AttributeDomain, AttributeId, PointId, PrimId, VertexId};
use crate::libs::geometry::mesh::{Attribute, Geometry, GeoPrimitive};

pub type GeoIn = Arc<dyn GeometryRef>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimKind { Polygon, Polyline, BezierCurve }

#[derive(Clone, Debug, Default)]
pub struct ForEachMeta { pub iteration: i32, pub numiterations: i32, pub value: String, pub ivalue: i32 }

pub enum AttrView<'a, T: Clone + Send + Sync + 'static> {
    Slice(&'a [T]),
    Paged(&'a PagedBuffer<T>),
    MapSlice { base: &'a [T], map: &'a [usize] },
    MapPaged { base: &'a PagedBuffer<T>, map: &'a [usize] },
}

impl<'a, T: Clone + Send + Sync + 'static> AttrView<'a, T> {
    #[inline] pub fn get(&self, i: usize) -> Option<T> {
        match self {
            Self::Slice(s) => s.get(i).cloned(),
            Self::Paged(p) => p.get(i),
            Self::MapSlice { base, map } => base.get(*map.get(i)?).cloned(),
            Self::MapPaged { base, map } => base.get(*map.get(i)?),
        }
    }
    #[inline] pub fn as_slice(&self) -> Option<&'a [T]> { if let Self::Slice(s) = self { Some(s) } else { None } }
}

pub trait GeometryRef: Send + Sync {
    fn point_len(&self) -> usize;
    fn vertex_len(&self) -> usize;
    fn prim_len(&self) -> usize;
    fn edge_len(&self) -> usize;

    fn iter_points_dense<'a>(&'a self) -> Box<dyn Iterator<Item = usize> + 'a>;
    fn iter_vertices_dense<'a>(&'a self) -> Box<dyn Iterator<Item = usize> + 'a>;
    fn iter_prims_dense<'a>(&'a self) -> Box<dyn Iterator<Item = usize> + 'a>;

    fn prim_kind(&self, prim_dense: usize) -> Option<PrimKind>;
    fn prim_vertices(&self, prim_dense: usize) -> Option<&[VertexId]>;
    fn vertex_point(&self, vid: VertexId) -> Option<PointId>;

    fn point_dense(&self, pid: PointId) -> Option<usize>;
    fn vertex_dense(&self, vid: VertexId) -> Option<usize>;
    fn prim_dense(&self, pid: PrimId) -> Option<usize>;

    fn point_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, name: &str) -> Option<AttrView<'a, T>> where Self: Sized;
    fn vertex_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, name: &str) -> Option<AttrView<'a, T>> where Self: Sized;
    fn prim_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, name: &str) -> Option<AttrView<'a, T>> where Self: Sized;
    fn edge_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, name: &str) -> Option<AttrView<'a, T>> where Self: Sized;
    fn detail_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, name: &str) -> Option<AttrView<'a, T>> where Self: Sized;

    fn foreach_meta(&self) -> Option<&ForEachMeta> { None }
    fn materialize(&self) -> Geometry { Geometry::new() }
}

#[inline]
fn attr_view<'a, T: Clone + Send + Sync + 'static>(a: &'a Attribute) -> Option<AttrView<'a, T>> {
    a.as_slice::<T>().map(AttrView::Slice).or_else(|| a.as_paged::<T>().map(AttrView::Paged))
}

impl GeometryRef for Geometry {
    #[inline] fn point_len(&self) -> usize { self.points().len() }
    #[inline] fn vertex_len(&self) -> usize { self.vertices().len() }
    #[inline] fn prim_len(&self) -> usize { self.primitives().len() }
    #[inline] fn edge_len(&self) -> usize { self.edges().len() }

    fn iter_points_dense<'a>(&'a self) -> Box<dyn Iterator<Item = usize> + 'a> { Box::new(0..self.points().len()) }
    fn iter_vertices_dense<'a>(&'a self) -> Box<dyn Iterator<Item = usize> + 'a> { Box::new(0..self.vertices().len()) }
    fn iter_prims_dense<'a>(&'a self) -> Box<dyn Iterator<Item = usize> + 'a> { Box::new(0..self.primitives().len()) }

    fn prim_kind(&self, prim_dense: usize) -> Option<PrimKind> {
        let id = self.primitives().get_id_from_dense(prim_dense)?;
        match self.primitives().get(id) {
            Some(GeoPrimitive::Polygon(_)) => Some(PrimKind::Polygon),
            Some(GeoPrimitive::Polyline(_)) => Some(PrimKind::Polyline),
            Some(GeoPrimitive::BezierCurve(_)) => Some(PrimKind::BezierCurve),
            None => None,
        }
    }

    fn prim_vertices(&self, prim_dense: usize) -> Option<&[VertexId]> {
        let id = self.primitives().get_id_from_dense(prim_dense)?;
        self.primitives().get(id).map(|p| p.vertices())
    }

    fn vertex_point(&self, vid: VertexId) -> Option<PointId> { self.vertices().get(vid.into()).map(|v| v.point_id) }
    fn point_dense(&self, pid: PointId) -> Option<usize> { self.points().get_dense_index(pid.into()) }
    fn vertex_dense(&self, vid: VertexId) -> Option<usize> { self.vertices().get_dense_index(vid.into()) }
    fn prim_dense(&self, pid: PrimId) -> Option<usize> { self.primitives().get_dense_index(pid.into()) }

    fn point_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, name: &str) -> Option<AttrView<'a, T>> { attr_view(self.get_point_attribute(name)?) }
    fn vertex_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, name: &str) -> Option<AttrView<'a, T>> { attr_view(self.get_vertex_attribute(name)?) }
    fn prim_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, name: &str) -> Option<AttrView<'a, T>> { attr_view(self.get_primitive_attribute(name)?) }
    fn edge_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, name: &str) -> Option<AttrView<'a, T>> { attr_view(self.get_edge_attribute(name)?) }
    fn detail_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, name: &str) -> Option<AttrView<'a, T>> { attr_view(self.get_detail_attribute(name)?) }
    fn materialize(&self) -> Geometry { self.clone() }
}

#[derive(Clone)]
pub struct GeometryView {
    pub base: Arc<Geometry>,
    pub prim_l2b: Vec<usize>,
    pub point_l2b: Vec<usize>,
    pub vertex_l2b: Vec<usize>,
    pub prim_b2l: HashMap<usize, usize>,
    pub point_b2l: HashMap<usize, usize>,
    pub vertex_b2l: HashMap<usize, usize>,
    pub meta: Option<ForEachMeta>,
    pub detail_overrides: HashMap<AttributeId, Attribute>,
    pub materialized: Arc<OnceLock<Arc<Geometry>>>,
}

impl GeometryView {
    pub fn from_masks(base: Arc<Geometry>, prim_mask: Option<&ElementGroupMask>, point_mask: Option<&ElementGroupMask>, meta: Option<ForEachMeta>) -> Self {
        let prim_l2b: Vec<usize> = match prim_mask { Some(m) => m.iter_ones().collect(), None => (0..base.primitives().len()).collect() };
        let point_l2b: Vec<usize> = match point_mask { Some(m) => m.iter_ones().collect(), None => (0..base.points().len()).collect() };
        let mut vertex_l2b_set = std::collections::HashSet::<usize>::new();
        if prim_mask.is_some() {
            for &pb in &prim_l2b {
                if let Some(vids) = GeometryRef::prim_vertices(&*base, pb) {
                    for &vid in vids { if let Some(vb) = base.vertices().get_dense_index(vid.into()) { vertex_l2b_set.insert(vb); } }
                }
            }
        } else {
            vertex_l2b_set.extend(0..base.vertices().len());
        }
        let mut vertex_l2b: Vec<usize> = vertex_l2b_set.into_iter().collect();
        vertex_l2b.sort_unstable();

        let prim_b2l = prim_l2b.iter().enumerate().map(|(l, &b)| (b, l)).collect();
        let point_b2l = point_l2b.iter().enumerate().map(|(l, &b)| (b, l)).collect();
        let vertex_b2l = vertex_l2b.iter().enumerate().map(|(l, &b)| (b, l)).collect();

        Self { base, prim_l2b, point_l2b, vertex_l2b, prim_b2l, point_b2l, vertex_b2l, meta, detail_overrides: HashMap::new(), materialized: Arc::new(OnceLock::new()) }
    }

    #[inline]
    pub fn materialize_arc(&self) -> Arc<Geometry> {
        self.materialized.get_or_init(|| Arc::new(materialize_view(self))).clone()
    }

    #[inline] fn map_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, domain: AttributeDomain, name: &str) -> Option<AttrView<'a, T>> {
        let map: &'a [usize] = match domain {
            AttributeDomain::Point => &self.point_l2b,
            AttributeDomain::Vertex => &self.vertex_l2b,
            AttributeDomain::Primitive => &self.prim_l2b,
            AttributeDomain::Edge => return None, // edges view not exposed yet
            AttributeDomain::Detail => return None,
        };
        let a = match domain {
            AttributeDomain::Point => self.base.get_point_attribute(name)?,
            AttributeDomain::Vertex => self.base.get_vertex_attribute(name)?,
            AttributeDomain::Primitive => self.base.get_primitive_attribute(name)?,
            AttributeDomain::Edge => self.base.get_edge_attribute(name)?,
            AttributeDomain::Detail => self.base.get_detail_attribute(name)?,
        };
        a.as_slice::<T>().map(|s| AttrView::MapSlice { base: s, map }).or_else(|| a.as_paged::<T>().map(|p| AttrView::MapPaged { base: p, map }))
    }
}

impl GeometryRef for GeometryView {
    fn point_len(&self) -> usize { self.point_l2b.len() }
    fn vertex_len(&self) -> usize { self.vertex_l2b.len() }
    fn prim_len(&self) -> usize { self.prim_l2b.len() }
    fn edge_len(&self) -> usize { self.base.edges().len() }

    fn iter_points_dense<'a>(&'a self) -> Box<dyn Iterator<Item = usize> + 'a> { Box::new(0..self.point_l2b.len()) }
    fn iter_vertices_dense<'a>(&'a self) -> Box<dyn Iterator<Item = usize> + 'a> { Box::new(0..self.vertex_l2b.len()) }
    fn iter_prims_dense<'a>(&'a self) -> Box<dyn Iterator<Item = usize> + 'a> { Box::new(0..self.prim_l2b.len()) }

    fn prim_kind(&self, prim_dense: usize) -> Option<PrimKind> { self.base.prim_kind(*self.prim_l2b.get(prim_dense)?) }
    fn prim_vertices(&self, prim_dense: usize) -> Option<&[VertexId]> { self.base.prim_vertices(*self.prim_l2b.get(prim_dense)?) }
    fn vertex_point(&self, vid: VertexId) -> Option<PointId> { self.base.vertex_point(vid) }

    fn point_dense(&self, pid: PointId) -> Option<usize> { self.base.point_dense(pid).and_then(|b| self.point_b2l.get(&b).copied()) }
    fn vertex_dense(&self, vid: VertexId) -> Option<usize> { self.base.vertex_dense(vid).and_then(|b| self.vertex_b2l.get(&b).copied()) }
    fn prim_dense(&self, pid: PrimId) -> Option<usize> { self.base.prim_dense(pid).and_then(|b| self.prim_b2l.get(&b).copied()) }

    fn point_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, name: &str) -> Option<AttrView<'a, T>> { self.map_attr(AttributeDomain::Point, name) }
    fn vertex_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, name: &str) -> Option<AttrView<'a, T>> { self.map_attr(AttributeDomain::Vertex, name) }
    fn prim_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, name: &str) -> Option<AttrView<'a, T>> { self.map_attr(AttributeDomain::Primitive, name) }
    fn edge_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, _name: &str) -> Option<AttrView<'a, T>> { None }
    fn detail_attr<'a, T: Clone + Send + Sync + 'static>(&'a self, name: &str) -> Option<AttrView<'a, T>> {
        if let Some(a) = self.detail_overrides.get(&AttributeId::from(name)) { return attr_view(a); }
        attr_view(self.base.get_detail_attribute(name)?)
    }

    fn foreach_meta(&self) -> Option<&ForEachMeta> { self.meta.as_ref() }
    fn materialize(&self) -> Geometry { (*self.materialize_arc()).clone() }
}

fn materialize_view(v: &GeometryView) -> Geometry {
    let b = &v.base;
    let mut out = Geometry::new();
    let mut pmap: HashMap<PointId, PointId> = HashMap::new();
    let mut vmap: HashMap<VertexId, VertexId> = HashMap::new();

    for &pb in &v.point_l2b {
        let Some(bpid) = b.points().get_id_from_dense(pb).map(PointId::from) else { continue; };
        pmap.insert(bpid, out.add_point());
    }
    // Ensure points referenced by selected vertices exist even if point_l2b was constrained.
    for &vb in &v.vertex_l2b {
        let Some(bvid) = b.vertices().get_id_from_dense(vb).map(VertexId::from) else { continue; };
        let Some(bp) = b.vertices().get(bvid.into()).map(|vv| vv.point_id) else { continue; };
        pmap.entry(bp).or_insert_with(|| out.add_point());
    }
    for &vb in &v.vertex_l2b {
        let Some(bvid) = b.vertices().get_id_from_dense(vb).map(VertexId::from) else { continue; };
        let Some(bp) = b.vertices().get(bvid.into()).map(|vv| vv.point_id) else { continue; };
        let Some(np) = pmap.get(&bp).copied() else { continue; };
        vmap.insert(bvid, out.add_vertex(np));
    }
    for &pb in &v.prim_l2b {
        let Some(bpid) = b.primitives().get_id_from_dense(pb).map(PrimId::from) else { continue; };
        let Some(p) = b.primitives().get(bpid.into()) else { continue; };
        let nv: Vec<VertexId> = p.vertices().iter().filter_map(|&vid| vmap.get(&vid).copied()).collect();
        if nv.len() < 2 { continue; }
        let np = match p {
            GeoPrimitive::Polygon(_) => GeoPrimitive::Polygon(crate::libs::geometry::mesh::PolygonPrim { vertices: nv }),
            GeoPrimitive::Polyline(pl) => GeoPrimitive::Polyline(crate::libs::geometry::mesh::PolylinePrim { vertices: nv, closed: pl.closed }),
            GeoPrimitive::BezierCurve(pl) => GeoPrimitive::BezierCurve(crate::libs::geometry::mesh::BezierCurvePrim { vertices: nv, closed: pl.closed }),
        };
        out.add_primitive(np);
    }

    // Copy known-typed attributes (robust: skip unknown types).
    copy_attrs_point(b, &mut out, &v.point_l2b);
    copy_attrs_vertex(b, &mut out, &v.vertex_l2b);
    copy_attrs_prim(b, &mut out, &v.prim_l2b);
    copy_attrs_detail(b, &mut out);
    if let Some(m) = v.foreach_meta() {
        out.insert_detail_attribute("iteration", Attribute::new(vec![m.iteration]));
        out.insert_detail_attribute("numiterations", Attribute::new(vec![m.numiterations]));
        out.insert_detail_attribute("value", Attribute::new(vec![m.value.clone()]));
        out.insert_detail_attribute("ivalue", Attribute::new(vec![m.ivalue]));
    }
    for (k, a) in &v.detail_overrides { out.insert_detail_attribute(*k, a.clone()); }
    out
}

fn copy_attrs_point(b: &Geometry, out: &mut Geometry, map: &[usize]) { copy_attrs_domain(b, out, AttributeDomain::Point, map); }
fn copy_attrs_vertex(b: &Geometry, out: &mut Geometry, map: &[usize]) { copy_attrs_domain(b, out, AttributeDomain::Vertex, map); }
fn copy_attrs_prim(b: &Geometry, out: &mut Geometry, map: &[usize]) { copy_attrs_domain(b, out, AttributeDomain::Primitive, map); }
fn copy_attrs_detail(b: &Geometry, out: &mut Geometry) { copy_attrs_domain(b, out, AttributeDomain::Detail, &[0]); }

fn copy_attrs_domain(b: &Geometry, out: &mut Geometry, d: AttributeDomain, map: &[usize]) {
    let src = match d {
        AttributeDomain::Point => &b.point_attributes,
        AttributeDomain::Vertex => &b.vertex_attributes,
        AttributeDomain::Primitive => &b.primitive_attributes,
        AttributeDomain::Edge => &b.edge_attributes,
        AttributeDomain::Detail => &b.detail_attributes,
    };
    for (id, h) in src {
        let a = &*h.data;
        if let Some(v) = copy_vec::<f32>(a, map) { insert_attr(out, d, *id, Attribute::new_auto(v)); continue; }
        if let Some(v) = copy_vec::<bevy::prelude::Vec2>(a, map) { insert_attr(out, d, *id, Attribute::new_auto(v)); continue; }
        if let Some(v) = copy_vec::<bevy::prelude::Vec3>(a, map) { insert_attr(out, d, *id, Attribute::new_auto(v)); continue; }
        if let Some(v) = copy_vec::<bevy::prelude::Vec4>(a, map) { insert_attr(out, d, *id, Attribute::new_auto(v)); continue; }
        if let Some(v) = copy_vec::<i32>(a, map) { insert_attr(out, d, *id, Attribute::new_auto(v)); continue; }
        if let Some(v) = copy_vec::<bool>(a, map) { insert_attr(out, d, *id, Attribute::new_auto(v)); continue; }
        if let Some(v) = copy_vec::<String>(a, map) { insert_attr(out, d, *id, Attribute::new_auto(v)); continue; }
        if let Some(v) = copy_bytes(a, map) { insert_attr(out, d, *id, Attribute::new(crate::libs::geometry::mesh::Bytes(v))); continue; }
    }
}

fn insert_attr(out: &mut Geometry, d: AttributeDomain, id: AttributeId, a: Attribute) {
    match d {
        AttributeDomain::Point => out.insert_point_attribute(id, a),
        AttributeDomain::Vertex => out.insert_vertex_attribute(id, a),
        AttributeDomain::Primitive => out.insert_primitive_attribute(id, a),
        AttributeDomain::Edge => out.insert_edge_attribute(id, a),
        AttributeDomain::Detail => out.insert_detail_attribute(id, a),
    }
}

fn copy_vec<T: Clone + Default + PartialEq + Send + Sync + 'static + std::fmt::Debug>(a: &Attribute, map: &[usize]) -> Option<Vec<T>> {
    a.as_slice::<T>().map(|s| map.iter().map(|&i| s.get(i).cloned().unwrap_or_default()).collect())
        .or_else(|| a.as_paged::<T>().map(|p| map.iter().map(|&i| p.get(i).unwrap_or_default()).collect()))
}

fn copy_bytes(a: &Attribute, map: &[usize]) -> Option<Vec<u8>> {
    a.as_storage::<crate::libs::geometry::mesh::Bytes>().map(|b| map.iter().map(|&i| *b.0.get(i).unwrap_or(&0)).collect())
}

