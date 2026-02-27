use super::Poly;
use bevy::prelude::*;
use rayon::prelude::*;
use std::collections::HashMap;

/// Result of parallel VMesh generation
#[derive(Clone, Default)]
pub struct VMeshResult {
    pub points: Vec<Vec3>,
    pub polys: Vec<Poly>,
}

impl VMeshResult {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_capacity(pts: usize, polys: usize) -> Self {
        Self {
            points: Vec::with_capacity(pts),
            polys: Vec::with_capacity(polys),
        }
    }
}

/// Generate VMesh for multiple corners in parallel (2-4x speedup on multicore)
pub fn parallel_vmesh_emit(corners: &[(VMeshGrid, usize)], ns: usize) -> VMeshResult {
    if corners.is_empty() {
        return VMeshResult::new();
    }
    if corners.len() == 1 {
        return emit_vmesh_result(&corners[0].0, ns);
    }

    let results: Vec<VMeshResult> = corners
        .par_iter()
        .map(|(vm, _)| emit_vmesh_result(vm, ns))
        .collect();

    // Merge all results
    let total_pts: usize = results.iter().map(|r| r.points.len()).sum();
    let total_polys: usize = results.iter().map(|r| r.polys.len()).sum();
    let mut merged = VMeshResult::with_capacity(total_pts, total_polys);

    for r in results {
        let base = merged.points.len();
        merged.points.extend(r.points);
        for poly in r.polys {
            merged
                .polys
                .push(poly.into_iter().map(|i| i + base).collect());
        }
    }
    merged
}

/// Result of generating a single VMesh (no external deps; can run in parallel)
fn emit_vmesh_result(vm: &VMeshGrid, ns: usize) -> VMeshResult {
    let n_bndv = vm.n;
    let ns2 = ns / 2;
    let odd = ns & 1;
    let mut pts = Vec::with_capacity((ns2 + 1) * (ns + 1) * n_bndv);
    let mut polys = Vec::with_capacity(ns2 * (ns2 + odd) * n_bndv);
    let mut vm_pts: HashMap<(usize, usize, usize), usize> = HashMap::new();

    let mut v_at = |i: usize, j: usize, k: usize| -> usize {
        let (ci, cj, ck) = vm.canon(i, j, k);
        if let Some(&id) = vm_pts.get(&(ci, cj, ck)) {
            return id;
        }
        let id = pts.len();
        pts.push(vm.get(ci, cj, ck));
        vm_pts.insert((ci, cj, ck), id);
        id
    };

    let kmax = if odd == 0 { ns2 } else { ns2 + 1 };
    for i in 0..n_bndv {
        for j in 0..ns2 {
            for k in 0..kmax {
                let a = v_at(i, j, k);
                let b = v_at(i, j, k + 1);
                let c = v_at(i, j + 1, k + 1);
                let d = v_at(i, j + 1, k);
                polys.push(vec![a, b, c, d]);
            }
        }
    }
    if odd != 0 {
        let mut poly: Vec<usize> = (0..n_bndv).map(|i| v_at(i, ns2, ns2)).collect();
        // poly is already CCW (0..n_bndv), which matches the hole boundary left by the grid (c->d was CW).
        if poly.len() >= 3 {
            polys.push(poly);
        }
    }
    VMeshResult { points: pts, polys }
}

