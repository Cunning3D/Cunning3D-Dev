//! VMesh Cutoff method: port of Blender bevel_build_cutoff (5751-5889).
//! Creates corner vertices at bottom of cutoff faces, closing off each profile.
use super::vmesh::VMeshGrid;
use bevy::prelude::*;

/// Build cutoff vmesh: creates closed-off profiles instead of grid fill.
/// Blender 5751-5889: bevel_build_cutoff.
pub fn build_cutoff_vmesh(
    boundary_positions: &[Vec3],
    profile_positions: &[Vec<Vec3>], // profile_positions[i] = points along profile i
    v_pos: Vec3,
    v_normal: Vec3,
    ns: usize, // segments
) -> (Vec<Vec3>, Vec<Vec<usize>>) {
    let n_bndv = boundary_positions.len();
    if n_bndv < 2 || ns == 0 {
        return (vec![], vec![]);
    }

    let mut out_points: Vec<Vec3> = Vec::new();
    let mut out_polys: Vec<Vec<usize>> = Vec::new();

    // 1) Compute corner vertices at bottom of cutoff faces (Blender 5761-5783)
    let mut corner_verts: Vec<Vec3> = Vec::with_capacity(n_bndv);
    for i in 0..n_bndv {
        let prev_i = if i == 0 { n_bndv - 1 } else { i - 1 };

        // Get profile plane normals (approximated from profile directions)
        let plane_no_curr = if profile_positions[i].len() >= 2 {
            (profile_positions[i][1] - profile_positions[i][0]).normalize_or_zero()
        } else {
            Vec3::Y
        };
        let plane_no_prev = if profile_positions[prev_i].len() >= 2 {
            (profile_positions[prev_i][1] - profile_positions[prev_i][0]).normalize_or_zero()
        } else {
            Vec3::Y
        };

        // Down direction: cross of adjacent profile normals
        let mut down = plane_no_curr.cross(plane_no_prev).normalize_or_zero();
        if down.dot(v_normal) > 0.0 {
            down = -down;
        }

        // Average profile height
        let height_curr = profile_height(&profile_positions[i]);
        let height_prev = profile_height(&profile_positions[prev_i]);
        let length = (height_curr / 2.0_f32.sqrt() + height_prev / 2.0_f32.sqrt()) / 2.0;

        // Corner vert position
        let corner = boundary_positions[i] + down * length;
        corner_verts.push(corner);
    }

    // 2) Check if we should build center face (corners not collapsed)
    let build_center = if n_bndv == 3 {
        (corner_verts[0] - corner_verts[1]).length_squared() > 1e-6
            && (corner_verts[0] - corner_verts[2]).length_squared() > 1e-6
            && (corner_verts[1] - corner_verts[2]).length_squared() > 1e-6
    } else {
        true
    };

    // 3) Add corner vertices to output
    let corner_start = out_points.len();
    for cv in &corner_verts {
        out_points.push(*cv);
    }

    // 4) Add profile vertices to output
    let mut profile_starts: Vec<usize> = Vec::with_capacity(n_bndv);
    for prof in profile_positions {
        profile_starts.push(out_points.len());
        for p in prof {
            out_points.push(*p);
        }
    }

    // 5) Build cutoff faces for each profile (Blender 5833-5876)
    for i in 0..n_bndv {
        let corner1_idx = corner_start + i;
        let prof_start = profile_starts[i];
        let prof_len = profile_positions[i].len();

        // Cutoff face: corner1 + profile points + corner2 (if build_center)
        let mut face: Vec<usize> = Vec::with_capacity(prof_len + 2);
        face.push(corner1_idx);
        for k in 0..prof_len {
            face.push(prof_start + k);
        }
        if build_center {
            let corner2_idx = corner_start + ((i + 1) % n_bndv);
            face.push(corner2_idx);
        }
        out_polys.push(face);
    }

    // 6) Build center face if needed (Blender 5879-5887)
    if build_center {
        let center_face: Vec<usize> = (0..n_bndv).map(|i| corner_start + i).collect();
        out_polys.push(center_face);
    }

    (out_points, out_polys)
}

/// Calculate profile height (distance from start to end).
fn profile_height(profile: &[Vec3]) -> f32 {
    if profile.len() < 2 {
        return 0.0;
    }
    (profile.last().unwrap() - profile.first().unwrap()).length()
}

/// Convert cutoff result to VMeshGrid for compatibility.
pub fn cutoff_to_vmesh(boundary: &[Vec3], profiles: &[Vec<Vec3>], ns: usize) -> VMeshGrid {
    let n = boundary.len();
    // Initialize grid with boundary positions
    let mut grid = VMeshGrid::new(n, ns);
    for i in 0..n {
        if profiles[i].len() > 0 {
            grid.set(i, 0, 0, profiles[i][0]); // First point of each profile
        }
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_cutoff_basic() {
        let boundary = vec![
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(-1.0, 0.0, 0.0),
        ];
        let profiles = vec![
            vec![Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.5, 0.5, 0.0)],
            vec![Vec3::new(0.0, 1.0, 0.0), Vec3::new(-0.5, 0.5, 0.0)],
            vec![Vec3::new(-1.0, 0.0, 0.0), Vec3::new(-0.5, -0.5, 0.0)],
        ];
        let v_pos = Vec3::ZERO;
        let v_normal = Vec3::Y;

        let (points, polys) = build_cutoff_vmesh(&boundary, &profiles, v_pos, v_normal, 1);
        assert!(!points.is_empty());
        assert!(!polys.is_empty());
    }
}
