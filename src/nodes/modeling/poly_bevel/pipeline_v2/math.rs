use bevy::prelude::*;

// Profile constants
pub const PRO_SQUARE_R: f32 = 1e4;
pub const PRO_CIRCLE_R: f32 = 2.0;
pub const PRO_LINE_R: f32 = 1.0;
pub const PRO_SQUARE_IN_R: f32 = 0.0;

// Precision constants (from Blender)
pub const BEVEL_EPSILON: f32 = 1e-6;
pub const BEVEL_EPSILON_SQ: f32 = 1e-12;
pub const BEVEL_EPSILON_BIG: f32 = 1e-4;
pub const BEVEL_EPSILON_ANG: f32 = 0.0349; // ~2 degrees in radians
pub const BEVEL_SMALL_ANG: f32 = 0.1745; // ~10 degrees in radians

/// Unified Epsilon Management for Bevel operations
#[derive(Clone, Copy, Debug)]
pub struct BevelEpsilon {
    pub eps: f32,
    pub eps_sq: f32,
    pub eps_big: f32,
    pub angle_eps: f32,
}

impl Default for BevelEpsilon {
    fn default() -> Self {
        Self {
            eps: BEVEL_EPSILON,
            eps_sq: BEVEL_EPSILON_SQ,
            eps_big: BEVEL_EPSILON_BIG,
            angle_eps: 0.001,
        }
    }
}

impl BevelEpsilon {
    pub fn new(bbox_size: f32) -> Self {
        // Scale epsilon based on model size, clamped to avoid becoming too small or too large
        let scale = bbox_size.max(1.0); // Assuming metric scale, max(1.0) prevents super small epsilon for tiny objects
                                        // Limit max epsilon to prevent rejecting small but valid bevel offsets
        let eps = (1e-6 * scale).min(1e-4);
        Self {
            eps,
            eps_sq: eps * eps,
            eps_big: (1e-4 * scale).min(1e-2),
            angle_eps: 0.001,
        }
    }

    #[inline]
    pub fn eq(&self, a: f32, b: f32) -> bool {
        (a - b).abs() < self.eps
    }

    #[inline]
    pub fn zero(&self, a: f32) -> bool {
        a.abs() < self.eps
    }

    #[inline]
    pub fn is_small_vec(&self, v: Vec3) -> bool {
        v.length_squared() < self.eps_sq
    }

    #[inline]
    pub fn parallel(&self, a: Vec3, b: Vec3) -> bool {
        // Normalized dot product check
        let la = a.length_squared();
        let lb = b.length_squared();
        if la < self.eps_sq || lb < self.eps_sq {
            return true;
        }
        (a.dot(b).powi(2) / (la * lb)) > (1.0 - self.eps_big)
    }
}

pub fn nearly_parallel(a: Vec3, b: Vec3) -> bool {
    let la = a.length_squared();
    let lb = b.length_squared();
    if la < 1e-12 || lb < 1e-12 {
        return true;
    }
    (a.normalize().dot(b.normalize())).abs() > 0.9999
}

pub fn isect_line_line_v3(l1a: Vec3, l1b: Vec3, l2a: Vec3, l2b: Vec3) -> Option<(Vec3, Vec3)> {
    let u = l1b - l1a;
    let v = l2b - l2a;
    let w0 = l1a - l2a;
    let a = u.dot(u);
    let b = u.dot(v);
    let c = v.dot(v);
    let d = u.dot(w0);
    let e = v.dot(w0);
    let denom = a * c - b * b;
    if denom.abs() < 1e-12 {
        return None;
    }
    let sc = (b * e - c * d) / denom;
    let tc = (a * e - b * d) / denom;
    Some((l1a + u * sc, l2a + v * tc))
}

pub fn isect_line_plane_v3(p0: Vec3, p1: Vec3, plane_co: Vec3, plane_no: Vec3) -> Option<Vec3> {
    let u = p1 - p0;
    let denom = plane_no.dot(u);
    if denom.abs() < 1e-12 {
        return None;
    }
    let t = plane_no.dot(plane_co - p0) / denom;
    Some(p0 + u * t)
}

pub fn closest_to_plane(p: Vec3, plane_point: Vec3, plane_no: Vec3) -> Vec3 {
    let n = plane_no.normalize_or_zero();
    p - n * n.dot(p - plane_point)
}

pub fn closest_to_line_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let ab = b - a;
    let t = if ab.length_squared() < 1e-12 {
        0.0
    } else {
        (p - a).dot(ab) / ab.length_squared()
    };
    a + ab * t.clamp(0.0, 1.0)
}