#[derive(Clone)]
pub struct VMeshGrid {
    pub n: usize,
    pub ns: usize,
    pub ns2: usize,
    pub data: Vec<Vec3>,
}
impl VMeshGrid {
    pub fn new(n: usize, ns: usize) -> Self {
        let ns2 = ns / 2;
        Self {
            n,
            ns,
            ns2,
            data: vec![Vec3::ZERO; n * (ns2 + 1) * (ns + 1)],
        }
    }
    fn idx(&self, i: usize, j: usize, k: usize) -> usize {
        (i * (self.ns2 + 1) + j) * (self.ns + 1) + k
    }
    pub fn get(&self, i: usize, j: usize, k: usize) -> Vec3 {
        self.data[self.idx(i, j, k)]
    }
    pub fn set(&mut self, i: usize, j: usize, k: usize, v: Vec3) {
        let id = self.idx(i, j, k);
        self.data[id] = v;
    }
    pub fn is_canon(&self, i: usize, j: usize, k: usize) -> bool {
        let ns2 = self.ns2;
        if self.ns & 1 == 1 {
            j <= ns2 && k <= ns2
        } else {
            (j < ns2 && k <= ns2) || (j == ns2 && k == ns2 && i == 0)
        }
    }
    pub fn canon(&self, i: usize, j: usize, k: usize) -> (usize, usize, usize) {
        let n = self.n;
        let ns = self.ns;
        let ns2 = self.ns2;
        let odd = ns & 1;
        if odd == 0 && j == ns2 && k == ns2 {
            return (0, ns2, ns2);
        }
        if (odd != 0 && j <= ns2 && k <= ns2) || (odd == 0 && j <= ns2 - 1 && k <= ns2) {
            return (i, j, k);
        }
        if k <= ns2 {
            return ((i + n - 1) % n, k, ns - j);
        }
        ((i + 1) % n, ns - k, j)
    }
    fn get_canon(&self, i: usize, j: usize, k: usize) -> Vec3 {
        let (ci, cj, ck) = self.canon(i, j, k);
        self.get(ci, cj, ck)
    }
    pub fn copy_equiv(&mut self) {
        for i in 0..self.n {
            for j in 0..=self.ns2 {
                for k in 0..=self.ns {
                    if self.is_canon(i, j, k) {
                        continue;
                    }
                    let v = self.get_canon(i, j, k);
                    self.set(i, j, k, v);
                }
            }
        }
    }
}

fn sabin_gamma(n: usize) -> f32 {
    if n < 3 {
        return 0.0;
    }
    if n == 3 {
        return 0.065247584;
    }
    if n == 4 {
        return 0.25;
    }
    if n == 5 {
        return 0.401983447;
    }
    if n == 6 {
        return 0.523423277;
    }
    let k = (std::f64::consts::PI / n as f64).cos();
    let k2 = k * k;
    let k4 = k2 * k2;
    let k6 = k4 * k2;
    let y = (3f64.sqrt() * (64.0 * k6 - 144.0 * k4 + 135.0 * k2 - 27.0).sqrt() + 9.0 * k)
        .powf(1.0 / 3.0);
    let x = 0.480749856769136 * y - (0.231120424783545 * (12.0 * k2 - 9.0)) / y;
    ((k * x + 2.0 * k2 - 1.0) / (x * x * (k * x + 1.0))) as f32
}

