pub mod c_api;
mod handle_arena;
pub mod rust_build;
pub mod build_jobs;

pub use build_jobs::{CompileRustPluginRequest, PluginBuildJobsPlugin};
pub use build_jobs::request_compile_rust_plugin;

use crate::cunning_core::registries::node_registry::{NodeRegistry, RuntimeNodeDescriptor};
use crate::cunning_core::traits::node_interface::NodeOp;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::Geometry;
use crate::nodes::parameter::Parameter;
use crate::nodes::parameter::{ParameterUIType, ParameterValue};
use bevy::prelude::*;
use bevy::prelude::{Vec2, Vec3, Vec4};
use bevy::tasks::{IoTaskPool, Task};
use handle_arena::GeoArena;
use libloading::{Library, Symbol};
use futures_lite::future;
use std::ffi::CStr;
use std::os::raw::c_void;
use std::path::Path;
use std::sync::{Arc, RwLock};

// ----------------------------------------------------------------------------
// Native Node Wrapper
// Acts as a bridge between Rust NodeOp trait and C++ VTable
// ----------------------------------------------------------------------------

struct NativeNodeWrapper {
    // We hold an Arc to the Library to ensure the DLL is not unloaded
    // while this node exists.
    _library: Arc<Library>,
    vtable: c_api::CNodeVTable,
    instance: *mut std::ffi::c_void,
}

unsafe impl Send for NativeNodeWrapper {}
unsafe impl Sync for NativeNodeWrapper {}

impl NodeOp for NativeNodeWrapper {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mut arena = GeoArena::default();
        let in_handles: Vec<c_api::GeoHandle> = inputs
            .iter()
            .map(|g| arena.insert_read(Arc::new(g.materialize())))
            .collect();
        let mut out = arena.insert_write(Geometry::new());

        enum Scratch {
            I32(Vec<i32>),
            F32(Vec<f32>),
            V2(Vec<Vec2>),
            V3(Vec<Vec3>),
            V4(Vec<Vec4>),
            Str(Vec<String>),
            #[allow(dead_code)]
            Curve(Vec<c_api::CCurveControlPoint>, Box<c_api::CCurveData>),
        }
        let mut scratch: Vec<Scratch> = Vec::new();

        let mut pv: Vec<c_api::CParamValue> = Vec::with_capacity(params.len());
        for p in params {
            pv.push(match &p.value {
                ParameterValue::Int(v) => c_api::CParamValue {
                    tag: c_api::CParamTag::Int,
                    _pad0: [0; 3],
                    a: *v as i64 as u64,
                    b: 0,
                },
                ParameterValue::Float(v) => c_api::CParamValue {
                    tag: c_api::CParamTag::Float,
                    _pad0: [0; 3],
                    a: v.to_bits() as u64,
                    b: 0,
                },
                ParameterValue::Bool(v) => c_api::CParamValue {
                    tag: c_api::CParamTag::Bool,
                    _pad0: [0; 3],
                    a: if *v { 1 } else { 0 },
                    b: 0,
                },
                ParameterValue::Vec2(v) => c_api::CParamValue {
                    tag: c_api::CParamTag::Vec2,
                    _pad0: [0; 3],
                    a: (v.x.to_bits() as u64) | ((v.y.to_bits() as u64) << 32),
                    b: 0,
                },
                ParameterValue::Vec3(v) => c_api::CParamValue {
                    tag: c_api::CParamTag::Vec3,
                    _pad0: [0; 3],
                    a: (v.x.to_bits() as u64) | ((v.y.to_bits() as u64) << 32),
                    b: v.z.to_bits() as u64,
                },
                ParameterValue::Vec4(v) => c_api::CParamValue {
                    tag: c_api::CParamTag::Vec4,
                    _pad0: [0; 3],
                    a: (v.x.to_bits() as u64) | ((v.y.to_bits() as u64) << 32),
                    b: (v.z.to_bits() as u64) | ((v.w.to_bits() as u64) << 32),
                },
                ParameterValue::String(s) => c_api::CParamValue {
                    tag: c_api::CParamTag::String,
                    _pad0: [0; 3],
                    a: s.as_ptr() as usize as u64,
                    b: s.len() as u64,
                },
                ParameterValue::Color(v) => c_api::CParamValue {
                    tag: c_api::CParamTag::Color3,
                    _pad0: [0; 3],
                    a: (v.x.to_bits() as u64) | ((v.y.to_bits() as u64) << 32),
                    b: v.z.to_bits() as u64,
                },
                ParameterValue::Color4(v) => c_api::CParamValue {
                    tag: c_api::CParamTag::Color4,
                    _pad0: [0; 3],
                    a: (v.x.to_bits() as u64) | ((v.y.to_bits() as u64) << 32),
                    b: (v.z.to_bits() as u64) | ((v.w.to_bits() as u64) << 32),
                },
                ParameterValue::Curve(c) => {
                    let mut pts: Vec<c_api::CCurveControlPoint> =
                        Vec::with_capacity(c.points.len());
                    for pt in &c.points {
                        pts.push(c_api::CCurveControlPoint {
                            id: to_cuuid(pt.id),
                            position: [pt.position.x, pt.position.y, pt.position.z],
                            mode: match pt.mode {
                                crate::nodes::parameter::PointMode::Corner => {
                                    c_api::CPointMode::Corner
                                }
                                crate::nodes::parameter::PointMode::Bezier => {
                                    c_api::CPointMode::Bezier
                                }
                            },
                            _pad0: [0; 2],
                            handle_in: [pt.handle_in.x, pt.handle_in.y, pt.handle_in.z],
                            _pad1: 0,
                            handle_out: [pt.handle_out.x, pt.handle_out.y, pt.handle_out.z],
                            _pad2: 0,
                            weight: pt.weight,
                            _pad3: [0; 3],
                        });
                    }
                    let data = Box::new(c_api::CCurveData {
                        pts: pts.as_ptr(),
                        len: pts.len() as u32,
                        closed: if c.is_closed { 1 } else { 0 },
                        curve_type: match c.curve_type {
                            crate::nodes::parameter::CurveType::Polygon => {
                                c_api::CCurveType::Polygon as u32
                            }
                            crate::nodes::parameter::CurveType::Bezier => {
                                c_api::CCurveType::Bezier as u32
                            }
                            crate::nodes::parameter::CurveType::Nurbs => {
                                c_api::CCurveType::Nurbs as u32
                            }
                        },
                    });
                    let ptr = data.as_ref() as *const c_api::CCurveData as usize as u64;
                    scratch.push(Scratch::Curve(pts, data));
                    c_api::CParamValue {
                        tag: c_api::CParamTag::Curve,
                        _pad0: [0; 3],
                        a: ptr,
                        b: 0,
                    }
                }
                _ => c_api::CParamValue {
                    tag: c_api::CParamTag::Int,
                    _pad0: [0; 3],
                    a: 0,
                    b: 0,
                },
            });
        }
        struct HostState {
            arena: GeoArena,
            scratch: Vec<Scratch>,
        }

        #[inline]
        fn sv_to_string(s: c_api::CStringView) -> Option<String> {
            if s.ptr.is_null() {
                return None;
            }
            let b = unsafe { std::slice::from_raw_parts(s.ptr as *const u8, s.len as usize) };
            std::str::from_utf8(b).ok().map(|v| v.to_string())
        }
        #[inline]
        fn attr_id(s: c_api::CStringView) -> Option<crate::libs::geometry::ids::AttributeId> {
            Some(crate::libs::geometry::ids::AttributeId::from(sv_to_string(
                s,
            )?))
        }

