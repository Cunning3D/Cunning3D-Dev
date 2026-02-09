use bevy::math::{Mat3, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

pub const DEFAULT_TENSION: f32 = 1.0 / 3.0;
pub const CATMULL_ROM_TENSION: f32 = 1.0 / 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TangentMode { AutoSmooth = 0, Linear = 1, Mirrored = 2, Continuous = 3, Broken = 4 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BezierTangent { In, Out }

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BezierKnot { pub position: Vec3, pub tangent_in: Vec3, pub tangent_out: Vec3, pub rotation: Quat }

impl Default for BezierKnot { fn default() -> Self { Self { position: Vec3::ZERO, tangent_in: Vec3::ZERO, tangent_out: Vec3::ZERO, rotation: Quat::IDENTITY } } }

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BezierCurve { pub p0: Vec3, pub p1: Vec3, pub p2: Vec3, pub p3: Vec3 }

impl BezierCurve {
    #[inline] pub fn from_knots(a: BezierKnot, b: BezierKnot) -> Self {
        Self { p0: a.position, p1: a.position + a.rotation.mul_vec3(a.tangent_out), p2: b.position + b.rotation.mul_vec3(b.tangent_in), p3: b.position }
    }
    #[inline] pub fn tangent0(&self) -> Vec3 { self.p1 - self.p0 }
    #[inline] pub fn tangent1(&self) -> Vec3 { self.p2 - self.p3 }
}

#[inline] fn clamp01(t: f32) -> f32 { t.clamp(0.0, 1.0) }
#[inline] fn rsqrt(x: f32) -> f32 { 1.0 / x.sqrt() }
#[inline] fn isfinite(x: f32) -> bool { x.is_finite() }

#[inline]
pub fn evaluate_position(curve: BezierCurve, t: f32) -> Vec3 {
    let t = clamp01(t); let t2 = t * t; let t3 = t2 * t;
    curve.p0 * (-1.0 * t3 + 3.0 * t2 - 3.0 * t + 1.0) +
    curve.p1 * ( 3.0 * t3 - 6.0 * t2 + 3.0 * t) +
    curve.p2 * (-3.0 * t3 + 3.0 * t2) +
    curve.p3 * ( t3 )
}

#[inline]
pub fn evaluate_tangent(curve: BezierCurve, t: f32) -> Vec3 {
    let t = clamp01(t); let t2 = t * t;
    curve.p0 * (-3.0 * t2 + 6.0 * t - 3.0) +
    curve.p1 * ( 9.0 * t2 - 12.0 * t + 3.0) +
    curve.p2 * (-9.0 * t2 + 6.0 * t) +
    curve.p3 * ( 3.0 * t2 )
}

#[inline]
pub fn split_curve(curve: BezierCurve, t: f32) -> (BezierCurve, BezierCurve) {
    let t = clamp01(t);
    let split0 = curve.p0.lerp(curve.p1, t);
    let split1 = curve.p1.lerp(curve.p2, t);
    let split2 = curve.p2.lerp(curve.p3, t);
    let split3 = split0.lerp(split1, t);
    let split4 = split1.lerp(split2, t);
    let split5 = split3.lerp(split4, t);
    (BezierCurve { p0: curve.p0, p1: split0, p2: split3, p3: split5 }, BezierCurve { p0: split5, p1: split4, p2: split2, p3: curve.p3 })
}

#[inline]
pub fn are_tangents_modifiable(mode: TangentMode) -> bool {
    matches!(mode, TangentMode::Broken | TangentMode::Continuous | TangentMode::Mirrored)
}
#[inline]
pub fn get_explicit_linear_tangent(point: Vec3, to: Vec3) -> Vec3 { (to - point) / 3.0 }

#[inline]
pub fn get_explicit_linear_tangent_knots(from: BezierKnot, to: BezierKnot) -> Vec3 {
    from.rotation.inverse().mul_vec3((to.position - from.position) * (1.0/3.0))
}

#[inline]
pub fn get_auto_smooth_tangent(previous: Vec3, next: Vec3, tension: f32) -> Vec3 {
    if next == previous { return Vec3::ZERO; }
    let d = next - previous;
    d / d.length().sqrt() * tension
}

#[inline]
pub fn get_auto_smooth_tangent3(previous: Vec3, current: Vec3, next: Vec3, tension: f32) -> Vec3 {
    let d1 = (current - previous).length();
    let d2 = (next - current).length();
    if d1 == 0.0 { return (next - current) * 0.1; }
    if d2 == 0.0 { return (current - previous) * 0.1; }
    let a = tension; let two_a = 2.0 * tension;
    let d1a = d1.powf(a); let d12a = d1.powf(two_a);
    let d2a = d2.powf(a); let d22a = d2.powf(two_a);
    (d12a * next - d22a * previous + (d22a - d12a) * current) / (3.0 * d1a * (d1a + d2a))
}

#[inline]
pub fn look_rotation_safe(forward: Vec3, up: Vec3) -> Quat {
    let f2 = forward.dot(forward);
    let u2 = up.dot(up);
    let f = forward * rsqrt(f2);
    let u = up * rsqrt(u2);
    let mut t = u.cross(f);
    let t2 = t.dot(t);
    t *= rsqrt(t2);
    let mn = f2.min(u2).min(t2);
    let mx = f2.max(u2).max(t2);
    let accept = mn > 1e-35 && mx < 1e35 && isfinite(f2) && isfinite(u2) && isfinite(t2);
    if !accept { return Quat::IDENTITY; }
    let m = Mat3::from_cols(t, f.cross(t), f);
    Quat::from_mat3(&m)
}

#[inline]
pub fn get_knot_rotation(mut tangent: Vec3, normal: Vec3) -> Quat {
    if tangent.length_squared() == 0.0 {
        let n = if normal.length_squared() == 0.0 { Vec3::Y } else { normal.normalize() };
        tangent = Quat::from_rotation_arc(Vec3::Y, n).mul_vec3(Vec3::Z);
    }
    let tn = tangent.normalize_or_zero();
    let nn = normal.normalize_or_zero();
    let colinear = (tn.dot(nn).abs() - 1.0).abs() < 1e-6;
    let up = if colinear { tn.cross(Vec3::X).normalize_or_zero() } else { (normal - tn * normal.dot(tn)).normalize_or_zero() };
    look_rotation_safe(tn, up)
}

impl BezierKnot {
    #[inline]
    pub fn bake_tangent_direction_to_rotation(self, mirrored: bool, main: BezierTangent) -> Self {
        let up = self.rotation.mul_vec3(Vec3::Y);
        let lead = if main == BezierTangent::In { self.tangent_in.length() } else { self.tangent_out.length() };
        if mirrored {
            let dir = self.rotation.mul_vec3(if main == BezierTangent::In { -self.tangent_in } else { self.tangent_out });
            return Self { position: self.position, tangent_in: Vec3::new(0.0, 0.0, -lead), tangent_out: Vec3::new(0.0, 0.0, lead), rotation: get_knot_rotation(dir, up) };
        }
        let dir = self.rotation.mul_vec3(if main == BezierTangent::In { -self.tangent_in } else { self.tangent_out });
        Self { position: self.position, tangent_in: Vec3::new(0.0, 0.0, -self.tangent_in.length()), tangent_out: Vec3::new(0.0, 0.0, self.tangent_out.length()), rotation: get_knot_rotation(dir, up) }
    }

    #[inline]
    pub fn transform(self, matrix: Mat4) -> Self {
        let (_s, mrot, _t) = matrix.to_scale_rotation_translation();
        let rotation = mrot * self.rotation;
        let inv_rotation = rotation.inverse();
        let tin = self.rotation.mul_vec3(self.tangent_in);
        let tout = self.rotation.mul_vec3(self.tangent_out);
        Self {
            position: matrix.transform_point3(self.position),
            tangent_in: inv_rotation.mul_vec3(matrix.transform_vector3(tin)),
            tangent_out: inv_rotation.mul_vec3(matrix.transform_vector3(tout)),
            rotation,
        }
    }
}

#[inline]
pub fn get_auto_smooth_knot(position: Vec3, previous: Vec3, next: Vec3, normal: Vec3, tension: f32) -> BezierKnot {
    let tan_in = get_auto_smooth_tangent3(next, position, previous, tension);
    let tan_out = get_auto_smooth_tangent3(previous, position, next, tension);
    let mut dir_in = Vec3::new(0.0, 0.0, tan_in.length());
    let mut dir_out = Vec3::new(0.0, 0.0, tan_out.length());
    let mut dir_rot = tan_out;
    if dir_in.z == 0.0 { dir_in.z = dir_out.z; }
    if dir_out.z == 0.0 { dir_out.z = dir_in.z; dir_rot = -tan_in; }
    BezierKnot { position, tangent_in: -dir_in, tangent_out: dir_out, rotation: get_knot_rotation(dir_rot, normal) }
}

#[inline]
pub fn apply_tangent_mode(knot: BezierKnot, mode: TangentMode, previous: Vec3, next: Vec3, tension: f32, main: BezierTangent) -> BezierKnot {
    match mode {
        TangentMode::Continuous => knot.bake_tangent_direction_to_rotation(false, main),
        TangentMode::Mirrored => knot.bake_tangent_direction_to_rotation(true, main),
        TangentMode::Linear => BezierKnot { tangent_in: Vec3::ZERO, tangent_out: Vec3::ZERO, ..knot },
        TangentMode::AutoSmooth => get_auto_smooth_knot(knot.position, previous, next, knot.rotation.mul_vec3(Vec3::Y), tension),
        TangentMode::Broken => knot,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DistanceToInterpolation { pub distance: f32, pub t: f32 }

pub const INVALID_DTI: DistanceToInterpolation = DistanceToInterpolation { distance: -1.0, t: -1.0 };
pub const CURVE_DISTANCE_LUT_RESOLUTION: usize = 30;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetaData { pub mode: TangentMode, pub tension: f32, pub lut: [DistanceToInterpolation; CURVE_DISTANCE_LUT_RESOLUTION], pub ups: [Vec3; CURVE_DISTANCE_LUT_RESOLUTION] }

impl MetaData {
    #[inline] pub fn new(mode: TangentMode, tension: f32) -> Self { Self { mode, tension, lut: [INVALID_DTI; CURVE_DISTANCE_LUT_RESOLUTION], ups: [Vec3::ZERO; CURVE_DISTANCE_LUT_RESOLUTION] } }
    #[inline] pub fn invalidate(&mut self) { self.lut[0] = INVALID_DTI; self.ups[0] = Vec3::ZERO; }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spline { pub knots: Vec<BezierKnot>, pub closed: bool, pub meta: Vec<MetaData>, length: f32 }

impl Default for Spline { fn default() -> Self { Self { knots: Vec::new(), closed: false, meta: Vec::new(), length: -1.0 } } }

impl Spline {
    #[inline] fn ensure_meta_valid(&mut self) { while self.meta.len() < self.knots.len() { self.meta.push(MetaData::new(TangentMode::Broken, CATMULL_ROM_TENSION)); } }
    #[inline] fn invalidate_all(&mut self) { self.length = -1.0; for m in self.meta.iter_mut() { m.invalidate(); } }

    #[inline] pub fn count(&self) -> usize { self.knots.len() }
    #[inline] pub fn previous_index(&self, index: usize) -> usize { let c=self.count(); if c==0 {0} else if self.closed { (index + c - 1) % c } else { index.saturating_sub(1) } }
    #[inline] pub fn next_index(&self, index: usize) -> usize { let c=self.count(); if c==0 {0} else if self.closed { (index + 1) % c } else { (index + 1).min(c - 1) } }

    #[inline] pub fn get_curve(&self, index: usize) -> BezierCurve {
        let c=self.count(); if c==0 { return BezierCurve{p0:Vec3::ZERO,p1:Vec3::ZERO,p2:Vec3::ZERO,p3:Vec3::ZERO}; }
        let next = if self.closed { (index + 1) % c } else { (index + 1).min(c - 1) };
        BezierCurve::from_knots(self.knots[index], self.knots[next])
    }

    #[inline] pub fn get_curve_length(&mut self, index: usize) -> f32 {
        self.ensure_meta_valid();
        if self.meta[index].lut[0].distance < 0.0 { calculate_curve_lengths(self.get_curve(index), &mut self.meta[index].lut); }
        self.meta[index].lut[CURVE_DISTANCE_LUT_RESOLUTION - 1].distance
    }

    #[inline] pub fn get_length(&mut self) -> f32 {
        if self.length < 0.0 {
            let c = self.count();
            self.length = 0.0;
            if c > 1 {
                let n = if self.closed { c } else { c - 1 };
                for i in 0..n { self.length += self.get_curve_length(i); }
            }
        }
        self.length
    }

    #[inline] pub fn get_curve_interpolation(&mut self, curve_index: usize, curve_distance: f32) -> f32 {
        self.ensure_meta_valid();
        if self.meta[curve_index].lut[0].distance < 0.0 { calculate_curve_lengths(self.get_curve(curve_index), &mut self.meta[curve_index].lut); }
        get_distance_to_interpolation(&self.meta[curve_index].lut, curve_distance)
    }

            pub fn get_curve_up_vector(&mut self, index: usize, t: f32) -> Vec3 {
        self.ensure_meta_valid();
        let need = self.meta[index].ups[0] == Vec3::ZERO;
        if need {
            let next = self.next_index(index);
            let start_up = self.knots[index].rotation.mul_vec3(Vec3::Y);
            let end_up = self.knots[next].rotation.mul_vec3(Vec3::Y);
            let curve = self.get_curve(index);
            let ups = &mut self.meta[index].ups;
            evaluate_up_vectors(curve, start_up, end_up, ups);
        }
        let ups = &self.meta[index].ups;
        let offset = 1.0 / ((ups.len() - 1) as f32);
        let mut curve_t = 0.0;
        for i in 0..ups.len() {
            if i + 1 >= ups.len() { break; }
            if t <= curve_t + offset {
                return ups[i].lerp(ups[i + 1], (t - curve_t) / offset);
            }
            curve_t += offset;
        }
        ups[ups.len() - 1]
    }    
    pub fn warmup(&mut self) { let _ = self.get_length(); self.warm_up_curve_ups(); }

    fn warm_up_curve_ups(&mut self) {
        self.ensure_meta_valid();
        let c = self.count();
        if c <= 1 { return; }
        let n = if self.closed { c } else { c - 1 };
        for i in 0..n {
            let _ = self.get_curve_up_vector(i, 0.0);
        }
    }

    pub fn enforce_tangent_mode_no_notify(&mut self, index: usize) { self.ensure_meta_valid(); self.apply_tangent_mode_no_notify(index, BezierTangent::Out); }

    pub fn enforce_tangent_mode_range_no_notify(&mut self, start: usize, count: usize) {
        self.ensure_meta_valid();
        let end = (start + count).min(self.count());
        for i in start..end { self.apply_tangent_mode_no_notify(i, BezierTangent::Out); }
    }

    pub fn set_knot(&mut self, index: usize, value: BezierKnot, main: BezierTangent) { self.set_knot_no_notify(index, value, main); self.invalidate_all(); }

    pub fn set_knot_no_notify(&mut self, index: usize, value: BezierKnot, main: BezierTangent) {
        self.ensure_meta_valid();
        if index >= self.count() { return; }
        self.knots[index] = value;
        self.apply_tangent_mode_no_notify(index, main);

        // Setting knot position affects tangents of neighbor AutoSmooth knots.
        let p = self.previous_index(index);
        let n = self.next_index(index);
        if self.meta.get(p).map(|m| m.mode == TangentMode::AutoSmooth).unwrap_or(false) { self.apply_tangent_mode_no_notify(p, main); }
        if self.meta.get(n).map(|m| m.mode == TangentMode::AutoSmooth).unwrap_or(false) { self.apply_tangent_mode_no_notify(n, main); }
    }

    pub fn evaluate(&mut self, t: f32) -> Option<(Vec3, Vec3, Vec3)> {
        if self.count() < 1 { return None; }
        let (curve_index, curve_t) = self.spline_to_curve_t(t, true);
        let curve = self.get_curve(curve_index);
        let pos = evaluate_position(curve, curve_t);
        let tan = evaluate_tangent(curve, curve_t);
        let up = self.get_curve_up_vector(curve_index, curve_t);
        Some((pos, tan, up))
    }

    pub fn evaluate_position(&mut self, t: f32) -> Option<Vec3> { self.evaluate(t).map(|x| x.0) }
    pub fn evaluate_tangent(&mut self, t: f32) -> Option<Vec3> { self.evaluate(t).map(|x| x.1) }
    pub fn evaluate_up_vector(&mut self, t: f32) -> Option<Vec3> { self.evaluate(t).map(|x| x.2) }

    pub fn curve_to_spline_t(&mut self, curve: f32) -> f32 {
        if self.count() <= 1 || curve < 0.0 { return 0.0; }
        let max_curve = if self.closed { self.count() as f32 } else { (self.count() - 1) as f32 };
        if curve >= max_curve { return 1.0; }
        let curve_index = curve.floor() as usize;
        let frac = curve.fract();
        let mut dist = 0.0;
        for i in 0..curve_index { dist += self.get_curve_length(i); }
        dist += self.get_curve_length(curve_index) * frac;
        let len = self.get_length();
        if len == 0.0 { 0.0 } else { dist / len }
    }
#[inline] pub fn spline_to_curve_t(&mut self, spline_t: f32, use_lut: bool) -> (usize, f32) {
        let knot_count = self.count();
        if knot_count <= 1 { return (0, 0.0); }
        let spline_t = spline_t.clamp(0.0, 1.0);
        let t_length = spline_t * self.get_length();
        let mut start = 0.0;
        let closed = self.closed;
        let c = if closed { knot_count } else { knot_count - 1 };
        for i in 0..c {
            let index = i % knot_count;
            let curve_len = self.get_curve_length(index);
            if t_length <= start + curve_len {
                let ct = if use_lut { self.get_curve_interpolation(index, t_length - start) } else { (t_length - start) / curve_len };
                return (index, ct);
            }
            start += curve_len;
        }
        (if closed { knot_count - 1 } else { knot_count - 2 }, 1.0)
    }

    #[inline] pub fn get_tangent_mode(&mut self, index: usize) -> TangentMode { self.ensure_meta_valid(); self.meta[index].mode }

    pub fn set_tangent_mode_no_notify(&mut self, index: usize, mode: TangentMode, mut main: BezierTangent) {
        self.ensure_meta_valid();
        if self.meta[index].mode == mode { return; }
        if index == self.count().saturating_sub(1) && !self.closed { main = BezierTangent::In; }
        let mut knot = self.knots[index];
        if self.meta[index].mode == TangentMode::Linear && (mode as u8) >= (TangentMode::Mirrored as u8) {
            let p = self.previous_index(index);
            let n = self.next_index(index);
            knot.tangent_in = get_explicit_linear_tangent_knots(knot, self.knots[p]);
            knot.tangent_out = get_explicit_linear_tangent_knots(knot, self.knots[n]);
        }
        self.meta[index].mode = mode;
        self.knots[index] = knot;
        self.apply_tangent_mode_no_notify(index, main);
    }

    pub fn apply_tangent_mode_no_notify(&mut self, index: usize, main: BezierTangent) {
        self.ensure_meta_valid();
        let p = self.previous_index(index);
        let n = self.next_index(index);
        let mode = self.meta[index].mode;
        let tension = self.meta[index].tension;
        let knot = self.knots[index];
        self.knots[index] = apply_tangent_mode(knot, mode, self.knots[p].position, self.knots[n].position, tension, main);
        self.meta[index].invalidate();
        self.length = -1.0;
    }

    pub fn insert(&mut self, index: usize, knot: BezierKnot, mode: TangentMode, tension: f32) {
        self.ensure_meta_valid();
        let idx = index.min(self.count());
        self.knots.insert(idx, knot);
        self.meta.insert(idx, MetaData::new(mode, tension));
        let prev = self.previous_index(idx);
        if prev != idx { self.apply_tangent_mode_no_notify(prev, BezierTangent::Out); }
        self.apply_tangent_mode_no_notify(idx, BezierTangent::Out);
        let next = self.next_index(idx);
        if next != idx { self.apply_tangent_mode_no_notify(next, BezierTangent::Out); }
        self.invalidate_all();
    }
    pub fn insert_on_curve(&mut self, index: usize, curve_t: f32) {
        let count = self.count();
        if count < 2 || index >= count { return; }
        self.ensure_meta_valid();

        let prev_index = self.previous_index(index);
        let mut previous = self.knots[prev_index];
        let mut next = self.knots[index];

        let curve_to_split = BezierCurve::from_knots(previous, self.knots[index]);
        let (left_curve, right_curve) = split_curve(curve_to_split, curve_t);

        if self.meta[prev_index].mode == TangentMode::Mirrored { self.set_tangent_mode_no_notify(prev_index, TangentMode::Continuous, BezierTangent::Out); previous = self.knots[prev_index]; }
        if self.meta[index].mode == TangentMode::Mirrored { self.set_tangent_mode_no_notify(index, TangentMode::Continuous, BezierTangent::Out); next = self.knots[index]; }

        if are_tangents_modifiable(self.meta[prev_index].mode) { previous.tangent_out = previous.rotation.inverse().mul_vec3(left_curve.tangent0()); }
        if are_tangents_modifiable(self.meta[index].mode) { next.tangent_in = next.rotation.inverse().mul_vec3(right_curve.tangent1()); }

        let up = evaluate_up_vector(curve_to_split, curve_t, previous.rotation.mul_vec3(Vec3::Y), self.knots[self.next_index(index)].rotation.mul_vec3(Vec3::Y));
        let rotation = look_rotation_safe(right_curve.tangent0().normalize_or_zero(), up);
        let inv_rotation = rotation.inverse();

        self.set_knot_no_notify(prev_index, previous, BezierTangent::Out);
        self.set_knot_no_notify(index, next, BezierTangent::Out);

        let bezier_knot = BezierKnot { position: left_curve.p3, tangent_in: inv_rotation.mul_vec3(left_curve.tangent1()), tangent_out: inv_rotation.mul_vec3(right_curve.tangent0()), rotation };
        self.insert(index, bezier_knot, TangentMode::Broken, CATMULL_ROM_TENSION);
    }

    pub fn remove_at(&mut self, index: usize) {
        self.ensure_meta_valid();
        if self.count() == 0 || index >= self.count() { return; }
        self.knots.remove(index);
        self.meta.remove(index);
        let next = index.min(self.count().saturating_sub(1));
        if self.count() > 0 {
            let p = self.previous_index(next);
            self.apply_tangent_mode_no_notify(p, BezierTangent::Out);
            self.apply_tangent_mode_no_notify(next, BezierTangent::Out);
        }
        self.invalidate_all();
    }

    pub fn set_auto_smooth_tension_no_notify(&mut self, index: usize, tension: f32) {
        self.ensure_meta_valid();
        self.meta[index].tension = tension;
        if self.meta[index].mode == TangentMode::AutoSmooth { self.apply_tangent_mode_no_notify(index, BezierTangent::Out); }
        self.invalidate_all();
    }

    pub fn set_tangent_length(&mut self, index: usize, tangent: BezierTangent, length: f32) {
        self.ensure_meta_valid();
        if index >= self.count() { return; }
        let mode = self.meta[index].mode;
        if !are_tangents_modifiable(mode) { return; }
        let len = length.max(0.0);
        let mut k = self.knots[index];
        match mode {
            TangentMode::Mirrored => {
                k.tangent_in = Vec3::new(0.0, 0.0, -len);
                k.tangent_out = Vec3::new(0.0, 0.0, len);
            }
            TangentMode::Continuous => {
                if tangent == BezierTangent::In { k.tangent_in = Vec3::new(0.0, 0.0, -len); } else { k.tangent_out = Vec3::new(0.0, 0.0, len); }
            }
            TangentMode::Broken => {
                let v = if tangent == BezierTangent::In { k.tangent_in } else { k.tangent_out };
                let nv = if v.length_squared() < 1e-12 { Vec3::new(0.0, 0.0, if tangent == BezierTangent::In { -len } else { len }) } else { v.normalize_or_zero() * len };
                if tangent == BezierTangent::In { k.tangent_in = nv; } else { k.tangent_out = nv; }
            }
            _ => {}
        }
        self.set_knot(index, k, tangent);
    }
}

