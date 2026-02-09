use super::super::structures::Profile;
use super::custom_profile::CustomProfile;
use super::math::*;
use super::spacing::build_profile_spacing;
use bevy::prelude::*;

pub fn build_profile_samples(
    seg: usize,
    invert: bool,
    pro_r: f32,
    custom_profile: Option<&CustomProfile>,
    start: Vec3,
    end: Vec3,
    u_pos: Vec3,
    v_pos: Vec3,
    selcount: usize,
    prev_dir: Option<Vec3>,
    next_dir: Option<Vec3>,
) -> (Profile, super::super::structures::ProfileSpacing) {
    let mut p = Profile::default();
    p.start = start;
    p.end = end;
    let mut do_linear = true;
    let pro_spacing = build_profile_spacing(seg, pro_r);
    if seg > 1 {
        do_linear = false;
        p.super_r = pro_r;
        p.proj_dir = (v_pos - u_pos).normalize_or_zero();
        if p.proj_dir.length_squared() < 1e-12 {
            p.proj_dir = Vec3::Y;
        }
        p.middle = project_to_edge(u_pos, v_pos, start, end);
        let mut d1 = (p.middle - start).normalize_or_zero();
        let mut d2 = (p.middle - end).normalize_or_zero();
        p.plane_no = d1.cross(d2).normalize_or_zero();
        if p.plane_no.length_squared() < 1e-12 || nearly_parallel(d1, d2) {
            p.middle = u_pos;
            if selcount >= 3 {
                if let (Some(pd), Some(nd)) = (prev_dir, next_dir) {
                    if nearly_parallel(pd, nd) {
                        p.middle = start.lerp(end, 0.5);
                        do_linear = true;
                    } else if let Some((meet, _)) =
                        isect_line_line_v3(start, start + pd, end, end + nd)
                    {
                        p.middle = meet;
                    } else {
                        p.middle = start.lerp(end, 0.5);
                        do_linear = true;
                    }
                } else {
                    p.middle = start.lerp(end, 0.5);
                    do_linear = true;
                }
            } else {
                p.middle = start.lerp(end, 0.5);
                do_linear = true;
            }
            d1 = (p.middle - start).normalize_or_zero();
            d2 = (p.middle - end).normalize_or_zero();
            p.plane_no = d1.cross(d2).normalize_or_zero();
            if p.plane_no.length_squared() < 1e-12 || nearly_parallel(d1, d2) {
                do_linear = true;
            } else {
                p.plane_co = u_pos;
                p.proj_dir = p.plane_no;
            }
        }
        p.plane_co = start;
    }
    if do_linear {
        p.super_r = PRO_LINE_R;
        p.middle = u_pos;
        p.plane_co = Vec3::ZERO;
        p.plane_no = Vec3::ZERO;
        p.proj_dir = Vec3::ZERO;
    }

    let map = if p.super_r != PRO_LINE_R {
        make_unit_square_map(p.start, p.middle, p.end)
    } else {
        None
    };
    let calc = |ns: usize, xvals: &[f64], yvals: &[f64]| -> Vec<Vec3> {
        let use_map = map.is_some() && p.super_r != PRO_LINE_R;
        let project = |co: Vec3| -> Vec3 {
            if p.proj_dir.length_squared() < 1e-12 || p.plane_no.length_squared() < 1e-12 {
                return co;
            }
            isect_line_plane_v3(co, co + p.proj_dir, p.plane_co, p.plane_no).unwrap_or(co)
        };
        let chord_mid = (p.start + p.end) * 0.5;
        (0..=ns)
            .map(|k| {
                let mut co = if k == 0 {
                    p.start
                } else if k == ns {
                    p.end
                } else if use_map {
                    if let Some(cp) = custom_profile {
                        let t = k as f32 / ns as f32;
                        let s = cp.sample_arc_length(t);
                        map.unwrap().transform_point3(Vec3::new(s.x, s.y, 0.0))
                    } else {
                        map.unwrap().transform_point3(Vec3::new(
                            xvals[k] as f32,
                            yvals[k] as f32,
                            0.0,
                        ))
                    }
                } else {
                    p.start.lerp(p.end, k as f32 / ns as f32)
                };
                co = project(co);
                if invert && k != 0 && k != ns {
                    co = chord_mid - (co - chord_mid);
                }
                co
            })
            .collect()
    };

    p.prof_co = calc(seg, &pro_spacing.xvals, &pro_spacing.yvals);
    p.prof_co_2 = if pro_spacing.seg_2 == seg {
        p.prof_co.clone()
    } else {
        calc(
            pro_spacing.seg_2,
            &pro_spacing.xvals_2,
            &pro_spacing.yvals_2,
        )
    };
    (p, pro_spacing)
}
