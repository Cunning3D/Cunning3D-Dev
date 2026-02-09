//! Miter handling: port of Blender adjust_miter_coords / adjust_miter_inner_coords (3059–3136).
//! Prevents self-intersection at sharp outer/inner angles by inserting extra geometry.
#[allow(unused_imports)]
use super::super::super::structures::{BevelGraph, MiterType};
use super::super::boundary::{AngleKind, BoundVertLite, BoundaryResult};
use bevy::prelude::*;

/// Result of miter analysis for a single BevVert.
#[derive(Clone, Debug, Default)]
pub struct MiterResult {
    /// Index of the first edge of an outer miter (if any). -1 if none.
    pub emiter_idx: i32,
    /// Adjusted positions for miter boundverts: (original_idx, new_pos).
    pub adjustments: Vec<(usize, Vec3)>,
    /// Extra boundverts to insert: (after_idx, position, is_patch_mid).
    pub extra_verts: Vec<(usize, Vec3, bool)>,
}

/// Blender adjust_miter_coords (3060-3107): adjust outer miter boundvert positions.
/// v1 = emiter.rightv, v2 = middle (patch only), v3 = v1.next or v2.next
/// co1/co3 are computed by intersecting lines through co2 with planes through neighbors.
pub fn adjust_miter_coords(
    bndv: &[BoundVertLite],
    emiter_idx: usize,
    v_pos: Vec3,
    spoke_dirs: &[Vec3],
    miter_outer: MiterType,
    offset: f32,
    seg: usize,
) -> Vec<(usize, Vec3)> {
    if bndv.is_empty() || emiter_idx >= bndv.len() {
        return vec![];
    }
    let n = bndv.len();

    // v1 = emiter.rightv (in Blender terms, the boundvert to the right of the miter edge)
    let v1_idx = emiter_idx;
    let v1 = &bndv[v1_idx];
    let (v2_idx, v3_idx) = match miter_outer {
        MiterType::Patch => ((v1_idx + 1) % n, (v1_idx + 2) % n),
        MiterType::Arc => (usize::MAX, (v1_idx + 1) % n),
        MiterType::Sharp => return vec![],
    };
    let v1prev_idx = if v1_idx == 0 { n - 1 } else { v1_idx - 1 };
    let v3next_idx = (v3_idx + 1) % n;

    let co2 = v1.pos;
    let d = offset / (seg as f32 / 2.0).max(1.0); // Fallback move amount

    // Edge direction from spoke_dirs
    let edge_dir = spoke_dirs
        .get(emiter_idx)
        .copied()
        .unwrap_or(Vec3::X)
        .normalize_or_zero();

    // co1: intersection of line(co2, co2+edge_dir*d) with plane(v1prev.pos, edge_dir)
    let line_p = co2 + edge_dir * d;
    let v1prev_pos = bndv[v1prev_idx].pos;
    let co1 = isect_line_plane(co2, line_p, v1prev_pos, edge_dir).unwrap_or(line_p);

    // co3: similar for v3next
    let v3 = &bndv[v3_idx];
    let v3_edge_idx = v3.elast.unwrap_or(v3_idx);
    let edge_dir3 = spoke_dirs
        .get(v3_edge_idx)
        .copied()
        .unwrap_or(-edge_dir)
        .normalize_or_zero();
    let line_p3 = co2 + edge_dir3 * d;
    let v3next_pos = bndv[v3next_idx].pos;
    let co3 = isect_line_plane(co2, line_p3, v3next_pos, edge_dir3).unwrap_or(line_p3);

    vec![(v1_idx, co1), (v3_idx, co3)]
}