pub fn project_to_edge(edge_v1: Vec3, edge_v2: Vec3, co_a: Vec3, co_b: Vec3) -> Vec3 {
    if let Some((proj_co, _)) = isect_line_line_v3(edge_v1, edge_v2, co_a, co_b) {
        proj_co
    } else {
        edge_v1
    }
}

pub fn make_unit_square_map(va: Vec3, vmid: Vec3, vb: Vec3) -> Option<Mat4> {
    let va_vmid = vmid - va;
    let vb_vmid = vmid - vb;
    if va_vmid.length_squared() < 1e-12 || vb_vmid.length_squared() < 1e-12 {
        return None;
    }
    if (va_vmid.angle_between(vb_vmid) - std::f32::consts::PI).abs() <= 1e-4 {
        return None;
    }
    let vo = va - vb_vmid;
    let vddir = vb_vmid.cross(va_vmid).normalize_or_zero();
    let vd = vo + vddir;
    let c0 = vmid - va;
    let c1 = vmid - vb;
    let c2 = vmid + vd - va - vb;
    let c3 = va + vb - vmid;
    Some(Mat4::from_cols(
        Vec4::new(c0.x, c0.y, c0.z, 0.0),
        Vec4::new(c1.x, c1.y, c1.z, 0.0),
        Vec4::new(c2.x, c2.y, c2.z, 0.0),
        Vec4::new(c3.x, c3.y, c3.z, 1.0),
    ))
}

pub fn make_unit_cube_map(va: Vec3, vb: Vec3, vc: Vec3, vd: Vec3) -> Mat4 {
    let c0 = (va - vb - vc + vd) * 0.5;
    let c1 = (-va + vb - vc + vd) * 0.5;
    let c2 = (-va - vb + vc + vd) * 0.5;
    let c3 = (va + vb + vc - vd) * 0.5;
    Mat4::from_cols(
        Vec4::new(c0.x, c0.y, c0.z, 0.0),
        Vec4::new(c1.x, c1.y, c1.z, 0.0),
        Vec4::new(c2.x, c2.y, c2.z, 0.0),
        Vec4::new(c3.x, c3.y, c3.z, 1.0),
    )
}

// Robustness utilities (from Blender)

/// Safe slide distance to prevent geometric errors
pub fn slide_dist_safe(edge_dir: Vec3, dist: f32) -> (Vec3, f32) {
    let len = edge_dir.length();
    if len < BEVEL_EPSILON {
        return (Vec3::ZERO, 0.0);
    }
    let safe_dist = dist.min(len - 50.0 * BEVEL_EPSILON); // Blender: len - 50*EPSILON
    let dir = edge_dir / len;
    (dir * safe_dist.max(0.0), safe_dist.max(0.0))
}

/// Classify angle: Smaller(-1) / Straight(0) / Larger(1)
pub fn classify_angle(dir1: Vec3, dir2: Vec3, face_normal: Vec3) -> i32 {
    let d1 = dir1.normalize_or_zero();
    let d2 = dir2.normalize_or_zero();
    if nearly_parallel(d1, d2) {
        return 0;
    } // ANGLE_STRAIGHT
    let cross = d1.cross(d2);
    if cross.dot(face_normal) > 0.0 {
        -1
    } else {
        1
    } // SMALLER or LARGER
}

/// Safe offset clamp to half edge length
pub fn clamp_offset_to_edge(offset: f32, edge_len: f32) -> f32 {
    offset.min(edge_len * 0.5 - BEVEL_EPSILON_BIG)
}

/// Check if two vectors are nearly opposite (180 degrees)
pub fn nearly_antiparallel(a: Vec3, b: Vec3) -> bool {
    let la = a.length_squared();
    let lb = b.length_squared();
    if la < BEVEL_EPSILON_SQ || lb < BEVEL_EPSILON_SQ {
        return false;
    }
    a.normalize().dot(b.normalize()) < -0.9999
}

/// Safe intersection with fallback
pub fn safe_offset_meet(p1: Vec3, d1: Vec3, p2: Vec3, d2: Vec3, fallback: Vec3) -> Vec3 {
    if nearly_parallel(d1, d2) || nearly_antiparallel(d1, d2) {
        return fallback;
    }
    isect_line_line_v3(p1, p1 + d1, p2, p2 + d2)
        .map(|(meet, _)| meet)
        .unwrap_or(fallback)
}

/// Check if point is within valid edge range
pub fn point_on_edge_range(point: Vec3, edge_v1: Vec3, edge_v2: Vec3) -> bool {
    let edge = edge_v2 - edge_v1;
    let len_sq = edge.length_squared();
    if len_sq < BEVEL_EPSILON_SQ {
        return false;
    }
    let t = (point - edge_v1).dot(edge) / len_sq;
    t >= -BEVEL_EPSILON_BIG && t <= 1.0 + BEVEL_EPSILON_BIG
}
