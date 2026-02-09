use super::super::structures::ProfileSpacing;
use super::math::*;
use super::spacing::build_profile_spacing;
use super::vmesh::{cubic_subdiv, interp_vmesh, VMeshGrid};
use crate::libs::geometry::ids::HalfEdgeId;
use bevy::prelude::*;

fn snap_to_superellipsoid(mut co: Vec3, r: f32, midline: bool) -> Vec3 {
    if (r - PRO_CIRCLE_R).abs() < 1e-6 {
        let l2 = co.length_squared();
        if l2 > 1e-12 {
            co /= l2.sqrt();
        }
        return co;
    }
    let a = co.x.max(0.0);
    let b = co.y.max(0.0);
    let c = co.z.max(0.0);
    let (mut x, mut y) = (a, b);
    if (r - PRO_SQUARE_R).abs() < 1e-3 || (r - PRO_SQUARE_IN_R).abs() < 1e-6 {
        x = x.min(1.0);
        y = y.min(1.0);
        if (r - PRO_SQUARE_R).abs() < 1e-3 {
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
        return Vec3::new(x, y, 0.0);
    }
    let rinv = 1.0 / r;
    if a == 0.0 {
        if b == 0.0 {
            return Vec3::new(0.0, 0.0, c.powf(rinv));
        }
        y = (1.0 / (1.0 + (c / b).powf(r))).powf(rinv);
        return Vec3::new(0.0, y, c * y / b);
    }
    x = (1.0 / (1.0 + (b / a).powf(r) + (c / a).powf(r))).powf(rinv);
    Vec3::new(x, b * x / a, c * x / a)
}

fn calc_profile_segments(
    start: Vec3,
    _middle: Vec3,
    end: Vec3,
    plane_co: Vec3,
    plane_no: Vec3,
    proj_dir: Vec3,
    use_map: bool,
    map: Mat4,
    xvals: &[f64],
    yvals: &[f64],
    ns: usize,
) -> Vec<Vec3> {
    let project = |co: Vec3| {
        if proj_dir.length_squared() < 1e-12 {
            return co;
        }
        isect_line_plane_v3(co, co + proj_dir, plane_co, plane_no).unwrap_or(co)
    };
    (0..=ns)
        .map(|k| {
            let mut co = if k == 0 {
                start
            } else if k == ns {
                end
            } else if use_map {
                map.transform_point3(Vec3::new(xvals[k] as f32, yvals[k] as f32, 0.0))
            } else {
                start.lerp(end, k as f32 / ns as f32)
            };
            co = project(co);
            co
        })
        .collect()
}

fn get_profile_point(
    prof_co: &[Vec3],
    prof_co_2: &[Vec3],
    seg: usize,
    seg_2: usize,
    i: usize,
    nseg: usize,
) -> Vec3 {
    if seg == 1 {
        return if i == 0 {
            prof_co[0]
        } else {
            *prof_co.last().unwrap_or(&prof_co[0])
        };
    }
    if nseg == seg {
        return prof_co[i.min(seg)];
    }
    let step = (seg_2 / nseg).max(1);
    prof_co_2[(i * step).min(seg_2)]
}

fn make_cube_corner_square(nseg: usize) -> VMeshGrid {
    let ns2 = nseg / 2;
    let mut vm = VMeshGrid::new(3, nseg);
    for i in 0..3 {
        vm.set(
            i,
            0,
            0,
            Vec3::new(
                (i == 0) as i32 as f32,
                (i == 1) as i32 as f32,
                (i == 2) as i32 as f32,
            ),
        );
    }
    for i in 0..3 {
        for j in 0..=ns2 {
            for k in 0..=ns2 {
                if !vm.is_canon(i, j, k) {
                    continue;
                }
                let mut co = Vec3::ZERO;
                co[i] = 1.0;
                co[(i + 1) % 3] = k as f32 * 2.0 / nseg as f32;
                co[(i + 2) % 3] = j as f32 * 2.0 / nseg as f32;
                vm.set(i, j, k, co);
            }
        }
    }
    vm.copy_equiv();
    vm
}

fn make_cube_corner_square_in(nseg: usize) -> VMeshGrid {
    let ns2 = nseg / 2;
    let odd = (nseg & 1) == 1;
    let mut vm = VMeshGrid::new(3, nseg);
    let b = if odd {
        2.0 / (2.0 * ns2 as f32 + std::f32::consts::SQRT_2)
    } else {
        2.0 / nseg as f32
    };
    for i in 0..3 {
        for k in 0..=ns2 {
            let mut co = Vec3::ZERO;
            co[i] = 1.0 - k as f32 * b;
            vm.set(i, 0, k, co);
            let mut co2 = Vec3::ZERO;
            co2[(i + 1) % 3] = 1.0 - k as f32 * b;
            vm.set(i, 0, nseg - k, co2);
        }
    }
    vm.copy_equiv();
    vm
}

fn make_cube_corner_adj_vmesh(nseg: usize, r: f32) -> (VMeshGrid, ProfileSpacing) {
    let pro_spacing = build_profile_spacing(nseg, r);
    if (r - PRO_SQUARE_R).abs() < 1e-3 {
        return (make_cube_corner_square(nseg), pro_spacing);
    }
    if (r - PRO_SQUARE_IN_R).abs() < 1e-6 {
        return (make_cube_corner_square_in(nseg), pro_spacing);
    }

    let mut vm0 = VMeshGrid::new(3, 2);
    let b = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
    ];
    let mut prof_co: [Vec<Vec3>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut prof_co_2: [Vec<Vec3>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let seg_2 = pro_spacing.seg_2.max(4);
    for i in 0..3 {
        let start = b[i];
        let end = b[(i + 1) % 3];
        let mut coc = Vec3::ZERO;
        coc[i] = 1.0;
        coc[(i + 1) % 3] = 1.0;
        coc[(i + 2) % 3] = 0.0;
        let plane_co = start;
        let plane_no = start.cross(end).normalize_or_zero();
        let proj_dir = plane_no;
        let map_opt = make_unit_square_map(start, coc, end);
        let use_map = map_opt.is_some();
        let map = map_opt.unwrap_or(Mat4::IDENTITY);
        prof_co[i] = calc_profile_segments(
            start,
            coc,
            end,
            plane_co,
            plane_no,
            proj_dir,
            use_map,
            map,
            &pro_spacing.xvals,
            &pro_spacing.yvals,
            nseg,
        );
        prof_co_2[i] = if seg_2 == nseg {
            prof_co[i].clone()
        } else {
            calc_profile_segments(
                start,
                coc,
                end,
                plane_co,
                plane_no,
                proj_dir,
                use_map,
                map,
                &pro_spacing.xvals_2,
                &pro_spacing.yvals_2,
                seg_2,
            )
        };
        vm0.set(i, 0, 0, start);
        vm0.set(
            i,
            0,
            1,
            get_profile_point(&prof_co[i], &prof_co_2[i], nseg, seg_2, 1, 2),
        );
        vm0.set(i, 0, 2, end);
    }
    let mut center = Vec3::splat((1.0f32 / 3.0).sqrt());
    if nseg > 2 {
        if r > 1.5 {
            center *= 1.4;
        } else if r < 0.75 {
            center *= 0.6;
        }
    }
    vm0.set(0, 1, 1, center);
    vm0.copy_equiv();
    let prof_at = |i: usize, k: usize, ns: usize| -> Vec3 {
        get_profile_point(&prof_co[i % 3], &prof_co_2[i % 3], nseg, seg_2, k, ns)
    };
    let mut vm1 = vm0;
    while vm1.ns < nseg {
        vm1 = cubic_subdiv(vm1, &prof_at);
    }
    if vm1.ns != nseg {
        vm1 = interp_vmesh(vm1, &prof_at, nseg);
    }
    let ns2 = nseg / 2;
    for i in 0..3 {
        for j in 0..=ns2 {
            for k in 0..=nseg {
                if !vm1.is_canon(i, j, k) {
                    continue;
                }
                vm1.set(i, j, k, snap_to_superellipsoid(vm1.get(i, j, k), r, false));
            }
        }
    }
    vm1.copy_equiv();
    (vm1, pro_spacing)
}

pub fn tri_corner_test(
    spokes: &[HalfEdgeId],
    selcount: usize,
    r: f32,
    offsets: &[f32],
    is_bev: &[bool],
    face_n: &[Vec3],
    he_pair: &dyn Fn(HalfEdgeId) -> HalfEdgeId,
    he_face: &dyn Fn(HalfEdgeId) -> Option<usize>,
    he_dir: &dyn Fn(HalfEdgeId) -> Option<Vec3>,
) -> i32 {
    if selcount != 3 {
        return 0;
    }
    let edgecount = spokes.len();
    if edgecount == 0 || offsets.len() < edgecount || is_bev.len() < edgecount {
        return -1;
    }
    let mut in_plane_e = 0usize;
    let mut totang = 0.0f32;
    let mut base_off = 0.0f32;
    for i in 0..edgecount {
        if is_bev[i] {
            base_off = offsets[i];
            break;
        }
    }
    for (i, &he) in spokes.iter().enumerate().take(edgecount) {
        let pair = he_pair(he);
        if !pair.is_valid() {
            return -1;
        }
        let fi1 = he_face(he).unwrap_or(usize::MAX);
        let fi2 = he_face(pair).unwrap_or(usize::MAX);
        if fi1 >= face_n.len() || fi2 >= face_n.len() {
            return -1;
        }
        let n1 = face_n[fi1].normalize_or_zero();
        let n2 = face_n[fi2].normalize_or_zero();
        let e = he_dir(he).unwrap_or(Vec3::X).normalize_or_zero();
        let ang = (n1.cross(n2).dot(e)).atan2(n1.dot(n2));
        let absang = ang.abs();
        if absang <= std::f32::consts::FRAC_PI_4 {
            in_plane_e += 1;
        } else if absang >= 3.0 * std::f32::consts::FRAC_PI_4 {
            return -1;
        }
        if is_bev[i] && (offsets[i] - base_off).abs() > 1e-4 {
            return -1;
        }
        totang += ang;
    }
    if in_plane_e != edgecount.saturating_sub(3) {
        return -1;
    }
    let angdiff = (totang.abs() - 3.0 * std::f32::consts::FRAC_PI_2).abs();
    let lim = if (r - PRO_SQUARE_R).abs() < 1e-3 {
        std::f32::consts::PI / 16.0
    } else {
        std::f32::consts::FRAC_PI_4
    };
    if angdiff > lim {
        return -1;
    }
    if edgecount != 3 || selcount != 3 {
        return 0;
    }
    1
}

pub fn tri_corner_vmesh(
    bound0: Vec3,
    bound1: Vec3,
    bound2: Vec3,
    corner_pos: Vec3,
    ns: usize,
    r: f32,
) -> Option<VMeshGrid> {
    if ns <= 1 {
        return None;
    }
    let (base, _ps) = make_cube_corner_adj_vmesh(ns, r);
    let mat = make_unit_cube_map(bound0, bound1, bound2, corner_pos);
    let mut out = base.clone();
    for i in 0..out.n {
        for j in 0..=out.ns2 {
            for k in 0..=out.ns {
                if !out.is_canon(i, j, k) {
                    continue;
                }
                out.set(i, j, k, mat.transform_point3(base.get(i, j, k)));
            }
        }
    }
    out.copy_equiv();
    Some(out)
}
