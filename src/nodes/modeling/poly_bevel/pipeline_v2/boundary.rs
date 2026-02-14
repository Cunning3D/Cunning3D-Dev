//! BoundaryLite layer: minimal port of Blender build_boundary (3152–3392) for SquareOut/SquareIn/pipe/tri_corner.
use super::super::structures::{BevelParams, OffsetType};
use super::loop_slide::{good_offset_on_edge_between, offset_on_edge_between};
use super::math::{isect_line_line_v3, nearly_parallel};
use crate::libs::geometry::ids::HalfEdgeId;
use bevy::prelude::*;

/// AngleKind: classify angle between two edges (port of Blender 1317+).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AngleKind {
    Straight,
    Smaller,
    Larger,
}

/// Miter type (Blender BEVEL_MITER_*).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MiterKind {
    #[default]
    Sharp,
    Patch,
    Arc,
}

/// BevelParamsLite: geometry-only parameters from BevelParams (no UV/attribute fields).
#[derive(Clone, Debug)]
pub struct BevelParamsLite {
    pub offset: f32,
    pub seg: usize,
    pub loop_slide: bool,
    pub miter_outer: MiterKind,
    pub miter_inner: MiterKind,
    pub spread: f32,
}
impl Default for BevelParamsLite {
    fn default() -> Self {
        Self {
            offset: 0.1,
            seg: 4,
            loop_slide: true,
            miter_outer: MiterKind::Sharp,
            miter_inner: MiterKind::Sharp,
            spread: 0.1,
        }
    }
}

/// Convert BevelParamsLite to BevelParams for loop_slide functions.
fn to_full_params(lite: &BevelParamsLite) -> BevelParams {
    BevelParams {
        offset: lite.offset,
        seg: lite.seg,
        loop_slide: lite.loop_slide,
        spread: lite.spread,
        ..Default::default()
    }
}

/// BoundVertLite: minimal port of Blender BoundVert (222–256).
#[derive(Clone, Debug)]
pub struct BoundVertLite {
    pub pos: Vec3,
    pub prev: usize,
    pub next: usize,
    pub index: usize,
    /// Index of first spoke (in spoke ring) this boundvert covers (efirst in Blender).
    pub efirst: Option<usize>,
    /// Index of last spoke this boundvert covers (elast in Blender).
    pub elast: Option<usize>,
    /// Index of the "on edge" spoke if boundvert is placed on an edge (eon).
    pub eon: Option<usize>,
    /// Ratio of sines when eon is set (sinratio).
    pub sinratio: f32,
    /// This boundvert starts an arc profile.
    pub is_arc_start: bool,
    /// This boundvert starts a patch profile.
    pub is_patch_start: bool,
    /// Profile index for in-between edges (Blender e->profile_index).
    pub profile_index: i32,
    /// Seam length from this boundvert.
    pub seam_len: usize,
    /// Sharp edges length from this boundvert.
    pub sharp_len: usize,
    /// Middle point for arc profiles (Blender profile.middle).
    pub arc_middle: Vec3,
}
impl Default for BoundVertLite {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            prev: 0,
            next: 0,
            index: 0,
            efirst: None,
            elast: None,
            eon: None,
            sinratio: 1.0,
            is_arc_start: false,
            is_patch_start: false,
            profile_index: 0,
            seam_len: 0,
            sharp_len: 0,
            arc_middle: Vec3::ZERO,
        }
    }
}

/// EdgeInfo: per-spoke metadata used during boundary build.
#[derive(Clone, Debug)]
pub struct EdgeInfo {
    pub is_bev: bool,
    pub is_seam: bool,
    pub is_sharp: bool,
    pub in_plane: bool,
    pub profile_index: i32,
    /// Left/right boundary vert indices (after build_boundary_lite).
    pub left_bv: Option<usize>,
    pub right_bv: Option<usize>,
}
impl Default for EdgeInfo {
    fn default() -> Self {
        Self {
            is_bev: false,
            is_seam: false,
            is_sharp: false,
            in_plane: false,
            profile_index: 0,
            left_bv: None,
            right_bv: None,
        }
    }
}

/// BoundaryResult: output of build_boundary_lite.
#[derive(Clone, Debug, Default)]
pub struct BoundaryResult {
    pub bnd_verts: Vec<BoundVertLite>,
    pub edges: Vec<EdgeInfo>,
    pub count: usize,
}