pub fn cubic_subdiv(
    mut vm_in: VMeshGrid,
    prof_at: &dyn Fn(usize, usize, usize) -> Vec3,
) -> VMeshGrid {
    let n = vm_in.n;
    let ns_in = vm_in.ns;
    let ns_in2 = ns_in / 2;
    debug_assert!(ns_in % 2 == 0);
    let ns_out = 2 * ns_in;
    let mut vm_out = VMeshGrid::new(n, ns_out);
    let smooth_1_6 = |co: Vec3, co1: Vec3, co2: Vec3| {
        let acc = co1 + co2 - 2.0 * co;
        co - acc * (1.0 / 6.0)
    };
    for i in 0..n {
        vm_out.set(i, 0, 0, vm_in.get(i, 0, 0));
        for k in 1..ns_in {
            vm_out.set(
                i,
                0,
                2 * k,
                smooth_1_6(
                    vm_in.get(i, 0, k),
                    vm_in.get(i, 0, k - 1),
                    vm_in.get(i, 0, k + 1),
                ),
            );
        }
    }
    vm_out.copy_equiv();
    for i in 0..n {
        for k in (1..ns_out).step_by(2) {
            let co = prof_at(i, k, ns_out);
            let co1 = vm_out.get_canon(i, 0, k - 1);
            let co2 = vm_out.get_canon(i, 0, k + 1);
            vm_out.set(i, 0, k, smooth_1_6(co, co1, co2));
        }
    }
    vm_out.copy_equiv();
    for i in 0..n {
        for k in 0..=ns_in {
            vm_in.set(i, 0, k, vm_out.get_canon(i, 0, 2 * k));
        }
    }
    vm_in.copy_equiv();
    let avg4 = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| (a + b + c + d) * 0.25;
    for i in 0..n {
        for j in 0..ns_in2 {
            for k in 0..ns_in2 {
                vm_out.set(
                    i,
                    2 * j + 1,
                    2 * k + 1,
                    avg4(
                        vm_in.get(i, j, k),
                        vm_in.get(i, j, k + 1),
                        vm_in.get(i, j + 1, k),
                        vm_in.get(i, j + 1, k + 1),
                    ),
                );
            }
        }
    }
    for i in 0..n {
        for j in 0..ns_in2 {
            for k in 1..=ns_in2 {
                vm_out.set(
                    i,
                    2 * j + 1,
                    2 * k,
                    avg4(
                        vm_in.get(i, j, k),
                        vm_in.get(i, j + 1, k),
                        vm_out.get_canon(i, 2 * j + 1, 2 * k - 1),
                        vm_out.get_canon(i, 2 * j + 1, 2 * k + 1),
                    ),
                );
            }
        }
    }
    for i in 0..n {
        for j in 1..ns_in2 {
            for k in 0..ns_in2 {
                vm_out.set(
                    i,
                    2 * j,
                    2 * k + 1,
                    avg4(
                        vm_in.get(i, j, k),
                        vm_in.get(i, j, k + 1),
                        vm_out.get_canon(i, 2 * j - 1, 2 * k + 1),
                        vm_out.get_canon(i, 2 * j + 1, 2 * k + 1),
                    ),
                );
            }
        }
    }
    let gamma = 0.25f32;
    let beta = -gamma;
    for i in 0..n {
        for j in 1..ns_in2 {
            for k in 1..=ns_in2 {
                let co1 = avg4(
                    vm_out.get_canon(i, 2 * j, 2 * k - 1),
                    vm_out.get_canon(i, 2 * j, 2 * k + 1),
                    vm_out.get_canon(i, 2 * j - 1, 2 * k),
                    vm_out.get_canon(i, 2 * j + 1, 2 * k),
                );
                let co2 = avg4(
                    vm_out.get_canon(i, 2 * j - 1, 2 * k - 1),
                    vm_out.get_canon(i, 2 * j + 1, 2 * k - 1),
                    vm_out.get_canon(i, 2 * j - 1, 2 * k + 1),
                    vm_out.get_canon(i, 2 * j + 1, 2 * k + 1),
                );
                vm_out.set(
                    i,
                    2 * j,
                    2 * k,
                    co1 + co2 * beta + vm_in.get(i, j, k) * gamma,
                );
            }
        }
    }
    vm_out.copy_equiv();
    let g = sabin_gamma(n);
    let b = -g;
    let mut co1 = Vec3::ZERO;
    let mut co2 = Vec3::ZERO;
    for i in 0..n {
        co1 += vm_out.get(i, ns_in, ns_in - 1);
        co2 += vm_out.get(i, ns_in - 1, ns_in - 1);
        co2 += vm_out.get(i, ns_in - 1, ns_in + 1);
    }
    let mut co = co1 * (1.0 / n as f32);
    co += co2 * (b / (2.0 * n as f32));
    co += vm_in.get(0, ns_in2, ns_in2) * g;
    for i in 0..n {
        vm_out.set(i, ns_in, ns_in, co);
    }
    vm_out.copy_equiv();
    for i in 0..n {
        let inext = (i + 1) % n;
        for k in 0..=ns_out {
            let v = prof_at(i, k, ns_out);
            vm_out.set(i, 0, k, v);
            if k >= ns_in && k < ns_out {
                vm_out.set(inext, ns_out - k, 0, v);
            }
        }
    }
    vm_out.copy_equiv();
    vm_out
}

