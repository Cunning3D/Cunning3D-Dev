use bevy::prelude::Vec3;
use parry3d::na::{Matrix3, Vector3 as NaVec3, Point3 as NaPoint3};

/// Helper to compute gradient (normal) at a grid point using central differences
fn compute_gradient(data: &[f32], dims: &[usize; 3], x: usize, y: usize, z: usize) -> Vec3 {
    let (w, h, d) = (dims[0], dims[1], dims[2]);
    let idx = |x, y, z| x + y * w + z * w * h;
    let v_x0 = if x > 0 { data[idx(x - 1, y, z)] } else { data[idx(x, y, z)] };
    let v_x1 = if x < w - 1 { data[idx(x + 1, y, z)] } else { data[idx(x, y, z)] };
    let v_y0 = if y > 0 { data[idx(x, y - 1, z)] } else { data[idx(x, y, z)] };
    let v_y1 = if y < h - 1 { data[idx(x, y + 1, z)] } else { data[idx(x, y, z)] };
    let v_z0 = if z > 0 { data[idx(x, y, z - 1)] } else { data[idx(x, y, z)] };
    let v_z1 = if z < d - 1 { data[idx(x, y, z + 1)] } else { data[idx(x, y, z)] };
    Vec3::new(v_x1 - v_x0, v_y1 - v_y0, v_z1 - v_z0).normalize_or_zero()
}

/// Solve QEF (Quadratic Error Function) for Dual Contouring
/// Returns the position that minimizes the sum of squared distances to the tangent planes.
/// Fallbacks to centroid if unstable.
fn solve_qef(points: &[Vec3], normals: &[Vec3], centroid: Vec3) -> Vec3 {
    let mut ata = Matrix3::zeros();
    let mut atb = NaVec3::zeros();
    for (p, n) in points.iter().zip(normals.iter()) {
        let na = NaVec3::new(n.x, n.y, n.z);
        let pa = NaVec3::new(p.x, p.y, p.z);
        ata += na * na.transpose();
        let d = na.dot(&pa);
        atb += na * d;
    }
    let reg = 0.1;
    let center_na = NaVec3::new(centroid.x, centroid.y, centroid.z);
    ata += Matrix3::identity() * reg;
    atb += center_na * reg;
    match ata.try_inverse() { Some(inv) => { let res = inv * atb; Vec3::new(res.x, res.y, res.z) }, None => centroid }
}

