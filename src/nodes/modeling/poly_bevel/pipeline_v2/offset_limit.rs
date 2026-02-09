//! Offset limiting: port of Blender bevel_limit_offset / geometry_collide_offset / vertex_collide_offset (7625–7850).
//! Prevents geometry self-intersection by clamping offsets before they cause collisions.
use super::super::structures::{BevelAffect, BevelGraph, BevelParams, EdgeHalf, OffsetType};
use bevy::prelude::*;

const BEVEL_EPSILON: f32 = 1e-6;

/// Result of collision analysis for an edge.
#[derive(Clone, Debug)]
pub struct CollisionResult {
    pub edge_idx: usize,
    pub max_offset: f32,
    pub is_limited: bool,
}

/// Blender geometry_collide_offset (7625-7770): calculate max offset before edge B collapses.
/// Returns the maximum safe offset for edge `eb_idx` before geometry collision.
pub fn geometry_collide_offset(
    graph: &BevelGraph,
    eb_idx: usize,
    base_offset: f32,
    positions: &dyn Fn(usize) -> Vec3,  // point_id -> position
    edge_length: &dyn Fn(usize) -> f32, // edge_idx -> length
) -> f32 {
    let no_collide = base_offset + 1e6;
    if base_offset < BEVEL_EPSILON {
        return no_collide;
    }

    let eb = match graph.edges.get(eb_idx) {
        Some(e) => e,
        None => return no_collide,
    };
    if !eb.is_bev {
        return no_collide;
    }

    // kb = offset spec for edge B left side
    let kb = eb.offset_l;

    // ea = eb.next (direction b --> a)
    let ea_idx = eb.next_index;
    let ea = match graph.edges.get(ea_idx) {
        Some(e) => e,
        None => return no_collide,
    };
    let ka = ea.offset_r;

    // Get the BevVert for this edge's origin
    let bv_idx = eb.origin_bev_vert;
    let bv = match graph.verts.get(bv_idx) {
        Some(v) => v,
        None => return no_collide,
    };

    // Vertices: va, vb (origin), vc (dest of eb)
    let vb_pos = positions(bv.p_id.index());

    // Get other endpoint of eb
    let eb_pair_idx = eb.pair_index;
    let eb_pair = graph.edges.get(eb_pair_idx);
    let vc_idx = eb_pair
        .map(|e| graph.verts.get(e.origin_bev_vert).map(|v| v.p_id.index()))
        .flatten();
    let vc_pos = vc_idx.map(|i| positions(i)).unwrap_or(vb_pos);

    // Get va (dest of ea from vb's perspective)
    let ea_pair = graph.edges.get(ea.pair_index);
    let va_idx = ea_pair
        .map(|e| graph.verts.get(e.origin_bev_vert).map(|v| v.p_id.index()))
        .flatten();
    let va_pos = va_idx.map(|i| positions(i)).unwrap_or(vb_pos);

    // Find edge on other end of eb (at vc)
    let kc: f32;
    let vd_pos: Vec3;
    if let Some(pair) = eb_pair {
        let ec_idx = pair.prev_index;
        if let Some(ec) = graph.edges.get(ec_idx) {
            kc = ec.offset_l;
            // Get vd (dest of ec from vc's perspective)
            let ec_pair = graph.edges.get(ec.pair_index);
            let vd_idx = ec_pair
                .map(|e| graph.verts.get(e.origin_bev_vert).map(|v| v.p_id.index()))
                .flatten();
            vd_pos = vd_idx.map(|i| positions(i)).unwrap_or(vc_pos);
        } else {
            kc = 0.0;
            vd_pos = vc_pos + (vc_pos - vb_pos); // Fallback
        }
    } else {
        kc = 0.0;
        vd_pos = vc_pos + (vc_pos - vb_pos);
    }

    // Calculate angles
    let th1 = angle_v3v3v3(va_pos, vb_pos, vc_pos);
    let th2 = angle_v3v3v3(vb_pos, vc_pos, vd_pos);

    let sin1 = th1.sin();
    let sin2 = th2.sin();
    let cos1 = th1.cos();
    let cos2 = th2.cos();

    // Blender 7705-7711: offset at which edge B collapses
    let mut limit = no_collide;
    let offsets_proj = safe_divide(ka + cos1 * kb, sin1) + safe_divide(kc + cos2 * kb, sin2);
    if offsets_proj > BEVEL_EPSILON {
        let edge_len = edge_length(eb_idx);
        let proj = base_offset * (edge_len / offsets_proj);
        if proj > BEVEL_EPSILON {
            limit = proj;
        }
    }

    limit
}

