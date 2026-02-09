use bevy::math::{Mat4, Quat, Vec2, Vec3};
use super::direct_manipulation::Ray;
use super::super::unity_spline::*;
use super::spline_transform_context::SnappingFlags;

pub const PICKING_DISTANCE: f32 = 8.0;

#[inline] fn snap_scalar(v: f32, snap: f32) -> f32 { if snap == 0.0 { v } else { (v / snap).round() * snap } }

/// Equivalent to UnityEditor.Splines.SplineHandleUtility.TransformRay.
#[inline]
pub fn transform_ray(ray: Ray, matrix: Mat4) -> Ray {
    Ray { origin: matrix.transform_point3(ray.origin), dir: matrix.transform_vector3(ray.dir) }
}

/// Equivalent to UnityEditor.Splines.SplineHandleUtility.DoIncrementSnap core math.
#[inline]
pub fn do_increment_snap(position: Vec3, previous: Vec3, handle_rotation: Quat, move_snap: Vec3) -> Vec3 {
    let delta = position - previous;
    let right = handle_rotation * Vec3::X;
    let up = handle_rotation * Vec3::Y;
    let forward = handle_rotation * Vec3::Z;
    let snapped_delta =
        snap_scalar(delta.dot(right), move_snap.x) * right +
        snap_scalar(delta.dot(up), move_snap.y) * up +
        snap_scalar(delta.dot(forward), move_snap.z) * forward;
    previous + snapped_delta
}

/// UnityEditor.Splines.SplineHandleUtility.GetMinDifference (needs handle size from UI).
#[inline]
pub fn get_min_difference(handle_size: f32) -> Vec3 { Vec3::splat(handle_size / 80.0) }

/// UnityEditor.Splines.SplineHandleUtility.RoundBasedOnMinimumDifference.
#[inline]
pub fn round_based_on_min_difference(value: f32, min_difference: f32) -> f32 {
    const MAX_DECIMALS: i32 = 15;
    let md = min_difference.abs();
    if md == 0.0 || !md.is_finite() { return value; }
    let decimals = (-md.log10().floor() as i32).clamp(0, MAX_DECIMALS);
    let s = 10_f32.powi(decimals);
    (value * s).round() / s
}

#[inline]
fn approx_zero(x: f32) -> bool { x.abs() <= 1e-6 }

/// UnityEditor.Splines.TransformOperation.ApplySmartRounding (gated by snapping flags).
pub fn apply_smart_rounding(pos: Vec3, handle_size: f32, snapping: SnappingFlags) -> Vec3 {
    if snapping.incremental_snap_active || snapping.grid_snap_active { return pos; }
    let md = get_min_difference(handle_size);
    Vec3::new(
        if approx_zero(pos.x) { pos.x } else { round_based_on_min_difference(pos.x, md.x) },
        if approx_zero(pos.y) { pos.y } else { round_based_on_min_difference(pos.y, md.y) },
        if approx_zero(pos.z) { pos.z } else { round_based_on_min_difference(pos.z, md.z) },
    )
}

pub trait SplinePickingAdapter {
    fn world_to_gui_point(&self, world: Vec3) -> Vec2;
}

#[inline]
fn dist_point_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() == 0.0 { 0.0 } else { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) };
    (a + ab * t).distance(p)
}

/// UI-agnostic equivalent of `SplineHandleUtility.GetNearestPointOnCurve` (screen-space distance, sampled segments).
pub fn get_nearest_point_on_curve<A: SplinePickingAdapter>(a: &A, curve_world: BezierCurve, mouse: Vec2, segments: usize) -> (Vec3, f32, f32) {
    let segs = segments.max(4);
    let mut closest_dist = f32::MAX;
    let mut closest_i = 0usize;
    let mut pts: Vec<Vec3> = Vec::with_capacity(segs);
    for i in 0..segs {
        let t = i as f32 / (segs as f32 - 1.0);
        pts.push(evaluate_position(curve_world, t));
    }
    for i in 0..(pts.len() - 1) {
        let da = a.world_to_gui_point(pts[i]);
        let db = a.world_to_gui_point(pts[i + 1]);
        let d = dist_point_to_segment(mouse, da, db);
        if d < closest_dist { closest_dist = d; closest_i = i; }
    }

    let pa = a.world_to_gui_point(pts[closest_i]);
    let pb = a.world_to_gui_point(pts[closest_i + 1]);
    let ab = pb - pa;
    let mut dot = if ab.length_squared() == 0.0 { 0.0 } else { (mouse - pa).dot(ab) / ab.length_squared() };
    dot = dot.clamp(0.0, 1.0);
    let position = pts[closest_i].lerp(pts[closest_i + 1], dot);

    let percent_per_segment = 1.0 / (segs as f32 - 1.0);
    let t = closest_i as f32 * percent_per_segment + percent_per_segment * (pts[closest_i].distance(position) / pts[closest_i].distance(pts[closest_i + 1]).max(1e-8));
    (position, t, closest_dist)
}

