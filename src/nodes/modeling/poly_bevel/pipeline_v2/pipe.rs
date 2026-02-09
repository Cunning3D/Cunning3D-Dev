use super::super::structures::Profile;
use super::boundary::{BoundVertLite, BoundaryResult};
use super::math::*;
use super::vmesh::VMeshGrid;
use crate::libs::geometry::ids::HalfEdgeId;
use bevy::prelude::*;

fn snap_to_superellipsoid(mut co: Vec3, r: f32, midline: bool, eps: &BevelEpsilon) -> Vec3 {
    if (r - PRO_CIRCLE_R).abs() < eps.eps {
        let l2 = co.length_squared();
        if l2 > eps.eps_sq {
            co /= l2.sqrt();
        }
        return co;
    }
    if (r - PRO_LINE_R).abs() < eps.eps {
        return co;
    }
    let (mut x, mut y, mut z) = (co.x.max(0.0), co.y.max(0.0), co.z.max(0.0));
    if (r - PRO_SQUARE_R).abs() < eps.angle_eps || (r - PRO_SQUARE_IN_R).abs() < eps.eps {
        z = 0.0;
        x = x.min(1.0);
        y = y.min(1.0);
        if (r - PRO_SQUARE_R).abs() < eps.angle_eps {
            let dx = 1.0 - x;
            let dy = 1.0 - y;
            if dx < dy {
                x = 1.0;
                y = if midline { 1.0 } else { y };
            } else {
                y = 1.0;
                x = if midline { 1.0 } else { x };
            }
        } else {
            if x < y {
                x = 0.0;
                y = if midline { 0.0 } else { y };
            } else {
                y = 0.0;
                x = if midline { 0.0 } else { x };
            }
        }
        return Vec3::new(x, y, z);
    }
    let rinv = 1.0 / r;
    if x == 0.0 {
        if y == 0.0 {
            z = z.powf(rinv);
        } else {
            y = (1.0 / (1.0 + (z / y).powf(r))).powf(rinv);
            z = z * y / co.y.max(eps.eps_sq);
        }
        return Vec3::new(0.0, y, z);
    }
    x = (1.0 / (1.0 + (y / x).powf(r) + (z / x).powf(r))).powf(rinv);
    y = co.y.max(0.0) * x / co.x.max(eps.eps_sq);
    z = co.z.max(0.0) * x / co.x.max(eps.eps_sq);
    Vec3::new(x, y, z)
}

pub fn snap_to_pipe_profile(
    pro: &Profile,
    edir: Vec3,
    midline: bool,
    co: Vec3,
    eps: &BevelEpsilon,
) -> Vec3 {
    if (pro.start - pro.end).length_squared() < eps.eps_sq {
        return pro.start;
    }
    let n = edir.normalize_or_zero();
    let start_plane = closest_to_plane(pro.start, co, n);
    let end_plane = closest_to_plane(pro.end, co, n);
    let mid_plane = closest_to_plane(pro.middle, co, n);
    if let Some(m) = make_unit_square_map(start_plane, mid_plane, end_plane) {
        let det = m.determinant();
        if det.abs() > eps.eps_sq {
            let p = m.inverse().transform_point3(co);
            return m.transform_point3(snap_to_superellipsoid(p, pro.super_r, midline, eps));
        }
    }
    closest_to_line_segment(co, start_plane, end_plane)
}

