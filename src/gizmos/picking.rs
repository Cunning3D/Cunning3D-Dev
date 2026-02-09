use bevy::prelude::*;

#[inline]
pub fn ray_sphere_hit(ray_origin: Vec3, ray_dir: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let m = ray_origin - center;
    let b = m.dot(ray_dir);
    let c = m.dot(m) - radius * radius;
    if c > 0.0 && b > 0.0 {
        return None;
    }
    let discr = b * b - c;
    if discr < 0.0 {
        return None;
    }
    let t = -b - discr.sqrt();
    if t > 0.0 {
        Some(t)
    } else {
        None
    }
}

#[inline]
pub fn world_to_screen(
    camera: &Camera,
    camera_xform: &GlobalTransform,
    world: Vec3,
) -> Option<Vec2> {
    camera.world_to_viewport(camera_xform, world).ok()
}

#[inline]
pub fn dist_point_to_segment_2d(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() == 0.0 {
        0.0
    } else {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    };
    (a + ab * t).distance(p)
}

pub fn dist_point_to_polyline_2d(p: Vec2, pts: &[Vec2]) -> f32 {
    if pts.len() < 2 {
        return f32::MAX;
    }
    let mut best = f32::MAX;
    for i in 0..(pts.len() - 1) {
        best = best.min(dist_point_to_segment_2d(p, pts[i], pts[i + 1]));
    }
    best
}
