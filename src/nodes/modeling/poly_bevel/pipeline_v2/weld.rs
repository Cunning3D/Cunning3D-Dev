//! Weld case handling: port of Blender weld profile logic (6092-6176).
//! Special handling when exactly 2 beveled edges meet at a vertex.
use bevy::prelude::*;

const BEVEL_EPSILON: f32 = 1e-6;

/// Check if this is a weld case: exactly 2 beveled edges at vertex with only 2 boundary verts.
pub fn is_weld_case(selcount: usize, n_bndv: usize) -> bool {
    selcount == 2 && n_bndv == 2
}

/// Blender move_weld_profile_planes (2066-2099) port: update projection plane normals for two weld profiles.
pub fn move_weld_profile_planes(
    v_pos: Vec3,
    bnd1: Vec3,
    bnd2: Vec3,
    proj_dir1: Vec3,
    proj_dir2: Vec3,
    plane_no1: &mut Vec3,
    plane_no2: &mut Vec3,
) {
    if proj_dir1.length_squared() < 1e-12 || proj_dir2.length_squared() < 1e-12 {
        return;
    }
    let d1 = v_pos - bnd1;
    let d2 = v_pos - bnd2;
    let mut no = d1.cross(d2);
    let l1 = no.length();
    if l1 < 1e-12 {
        return;
    }
    no /= l1;
    let mut no2 = d1.cross(proj_dir1);
    let l2 = no2.length();
    if l2 >= 1e-12 {
        no2 /= l2;
    }
    let mut no3 = d2.cross(proj_dir2);
    let l3 = no3.length();
    if l3 >= 1e-12 {
        no3 /= l3;
    }
    if l2 < 1e-12 && l3 < 1e-12 {
        return;
    }
    let dot1 = no.dot(no2).abs();
    let dot2 = no.dot(no3).abs();
    if (dot1 - 1.0).abs() > BEVEL_EPSILON {
        *plane_no1 = no;
    }
    if (dot2 - 1.0).abs() > BEVEL_EPSILON {
        *plane_no2 = no;
    }
}

/// Build weld profile: creates a single curved edge connecting two boundverts (Blender 6150-6176).
/// Returns merged profile positions.
pub fn build_weld_profile(
    profile1: &[Vec3],
    profile2: &[Vec3],
    super_r1: f32,
    super_r2: f32,
    ns: usize,
) -> Vec<Vec3> {
    let mut result = Vec::with_capacity(ns + 1);

    const PRO_LINE_R: f32 = 1e4;

    for k in 0..=ns {
        let idx1 = k;
        let idx2 = ns - k;

        let v1 = profile1.get(idx1).copied().unwrap_or(Vec3::ZERO);
        let v2 = profile2.get(idx2).copied().unwrap_or(Vec3::ZERO);

        // Use the point from the other profile if one is a special case (line profile)
        let merged = if super_r1.abs() > PRO_LINE_R * 0.9 && super_r2.abs() < PRO_LINE_R * 0.9 {
            v2
        } else if super_r2.abs() > PRO_LINE_R * 0.9 && super_r1.abs() < PRO_LINE_R * 0.9 {
            v1
        } else {
            // Midpoint if profiles aren't on the same plane
            (v1 + v2) * 0.5
        };

        result.push(merged);
    }

    result
}

/// Emit weld edge polygons (strip of quads connecting the weld profiles).
pub fn emit_weld_edge_polys(
    weld_profile: &[Vec3],
    profile1_ids: &[usize],
    profile2_ids: &[usize],
    out_p: &mut Vec<Vec3>,
    out_polys: &mut Vec<Vec<usize>>,
) {
    let ns = weld_profile.len().saturating_sub(1);
    if ns == 0 {
        return;
    }

    // Add weld profile points
    let base = out_p.len();
    for pt in weld_profile {
        out_p.push(*pt);
    }

    // Create quads between profile1 and weld profile
    for k in 0..ns {
        let p1_k = profile1_ids.get(k).copied().unwrap_or(base + k);
        let p1_k1 = profile1_ids.get(k + 1).copied().unwrap_or(base + k + 1);
        let w_k = base + k;
        let w_k1 = base + k + 1;

        out_polys.push(vec![p1_k, p1_k1, w_k1, w_k]);
    }

    // Create quads between weld profile and profile2
    for k in 0..ns {
        let p2_k = profile2_ids.get(ns - k).copied().unwrap_or(base + ns - k);
        let p2_k1 = profile2_ids
            .get(ns - k - 1)
            .copied()
            .unwrap_or(base + ns - k - 1);
        let w_k = base + k;
        let w_k1 = base + k + 1;

        out_polys.push(vec![w_k, w_k1, p2_k1, p2_k]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_weld_case() {
        assert!(is_weld_case(2, 2));
        assert!(!is_weld_case(2, 3));
        assert!(!is_weld_case(3, 2));
    }

    #[test]
    fn test_build_weld_profile() {
        let p1 = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.5, 0.5, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        ];
        let p2 = vec![
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.5, -0.5, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
        ];

        let result = build_weld_profile(&p1, &p2, 0.5, 0.5, 2);
        assert_eq!(result.len(), 3);
    }
}