/// Classify angle between two edge directions (Blender edges_angle_kind 1317+).
pub fn edges_angle_kind(dir1: Vec3, dir2: Vec3, face_no: Vec3) -> AngleKind {
    let d1 = dir1.normalize_or_zero();
    let d2 = dir2.normalize_or_zero();
    if d1.length_squared() < 1e-12 || d2.length_squared() < 1e-12 {
        return AngleKind::Straight;
    }
    if nearly_parallel(d1, d2) {
        return AngleKind::Straight;
    }
    let cross = d1.cross(d2).normalize_or_zero();
    if cross.dot(face_no.normalize_or_zero()) < 0.0 {
        AngleKind::Larger
    } else {
        AngleKind::Smaller
    }
}

/// offset_meet: compute meeting point of two offset edges (Blender offset_meet 1520+ subset).
pub fn offset_meet(
    v_pos: Vec3,
    e1_dir: Vec3,
    e2_dir: Vec3,
    face_no: Vec3,
    off1_r: f32,
    off2_l: f32,
) -> Vec3 {
    let d1 = e1_dir.normalize_or_zero();
    let d2 = e2_dir.normalize_or_zero();
    let n = stabilize_face_no(face_no, d1, d2);
    let in1 = n.cross(d1).normalize_or_zero();
    // For the second spoke, inward offset is the opposite cross order.
    let in2 = d2.cross(n).normalize_or_zero();
    let p1 = v_pos + in1 * off1_r;
    let p2 = v_pos + in2 * off2_l;
    if let Some((m, _)) = isect_line_line_v3(p1, p1 + e1_dir, p2, p2 + e2_dir) {
        m
    } else {
        v_pos + (in1 * off1_r + in2 * off2_l).normalize_or_zero() * off1_r.max(off2_l)
    }
}

/// Normalize and orient face normal so offset direction is stable under winding flips.
#[inline]
fn stabilize_face_no(face_no: Vec3, d1: Vec3, d2: Vec3) -> Vec3 {
    let mut n = face_no.normalize_or_zero();
    let c = d1.cross(d2);
    if n.length_squared() < 1e-12 {
        return if c.length_squared() > 1e-12 {
            c.normalize_or_zero()
        } else {
            Vec3::Y
        };
    }
    if c.length_squared() > 1e-12 && n.dot(c) < 0.0 {
        n = -n;
    }
    n
}

/// adjust_miter_inner_coords: spread inner miter verts along their edges (Blender 3109-3136).
fn adjust_miter_inner_coords(
    bnd_verts: &mut [BoundVertLite],
    spoke_dirs: &[Vec3],
    v_pos: Vec3,
    spread: f32,
    emiter_idx: Option<usize>,
) {
    let n = bnd_verts.len();
    if n == 0 {
        return;
    }
    let mut i = 0;
    while i < n {
        let bv = &bnd_verts[i];
        if bv.is_arc_start {
            let v3_idx = bv.next;
            if v3_idx >= n {
                i += 1;
                continue;
            }
            if let Some(efirst) = bv.efirst {
                // Skip if this is the outer miter edge
                if Some(efirst) == emiter_idx {
                    i = v3_idx + 1;
                    continue;
                }
                // Move v along edge direction by spread
                let edge_dir = spoke_dirs
                    .get(efirst)
                    .copied()
                    .unwrap_or(Vec3::X)
                    .normalize_or_zero();
                let co = bv.pos;
                bnd_verts[i].pos = co + edge_dir * spread;
                // Move v3 along its edge direction
                if let Some(elast) = bnd_verts[v3_idx].elast {
                    let edge_dir3 = spoke_dirs
                        .get(elast)
                        .copied()
                        .unwrap_or(Vec3::X)
                        .normalize_or_zero();
                    bnd_verts[v3_idx].pos = co + edge_dir3 * spread;
                }
            }
            i = v3_idx + 1;
        } else {
            i += 1;
        }
    }
}

