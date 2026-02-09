//! SquareIn vmesh: port of Blender build_square_in_vmesh (5127–5155).
//! Special case when profile == 0 (PRO_SQUARE_IN_R) and there are 3 beveled edges (tri-corner).
//! Boundary verts merge according to pattern: (i, 0, k) merged with (i+1, 0, ns-k) for k <= ns/2.
use super::super::boundary::BoundaryResult;
use super::super::vmesh::VMeshGrid;
use bevy::prelude::*;

/// Result of SquareIn weld operation.
#[derive(Clone, Debug)]
pub struct SquareInResult {
    /// Welded vertex indices for each boundary profile.
    /// ids[i][k] gives the final vertex index for profile i at position k.
    pub ids: Vec<Vec<usize>>,
    /// If odd segments, the center triangle vertex indices.
    pub center_tri: Option<[usize; 3]>,
}

/// Apply the SquareIn vertex welding pattern.
///
/// # Preconditions
/// - Profile is PRO_SQUARE_IN_R (profile_r ~ 0.0)
/// - Exactly 3 beveled edges (tri-corner)
/// - ns >= 2
///
/// # Arguments
/// * `boundary` - boundary result (must have count == 3)
/// * `profiles` - coordinates for each profile point, profiles[i][k] = Vec3
/// * `ns` - number of segments
/// * `add_point` - closure to add a new point, returns its index
///
/// # Returns
/// None if preconditions not met; otherwise SquareInResult with welded ids.
pub fn try_square_in_weld<F>(
    boundary: &BoundaryResult,
    profiles: &[Vec<Vec3>],
    ns: usize,
    mut add_point: F,
) -> Option<SquareInResult>
where
    F: FnMut(Vec3) -> usize,
{
    let n = boundary.count;
    if n != 3 || ns < 2 {
        return None;
    }
    if profiles.len() != n {
        return None;
    }
    for p in profiles {
        if p.len() != ns + 1 {
            return None;
        }
    }

    let ns2 = ns / 2;
    let odd = ns % 2 == 1;

    // ids[i][k] will hold the final vertex index for profile i, position k.
    let mut ids: Vec<Vec<usize>> = vec![vec![usize::MAX; ns + 1]; n];

    // Blender 5135–5147: for each boundvert and each k in [1, ns-1]:
    // - if i > 0 && k <= ns2: merge with (i-1, 0, ns-k)
    // - else if i == n-1 && k > ns2: merge with (0, 0, ns-k)
    // - else: create new vertex
    for i in 0..n {
        for k in 1..ns {
            let co = profiles[i][k];
            if i > 0 && k <= ns2 {
                // Merge with previous profile's mirrored position.
                ids[i][k] = ids[i - 1][ns - k];
            } else if i == n - 1 && k > ns2 {
                // Last profile, wrap to first.
                ids[i][k] = ids[0][ns - k];
            } else {
                // Create new vertex.
                ids[i][k] = add_point(co);
            }
        }
    }

    // Endpoints (k=0 and k=ns) are the boundary corners, should be provided externally.
    // For now, mark them as MAX (caller must fill in).
    for i in 0..n {
        ids[i][0] = usize::MAX;
        ids[i][ns] = usize::MAX;
    }

    // Blender 5149–5154: if odd, set center points and build center ngon.
    let center_tri = if odd {
        // Center vertex for each profile is at (i, ns2, ns2) which maps to (i, 0, ns2).
        // All n profiles share these ns2 positions for center.
        Some([ids[0][ns2], ids[1][ns2], ids[2][ns2]])
    } else {
        None
    };

    Some(SquareInResult { ids, center_tri })
}

/// Apply SquareIn weld and update profile ids in-place.
/// This is the high-level function to integrate with the pipeline.
///
/// # Arguments
/// * `profile_ids` - mutable slice of profile id arrays, will be updated with welded ids
/// * `profile_coords` - coordinates for each profile
/// * `ns` - segments
/// * `add_point` - closure to add new vertex
///
/// Returns true if weld was applied, false if skipped (preconditions not met).
pub fn apply_square_in_weld<F>(
    profile_ids: &mut [Vec<usize>],
    profile_coords: &[Vec<Vec3>],
    ns: usize,
    add_point: F,
) -> bool
where
    F: FnMut(Vec3) -> usize,
{
    if profile_ids.len() != 3 || profile_coords.len() != 3 {
        return false;
    }
    let boundary = BoundaryResult {
        bnd_verts: vec![],
        edges: vec![],
        count: 3,
    };
    let result = match try_square_in_weld(&boundary, profile_coords, ns, add_point) {
        Some(r) => r,
        None => return false,
    };

    // Copy welded ids to profiles (excluding endpoints which are already set).
    for i in 0..3 {
        for k in 1..ns {
            if result.ids[i][k] != usize::MAX {
                profile_ids[i][k] = result.ids[i][k];
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_square_in_weld_basic() {
        let mut next_id = 0usize;
        let add_point = |_: Vec3| {
            let id = next_id;
            next_id += 1;
            id
        };
        let boundary = BoundaryResult {
            bnd_verts: vec![],
            edges: vec![],
            count: 3,
        };
        let profiles = vec![
            vec![Vec3::ZERO; 5], // ns=4, so 5 points
            vec![Vec3::ZERO; 5],
            vec![Vec3::ZERO; 5],
        ];
        let result = try_square_in_weld(&boundary, &profiles, 4, add_point);
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.ids.len(), 3);
        // Check merging pattern: ids[1][1] should equal ids[0][3], ids[1][2] should equal ids[0][2].
        assert_eq!(r.ids[1][1], r.ids[0][3]);
        assert_eq!(r.ids[1][2], r.ids[0][2]);
        // ids[2][3] should equal ids[0][1].
        assert_eq!(r.ids[2][3], r.ids[0][1]);
    }

    #[test]
    fn test_square_in_odd() {
        let mut next_id = 0usize;
        let add_point = |_: Vec3| {
            let id = next_id;
            next_id += 1;
            id
        };
        let boundary = BoundaryResult {
            bnd_verts: vec![],
            edges: vec![],
            count: 3,
        };
        let profiles = vec![
            vec![Vec3::ZERO; 6], // ns=5 (odd)
            vec![Vec3::ZERO; 6],
            vec![Vec3::ZERO; 6],
        ];
        let result = try_square_in_weld(&boundary, &profiles, 5, add_point);
        assert!(result.is_some());
        let r = result.unwrap();
        assert!(r.center_tri.is_some());
    }
}
