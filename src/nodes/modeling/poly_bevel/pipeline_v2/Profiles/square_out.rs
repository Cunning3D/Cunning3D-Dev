//! SquareOut vmesh: complete port of Blender square_out_adj_vmesh (5182–5405).
use super::super::super::structures::Profile;
use super::super::boundary::{edges_angle_kind, AngleKind, BoundVertLite, BoundaryResult};
use super::super::math::{closest_to_line_segment, isect_line_line_v3, nearly_parallel};
use super::super::vmesh::VMeshGrid;
#[allow(unused_imports)]
use crate::libs::geometry::ids::HalfEdgeId;
use bevy::prelude::*;

/// Blender closer_v3_v3v3v3: update dst to be the closer of a or b to ref_pos.
fn closer_v3(dst: &mut Vec3, a: Vec3, b: Vec3, ref_pos: Vec3) {
    let da = (a - ref_pos).length_squared();
    let db = (b - ref_pos).length_squared();
    *dst = if da < db { a } else { b };
}

/// Try to build a SquareOut vmesh. Returns None if preconditions not met.
///
/// # Preconditions (Blender 5527–5529)
/// - Profile is PRO_SQUARE_R
/// - selcount >= 3
/// - segments are even (odd == 0) OR odd is handled
///
/// # Arguments
/// * `boundary` - result from build_boundary_lite
/// * `profiles` - profile start positions for each boundary vert
/// * `profile_middles` - profile middle positions (for arc_start)
/// * `spoke_dirs` - direction from v_pos to each spoke's dest
/// * `v_pos` - vertex position
/// * `ns` - number of segments
pub fn try_square_out_adj_vmesh(
    boundary: &BoundaryResult,
    profiles: &[Vec3],
    profile_middles: &[Vec3],
    spoke_dirs: &[Vec3],
    face_normals: &[Vec3],
    pair_face_normals: &[Vec3],
    v_pos: Vec3,
    ns: usize,
) -> Option<VMeshGrid> {
    let n_bndv = boundary.count;
    if n_bndv < 3 || ns < 2 {
        return None;
    }
    let ns2 = ns / 2;
    let odd = ns % 2 == 1;
    let ns2inv = 1.0 / ns2 as f32;

    let bvs = &boundary.bnd_verts;
    if bvs.len() != n_bndv {
        return None;
    }

    // Centerline storage: for each bndv, store ns2+1 points along the centerline to v_pos.
    let mut centerline: Vec<Vec3> = vec![Vec3::ZERO; n_bndv * (ns2 + 1)];
    let mut cset: Vec<bool> = vec![false; n_bndv];
    let cl = |i: usize, j: usize| -> usize { i * (ns2 + 1) + j };

    // === Pass 1 (5194–5281): set centerline anchors ===
    let mut i = 0usize;
    while i < n_bndv {
        let bv = &bvs[i];
        let bndco = profiles.get(i).copied().unwrap_or(bv.pos);

        // Edge directions for angle_kind classification.
        let e1_idx = bv.efirst.unwrap_or(0);
        let e2_idx = bv.elast.unwrap_or(0);
        let d1 = spoke_dirs.get(e1_idx).copied().unwrap_or(Vec3::X);
        let d2 = spoke_dirs.get(e2_idx).copied().unwrap_or(Vec3::X);
        // Use real face normals from the boundary build (Blender uses adjacent faces around the corner).
        let face_no = {
            let n1 = face_normals.get(e1_idx).copied().unwrap_or(Vec3::ZERO);
            let n2 = pair_face_normals.get(e1_idx).copied().unwrap_or(Vec3::ZERO);
            let n = if n1.length_squared() > 1e-12 { n1 } else { n2 };
            if n.length_squared() > 1e-12 {
                n.normalize_or_zero()
            } else {
                d1.cross(d2).normalize_or_zero()
            }
        };
        let ang_kind = if d1.length_squared() > 1e-12 && d2.length_squared() > 1e-12 {
            edges_angle_kind(
                d1,
                d2,
                if face_no.length_squared() > 1e-12 {
                    face_no
                } else {
                    Vec3::Y
                },
            )
        } else {
            AngleKind::Straight
        };

        // 5206–5215: is_patch_start
        if bv.is_patch_start {
            let inext = bv.next;
            centerline[cl(i, 0)] = (bv.pos + bvs[inext].pos) * 0.5;
            cset[i] = true;
            i += 1;
            if i < n_bndv {
                let bv2 = &bvs[i];
                let inext2 = bv2.next;
                centerline[cl(i, 0)] = (bv2.pos + bvs[inext2].pos) * 0.5;
                cset[i] = true;
                i += 1;
            }
            continue;
        }

        // 5217–5224: is_arc_start
        if bv.is_arc_start {
            centerline[cl(i, 0)] = profile_middles.get(i).copied().unwrap_or(bv.arc_middle);
            cset[i] = true;
            i += 1;
            continue;
        }

        // 5226–5278: ANGLE_SMALLER case
        if ang_kind == AngleKind::Smaller {
            // Intersect e1 with line through bndco parallel to e2 -> v1co.
            let co1 = bndco + d1;
            let co2 = bndco + d2;
            // Infinite lines are scale-invariant for intersection; use stable unit segments.
            let e1_v1 = v_pos;
            let e1_v2 = v_pos + d1;
            let e2_v1 = v_pos;
            let e2_v2 = v_pos + d2;

            let v1co = isect_line_line_v3(e1_v1, e1_v2, bndco, co2).map(|(m, _)| m);
            let v2co = isect_line_line_v3(e2_v1, e2_v2, bndco, co1).map(|(m, _)| m);

            // 5257–5278: v2co competes for on_edge[i], v1co competes for on_edge[i-1].
            if let Some(v2) = v2co {
                let idx = cl(i, 0);
                if cset[i] {
                    let cur = centerline[idx];
                    closer_v3(&mut centerline[idx], cur, v2, v_pos);
                } else {
                    centerline[idx] = v2;
                    cset[i] = true;
                }
            }
            if let Some(v1) = v1co {
                let iprev = if i == 0 { n_bndv - 1 } else { i - 1 };
                let idx = cl(iprev, 0);
                if cset[iprev] {
                    let cur = centerline[idx];
                    closer_v3(&mut centerline[idx], cur, v1, v_pos);
                } else {
                    centerline[idx] = v1;
                    cset[iprev] = true;
                }
            }
        }

        i += 1;
    }

    // === Pass 2 (5283–5316): fill unset centerline anchors ===
    for i in 0..n_bndv {
        if cset[i] {
            continue;
        }
        let bv = &bvs[i];
        let inext = bv.next;
        let iprev = bv.prev;
        let co1 = profiles.get(i).copied().unwrap_or(bv.pos);
        let co2 = profiles.get(inext).copied().unwrap_or(bvs[inext].pos);

        // Edge for this boundvert (efirst of next).
        let e1_idx = bvs[inext].efirst.unwrap_or(0);
        let e1_dir = spoke_dirs.get(e1_idx).copied().unwrap_or(Vec3::X);
        let e1_v1 = v_pos;
        let e1_v2 = v_pos + e1_dir;

        // 5292–5298: if prev and next are arc_start, use line-line.
        if bvs[iprev].is_arc_start && bvs[inext].is_arc_start {
            if let Some((meet, _)) = isect_line_line_v3(e1_v1, e1_v2, co1, co2) {
                centerline[cl(i, 0)] = meet;
                cset[i] = true;
                continue;
            }
        }

        // 5300–5307: closest_to_line_segment.
        let seed = if bvs[iprev].is_arc_start { co1 } else { co2 };
        centerline[cl(i, 0)] = closest_to_line_segment(seed, e1_v1, e1_v2);
        cset[i] = true;
    }

    // Fallback: any still unset -> midpoint.
    for i in 0..n_bndv {
        if !cset[i] {
            let bv = &bvs[i];
            let inext = bv.next;
            let co1 = profiles.get(i).copied().unwrap_or(bv.pos);
            let co2 = profiles.get(inext).copied().unwrap_or(bvs[inext].pos);
            centerline[cl(i, 0)] = (co1 + co2) * 0.5;
        }
    }

    // === Pass 3 (5318–5348): fill centerlines by interpolation to center ===
    let co2 = v_pos;
    for i in 0..n_bndv {
        let mut local_ns2inv = ns2inv;
        if odd {
            // 5323–5338: compute finalfrac based on angle.
            let bv = &bvs[i];
            let inext = bv.next;
            let a = profiles.get(i).copied().unwrap_or(bv.pos);
            let b = profiles.get(inext).copied().unwrap_or(bvs[inext].pos);
            let ang = 0.5 * (a - centerline[cl(i, 0)]).angle_between(b - centerline[cl(i, 0)]);
            let finalfrac = if ang > 0.01 {
                (0.5 / ang.sin()).min(0.8)
            } else {
                0.8
            };
            local_ns2inv = 1.0 / (ns2 as f32 + finalfrac);
        }
        let co1 = centerline[cl(i, 0)];
        for j in 1..=ns2 {
            centerline[cl(i, j)] = co1.lerp(co2, j as f32 * local_ns2inv);
        }
    }

    // === Pass 4 (5350–5367): edge coords and mid-line coords ===
    let mut vm = VMeshGrid::new(n_bndv, ns);
    for i in 0..n_bndv {
        let im1 = if i == 0 { n_bndv - 1 } else { i - 1 };
        let co1 = profiles.get(i).copied().unwrap_or(bvs[i].pos);
        let co2 = centerline[cl(im1, 0)];
        // 5355–5357: j in [0, ns2+odd-1].
        for j in 0..(ns2 + if odd { 1 } else { 0 }) {
            vm.set(i, j, 0, co1.lerp(co2, j as f32 * ns2inv));
        }
        let co2b = centerline[cl(i, 0)];
        // 5359–5361: k in [1, ns2].
        for k in 1..=ns2 {
            vm.set(i, 0, k, co1.lerp(co2b, k as f32 * ns2inv));
        }
    }
    if !odd {
        vm.set(0, ns2, ns2, v_pos);
    }
    vm.copy_equiv();

    // === Pass 5 (5369–5400): interior points via line-line intersection ===
    for i in 0..n_bndv {
        let im1 = if i == 0 { n_bndv - 1 } else { i - 1 };
        for j in 1..(ns2 + if odd { 1 } else { 0 }) {
            for k in 1..=ns2 {
                let a0 = vm.get(i, 0, k);
                let a1 = centerline[cl(im1, k)];
                let b0 = vm.get(i, j, 0);
                let b1 = centerline[cl(i, j)];
                let v = match isect_line_line_v3(a0, a1, b0, b1) {
                    None => a0.lerp(a1, j as f32 * ns2inv), // ikind == 0 fallback
                    Some((m1, m2)) => {
                        if (m1 - m2).length_squared() < 1e-10 {
                            m1
                        }
                        // ikind == 1
                        else {
                            (m1 + m2) * 0.5
                        } // ikind == 2
                    }
                };
                vm.set(i, j, k, v);
            }
        }
    }
    vm.copy_equiv();

    Some(vm)
}