        extern "C" fn geo_create(u: *mut c_void) -> c_api::GeoHandle {
            unsafe {
                (&mut *(u as *mut HostState))
                    .arena
                    .insert_write(Geometry::new())
            }
        }
        extern "C" fn geo_clone(u: *mut c_void, src: c_api::GeoHandle) -> c_api::GeoHandle {
            unsafe {
                (&mut *(u as *mut HostState))
                    .arena
                    .clone_to_write(src)
                    .unwrap_or(0)
            }
        }
        extern "C" fn geo_drop(u: *mut c_void, h: c_api::GeoHandle) {
            unsafe {
                (&mut *(u as *mut HostState)).arena.take_write(h);
            }
        }
        extern "C" fn geo_point_count(u: *mut c_void, h: c_api::GeoHandle) -> u32 {
            unsafe { (&*(u as *mut HostState)).arena.point_count_any(h) }
        }
        extern "C" fn geo_vertex_count(u: *mut c_void, h: c_api::GeoHandle) -> u32 {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let g0;
                let g: &Geometry = if let Some(a) = st.arena.get_read(h) {
                    a.as_ref()
                } else if let Some(w) = st.arena.get_write(h) {
                    g0 = w as *const Geometry;
                    &*g0
                } else {
                    return 0;
                };
                g.vertices().len() as u32
            }
        }
        extern "C" fn geo_prim_count(u: *mut c_void, h: c_api::GeoHandle) -> u32 {
            unsafe { (&*(u as *mut HostState)).arena.prim_count_any(h) }
        }
        extern "C" fn geo_edge_count(u: *mut c_void, h: c_api::GeoHandle) -> u32 {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let g0;
                let g: &Geometry = if let Some(a) = st.arena.get_read(h) {
                    a.as_ref()
                } else if let Some(w) = st.arena.get_write(h) {
                    g0 = w as *const Geometry;
                    &*g0
                } else {
                    return 0;
                };
                g.edges().len() as u32
            }
        }
        extern "C" fn geo_add_point(u: *mut c_void, h: c_api::GeoHandle) -> u32 {
            unsafe {
                (&mut *(u as *mut HostState))
                    .arena
                    .get_write(h)
                    .map(|g| g.add_point().index)
                    .unwrap_or(0)
            }
        }
        extern "C" fn geo_add_vertex(u: *mut c_void, h: c_api::GeoHandle, point_dense: u32) -> u32 {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let Some(g) = st.arena.get_write(h) else {
                    return 0;
                };
                let Some(pid) = g
                    .points()
                    .get_id_from_dense(point_dense as usize)
                    .map(crate::libs::geometry::ids::PointId::from)
                else {
                    return 0;
                };
                g.add_vertex(pid).index
            }
        }
        extern "C" fn geo_remove_point(u: *mut c_void, h: c_api::GeoHandle, point_dense: u32) {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let Some(g) = st.arena.get_write(h) else {
                    return;
                };
                let Some(pid) = g
                    .points()
                    .get_id_from_dense(point_dense as usize)
                    .map(crate::libs::geometry::ids::PointId::from)
                else {
                    return;
                };
                g.remove_point(pid);
            }
        }
        extern "C" fn geo_remove_vertex(u: *mut c_void, h: c_api::GeoHandle, vtx_dense: u32) {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let Some(g) = st.arena.get_write(h) else {
                    return;
                };
                let Some(vid) = g
                    .vertices()
                    .get_id_from_dense(vtx_dense as usize)
                    .map(crate::libs::geometry::ids::VertexId::from)
                else {
                    return;
                };
                g.remove_vertex(vid);
            }
        }
        extern "C" fn geo_add_edge(
            u: *mut c_void,
            h: c_api::GeoHandle,
            p0_dense: u32,
            p1_dense: u32,
        ) -> u32 {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let Some(g) = st.arena.get_write(h) else {
                    return 0;
                };
                let Some(p0) = g
                    .points()
                    .get_id_from_dense(p0_dense as usize)
                    .map(crate::libs::geometry::ids::PointId::from)
                else {
                    return 0;
                };
                let Some(p1) = g
                    .points()
                    .get_id_from_dense(p1_dense as usize)
                    .map(crate::libs::geometry::ids::PointId::from)
                else {
                    return 0;
                };
                g.add_edge(p0, p1).index
            }
        }
        extern "C" fn geo_remove_edge(u: *mut c_void, h: c_api::GeoHandle, edge_dense: u32) {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let Some(g) = st.arena.get_write(h) else {
                    return;
                };
                let Some(eid) = g
                    .edges()
                    .get_id_from_dense(edge_dense as usize)
                    .map(crate::libs::geometry::ids::EdgeId::from)
                else {
                    return;
                };
                g.remove_edge(eid);
            }
        }
        extern "C" fn geo_add_polygon(
            u: *mut c_void,
            h: c_api::GeoHandle,
            point_dense: *const u32,
            point_len: u32,
        ) -> u32 {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let Some(g) = st.arena.get_write(h) else {
                    return 0;
                };
                let pts = if point_dense.is_null() || point_len < 3 {
                    return 0;
                } else {
                    std::slice::from_raw_parts(point_dense, point_len as usize)
                };
                let mut vids: Vec<crate::libs::geometry::ids::VertexId> =
                    Vec::with_capacity(pts.len());
                for &pd in pts {
                    let Some(pid) = g
                        .points()
                        .get_id_from_dense(pd as usize)
                        .map(crate::libs::geometry::ids::PointId::from)
                    else {
                        return 0;
                    };
                    vids.push(g.add_vertex(pid));
                }
                let prim = g.add_primitive(crate::libs::geometry::mesh::GeoPrimitive::Polygon(
                    crate::libs::geometry::mesh::PolygonPrim { vertices: vids },
                ));
                prim.index
            }
        }
        extern "C" fn geo_add_polyline(
            u: *mut c_void,
            h: c_api::GeoHandle,
            point_dense: *const u32,
            point_len: u32,
            closed: u32,
        ) -> u32 {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let Some(g) = st.arena.get_write(h) else {
                    return 0;
                };
                let pts = if point_dense.is_null() || point_len < 2 {
                    return 0;
                } else {
                    std::slice::from_raw_parts(point_dense, point_len as usize)
                };
                let mut vids: Vec<crate::libs::geometry::ids::VertexId> =
                    Vec::with_capacity(pts.len() + 1);
                for &pd in pts {
                    let Some(pid) = g
                        .points()
                        .get_id_from_dense(pd as usize)
                        .map(crate::libs::geometry::ids::PointId::from)
                    else {
                        return 0;
                    };
                    vids.push(g.add_vertex(pid));
                }
                let closed = closed != 0;
                if closed && vids.len() >= 2 {
                    vids.push(vids[0]);
                }
                let prim = g.add_primitive(crate::libs::geometry::mesh::GeoPrimitive::Polyline(
                    crate::libs::geometry::mesh::PolylinePrim {
                        vertices: vids,
                        closed,
                    },
                ));
                prim.index
            }
        }
        extern "C" fn geo_set_prim_vertices(
            u: *mut c_void,
            h: c_api::GeoHandle,
            prim_dense: u32,
            vtx_dense: *const u32,
            vtx_len: u32,
        ) {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let Some(g) = st.arena.get_write(h) else {
                    return;
                };
                let Some(pid) = g
                    .primitives()
                    .get_id_from_dense(prim_dense as usize)
                    .map(crate::libs::geometry::ids::PrimId::from)
                else {
                    return;
                };
                let vs = std::slice::from_raw_parts(vtx_dense, vtx_len as usize);
                let mut out: Vec<crate::libs::geometry::ids::VertexId> =
                    Vec::with_capacity(vs.len());
                for &vdi in vs {
                    if let Some(vid) = g
                        .vertices()
                        .get_id_from_dense(vdi as usize)
                        .map(crate::libs::geometry::ids::VertexId::from)
                    {
                        out.push(vid);
                    }
                }
                g.set_primitive_vertices(pid, out);
            }
        }
        extern "C" fn geo_remove_prim(u: *mut c_void, h: c_api::GeoHandle, prim_dense: u32) {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let Some(g) = st.arena.get_write(h) else {
                    return;
                };
                let Some(pid) = g
                    .primitives()
                    .get_id_from_dense(prim_dense as usize)
                    .map(crate::libs::geometry::ids::PrimId::from)
                else {
                    return;
                };
                g.remove_primitive(pid);
            }
        }
        extern "C" fn geo_vertex_point(u: *mut c_void, h: c_api::GeoHandle, vtx_dense: u32) -> u32 {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let g0;
                let g: &Geometry = if let Some(a) = st.arena.get_read(h) {
                    a.as_ref()
                } else if let Some(w) = st.arena.get_write(h) {
                    g0 = w as *const Geometry;
                    &*g0
                } else {
                    return 0;
                };
                let Some(vid) = g
                    .vertices()
                    .get_id_from_dense(vtx_dense as usize)
                    .map(crate::libs::geometry::ids::VertexId::from)
                else {
                    return 0;
                };
                g.vertices()
                    .get(vid.into())
                    .map(|v| g.points().get_dense_index(v.point_id.into()).unwrap_or(0) as u32)
                    .unwrap_or(0)
            }
        }
        extern "C" fn geo_edge_points(
            u: *mut c_void,
            h: c_api::GeoHandle,
            edge_dense: u32,
            out_p0p1: *mut u32,
        ) -> u32 {
            unsafe {
                if out_p0p1.is_null() {
                    return 0;
                }
                let st = &mut *(u as *mut HostState);
                let g0;
                let g: &Geometry = if let Some(a) = st.arena.get_read(h) {
                    a.as_ref()
                } else if let Some(w) = st.arena.get_write(h) {
                    g0 = w as *const Geometry;
                    &*g0
                } else {
                    return 0;
                };
                let Some(eid) = g
                    .edges()
                    .get_id_from_dense(edge_dense as usize)
                    .map(crate::libs::geometry::ids::EdgeId::from)
                else {
                    return 0;
                };
                let Some(e) = g.edges().get(eid.into()) else {
                    return 0;
                };
                let p0 = g.points().get_dense_index(e.p0.into()).unwrap_or(0) as u32;
                let p1 = g.points().get_dense_index(e.p1.into()).unwrap_or(0) as u32;
                let out = std::slice::from_raw_parts_mut(out_p0p1, 2);
                out[0] = p0;
                out[1] = p1;
                1
            }
        }
        extern "C" fn geo_prim_point_count(
            u: *mut c_void,
            h: c_api::GeoHandle,
            prim_dense: u32,
        ) -> u32 {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let g0;
                let g: &Geometry = if let Some(a) = st.arena.get_read(h) {
                    a.as_ref()
                } else if let Some(w) = st.arena.get_write(h) {
                    g0 = w as *const Geometry;
                    &*g0
                } else {
                    return 0;
                };
                let Some(pid) = g
                    .primitives()
                    .get_id_from_dense(prim_dense as usize)
                    .map(crate::libs::geometry::ids::PrimId::from)
                else {
                    return 0;
                };
                g.primitives()
                    .get(pid.into())
                    .map(|p| p.vertices().len() as u32)
                    .unwrap_or(0)
            }
        }
        extern "C" fn geo_prim_points(
            u: *mut c_void,
            h: c_api::GeoHandle,
            prim_dense: u32,
            out_points: *mut u32,
            out_cap: u32,
        ) -> u32 {
            unsafe {
                if out_points.is_null() || out_cap == 0 {
                    return 0;
                }
                let st = &mut *(u as *mut HostState);
                let g0;
                let g: &Geometry = if let Some(a) = st.arena.get_read(h) {
                    a.as_ref()
                } else if let Some(w) = st.arena.get_write(h) {
                    g0 = w as *const Geometry;
                    &*g0
                } else {
                    return 0;
                };
                let Some(pid) = g
                    .primitives()
                    .get_id_from_dense(prim_dense as usize)
                    .map(crate::libs::geometry::ids::PrimId::from)
                else {
                    return 0;
                };
                let Some(p) = g.primitives().get(pid.into()) else {
                    return 0;
                };
                let out = std::slice::from_raw_parts_mut(out_points, out_cap as usize);
                let mut n = 0usize;
                for &vid in p.vertices().iter() {
                    if n >= out.len() {
                        break;
                    }
                    let Some(v) = g.vertices().get(vid.into()) else {
                        break;
                    };
                    let pd = g.points().get_dense_index(v.point_id.into()).unwrap_or(0) as u32;
                    out[n] = pd;
                    n += 1;
                }
                n as u32
            }
        }
        extern "C" fn geo_get_point_position(
            u: *mut c_void,
            h: c_api::GeoHandle,
            point_dense: u32,
            out_xyz: *mut f32,
        ) -> u32 {
            unsafe {
                if out_xyz.is_null() {
                    return 0;
                }
                let st = &mut *(u as *mut HostState);
                let g0;
                let g: &Geometry = if let Some(a) = st.arena.get_read(h) {
                    a.as_ref()
                } else if let Some(w) = st.arena.get_write(h) {
                    g0 = w as *const Geometry;
                    &*g0
                } else {
                    return 0;
                };
                let p = g
                    .get_point_attribute(crate::libs::geometry::attrs::P)
                    .and_then(|a| a.as_vec3())
                    .and_then(|s| s.get(point_dense as usize))
                    .copied()
                    .unwrap_or(Vec3::ZERO);
                let out = std::slice::from_raw_parts_mut(out_xyz, 3);
                out[0] = p.x;
                out[1] = p.y;
                out[2] = p.z;
                1
            }
        }
        extern "C" fn geo_set_point_position(
            u: *mut c_void,
            h: c_api::GeoHandle,
            point_dense: u32,
            xyz: *const f32,
        ) -> u32 {
            unsafe {
                if xyz.is_null() {
                    return 0;
                }
                let st = &mut *(u as *mut HostState);
                let Some(g) = st.arena.get_write(h) else {
                    return 0;
                };
                let xyz = std::slice::from_raw_parts(xyz, 3);
                let dlen = g.points().len().max((point_dense + 1) as usize);
                if g.get_point_attribute_mut(crate::libs::geometry::attrs::P)
                    .is_none()
                {
                    g.insert_point_attribute(
                        crate::libs::geometry::attrs::P,
                        crate::mesh::Attribute::new_auto(vec![Vec3::ZERO; dlen]),
                    );
                }
                let Some(a) = g.get_point_attribute_mut(crate::libs::geometry::attrs::P) else {
                    return 0;
                };
                if a.is_paged() {
                    if let Some(v) = a.to_vec::<Vec3>().map(crate::mesh::Attribute::new) {
                        *a = v;
                    }
                }
                while a.len() < dlen {
                    a.push_default();
                }
                let Some(s) = a.as_vec3_mut() else {
                    return 0;
                };
                if (point_dense as usize) >= s.len() {
                    return 0;
                }
                s[point_dense as usize] = Vec3::new(xyz[0], xyz[1], xyz[2]);
                1
            }
        }
        extern "C" fn attr_ensure(
            u: *mut c_void,
            h: c_api::GeoHandle,
            domain: c_api::CAttrDomain,
            ty: c_api::CAttrType,
            name: c_api::CStringView,
            len: u32,
        ) -> c_api::CGeoSlice {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let Some(g) = st.arena.get_write(h) else {
                    return c_api::CGeoSlice {
                        ptr: std::ptr::null_mut(),
                        len: 0,
                        stride: 0,
                    };
                };
                let Some(aid) = attr_id(name) else {
                    return c_api::CGeoSlice {
                        ptr: std::ptr::null_mut(),
                        len: 0,
                        stride: 0,
                    };
                };
                let dlen = match domain {
                    c_api::CAttrDomain::Point => g.points().len().max(len as usize),
                    c_api::CAttrDomain::Vertex => g.vertices().len().max(len as usize),
                    c_api::CAttrDomain::Primitive => g.primitives().len().max(len as usize),
                    c_api::CAttrDomain::Edge => g.edges().len().max(len as usize),
                    c_api::CAttrDomain::Detail => 1usize.max(len as usize),
                };
                let stride = match ty {
                    c_api::CAttrType::I32 => 4,
                    c_api::CAttrType::F32 => 4,
                    c_api::CAttrType::Vec2 => core::mem::size_of::<Vec2>() as u32,
                    c_api::CAttrType::Vec3 => core::mem::size_of::<Vec3>() as u32,
                    c_api::CAttrType::Vec4 => core::mem::size_of::<Vec4>() as u32,
                    _ => 0,
                };
                if stride == 0 {
                    return c_api::CGeoSlice {
                        ptr: std::ptr::null_mut(),
                        len: 0,
                        stride: 0,
                    };
                }

                let mut ensure = |get: fn(
                    &mut Geometry,
                    crate::libs::geometry::ids::AttributeId,
                )
                    -> Option<&mut crate::mesh::Attribute>,
                                  ins: fn(
                    &mut Geometry,
                    crate::libs::geometry::ids::AttributeId,
                    crate::mesh::Attribute,
                )| {
                    if get(g, aid).is_none() {
                        let a = match ty {
                            c_api::CAttrType::I32 => {
                                crate::mesh::Attribute::new_auto(vec![0i32; dlen])
                            }
                            c_api::CAttrType::F32 => {
                                crate::mesh::Attribute::new_auto(vec![0.0f32; dlen])
                            }
                            c_api::CAttrType::Vec2 => {
                                crate::mesh::Attribute::new_auto(vec![Vec2::ZERO; dlen])
                            }
                            c_api::CAttrType::Vec3 => {
                                crate::mesh::Attribute::new_auto(vec![Vec3::ZERO; dlen])
                            }
                            c_api::CAttrType::Vec4 => {
                                crate::mesh::Attribute::new_auto(vec![Vec4::ZERO; dlen])
                            }
                            _ => crate::mesh::Attribute::new_auto(vec![0.0f32; dlen]),
                        };
                        ins(g, aid, a);
                    }
                    if let Some(a) = get(g, aid) {
                        if a.is_paged() {
                            if let Some(v) = match ty {
                                c_api::CAttrType::I32 => {
                                    a.to_vec::<i32>().map(|v| crate::mesh::Attribute::new(v))
                                }
                                c_api::CAttrType::F32 => {
                                    a.to_vec::<f32>().map(|v| crate::mesh::Attribute::new(v))
                                }
                                c_api::CAttrType::Vec2 => {
                                    a.to_vec::<Vec2>().map(|v| crate::mesh::Attribute::new(v))
                                }
                                c_api::CAttrType::Vec3 => {
                                    a.to_vec::<Vec3>().map(|v| crate::mesh::Attribute::new(v))
                                }
                                c_api::CAttrType::Vec4 => {
                                    a.to_vec::<Vec4>().map(|v| crate::mesh::Attribute::new(v))
                                }
                                _ => None,
                            } {
                                *a = v;
                            }
                        }
                        while a.len() < dlen {
                            a.push_default();
                        }
                    }
                };

                match domain {
                    c_api::CAttrDomain::Point => ensure(
                        |g, n| g.get_point_attribute_mut(n),
                        |g, n, a| g.insert_point_attribute(n, a),
                    ),
                    c_api::CAttrDomain::Vertex => ensure(
                        |g, n| g.get_vertex_attribute_mut(n),
                        |g, n, a| g.insert_vertex_attribute(n, a),
                    ),
                    c_api::CAttrDomain::Primitive => ensure(
                        |g, n| g.get_primitive_attribute_mut(n),
                        |g, n, a| g.insert_primitive_attribute(n, a),
                    ),
                    c_api::CAttrDomain::Edge => ensure(
                        |g, n| g.get_edge_attribute_mut(n),
                        |g, n, a| g.insert_edge_attribute(n, a),
                    ),
                    c_api::CAttrDomain::Detail => {
                        let nm = aid.as_str();
                        if g.get_detail_attribute_mut(nm).is_none() {
                            let a = match ty {
                                c_api::CAttrType::I32 => {
                                    crate::mesh::Attribute::new_auto(vec![0i32; dlen])
                                }
                                c_api::CAttrType::F32 => {
                                    crate::mesh::Attribute::new_auto(vec![0.0f32; dlen])
                                }
                                c_api::CAttrType::Vec2 => {
                                    crate::mesh::Attribute::new_auto(vec![Vec2::ZERO; dlen])
                                }
                                c_api::CAttrType::Vec3 => {
                                    crate::mesh::Attribute::new_auto(vec![Vec3::ZERO; dlen])
                                }
                                c_api::CAttrType::Vec4 => {
                                    crate::mesh::Attribute::new_auto(vec![Vec4::ZERO; dlen])
                                }
                                _ => crate::mesh::Attribute::new_auto(vec![0.0f32; dlen]),
                            };
                            g.insert_detail_attribute(aid, a);
                        }
                        if let Some(a) = g.get_detail_attribute_mut(nm) {
                            if a.is_paged() {
                                if let Some(v) = match ty {
                                    c_api::CAttrType::I32 => {
                                        a.to_vec::<i32>().map(|v| crate::mesh::Attribute::new(v))
                                    }
                                    c_api::CAttrType::F32 => {
                                        a.to_vec::<f32>().map(|v| crate::mesh::Attribute::new(v))
                                    }
                                    c_api::CAttrType::Vec2 => {
                                        a.to_vec::<Vec2>().map(|v| crate::mesh::Attribute::new(v))
                                    }
                                    c_api::CAttrType::Vec3 => {
                                        a.to_vec::<Vec3>().map(|v| crate::mesh::Attribute::new(v))
                                    }
                                    c_api::CAttrType::Vec4 => {
                                        a.to_vec::<Vec4>().map(|v| crate::mesh::Attribute::new(v))
                                    }
                                    _ => None,
                                } {
                                    *a = v;
                                }
                            }
                            while a.len() < dlen {
                                a.push_default();
                            }
                        }
                    }
                };

                let ptr = match (domain, ty) {
                    (c_api::CAttrDomain::Point, c_api::CAttrType::Vec3) => g
                        .get_point_attribute_mut(aid)
                        .and_then(|a| a.as_vec3_mut())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Point, c_api::CAttrType::F32) => g
                        .get_point_attribute_mut(aid)
                        .and_then(|a| a.as_f32_mut())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Point, c_api::CAttrType::I32) => g
                        .get_point_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<i32>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Point, c_api::CAttrType::Vec2) => g
                        .get_point_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<Vec2>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Point, c_api::CAttrType::Vec4) => g
                        .get_point_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<Vec4>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),

                    (c_api::CAttrDomain::Vertex, c_api::CAttrType::I32) => g
                        .get_vertex_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<i32>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Vertex, c_api::CAttrType::F32) => g
                        .get_vertex_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<f32>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Vertex, c_api::CAttrType::Vec2) => g
                        .get_vertex_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<Vec2>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Vertex, c_api::CAttrType::Vec3) => g
                        .get_vertex_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<Vec3>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Vertex, c_api::CAttrType::Vec4) => g
                        .get_vertex_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<Vec4>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),

                    (c_api::CAttrDomain::Primitive, c_api::CAttrType::I32) => g
                        .get_primitive_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<i32>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Primitive, c_api::CAttrType::F32) => g
                        .get_primitive_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<f32>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Primitive, c_api::CAttrType::Vec2) => g
                        .get_primitive_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<Vec2>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Primitive, c_api::CAttrType::Vec3) => g
                        .get_primitive_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<Vec3>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Primitive, c_api::CAttrType::Vec4) => g
                        .get_primitive_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<Vec4>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),

                    (c_api::CAttrDomain::Edge, c_api::CAttrType::I32) => g
                        .get_edge_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<i32>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Edge, c_api::CAttrType::F32) => g
                        .get_edge_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<f32>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Edge, c_api::CAttrType::Vec2) => g
                        .get_edge_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<Vec2>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Edge, c_api::CAttrType::Vec3) => g
                        .get_edge_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<Vec3>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Edge, c_api::CAttrType::Vec4) => g
                        .get_edge_attribute_mut(aid)
                        .and_then(|a| a.as_mut_slice::<Vec4>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),

                    (c_api::CAttrDomain::Detail, c_api::CAttrType::I32) => g
                        .get_detail_attribute_mut(aid.as_str())
                        .and_then(|a| a.as_mut_slice::<i32>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Detail, c_api::CAttrType::F32) => g
                        .get_detail_attribute_mut(aid.as_str())
                        .and_then(|a| a.as_mut_slice::<f32>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Detail, c_api::CAttrType::Vec2) => g
                        .get_detail_attribute_mut(aid.as_str())
                        .and_then(|a| a.as_mut_slice::<Vec2>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Detail, c_api::CAttrType::Vec3) => g
                        .get_detail_attribute_mut(aid.as_str())
                        .and_then(|a| a.as_mut_slice::<Vec3>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    (c_api::CAttrDomain::Detail, c_api::CAttrType::Vec4) => g
                        .get_detail_attribute_mut(aid.as_str())
                        .and_then(|a| a.as_mut_slice::<Vec4>())
                        .map(|s| s.as_mut_ptr() as *mut c_void),
                    _ => None,
                }
                .unwrap_or(std::ptr::null_mut());
                c_api::CGeoSlice {
                    ptr,
                    len: dlen as u32,
                    stride,
                }
            }
        }
        extern "C" fn attr_view(
            u: *mut c_void,
            h: c_api::GeoHandle,
            domain: c_api::CAttrDomain,
            ty: c_api::CAttrType,
            name: c_api::CStringView,
        ) -> c_api::CGeoSlice {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let g0;
                let g: &Geometry = if let Some(a) = st.arena.get_read(h) {
                    a.as_ref()
                } else if let Some(w) = st.arena.get_write(h) {
                    g0 = w as *const Geometry;
                    unsafe { &*g0 }
                } else {
                    return c_api::CGeoSlice {
                        ptr: std::ptr::null_mut(),
                        len: 0,
                        stride: 0,
                    };
                };
                let Some(aid) = attr_id(name) else {
                    return c_api::CGeoSlice {
                        ptr: std::ptr::null_mut(),
                        len: 0,
                        stride: 0,
                    };
                };
                let view = |a: &crate::mesh::Attribute| -> Option<c_api::CGeoSlice> {
                    match ty {
                        c_api::CAttrType::I32 => a
                            .as_slice::<i32>()
                            .map(|s| c_api::CGeoSlice {
                                ptr: s.as_ptr() as *mut c_void,
                                len: s.len() as u32,
                                stride: 4,
                            })
                            .or_else(|| {
                                a.as_paged::<i32>().map(|p| {
                                    st.scratch.push(Scratch::I32(p.flatten()));
                                    let Scratch::I32(v) = st.scratch.last().unwrap() else {
                                        unreachable!()
                                    };
                                    c_api::CGeoSlice {
                                        ptr: v.as_ptr() as *mut c_void,
                                        len: v.len() as u32,
                                        stride: 4,
                                    }
                                })
                            }),
                        c_api::CAttrType::F32 => a
                            .as_slice::<f32>()
                            .map(|s| c_api::CGeoSlice {
                                ptr: s.as_ptr() as *mut c_void,
                                len: s.len() as u32,
                                stride: 4,
                            })
                            .or_else(|| {
                                a.as_paged::<f32>().map(|p| {
                                    st.scratch.push(Scratch::F32(p.flatten()));
                                    let Scratch::F32(v) = st.scratch.last().unwrap() else {
                                        unreachable!()
                                    };
                                    c_api::CGeoSlice {
                                        ptr: v.as_ptr() as *mut c_void,
                                        len: v.len() as u32,
                                        stride: 4,
                                    }
                                })
                            }),
                        c_api::CAttrType::Vec2 => a
                            .as_slice::<Vec2>()
                            .map(|s| c_api::CGeoSlice {
                                ptr: s.as_ptr() as *mut c_void,
                                len: s.len() as u32,
                                stride: core::mem::size_of::<Vec2>() as u32,
                            })
                            .or_else(|| {
                                a.as_paged::<Vec2>().map(|p| {
                                    st.scratch.push(Scratch::V2(p.flatten()));
                                    let Scratch::V2(v) = st.scratch.last().unwrap() else {
                                        unreachable!()
                                    };
                                    c_api::CGeoSlice {
                                        ptr: v.as_ptr() as *mut c_void,
                                        len: v.len() as u32,
                                        stride: core::mem::size_of::<Vec2>() as u32,
                                    }
                                })
                            }),
                        c_api::CAttrType::Vec3 => a
                            .as_slice::<Vec3>()
                            .map(|s| c_api::CGeoSlice {
                                ptr: s.as_ptr() as *mut c_void,
                                len: s.len() as u32,
                                stride: core::mem::size_of::<Vec3>() as u32,
                            })
                            .or_else(|| {
                                a.as_paged::<Vec3>().map(|p| {
                                    st.scratch.push(Scratch::V3(p.flatten()));
                                    let Scratch::V3(v) = st.scratch.last().unwrap() else {
                                        unreachable!()
                                    };
                                    c_api::CGeoSlice {
                                        ptr: v.as_ptr() as *mut c_void,
                                        len: v.len() as u32,
                                        stride: core::mem::size_of::<Vec3>() as u32,
                                    }
                                })
                            }),
                        c_api::CAttrType::Vec4 => a
                            .as_slice::<Vec4>()
                            .map(|s| c_api::CGeoSlice {
                                ptr: s.as_ptr() as *mut c_void,
                                len: s.len() as u32,
                                stride: core::mem::size_of::<Vec4>() as u32,
                            })
                            .or_else(|| {
                                a.as_paged::<Vec4>().map(|p| {
                                    st.scratch.push(Scratch::V4(p.flatten()));
                                    let Scratch::V4(v) = st.scratch.last().unwrap() else {
                                        unreachable!()
                                    };
                                    c_api::CGeoSlice {
                                        ptr: v.as_ptr() as *mut c_void,
                                        len: v.len() as u32,
                                        stride: core::mem::size_of::<Vec4>() as u32,
                                    }
                                })
                            }),
                        _ => None,
                    }
                };
                let a = match domain {
                    c_api::CAttrDomain::Point => g.get_point_attribute(aid),
                    c_api::CAttrDomain::Vertex => g.get_vertex_attribute(aid),
                    c_api::CAttrDomain::Primitive => g.get_primitive_attribute(aid),
                    c_api::CAttrDomain::Edge => g.get_edge_attribute(aid),
                    c_api::CAttrDomain::Detail => g.get_detail_attribute(aid.as_str()),
                };
                a.and_then(view).unwrap_or(c_api::CGeoSlice {
                    ptr: std::ptr::null_mut(),
                    len: 0,
                    stride: 0,
                })
            }
        }

        extern "C" fn attr_bool_get(
            u: *mut c_void,
            h: c_api::GeoHandle,
            domain: c_api::CAttrDomain,
            name: c_api::CStringView,
            index: u32,
        ) -> u32 {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let g0;
                let g: &Geometry = if let Some(a) = st.arena.get_read(h) {
                    a.as_ref()
                } else if let Some(w) = st.arena.get_write(h) {
                    g0 = w as *const Geometry;
                    &*g0
                } else {
                    return 0;
                };
                let Some(aid) = attr_id(name) else {
                    return 0;
                };
                let a = match domain {
                    c_api::CAttrDomain::Point => g.get_point_attribute(aid),
                    c_api::CAttrDomain::Vertex => g.get_vertex_attribute(aid),
                    c_api::CAttrDomain::Primitive => g.get_primitive_attribute(aid),
                    c_api::CAttrDomain::Edge => g.get_edge_attribute(aid),
                    c_api::CAttrDomain::Detail => g.get_detail_attribute(aid.as_str()),
                };
                let Some(a) = a else {
                    return 0;
                };
                if let Some(v) = a
                    .as_slice::<bool>()
                    .and_then(|s| s.get(index as usize))
                    .copied()
                {
                    return if v { 1 } else { 0 };
                }
                if let Some(pb) = a.as_paged::<bool>() {
                    return pb
                        .get(index as usize)
                        .map(|v| if v { 1 } else { 0 })
                        .unwrap_or(0);
                }
                0
            }
        }
        extern "C" fn attr_bool_set(
            u: *mut c_void,
            h: c_api::GeoHandle,
            domain: c_api::CAttrDomain,
            name: c_api::CStringView,
            index: u32,
            value: u32,
        ) -> u32 {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let Some(g) = st.arena.get_write(h) else {
                    return 0;
                };
                let Some(aid) = attr_id(name) else {
                    return 0;
                };
                let dlen = match domain {
                    c_api::CAttrDomain::Point => g.points().len().max((index as usize) + 1),
                    c_api::CAttrDomain::Vertex => g.vertices().len().max((index as usize) + 1),
                    c_api::CAttrDomain::Primitive => g.primitives().len().max((index as usize) + 1),
                    c_api::CAttrDomain::Edge => g.edges().len().max((index as usize) + 1),
                    c_api::CAttrDomain::Detail => 1usize.max((index as usize) + 1),
                };
                let mut missing = false;
                match domain {
                    c_api::CAttrDomain::Point => missing = g.get_point_attribute_mut(aid).is_none(),
                    c_api::CAttrDomain::Vertex => {
                        missing = g.get_vertex_attribute_mut(aid).is_none()
                    }
                    c_api::CAttrDomain::Primitive => {
                        missing = g.get_primitive_attribute_mut(aid).is_none()
                    }
                    c_api::CAttrDomain::Edge => missing = g.get_edge_attribute_mut(aid).is_none(),
                    c_api::CAttrDomain::Detail => {
                        missing = g.get_detail_attribute_mut(aid.as_str()).is_none()
                    }
                }
                if missing {
                    let a = crate::mesh::Attribute::new(vec![false; dlen]);
                    match domain {
                        c_api::CAttrDomain::Point => g.insert_point_attribute(aid, a),
                        c_api::CAttrDomain::Vertex => g.insert_vertex_attribute(aid, a),
                        c_api::CAttrDomain::Primitive => g.insert_primitive_attribute(aid, a),
                        c_api::CAttrDomain::Edge => g.insert_edge_attribute(aid, a),
                        c_api::CAttrDomain::Detail => g.insert_detail_attribute(aid, a),
                    }
                }
                let a: &mut crate::mesh::Attribute = match domain {
                    c_api::CAttrDomain::Point => g.get_point_attribute_mut(aid),
                    c_api::CAttrDomain::Vertex => g.get_vertex_attribute_mut(aid),
                    c_api::CAttrDomain::Primitive => g.get_primitive_attribute_mut(aid),
                    c_api::CAttrDomain::Edge => g.get_edge_attribute_mut(aid),
                    c_api::CAttrDomain::Detail => g.get_detail_attribute_mut(aid.as_str()),
                }
                .unwrap();
                if a.is_paged() {
                    if let Some(v) = a.to_vec::<bool>().map(crate::mesh::Attribute::new) {
                        *a = v;
                    }
                }
                while a.len() < dlen {
                    a.push_default();
                }
                let Some(sl) = a.as_storage_mut::<Vec<bool>>() else {
                    return 0;
                };
                if (index as usize) >= sl.len() {
                    return 0;
                }
                sl[index as usize] = value != 0;
                1
            }
        }
        extern "C" fn attr_string_get(
            u: *mut c_void,
            h: c_api::GeoHandle,
            domain: c_api::CAttrDomain,
            name: c_api::CStringView,
            index: u32,
            out_ptr: *mut u8,
            out_cap: u32,
        ) -> u32 {
            unsafe {
                if out_ptr.is_null() || out_cap == 0 {
                    return 0;
                }
                let st = &mut *(u as *mut HostState);
                let g0;
                let g: &Geometry = if let Some(a) = st.arena.get_read(h) {
                    a.as_ref()
                } else if let Some(w) = st.arena.get_write(h) {
                    g0 = w as *const Geometry;
                    &*g0
                } else {
                    return 0;
                };
                let Some(aid) = attr_id(name) else {
                    return 0;
                };
                let a = match domain {
                    c_api::CAttrDomain::Point => g.get_point_attribute(aid),
                    c_api::CAttrDomain::Vertex => g.get_vertex_attribute(aid),
                    c_api::CAttrDomain::Primitive => g.get_primitive_attribute(aid),
                    c_api::CAttrDomain::Edge => g.get_edge_attribute(aid),
                    c_api::CAttrDomain::Detail => g.get_detail_attribute(aid.as_str()),
                };
                let Some(a) = a else {
                    return 0;
                };
                let s: &str =
                    if let Some(v) = a.as_slice::<String>().and_then(|v| v.get(index as usize)) {
                        v.as_str()
                    } else if let Some(pb) = a.as_paged::<String>() {
                        st.scratch.push(Scratch::Str(pb.flatten()));
                        let Scratch::Str(v) = st.scratch.last().unwrap() else {
                            unreachable!()
                        };
                        v.get(index as usize).map(|s| s.as_str()).unwrap_or("")
                    } else {
                        return 0;
                    };
                let b = s.as_bytes();
                let n = b.len().min(out_cap as usize);
                std::ptr::copy_nonoverlapping(b.as_ptr(), out_ptr, n);
                n as u32
            }
        }
        extern "C" fn attr_string_set(
            u: *mut c_void,
            h: c_api::GeoHandle,
            domain: c_api::CAttrDomain,
            name: c_api::CStringView,
            index: u32,
            value: c_api::CStringView,
        ) -> u32 {
            unsafe {
                let st = &mut *(u as *mut HostState);
                let Some(g) = st.arena.get_write(h) else {
                    return 0;
                };
                let Some(aid) = attr_id(name) else {
                    return 0;
                };
                let Some(vs) = sv_to_string(value) else {
                    return 0;
                };
                let dlen = match domain {
                    c_api::CAttrDomain::Point => g.points().len().max((index as usize) + 1),
                    c_api::CAttrDomain::Vertex => g.vertices().len().max((index as usize) + 1),
                    c_api::CAttrDomain::Primitive => g.primitives().len().max((index as usize) + 1),
                    c_api::CAttrDomain::Edge => g.edges().len().max((index as usize) + 1),
                    c_api::CAttrDomain::Detail => 1usize.max((index as usize) + 1),
                };
                let mut missing = false;
                match domain {
                    c_api::CAttrDomain::Point => missing = g.get_point_attribute_mut(aid).is_none(),
                    c_api::CAttrDomain::Vertex => {
                        missing = g.get_vertex_attribute_mut(aid).is_none()
                    }
                    c_api::CAttrDomain::Primitive => {
                        missing = g.get_primitive_attribute_mut(aid).is_none()
                    }
                    c_api::CAttrDomain::Edge => missing = g.get_edge_attribute_mut(aid).is_none(),
                    c_api::CAttrDomain::Detail => {
                        missing = g.get_detail_attribute_mut(aid.as_str()).is_none()
                    }
                }
                if missing {
                    let a = crate::mesh::Attribute::new(vec![String::new(); dlen]);
                    match domain {
                        c_api::CAttrDomain::Point => g.insert_point_attribute(aid, a),
                        c_api::CAttrDomain::Vertex => g.insert_vertex_attribute(aid, a),
                        c_api::CAttrDomain::Primitive => g.insert_primitive_attribute(aid, a),
                        c_api::CAttrDomain::Edge => g.insert_edge_attribute(aid, a),
                        c_api::CAttrDomain::Detail => g.insert_detail_attribute(aid, a),
                    }
                }
                let a: &mut crate::mesh::Attribute = match domain {
                    c_api::CAttrDomain::Point => g.get_point_attribute_mut(aid),
                    c_api::CAttrDomain::Vertex => g.get_vertex_attribute_mut(aid),
                    c_api::CAttrDomain::Primitive => g.get_primitive_attribute_mut(aid),
                    c_api::CAttrDomain::Edge => g.get_edge_attribute_mut(aid),
                    c_api::CAttrDomain::Detail => g.get_detail_attribute_mut(aid.as_str()),
                }
                .unwrap();
                if a.is_paged() {
                    if let Some(v) = a.to_vec::<String>().map(crate::mesh::Attribute::new) {
                        *a = v;
                    }
                }
                while a.len() < dlen {
                    a.push_default();
                }
                let Some(sl) = a.as_storage_mut::<Vec<String>>() else {
                    return 0;
                };
                if (index as usize) >= sl.len() {
                    return 0;
                }
                sl[index as usize] = vs;
                1
            }
        }

        extern "C" fn node_state_get(
            _u: *mut c_void,
            _node: c_api::CUuid,
            _key: c_api::CStringView,
            _out_ptr: *mut u8,
            _out_cap: u32,
        ) -> u32 {
            0
        }
        extern "C" fn node_state_set(
            _u: *mut c_void,
            _node: c_api::CUuid,
            _key: c_api::CStringView,
            _bytes: *const u8,
            _len: u32,
        ) -> u32 {
            0
        }
        extern "C" fn node_curve_get(
            _u: *mut c_void,
            _node: c_api::CUuid,
            _param: c_api::CStringView,
            _out_pts: *mut c_api::CCurveControlPoint,
            _out_cap: u32,
            _out_len: *mut u32,
            _out_closed: *mut u32,
            _out_ty: *mut u32,
        ) -> u32 {
            0
        }
        extern "C" fn node_curve_set(
            _u: *mut c_void,
            _node: c_api::CUuid,
            _param: c_api::CStringView,
            _pts: *const c_api::CCurveControlPoint,
            _len: u32,
            _closed: u32,
            _ty: u32,
        ) -> u32 {
            0
        }

        let mut st = HostState { arena, scratch };
        let host = c_api::CHostApi {
            userdata: &mut st as *mut _ as *mut c_void,
            geo_create,
            geo_clone,
            geo_drop,
            geo_point_count,
            geo_vertex_count,
            geo_prim_count,
            geo_edge_count,
            geo_add_point,
            geo_add_vertex,
            geo_remove_point,
            geo_remove_vertex,
            geo_add_edge,
            geo_remove_edge,
            geo_add_polygon,
            geo_add_polyline,
            geo_set_prim_vertices,
            geo_remove_prim,
            geo_vertex_point,
            geo_edge_points,
            geo_prim_point_count,
            geo_prim_points,
            geo_get_point_position,
            geo_set_point_position,
            attr_ensure,
            attr_view,
            attr_bool_get,
            attr_bool_set,
            attr_string_get,
            attr_string_set,
            node_state_get,
            node_state_set,
            node_curve_get,
            node_curve_set,
        };
        let ctx = c_api::CExecutionCtx {
            time: 0.0,
            frame: 0.0,
        };
        let rc = (self.vtable.compute)(
            self.instance,
            &host,
            &ctx,
            in_handles.as_ptr(),
            in_handles.len() as u32,
            pv.as_ptr(),
            pv.len() as u32,
            &mut out,
        );
        let mut arena2 = st.arena;
        if rc != 0 {
            return Arc::new(Geometry::new());
        }
        Arc::new(arena2.take_write(out).unwrap_or_else(Geometry::new))
    }
}