/// Blender vertex_collide_offset (7777-7792): for vertex-only bevels.
pub fn vertex_collide_offset(
    graph: &BevelGraph,
    ea_idx: usize,
    base_offset: f32,
    edge_length: &dyn Fn(usize) -> f32,
) -> f32 {
    let no_collide = base_offset + 1e6;
    if base_offset < BEVEL_EPSILON {
        return no_collide;
    }

    let ea = match graph.edges.get(ea_idx) {
        Some(e) => e,
        None => return no_collide,
    };
    let ka = ea.offset_l / base_offset;

    // Find other end edge half
    let eb_idx = ea.pair_index;
    let kb = graph
        .edges
        .get(eb_idx)
        .map(|e| e.offset_l / base_offset)
        .unwrap_or(0.0);

    let kab = ka + kb;
    let la = edge_length(ea_idx);

    if kab <= 0.0 {
        no_collide
    } else {
        la / kab
    }
}

/// Blender bevel_limit_offset (7799-7850): clamp all offsets to prevent collision.
/// Returns the clamped offset value and whether clamping occurred.
pub fn limit_offset(
    graph: &mut BevelGraph,
    params: &BevelParams,
    positions: &dyn Fn(usize) -> Vec3,
    edge_length: &dyn Fn(usize) -> f32,
) -> (f32, bool) {
    if !params.clamp_overlap {
        return (params.offset, false);
    }

    let mut limited = params.offset;

    for (ei, edge) in graph.edges.iter().enumerate() {
        if !edge.is_bev {
            continue;
        }

        let collision = if params.affect == BevelAffect::Vertices {
            vertex_collide_offset(graph, ei, params.offset, edge_length)
        } else {
            geometry_collide_offset(graph, ei, params.offset, positions, edge_length)
        };
        limited = limited.min(collision);
    }

    let was_limited = limited < params.offset;
    if was_limited {
        // Scale all offsets by reduction factor
        let factor = limited / params.offset;
        for edge in graph.edges.iter_mut() {
            edge.offset_l *= factor;
            edge.offset_r *= factor;
        }
    }

    (limited, was_limited)
}

/// Calculate angle at vertex B between edges BA and BC.
fn angle_v3v3v3(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let ba = (a - b).normalize_or_zero();
    let bc = (c - b).normalize_or_zero();
    ba.dot(bc).clamp(-1.0, 1.0).acos()
}

/// Safe division that returns 0 if denominator is too small.
fn safe_divide(num: f32, denom: f32) -> f32 {
    if denom.abs() < BEVEL_EPSILON {
        0.0
    } else {
        num / denom
    }
}

/// Blender slide_dist: slide along edge by distance d from vertex v.
pub fn slide_dist(edge_dir: Vec3, v_pos: Vec3, d: f32) -> Vec3 {
    v_pos + edge_dir.normalize_or_zero() * d
}

/// Blender offset_in_plane: compute offset position in face plane.
pub fn offset_in_plane(
    edge_dir: Vec3,
    v_pos: Vec3,
    face_no: Vec3,
    offset: f32,
    left: bool,
) -> Vec3 {
    let perp = if left {
        face_no.cross(edge_dir)
    } else {
        edge_dir.cross(face_no)
    };
    v_pos + perp.normalize_or_zero() * offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_angle_v3v3v3() {
        let a = Vec3::X;
        let b = Vec3::ZERO;
        let c = Vec3::Y;
        let angle = angle_v3v3v3(a, b, c);
        assert!((angle - std::f32::consts::FRAC_PI_2).abs() < 0.01);
    }

    #[test]
    fn test_safe_divide() {
        assert_eq!(safe_divide(1.0, 2.0), 0.5);
        assert_eq!(safe_divide(1.0, 0.0), 0.0);
    }
}