#[cfg(test)]
mod tests {
    use super::super::super::boundary::{BoundVertLite, BoundaryResult};
    use super::*;

    #[test]
    fn test_square_out_basic() {
        // 3 boundverts, 4 segments -> should produce Some(vm).
        let bvs = vec![
            BoundVertLite {
                pos: Vec3::new(1.0, 0.0, 0.0),
                prev: 2,
                next: 1,
                index: 0,
                efirst: Some(0),
                elast: Some(1),
                ..Default::default()
            },
            BoundVertLite {
                pos: Vec3::new(0.0, 1.0, 0.0),
                prev: 0,
                next: 2,
                index: 1,
                efirst: Some(1),
                elast: Some(2),
                ..Default::default()
            },
            BoundVertLite {
                pos: Vec3::new(0.0, 0.0, 1.0),
                prev: 1,
                next: 0,
                index: 2,
                efirst: Some(2),
                elast: Some(0),
                ..Default::default()
            },
        ];
        let boundary = BoundaryResult {
            bnd_verts: bvs.clone(),
            edges: vec![],
            count: 3,
        };
        let profiles: Vec<Vec3> = bvs.iter().map(|b| b.pos).collect();
        let middles: Vec<Vec3> = profiles.clone();
        let dirs = vec![Vec3::X, Vec3::Y, Vec3::Z];
        let v_pos = Vec3::ZERO;
        let face_n = vec![Vec3::Z; 3];
        let pair_n = vec![Vec3::Z; 3];
        let result = try_square_out_adj_vmesh(
            &boundary, &profiles, &middles, &dirs, &face_n, &pair_n, v_pos, 4,
        );
        assert!(result.is_some());
    }
}