impl Drop for NativeNodeWrapper {
    fn drop(&mut self) {
        (self.vtable.destroy)(self.instance);
    }
}

// ----------------------------------------------------------------------------
// Plugin Interaction (HUD / Gizmo / Input)
// ----------------------------------------------------------------------------

pub(crate) struct PluginInteractionShared {
    _library: Arc<Library>,
    node_vtable: c_api::CNodeVTable,
    interaction_vtable: c_api::CNodeInteractionVTable,
    instance: *mut std::ffi::c_void,
    node_name: String,
    node_state: Arc<RwLock<std::collections::HashMap<(crate::nodes::NodeId, String), Vec<u8>>>>,
}

unsafe impl Send for PluginInteractionShared {}
unsafe impl Sync for PluginInteractionShared {}

impl Drop for PluginInteractionShared {
    fn drop(&mut self) {
        (self.node_vtable.destroy)(self.instance);
    }
}

#[derive(Clone)]
struct PluginInteractionNode {
    shared: Arc<PluginInteractionShared>,
}

impl crate::cunning_core::traits::node_interface::NodeInteraction for PluginInteractionNode {
    fn draw_hud(
        &self,
        ui: &mut bevy_egui::egui::Ui,
        services: &dyn crate::cunning_core::traits::node_interface::ServiceProvider,
        node_id: uuid::Uuid,
    ) {
        let _ = services; // no-hitch: do not depend on mutable graph access here
        let snapshot = crate::nodes::graph_model::global_graph_snapshot()
            .unwrap_or_else(|| Arc::new(Default::default()));
        let mut host_state = InteractionHostState {
            snapshot,
            node_state: self.shared.node_state.clone(),
        };
        let host = build_interaction_host_api(&mut host_state as *mut _ as *mut c_void);

        let mut cmds: Vec<c_api::CHudCmd> = vec![
            c_api::CHudCmd {
                tag: c_api::CHudCmdTag::Separator,
                id: 0,
                value: 0,
                _pad0: 0,
                text: c_api::CStringView {
                    ptr: core::ptr::null(),
                    len: 0
                }
            };
            64
        ];
        let n = (self.shared.interaction_vtable.hud_build)(
            self.shared.instance,
            &host,
            to_cuuid(node_id),
            cmds.as_mut_ptr(),
            cmds.len() as u32,
        ) as usize;
        let cmds = &cmds[..n.min(cmds.len())];

        let accent = bevy_egui::egui::Color32::from_rgb(255, 180, 0);
        let normal = ui.visuals().text_color();
        for c in cmds {
            match c.tag {
                c_api::CHudCmdTag::Label => {
                    ui.label(sv_to_string(c.text));
                }
                c_api::CHudCmdTag::Separator => {
                    ui.separator();
                }
                c_api::CHudCmdTag::Button => {
                    let text = sv_to_string(c.text);
                    let color = if c.value != 0 { accent } else { normal };
                    let resp = ui.add(
                        bevy_egui::egui::Label::new(
                            bevy_egui::egui::RichText::new(text).color(color),
                        )
                        .sense(bevy_egui::egui::Sense::click()),
                    );
                    if resp.hovered() {
                        ui.output_mut(|o| {
                            o.cursor_icon = bevy_egui::egui::CursorIcon::PointingHand
                        });
                    }
                    if resp.clicked() {
                        let e = c_api::CHudEvent { id: c.id, value: 1 };
                        let _ = (self.shared.interaction_vtable.hud_event)(
                            self.shared.instance,
                            &host,
                            to_cuuid(node_id),
                            &e as *const _,
                        );
                    }
                }
                c_api::CHudCmdTag::Toggle => {
                    let text = sv_to_string(c.text);
                    let on = c.value != 0;
                    let color = if on { accent } else { normal };
                    let resp = ui.add(
                        bevy_egui::egui::Label::new(
                            bevy_egui::egui::RichText::new(text).color(color),
                        )
                        .sense(bevy_egui::egui::Sense::click()),
                    );
                    if resp.hovered() {
                        ui.output_mut(|o| {
                            o.cursor_icon = bevy_egui::egui::CursorIcon::PointingHand
                        });
                    }
                    if resp.clicked() {
                        let e = c_api::CHudEvent {
                            id: c.id,
                            value: if on { 0 } else { 1 },
                        };
                        let _ = (self.shared.interaction_vtable.hud_event)(
                            self.shared.instance,
                            &host,
                            to_cuuid(node_id),
                            &e as *const _,
                        );
                    }
                }
            }
        }
    }
}