pub fn pipe_test(
    n_bndv: usize,
    selcount: usize,
    spokes: &[HalfEdgeId],
    corner_pos: Vec3,
    face_n: &[Vec3],
    bev: &dyn Fn(usize) -> bool,
    he_face: &dyn Fn(HalfEdgeId) -> Option<usize>,
    he_dest_pos: &dyn Fn(HalfEdgeId) -> Option<Vec3>,
    eps: &BevelEpsilon,
) -> Option<(usize, Vec3)> {
    if !(3..=4).contains(&n_bndv) || !(3..=4).contains(&selcount) {
        return None;
    }
    for i in 0..n_bndv {
        let i1 = (i + 1) % n_bndv;
        let i2 = (i + 2) % n_bndv;
        if !(bev(i) && bev(i1) && bev(i2)) {
            continue;
        }
        let other1 = he_dest_pos(spokes[i])?;
        let other3 = he_dest_pos(spokes[i2])?;
        let dir1 = (corner_pos - other1).normalize_or_zero();
        let dir3 = (other3 - corner_pos).normalize_or_zero();
        if dir1.length_squared() < eps.eps_sq || dir3.length_squared() < eps.eps_sq {
            continue;
        }
        if dir1.angle_between(dir3) > eps.angle_eps {
            continue;
        }
        let mut ok = true;
        for &he in spokes {
            if let Some(fi) = he_face(he) {
                if fi < face_n.len()
                    && dir1.dot(face_n[fi].normalize_or_zero()).abs() > eps.angle_eps
                {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            return Some((i, dir1));
        }
    }
    None
}

pub fn pipe_snap(
    vm: &mut VMeshGrid,
    ns: usize,
    n_bndv: usize,
    ipipe: usize,
    pipe_dir: Vec3,
    profiles: &[&Profile],
    eps: &BevelEpsilon,
) {
    let pro = profiles[ipipe % n_bndv];
    let half_ns = ns / 2;
    for i in 0..n_bndv {
        for j in 1..=half_ns {
            for k in 0..=half_ns {
                if !vm.is_canon(i, j, k) {
                    continue;
                }
                let co = vm.get(i, j, k);
                let snapped = snap_to_pipe_profile(pro, pipe_dir, false, co, eps);
                vm.set(i, j, k, snapped);
            }
        }
    }
    vm.copy_equiv();
}

/// Pipe test using BoundaryLite semantics.
/// Returns Some((ipipe, pipe_dir)) if pipe case detected.
pub fn pipe_test_with_boundary(
    boundary: &BoundaryResult,
    spoke_dirs: &[Vec3],
    face_n: &[Vec3],
    eps: &BevelEpsilon,
) -> Option<(usize, Vec3)> {
    let n = boundary.count;
    if !(3..=4).contains(&n) {
        return None;
    }
    let bvs = &boundary.bnd_verts;
    if bvs.len() != n {
        return None;
    }

    // Count beveled edges from boundary (efirst being Some indicates a beveled edge).
    let selcount = bvs.iter().filter(|b| b.efirst.is_some()).count();
    if !(3..=4).contains(&selcount) {
        return None;
    }

    for i in 0..n {
        let i1 = (i + 1) % n;
        let i2 = (i + 2) % n;
        // Check if all three are beveled.
        if bvs[i].efirst.is_none() || bvs[i1].efirst.is_none() || bvs[i2].efirst.is_none() {
            continue;
        }

        // Get spoke directions.
        let e0_idx = bvs[i].efirst.unwrap_or(0);
        let e2_idx = bvs[i2].efirst.unwrap_or(0);
        let dir0 = spoke_dirs.get(e0_idx).copied().unwrap_or(Vec3::X);
        let dir2 = spoke_dirs.get(e2_idx).copied().unwrap_or(Vec3::X);

        // Check if directions are nearly parallel (pipe case).
        if dir0.length_squared() < eps.eps_sq || dir2.length_squared() < eps.eps_sq {
            continue;
        }
        let d0n = dir0.normalize_or_zero();
        let d2n = dir2.normalize_or_zero();
        // Pipe: dir0 and dir2 should be nearly antiparallel (pointing opposite directions).
        if d0n.dot(d2n).abs() < (1.0 - eps.eps) {
            continue;
        }

        // Check that pipe direction is perpendicular to faces.
        let pipe_dir = d0n;
        let mut ok = true;
        for bv in bvs {
            if let Some(ei) = bv.efirst {
                if ei < face_n.len()
                    && pipe_dir.dot(face_n[ei].normalize_or_zero()).abs() > eps.angle_eps
                {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            return Some((i, pipe_dir));
        }
    }
    None
}

#[allow(dead_code)]
/// Placeholder for using BoundVertLite; ensures import is used.
fn _use_boundvert_lite(_: &BoundVertLite) {}