/// Build boundary vertices for a single BevVert (port of Blender build_boundary 3152–3392).
///
/// # Arguments
/// * `spokes` - all half-edges around the vertex (CCW order from topology)
/// * `spoke_is_bev` - whether each spoke is beveled
/// * `spoke_dirs` - direction from vertex to dest(spoke) for each spoke
/// * `face_normals` - face normal for each spoke's face
/// * `v_pos` - position of the vertex being beveled
/// * `params` - bevel parameters
pub fn build_boundary_lite(
    spokes: &[HalfEdgeId],
    spoke_is_bev: &[bool],
    spoke_dirs: &[Vec3],
    spoke_ends: &[Vec3],
    spoke_off_l: &[f32],
    spoke_off_r: &[f32],
    face_normals: &[Vec3],
    pair_face_normals: &[Vec3],
    v_pos: Vec3,
    params: &BevelParamsLite,
) -> BoundaryResult {
    let n_spokes = spokes.len();
    if n_spokes < 2 {
        return BoundaryResult::default();
    }
    let selcount = spoke_is_bev.iter().filter(|&&b| b).count();
    if selcount == 0 {
        return BoundaryResult::default();
    }

    // Find first beveled edge.
    let efirst_idx = spoke_is_bev.iter().position(|&b| b).unwrap_or(0);
    let mut bnd_verts: Vec<BoundVertLite> = Vec::new();
    let mut edges: Vec<EdgeInfo> = spoke_is_bev
        .iter()
        .map(|&b| EdgeInfo {
            is_bev: b,
            ..Default::default()
        })
        .collect();

    // Miter settings: outer miter only if selcount >= 3.
    let miter_outer = if selcount >= 3 {
        params.miter_outer
    } else {
        MiterKind::Sharp
    };
    let miter_inner = params.miter_inner;

    // Main loop (Blender 3188–3362): iterate over beveled edges.
    let mut e_idx = efirst_idx;
    let mut emiter_idx: Option<usize> = None;
    loop {
        if !spoke_is_bev[e_idx] {
            e_idx = (e_idx + 1) % n_spokes;
            if e_idx == efirst_idx {
                break;
            }
            continue;
        }
        // Find next beveled edge (e2).
        let mut e2_idx = (e_idx + 1) % n_spokes;
        let mut in_plane_count = 0usize;
        let mut not_in_plane_count = 0usize;
        while !spoke_is_bev[e2_idx] {
            let n1 = face_normals
                .get(e2_idx)
                .copied()
                .unwrap_or(Vec3::ZERO)
                .normalize_or_zero();
            let n2 = pair_face_normals
                .get(e2_idx)
                .copied()
                .unwrap_or(Vec3::ZERO)
                .normalize_or_zero();
            let on_plane = n1.length_squared() > 1e-12
                && n2.length_squared() > 1e-12
                && n1.dot(n2) > (1.0 - 0.0152);
            if on_plane {
                in_plane_count += 1;
            } else {
                not_in_plane_count += 1;
            }
            e2_idx = (e2_idx + 1) % n_spokes;
            if e2_idx == e_idx {
                break;
            }
        }

        // Compute boundary vert position (offset_meet or loop_slide).
        let d1 = spoke_dirs[e_idx];
        let d2 = spoke_dirs[e2_idx];
        let off1_r = spoke_off_r.get(e_idx).copied().unwrap_or(params.offset);
        let off2_l = spoke_off_l.get(e2_idx).copied().unwrap_or(params.offset);
        let fn_idx = e_idx.min(face_normals.len().saturating_sub(1));
        let face_no_raw = face_normals.get(fn_idx).copied().unwrap_or(Vec3::Y);
        let face_no = stabilize_face_no(face_no_raw, d1, d2);

        // [LOOP_SLIDE] Blender 3217-3239: slide along intermediate edge if loop_slide enabled
        let mut eon: Option<usize> = None;
        let mut sinratio = 1.0f32;
        let co = if in_plane_count == 0 && not_in_plane_count == 0 {
            offset_meet(v_pos, d1, d2, face_no, off1_r, off2_l)
        } else if not_in_plane_count > 0 && params.loop_slide && not_in_plane_count == 1 {
            // Try to slide along the non-in-plane edge
            let enip_idx = ((e_idx + 1)..e2_idx)
                .find(|&i| !spoke_is_bev[i % n_spokes])
                .map(|i| i % n_spokes);
            if let Some(emid_idx) = enip_idx {
                let d_mid = spoke_dirs[emid_idx];
                let v2_pos = spoke_ends.get(emid_idx).copied().unwrap_or(v_pos + d_mid);
                let bp_full = to_full_params(params);
                if good_offset_on_edge_between(
                    v_pos,
                    face_no,
                    d1,
                    d_mid,
                    d2,
                    params.offset,
                    params.offset,
                ) {
                    let (meetco, sr) = offset_on_edge_between(
                        &bp_full, v_pos, face_no, v2_pos, d1, d2, d_mid, off1_r, off2_l,
                    );
                    if sr.is_some() {
                        eon = Some(emid_idx);
                        sinratio = sr.unwrap_or(1.0);
                    }
                    meetco
                } else {
                    offset_meet(v_pos, d1, d2, face_no, off1_r, off2_l)
                }
            } else {
                offset_meet(v_pos, d1, d2, face_no, off1_r, off2_l)
            }
        } else if in_plane_count > 0
            && not_in_plane_count == 0
            && params.loop_slide
            && in_plane_count == 1
        {
            // Try to slide along the in-plane edge
            let eip_idx = ((e_idx + 1)..e2_idx)
                .find(|&i| !spoke_is_bev[i % n_spokes])
                .map(|i| i % n_spokes);
            if let Some(emid_idx) = eip_idx {
                let d_mid = spoke_dirs[emid_idx];
                let v2_pos = spoke_ends.get(emid_idx).copied().unwrap_or(v_pos + d_mid);
                let bp_full = to_full_params(params);
                if good_offset_on_edge_between(
                    v_pos,
                    face_no,
                    d1,
                    d_mid,
                    d2,
                    params.offset,
                    params.offset,
                ) {
                    let (meetco, sr) = offset_on_edge_between(
                        &bp_full, v_pos, face_no, v2_pos, d1, d2, d_mid, off1_r, off2_l,
                    );
                    if sr.is_some() {
                        eon = Some(emid_idx);
                        sinratio = sr.unwrap_or(1.0);
                    }
                    meetco
                } else {
                    offset_meet(v_pos, d1, d2, face_no, off1_r, off2_l)
                }
            } else {
                offset_meet(v_pos, d1, d2, face_no, off1_r, off2_l)
            }
        } else {
            offset_meet(v_pos, d1, d2, face_no, off1_r, off2_l)
        };

        // Add BoundVert.
        let bv_idx = bnd_verts.len();
        let mut bv = BoundVertLite {
            pos: co,
            prev: if bv_idx > 0 { bv_idx - 1 } else { 0 },
            next: bv_idx + 1,
            index: bv_idx,
            efirst: Some(e_idx),
            elast: Some(e2_idx),
            eon,      // [LOOP_SLIDE] Set eon from loop slide calculation
            sinratio, // [LOOP_SLIDE] Set sinratio from loop slide
            ..Default::default()
        };

        // Assign left/right boundary verts to edges.
        edges[e_idx].left_bv = Some(bv_idx);
        edges[e2_idx].right_bv = Some(bv_idx);
        // In-between edges also point to this bv.
        let mut e3_idx = (e_idx + 1) % n_spokes;
        while e3_idx != e2_idx {
            edges[e3_idx].left_bv = Some(bv_idx);
            edges[e3_idx].right_bv = Some(bv_idx);
            e3_idx = (e3_idx + 1) % n_spokes;
        }

        // Check angle kind for miter (Blender 3255–3330).
        let ang_kind = edges_angle_kind(d1, d2, face_no);
        let do_miter = (miter_outer != MiterKind::Sharp
            && emiter_idx.is_none()
            && ang_kind == AngleKind::Larger)
            || (miter_inner != MiterKind::Sharp && ang_kind == AngleKind::Smaller);

        if do_miter {
            if ang_kind == AngleKind::Larger {
                emiter_idx = Some(e_idx);
            }
            // Create extra boundverts for miter (Blender 3267–3330).
            if ang_kind == AngleKind::Larger && miter_outer == MiterKind::Patch {
                bv.is_patch_start = true;
                bv.elast = Some(e_idx);
                bnd_verts.push(bv.clone());
                // Add v2 (middle of patch).
                let mut v2 = BoundVertLite {
                    pos: co,
                    prev: bv_idx,
                    next: bv_idx + 2,
                    index: bv_idx + 1,
                    ..Default::default()
                };
                let mut v2_elast = None;
                let mut e3_idx = (e_idx + 1) % n_spokes;
                if e3_idx != e2_idx {
                    v2.efirst = Some(e3_idx);
                }
                while e3_idx != e2_idx {
                    edges[e3_idx].left_bv = Some(bv_idx + 1);
                    edges[e3_idx].right_bv = Some(bv_idx + 1);
                    v2_elast = Some(e3_idx);
                    e3_idx = (e3_idx + 1) % n_spokes;
                }
                v2.elast = v2_elast;
                bnd_verts.push(v2);
                // Add v3.
                let v3 = BoundVertLite {
                    pos: co,
                    prev: bv_idx + 1,
                    next: bv_idx + 3,
                    index: bv_idx + 2,
                    efirst: Some(e2_idx),
                    elast: Some(e2_idx),
                    ..Default::default()
                };
                edges[e2_idx].right_bv = Some(bv_idx + 2);
                bnd_verts.push(v3);
            } else {
                // Arc miter.
                bv.is_arc_start = true;
                bv.arc_middle = co;
                // Assign profile_index to in-between edges (Blender 3310–3328).
                let between = in_plane_count + not_in_plane_count;
                let bet2 = between / 2;
                let betodd = (between % 2) == 1;
                let seg = params.seg;
                let mut i = 0usize;
                let mut e3_idx = (e_idx + 1) % n_spokes;
                while e3_idx != e2_idx {
                    bv.elast = Some(e3_idx);
                    if i < bet2 {
                        edges[e3_idx].profile_index = 0;
                    } else if betodd && i == bet2 {
                        edges[e3_idx].profile_index = (seg / 2) as i32;
                    } else {
                        edges[e3_idx].profile_index = seg as i32;
                    }
                    i += 1;
                    e3_idx = (e3_idx + 1) % n_spokes;
                }
                bnd_verts.push(bv.clone());
                // Add v3.
                let v3 = BoundVertLite {
                    pos: co,
                    prev: bv_idx,
                    next: bv_idx + 2,
                    index: bv_idx + 1,
                    efirst: Some(e2_idx),
                    elast: Some(e2_idx),
                    ..Default::default()
                };
                edges[e2_idx].right_bv = Some(bv_idx + 1);
                bnd_verts.push(v3);
            }
        } else {
            bnd_verts.push(bv);
        }

        // Advance to next beveled edge.
        e_idx = e2_idx;
        if e_idx == efirst_idx {
            break;
        }
    }

    // Fix prev/next links to form circular list.
    let n = bnd_verts.len();
    if n > 0 {
        for i in 0..n {
            bnd_verts[i].prev = if i == 0 { n - 1 } else { i - 1 };
            bnd_verts[i].next = if i == n - 1 { 0 } else { i + 1 };
            bnd_verts[i].index = i;
        }
    }

    // set_bound_vert_seams (Blender 2820–2841): compute seam/sharp for each boundvert.
    set_bound_vert_seams(&mut bnd_verts, &edges);

    // [SPREAD] adjust_miter_inner_coords (Blender 3109-3136): spread inner miter verts along edges.
    if params.spread > 0.0 && miter_inner != MiterKind::Sharp {
        adjust_miter_inner_coords(
            &mut bnd_verts,
            &spoke_dirs,
            v_pos,
            params.spread,
            emiter_idx,
        );
    }

    BoundaryResult {
        bnd_verts,
        edges,
        count: n,
    }
}