struct InteractionHostState {
    snapshot: Arc<crate::nodes::graph_model::NodeGraphSnapshot>,
    node_state: Arc<RwLock<std::collections::HashMap<(crate::nodes::NodeId, String), Vec<u8>>>>,
}

#[inline]
fn to_cuuid(id: uuid::Uuid) -> c_api::CUuid {
    let b = id.as_bytes();
    let lo = u64::from_le_bytes(b[0..8].try_into().unwrap());
    let hi = u64::from_le_bytes(b[8..16].try_into().unwrap());
    c_api::CUuid { lo, hi }
}
#[inline]
fn from_cuuid(id: c_api::CUuid) -> uuid::Uuid {
    let mut b = [0u8; 16];
    b[0..8].copy_from_slice(&id.lo.to_le_bytes());
    b[8..16].copy_from_slice(&id.hi.to_le_bytes());
    uuid::Uuid::from_bytes(b)
}
#[inline]
fn sv_to_string(s: c_api::CStringView) -> String {
    if s.ptr.is_null() || s.len == 0 {
        String::new()
    } else {
        unsafe {
            String::from_utf8_lossy(core::slice::from_raw_parts(
                s.ptr as *const u8,
                s.len as usize,
            ))
            .to_string()
        }
    }
}

fn build_interaction_host_api(userdata: *mut c_void) -> c_api::CHostApi {
    extern "C" fn z0_u32(_: *mut c_void, _: c_api::GeoHandle) -> u32 {
        0
    }
    extern "C" fn z0_u32_0(_: *mut c_void) -> c_api::GeoHandle {
        0
    }
    extern "C" fn z0_clone(_: *mut c_void, _: c_api::GeoHandle) -> c_api::GeoHandle {
        0
    }
    extern "C" fn z0_void(_: *mut c_void, _: c_api::GeoHandle) {}
    extern "C" fn z0_add_point(_: *mut c_void, _: c_api::GeoHandle) -> u32 {
        0
    }
    extern "C" fn z0_add_vertex(_: *mut c_void, _: c_api::GeoHandle, _: u32) -> u32 {
        0
    }
    extern "C" fn z0_remove_point(_: *mut c_void, _: c_api::GeoHandle, _: u32) {}
    extern "C" fn z0_remove_vertex(_: *mut c_void, _: c_api::GeoHandle, _: u32) {}
    extern "C" fn z0_add_edge(_: *mut c_void, _: c_api::GeoHandle, _: u32, _: u32) -> u32 {
        0
    }
    extern "C" fn z0_remove_edge(_: *mut c_void, _: c_api::GeoHandle, _: u32) {}
    extern "C" fn z0_add_polygon(
        _: *mut c_void,
        _: c_api::GeoHandle,
        _: *const u32,
        _: u32,
    ) -> u32 {
        0
    }
    extern "C" fn z0_add_polyline(
        _: *mut c_void,
        _: c_api::GeoHandle,
        _: *const u32,
        _: u32,
        _: u32,
    ) -> u32 {
        0
    }
    extern "C" fn z0_set_prim_vertices(
        _: *mut c_void,
        _: c_api::GeoHandle,
        _: u32,
        _: *const u32,
        _: u32,
    ) {
    }
    extern "C" fn z0_remove_prim(_: *mut c_void, _: c_api::GeoHandle, _: u32) {}
    extern "C" fn z0_vertex_point(_: *mut c_void, _: c_api::GeoHandle, _: u32) -> u32 {
        0
    }
    extern "C" fn z0_edge_points(_: *mut c_void, _: c_api::GeoHandle, _: u32, _: *mut u32) -> u32 {
        0
    }
    extern "C" fn z0_prim_point_count(_: *mut c_void, _: c_api::GeoHandle, _: u32) -> u32 {
        0
    }
    extern "C" fn z0_prim_points(
        _: *mut c_void,
        _: c_api::GeoHandle,
        _: u32,
        _: *mut u32,
        _: u32,
    ) -> u32 {
        0
    }
    extern "C" fn z0_get_p(_: *mut c_void, _: c_api::GeoHandle, _: u32, _: *mut f32) -> u32 {
        0
    }
    extern "C" fn z0_set_p(_: *mut c_void, _: c_api::GeoHandle, _: u32, _: *const f32) -> u32 {
        0
    }
    extern "C" fn z0_slice(
        _: *mut c_void,
        _: c_api::GeoHandle,
        _: c_api::CAttrDomain,
        _: c_api::CAttrType,
        _: c_api::CStringView,
        _: u32,
    ) -> c_api::CGeoSlice {
        c_api::CGeoSlice {
            ptr: core::ptr::null_mut(),
            len: 0,
            stride: 0,
        }
    }
    extern "C" fn z0_slice_view(
        _: *mut c_void,
        _: c_api::GeoHandle,
        _: c_api::CAttrDomain,
        _: c_api::CAttrType,
        _: c_api::CStringView,
    ) -> c_api::CGeoSlice {
        c_api::CGeoSlice {
            ptr: core::ptr::null_mut(),
            len: 0,
            stride: 0,
        }
    }
    extern "C" fn z0_attr_bool_get(
        _: *mut c_void,
        _: c_api::GeoHandle,
        _: c_api::CAttrDomain,
        _: c_api::CStringView,
        _: u32,
    ) -> u32 {
        0
    }
    extern "C" fn z0_attr_bool_set(
        _: *mut c_void,
        _: c_api::GeoHandle,
        _: c_api::CAttrDomain,
        _: c_api::CStringView,
        _: u32,
        _: u32,
    ) -> u32 {
        0
    }
    extern "C" fn z0_attr_string_get(
        _: *mut c_void,
        _: c_api::GeoHandle,
        _: c_api::CAttrDomain,
        _: c_api::CStringView,
        _: u32,
        _: *mut u8,
        _: u32,
    ) -> u32 {
        0
    }
    extern "C" fn z0_attr_string_set(
        _: *mut c_void,
        _: c_api::GeoHandle,
        _: c_api::CAttrDomain,
        _: c_api::CStringView,
        _: u32,
        _: c_api::CStringView,
    ) -> u32 {
        0
    }

    extern "C" fn node_state_get(
        u: *mut c_void,
        node: c_api::CUuid,
        key: c_api::CStringView,
        out_ptr: *mut u8,
        out_cap: u32,
    ) -> u32 {
        unsafe {
            let st = &mut *(u as *mut InteractionHostState);
            let nid = from_cuuid(node);
            let k = sv_to_string(key);
            let map = st.node_state.read().unwrap();
            let Some(v) = map.get(&(nid, k)) else {
                return 0;
            };
            if !out_ptr.is_null() && out_cap > 0 {
                let n = (v.len() as u32).min(out_cap) as usize;
                core::ptr::copy_nonoverlapping(v.as_ptr(), out_ptr, n);
            }
            v.len() as u32
        }
    }

    extern "C" fn node_state_set(
        u: *mut c_void,
        node: c_api::CUuid,
        key: c_api::CStringView,
        bytes: *const u8,
        len: u32,
    ) -> u32 {
        unsafe {
            let st = &mut *(u as *mut InteractionHostState);
            let nid = from_cuuid(node);
            let k = sv_to_string(key);
            let b = if bytes.is_null() || len == 0 {
                &[]
            } else {
                core::slice::from_raw_parts(bytes, len as usize)
            };
            st.node_state.write().unwrap().insert((nid, k), b.to_vec());
            1
        }
    }

    extern "C" fn node_curve_get(
        u: *mut c_void,
        node: c_api::CUuid,
        param: c_api::CStringView,
        out_pts: *mut c_api::CCurveControlPoint,
        out_cap: u32,
        out_len: *mut u32,
        out_closed: *mut u32,
        out_ty: *mut u32,
    ) -> u32 {
        unsafe {
            let st = &mut *(u as *mut InteractionHostState);
            let nid = from_cuuid(node);
            let pname = sv_to_string(param);
            let log = std::env::var_os("DCC_LOG_CURVE_PLUGIN").is_some() && pname == "curve_data";
            #[inline]
            fn ensure_curve_param(
                root: &mut crate::nodes::NodeGraph,
                node_id: crate::nodes::NodeId,
                param: &str,
            ) -> bool {
                if let Some(n) = root.nodes.get_mut(&node_id) {
                    if n.parameters.iter().any(|p| p.name == param) {
                        return true;
                    }
                    let mut d: crate::nodes::parameter::CurveData = Default::default();
                    d.curve_type = crate::nodes::parameter::CurveType::Bezier;
                    n.parameters.push(crate::nodes::parameter::Parameter::new(
                        param,
                        "Curve Data",
                        "Geometry",
                        crate::nodes::parameter::ParameterValue::Curve(d),
                        crate::nodes::parameter::ParameterUIType::CurvePoints,
                    ));
                    root.mark_dirty(node_id);
                    return true;
                }
                if let Some(lib) = crate::cunning_core::cda::library::global_cda_library() {
                    let mut hit = false;
                    lib.with_defs_mut(|defs| {
                        for a in defs.values_mut() {
                            if let Some(n) = a.inner_graph.nodes.get_mut(&node_id) {
                                if !n.parameters.iter().any(|p| p.name == param) {
                                    let mut d: crate::nodes::parameter::CurveData =
                                        Default::default();
                                    d.curve_type = crate::nodes::parameter::CurveType::Bezier;
                                    n.parameters.push(crate::nodes::parameter::Parameter::new(
                                        param,
                                        "Curve Data",
                                        "Geometry",
                                        crate::nodes::parameter::ParameterValue::Curve(d),
                                        crate::nodes::parameter::ParameterUIType::CurvePoints,
                                    ));
                                    a.inner_graph.mark_dirty(node_id);
                                }
                                hit = true;
                                break;
                            }
                        }
                    });
                    return hit;
                }
                false
            }
            let mut out_n = 0u32;
            let mut out_c = 0u32;
            let mut out_t = c_api::CCurveType::Polygon as u32;
            let mut pts: Vec<c_api::CCurveControlPoint> = Vec::new();
            let found = st
                .snapshot
                .nodes
                .get(&nid)
                .and_then(|n| n.parameters.iter().find(|p| p.name == pname))
                .and_then(|p| match &p.value {
                    crate::nodes::parameter::ParameterValue::Curve(d) => Some(d.clone()),
                    _ => None,
                });
            let Some(data) = found else {
                return 0;
            };
            out_c = if data.is_closed { 1 } else { 0 };
            out_t = match data.curve_type {
                crate::nodes::parameter::CurveType::Polygon => c_api::CCurveType::Polygon as u32,
                crate::nodes::parameter::CurveType::Bezier => c_api::CCurveType::Bezier as u32,
                crate::nodes::parameter::CurveType::Nurbs => c_api::CCurveType::Nurbs as u32,
            };
            pts = data
                .points
                .iter()
                .map(|p| c_api::CCurveControlPoint {
                    id: to_cuuid(p.id),
                    position: [p.position.x, p.position.y, p.position.z],
                    mode: match p.mode {
                        crate::nodes::parameter::PointMode::Corner => c_api::CPointMode::Corner,
                        crate::nodes::parameter::PointMode::Bezier => c_api::CPointMode::Bezier,
                    },
                    _pad0: [0; 2],
                    handle_in: [p.handle_in.x, p.handle_in.y, p.handle_in.z],
                    _pad1: 0,
                    handle_out: [p.handle_out.x, p.handle_out.y, p.handle_out.z],
                    _pad2: 0,
                    weight: p.weight,
                    _pad3: [0; 3],
                })
                .collect();
            if log {
                bevy::log::info!(
                    "[CurvePluginHost] node_curve_get node={:?} found={} points={}",
                    nid,
                    true,
                    pts.len()
                );
            }
            out_n = pts.len() as u32;
            if !out_len.is_null() {
                *out_len = out_n;
            }
            if !out_closed.is_null() {
                *out_closed = out_c;
            }
            if !out_ty.is_null() {
                *out_ty = out_t;
            }
            if !out_pts.is_null() && out_cap > 0 {
                let n = (out_n.min(out_cap)) as usize;
                core::ptr::copy_nonoverlapping(pts.as_ptr(), out_pts, n);
            }
            1
        }
    }

    extern "C" fn node_curve_set(
        u: *mut c_void,
        node: c_api::CUuid,
        param: c_api::CStringView,
        pts: *const c_api::CCurveControlPoint,
        len: u32,
        closed: u32,
        ty: u32,
    ) -> u32 {
        unsafe {
            let st = &mut *(u as *mut InteractionHostState);
            let nid = from_cuuid(node);
            let pname = sv_to_string(param);
            let log = std::env::var_os("DCC_LOG_CURVE_PLUGIN").is_some() && pname == "curve_data";
            let pts = if pts.is_null() || len == 0 {
                &[]
            } else {
                core::slice::from_raw_parts(pts, len as usize)
            };
            let points: Vec<crate::nodes::parameter::CurveControlPoint> = pts
                .iter()
                .map(|p| crate::nodes::parameter::CurveControlPoint {
                    id: from_cuuid(p.id),
                    position: Vec3::new(p.position[0], p.position[1], p.position[2]),
                    mode: match p.mode {
                        c_api::CPointMode::Bezier => crate::nodes::parameter::PointMode::Bezier,
                        _ => crate::nodes::parameter::PointMode::Corner,
                    },
                    handle_in: Vec3::new(p.handle_in[0], p.handle_in[1], p.handle_in[2]),
                    handle_out: Vec3::new(p.handle_out[0], p.handle_out[1], p.handle_out[2]),
                    weight: p.weight,
                })
                .collect();
            let ty = match ty {
                x if x == c_api::CCurveType::Bezier as u32 => crate::nodes::parameter::CurveType::Bezier,
                x if x == c_api::CCurveType::Nurbs as u32 => crate::nodes::parameter::CurveType::Nurbs,
                _ => crate::nodes::parameter::CurveType::Polygon,
            };
            let closed = closed != 0;
            let pname2 = pname.clone();
            let points_len = points.len();
            let _ = crate::nodes::graph_model::enqueue_graph_command(Box::new(
                move |root: &mut crate::nodes::NodeGraph| {
                    let mut eff = crate::nodes::graph_model::GraphCommandEffect {
                        graph_changed: true,
                        geometry_changed: true,
                    };
                    if let Some(n) = root.nodes.get_mut(&nid) {
                        let mut found = false;
                        for p in &mut n.parameters {
                            if p.name == pname2 {
                                if let crate::nodes::parameter::ParameterValue::Curve(d) = &mut p.value {
                                    d.is_closed = closed;
                                    d.curve_type = ty.clone();
                                    d.points = points.clone();
                                    found = true;
                                    break;
                                }
                            }
                        }
                        if !found {
                            let mut d: crate::nodes::parameter::CurveData = Default::default();
                            d.is_closed = closed;
                            d.curve_type = ty;
                            d.points = points;
                            n.parameters.push(crate::nodes::parameter::Parameter::new(
                                &pname2,
                                "Curve Data",
                                "Geometry",
                                crate::nodes::parameter::ParameterValue::Curve(d),
                                crate::nodes::parameter::ParameterUIType::CurvePoints,
                            ));
                        }
                        root.mark_dirty(nid);
                    } else {
                        eff.geometry_changed = false;
                    }
                    eff
                },
            ));
            if log {
                bevy::log::info!(
                    "[CurvePluginHost] node_curve_set node={:?} queued points={}",
                    nid,
                    points_len
                );
            }
            1
        }
    }

    c_api::CHostApi {
        userdata,
        geo_create: z0_u32_0,
        geo_clone: z0_clone,
        geo_drop: z0_void,
        geo_point_count: z0_u32,
        geo_vertex_count: z0_u32,
        geo_prim_count: z0_u32,
        geo_edge_count: z0_u32,
        geo_add_point: z0_add_point,
        geo_add_vertex: z0_add_vertex,
        geo_remove_point: z0_remove_point,
        geo_remove_vertex: z0_remove_vertex,
        geo_add_edge: z0_add_edge,
        geo_remove_edge: z0_remove_edge,
        geo_add_polygon: z0_add_polygon,
        geo_add_polyline: z0_add_polyline,
        geo_set_prim_vertices: z0_set_prim_vertices,
        geo_remove_prim: z0_remove_prim,
        geo_vertex_point: z0_vertex_point,
        geo_edge_points: z0_edge_points,
        geo_prim_point_count: z0_prim_point_count,
        geo_prim_points: z0_prim_points,
        geo_get_point_position: z0_get_p,
        geo_set_point_position: z0_set_p,
        attr_ensure: z0_slice,
        attr_view: z0_slice_view,
        attr_bool_get: z0_attr_bool_get,
        attr_bool_set: z0_attr_bool_set,
        attr_string_get: z0_attr_string_get,
        attr_string_set: z0_attr_string_set,
        node_state_get,
        node_state_set,
        node_curve_get,
        node_curve_set,
    }
}