pub fn interp_vmesh(
    vm_in: VMeshGrid,
    prof_at: &dyn Fn(usize, usize, usize) -> Vec3,
    nseg: usize,
) -> VMeshGrid {
    let n = vm_in.n;
    let ns_in = vm_in.ns;
    let nseg2 = nseg / 2;
    let odd = nseg & 1;
    let mut vm_out = VMeshGrid::new(n, nseg);
    let eps = 1e-6f32;
    let fill_fracs = |vm: &VMeshGrid, i: usize| -> Vec<f32> {
        let mut frac = vec![0.0f32; vm.ns + 1];
        let mut total = 0.0f32;
        for k in 0..vm.ns {
            total += vm.get(i, 0, k).distance(vm.get(i, 0, k + 1));
            frac[k + 1] = total;
        }
        if total > 0.0 {
            for k in 1..=vm.ns {
                frac[k] /= total;
            }
        } else {
            frac[vm.ns] = 1.0;
        }
        frac
    };
    let interp_range = |frac: &[f32], n: usize, f: f32| -> (usize, f32) {
        for i in 0..n {
            if f <= frac[i + 1] {
                let rest = f - frac[i];
                let r = if rest == 0.0 {
                    0.0
                } else {
                    rest / (frac[i + 1] - frac[i])
                };
                if i == n - 1 && (r - 1.0).abs() < eps {
                    return (n, 0.0);
                }
                return (i, r);
            }
        }
        (n, 0.0)
    };
    let bilerp = |q0: Vec3, q1: Vec3, q2: Vec3, q3: Vec3, u: f32, v: f32| {
        q0.lerp(q1, u).lerp(q3.lerp(q2, u), v)
    };
    let mut prev_frac = fill_fracs(&vm_in, n - 1);
    let mut prev_new_frac = {
        let mut frac = vec![0.0f32; nseg + 1];
        let mut total = 0.0f32;
        for k in 0..nseg {
            total += prof_at(n - 1, k, nseg).distance(prof_at(n - 1, k + 1, nseg));
            frac[k + 1] = total;
        }
        if total > 0.0 {
            for k in 1..=nseg {
                frac[k] /= total;
            }
        } else {
            frac[nseg] = 1.0;
        }
        frac
    };
    for i in 0..n {
        let frac = fill_fracs(&vm_in, i);
        let new_frac = {
            let mut frac = vec![0.0f32; nseg + 1];
            let mut total = 0.0f32;
            for k in 0..nseg {
                total += prof_at(i, k, nseg).distance(prof_at(i, k + 1, nseg));
                frac[k + 1] = total;
            }
            if total > 0.0 {
                for k in 1..=nseg {
                    frac[k] /= total;
                }
            } else {
                frac[nseg] = 1.0;
            }
            frac
        };
        for j in 0..=nseg2 - 1 + odd {
            for k in 0..=nseg2 {
                let (k_in, restk) = interp_range(&frac, ns_in, new_frac[k]);
                let (k_in_prev, restkprev) =
                    interp_range(&prev_frac, ns_in, prev_new_frac[nseg - j]);
                let mut j_in = ns_in - k_in_prev;
                let mut restj = -restkprev;
                if restj > -eps {
                    restj = 0.0;
                } else {
                    j_in = j_in.saturating_sub(1);
                    restj = 1.0 + restj;
                }
                let co = if restj < eps && restk < eps {
                    vm_in.get_canon(i, j_in, k_in)
                } else {
                    let j0 = if restj < eps || j_in == ns_in { 0 } else { 1 };
                    let k0 = if restk < eps || k_in == ns_in { 0 } else { 1 };
                    let q0 = vm_in.get_canon(i, j_in, k_in);
                    let q1 = vm_in.get_canon(i, j_in, (k_in + k0).min(ns_in));
                    let q2 = vm_in.get_canon(i, (j_in + j0).min(ns_in), (k_in + k0).min(ns_in));
                    let q3 = vm_in.get_canon(i, (j_in + j0).min(ns_in), k_in);
                    bilerp(q0, q1, q2, q3, restk, restj)
                };
                vm_out.set(i, j, k, co);
            }
        }
        prev_frac = frac;
        prev_new_frac = new_frac;
    }
    if odd == 0 {
        let ns2 = ns_in / 2;
        vm_out.set(0, nseg2, nseg2, vm_in.get(0, ns2, ns2));
    }
    vm_out.copy_equiv();
    for i in 0..n {
        let inext = (i + 1) % n;
        for k in 0..=nseg {
            let v = prof_at(i, k, nseg);
            vm_out.set(i, 0, k, v);
            let nseg2 = nseg / 2;
            if k >= (nseg - nseg2) && k < nseg {
                vm_out.set(inext, nseg - k, 0, v);
            }
        }
    }
    vm_out.copy_equiv();
    vm_out
}