#[inline]
pub fn calculate_curve_lengths(curve: BezierCurve, lut: &mut [DistanceToInterpolation; CURVE_DISTANCE_LUT_RESOLUTION]) {
    let mut magnitude = 0.0;
    let mut prev = evaluate_position(curve, 0.0);
    lut[0] = DistanceToInterpolation { distance: 0.0, t: 0.0 };
    let denom = (CURVE_DISTANCE_LUT_RESOLUTION as f32) - 1.0;
    for i in 1..CURVE_DISTANCE_LUT_RESOLUTION {
        let t = (i as f32) / denom;
        let point = evaluate_position(curve, t);
        magnitude += (point - prev).length();
        lut[i] = DistanceToInterpolation { distance: magnitude, t };
        prev = point;
    }
}

#[inline]
pub fn get_distance_to_interpolation(lut: &[DistanceToInterpolation; CURVE_DISTANCE_LUT_RESOLUTION], distance: f32) -> f32 {
    if distance <= 0.0 { return 0.0; }
    let curve_len = lut[CURVE_DISTANCE_LUT_RESOLUTION - 1].distance;
    if distance >= curve_len { return 1.0; }
    let mut prev = lut[0];
    for i in 1..CURVE_DISTANCE_LUT_RESOLUTION {
        let cur = lut[i];
        if distance < cur.distance {
            let denom = cur.distance - prev.distance;
            return if denom == 0.0 { cur.t } else { prev.t + (cur.t - prev.t) * ((distance - prev.distance) / denom) };
        }
        prev = cur;
    }
    1.0
}