fn find_curve_param_any(
    root: &mut crate::nodes::NodeGraph,
    node_id: crate::nodes::NodeId,
    param: &str,
    f: impl FnOnce(&crate::nodes::parameter::CurveData),
) -> bool {
    if let Some(n) = root.nodes.get(&node_id) {
        if let Some(p) = n.parameters.iter().find(|p| p.name == param) {
            if let crate::nodes::parameter::ParameterValue::Curve(d) = &p.value {
                f(d);
                return true;
            }
        }
    }
    if let Some(lib) = crate::cunning_core::cda::library::global_cda_library() {
        let mut hit = false;
        lib.with_defs_mut(|defs| {
            for a in defs.values() {
                if let Some(n) = a.inner_graph.nodes.get(&node_id) {
                    if let Some(p) = n.parameters.iter().find(|p| p.name == param) {
                        if let crate::nodes::parameter::ParameterValue::Curve(d) = &p.value {
                            f(d);
                            hit = true;
                            break;
                        }
                    }
                }
            }
        });
        if hit {
            return true;
        }
    }
    false
}

fn find_curve_param_any_mut(
    root: &mut crate::nodes::NodeGraph,
    node_id: crate::nodes::NodeId,
    param: &str,
    f: impl FnOnce(&mut crate::nodes::parameter::CurveData),
) -> bool {
    if let Some(n) = root.nodes.get_mut(&node_id) {
        if let Some(p) = n.parameters.iter_mut().find(|p| p.name == param) {
            if let crate::nodes::parameter::ParameterValue::Curve(d) = &mut p.value {
                f(d);
                return true;
            }
        }
    }
    if let Some(lib) = crate::cunning_core::cda::library::global_cda_library() {
        let mut hit = false;
        lib.with_defs_mut(|defs| {
            for a in defs.values_mut() {
                if let Some(n) = a.inner_graph.nodes.get_mut(&node_id) {
                    if let Some(p) = n.parameters.iter_mut().find(|p| p.name == param) {
                        if let crate::nodes::parameter::ParameterValue::Curve(d) = &mut p.value {
                            f(d);
                            a.inner_graph.mark_dirty(node_id);
                            hit = true;
                            break;
                        }
                    }
                }
            }
        });
        return hit;
    }
    false
}

