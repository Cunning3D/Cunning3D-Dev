use crate::libs::geometry::mesh::{Geometry, Attribute};
use bevy::math::{Vec2, Vec3, Vec4};

/// Interpolates all point attributes between p0 and p1 with factor t, 
/// appending the new values to the attribute buffers.
/// Returns nothing (values are pushed).
pub fn interpolate_point_attributes(geo: &mut Geometry, p0_idx: usize, p1_idx: usize, t: f32) {
    // Iterate all point attributes
    // We need to collect keys first to avoid borrowing issues
    let keys: Vec<_> = geo.point_attributes.keys().copied().collect();
    
    for key in keys {
        if let Some(attr_handle) = geo.point_attributes.get_mut(&key) {
            let attr = attr_handle.get_mut();
            
            if let Some(buf) = attr.as_storage_mut::<Vec<f32>>() {
                let v0 = buf.get(p0_idx).copied().unwrap_or(0.0);
                let v1 = buf.get(p1_idx).copied().unwrap_or(0.0);
                buf.push(v0 + (v1 - v0) * t);
                continue;
            }
            
            if let Some(buf) = attr.as_storage_mut::<Vec<Vec2>>() {
                let v0 = buf.get(p0_idx).copied().unwrap_or(Vec2::ZERO);
                let v1 = buf.get(p1_idx).copied().unwrap_or(Vec2::ZERO);
                buf.push(v0.lerp(v1, t));
                continue;
            }

            if let Some(buf) = attr.as_storage_mut::<Vec<Vec3>>() {
                let v0 = buf.get(p0_idx).copied().unwrap_or(Vec3::ZERO);
                let v1 = buf.get(p1_idx).copied().unwrap_or(Vec3::ZERO);
                buf.push(v0.lerp(v1, t));
                continue;
            }

            if let Some(buf) = attr.as_storage_mut::<Vec<Vec4>>() {
                let v0 = buf.get(p0_idx).copied().unwrap_or(Vec4::ZERO);
                let v1 = buf.get(p1_idx).copied().unwrap_or(Vec4::ZERO);
                buf.push(v0.lerp(v1, t));
                continue;
            }

            if let Some(buf) = attr.as_storage_mut::<Vec<i32>>() {
                let v0 = buf.get(p0_idx).copied().unwrap_or(0);
                buf.push(v0);
                continue;
            }
        }
    }
}

/// Interpolates all vertex attributes for a newly created vertex.
/// v0_idx, v1_idx are the indices in the `vertices` array, not point indices.
pub fn interpolate_vertex_attributes(geo: &mut Geometry, v0_idx: usize, v1_idx: usize, t: f32) {
     let keys: Vec<_> = geo.vertex_attributes.keys().copied().collect();
    
    for key in keys {
        if let Some(attr_handle) = geo.vertex_attributes.get_mut(&key) {
             let attr = attr_handle.get_mut();
            
            if let Some(buf) = attr.as_storage_mut::<Vec<f32>>() {
                let v0 = buf.get(v0_idx).copied().unwrap_or(0.0);
                let v1 = buf.get(v1_idx).copied().unwrap_or(0.0);
                buf.push(v0 + (v1 - v0) * t);
                continue;
            }

            if let Some(buf) = attr.as_storage_mut::<Vec<Vec2>>() {
                let v0 = buf.get(v0_idx).copied().unwrap_or(Vec2::ZERO);
                let v1 = buf.get(v1_idx).copied().unwrap_or(Vec2::ZERO);
                buf.push(v0.lerp(v1, t));
                continue;
            }

            if let Some(buf) = attr.as_storage_mut::<Vec<Vec3>>() {
                let v0 = buf.get(v0_idx).copied().unwrap_or(Vec3::ZERO);
                let v1 = buf.get(v1_idx).copied().unwrap_or(Vec3::ZERO);
                buf.push(v0.lerp(v1, t));
                continue;
            }

            if let Some(buf) = attr.as_storage_mut::<Vec<Vec4>>() {
                let v0 = buf.get(v0_idx).copied().unwrap_or(Vec4::ZERO);
                let v1 = buf.get(v1_idx).copied().unwrap_or(Vec4::ZERO);
                buf.push(v0.lerp(v1, t));
                continue;
            }

            if let Some(buf) = attr.as_storage_mut::<Vec<i32>>() {
                let v0 = buf.get(v0_idx).copied().unwrap_or(0);
                buf.push(v0);
                continue;
            }
        }
    }
}