/// Dual Voxel Meshing Algorithm (Surface Nets / Dual Contouring)
pub fn extract_surface_nets(data: &[f32], dims: [usize; 3], iso_value: f32, hard_surface: bool) -> (Vec<f32>, Vec<usize>) {
    let (width, height, depth) = (dims[0], dims[1], dims[2]);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut voxel_to_vertex = vec![-1i32; width * height * depth];
    let (step_x, step_y, step_z) = (1, width, width * height);

    for z in 0..depth.saturating_sub(1) {
        for y in 0..height.saturating_sub(1) {
            for x in 0..width.saturating_sub(1) {
                let idx = x + y * width + z * width * height;
                let (v0, v1, v2, v3) = (data[idx], data[idx + step_x], data[idx + step_y], data[idx + step_x + step_y]);
                let (v4, v5, v6, v7) = (data[idx + step_z], data[idx + step_x + step_z], data[idx + step_y + step_z], data[idx + step_x + step_y + step_z]);
                let mut mask = 0;
                if v0 < iso_value { mask |= 1; } if v1 < iso_value { mask |= 2; } if v2 < iso_value { mask |= 4; } if v3 < iso_value { mask |= 8; }
                if v4 < iso_value { mask |= 16; } if v5 < iso_value { mask |= 32; } if v6 < iso_value { mask |= 64; } if v7 < iso_value { mask |= 128; }
                if mask == 0 || mask == 255 { continue; }

                let mut intersection_sum = Vec3::ZERO;
                let mut intersection_count = 0.0;
                let mut qef_points = Vec::new();
                let mut qef_normals = Vec::new();
                let mut check_edge = |val_a: f32, val_b: f32, pos_a: Vec3, pos_b: Vec3, grad_a: Vec3, grad_b: Vec3| {
                    if (val_a < iso_value) != (val_b < iso_value) {
                        let t = (iso_value - val_a) / (val_b - val_a);
                        let p = pos_a + (pos_b - pos_a) * t;
                        intersection_sum += p;
                        intersection_count += 1.0;
                        if hard_surface {
                            let n = grad_a.lerp(grad_b, t).normalize_or_zero();
                            qef_points.push(p);
                            qef_normals.push(n);
                        }
                    }
                };

                let p_base = Vec3::new(x as f32, y as f32, z as f32);
                let (px, py, pz) = (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0));
                let mut g = [Vec3::ZERO; 8];
                if hard_surface {
                    g[0] = compute_gradient(data, &dims, x, y, z);
                    g[1] = compute_gradient(data, &dims, x + 1, y, z);
                    g[2] = compute_gradient(data, &dims, x, y + 1, z);
                    g[3] = compute_gradient(data, &dims, x + 1, y + 1, z);
                    g[4] = compute_gradient(data, &dims, x, y, z + 1);
                    g[5] = compute_gradient(data, &dims, x + 1, y, z + 1);
                    g[6] = compute_gradient(data, &dims, x, y + 1, z + 1);
                    g[7] = compute_gradient(data, &dims, x + 1, y + 1, z + 1);
                }

                check_edge(v0, v1, p_base, p_base + px, g[0], g[1]);
                check_edge(v2, v3, p_base + py, p_base + py + px, g[2], g[3]);
                check_edge(v0, v2, p_base, p_base + py, g[0], g[2]);
                check_edge(v1, v3, p_base + px, p_base + px + py, g[1], g[3]);

                let p_top = p_base + pz;
                check_edge(v4, v5, p_top, p_top + px, g[4], g[5]);
                check_edge(v6, v7, p_top + py, p_top + py + px, g[6], g[7]);
                check_edge(v4, v6, p_top, p_top + py, g[4], g[6]);
                check_edge(v5, v7, p_top + px, p_top + px + py, g[5], g[7]);

                check_edge(v0, v4, p_base, p_base + pz, g[0], g[4]);
                check_edge(v1, v5, p_base + px, p_base + px + pz, g[1], g[5]);
                check_edge(v2, v6, p_base + py, p_base + py + pz, g[2], g[6]);
                check_edge(v3, v7, p_base + px + py, p_base + px + py + pz, g[3], g[7]);

                if intersection_count > 0.0 {
                    let centroid = intersection_sum / intersection_count;
                    let mut final_pos = centroid;
                    if hard_surface && qef_points.len() >= 3 {
                        let qef_pos = solve_qef(&qef_points, &qef_normals, centroid);
                        final_pos = centroid.lerp(qef_pos.clamp(p_base, p_base + Vec3::ONE), 0.7);
                    }
                    let new_idx = vertices.len() / 3;
                    voxel_to_vertex[idx] = new_idx as i32;
                    vertices.extend_from_slice(&[final_pos.x, final_pos.y, final_pos.z]);
                }
            }
        }
    }

    for z in 1..depth.saturating_sub(1) {
        for y in 1..height.saturating_sub(1) {
            for x in 0..width.saturating_sub(1) {
                let idx = x + y * width + z * width * height;
                let idx_next = (x + 1) + y * width + z * width * height;
                let inside_curr = data[idx] < iso_value;
                let inside_next = data[idx_next] < iso_value;
                if inside_curr != inside_next {
                    let c_bl = x + (y - 1) * width + (z - 1) * width * height;
                    let c_br = x + y * width + (z - 1) * width * height;
                    let c_tr = x + y * width + z * width * height;
                    let c_tl = x + (y - 1) * width + z * width * height;
                    let (i_bl, i_br, i_tr, i_tl) = (voxel_to_vertex[c_bl], voxel_to_vertex[c_br], voxel_to_vertex[c_tr], voxel_to_vertex[c_tl]);
                    if i_bl != -1 && i_br != -1 && i_tr != -1 && i_tl != -1 {
                        if inside_curr { indices.extend_from_slice(&[i_bl as usize, i_br as usize, i_tr as usize, i_tl as usize]); }
                        else { indices.extend_from_slice(&[i_bl as usize, i_tl as usize, i_tr as usize, i_br as usize]); }
                    }
                }
            }
        }
    }

    for z in 1..depth.saturating_sub(1) {
        for y in 0..height.saturating_sub(1) {
            for x in 1..width.saturating_sub(1) {
                let idx = x + y * width + z * width * height;
                let idx_next = x + (y + 1) * width + z * width * height;
                let inside_curr = data[idx] < iso_value;
                let inside_next = data[idx_next] < iso_value;
                if inside_curr != inside_next {
                    let c_bl = (x - 1) + y * width + (z - 1) * width * height;
                    let c_br = x + y * width + (z - 1) * width * height;
                    let c_tr = x + y * width + z * width * height;
                    let c_tl = (x - 1) + y * width + z * width * height;
                    let (i_bl, i_br, i_tr, i_tl) = (voxel_to_vertex[c_bl], voxel_to_vertex[c_br], voxel_to_vertex[c_tr], voxel_to_vertex[c_tl]);
                    if i_bl != -1 && i_br != -1 && i_tr != -1 && i_tl != -1 {
                        if inside_curr { indices.extend_from_slice(&[i_bl as usize, i_tl as usize, i_tr as usize, i_br as usize]); }
                        else { indices.extend_from_slice(&[i_bl as usize, i_br as usize, i_tr as usize, i_tl as usize]); }
                    }
                }
            }
        }
    }

    for z in 0..depth.saturating_sub(1) {
        for y in 1..height.saturating_sub(1) {
            for x in 1..width.saturating_sub(1) {
                let idx = x + y * width + z * width * height;
                let idx_next = x + y * width + (z + 1) * width * height;
                let inside_curr = data[idx] < iso_value;
                let inside_next = data[idx_next] < iso_value;
                if inside_curr != inside_next {
                    let c_bl = (x - 1) + (y - 1) * width + z * width * height;
                    let c_br = x + (y - 1) * width + z * width * height;
                    let c_tr = x + y * width + z * width * height;
                    let c_tl = (x - 1) + y * width + z * width * height;
                    let (i_bl, i_br, i_tr, i_tl) = (voxel_to_vertex[c_bl], voxel_to_vertex[c_br], voxel_to_vertex[c_tr], voxel_to_vertex[c_tl]);
                    if i_bl != -1 && i_br != -1 && i_tr != -1 && i_tl != -1 {
                        if inside_curr { indices.extend_from_slice(&[i_bl as usize, i_br as usize, i_tr as usize, i_tl as usize]); }
                        else { indices.extend_from_slice(&[i_bl as usize, i_tl as usize, i_tr as usize, i_br as usize]); }
                    }
                }
            }
        }
    }

    (vertices, indices)
}