// ----------------------------------------------------------------------------
// Plugin System
// ----------------------------------------------------------------------------

#[derive(Resource, Default)]
pub struct PluginSystem {
    loaded_libraries: Arc<RwLock<Vec<Arc<Library>>>>,
    loaded_paths: Arc<RwLock<std::collections::HashSet<std::path::PathBuf>>>,
    interactions: Arc<RwLock<std::collections::HashMap<String, Arc<PluginInteractionShared>>>>,
    node_state: Arc<RwLock<std::collections::HashMap<(crate::nodes::NodeId, String), Vec<u8>>>>,
}

impl Clone for PluginSystem {
    fn clone(&self) -> Self {
        Self {
            loaded_libraries: self.loaded_libraries.clone(),
            loaded_paths: self.loaded_paths.clone(),
            interactions: self.interactions.clone(),
            node_state: self.node_state.clone(),
        }
    }
}

impl PluginSystem {
    pub(crate) fn interaction_shared(
        &self,
        node_name: &str,
    ) -> Option<Arc<PluginInteractionShared>> {
        self.interactions.read().unwrap().get(node_name).cloned()
    }

    pub fn plugin_gizmo_build(
        &self,
        node_graph_res: &crate::NodeGraphResource,
        node_type_key: &str,
        node_id: crate::nodes::NodeId,
        cap: u32,
    ) -> Vec<c_api::CGizmoCmd> {
        let Some(shared) = self.interaction_shared(node_type_key) else {
            return Vec::new();
        };
        let _ = node_graph_res;
        let snapshot = crate::nodes::graph_model::global_graph_snapshot()
            .unwrap_or_else(|| Arc::new(Default::default()));
        let mut host_state = InteractionHostState {
            snapshot,
            node_state: shared.node_state.clone(),
        };
        let host = build_interaction_host_api(&mut host_state as *mut _ as *mut c_void);
        let mut out: Vec<c_api::CGizmoCmd> =
            vec![unsafe { std::mem::zeroed() }; cap.max(1) as usize];
        let n = (shared.interaction_vtable.gizmo_build)(
            shared.instance,
            &host,
            to_cuuid(node_id),
            out.as_mut_ptr(),
            out.len() as u32,
        ) as usize;
        out.truncate(n.min(out.len()));
        out
    }

    pub fn plugin_gizmo_event_drag(
        &self,
        node_graph_res: &crate::NodeGraphResource,
        node_type_key: &str,
        node_id: crate::nodes::NodeId,
        pick_id: u32,
        world_pos: Vec3,
    ) -> bool {
        let Some(shared) = self.interaction_shared(node_type_key) else {
            return false;
        };
        let _ = node_graph_res;
        let snapshot = crate::nodes::graph_model::global_graph_snapshot()
            .unwrap_or_else(|| Arc::new(Default::default()));
        let mut host_state = InteractionHostState {
            snapshot,
            node_state: shared.node_state.clone(),
        };
        let host = build_interaction_host_api(&mut host_state as *mut _ as *mut c_void);
        let e = c_api::CGizmoEvent {
            tag: c_api::CGizmoEventTag::Drag,
            pick_id,
            world_pos: [world_pos.x, world_pos.y, world_pos.z],
            _pad0: 0,
        };
        (shared.interaction_vtable.gizmo_event)(
            shared.instance,
            &host,
            to_cuuid(node_id),
            &e as *const _,
        ) == 0
    }

