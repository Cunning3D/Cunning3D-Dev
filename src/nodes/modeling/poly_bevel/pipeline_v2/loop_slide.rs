//! Loop Slide: port of Blender offset_on_edge_between / offset_meet_edge (1721–1847).
//! Calculates meeting point between beveled and non-beveled edges for smooth profile continuity.
use super::super::structures::{BevelParams, OffsetType};
use bevy::prelude::*;

const BEVEL_GOOD_ANGLE: f32 = 0.1;

/// Blender offset_meet_edge (1721-1764): Calculate meeting point between e1 and e2.
/// e1 precedes e2 in CCW order. Returns (success, meetco, angle).
pub fn offset_meet_edge(
    v_pos: Vec3,
    v_normal: Vec3,
    dir1: Vec3,     // Direction from v to other vert of e1
    dir2: Vec3,     // Direction from v to other vert of e2
    offset1_r: f32, // Right offset of e1
    offset2_l: f32, // Left offset of e2
) -> (bool, Vec3, f32) {
    let d1 = dir1.normalize_or_zero();
    let d2 = dir2.normalize_or_zero();

    // Find angle from dir1 to dir2 as viewed from vertex normal side
    let mut ang = d1.dot(d2).clamp(-1.0, 1.0).acos();
    if ang.abs() < BEVEL_GOOD_ANGLE {
        return (false, v_pos, 0.0);
    }

    let fno = d1.cross(d2);
    if fno.dot(v_normal) < 0.0 {
        ang = 2.0 * std::f32::consts::PI - ang; // Angle is reflex
        return (false, v_pos, ang);
    }

    if (ang - std::f32::consts::PI).abs() < BEVEL_GOOD_ANGLE {
        return (false, v_pos, ang);
    }

    let sinang = ang.sin();
    let mut meetco = v_pos;

    if offset1_r == 0.0 {
        meetco += d1 * (offset2_l / sinang);
    } else {
        meetco += d2 * (offset1_r / sinang);
    }

    (true, meetco, ang)
}

/// Blender good_offset_on_edge_between (1771-1777): Check if meeting point looks good.
pub fn good_offset_on_edge_between(
    v_pos: Vec3,
    v_normal: Vec3,
    dir1: Vec3,
    dir_mid: Vec3,
    dir2: Vec3,
    offset1_r: f32,
    offset2_l: f32,
) -> bool {
    let (ok1, _, _) = offset_meet_edge(v_pos, v_normal, dir1, dir_mid, offset1_r, 0.0);
    let (ok2, _, _) = offset_meet_edge(v_pos, v_normal, dir_mid, dir2, 0.0, offset2_l);
    ok1 && ok2
}

/// Blender offset_on_edge_between (1787-1847): Calculate best meeting point on in-between edge.
/// e1 and e2 are beveled edges, emid is non-beveled edge between them.
/// Returns (meetco, sin_ratio_option).
pub fn offset_on_edge_between(
    params: &BevelParams,
    v_pos: Vec3,
    v_normal: Vec3,
    v2_pos: Vec3,  // Other vert of emid
    dir1: Vec3,    // Direction from v to other vert of e1
    dir2: Vec3,    // Direction from v to other vert of e2
    dir_mid: Vec3, // Direction from v to v2 (emid direction)
    offset1_r: f32,
    offset2_l: f32,
) -> (Vec3, Option<f32>) {
    // For PERCENT or ABSOLUTE offset types, just slide along emid
    match params.offset_type {
        OffsetType::Percent => {
            let meetco = v_pos.lerp(v2_pos, params.offset / 100.0);
            let (_, _, ang1) = offset_meet_edge(v_pos, v_normal, dir1, dir_mid, offset1_r, 0.0);
            let (_, _, ang2) = offset_meet_edge(v_pos, v_normal, dir_mid, dir2, 0.0, offset2_l);
            let sinratio = if ang1 == 0.0 {
                1.0
            } else {
                ang2.sin() / ang1.sin()
            };
            return (meetco, Some(sinratio));
        }
        OffsetType::Absolute => {
            let d = dir_mid.normalize_or_zero();
            let meetco = v_pos + d * params.offset;
            let (_, _, ang1) = offset_meet_edge(v_pos, v_normal, dir1, dir_mid, offset1_r, 0.0);
            let (_, _, ang2) = offset_meet_edge(v_pos, v_normal, dir_mid, dir2, 0.0, offset2_l);
            let sinratio = if ang1 == 0.0 {
                1.0
            } else {
                ang2.sin() / ang1.sin()
            };
            return (meetco, Some(sinratio));
        }
        OffsetType::Depth => {
            let d = dir_mid.normalize_or_zero();
            let meetco = v_pos + d * params.offset;
            let (_, _, ang1) = offset_meet_edge(v_pos, v_normal, dir1, dir_mid, offset1_r, 0.0);
            let (_, _, ang2) = offset_meet_edge(v_pos, v_normal, dir_mid, dir2, 0.0, offset2_l);
            let sinratio = if ang1 == 0.0 {
                1.0
            } else {
                ang2.sin() / ang1.sin()
            };
            return (meetco, Some(sinratio));
        }
        _ => {}
    }

    // Standard case: find meeting points and average
    let (ok1, meet1, ang1) = offset_meet_edge(v_pos, v_normal, dir1, dir_mid, offset1_r, 0.0);
    let (ok2, meet2, ang2) = offset_meet_edge(v_pos, v_normal, dir_mid, dir2, 0.0, offset2_l);

    if ok1 && ok2 {
        let meetco = (meet1 + meet2) * 0.5;
        let sinratio = if ang1 == 0.0 {
            1.0
        } else {
            ang2.sin() / ang1.sin()
        };
        (meetco, Some(sinratio))
    } else if ok1 {
        (meet1, None)
    } else if ok2 {
        (meet2, None)
    } else {
        // Neither offset line met emid - slide along emid by offset
        let d = dir_mid.normalize_or_zero();
        (v_pos + d * offset1_r, None)
    }
}

/// Blender slide_dist: slide point along edge by distance.
pub fn slide_dist(v_pos: Vec3, edge_dir: Vec3, dist: f32) -> Vec3 {
    v_pos + edge_dir.normalize_or_zero() * dist
}

/// Blender offset_in_plane (1851+): offset point in a plane perpendicular to edge.
pub fn offset_in_plane(
    v_pos: Vec3,
    edge_dir: Vec3,
    plane_no: Option<Vec3>,
    offset: f32,
    left: bool,
) -> Vec3 {
    let d = edge_dir.normalize_or_zero();

    // Find perpendicular direction in plane
    let perp = if let Some(pn) = plane_no {
        d.cross(pn).normalize_or_zero()
    } else {
        // Arbitrary perpendicular
        let up = if d.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
        d.cross(up).normalize_or_zero()
    };

    if left {
        v_pos + perp * offset
    } else {
        v_pos - perp * offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offset_meet_edge() {
        let v = Vec3::ZERO;
        let vn = Vec3::Y;
        let d1 = Vec3::X;
        let d2 = Vec3::Z;
        let (ok, meet, ang) = offset_meet_edge(v, vn, d1, d2, 0.0, 0.5);
        assert!(ok);
        assert!(ang > 1.5 && ang < 1.6); // ~PI/2
        assert!(meet.x.abs() > 0.4); // Should be offset along d1
    }

    #[test]
    fn test_slide_dist() {
        let v = Vec3::new(1.0, 0.0, 0.0);
        let d = Vec3::X;
        let result = slide_dist(v, d, 0.5);
        assert!((result.x - 1.5).abs() < 0.01);
    }
}
