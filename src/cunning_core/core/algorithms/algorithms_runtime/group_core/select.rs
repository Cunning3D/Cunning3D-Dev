use bevy::prelude::*;
use crate::libs::geometry::attrs;
use crate::libs::geometry::group::ElementGroupMask;
use crate::libs::geometry::ids::VertexId;
use crate::libs::geometry::mesh::Geometry;
use crate::libs::geometry::topology::Topology;

pub fn keep_by_bounding_box(geo: &Geometry, min: Vec3, max: Vec3, mask: &mut ElementGroupMask) {
    if let Some(pos) = geo.get_point_position_attribute() {
        for i in mask.ones_vec() {
            let keep = pos.get(i).map(|p| p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y && p.z >= min.z && p.z <= max.z).unwrap_or(false);
            if !keep { mask.set(i, false); }
        }
    }
}

pub fn keep_by_bounding_sphere(geo: &Geometry, center: Vec3, radius: f32, mask: &mut ElementGroupMask) {
    let r2 = radius * radius;
    if let Some(pos) = geo.get_point_position_attribute() {
        for i in mask.ones_vec() {
            let keep = pos.get(i).map(|p| p.distance_squared(center) <= r2).unwrap_or(false);
            if !keep { mask.set(i, false); }
        }
    }
}

pub fn keep_by_normal(geo: &Geometry, direction: Vec3, angle_degrees: f32, mask: &mut ElementGroupMask) {
    let threshold = angle_degrees.to_radians().cos();
    let dir = direction.normalize_or_zero();
    let Some(normals) = geo.get_point_attribute(attrs::N).and_then(|a| a.as_slice::<Vec3>()) else { return; };
    for i in mask.ones_vec() {
        if normals.get(i).map(|n| n.dot(dir) >= threshold).unwrap_or(false) == false { mask.set(i, false); }
    }
}

pub fn keep_boundary_points(geo: &Geometry, topo: &Topology, mask: &mut ElementGroupMask) {
    let mut boundary = ElementGroupMask::new(mask.len());
    for &he in topo.get_boundary_edges() {
        let Some(h) = topo.half_edges.get(he.into()) else { continue; };
        let p1 = h.origin_point;
        let p2 = topo.dest_point(he);
        if let Some(i) = geo.points().get_dense_index(p1.into()) { boundary.set(i, true); }
        if let Some(i) = geo.points().get_dense_index(p2.into()) { boundary.set(i, true); }
    }
    mask.intersect_with(&boundary);
}

pub fn keep_primitives_by_bounding_box(geo: &Geometry, min: Vec3, max: Vec3, mask: &mut ElementGroupMask) {
    let Some(pos) = geo.get_point_position_attribute() else { return; };
    for prim_idx in mask.ones_vec() {
        let keep = (|| {
            let prim = geo.primitives().values().get(prim_idx)?;
            let mut c = Vec3::ZERO; let mut n = 0f32;
            for &vid in prim.vertices() {
                let v = geo.vertices().get(vid.into())?;
                let pi = geo.points().get_dense_index(v.point_id.into())?;
                c += *pos.get(pi)?; n += 1.0;
            }
            if n <= 0.0 { return Some(false); }
            c /= n;
            Some(c.x >= min.x && c.x <= max.x && c.y >= min.y && c.y <= max.y && c.z >= min.z && c.z <= max.z)
        })().unwrap_or(false);
        if !keep { mask.set(prim_idx, false); }
    }
}

pub fn keep_primitives_by_bounding_sphere(geo: &Geometry, center: Vec3, radius: f32, mask: &mut ElementGroupMask) {
    let Some(pos) = geo.get_point_position_attribute() else { return; };
    let r2 = radius * radius;
    for prim_idx in mask.ones_vec() {
        let keep = (|| {
            let prim = geo.primitives().values().get(prim_idx)?;
            let mut c = Vec3::ZERO; let mut n = 0f32;
            for &vid in prim.vertices() {
                let v = geo.vertices().get(vid.into())?;
                let pi = geo.points().get_dense_index(v.point_id.into())?;
                c += *pos.get(pi)?; n += 1.0;
            }
            if n <= 0.0 { return Some(false); }
            c /= n;
            Some(c.distance_squared(center) <= r2)
        })().unwrap_or(false);
        if !keep { mask.set(prim_idx, false); }
    }
}

pub fn keep_primitives_by_normal(geo: &Geometry, direction: Vec3, angle_degrees: f32, mask: &mut ElementGroupMask) {
    let threshold = angle_degrees.to_radians().cos();
    let dir = direction.normalize_or_zero();
    if let Some(normals) = geo.get_primitive_attribute(attrs::N).and_then(|a| a.as_slice::<Vec3>()) {
        for i in mask.ones_vec() {
            if normals.get(i).map(|n| n.dot(dir) >= threshold).unwrap_or(false) == false { mask.set(i, false); }
        }
        return;
    }
    let Some(pos) = geo.get_point_position_attribute() else { return; };
    for prim_idx in mask.ones_vec() {
        let keep = (|| {
            let prim = geo.primitives().values().get(prim_idx)?;
            let vids = prim.vertices();
            if vids.len() < 3 { return Some(false); }
            let getp = |vid: VertexId| {
                let v = geo.vertices().get(vid.into())?;
                let pi = geo.points().get_dense_index(v.point_id.into())?;
                Some(*pos.get(pi)?)
            };
            let (p0, p1, p2) = (getp(vids[0])?, getp(vids[1])?, getp(vids[2])?);
            Some((p1 - p0).cross(p2 - p0).normalize_or_zero().dot(dir) >= threshold)
        })().unwrap_or(false);
        if !keep { mask.set(prim_idx, false); }
    }
}