pub const NORMALS_PER_CURVE: usize = 16;
const UP_EPSILON: f32 = 0.0001;

#[derive(Clone, Copy, Debug, Default)]
struct FrenetFrame { origin: Vec3, tangent: Vec3, normal: Vec3, binormal: Vec3 }

#[inline]
fn approximately(a: f32, b: f32) -> bool {
    (b - a).abs() < (0.000001 * a.abs().max(b.abs())).max(UP_EPSILON * 8.0)
}

fn vec3_slerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    let la = a.length();
    let lb = b.length();
    if la == 0.0 || lb == 0.0 { return a.lerp(b, t); }
    let an = a / la;
    let bn = b / lb;
    let dot = an.dot(bn).clamp(-1.0, 1.0);
    let omega = dot.acos();
    if omega.abs() < 1e-6 { return a.lerp(b, t); }
    let sin_om = omega.sin();
    let s0 = ((1.0 - t) * omega).sin() / sin_om;
    let s1 = (t * omega).sin() / sin_om;
    let dir = an * s0 + bn * s1;
    let l = la + (lb - la) * t;
    dir * l
}

#[inline]
fn get_next_rotation_minimizing_frame(curve: BezierCurve, prev: FrenetFrame, next_t: f32) -> FrenetFrame {
    let mut next = FrenetFrame::default();
    next.origin = evaluate_position(curve, next_t);
    next.tangent = evaluate_tangent(curve, next_t);

    let to_cur = next.origin - prev.origin;
    let c1 = to_cur.dot(to_cur);
    let ri_l = prev.binormal - to_cur * (2.0 / c1) * to_cur.dot(prev.binormal);
    let ti_l = prev.tangent - to_cur * (2.0 / c1) * to_cur.dot(prev.tangent);

    let v2 = next.tangent - ti_l;
    let c2 = v2.dot(v2);

    next.binormal = (ri_l - v2 * (2.0 / c2) * v2.dot(ri_l)).normalize();
    next.normal = next.binormal.cross(next.tangent).normalize();
    next
}

/// Unity Editor equivalent of TransformOperation.CalculateMirroredTangentTranslationDeltas.
/// This computes how a knot rotation and tangent magnitude should change to move a mirrored/continuous tangent handle
/// to a target world-space position while respecting a spline local-to-world transform (including non-uniform scale).
#[inline]
pub fn calculate_mirrored_tangent_translation_deltas(
    spline_local_to_world: Mat4,
    knot_rotation: Quat,
    knot_world_pos: Vec3,
    tangent_world_pos: Vec3,
    tangent_local_dir: Vec3,
    tangent: BezierTangent,
    target_world_pos: Vec3,
) -> (Quat, f32) {
    let inv = spline_local_to_world.inverse();
    let (_s, spline_rot, spline_pos) = spline_local_to_world.to_scale_rotation_translation();

    let unscaled_target = spline_pos + spline_rot.mul_vec3(inv.transform_point3(target_world_pos));
    let unscaled_current = spline_pos + spline_rot.mul_vec3(inv.transform_point3(tangent_world_pos));
    let unscaled_knot = spline_pos + spline_rot.mul_vec3(inv.transform_point3(knot_world_pos));

    let knot_rot_inv = knot_rotation.inverse();
    let sign = if tangent == BezierTangent::In { -1.0 } else { 1.0 };
    let forward = sign * (unscaled_target - unscaled_knot).normalize_or_zero();
    let up = knot_rotation.mul_vec3(Vec3::Y);
    let look = look_rotation_safe(forward, up);
    let rot_delta = look * knot_rot_inv;

    let target_local = knot_rot_inv.mul_vec3(unscaled_target - unscaled_knot);
    let mag_delta = target_local.length() - tangent_local_dir.length();

    // Note: unscaled_current is kept for parity with Unity implementation (debugging), even if unused here.
    let _ = unscaled_current;

    (rot_delta, mag_delta)
}

/// Unity Editor equivalent of EditorSplineUtility.ApplyPositionToTangent.
#[inline]
pub fn apply_position_to_tangent(
    spline_local_to_world: Mat4,
    knot: &mut BezierKnot,
    mode: TangentMode,
    tangent: BezierTangent,
    knot_world_pos: Vec3,
    tangent_world_pos: Vec3,
    target_world_pos: Vec3,
) {
    match mode {
        TangentMode::Broken => {
            // Broken tangents are freely editable: set local tangent vector directly from target.
            let inv = spline_local_to_world.inverse();
            let (_s, spline_rot, spline_pos) = spline_local_to_world.to_scale_rotation_translation();
            let unscaled_target = spline_pos + spline_rot.mul_vec3(inv.transform_point3(target_world_pos));
            let unscaled_knot = spline_pos + spline_rot.mul_vec3(inv.transform_point3(knot_world_pos));
            let local = knot.rotation.inverse().mul_vec3(unscaled_target - unscaled_knot);
            if tangent == BezierTangent::In { knot.tangent_in = local; } else { knot.tangent_out = local; }
        }
        TangentMode::Continuous | TangentMode::Mirrored => {
            let local_dir = if tangent == BezierTangent::In { knot.tangent_in } else { knot.tangent_out };
            let (rot_delta, mag_delta) = calculate_mirrored_tangent_translation_deltas(
                spline_local_to_world,
                knot.rotation,
                knot_world_pos,
                tangent_world_pos,
                local_dir,
                tangent,
                target_world_pos,
            );
            knot.rotation = rot_delta * knot.rotation;
            let mut ld = if local_dir.length() == 0.0 { Vec3::new(0.0, 0.0, 1.0) } else { local_dir };
            ld += ld.normalize_or_zero() * mag_delta;
            if tangent == BezierTangent::In { knot.tangent_in = ld; } else { knot.tangent_out = ld; }
        }
        _ => {}
    }
}pub fn evaluate_up_vectors(curve: BezierCurve, start_up: Vec3, end_up: Vec3, ups: &mut [Vec3; CURVE_DISTANCE_LUT_RESOLUTION]) {
    ups[0] = start_up;
    ups[CURVE_DISTANCE_LUT_RESOLUTION - 1] = end_up;
    for i in 1..(CURVE_DISTANCE_LUT_RESOLUTION - 1) {
        let tt = (i as f32) / ((CURVE_DISTANCE_LUT_RESOLUTION - 1) as f32);
        ups[i] = evaluate_up_vector(curve, tt, ups[0], end_up);
    }
}

#[inline]
pub fn get_curve_middle_interpolation(curve: BezierCurve) -> f32 {
    let mut lut = [DistanceToInterpolation { distance: 0.0, t: 0.0 }; CURVE_DISTANCE_LUT_RESOLUTION];
    calculate_curve_lengths(curve, &mut lut);
    get_distance_to_interpolation(&lut, lut[CURVE_DISTANCE_LUT_RESOLUTION - 1].distance * 0.5)
}

#[inline]
pub fn get_inserted_knot_preview(spline: &mut Spline, index: usize, t: f32) -> (BezierKnot, Vec3, Vec3) {
    let previous_index = spline.previous_index(index);
    let previous = spline.knots[previous_index];
    let curve_to_split = BezierCurve::from_knots(previous, spline.knots[index]);
    let (left_curve, right_curve) = split_curve(curve_to_split, t);

    let next_index = spline.next_index(index);
    let next = spline.knots[next_index];

    let up = evaluate_up_vector(curve_to_split, t, previous.rotation.mul_vec3(Vec3::Y), next.rotation.mul_vec3(Vec3::Y));
    let rotation = look_rotation_safe(right_curve.tangent0().normalize_or_zero(), up);
    let inv_rotation = rotation.inverse();

    let left_out_tangent = left_curve.tangent0();
    let right_in_tangent = right_curve.tangent1();

    (BezierKnot { position: left_curve.p3, tangent_in: inv_rotation.mul_vec3(left_curve.tangent1()), tangent_out: inv_rotation.mul_vec3(right_curve.tangent0()), rotation }, left_out_tangent, right_in_tangent)
}

#[inline]
pub fn get_preview_curve_internal(
    spline: &Spline,
    spline_local_to_world: Mat4,
    from: usize,
    from_local_tangent: Vec3,
    to_world_point: Vec3,
    to_world_tangent: Vec3,
    to_mode: TangentMode,
    previous_index: usize,
) -> BezierCurve {
    let a_mode = spline.meta[from].mode;
    let b_mode = to_mode;
    let from_k = spline.knots[from];

    let p0 = spline_local_to_world.transform_point3(from_k.position);
    let p1_raw = from_k.position + from_k.rotation.mul_vec3(from_local_tangent);
    let mut p1 = spline_local_to_world.transform_point3(p1_raw);
    let p3 = to_world_point;
    let mut p2 = p3 - to_world_tangent;

    if !are_tangents_modifiable(a_mode) {
        p1 = if a_mode == TangentMode::Linear { p0 } else {
            let prev = spline_local_to_world.transform_point3(spline.knots[previous_index].position);
            p0 + get_auto_smooth_tangent3(prev, p0, p3, CATMULL_ROM_TENSION)
        };
    }
    if !are_tangents_modifiable(b_mode) {
        p2 = if b_mode == TangentMode::Linear { p3 } else { p3 + get_auto_smooth_tangent3(p3, p3, p0, CATMULL_ROM_TENSION) };
    }
    BezierCurve { p0, p1, p2, p3 }
}

#[inline]
pub fn get_preview_curve_from_end(container: &SplineContainer, spline_index: usize, from: usize, to_world_point: Vec3, to_world_tangent: Vec3, to_mode: TangentMode) -> BezierCurve {
    let spline = &container.splines[spline_index];
    let mut tangent_out = spline.knots[from].tangent_out;
    if spline.closed && (from == 0 || container.are_knot_linked(SplineKnotIndex::new(spline_index as i32, from as i32), SplineKnotIndex::new(spline_index as i32, 0))) {
        let fk = spline.knots[from];
        tangent_out = -fk.tangent_in;
    }
    get_preview_curve_internal(spline, container.local_to_world, from, tangent_out, to_world_point, to_world_tangent, to_mode, spline.previous_index(from))
}

#[inline]
pub fn get_preview_curve_from_start(container: &SplineContainer, spline_index: usize, from: usize, to_world_point: Vec3, to_world_tangent: Vec3, to_mode: TangentMode) -> BezierCurve {
    let spline = &container.splines[spline_index];
    let mut tangent_in = spline.knots[from].tangent_in;
    if spline.closed && (from == spline.count().saturating_sub(1) || container.are_knot_linked(SplineKnotIndex::new(spline_index as i32, from as i32), SplineKnotIndex::new(spline_index as i32, (spline.count().saturating_sub(1)) as i32))) {
        let fk = spline.knots[from];
        tangent_in = -fk.tangent_out;
    }
    get_preview_curve_internal(spline, container.local_to_world, from, tangent_in, to_world_point, to_world_tangent, to_mode, spline.next_index(from))
}

#[inline]
pub fn mode_from_placement_tangent(tangent: Vec3, default_mode: TangentMode) -> TangentMode {
    if tangent.length_squared() < f32::EPSILON { default_mode } else { TangentMode::Mirrored }
}