#[inline]
fn closest_points_ray_line(ray_origin: Vec3, ray_dir: Vec3, line_origin: Vec3, line_dir: Vec3) -> (Vec3, Vec3) {
    let w0 = ray_origin - line_origin;
    let a = ray_dir.dot(ray_dir);
    let b = ray_dir.dot(line_dir);
    let c = line_dir.dot(line_dir);
    let d = ray_dir.dot(w0);
    let e = line_dir.dot(w0);
    let denom = a * c - b * b;
    let (sc, tc) = if denom < 1e-5 {
        (0.0, if b > c { d / b } else { e / c })
    } else {
        ((b * e - c * d) / denom, (a * e - b * d) / denom)
    };
    (ray_origin + ray_dir * sc, line_origin + line_dir * tc)
}

#[inline]
fn distance_sq_ray_point(ray_origin: Vec3, ray_dir: Vec3, point: Vec3) -> (f32, Vec3) {
    let w = point - ray_origin;
    let proj = w.dot(ray_dir);
    if proj < 0.0 { return ((ray_origin - point).length_squared(), ray_origin); }
    let closest = ray_origin + ray_dir * proj;
    ((closest - point).length_squared(), closest)
}

#[inline]
fn distance_sq_ray_segment(ray_origin: Vec3, ray_dir: Vec3, p0: Vec3, p1: Vec3) -> (f32, f32) {
    let seg_dir = p1 - p0;
    let seg_len_sq = seg_dir.length_squared();
    if seg_len_sq < 1e-5 { return ((ray_origin - p0).length_squared(), 0.0); }
    let (_, p_line) = closest_points_ray_line(ray_origin, ray_dir, p0, seg_dir);
    let t = (p_line - p0).dot(seg_dir) / seg_len_sq;
    let t_clamped = t.clamp(0.0, 1.0);
    let p_seg = p0 + seg_dir * t_clamped;
    let (dist_sq, _) = distance_sq_ray_point(ray_origin, ray_dir, p_seg);
    (dist_sq, t_clamped)
}

/// Ray-based nearest point on a Bezier curve (world-space), used by DCC viewport picking (not screen-space).
/// Returns (world_position, t_on_curve, distance_world).
pub fn get_nearest_point_on_curve_ray(curve_world: BezierCurve, ray_origin: Vec3, ray_dir: Vec3, segments: usize) -> (Vec3, f32, f32) {
    let segs = segments.max(4);
    let mut pts: Vec<Vec3> = Vec::with_capacity(segs);
    for i in 0..segs {
        let t = i as f32 / (segs as f32 - 1.0);
        pts.push(evaluate_position(curve_world, t));
    }

    let mut best_dist_sq = f32::MAX;
    let mut best_i = 0usize;
    let mut best_alpha = 0.0f32;

    for i in 0..(pts.len() - 1) {
        let (dist_sq, alpha) = distance_sq_ray_segment(ray_origin, ray_dir, pts[i], pts[i + 1]);
        if dist_sq < best_dist_sq {
            best_dist_sq = dist_sq;
            best_i = i;
            best_alpha = alpha;
        }
    }

    let position = pts[best_i].lerp(pts[best_i + 1], best_alpha);
    let percent_per_segment = 1.0 / (segs as f32 - 1.0);
    let t = best_i as f32 * percent_per_segment + percent_per_segment * best_alpha;
    (position, t, best_dist_sq.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increment_snap_snaps_in_handle_axes() {
        let prev = Vec3::ZERO;
        let pos = Vec3::new(0.49, 0.0, 0.51);
        let snapped = do_increment_snap(pos, prev, Quat::IDENTITY, Vec3::new(1.0, 1.0, 1.0));
        assert_eq!(snapped, Vec3::new(0.0, 0.0, 1.0));
    }

    #[derive(Clone, Copy, Debug)]
    struct DummyPick;
    impl SplinePickingAdapter for DummyPick {
        fn world_to_gui_point(&self, w: Vec3) -> Vec2 { Vec2::new(w.x, w.z) }
    }

    #[test]
    fn nearest_point_on_curve_returns_mid_for_straight_line() {
        let a = DummyPick;
        let curve = BezierCurve { p0: Vec3::ZERO, p1: Vec3::ZERO, p2: Vec3::Z, p3: Vec3::Z };
        let (p, t, _d) = get_nearest_point_on_curve(&a, curve, Vec2::new(0.0, 0.5), 30);
        assert!((p.z - 0.5).abs() < 1e-2);
        assert!((t - 0.5).abs() < 1e-2);
    }

    #[test]
    fn smart_rounding_disabled_when_snapping_enabled() {
        let p = Vec3::new(0.123456, 0.0, 0.0);
        let s = apply_smart_rounding(p, 10.0, SnappingFlags { incremental_snap_active: true, grid_snap_active: false });
        assert_eq!(s, p);
    }
}