    pub fn plugin_gizmo_event_click(
        &self,
        node_graph_res: &crate::NodeGraphResource,
        node_type_key: &str,
        node_id: crate::nodes::NodeId,
        pick_id: u32,
        world_pos: Vec3,
    ) -> bool {
        let Some(shared) = self.interaction_shared(node_type_key) else {
            return false;
        };
        let _ = node_graph_res;
        let snapshot = crate::nodes::graph_model::global_graph_snapshot()
            .unwrap_or_else(|| Arc::new(Default::default()));
        let mut host_state = InteractionHostState {
            snapshot,
            node_state: shared.node_state.clone(),
        };
        let host = build_interaction_host_api(&mut host_state as *mut _ as *mut c_void);
        let e = c_api::CGizmoEvent {
            tag: c_api::CGizmoEventTag::Click,
            pick_id,
            world_pos: [world_pos.x, world_pos.y, world_pos.z],
            _pad0: 0,
        };
        (shared.interaction_vtable.gizmo_event)(
            shared.instance,
            &host,
            to_cuuid(node_id),
            &e as *const _,
        ) == 0
    }

    pub fn plugin_gizmo_event_release(
        &self,
        node_graph_res: &crate::NodeGraphResource,
        node_type_key: &str,
        node_id: crate::nodes::NodeId,
        pick_id: u32,
        world_pos: Vec3,
    ) -> bool {
        let Some(shared) = self.interaction_shared(node_type_key) else {
            return false;
        };
        let _ = node_graph_res;
        let snapshot = crate::nodes::graph_model::global_graph_snapshot()
            .unwrap_or_else(|| Arc::new(Default::default()));
        let mut host_state = InteractionHostState {
            snapshot,
            node_state: shared.node_state.clone(),
        };
        let host = build_interaction_host_api(&mut host_state as *mut _ as *mut c_void);
        let e = c_api::CGizmoEvent {
            tag: c_api::CGizmoEventTag::Release,
            pick_id,
            world_pos: [world_pos.x, world_pos.y, world_pos.z],
            _pad0: 0,
        };
        (shared.interaction_vtable.gizmo_event)(
            shared.instance,
            &host,
            to_cuuid(node_id),
            &e as *const _,
        ) == 0
    }

    pub fn plugin_input_key_down(
        &self,
        node_graph_res: &crate::NodeGraphResource,
        node_type_key: &str,
        node_id: crate::nodes::NodeId,
        key: c_api::CKeyCode,
    ) -> bool {
        let Some(shared) = self.interaction_shared(node_type_key) else {
            return false;
        };
        let _ = node_graph_res;
        let snapshot = crate::nodes::graph_model::global_graph_snapshot()
            .unwrap_or_else(|| Arc::new(Default::default()));
        let mut host_state = InteractionHostState {
            snapshot,
            node_state: shared.node_state.clone(),
        };
        let host = build_interaction_host_api(&mut host_state as *mut _ as *mut c_void);
        let e = c_api::CInputEvent {
            tag: c_api::CInputEventTag::KeyDown,
            key,
            _pad0: [0; 2],
        };
        (shared.interaction_vtable.input_event)(
            shared.instance,
            &host,
            to_cuuid(node_id),
            &e as *const _,
        ) == 0
    }

    fn pick_latest_plugins(dir_path: &Path) -> Vec<std::path::PathBuf> {
        let mut best: std::collections::HashMap<String, (u128, std::path::PathBuf)> =
            std::collections::HashMap::new();
        let Ok(entries) = std::fs::read_dir(dir_path) else {
            return Vec::new();
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let Some(ext) = p.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if !(ext.eq_ignore_ascii_case("dll")
                || ext.eq_ignore_ascii_case("so")
                || ext.eq_ignore_ascii_case("dylib"))
            {
                continue;
            }
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let (base, ts) = stem
                .rsplit_once('_')
                .and_then(|(b, t)| {
                    if !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit()) {
                        t.parse::<u128>().ok().map(|ts| (b.to_string(), ts))
                    } else {
                        None
                    }
                })
                .unwrap_or((stem.to_string(), 0));
            let e = best.entry(base).or_insert((0, p.clone()));
            if ts >= e.0 {
                *e = (ts, p);
            }
        }
        let mut out: Vec<_> = best.into_values().map(|(_, p)| p).collect();
        out.sort();
        out
    }

    pub fn scan_plugins(&self, dir_path: &str, registry: &NodeRegistry) {
        let path = Path::new(dir_path);
        if !path.exists() {
            warn!("Plugin directory not found: {}", dir_path);
            return;
        }

        info!("Scanning for plugins in: {}", dir_path);

        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(ext) = p.extension() {
                    // Load .dll on Windows, .so on Linux, .dylib on Mac
                    if ext == "dll" || ext == "so" || ext == "dylib" {
                        if self.loaded_paths.read().unwrap().contains(&p) {
                            continue;
                        }
                        self.load_plugin(&p, registry);
                    }
                }
            }
        }
    }

    /// Scan and load latest plugins; returns list of newly loaded file paths.
    pub fn scan_plugins_latest(&self, dir_path: &str, registry: &NodeRegistry) -> Vec<std::path::PathBuf> {
        let path = Path::new(dir_path);
        if !path.exists() {
            warn!("Plugin directory not found: {}", dir_path);
            return Vec::new();
        }
        let picks = Self::pick_latest_plugins(path);
        info!("Scanning latest plugins in: {} ({} candidates)", dir_path, picks.len());
        let mut loaded = Vec::new();
        for p in picks {
            if self.loaded_paths.read().unwrap().contains(&p) { continue; }
            self.load_plugin(&p, registry);
            loaded.push(p);
        }
        loaded
    }

    fn load_plugin(&self, path: &Path, registry: &NodeRegistry) {
        unsafe {
            let lib = Library::new(path);
            if let Err(e) = lib {
                error!("Failed to load plugin {:?}: {}", path, e);
                return;
            }
            let lib = Arc::new(lib.unwrap());

            // 1. Handshake: Get Plugin Info
            let info_fn: Result<Symbol<c_api::PluginInfoFn>, _> = lib.get(b"cunning_plugin_info");
            if let Ok(info_fn) = info_fn {
                let details = info_fn();
                if details.abi_version != c_api::CUNNING_PLUGIN_ABI_VERSION {
                    warn!(
                        "Skipping {:?}: ABI mismatch plugin={} host={}",
                        path,
                        details.abi_version,
                        c_api::CUNNING_PLUGIN_ABI_VERSION
                    );
                    return;
                }
                let name = std::str::from_utf8(std::slice::from_raw_parts(
                    details.name.ptr as *const u8,
                    details.name.len as usize,
                ))
                .unwrap_or("plugin")
                .to_string();
                let version = std::str::from_utf8(std::slice::from_raw_parts(
                    details.version.ptr as *const u8,
                    details.version.len as usize,
                ))
                .unwrap_or("0")
                .to_string();

                info!("Loaded Plugin: {} (v{})", name, version);

                let node_count_fn: Result<Symbol<c_api::PluginNodeCountFn>, _> =
                    lib.get(b"cunning_plugin_node_count");
                let node_desc_fn: Result<Symbol<c_api::PluginGetNodeDescFn>, _> =
                    lib.get(b"cunning_plugin_get_node_desc");
                let node_vt_fn: Result<Symbol<c_api::PluginGetNodeVTableFn>, _> =
                    lib.get(b"cunning_plugin_get_node_vtable");
                let node_ivt_fn: Option<Symbol<c_api::PluginGetNodeInteractionVTableFn>> =
                    lib.get(b"cunning_plugin_get_node_interaction_vtable").ok();
                let (Ok(node_count_fn), Ok(node_desc_fn), Ok(node_vt_fn)) =
                    (node_count_fn, node_desc_fn, node_vt_fn)
                else {
                    warn!("Skipping {:?}: Missing node exports", path);
                    return;
                };

                let n = node_count_fn();
                for i in 0..n {
                    let mut nd: c_api::CNodeDesc = std::mem::zeroed();
                    if node_desc_fn(i, &mut nd as *mut _) != 0 {
                        continue;
                    }
                    let vt = node_vt_fn(i);
                    let nname = std::str::from_utf8(std::slice::from_raw_parts(
                        nd.name.ptr as *const u8,
                        nd.name.len as usize,
                    ))
                    .unwrap_or(&name)
                    .to_string();
                    let cat = std::str::from_utf8(std::slice::from_raw_parts(
                        nd.category.ptr as *const u8,
                        nd.category.len as usize,
                    ))
                    .unwrap_or("External")
                    .to_string();
                    let inputs: Vec<String> =
                        std::slice::from_raw_parts(nd.inputs.ptr, nd.inputs.len as usize)
                            .iter()
                            .map(|s| {
                                std::str::from_utf8(std::slice::from_raw_parts(
                                    s.ptr as *const u8,
                                    s.len as usize,
                                ))
                                .unwrap_or("Input")
                                .to_string()
                            })
                            .collect();
                    let outputs: Vec<String> =
                        std::slice::from_raw_parts(nd.outputs.ptr, nd.outputs.len as usize)
                            .iter()
                            .map(|s| {
                                std::str::from_utf8(std::slice::from_raw_parts(
                                    s.ptr as *const u8,
                                    s.len as usize,
                                ))
                                .unwrap_or("Output")
                                .to_string()
                            })
                            .collect();

                    let params: Vec<crate::nodes::parameter::Parameter> =
                        std::slice::from_raw_parts(nd.params, nd.params_len as usize)
                            .iter()
                            .map(|p| {
                                let pn = std::str::from_utf8(std::slice::from_raw_parts(
                                    p.name.ptr as *const u8,
                                    p.name.len as usize,
                                ))
                                .unwrap_or("")
                                .to_string();
                                let pl = std::str::from_utf8(std::slice::from_raw_parts(
                                    p.label.ptr as *const u8,
                                    p.label.len as usize,
                                ))
                                .unwrap_or(&pn)
                                .to_string();
                                let pg = std::str::from_utf8(std::slice::from_raw_parts(
                                    p.group.ptr as *const u8,
                                    p.group.len as usize,
                                ))
                                .unwrap_or("General")
                                .to_string();
                                let dv = match p.default_value.tag {
                                    c_api::CParamTag::Int => {
                                        ParameterValue::Int(p.default_value.a as i64 as i32)
                                    }
                                    c_api::CParamTag::Float => ParameterValue::Float(
                                        f32::from_bits(p.default_value.a as u32),
                                    ),
                                    c_api::CParamTag::Bool => {
                                        ParameterValue::Bool(p.default_value.a != 0)
                                    }
                                    c_api::CParamTag::Vec2 => ParameterValue::Vec2(Vec2::new(
                                        f32::from_bits(p.default_value.a as u32),
                                        f32::from_bits((p.default_value.a >> 32) as u32),
                                    )),
                                    c_api::CParamTag::Vec3 => ParameterValue::Vec3(Vec3::new(
                                        f32::from_bits(p.default_value.a as u32),
                                        f32::from_bits((p.default_value.a >> 32) as u32),
                                        f32::from_bits(p.default_value.b as u32),
                                    )),
                                    c_api::CParamTag::Vec4 => ParameterValue::Vec4(Vec4::new(
                                        f32::from_bits(p.default_value.a as u32),
                                        f32::from_bits((p.default_value.a >> 32) as u32),
                                        f32::from_bits(p.default_value.b as u32),
                                        f32::from_bits((p.default_value.b >> 32) as u32),
                                    )),
                                    c_api::CParamTag::String => {
                                        let sp = p.default_value.a as usize as *const u8;
                                        let sl = p.default_value.b as usize;
                                        let s = if !sp.is_null() && sl > 0 {
                                            std::str::from_utf8(std::slice::from_raw_parts(sp, sl))
                                                .unwrap_or("")
                                        } else {
                                            ""
                                        };
                                        ParameterValue::String(s.to_string())
                                    }
                                    c_api::CParamTag::Color3 => ParameterValue::Color(Vec3::new(
                                        f32::from_bits(p.default_value.a as u32),
                                        f32::from_bits((p.default_value.a >> 32) as u32),
                                        f32::from_bits(p.default_value.b as u32),
                                    )),
                                    c_api::CParamTag::Color4 => ParameterValue::Color4(Vec4::new(
                                        f32::from_bits(p.default_value.a as u32),
                                        f32::from_bits((p.default_value.a >> 32) as u32),
                                        f32::from_bits(p.default_value.b as u32),
                                        f32::from_bits((p.default_value.b >> 32) as u32),
                                    )),
                                    c_api::CParamTag::Curve => {
                                        ParameterValue::Curve(Default::default())
                                    }
                                };
                                let ui = match p.ui.tag {
                                    c_api::CParamUiTag::FloatSlider => {
                                        ParameterUIType::FloatSlider {
                                            min: f32::from_bits(p.ui.a as u32),
                                            max: f32::from_bits(p.ui.b as u32),
                                        }
                                    }
                                    c_api::CParamUiTag::IntSlider => ParameterUIType::IntSlider {
                                        min: p.ui.a as i64 as i32,
                                        max: p.ui.b as i64 as i32,
                                    },
                                    c_api::CParamUiTag::Vec2Drag => ParameterUIType::Vec2Drag,
                                    c_api::CParamUiTag::Vec3Drag => ParameterUIType::Vec3Drag,
                                    c_api::CParamUiTag::Vec4Drag => ParameterUIType::Vec4Drag,
                                    c_api::CParamUiTag::String => ParameterUIType::String,
                                    c_api::CParamUiTag::Toggle => ParameterUIType::Toggle,
                                    c_api::CParamUiTag::Color => ParameterUIType::Color {
                                        show_alpha: p.ui.a != 0,
                                    },
                                    c_api::CParamUiTag::Code => ParameterUIType::Code,
                                    c_api::CParamUiTag::CurvePoints => ParameterUIType::CurvePoints,
                                    _ => ParameterUIType::String,
                                };
                                crate::nodes::parameter::Parameter::new(&pn, &pl, &pg, dv, ui)
                            })
                            .collect();

                    let params_c = params.clone();
                    let lib_clone = lib.clone();
                    let vt_c = vt;
                    let interaction_factory: Option<crate::cunning_core::registries::node_registry::RuntimeNodeInteractionFactory> = node_ivt_fn.as_ref().and_then(|ivt_fn| {
                        // NOTE: `CNodeInteractionVTable` contains function pointers, which are NOT valid when zero-initialized.
                        // Provide no-op defaults, then let plugin overwrite them.
                        extern "C" fn hud_build_noop(_inst: *mut c_void, _host: *const c_api::CHostApi, _node: c_api::CUuid, _out: *mut c_api::CHudCmd, _cap: u32) -> u32 { 0 }
                        extern "C" fn hud_event_noop(_inst: *mut c_void, _host: *const c_api::CHostApi, _node: c_api::CUuid, _e: *const c_api::CHudEvent) -> i32 { 0 }
                        extern "C" fn gizmo_build_noop(_inst: *mut c_void, _host: *const c_api::CHostApi, _node: c_api::CUuid, _out: *mut c_api::CGizmoCmd, _cap: u32) -> u32 { 0 }
                        extern "C" fn gizmo_event_noop(_inst: *mut c_void, _host: *const c_api::CHostApi, _node: c_api::CUuid, _e: *const c_api::CGizmoEvent) -> i32 { -1 }
                        extern "C" fn input_event_noop(_inst: *mut c_void, _host: *const c_api::CHostApi, _node: c_api::CUuid, _e: *const c_api::CInputEvent) -> i32 { 0 }
                        let mut ivt: c_api::CNodeInteractionVTable = c_api::CNodeInteractionVTable {
                            hud_build: hud_build_noop,
                            hud_event: hud_event_noop,
                            gizmo_build: gizmo_build_noop,
                            gizmo_event: gizmo_event_noop,
                            input_event: input_event_noop,
                        };
                        if ivt_fn(i, &mut ivt as *mut _) != 0 { return None; }
                        let inst = (vt.create)();
                        let shared = Arc::new(PluginInteractionShared {
                            _library: lib.clone(),
                            node_vtable: vt,
                            interaction_vtable: ivt,
                            instance: inst,
                            node_name: nname.clone(),
                            node_state: self.node_state.clone(),
                        });
                        self.interactions.write().unwrap().insert(nname.clone(), shared.clone());
                        let f: crate::cunning_core::registries::node_registry::RuntimeNodeInteractionFactory =
                            Arc::new(move || -> Box<dyn crate::cunning_core::traits::node_interface::NodeInteraction> { Box::new(PluginInteractionNode { shared: shared.clone() }) });
                        Some(f)
                    });
                    registry.register_dynamic(RuntimeNodeDescriptor {
                        name: nname.clone(),
                        display_name: format!("{} (Plugin)", nname),
                        display_name_lc: format!("{} (plugin)", nname.to_lowercase()),
                        category: cat,
                        op_factory: Arc::new(move || {
                            let instance = (vt_c.create)();
                            Box::new(NativeNodeWrapper { _library: lib_clone.clone(), vtable: vt_c, instance })
                        }),
                        interaction_factory,
                        coverlay_kinds: Vec::new(),
                        parameters_factory: Arc::new(move || params_c.clone()),
                        inputs,
                        outputs,
                        input_style: match nd.input_style { c_api::CInputStyle::Multi => crate::cunning_core::registries::node_registry::InputStyle::Multi, c_api::CInputStyle::NamedPorts => crate::cunning_core::registries::node_registry::InputStyle::NamedPorts, _ => crate::cunning_core::registries::node_registry::InputStyle::Single },
                        node_style: match nd.node_style { c_api::CNodeStyle::Large => crate::cunning_core::registries::node_registry::NodeStyle::Large, _ => crate::cunning_core::registries::node_registry::NodeStyle::Normal },
                        origin: crate::cunning_core::registries::node_registry::NodeOrigin::Plugin,
                    });
                }

                self.loaded_libraries.write().unwrap().push(lib);
                self.loaded_paths
                    .write()
                    .unwrap()
                    .insert(path.to_path_buf());
            } else {
                warn!("Skipping {:?}: Missing 'cunning_plugin_info' symbol", path);
            }
        }
    }
}