/// Set seam/sharp flags on BoundVerts based on covered edges (Blender 2820–2841).
pub fn set_bound_vert_seams(bnd_verts: &mut [BoundVertLite], edges: &[EdgeInfo]) {
    for bv in bnd_verts.iter_mut() {
        let mut any_seam = false;
        let mut any_sharp = false;
        let mut seam_len = 0usize;
        let mut sharp_len = 0usize;

        // Iterate from efirst to elast.
        if let (Some(first), Some(last)) = (bv.efirst, bv.elast) {
            let mut e = first;
            loop {
                if let Some(ei) = edges.get(e) {
                    if ei.is_seam {
                        any_seam = true;
                        seam_len += 1;
                    }
                    if ei.is_sharp {
                        any_sharp = true;
                        sharp_len += 1;
                    }
                }
                if e == last {
                    break;
                }
                e = (e + 1) % edges.len();
                if e == first {
                    break;
                } // Safety: prevent infinite loop.
            }
        }

        // Store results. Blender uses seam_len/sharp_len for UV propagation; we store for debug.
        bv.seam_len = seam_len;
        bv.sharp_len = sharp_len;
        // Note: Blender also has bv.any_seam bool, we use seam_len > 0.
        let _ = (any_seam, any_sharp); // Mark as used.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_angle_kind() {
        let d1 = Vec3::X;
        let d2 = Vec3::Y;
        let no = Vec3::Z;
        assert_eq!(edges_angle_kind(d1, d2, no), AngleKind::Smaller);
        assert_eq!(edges_angle_kind(d2, d1, no), AngleKind::Larger);
    }

    #[test]
    fn test_offset_meet_for_smaller_angle_moves_toward_corner_interior() {
        let meet = offset_meet(Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z, 1.0, 1.0);
        let bisector = (Vec3::X + Vec3::Y).normalize_or_zero();
        assert!(meet.dot(bisector) > 0.0);
        assert!(meet.x > 0.0);
        assert!(meet.y > 0.0);
    }

    #[test]
    fn test_offset_meet_stable_under_face_normal_flip() {
        let a = offset_meet(Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z, 1.0, 1.0);
        let b = offset_meet(Vec3::ZERO, Vec3::X, Vec3::Y, -Vec3::Z, 1.0, 1.0);
        assert!((a - b).length() < 1e-5);
    }
}