/// Blender adjust_miter_inner_coords (3109-3136): spread inner miter verts along edge directions.
pub fn adjust_miter_inner_coords(
    bndv: &mut [BoundVertLite],
    emiter_idx: Option<usize>,
    v_pos: Vec3,
    spoke_dirs: &[Vec3],
    spread: f32,
) {
    let n = bndv.len();
    let mut i = 0;
    while i < n {
        if bndv[i].is_arc_start {
            let v3_idx = (i + 1) % n;
            let e_idx = bndv[i].efirst.unwrap_or(i);
            // Skip if this is the outer miter edge
            if emiter_idx.map_or(true, |em| e_idx != em) {
                let co = bndv[i].pos;
                // Direction from v_pos to spoke dest (edge_dir)
                if let Some(&dir) = spoke_dirs.get(e_idx) {
                    bndv[i].pos = co + dir.normalize_or_zero() * spread;
                }
                // v3's edge
                let e3_idx = bndv[v3_idx].elast.unwrap_or(v3_idx);
                if let Some(&dir3) = spoke_dirs.get(e3_idx) {
                    bndv[v3_idx].pos = co + dir3.normalize_or_zero() * spread;
                }
            }
            i = v3_idx + 1;
        } else {
            i += 1;
        }
    }
}

/// Check if miter handling is needed for this vertex configuration.
/// Returns (needs_outer_miter, outer_miter_edge_idx, needs_inner_miter).
pub fn miter_test(
    selcount: usize,
    angle_kinds: &[AngleKind],
    miter_outer: MiterType,
    miter_inner: MiterType,
) -> (bool, Option<usize>, bool) {
    // Blender 3178-3180: outer miter only for 3+ beveled edges
    let effective_outer = if selcount >= 3 {
        miter_outer
    } else {
        MiterType::Sharp
    };

    let mut outer_idx: Option<usize> = None;
    let mut needs_inner = false;

    for (i, &ang) in angle_kinds.iter().enumerate() {
        // Blender 3261-3262: outer miter at first ANGLE_LARGER, inner at ANGLE_SMALLER
        if effective_outer != MiterType::Sharp && outer_idx.is_none() && ang == AngleKind::Larger {
            outer_idx = Some(i);
        }
        if miter_inner != MiterType::Sharp && ang == AngleKind::Smaller {
            needs_inner = true;
        }
    }

    (outer_idx.is_some(), outer_idx, needs_inner)
}

/// Blender's isect_line_plane_v3: find intersection of line with plane.
fn isect_line_plane(l1: Vec3, l2: Vec3, plane_co: Vec3, plane_no: Vec3) -> Option<Vec3> {
    let u = l2 - l1;
    let dot = plane_no.dot(u);
    if dot.abs() < 1e-10 {
        return None;
    }
    let w = l1 - plane_co;
    let fac = -plane_no.dot(w) / dot;
    Some(l1 + u * fac)
}

/// Build extra miter boundverts for PATCH mode. Returns new verts to insert after v1.
/// Blender 3271-3301: for PATCH, insert one extra vert; for ARC, just mark v1.is_arc_start.
pub fn build_miter_boundverts(
    v1_pos: Vec3,
    miter_type: MiterType,
    angle_kind: AngleKind,
) -> Vec<(Vec3, bool)> {
    match (miter_type, angle_kind) {
        (MiterType::Patch, AngleKind::Larger) => {
            // Insert one extra vert at same position (will be adjusted later)
            vec![(v1_pos, true)] // (pos, is_patch_mid)
        }
        (MiterType::Arc, AngleKind::Larger)
        | (MiterType::Arc, AngleKind::Smaller)
        | (MiterType::Patch, AngleKind::Smaller) => {
            // No extra vert, just mark arc_start
            vec![]
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_miter_test_basic() {
        // 3 beveled edges, one LARGER angle
        let angles = vec![AngleKind::Straight, AngleKind::Larger, AngleKind::Smaller];
        let (outer, idx, inner) = miter_test(3, &angles, MiterType::Patch, MiterType::Sharp);
        assert!(outer);
        assert_eq!(idx, Some(1));
        assert!(!inner);
    }

    #[test]
    fn test_miter_sharp_no_action() {
        let angles = vec![AngleKind::Larger, AngleKind::Smaller];
        let (outer, idx, _) = miter_test(3, &angles, MiterType::Sharp, MiterType::Sharp);
        assert!(!outer);
        assert!(idx.is_none());
    }
}
