use bevy::prelude::Vec3;
use crate::libs::algorithms::algorithms_runtime::unity_spline::unity_spline::{BezierCurve, BezierKnot, evaluate_position, split_curve};

#[derive(Clone, Copy, Debug)]
pub struct ResamplePolicy { pub max_error: f32, pub min_seg_len: f32, pub max_segments: usize }
impl Default for ResamplePolicy { fn default() -> Self { Self { max_error: 0.01, min_seg_len: 1e-4, max_segments: 4096 } } }

#[derive(Clone, Debug)]
struct Seg { t0: f32, t1: f32, span: usize, c: BezierCurve }

#[inline] fn dist_point_seg(p: Vec3, a: Vec3, b: Vec3) -> f32 { let ab=b-a; let t=(p-a).dot(ab)/ab.dot(ab).max(1e-20); let t=t.clamp(0.0,1.0); (a+ab*t-p).length() }
#[inline] fn seg_err(s: &Seg) -> f32 { dist_point_seg(evaluate_position(s.c, 0.5), s.c.p0, s.c.p3) }

pub fn resample_bezier_knots(knots: &[BezierKnot], closed: bool, policy: ResamplePolicy) -> (Vec<Vec3>, Vec<f32>, Vec<usize>) {
    let n = knots.len(); if n < 2 { return (Vec::new(), Vec::new(), Vec::new()); }
    let spans = if closed { n } else { n - 1 };
    let mut out_p: Vec<Vec3> = Vec::new();
    let mut out_u: Vec<f32> = Vec::new();
    let mut out_span: Vec<usize> = Vec::new();
    for i in 0..spans {
        let a = knots[i];
        let b = knots[(i + 1) % n];
        let c0 = BezierCurve::from_knots(a, b);
        let (t0, t1) = (i as f32 / spans as f32, (i as f32 + 1.0) / spans as f32);
        let (p, u, sp) = resample_curve_span(Seg { t0, t1, span: i, c: c0 }, policy);
        if out_p.is_empty() { out_p.extend_from_slice(&p); out_u.extend_from_slice(&u); out_span.extend_from_slice(&sp); }
        else { out_p.extend_from_slice(&p[1..]); out_u.extend_from_slice(&u[1..]); out_span.extend_from_slice(&sp[1..]); }
    }
    (out_p, out_u, out_span)
}

fn resample_curve_span(root: Seg, policy: ResamplePolicy) -> (Vec<Vec3>, Vec<f32>, Vec<usize>) {
    let mut segs: Vec<Seg> = vec![root];
    while segs.len() < policy.max_segments {
        let mut worst = None;
        for (i, s) in segs.iter().enumerate() {
            let chord = (s.c.p3 - s.c.p0).length();
            if chord <= policy.min_seg_len { continue; }
            let e = seg_err(s);
            if e <= policy.max_error { continue; }
            worst = match worst { None => Some((i, e)), Some((wi, we)) => if e > we { Some((i, e)) } else { Some((wi, we)) } };
        }
        let Some((idx, _)) = worst else { break; };
        let s = segs.remove(idx);
        let (l, r) = split_curve(s.c, 0.5);
        let tm = (s.t0 + s.t1) * 0.5;
        segs.push(Seg { t0: s.t0, t1: tm, span: s.span, c: l });
        segs.push(Seg { t0: tm, t1: s.t1, span: s.span, c: r });
    }
    segs.sort_by(|a, b| a.t0.partial_cmp(&b.t0).unwrap());
    let mut p: Vec<Vec3> = Vec::with_capacity(segs.len() + 1);
    let mut u: Vec<f32> = Vec::with_capacity(segs.len() + 1);
    let mut sp: Vec<usize> = Vec::with_capacity(segs.len() + 1);
    if let Some(s0) = segs.first() { p.push(s0.c.p0); u.push(s0.t0); sp.push(s0.span); }
    for s in segs { p.push(s.c.p3); u.push(s.t1); sp.push(s.span); }
    (p, u, sp)
}

pub fn resample_polyline(points: &[Vec3], closed: bool, segment_len: f32, max_segments: usize) -> (Vec<Vec3>, Vec<f32>, Vec<usize>) {
    if points.len() < 2 || segment_len <= 0.0 || max_segments == 0 { return (Vec::new(), Vec::new(), Vec::new()); }
    let segs = if closed { points.len() } else { points.len() - 1 };
    let mut out: Vec<Vec3> = Vec::new();
    let mut u: Vec<f32> = Vec::new();
    let mut sp: Vec<usize> = Vec::new();
    let mut acc = 0.0f32;
    let mut total = 0.0f32;
    for i in 0..segs { total += (points[(i + 1) % points.len()] - points[i]).length(); }
    if total <= 1e-20 { return (Vec::new(), Vec::new(), Vec::new()); }
    let mut pushed = 0usize;
    for i in 0..segs {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        let len = (b - a).length();
        let steps = ((len / segment_len).ceil() as usize).clamp(1, max_segments.saturating_sub(pushed).max(1));
        for s in 0..steps {
            if pushed >= max_segments { break; }
            let t = (s as f32) / (steps as f32);
            let p = a.lerp(b, t);
            if out.is_empty() { out.push(p); u.push(acc / total); sp.push(i); pushed += 1; continue; }
            if s == 0 { continue; } // avoid duplicate at segment boundary
            out.push(p); u.push((acc + len * t) / total); sp.push(i); pushed += 1;
        }
        acc += len;
    }
    if closed && out.len() > 2 { if (out[0] - *out.last().unwrap()).length_squared() < 1e-10 { out.pop(); u.pop(); sp.pop(); } }
    (out, u, sp)
}