pub fn emit_vmesh_faces(
    vm: &VMeshGrid,
    ns: usize,
    out_p: &mut Vec<Vec3>,
    add_point: &dyn Fn(Vec3, &mut Vec<Vec3>) -> usize,
    boundary_id: &dyn Fn(usize, usize) -> Option<usize>,
    out_polys: &mut Vec<Poly>,
) {
    let n_bndv = vm.n;
    let ns2 = ns / 2;
    let odd = ns & 1;
    let mut vm_pts: HashMap<(usize, usize, usize), usize> = HashMap::new();
    let mut v_at = |i: usize, j: usize, k: usize| -> usize {
        let (ci, cj, ck) = vm.canon(i, j, k);
        // Blender overlap rule: some boundary points are stored on neighbor sector's k==0 edge:
        // vm_out[inext, ns-k, 0] = profile[k] and symmetric on the other side.
        // So canonical points with ck==0/ns and cj>0 should reuse adjacent profile ids.
        if cj == 0 {
            if let Some(id) = boundary_id(ci, ck) {
                return id;
            }
        }
        if ck == 0 && cj > 0 && cj <= ns2 {
            let iprev = (ci + n_bndv - 1) % n_bndv;
            if let Some(id) = boundary_id(iprev, ns.saturating_sub(cj)) {
                return id;
            }
        }
        if ck == ns && cj > 0 && cj <= ns2 {
            let inext = (ci + 1) % n_bndv;
            if let Some(id) = boundary_id(inext, cj) {
                return id;
            }
        }
        if let Some(&id) = vm_pts.get(&(ci, cj, ck)) {
            return id;
        }
        let id = add_point(vm.get(ci, cj, ck), out_p);
        vm_pts.insert((ci, cj, ck), id);
        id
    };
    let kmax = if odd == 0 { ns2 } else { ns2 + 1 };
    for i in 0..n_bndv {
        for j in 0..ns2 {
            for k in 0..kmax {
                let a = v_at(i, j, k);
                let b = v_at(i, j, k + 1);
                let c = v_at(i, j + 1, k + 1);
                let d = v_at(i, j + 1, k);
                out_polys.push(vec![a, b, c, d]);
            }
        }
    }
    if odd != 0 {
        let mut poly: Vec<usize> = (0..n_bndv).map(|i| v_at(i, ns2, ns2)).collect();
        // poly is already CCW
        if poly.len() >= 3 {
            out_polys.push(poly);
        }
    }
}
