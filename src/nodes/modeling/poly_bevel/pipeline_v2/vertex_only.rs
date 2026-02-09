//! Vertex-Only Bevel: port of Blender build_boundary_vertex_only (2882-2915) and bevel_vert_two_edges (6028-6076).
//! Bevels vertices instead of edges - creates a chamfer/fillet at each selected vertex.
use super::super::structures::BevelParams;
use super::loop_slide::slide_dist;
use bevy::prelude::*;

/// Build boundary for vertex-only bevel (Blender 2882-2915).
/// Creates boundary verts by sliding along each edge from the vertex.
pub fn build_vertex_only_boundary(
    v_pos: Vec3,
    edge_dirs: &[Vec3], // Directions from v to each connected vertex
    offset: f32,
) -> Vec<Vec3> {
    // For vertex-only bevel, each edge gets a boundary vert at offset distance
    edge_dirs
        .iter()
        .map(|&dir| slide_dist(v_pos, dir, offset))
        .collect()
}

/// Build vertex-only vmesh: create geometry at a single vertex (Blender 6028-6076).
/// Returns (boundary_positions, center_position, output_polygons).
pub fn build_vertex_only_vmesh(
    v_pos: Vec3,
    edge_dirs: &[Vec3],
    face_normals: &[Vec3],
    offset: f32,
    seg: usize,
    pro_super_r: f32,
) -> (Vec<Vec3>, Vec<Vec<usize>>) {
    let n = edge_dirs.len();
    if n < 2 {
        return (vec![], vec![]);
    }

    let mut out_points: Vec<Vec3> = Vec::new();
    let mut out_polys: Vec<Vec<usize>> = Vec::new();

    // 1) Create boundary verts by sliding along each edge
    let boundary = build_vertex_only_boundary(v_pos, edge_dirs, offset);
    let boundary_base = out_points.len();
    for pt in &boundary {
        out_points.push(*pt);
    }

    // 2) For 2 edges case, create curved edge between them (bevel_vert_two_edges logic)
    if n == 2 && seg > 1 {
        let v1 = boundary[0];
        let v2 = boundary[1];
        let profile_base = out_points.len();
        for k in 1..seg {
            let t = k as f32 / seg as f32;
            out_points.push(superellipse_interp(v1, v2, v_pos, t, pro_super_r));
        }
        // Generate quad strip between boundary[0]→profile→boundary[1] and face_normals-based fan
        let avg_n = face_normals.iter().copied().fold(Vec3::ZERO, |a, b| a + b).normalize_or_zero();
        let mid_pt_idx = out_points.len();
        out_points.push(v_pos + avg_n * offset * 0.01); // Collapsed center point
        // Triangle fan: boundary[0] → profile_k → profile_k+1, then profile_last → boundary[1]
        let mut strip: Vec<usize> = vec![boundary_base];
        for k in 0..(seg - 1) { strip.push(profile_base + k); }
        strip.push(boundary_base + 1);
        for k in 0..(strip.len() - 1) {
            out_polys.push(vec![strip[k], strip[k + 1], mid_pt_idx]);
        }
    }
    // 3) For 3+ edges, create center polygon connecting all boundary verts
    else if n >= 3 {
        if seg == 1 {
            let poly: Vec<usize> = (0..n).map(|i| boundary_base + i).collect();
            out_polys.push(poly);
        } else {
            // ADJ mesh for multi-segment vertex bevel with proper ring-to-ring connections
            let mut prev_ring_base = boundary_base;
            for ring in 1..seg {
                let t = ring as f32 / seg as f32;
                let ring_base = out_points.len();
                for i in 0..n {
                    let boundary_pt = boundary[i];
                    let ring_pt = superellipse_interp(boundary_pt, v_pos, v_pos, t, pro_super_r);
                    out_points.push(ring_pt);
                }
                // Connect current ring to previous ring (or boundary if ring==1)
                for i in 0..n {
                    let i_next = (i + 1) % n;
                    out_polys.push(vec![
                        prev_ring_base + i,
                        prev_ring_base + i_next,
                        ring_base + i_next,
                        ring_base + i,
                    ]);
                }
                prev_ring_base = ring_base;
            }
            // Center polygon from last ring
            let center_poly: Vec<usize> = (0..n).map(|i| prev_ring_base + i).collect();
            out_polys.push(center_poly);
        }
    }

    (out_points, out_polys)
}

/// Superellipse interpolation from v1 to v2 with middle at v_pos.
fn superellipse_interp(v1: Vec3, v2: Vec3, v_mid: Vec3, t: f32, pro_super_r: f32) -> Vec3 {
    // For r=0: circle, r->inf: square
    const PRO_CIRCLE_R: f32 = 0.0;
    const PRO_SQUARE_R: f32 = 1e4;

    if pro_super_r.abs() < 0.01 {
        // Circle: use cosine interpolation through mid
        let angle = t * std::f32::consts::PI;
        let sin_a = angle.sin();
        let cos_a = angle.cos();

        // Parametric: blend between v1, v_mid, v2
        let a = v1.lerp(v_mid, (1.0 - cos_a) / 2.0);
        let b = v_mid.lerp(v2, (1.0 + cos_a) / 2.0);
        a.lerp(b, t)
    } else if pro_super_r > PRO_SQUARE_R * 0.9 {
        // Square: linear interpolation
        if t < 0.5 {
            v1.lerp(v_mid, t * 2.0)
        } else {
            v_mid.lerp(v2, (t - 0.5) * 2.0)
        }
    } else {
        // General superellipse
        let r = pro_super_r.clamp(0.01, 100.0);
        let angle = t * std::f32::consts::FRAC_PI_2;

        let cos_pow = angle.cos().powf(2.0 / r);
        let sin_pow = angle.sin().powf(2.0 / r);

        // Blend between axes
        let axis1 = v1 - v_mid;
        let axis2 = v2 - v_mid;

        v_mid + axis1 * cos_pow + axis2 * sin_pow
    }
}

/// Check if vertex-only bevel should be used (Blender affect_type == BEVEL_AFFECT_VERTICES).
pub fn is_vertex_only(params: &BevelParams) -> bool {
    matches!(
        params.affect,
        super::super::structures::BevelAffect::Vertices
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_vertex_only_boundary() {
        let v = Vec3::ZERO;
        let dirs = vec![Vec3::X, Vec3::Y, Vec3::NEG_X];
        let boundary = build_vertex_only_boundary(v, &dirs, 0.5);

        assert_eq!(boundary.len(), 3);
        assert!((boundary[0] - Vec3::new(0.5, 0.0, 0.0)).length() < 0.01);
        assert!((boundary[1] - Vec3::new(0.0, 0.5, 0.0)).length() < 0.01);
    }

    #[test]
    fn test_superellipse_circle() {
        let v1 = Vec3::new(1.0, 0.0, 0.0);
        let v2 = Vec3::new(0.0, 1.0, 0.0);
        let mid = Vec3::ZERO;

        let pt = superellipse_interp(v1, v2, mid, 0.5, 0.0);
        // Should be approximately on the arc
        assert!(pt.length() > 0.4);
    }
}