pub fn auto_reload_latest_plugins_system(
    time: Res<Time>,
    ps: Res<PluginSystem>,
    reg: Res<NodeRegistry>,
    mut node_graph_res: Option<ResMut<crate::NodeGraphResource>>,
    mut acc: Local<f32>,
    mut last_fp: Local<u64>,
) {
    *acc += time.delta_secs();
    if *acc < 0.75 {
        return;
    }
    *acc = 0.0;
    let picks = PluginSystem::pick_latest_plugins(std::path::Path::new("plugins"));
    let mut fp = 1469598103934665603u64 ^ (picks.len() as u64);
    for p in &picks {
        fp ^= p
            .to_string_lossy()
            .as_bytes()
            .iter()
            .fold(0u64, |a, &b| a.wrapping_mul(1099511628211) ^ (b as u64));
        fp = fp.wrapping_mul(1099511628211);
    }
    if *last_fp == fp {
        return;
    }
    *last_fp = fp;
    ps.scan_plugins_latest("plugins", &*reg);
    let Some(node_graph_res) = node_graph_res.as_deref_mut() else {
        return;
    };
    // Repair: nodes created before plugin load (or via paths that didn't apply descriptor) miss plugin params.
    // We only add missing params; never overwrite existing values.
    let g = &mut node_graph_res.0;
    let mut dirty: Vec<crate::nodes::NodeId> = Vec::new();
    for n in g.nodes.values_mut() {
        let crate::nodes::structs::NodeType::Generic(key) = &n.node_type else {
            continue;
        };
        let Some(desc) = reg.nodes.read().unwrap().get(key).cloned() else {
            continue;
        };
        let defaults = (desc.parameters_factory)();
        if defaults.is_empty() {
            continue;
        }
        let mut changed = false;
        for p in defaults {
            if n.parameters.iter().any(|x| x.name == p.name) {
                continue;
            }
            n.parameters.push(p);
            changed = true;
        }
        if changed {
            dirty.push(n.id);
        }
    }
    for id in dirty {
        g.mark_dirty(id);
    }
}

/// Global hot-reload shortcut: Ctrl+Alt+R opens hot-reload window + forces plugin rescan.
pub fn hot_reload_shortcut_system(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    ps: Option<Res<PluginSystem>>,
    reg: Option<Res<NodeRegistry>>,
    console: Option<Res<crate::console::ConsoleLog>>,
    hot_log: Option<Res<crate::tabs_system::pane::hot_reload::HotReloadLog>>,
    mut open_hr: MessageWriter<crate::ui::OpenHotReloadWindowEvent>,
    mut node_graph_res: Option<ResMut<crate::NodeGraphResource>>,
    mut scan_task: Local<Option<Task<Vec<std::path::PathBuf>>>>,
) {
    // Poll async scan completion (keeps UI responsive; avoids "freeze on hot reload shortcut").
    if let Some(tk) = scan_task.as_mut() {
        if let Some(loaded) = future::block_on(future::poll_once(tk)) {
            *scan_task = None;
            let t = time.elapsed_secs();
            if loaded.is_empty() {
                if let Some(hl) = hot_log.as_deref() { hl.info("No hot reload file update.", t); }
                if let Some(c) = console.as_deref() { c.info("Hot Reload: no new plugin files detected."); }
            } else {
                for p in &loaded {
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    if let Some(hl) = hot_log.as_deref() { hl.info(format!("Loaded plugin: {name}"), t); }
                    if let Some(c) = console.as_deref() { c.info(format!("Hot Reload: loaded {name}")); }
                }
                if let Some(hl) = hot_log.as_deref() { hl.info(format!("{} plugin(s) reloaded.", loaded.len()), t); }
            }
            // Repair nodes missing plugin params after scan.
            if let (Some(reg), Some(g)) = (reg.as_deref(), node_graph_res.as_deref_mut()) {
                let map = reg.nodes.read().unwrap();
                let gg = &mut g.0;
                let mut repaired = 0u32;
                for n in gg.nodes.values_mut() {
                    let crate::nodes::structs::NodeType::Generic(key) = &n.node_type else { continue };
                    let Some(desc) = map.get(key) else { continue };
                    let defaults = (desc.parameters_factory)();
                    for p in defaults {
                        if !n.parameters.iter().any(|x| x.name == p.name) {
                            n.parameters.push(p);
                            repaired += 1;
                        }
                    }
                }
                if repaired > 0 {
                    if let Some(hl) = hot_log.as_deref() { hl.info(format!("Repaired {repaired} missing plugin params."), t); }
                }
            }
            if let Some(hl) = hot_log.as_deref() { hl.info("Hot Reload complete.", t); }
        }
    }

    if !(keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight)) { return; }
    if !(keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight)) { return; }
    if !keys.just_pressed(KeyCode::KeyR) { return; }
    let t = time.elapsed_secs();
    // Open the hot-reload popup window
    open_hr.write_default();
    if let Some(c) = console.as_deref() { c.info("Hot Reload (Ctrl+Alt+R): scanning plugins + assets..."); }
    if let Some(hl) = hot_log.as_deref() { hl.info("Hot Reload triggered (Ctrl+Alt+R)", t); }
    info!("Hot Reload triggered (Ctrl+Alt+R)");
    if scan_task.is_some() {
        if let Some(hl) = hot_log.as_deref() { hl.warn("Hot Reload scan already running...", t); }
        return;
    }
    if let (Some(ps), Some(reg)) = (ps.as_deref(), reg.as_deref()) {
        let ps = ps.clone();
        let reg = reg.clone();
        *scan_task = Some(IoTaskPool::get().spawn(async move { ps.scan_plugins_latest("plugins", &reg) }));
    }
}

pub fn ensure_curve_reference_plugin_system(reg: Option<Res<NodeRegistry>>, mut fired: Local<bool>) {
    if *fired { return; }
    let Some(reg) = reg else { return; };
    if reg.get_descriptor("Curve").is_some() { *fired = true; return; }
    if !Path::new("plugins/extra_node/curve_plugin/Cargo.toml").exists() { *fired = true; return; }
    let has_dll = std::fs::read_dir("plugins").ok().into_iter().flatten().flatten().any(|e| {
        e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("dll")) && e.file_name().to_string_lossy().starts_with("curve_plugin")
    });
    if has_dll { *fired = true; return; }
    match request_compile_rust_plugin(CompileRustPluginRequest::for_extra_node("curve_plugin")) {
        Ok(()) => info!("Curve plugin missing; compiling plugins/extra_node/curve_plugin (will hot-load)."),
        Err(e) => warn!("Curve plugin compile request failed: {e}"),
    }
    *fired = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cunning_core::registries::node_registry::NodeRegistry;
    use crate::nodes::parameter::{
        CurveControlPoint, CurveData, CurveType, Parameter, ParameterUIType, ParameterValue,
        PointMode,
    };
    use std::path::Path;

    #[test]
    fn smoke_curve_plugin_compute_polygon() {
        // This test is strict, but only runs if a curve plugin DLL is present.
        if !Path::new("plugins").exists() {
            return;
        }

        let registry = NodeRegistry::default();
        registry.scan_and_load();

        let ps = PluginSystem::default();
        ps.scan_plugins_latest("plugins", &registry);

        let desc = registry.get_descriptor("Curve");
        if desc.is_none() {
            return;
        } // plugin not present -> nothing to assert in CI

        let op = registry.create_op("Curve").expect("Curve op");

        let mut p0 = CurveControlPoint::new(Vec3::new(0.0, 0.0, 0.0));
        p0.mode = PointMode::Corner;
        let mut p1 = CurveControlPoint::new(Vec3::new(1.0, 0.0, 0.0));
        p1.mode = PointMode::Corner;
        let curve = CurveData {
            points: vec![p0, p1],
            is_closed: false,
            curve_type: CurveType::Polygon,
        };

        let params = vec![Parameter::new(
            "curve_data",
            "Curve Data",
            "Geometry",
            ParameterValue::Curve(curve),
            ParameterUIType::CurvePoints,
        )];
        let out = op.compute(&params, &[]);

        assert_eq!(out.get_point_count(), 2);
        assert!(out.get_primitive_count() >= 1);
    }
}