#[inline]
pub fn calculate_knot_rotation(previous: Vec3, position: Vec3, next: Vec3, normal: Vec3) -> Quat {
    let mut tangent = Vec3::new(0.0, 0.0, 1.0);
    let has_prev = (position - previous).length_squared() > f32::EPSILON;
    let has_next = (next - position).length_squared() > f32::EPSILON;
    if has_prev && has_next { tangent = ((position - previous) + (next - position)) * 5.0; }
    else if has_prev { tangent = position - previous; }
    else if has_next { tangent = next - position; }
    get_knot_rotation(tangent, normal)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SelectableKnot { pub spline_index: usize, pub knot_index: usize }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrawingDirection { Start, End }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SelectableTangent { pub spline_index: usize, pub knot_index: usize, pub tangent: BezierTangent }

impl SelectableTangent {
    #[inline]
    fn knot_local_to_world(container: &SplineContainer, spline_index: usize, knot_index: usize) -> Mat4 {
        let k = container.splines[spline_index].knots[knot_index];
        Mat4::from_rotation_translation(k.rotation, k.position)
    }

    #[inline]
    pub fn local_to_world(container: &SplineContainer, spline_index: usize, knot_index: usize) -> Mat4 {
        container.local_to_world * Self::knot_local_to_world(container, spline_index, knot_index)
    }

    #[inline]
    pub fn set_direction_world(container: &mut SplineContainer, spline_index: usize, knot_index: usize, tangent: BezierTangent, dir_world: Vec3) {
        // Match Unity SelectableTangent.Direction setter: LocalDirection = inv(LocalToWorld) * dir_world
        if spline_index >= container.splines.len() || knot_index >= container.splines[spline_index].count() { return; }
        let l2w = Self::local_to_world(container, spline_index, knot_index);
        let local_dir = l2w.inverse().transform_vector3(dir_world);
        let mut k = container.splines[spline_index].knots[knot_index];
        if tangent == BezierTangent::In { k.tangent_in = local_dir; } else { k.tangent_out = local_dir; }
        container.splines[spline_index].set_knot(knot_index, k, tangent);
        container.set_linked_knot_position(SplineKnotIndex::new(spline_index as i32, knot_index as i32));
    }
}

impl SplineContainer {
    pub fn add_knot_to_end(&mut self, spline_index: usize, world_position: Vec3, normal_local: Vec3, tangent_out_world: Vec3, default_mode: TangentMode) -> Option<SelectableKnot> {
        let s = self.splines.get(spline_index)?;
        if s.closed && s.count() >= 2 { return None; }
        let index = self.splines[spline_index].count();
        let previous_index = index.saturating_sub(1);
        self.add_knot_internal(spline_index, world_position, normal_local, tangent_out_world, index, previous_index, default_mode)
    }

    pub fn add_knot_to_start(&mut self, spline_index: usize, world_position: Vec3, normal_local: Vec3, tangent_in_world: Vec3, default_mode: TangentMode) -> Option<SelectableKnot> {
        let s = self.splines.get(spline_index)?;
        if s.closed && s.count() >= 2 { return None; }
        // Unity passes -tangentIn as tangentOut when adding to start.
        self.add_knot_internal(spline_index, world_position, normal_local, -tangent_in_world, 0, 1, default_mode)
    }

    fn add_knot_internal(&mut self, spline_index: usize, world_position: Vec3, normal_local: Vec3, tangent_out_world: Vec3, index: usize, previous_index: usize, default_mode: TangentMode) -> Option<SelectableKnot> {
        let spline_local_to_world = self.local_to_world;
        let inv = spline_local_to_world.inverse();
        let local_position = inv.transform_point3(world_position);
        let mode = mode_from_placement_tangent(tangent_out_world, default_mode);

        let new_knot = if !are_tangents_modifiable(mode) {
            let prev_pos = if previous_index < self.splines[spline_index].count() { self.splines[spline_index].knots[previous_index].position } else { local_position };
            get_auto_smooth_knot(local_position, prev_pos, local_position, normal_local, CATMULL_ROM_TENSION)
        } else {
            let (_s, world_rot, _t) = spline_local_to_world.to_scale_rotation_translation();
            let local_rotation = world_rot.inverse() * look_rotation_safe(tangent_out_world, normal_local);
            let mag = tangent_out_world.length();
            BezierKnot { position: local_position, tangent_in: Vec3::new(0.0, 0.0, -mag), tangent_out: Vec3::new(0.0, 0.0, mag), rotation: local_rotation }
        };

        self.links.knot_inserted(SplineKnotIndex::new(spline_index as i32, index as i32));
        self.splines[spline_index].insert(index, new_knot, mode, CATMULL_ROM_TENSION);

        // Update previous knot rotation if previous knot is not modifiable (AutoSmooth/Linear). Matches Unity comment.
        if self.splines[spline_index].count() > 1 && previous_index < self.splines[spline_index].count() && !are_tangents_modifiable(self.splines[spline_index].meta[previous_index].mode) {
            let p = self.splines[spline_index].previous_index(previous_index);
            let n = self.splines[spline_index].next_index(previous_index);
            let mut cur = self.splines[spline_index].knots[previous_index];
            let prev = self.splines[spline_index].knots[p];
            let next = self.splines[spline_index].knots[n];
            cur.rotation = calculate_knot_rotation(prev.position, cur.position, next.position, normal_local);
            self.splines[spline_index].set_knot(previous_index, cur, BezierTangent::Out);
            self.set_linked_knot_position(SplineKnotIndex::new(spline_index as i32, previous_index as i32));
        }

        Some(SelectableKnot { spline_index, knot_index: index })
    }

    pub fn unclose_spline_if_needed(&mut self, spline_index: usize, dir: DrawingDirection) {
        if spline_index >= self.splines.len() { return; }
        if !self.splines[spline_index].closed { return; }
        self.splines[spline_index].closed = false;
        let count = self.splines[spline_index].count();
        if count == 0 { return; }
        match dir {
            DrawingDirection::Start => {
                let last = count - 1;
                let k = self.splines[spline_index].knots[last];
                let normal = k.rotation.mul_vec3(Vec3::Y);
                let tan = -k.rotation.mul_vec3(k.tangent_out);
                let _ = self.add_knot_to_start(spline_index, self.local_to_world.transform_point3(k.position), normal, tan, TangentMode::AutoSmooth);
                self.link_knots(SplineKnotIndex::new(spline_index as i32, 0), SplineKnotIndex::new(spline_index as i32, (self.splines[spline_index].count() - 1) as i32));
            }
            DrawingDirection::End => {
                let k = self.splines[spline_index].knots[0];
                let normal = k.rotation.mul_vec3(Vec3::Y);
                let tan = -k.rotation.mul_vec3(k.tangent_in);
                let _ = self.add_knot_to_end(spline_index, self.local_to_world.transform_point3(k.position), normal, tan, TangentMode::AutoSmooth);
                self.link_knots(SplineKnotIndex::new(spline_index as i32, 0), SplineKnotIndex::new(spline_index as i32, (self.splines[spline_index].count() - 1) as i32));
            }
        }
    }

    pub fn create_knot_on_knot(&mut self, spline_index: usize, dir: DrawingDirection, clicked: SelectableKnot, tangent_out_world: Vec3) {
        if spline_index >= self.splines.len() { return; }
        let close_knot_index = match dir { DrawingDirection::End => 0, DrawingDirection::Start => self.splines[spline_index].count().saturating_sub(1) };
        let close_ski = SplineKnotIndex::new(spline_index as i32, close_knot_index as i32);
        let clicked_ski = SplineKnotIndex::new(clicked.spline_index as i32, clicked.knot_index as i32);

        if clicked.spline_index == spline_index && (clicked.knot_index == close_knot_index || self.are_knot_linked(clicked_ski, close_ski)) {
            self.splines[spline_index].closed = true;
            let did_draw_tangent = tangent_out_world.length_squared() > f32::EPSILON;
            let mode = self.splines[spline_index].meta[close_knot_index].mode;
            if did_draw_tangent || mode == TangentMode::AutoSmooth {
                self.splines[spline_index].set_tangent_mode_no_notify(close_knot_index, TangentMode::Broken, BezierTangent::Out);
            }
            if did_draw_tangent {
                let t = match dir { DrawingDirection::Start => BezierTangent::Out, DrawingDirection::End => BezierTangent::In };
                SelectableTangent::set_direction_world(self, spline_index, close_knot_index, t, -tangent_out_world);
            }
            return;
        }

        self.unclose_spline_if_needed(spline_index, dir);
        let normal = self.splines[clicked.spline_index].knots[clicked.knot_index].rotation.mul_vec3(Vec3::Y);
        let pos = self.local_to_world.transform_point3(self.splines[clicked.spline_index].knots[clicked.knot_index].position);
        let last = match dir { DrawingDirection::Start => self.add_knot_to_start(spline_index, pos, normal, tangent_out_world, TangentMode::AutoSmooth), DrawingDirection::End => self.add_knot_to_end(spline_index, pos, normal, tangent_out_world, TangentMode::AutoSmooth) };
        if let Some(k) = last {
            // Mirror Unity behavior: link clicked to last added depending on direction and whether clicked is on same spline.
            if dir == DrawingDirection::End || clicked.spline_index != k.spline_index {
                self.link_knots(clicked_ski, SplineKnotIndex::new(k.spline_index as i32, k.knot_index as i32));
            } else {
                self.link_knots(SplineKnotIndex::new(clicked.spline_index as i32, (clicked.knot_index + 1) as i32), SplineKnotIndex::new(k.spline_index as i32, k.knot_index as i32));
            }
        }
    }

    pub fn create_knot_on_surface(&mut self, spline_index: usize, dir: DrawingDirection, world_position: Vec3, normal: Vec3, tangent_out_world: Vec3) {
        if spline_index >= self.splines.len() { return; }
        // Unity does incremental snap against last added knot; we keep it deterministic by requiring the caller to pass snapped position if desired.
        self.unclose_spline_if_needed(spline_index, dir);
        let _ = match dir {
            DrawingDirection::Start => self.add_knot_to_start(spline_index, world_position, normal, tangent_out_world, TangentMode::AutoSmooth),
            DrawingDirection::End => self.add_knot_to_end(spline_index, world_position, normal, tangent_out_world, TangentMode::AutoSmooth),
        };
    }
}

#[derive(Clone, Debug)]
pub struct AffectedCurvePreview { pub spline_index: usize, pub curve_index: usize, pub knots: Vec<BezierKnot> }

pub fn get_affected_curves_insert_on_segment(
    container: &SplineContainer,
    spline_index: usize,
    prev_knot_index: usize,
    next_knot_index: usize,
    curve_t: f32,
    hit_local_position: Vec3,
) -> Vec<AffectedCurvePreview> {
    let spline = &container.splines[spline_index];
    if spline.count() == 0 { return Vec::new(); }

    let mut out: Vec<AffectedCurvePreview> = Vec::new();
    let curve_index = prev_knot_index;

    let prev_k = spline.knots[prev_knot_index];
    let next_k = spline.knots[next_knot_index];
    let inserted = {
        let mut tmp = spline.clone();
        get_inserted_knot_preview(&mut tmp, next_knot_index, curve_t).0
    };
    let (_, left_tan, right_tan) = {
        let mut tmp = spline.clone();
        get_inserted_knot_preview(&mut tmp, next_knot_index, curve_t)
    };

    let mut preview_knots: Vec<BezierKnot> = Vec::new();
    let mut b_knot = prev_k;
    if spline.meta[prev_knot_index].mode == TangentMode::AutoSmooth {
        let previous_knot_index = spline.previous_index(prev_knot_index);
        let previous_knot = spline.knots[previous_knot_index];
        b_knot = get_auto_smooth_knot(prev_k.position, previous_knot.position, hit_local_position, Vec3::Y, CATMULL_ROM_TENSION);
        out.push(AffectedCurvePreview { spline_index, curve_index: previous_knot_index, knots: vec![previous_knot, b_knot] });
    } else {
        b_knot.tangent_out = b_knot.rotation.inverse().mul_vec3(left_tan);
    }
    preview_knots.push(b_knot);
    preview_knots.push(inserted);
    out.push(AffectedCurvePreview { spline_index, curve_index, knots: preview_knots.clone() });

    let mut b2 = next_k;
    if spline.meta[next_knot_index].mode == TangentMode::AutoSmooth {
        let next_next_index = spline.next_index(next_knot_index);
        let next_next = spline.knots[next_next_index];
        b2 = get_auto_smooth_knot(next_k.position, hit_local_position, next_next.position, Vec3::Y, CATMULL_ROM_TENSION);
        out.push(AffectedCurvePreview { spline_index, curve_index: next_knot_index, knots: vec![b2, next_next] });
    } else {
        b2.tangent_in = b2.rotation.inverse().mul_vec3(right_tan);
    }
    preview_knots.push(b2);
    // Unity doesn't push the final preview curve entry for the last knot here; we keep parity with their list shape.
    out
}

pub fn get_affected_curves_add_knot(
    spline: &Spline,
    spline_index: usize,
    knot_position: Vec3,
    adding_to_start: bool,
    last_knot_index: usize,
    previous_knot_index: usize,
    existing: &mut Vec<AffectedCurvePreview>,
) {
    if spline.count() == 0 { return; }
    if last_knot_index >= spline.count() || previous_knot_index >= spline.count() { return; }
    let affected_idx = existing.iter().position(|x| x.spline_index == spline_index && x.curve_index == if adding_to_start { last_knot_index } else { previous_knot_index });
    if spline.meta[last_knot_index].mode != TangentMode::AutoSmooth { return; }

    let previous_knot = spline.knots[previous_knot_index];
    let auto = if adding_to_start {
        get_auto_smooth_knot(spline.knots[last_knot_index].position, knot_position, previous_knot.position, Vec3::Y, CATMULL_ROM_TENSION)
    } else {
        get_auto_smooth_knot(spline.knots[last_knot_index].position, previous_knot.position, knot_position, Vec3::Y, CATMULL_ROM_TENSION)
    };

    match affected_idx {
        None => {
            if adding_to_start {
                existing.insert(0, AffectedCurvePreview { spline_index, curve_index: last_knot_index, knots: vec![auto, previous_knot] });
            } else {
                existing.push(AffectedCurvePreview { spline_index, curve_index: previous_knot_index, knots: vec![previous_knot, auto] });
            }
        }
        Some(i) => {
            let k = &mut existing[i].knots;
            let idx = if adding_to_start { 0 } else { 1 };
            if idx < k.len() { k[idx] = auto; }
        }
    }
}

impl Spline {
    pub fn clear_tangent(&mut self, knot_index: usize, tangent: BezierTangent) {
        if knot_index >= self.count() { return; }
        self.ensure_meta_valid();
        if self.meta[knot_index].mode == TangentMode::Mirrored { self.set_tangent_mode_no_notify(knot_index, TangentMode::Continuous, BezierTangent::Out); }
        let mut k = self.knots[knot_index];
        if tangent == BezierTangent::In { k.tangent_in = Vec3::ZERO; } else { k.tangent_out = Vec3::ZERO; }
        self.set_knot(knot_index, k, BezierTangent::Out);
    }
}

#[inline]
pub fn evaluate_up_vector(mut curve: BezierCurve, t: f32, start_up: Vec3, end_up: Vec3) -> Vec3 {
    let linear_len = get_explicit_linear_tangent(curve.p0, curve.p3).length();
    let dir = (curve.p3 - curve.p0).normalize_or_zero();
    let linear_out = dir * linear_len;
    if approximately((curve.p1 - curve.p0).length(), 0.0) { curve.p1 = curve.p0 + linear_out; }
    if approximately((curve.p2 - curve.p3).length(), 0.0) { curve.p2 = curve.p3 - linear_out; }

    let mut normals = [Vec3::ZERO; NORMALS_PER_CURVE];

    let mut frame = FrenetFrame::default();
    frame.origin = curve.p0;
    frame.tangent = curve.p1 - curve.p0;
    frame.normal = start_up;
    frame.binormal = frame.tangent.cross(frame.normal).normalize_or_zero();
    if !frame.binormal.is_finite() || frame.binormal.length_squared() == 0.0 { return Vec3::ZERO; }

    normals[0] = frame.normal;

    let step = 1.0 / ((NORMALS_PER_CURVE - 1) as f32);
    let mut cur_t = step;
    let mut prev_t = 0.0;
    let mut upv = Vec3::ZERO;

    for i in 1..NORMALS_PER_CURVE {
        let prev_frame = frame;
        frame = get_next_rotation_minimizing_frame(curve, prev_frame, cur_t);
        normals[i] = frame.normal;
        if prev_t <= t && cur_t >= t {
            let lerp_t = (t - prev_t) / step;
            upv = vec3_slerp(prev_frame.normal, frame.normal, lerp_t);
        }
        prev_t = cur_t;
        cur_t += step;
    }

    if prev_t <= t && cur_t >= t { upv = end_up; }

    let last_normal = normals[NORMALS_PER_CURVE - 1];
    let mut angle = last_normal.dot(end_up).clamp(-1.0, 1.0).acos();
    if angle == 0.0 { return upv; }

    let axis = frame.tangent.normalize_or_zero();
    let pos_r = Quat::from_axis_angle(axis, angle);
    let neg_r = Quat::from_axis_angle(axis, -angle);
    let pos_res = pos_r.mul_vec3(end_up).dot(last_normal).clamp(-1.0, 1.0).acos();
    let neg_res = neg_r.mul_vec3(end_up).dot(last_normal).clamp(-1.0, 1.0).acos();
    if pos_res > neg_res { angle *= -1.0; }

    cur_t = step;
    prev_t = 0.0;

    for i in 1..NORMALS_PER_CURVE {
        let normal = normals[i];
        let adj = angle * cur_t;
        let tan = evaluate_tangent(curve, cur_t).normalize_or_zero();
        let rot = Quat::from_axis_angle(tan, -adj);
        let adjusted = rot.mul_vec3(normal);
        normals[i] = adjusted;
        if prev_t <= t && cur_t >= t {
            let lerp_t = (t - prev_t) / step;
            return vec3_slerp(normals[i - 1], normals[i], lerp_t);
        }
        prev_t = cur_t;
        cur_t += step;
    }

    end_up
}

/// Unity-like SplinePath: evaluate a path composed of multiple spline slices, inserting degenerate curves at overlaps.
#[derive(Clone, Debug, Default)]
pub struct SplinePath {
    pub splines: Vec<Spline>,
    splits: Vec<usize>,
}

impl SplinePath {
    pub fn new(splines: Vec<Spline>) -> Self {
        let mut p = Self { splines, splits: Vec::new() };
        p.build_split_data();
        p
    }

    fn build_split_data(&mut self) {
        self.splits.clear();
        let mut k: usize = 0;
        for s in self.splines.iter() {
            let slice_count = s.count() + if s.closed { 1 } else { 0 };
            k += slice_count;
            self.splits.push(k.saturating_sub(1));
        }
    }

    pub fn count(&self) -> usize {
        self.splines.iter().map(|s| s.count() + if s.closed { 1 } else { 0 }).sum()
    }

    #[inline] pub fn closed(&self) -> bool { false }

    pub fn empty_curves(&self) -> &[usize] { &self.splits }

    fn is_degenerate(&self, index: usize) -> bool { self.splits.binary_search(&index).is_ok() }

    fn get_branch_knot_index(&self, mut knot: usize) -> (usize, usize) {
        let count = self.count();
        if count == 0 { return (0, 0); }
        knot = knot.min(count);
        let mut offset = 0usize;
        for (i, slice) in self.splines.iter().enumerate() {
            let slice_count = slice.count() + if slice.closed { 1 } else { 0 };
            if knot < offset + slice_count {
                let ki = knot.saturating_sub(offset);
                let kk = if slice.closed && slice.count() > 0 { ki % slice.count() } else { ki };
                return (i, kk);
            }
            offset += slice_count;
        }
        let last = self.splines.len().saturating_sub(1);
        (last, self.splines[last].count().saturating_sub(1))
    }

    pub fn knot(&self, index: usize) -> BezierKnot {
        let (si, ki) = self.get_branch_knot_index(index);
        self.splines[si].knots[ki]
    }

    pub fn next_knot(&self, index: usize) -> BezierKnot {
        let c = self.count();
        if c == 0 { return BezierKnot::default(); }
        self.knot((index + 1).min(c - 1))
    }

    pub fn get_curve(&self, knot: usize) -> BezierCurve {
        if self.is_degenerate(knot) {
            let p = BezierKnot { position: self.knot(knot).position, ..Default::default() };
            return BezierCurve::from_knots(p, p);
        }
        let a = self.knot(knot);
        let b = self.next_knot(knot);
        BezierCurve::from_knots(a, b)
    }

    pub fn get_curve_length(&mut self, index: usize) -> f32 {
        if self.is_degenerate(index) { return 0.0; }
        let (si, ki) = self.get_branch_knot_index(index);
        // For the last "closing" curve in a closed slice, compute directly.
        if si >= self.splines.len().saturating_sub(1) {
            let slice = &mut self.splines[si];
            if ki >= slice.count().saturating_sub(1) { return calculate_curve_lengths_len(self.get_curve(index)); }
            return slice.get_curve_length(ki);
        }
        self.splines[si].get_curve_length(ki)
    }

    pub fn get_curve_up_vector(&mut self, index: usize, t: f32) -> Vec3 {
        if self.is_degenerate(index) { return Vec3::ZERO; }
        let (si, ki) = self.get_branch_knot_index(index);
        if si >= self.splines.len().saturating_sub(1) {
            let slice = &mut self.splines[si];
            if ki >= slice.count().saturating_sub(1) {
                let a = self.knot(index);
                let b = self.next_knot(index);
                let curve = BezierCurve::from_knots(a, b);
                return evaluate_up_vector(curve, t, a.rotation.mul_vec3(Vec3::Y), b.rotation.mul_vec3(Vec3::Y));
            }
            return slice.get_curve_up_vector(ki, t);
        }
        self.splines[si].get_curve_up_vector(ki, t)
    }

    pub fn get_length(&mut self) -> f32 {
        let c = self.count();
        if c <= 1 { return 0.0; }
        let n = c - 1;
        let mut len = 0.0;
        for i in 0..n { len += self.get_curve_length(i); }
        len
    }
}

#[inline]
fn calculate_curve_lengths_len(curve: BezierCurve) -> f32 {
    let mut lut = [DistanceToInterpolation { distance: 0.0, t: 0.0 }; CURVE_DISTANCE_LUT_RESOLUTION];
    calculate_curve_lengths(curve, &mut lut);
    lut[CURVE_DISTANCE_LUT_RESOLUTION - 1].distance
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliceDirection { Forward, Backward }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplineRange { pub start: i32, pub count: i32, pub direction: SliceDirection }

impl SplineRange {
    #[inline] pub fn new(start: i32, count: i32) -> Self { Self { start, count: count.abs(), direction: if count < 0 { SliceDirection::Backward } else { SliceDirection::Forward } } }
    #[inline] pub fn new_dir(start: i32, count: i32, direction: SliceDirection) -> Self { Self { start, count: count.abs(), direction } }
    #[inline] pub fn at(&self, index: i32) -> i32 { if self.direction == SliceDirection::Backward { self.start - index } else { self.start + index } }
    #[inline] pub fn end(&self) -> i32 { self.at(self.count - 1) }
}

#[derive(Clone, Debug)]
pub struct SplineSlice { pub spline: Spline, pub range: SplineRange, pub transform: Mat4 }

impl SplineSlice {
    #[inline] pub fn new(spline: Spline, range: SplineRange) -> Self { Self { spline, range, transform: Mat4::IDENTITY } }
    #[inline] pub fn new_trs(spline: Spline, range: SplineRange, transform: Mat4) -> Self { Self { spline, range, transform } }

    pub fn count(&self) -> usize {
        let sc = self.spline.count() as i32;
        if sc <= 0 { return 0; }
        let c = self.range.count.max(0);
        if self.spline.closed { c.min(sc + 1) as usize }
        else if self.range.direction == SliceDirection::Backward { c.min(self.range.start + 1) as usize }
        else { c.min(sc - self.range.start) as usize }
    }

    #[inline] fn flip_tangents(k: BezierKnot) -> BezierKnot { BezierKnot { position: k.position, tangent_in: k.tangent_out, tangent_out: k.tangent_in, rotation: k.rotation } }

    pub fn knot(&self, index: usize) -> BezierKnot {
        let sc = self.spline.count() as i32;
        if sc <= 0 { return BezierKnot::default(); }
        let mut idx = self.range.at(index as i32);
        idx = (idx + sc) % sc;
        let k = self.spline.knots[idx as usize];
        if self.range.direction == SliceDirection::Backward { Self::flip_tangents(k).transform(self.transform) } else { k.transform(self.transform) }
    }

    pub fn get_curve(&self, index: usize) -> BezierCurve {
        let c = self.count();
        if c == 0 { return BezierCurve { p0: Vec3::ZERO, p1: Vec3::ZERO, p2: Vec3::ZERO, p3: Vec3::ZERO }; }
        let bi = (index + 1).clamp(0, c - 1);
        let a = self.knot(index);
        let b = self.knot(bi);
        if index == bi { BezierCurve { p0: a.position, p1: a.position, p2: b.position, p3: b.position } } else { BezierCurve::from_knots(a, b) }
    }

    #[inline] pub fn get_curve_length(&self, index: usize) -> f32 { calculate_curve_lengths_len(self.get_curve(index)) }
    #[inline] pub fn get_length(&self) -> f32 { (0..self.count()).map(|i| self.get_curve_length(i)).sum() }
    #[inline] pub fn get_curve_up_vector(&self, index: usize, t: f32) -> Vec3 {
        let curve = self.get_curve(index);
        let a = self.knot(index);
        let b = self.knot((index + 1).min(self.count().saturating_sub(1)));
        evaluate_up_vector(curve, t, a.rotation.mul_vec3(Vec3::Y), b.rotation.mul_vec3(Vec3::Y))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SplineKnotIndex { pub spline: i32, pub knot: i32 }

impl SplineKnotIndex {
    pub const INVALID: Self = Self { spline: -1, knot: -1 };
    #[inline] pub fn new(spline: i32, knot: i32) -> Self { Self { spline, knot } }
    #[inline] pub fn is_valid(self) -> bool { self.spline >= 0 && self.knot >= 0 }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KnotLinkCollection { links: Vec<Vec<SplineKnotIndex>> }

impl KnotLinkCollection {
    #[inline] pub fn count(&self) -> usize { self.links.len() }
    #[inline] pub fn clear(&mut self) { self.links.clear(); }
    #[inline] pub fn all_links(&self) -> Vec<Vec<SplineKnotIndex>> { self.links.clone() }

    fn get_link_idx(&self, index: SplineKnotIndex) -> Option<usize> {
        self.links.iter().position(|l| l.iter().any(|k| *k == index))
    }

    pub fn get_knot_links(&self, knot: SplineKnotIndex) -> Vec<SplineKnotIndex> {
        self.get_link_idx(knot).map(|i| self.links[i].clone()).unwrap_or_else(|| vec![knot])
    }

    pub fn link(&mut self, a: SplineKnotIndex, b: SplineKnotIndex) {
        if a == b { return; }
        let ai = self.get_link_idx(a);
        let bi = self.get_link_idx(b);
        match (ai, bi) {
            (Some(i), Some(j)) => {
                if i == j { return; }
                let mut rhs = self.links[j].clone();
                self.links[i].append(&mut rhs);
                self.links.remove(j);
            }
            (None, Some(j)) => if !self.links[j].contains(&a) { self.links[j].push(a); },
            (Some(i), None) => if !self.links[i].contains(&b) { self.links[i].push(b); },
            (None, None) => self.links.push(vec![a, b]),
        }
    }

    pub fn unlink(&mut self, knot: SplineKnotIndex) {
        let Some(i) = self.get_link_idx(knot) else { return; };
        self.links[i].retain(|k| *k != knot);
        if self.links[i].len() < 2 { self.links.remove(i); }
    }

    pub fn spline_removed(&mut self, spline_index: i32) {
        let mut i = self.links.len();
        while i > 0 {
            i -= 1;
            self.links[i].retain(|k| k.spline != spline_index);
            if self.links[i].len() < 2 { self.links.remove(i); continue; }
            for k in self.links[i].iter_mut() { if k.spline > spline_index { k.spline -= 1; } }
        }
    }

    pub fn spline_index_changed(&mut self, previous: i32, new_index: i32) {
        for link in self.links.iter_mut() {
            for k in link.iter_mut() {
                if k.spline == previous { k.spline = new_index; }
                else if k.spline > previous && k.spline <= new_index { k.spline -= 1; }
                else if k.spline < previous && k.spline >= new_index { k.spline += 1; }
            }
        }
    }

    pub fn knot_index_changed(&mut self, mut previous: SplineKnotIndex, mut new_index: SplineKnotIndex) {
        if previous.knot > new_index.knot { previous.knot += 1; } else { new_index.knot += 1; }
        self.knot_inserted(new_index);
        self.link(previous, new_index);
        self.knot_removed(previous);
    }

    pub fn knot_removed(&mut self, index: SplineKnotIndex) {
        self.unlink(index);
        self.shift_knot_indices(index, -1);
    }

    pub fn knot_inserted(&mut self, index: SplineKnotIndex) { self.shift_knot_indices(index, 1); }

    pub fn shift_knot_indices(&mut self, index: SplineKnotIndex, offset: i32) {
        for link in self.links.iter_mut() {
            for k in link.iter_mut() {
                if k.spline == index.spline && k.knot >= index.knot { k.knot += offset; }
            }
        }
    }
}

impl Spline {
    pub fn reverse_flow(&mut self) {
        let c = self.count();
        if c == 0 { return; }
        self.ensure_meta_valid();
        let knots = self.knots.clone();
        let modes: Vec<TangentMode> = self.meta.iter().take(c).map(|m| m.mode).collect();
        for prev in 0..c {
            let mode = modes[prev];
            let mut knot = knots[prev];
            let tin = knot.tangent_in;
            let tout = knot.tangent_out;
            let axis = knot.rotation.mul_vec3(Vec3::Y).normalize_or_zero();
            let reverse_rotation = Quat::from_axis_angle(axis, std::f32::consts::PI).normalize();
            knot.rotation = reverse_rotation * knot.rotation;
            if mode == TangentMode::Broken {
                let local_rot = Quat::from_axis_angle(Vec3::Y, std::f32::consts::PI);
                knot.tangent_in = local_rot.mul_vec3(tout);
                knot.tangent_out = local_rot.mul_vec3(tin);
            } else if mode == TangentMode::Continuous {
                knot.tangent_in = -tout;
                knot.tangent_out = -tin;
            }
            let new_idx = c - 1 - prev;
            self.meta[new_idx].mode = mode;
            self.knots[new_idx] = knot;
            self.meta[new_idx].invalidate();
        }
        self.length = -1.0;
    }

    #[inline] pub fn add(&mut self, knot: BezierKnot, mode: TangentMode, tension: f32) { self.insert(self.count(), knot, mode, tension); }

    pub fn resize(&mut self, new_size: usize) {
        let original = self.count();
        if new_size == original { return; }
        if new_size > original {
            while self.count() < new_size { self.insert(self.count(), BezierKnot::default(), TangentMode::Broken, CATMULL_ROM_TENSION); }
        } else {
            while self.count() > new_size { self.remove_at(self.count() - 1); }
            if new_size > 0 { self.apply_tangent_mode_no_notify(new_size - 1, BezierTangent::Out); }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SplineContainer {
    pub splines: Vec<Spline>,
    pub links: KnotLinkCollection,
    pub local_to_world: Mat4,
}

impl SplineContainer {
    #[inline] fn is_index_valid(&self, index: SplineKnotIndex) -> bool {
        (index.spline as usize) < self.splines.len() && index.knot >= 0 && (index.knot as usize) < self.splines[index.spline as usize].count()
    }

    #[inline] pub fn add_spline(&mut self, spline: Spline) { self.splines.push(spline); }

    pub fn remove_spline_at(&mut self, spline_index: usize) -> bool {
        if spline_index >= self.splines.len() { return false; }
        self.splines.remove(spline_index);
        self.links.spline_removed(spline_index as i32);
        true
    }

    pub fn reorder_spline(&mut self, previous: usize, new_index: usize) -> bool {
        if previous >= self.splines.len() || new_index >= self.splines.len() { return false; }
        if previous == new_index { return true; }
        let spline = self.splines.remove(previous);
        self.splines.insert(new_index, spline);
        self.links.spline_index_changed(previous as i32, new_index as i32);
        true
    }

    pub fn set_linked_knot_position(&mut self, index: SplineKnotIndex) {
        let knots = self.links.get_knot_links(index);
        if knots.len() <= 1 { return; }
        if !self.is_index_valid(index) { return; }
        let pos = self.splines[index.spline as usize].knots[index.knot as usize].position;
        for i in knots {
            if !self.is_index_valid(i) { return; }
            let s = &mut self.splines[i.spline as usize];
            let mut knot = s.knots[i.knot as usize];
            knot.position = pos;
            s.set_knot_no_notify(i.knot as usize, knot, BezierTangent::Out);
        }
    }

    pub fn unlink_knots(&mut self, knots: &[SplineKnotIndex]) { for k in knots { self.links.unlink(*k); } }

    pub fn link_knots(&mut self, a: SplineKnotIndex, b: SplineKnotIndex) {
        if !self.is_index_valid(a) || !self.is_index_valid(b) { return; }
        let pa = self.splines[a.spline as usize].knots[a.knot as usize].position;
        let pb = self.splines[b.spline as usize].knots[b.knot as usize].position;
        let similar = (pa - pb).length_squared() < 1e-12;
        let _knots_to_notify = if similar { None } else { Some(self.links.get_knot_links(b)) };
        self.links.link(a, b);
    }

    pub fn are_knot_linked(&self, a: SplineKnotIndex, b: SplineKnotIndex) -> bool {
        let links = self.links.get_knot_links(a);
        links.iter().any(|k| *k == b) && links.len() > 1
    }

    pub fn copy_knot_links(&mut self, src_spline: usize, dst_spline: usize) {
        if src_spline >= self.splines.len() || dst_spline >= self.splines.len() { return; }
        let sc = self.splines[src_spline].count();
        if sc == 0 || sc != self.splines[dst_spline].count() { return; }
        for i in 0..sc {
            let src = SplineKnotIndex::new(src_spline as i32, i as i32);
            if self.links.get_link_idx(src).is_some() {
                self.links.link(src, SplineKnotIndex::new(dst_spline as i32, i as i32));
            }
        }
    }

    pub fn reverse_flow(&mut self, spline_index: usize) {
        if spline_index >= self.splines.len() { return; }
        let (count, knots, modes) = {
            let spline = &mut self.splines[spline_index];
            let count = spline.count();
            if count == 0 { return; }
            spline.ensure_meta_valid();
            (count, spline.knots.clone(), spline.meta.iter().take(count).map(|m| m.mode).collect::<Vec<_>>())
        };

        let mut spline_links: Vec<Vec<SplineKnotIndex>> = Vec::with_capacity(count);
        for prev_knot_index in 0..count {
            let knot = SplineKnotIndex::new(spline_index as i32, prev_knot_index as i32);
            spline_links.push(self.links.get_knot_links(knot));
        }
        for linked in spline_links.iter() { self.unlink_knots(linked); }

        let spline = &mut self.splines[spline_index];
        for prev_knot_index in 0..count {
            let mode = modes[prev_knot_index];
            let mut knot = knots[prev_knot_index];
            let world_knot = knot.transform(self.local_to_world);
            let tangent_in = world_knot.tangent_in;
            let tangent_out = world_knot.tangent_out;

            let axis = knot.rotation.mul_vec3(Vec3::Y).normalize_or_zero();
            let reverse_rotation = Quat::from_axis_angle(axis, std::f32::consts::PI).normalize();
            knot.rotation = reverse_rotation * knot.rotation;

            if mode == TangentMode::Broken {
                let local_rot = Quat::from_axis_angle(Vec3::Y, std::f32::consts::PI);
                knot.tangent_in = local_rot.mul_vec3(tangent_out);
                knot.tangent_out = local_rot.mul_vec3(tangent_in);
            } else if mode == TangentMode::Continuous {
                knot.tangent_in = -tangent_out;
                knot.tangent_out = -tangent_in;
            }

            let new_idx = count - 1 - prev_knot_index;
            spline.meta[new_idx].mode = mode;
            spline.knots[new_idx] = knot;
            spline.meta[new_idx].invalidate();
        }
        spline.length = -1.0;

        for mut linked in spline_links {
            if linked.len() <= 1 { continue; }
            let mut original = linked[0];
            if original.spline == spline_index as i32 { original.knot = (count - 1 - (original.knot as usize)) as i32; }
            for i in 1..linked.len() {
                if linked[i].spline == spline_index as i32 { linked[i].knot = (count - 1 - (linked[i].knot as usize)) as i32; }
                self.link_knots(original, linked[i]);
            }
        }
    }

    pub fn join_splines_on_knots(&mut self, main_knot: SplineKnotIndex, other_knot: SplineKnotIndex) -> SplineKnotIndex {
        if main_knot.spline == other_knot.spline { return SplineKnotIndex::INVALID; }
        if !main_knot.is_valid() || !other_knot.is_valid() { return SplineKnotIndex::INVALID; }
        if (main_knot.spline as usize) >= self.splines.len() || (other_knot.spline as usize) >= self.splines.len() { return SplineKnotIndex::INVALID; }

        let active_count = self.splines[main_knot.spline as usize].count() as i32;
        let other_count = self.splines[other_knot.spline as usize].count() as i32;
        if main_knot.knot < 0 || main_knot.knot >= active_count { return SplineKnotIndex::INVALID; }
        if other_knot.knot < 0 || other_knot.knot >= other_count { return SplineKnotIndex::INVALID; }
        if !(main_knot.knot == 0 || main_knot.knot == active_count - 1) { return SplineKnotIndex::INVALID; }
        if !(other_knot.knot == 0 || other_knot.knot == other_count - 1) { return SplineKnotIndex::INVALID; }

        let is_active_at_start = main_knot.knot == 0;
        let is_other_at_start = other_knot.knot == 0;
        if is_active_at_start == is_other_at_start { self.reverse_flow(other_knot.spline as usize); }

        let active_spline_index = main_knot.spline as usize;
        let other_spline_index = other_knot.spline as usize;
        let active_spline_count = self.splines[active_spline_index].count();
        let other_spline_count = self.splines[other_spline_index].count();

        let mut links: Vec<Vec<SplineKnotIndex>> = Vec::with_capacity(active_spline_count + other_spline_count);
        for i in 0..active_spline_count { links.push(self.links.get_knot_links(SplineKnotIndex::new(active_spline_index as i32, i as i32))); }
        for i in 0..other_spline_count { links.push(self.links.get_knot_links(SplineKnotIndex::new(other_spline_index as i32, i as i32))); }
        for l in links.iter() { self.unlink_knots(l); }

        if other_spline_count > 1 {
            // clone other spline to avoid borrow conflicts
            let other = self.splines[other_spline_index].clone();
            if is_active_at_start {
                for i in (0..(other_spline_count - 1)).rev() {
                    let mode = other.meta[i].mode;
                    let tension = other.meta[i].tension;
                    self.splines[active_spline_index].insert(0, other.knots[i], mode, tension);
                }
            } else {
                for i in 1..other_spline_count {
                    let mode = other.meta[i].mode;
                    let tension = other.meta[i].tension;
                    let at = self.splines[active_spline_index].count();
                    self.splines[active_spline_index].insert(at, other.knots[i], mode, tension);
                }
            }
        }

        self.remove_spline_at(other_spline_index);
        let new_active_spline_index = if other_spline_index > active_spline_index { active_spline_index } else { active_spline_index.saturating_sub(1) };

        for mut linked in links {
            if linked.len() <= 1 { continue; }
            for k in linked.iter_mut() {
                if (k.spline as usize) == active_spline_index || (k.spline as usize) == other_spline_index {
                    let mut new_index = k.knot as usize;
                    if (k.spline as usize) == active_spline_index && is_active_at_start { new_index += other_spline_count.saturating_sub(1); }
                    if (k.spline as usize) == other_spline_index && !is_active_at_start { new_index += active_spline_count.saturating_sub(1); }
                    *k = SplineKnotIndex::new(active_spline_index as i32, new_index as i32);
                } else if (k.spline as usize) > other_spline_index {
                    k.spline -= 1;
                }
            }
            let original = linked[0];
            for i in 1..linked.len() { self.link_knots(original, linked[i]); }
        }

        let junction_knot = if is_active_at_start { (other_spline_count.saturating_sub(1)) as i32 } else { main_knot.knot };
        SplineKnotIndex::new(new_active_spline_index as i32, junction_knot)
    }

    pub fn duplicate_knot(&mut self, original: SplineKnotIndex, target_index: usize) -> SplineKnotIndex {
        if !self.is_index_valid(original) { return SplineKnotIndex::INVALID; }
        let si = original.spline as usize;
        let ki = original.knot as usize;
        let knot = self.splines[si].knots[ki];
        let mode = self.splines[si].meta[ki].mode;
        let tension = self.splines[si].meta[ki].tension;
        let at = target_index.min(self.splines[si].count());
        self.links.knot_inserted(SplineKnotIndex::new(original.spline, at as i32));
        self.splines[si].insert(at, knot, mode, tension);
        SplineKnotIndex::new(original.spline, at as i32)
    }

    pub fn duplicate_spline(&mut self, from: SplineKnotIndex, to: SplineKnotIndex) -> Option<usize> {
        if !(from.is_valid() && to.is_valid()) { return None; }
        if from.spline != to.spline { return None; }
        let si = from.spline as usize;
        if si >= self.splines.len() { return None; }
        let start = (from.knot.min(to.knot)).max(0) as usize;
        let end = (from.knot.max(to.knot)).max(0) as usize;
        if end >= self.splines[si].count() { return None; }

        let original = self.splines[si].clone();
        let mut dup = Spline::default();
        for i in start..=end {
            let mode = original.meta[i].mode;
            let tension = original.meta[i].tension;
            dup.insert(dup.count(), original.knots[i], mode, tension);
            let old = SplineKnotIndex::new(si as i32, i as i32);
            if self.links.get_link_idx(old).is_some() {
                self.links.link(old, SplineKnotIndex::new(self.splines.len() as i32, (i - start) as i32));
            }
        }
        self.add_spline(dup);
        Some(self.splines.len() - 1)
    }

    pub fn split_spline_on_knot(&mut self, knot: SplineKnotIndex) -> SplineKnotIndex {
        if !knot.is_valid() { return SplineKnotIndex::INVALID; }
        let si = knot.spline as usize;
        if si >= self.splines.len() { return SplineKnotIndex::INVALID; }
        let count = self.splines[si].count();
        let ki = knot.knot as usize;
        if ki >= count { return SplineKnotIndex::INVALID; }

        if self.splines[si].closed {
            self.splines[si].closed = false;
            let first = SplineKnotIndex::new(knot.spline, 0);
            let last = self.duplicate_knot(first, self.splines[si].count());
            if ki == 0 { return first; }
            self.link_knots(first, last);
        } else if ki == 0 || ki == count - 1 {
            return knot;
        }

        let end_knot = SplineKnotIndex::new(knot.spline, (self.splines[si].count() - 1) as i32);
        let Some(new_si) = self.duplicate_spline(knot, end_knot) else { return SplineKnotIndex::INVALID; };

        // Resize original to ki+1
        loop {
            let cur = self.splines[si].count();
            if cur <= ki + 1 { break; }
            let rm = cur - 1;
            self.links.knot_removed(SplineKnotIndex::new(knot.spline, rm as i32));
            self.splines[si].remove_at(rm);
        }
        SplineKnotIndex::new(new_si as i32, 0)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn spline2(a: Vec3, b: Vec3) -> Spline {
        let mut s = Spline::default();
        s.knots = vec![BezierKnot { position: a, ..Default::default() }, BezierKnot { position: b, ..Default::default() }];
        s.meta = vec![MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION), MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION)];
        s
    }

    #[test]
    fn bezier_endpoints_match() {
        let a = BezierKnot { position: Vec3::new(0.0, 0.0, 0.0), ..Default::default() };
        let b = BezierKnot { position: Vec3::new(1.0, 2.0, 3.0), ..Default::default() };
        let c = BezierCurve::from_knots(a, b);
        assert_eq!(evaluate_position(c, 0.0), c.p0);
        assert_eq!(evaluate_position(c, 1.0), c.p3);
    }

    #[test]
    fn auto_smooth_knot_has_z_tangents() {
        let k = get_auto_smooth_knot(Vec3::ZERO, Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::Y, CATMULL_ROM_TENSION);
        assert!(k.tangent_in.x == 0.0 && k.tangent_in.y == 0.0);
        assert!(k.tangent_out.x == 0.0 && k.tangent_out.y == 0.0);
        assert!(k.rotation.is_finite());
    }

    #[test]
    fn split_curve_midpoint_matches() {
        let a = BezierKnot { position: Vec3::new(0.0, 0.0, 0.0), tangent_out: Vec3::new(0.0, 0.0, 1.0), ..Default::default() };
        let b = BezierKnot { position: Vec3::new(0.0, 0.0, 10.0), tangent_in: Vec3::new(0.0, 0.0, -1.0), ..Default::default() };
        let c = BezierCurve::from_knots(a, b);
        let (l, r) = split_curve(c, 0.5);
        let mid = evaluate_position(c, 0.5);
        assert_eq!(l.p3, mid);
        assert_eq!(r.p0, mid);
    }

    #[test]
    fn insert_on_curve_increases_count() {
        let mut s = Spline::default();
        s.knots = vec![BezierKnot { position: Vec3::new(0.0, 0.0, 0.0), ..Default::default() }, BezierKnot { position: Vec3::new(0.0, 0.0, 10.0), ..Default::default() }];
        s.meta = vec![MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION), MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION)];
        s.insert_on_curve(1, 0.5);
        assert_eq!(s.count(), 3);
    }

    #[test]
    fn spline_range_backward_indexing() {
        let r = SplineRange::new_dir(6, 3, SliceDirection::Backward);
        assert_eq!(r.at(0), 6);
        assert_eq!(r.at(1), 5);
        assert_eq!(r.at(2), 4);
        assert_eq!(r.end(), 4);
    }

    #[test]
    fn knot_link_collection_merges_and_shifts() {
        let mut c = KnotLinkCollection::default();
        let a = SplineKnotIndex::new(0, 0);
        let b = SplineKnotIndex::new(0, 1);
        let d = SplineKnotIndex::new(1, 0);
        let e = SplineKnotIndex::new(1, 1);
        c.link(a, b);
        c.link(d, e);
        c.link(b, d);
        assert_eq!(c.count(), 1);
        assert_eq!(c.get_knot_links(a).len(), 4);

        c.knot_inserted(SplineKnotIndex::new(1, 1));
        let shifted = c.get_knot_links(a);
        assert!(shifted.iter().any(|k| *k == SplineKnotIndex::new(1, 2)));
    }

    #[test]
    fn reverse_flow_reverses_order() {
        let mut s = Spline::default();
        s.knots = vec![
            BezierKnot { position: Vec3::new(0.0, 0.0, 0.0), tangent_out: Vec3::new(0.0, 0.0, 1.0), ..Default::default() },
            BezierKnot { position: Vec3::new(0.0, 0.0, 10.0), tangent_in: Vec3::new(0.0, 0.0, -1.0), ..Default::default() },
        ];
        s.meta = vec![MetaData::new(TangentMode::Broken, CATMULL_ROM_TENSION), MetaData::new(TangentMode::Broken, CATMULL_ROM_TENSION)];
        s.reverse_flow();
        assert_eq!(s.knots[0].position, Vec3::new(0.0, 0.0, 10.0));
        assert_eq!(s.knots[1].position, Vec3::new(0.0, 0.0, 0.0));
    }

    #[test]
    fn container_reverse_flow_preserves_links() {
        let mut c = SplineContainer { splines: vec![spline2(Vec3::ZERO, Vec3::Z), spline2(Vec3::X, Vec3::X + Vec3::Z)], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        c.link_knots(SplineKnotIndex::new(0, 0), SplineKnotIndex::new(1, 1));
        c.reverse_flow(0);
        let links = c.links.get_knot_links(SplineKnotIndex::new(1, 1));
        assert!(links.iter().any(|k| *k == SplineKnotIndex::new(0, 1)));
    }

    #[test]
    fn join_splines_on_knots_remaps_links() {
        let mut c = SplineContainer { splines: vec![spline2(Vec3::ZERO, Vec3::Z), spline2(Vec3::X, Vec3::X + Vec3::Z), spline2(Vec3::Y, Vec3::Y + Vec3::Z)], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        // link other spline's last knot to third spline's first knot
        c.link_knots(SplineKnotIndex::new(1, 1), SplineKnotIndex::new(2, 0));
        let out = c.join_splines_on_knots(SplineKnotIndex::new(0, 1), SplineKnotIndex::new(1, 0));
        assert_eq!(c.splines.len(), 2);
        assert_eq!(out.spline, 0);
        assert_eq!(out.knot, 1);
        // third spline shifts from index 2 to 1 after removal
        let links = c.links.get_knot_links(SplineKnotIndex::new(1, 0));
        assert!(links.iter().any(|k| *k == SplineKnotIndex::new(0, 2)));
    }

    #[test]
    fn join_preserves_links_on_junction_knot() {
        let mut c = SplineContainer { splines: vec![spline2(Vec3::ZERO, Vec3::Z), spline2(Vec3::X, Vec3::X + Vec3::Z), spline2(Vec3::Y, Vec3::Y + Vec3::Z)], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        // junction knot (0,1) linked to third spline's first knot
        c.link_knots(SplineKnotIndex::new(0, 1), SplineKnotIndex::new(2, 0));
        let out = c.join_splines_on_knots(SplineKnotIndex::new(0, 1), SplineKnotIndex::new(1, 0));
        assert_eq!(out, SplineKnotIndex::new(0, 1));
        // third spline shifts from index 2 to 1 after removal
        assert!(c.are_knot_linked(SplineKnotIndex::new(0, 1), SplineKnotIndex::new(1, 0)));
    }

    #[test]
    fn reorder_spline_shifts_links() {
        let mut c = SplineContainer { splines: vec![spline2(Vec3::ZERO, Vec3::Z), spline2(Vec3::X, Vec3::X + Vec3::Z), spline2(Vec3::Y, Vec3::Y + Vec3::Z)], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        c.link_knots(SplineKnotIndex::new(2, 0), SplineKnotIndex::new(0, 1));
        assert!(c.reorder_spline(2, 0));
        // old spline 2 moved to 0, old spline 0 moved to 1
        assert!(c.are_knot_linked(SplineKnotIndex::new(0, 0), SplineKnotIndex::new(1, 1)));
    }

    #[test]
    fn copy_knot_links_links_matching_indices() {
        let mut c = SplineContainer { splines: vec![spline2(Vec3::ZERO, Vec3::Z), spline2(Vec3::X, Vec3::X + Vec3::Z), spline2(Vec3::Y, Vec3::Y + Vec3::Z)], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        c.link_knots(SplineKnotIndex::new(0, 0), SplineKnotIndex::new(2, 0));
        c.copy_knot_links(0, 1);
        assert!(c.are_knot_linked(SplineKnotIndex::new(1, 0), SplineKnotIndex::new(2, 0)));
    }

    #[test]
    fn duplicate_knot_shifts_existing_links() {
        let mut c = SplineContainer { splines: vec![spline2(Vec3::ZERO, Vec3::Z), spline2(Vec3::X, Vec3::X + Vec3::Z)], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        c.link_knots(SplineKnotIndex::new(0, 1), SplineKnotIndex::new(1, 0));
        let _ = c.duplicate_knot(SplineKnotIndex::new(0, 0), 0);
        assert!(c.are_knot_linked(SplineKnotIndex::new(0, 2), SplineKnotIndex::new(1, 0)));
    }

    #[test]
    fn split_spline_on_knot_extremity_noop() {
        let mut c = SplineContainer { splines: vec![spline2(Vec3::ZERO, Vec3::Z)], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        let out = c.split_spline_on_knot(SplineKnotIndex::new(0, 0));
        assert_eq!(out, SplineKnotIndex::new(0, 0));
        assert_eq!(c.splines.len(), 1);
    }

    #[test]
    fn split_spline_on_knot_creates_new_spline() {
        let mut s = Spline::default();
        s.knots = vec![
            BezierKnot { position: Vec3::new(0.0, 0.0, 0.0), ..Default::default() },
            BezierKnot { position: Vec3::new(0.0, 0.0, 5.0), ..Default::default() },
            BezierKnot { position: Vec3::new(0.0, 0.0, 10.0), ..Default::default() },
        ];
        s.meta = vec![MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION); 3];
        let mut c = SplineContainer { splines: vec![s], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        let out = c.split_spline_on_knot(SplineKnotIndex::new(0, 1));
        assert_eq!(c.splines.len(), 2);
        assert_eq!(out, SplineKnotIndex::new(1, 0));
        assert_eq!(c.splines[0].count(), 2);
        assert_eq!(c.splines[1].count(), 2);
    }

    #[test]
    fn split_open_spline_transfers_links_from_moved_segment() {
        let mut s0 = Spline::default();
        s0.knots = vec![
            BezierKnot { position: Vec3::ZERO, ..Default::default() },
            BezierKnot { position: Vec3::Z * 1.0, ..Default::default() },
            BezierKnot { position: Vec3::Z * 2.0, ..Default::default() },
            BezierKnot { position: Vec3::Z * 3.0, ..Default::default() },
        ];
        s0.meta = vec![MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION); 4];
        let s1 = spline2(Vec3::X, Vec3::X + Vec3::Z);
        let mut c = SplineContainer { splines: vec![s0, s1], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        // link tail knot (0,3) to other spline (1,0)
        c.link_knots(SplineKnotIndex::new(0, 3), SplineKnotIndex::new(1, 0));
        let out = c.split_spline_on_knot(SplineKnotIndex::new(0, 1));
        assert_eq!(out, SplineKnotIndex::new(2, 0));
        // old knot3 moved to new spline index 2, knot2
        assert!(c.are_knot_linked(SplineKnotIndex::new(1, 0), SplineKnotIndex::new(2, 2)));
    }

    #[test]
    fn split_closed_spline_on_first_knot_duplicates_end_but_no_new_spline() {
        let mut s = Spline::default();
        s.closed = true;
        s.knots = vec![
            BezierKnot { position: Vec3::new(0.0, 0.0, 0.0), ..Default::default() },
            BezierKnot { position: Vec3::new(1.0, 0.0, 0.0), ..Default::default() },
            BezierKnot { position: Vec3::new(2.0, 0.0, 0.0), ..Default::default() },
        ];
        s.meta = vec![MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION); 3];
        let mut c = SplineContainer { splines: vec![s], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        let out = c.split_spline_on_knot(SplineKnotIndex::new(0, 0));
        assert_eq!(out, SplineKnotIndex::new(0, 0));
        assert_eq!(c.splines.len(), 1);
        assert!(!c.splines[0].closed);
        assert_eq!(c.splines[0].count(), 4); // Unity duplicates first knot to end when unclosing
    }

    #[test]
    fn split_closed_spline_mid_preserves_links_via_duplicate_then_remove() {
        // Closed spline with 3 knots, and another spline. Link knot2 -> other.knot0.
        let mut s0 = Spline::default();
        s0.closed = true;
        s0.knots = vec![
            BezierKnot { position: Vec3::new(0.0, 0.0, 0.0), ..Default::default() },
            BezierKnot { position: Vec3::new(0.0, 0.0, 5.0), ..Default::default() },
            BezierKnot { position: Vec3::new(0.0, 0.0, 10.0), ..Default::default() },
        ];
        s0.meta = vec![MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION); 3];
        let s1 = spline2(Vec3::X, Vec3::X + Vec3::Z);

        let mut c = SplineContainer { splines: vec![s0, s1], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        c.link_knots(SplineKnotIndex::new(0, 2), SplineKnotIndex::new(1, 0));

        let out = c.split_spline_on_knot(SplineKnotIndex::new(0, 1));
        assert_eq!(out, SplineKnotIndex::new(2, 0)); // new spline appended
        assert_eq!(c.splines.len(), 3);

        // Original resized to [0,1], so old knot2 removed. The link should survive by being transferred to new spline's knot1.
        assert!(c.are_knot_linked(SplineKnotIndex::new(1, 0), SplineKnotIndex::new(2, 1)));
    }

    #[test]
    fn clear_tangent_mirrored_switches_to_continuous() {
        let mut s = Spline::default();
        s.knots = vec![BezierKnot { position: Vec3::ZERO, tangent_in: Vec3::new(0.0, 0.0, -1.0), tangent_out: Vec3::new(0.0, 0.0, 1.0), ..Default::default() }, BezierKnot { position: Vec3::Z, ..Default::default() }];
        s.meta = vec![MetaData::new(TangentMode::Mirrored, CATMULL_ROM_TENSION), MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION)];
        s.clear_tangent(0, BezierTangent::Out);
        assert_eq!(s.meta[0].mode, TangentMode::Continuous);
    }

    #[test]
    fn knot_link_collection_spline_removed_shifts_and_prunes() {
        let mut c = KnotLinkCollection::default();
        c.link(SplineKnotIndex::new(0, 0), SplineKnotIndex::new(1, 0));
        c.link(SplineKnotIndex::new(2, 1), SplineKnotIndex::new(3, 2));
        // remove spline 1 -> link(0,0)-(1,0) should be pruned (becomes single), and spline indices >1 shift down.
        c.spline_removed(1);
        assert_eq!(c.count(), 1);
        let links = c.get_knot_links(SplineKnotIndex::new(1, 1)); // old (2,1) becomes (1,1)
        assert!(links.iter().any(|k| *k == SplineKnotIndex::new(2, 2))); // old (3,2) becomes (2,2)
    }

    #[test]
    fn knot_link_collection_knot_index_changed_matches_unity_shift_rule() {
        let mut c = KnotLinkCollection::default();
        // Link three knots in same spline: 0,1,2
        c.link(SplineKnotIndex::new(0, 0), SplineKnotIndex::new(0, 1));
        c.link(SplineKnotIndex::new(0, 1), SplineKnotIndex::new(0, 2));
        // Move knot index 0 -> 2 (Unity inserts+links+removes with +1 adjustment)
        c.knot_index_changed(SplineKnotIndex::new(0, 0), SplineKnotIndex::new(0, 2));
        let links = c.get_knot_links(SplineKnotIndex::new(0, 2));
        // Still a single link group with 3 entries, all in spline 0
        assert_eq!(links.len(), 3);
        assert!(links.iter().all(|k| k.spline == 0));
    }

    #[test]
    fn inserted_knot_preview_has_midpoint_position() {
        let mut s = spline2(Vec3::ZERO, Vec3::new(0.0, 0.0, 10.0));
        let (k, _lo, _ri) = get_inserted_knot_preview(&mut s, 1, 0.5);
        let curve = BezierCurve::from_knots(s.knots[0], s.knots[1]);
        let mid = evaluate_position(curve, 0.5);
        assert_eq!(k.position, mid);
    }

    #[test]
    fn preview_curve_linear_modes_force_endpoints_as_controls() {
        let s = spline2(Vec3::ZERO, Vec3::new(0.0, 0.0, 10.0));
        let c = SplineContainer { splines: vec![s], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        let curve = get_preview_curve_from_end(&c, 0, 0, Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO, TangentMode::Linear);
        assert_eq!(curve.p1, curve.p0);
        assert_eq!(curve.p2, curve.p3);
    }

    #[test]
    fn affected_curves_insert_on_segment_reports_main_curve() {
        let mut s = Spline::default();
        s.knots = vec![
            BezierKnot { position: Vec3::new(0.0, 0.0, 0.0), ..Default::default() },
            BezierKnot { position: Vec3::new(0.0, 0.0, 10.0), ..Default::default() },
        ];
        s.meta = vec![MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION), MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION)];
        let c = SplineContainer { splines: vec![s], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        let list = get_affected_curves_insert_on_segment(&c, 0, 0, 1, 0.5, Vec3::new(0.0, 0.0, 5.0));
        assert!(list.iter().any(|x| x.curve_index == 0 && x.knots.len() >= 2));
    }

    #[test]
    fn placement_mode_from_tangent_matches_unity() {
        assert_eq!(mode_from_placement_tangent(Vec3::ZERO, TangentMode::AutoSmooth), TangentMode::AutoSmooth);
        assert_eq!(mode_from_placement_tangent(Vec3::X, TangentMode::AutoSmooth), TangentMode::Mirrored);
    }

    #[test]
    fn add_knot_to_end_inserts_and_updates_links_table() {
        let mut c = SplineContainer { splines: vec![spline2(Vec3::ZERO, Vec3::Z)], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        let out = c.add_knot_to_end(0, Vec3::new(0.0, 0.0, 2.0), Vec3::Y, Vec3::ZERO, TangentMode::AutoSmooth);
        assert!(out.is_some());
        assert_eq!(c.splines[0].count(), 3);
    }

    #[test]
    fn unclose_spline_adds_knot_and_links_ends() {
        let mut s = spline2(Vec3::ZERO, Vec3::Z);
        s.closed = true;
        let mut c = SplineContainer { splines: vec![s], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        c.unclose_spline_if_needed(0, DrawingDirection::End);
        assert!(!c.splines[0].closed);
        assert_eq!(c.splines[0].count(), 3);
        assert!(c.are_knot_linked(SplineKnotIndex::new(0, 0), SplineKnotIndex::new(0, 2)));
    }

    #[test]
    fn create_knot_on_knot_click_close_sets_closed() {
        let mut s = spline2(Vec3::ZERO, Vec3::Z);
        s.closed = false;
        let mut c = SplineContainer { splines: vec![s], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        c.create_knot_on_knot(0, DrawingDirection::End, SelectableKnot { spline_index: 0, knot_index: 0 }, Vec3::ZERO);
        assert!(c.splines[0].closed);
        assert_eq!(c.splines[0].count(), 2);
    }

    #[test]
    fn create_knot_on_surface_uncloses_then_adds() {
        let mut s = spline2(Vec3::ZERO, Vec3::Z);
        s.closed = true;
        let mut c = SplineContainer { splines: vec![s], links: KnotLinkCollection::default(), local_to_world: Mat4::IDENTITY };
        c.create_knot_on_surface(0, DrawingDirection::End, Vec3::new(0.0, 0.0, 2.0), Vec3::Y, Vec3::ZERO);
        assert!(!c.splines[0].closed);
        assert!(c.splines[0].count() >= 3);
    }
}















