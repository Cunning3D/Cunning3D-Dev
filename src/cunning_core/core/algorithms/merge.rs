use crate::libs::geometry::mesh::{Attribute, Geometry, AttributeHandle, GeoPrimitive, PolygonPrim, PolylinePrim, BezierCurvePrim, Bytes};
use crate::libs::geometry::group::ElementGroupMask;
use crate::libs::geometry::ids::{AttributeId, VertexId, PointId};
use crate::libs::algorithms::algorithms_dcc::PagedBuffer;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use bevy::math::{Vec2, Vec3, Vec4, DVec2, DVec3, DVec4};
use std::sync::Arc;

#[derive(Default)]
struct MergeScratch {
    point_groups: HashSet<AttributeId>,
    prim_groups: HashSet<AttributeId>,
    local_vid_map: HashMap<VertexId, VertexId>,
}

thread_local! { static MERGE_SCRATCH: RefCell<MergeScratch> = RefCell::new(MergeScratch::default()); }

fn init_attr_maps(target: &mut HashMap<AttributeId, AttributeHandle>, sources: &[&HashMap<AttributeId, AttributeHandle>], cap: usize) {
    for src in sources {
        'attrs: for (id, h) in *src {
            if target.contains_key(id) { continue; }
            let a = &*h.data;
            macro_rules! ins { ($t:ty) => { if a.as_storage::<Vec<$t>>().is_some() || a.as_storage::<PagedBuffer<$t>>().is_some() { target.insert(*id, AttributeHandle::new(Attribute::new(Vec::<$t>::with_capacity(cap)))); continue 'attrs; } }; }
            ins!(f32); ins!(Vec2); ins!(Vec3); ins!(Vec4); ins!(f64); ins!(DVec2); ins!(DVec3); ins!(DVec4); ins!(i32); ins!(bool); ins!(String);
            if a.as_storage::<Bytes>().is_some() { target.insert(*id, AttributeHandle::new(Attribute::new(Bytes(Vec::with_capacity(cap))))); }
        }
    }
}

fn init_attr_maps_iter<'a, I>(target: &mut HashMap<AttributeId, AttributeHandle>, sources: I, cap: usize)
where
    I: IntoIterator<Item = &'a HashMap<AttributeId, AttributeHandle>>,
{
    for src in sources {
        'attrs: for (id, h) in src {
            if target.contains_key(id) { continue; }
            let a = &*h.data;
            macro_rules! ins { ($t:ty) => { if a.as_storage::<Vec<$t>>().is_some() || a.as_storage::<PagedBuffer<$t>>().is_some() { target.insert(*id, AttributeHandle::new(Attribute::new(Vec::<$t>::with_capacity(cap)))); continue 'attrs; } }; }
            ins!(f32); ins!(Vec2); ins!(Vec3); ins!(Vec4); ins!(f64); ins!(DVec2); ins!(DVec3); ins!(DVec4); ins!(i32); ins!(bool); ins!(String);
            if a.as_storage::<Bytes>().is_some() { target.insert(*id, AttributeHandle::new(Attribute::new(Bytes(Vec::with_capacity(cap))))); }
        }
    }
}

fn copy_attr_maps(dst: &mut HashMap<AttributeId, AttributeHandle>, src: &HashMap<AttributeId, AttributeHandle>, off: usize, len: usize) {
    if len == 0 { return; }
    for (id, h) in src {
        let a = &*h.data;
        macro_rules! c {
            ($t:ty) => {
                if let Some(s) = a.as_storage::<Vec<$t>>() {
                    if let Some(d) = dst.get_mut(id).and_then(|h| h.get_mut().as_storage_mut::<Vec<$t>>()) {
                        for i in 0..len.min(s.len()) { d[off + i] = s[i].clone(); }
                    }
                    continue;
                }
                if let Some(s) = a.as_storage::<PagedBuffer<$t>>() {
                    if let Some(d) = dst.get_mut(id).and_then(|h| h.get_mut().as_storage_mut::<Vec<$t>>()) {
                        for i in 0..len.min(s.len()) { if let Some(v) = s.get(i) { d[off + i] = v; } }
                    }
                    continue;
                }
            };
        }
        c!(f32); c!(Vec2); c!(Vec3); c!(Vec4); c!(f64); c!(DVec2); c!(DVec3); c!(DVec4); c!(i32); c!(bool); c!(String);
        if let Some(s) = a.as_storage::<Bytes>() {
            if let Some(d) = dst.get_mut(id).and_then(|h| h.get_mut().as_storage_mut::<Bytes>()) {
                for i in 0..len.min(s.0.len()) { d.0[off + i] = s.0[i]; }
            }
        }
    }
}

fn merge_geometry_impl<'a>(len: usize, get: impl Fn(usize) -> &'a Geometry) -> Geometry {
    if len == 0 { return Geometry::new(); }
    MERGE_SCRATCH.with(|s| {
        let s = &mut *s.borrow_mut();
        let mut merged_geo = Geometry::new();
        let mut total_points = 0usize;
        let mut total_vertices = 0usize;
        let mut total_primitives = 0usize;
        s.point_groups.clear();
        s.prim_groups.clear();
        for i in 0..len {
            let input = get(i);
            total_points += input.get_point_count();
            total_vertices += input.vertices().len();
            total_primitives += input.primitives().len();
            for name in input.point_groups.keys() { s.point_groups.insert(*name); }
            for name in input.primitive_groups.keys() { s.prim_groups.insert(*name); }
        }
        for name in s.point_groups.iter().copied() { merged_geo.point_groups.insert(name, ElementGroupMask::new(0)); }
        for name in s.prim_groups.iter().copied() { merged_geo.primitive_groups.insert(name, ElementGroupMask::new(0)); }
        init_attr_maps_iter(&mut merged_geo.point_attributes, (0..len).map(|i| &get(i).point_attributes), total_points);
        init_attr_maps_iter(&mut merged_geo.vertex_attributes, (0..len).map(|i| &get(i).vertex_attributes), total_vertices);
        init_attr_maps_iter(&mut merged_geo.primitive_attributes, (0..len).map(|i| &get(i).primitive_attributes), total_primitives);
        merged_geo.points_mut().reserve_additional(total_points);
        merged_geo.vertices_mut().reserve_additional(total_vertices);
        merged_geo.primitives_mut().reserve_additional(total_primitives);
        merged_geo.add_points_batch(total_points);
        let mut point_offset = 0;
        let mut vertex_offset = 0;
        let mut prim_offset = 0;
        for i in 0..len {
            let input_geo = get(i);
            if merged_geo.detail_attributes.is_empty() { merged_geo.detail_attributes = input_geo.detail_attributes.clone(); }
            let p_len = input_geo.get_point_count();
            copy_attr_maps(&mut merged_geo.point_attributes, &input_geo.point_attributes, point_offset, p_len);
            for (name, mask) in &mut merged_geo.point_groups {
                if let Some(input_mask) = input_geo.point_groups.get(name) { for idx in input_mask.iter_ones() { mask.set(point_offset + idx, true); } }
            }
            s.local_vid_map.clear();
            for (old_vid_idx, old_v) in input_geo.vertices().iter_enumerated() {
                let old_pid = old_v.point_id;
                if let Some(old_p_dense) = input_geo.points().get_dense_index(old_pid.into()) {
                    let new_p_dense = point_offset + old_p_dense;
                    if let Some(new_pid) = merged_geo.points().get_id_from_dense(new_p_dense) {
                        let new_vid = merged_geo.add_vertex_no_invalidate(PointId::from(new_pid));
                        let old_vid: VertexId = old_vid_idx.into();
                        s.local_vid_map.insert(old_vid, new_vid);
                    }
                }
            }
            let v_len = merged_geo.vertices().len().saturating_sub(vertex_offset);
            copy_attr_maps(&mut merged_geo.vertex_attributes, &input_geo.vertex_attributes, vertex_offset, v_len);
            for prim in input_geo.primitives().values() {
                match prim {
                    GeoPrimitive::Polygon(p) => {
                        let new_verts = p.vertices.iter().filter_map(|v| s.local_vid_map.get(v).copied()).collect();
                        merged_geo.add_primitive_no_invalidate(GeoPrimitive::Polygon(PolygonPrim { vertices: new_verts }));
                    }
                    GeoPrimitive::Polyline(p) => {
                        let new_verts = p.vertices.iter().filter_map(|v| s.local_vid_map.get(v).copied()).collect();
                        merged_geo.add_primitive_no_invalidate(GeoPrimitive::Polyline(PolylinePrim { vertices: new_verts, closed: p.closed }));
                    }
                    GeoPrimitive::BezierCurve(p) => {
                        let new_verts = p.vertices.iter().filter_map(|v| s.local_vid_map.get(v).copied()).collect();
                        merged_geo.add_primitive_no_invalidate(GeoPrimitive::BezierCurve(BezierCurvePrim { vertices: new_verts, closed: p.closed }));
                    }
                    _ => {}
                }
            }
            let pr_len = merged_geo.primitives().len().saturating_sub(prim_offset);
            copy_attr_maps(&mut merged_geo.primitive_attributes, &input_geo.primitive_attributes, prim_offset, pr_len);
            for (name, mask) in &mut merged_geo.primitive_groups {
                if let Some(input_mask) = input_geo.primitive_groups.get(name) { for idx in input_mask.iter_ones() { mask.set(prim_offset + idx, true); } }
            }
            merged_geo.sdfs.extend(input_geo.sdfs.iter().cloned());
            point_offset += p_len;
            vertex_offset += v_len;
            prim_offset += pr_len;
        }
        merged_geo
    })
}

pub fn merge_geometry(inputs: Vec<&Geometry>) -> Geometry { merge_geometry_impl(inputs.len(), |i| inputs[i]) }

#[inline]
pub fn merge_geometry_arcs(inputs: &[Arc<Geometry>]) -> Geometry { merge_geometry_impl(inputs.len(), |i| inputs[i].as_ref()) }

#[inline]
pub fn merge_geometry_slice(inputs: &[Geometry]) -> Geometry { merge_geometry_impl(inputs.len(), |i| &inputs[i]) }

#[inline]
pub fn binary_merge(a: &Geometry, b: &Geometry) -> Geometry {
    MERGE_SCRATCH.with(|s| {
        let s = &mut *s.borrow_mut();
        if a.get_point_count() == 0 && a.primitives().is_empty() { return b.clone(); }
        if b.get_point_count() == 0 && b.primitives().is_empty() { return a.clone(); }

        let mut merged_geo = Geometry::new();
        let inputs: [&Geometry; 2] = [a, b];
        let total_points = a.get_point_count() + b.get_point_count();
        let total_vertices = a.vertices().len() + b.vertices().len();
        let total_primitives = a.primitives().len() + b.primitives().len();

        s.point_groups.clear();
        s.prim_groups.clear();
        for input in &inputs {
            for name in input.point_groups.keys() { s.point_groups.insert(*name); }
            for name in input.primitive_groups.keys() { s.prim_groups.insert(*name); }
        }
        for name in s.point_groups.iter().copied() { merged_geo.point_groups.insert(name, ElementGroupMask::new(0)); }
        for name in s.prim_groups.iter().copied() { merged_geo.primitive_groups.insert(name, ElementGroupMask::new(0)); }

        let psrc: [&HashMap<AttributeId, AttributeHandle>; 2] = [&a.point_attributes, &b.point_attributes];
        let vsrc: [&HashMap<AttributeId, AttributeHandle>; 2] = [&a.vertex_attributes, &b.vertex_attributes];
        let prsrc: [&HashMap<AttributeId, AttributeHandle>; 2] = [&a.primitive_attributes, &b.primitive_attributes];
        init_attr_maps(&mut merged_geo.point_attributes, &psrc, total_points);
        init_attr_maps(&mut merged_geo.vertex_attributes, &vsrc, total_vertices);
        init_attr_maps(&mut merged_geo.primitive_attributes, &prsrc, total_primitives);
        merged_geo.points_mut().reserve_additional(total_points);
        merged_geo.vertices_mut().reserve_additional(total_vertices);
        merged_geo.primitives_mut().reserve_additional(total_primitives);
        merged_geo.add_points_batch(total_points);

        let mut point_offset = 0;
        let mut vertex_offset = 0;
        let mut prim_offset = 0;
        for input_geo in inputs {
            if merged_geo.detail_attributes.is_empty() { merged_geo.detail_attributes = input_geo.detail_attributes.clone(); }
            let p_len = input_geo.get_point_count();
            copy_attr_maps(&mut merged_geo.point_attributes, &input_geo.point_attributes, point_offset, p_len);
            for (name, mask) in &mut merged_geo.point_groups {
                if let Some(input_mask) = input_geo.point_groups.get(name) { for idx in input_mask.iter_ones() { mask.set(point_offset + idx, true); } }
            }
            s.local_vid_map.clear();
            for (old_vid_idx, old_v) in input_geo.vertices().iter_enumerated() {
                let old_pid = old_v.point_id;
                if let Some(old_p_dense) = input_geo.points().get_dense_index(old_pid.into()) {
                    let new_p_dense = point_offset + old_p_dense;
                    if let Some(new_pid) = merged_geo.points().get_id_from_dense(new_p_dense) {
                        let new_vid = merged_geo.add_vertex_no_invalidate(PointId::from(new_pid));
                        let old_vid: VertexId = old_vid_idx.into();
                        s.local_vid_map.insert(old_vid, new_vid);
                    }
                }
            }
            copy_attr_maps(&mut merged_geo.vertex_attributes, &input_geo.vertex_attributes, vertex_offset, input_geo.vertices().len());
            for (name, mask) in &mut merged_geo.primitive_groups {
                if let Some(input_mask) = input_geo.primitive_groups.get(name) { for idx in input_mask.iter_ones() { mask.set(prim_offset + idx, true); } }
            }
            for (old_prim_idx, prim) in input_geo.primitives().iter_enumerated() {
                let new_prim = match prim {
                    GeoPrimitive::Polygon(p) => {
                        let mut new_poly = PolygonPrim { vertices: Vec::new() };
                        for v in &p.vertices { if let Some(new_vid) = s.local_vid_map.get(v) { new_poly.vertices.push(*new_vid); } }
                        GeoPrimitive::Polygon(new_poly)
                    }
                    GeoPrimitive::Polyline(p) => {
                        let mut new_poly = PolylinePrim { vertices: Vec::new(), closed: p.closed };
                        for v in &p.vertices { if let Some(new_vid) = s.local_vid_map.get(v) { new_poly.vertices.push(*new_vid); } }
                        GeoPrimitive::Polyline(new_poly)
                    }
                    GeoPrimitive::BezierCurve(p) => {
                        let mut new_curve = BezierCurvePrim { vertices: Vec::new(), closed: p.closed };
                        for v in &p.vertices { if let Some(new_vid) = s.local_vid_map.get(v) { new_curve.vertices.push(*new_vid); } }
                        GeoPrimitive::BezierCurve(new_curve)
                    }
                };
                merged_geo.add_primitive_no_invalidate(new_prim);
                let _ = old_prim_idx;
            }
            copy_attr_maps(&mut merged_geo.primitive_attributes, &input_geo.primitive_attributes, prim_offset, input_geo.primitives().len());
            point_offset += p_len;
            vertex_offset += input_geo.vertices().len();
            prim_offset += input_geo.primitives().len();
        }
        merged_geo
    })
}
