//! Terminal edge handling: port of Blender build_boundary_terminal_edge (2922–3057).
//! Handles the special case when only one edge is beveled at a vertex.
use super::super::structures::{BevelParams, MeshKind, OffsetType};
use super::boundary::{BoundVertLite, BoundaryResult};
use super::offset_limit::{offset_in_plane, slide_dist};
use bevy::prelude::*;

/// Result of terminal edge boundary construction.
#[derive(Clone, Debug, Default)]
pub struct TerminalEdgeResult {
    pub bnd_verts: Vec<BoundVertLite>,
    pub mesh_kind: MeshKind,
}

/// Blender build_boundary_terminal_edge (2922-3057): handle single beveled edge at vertex.
///
/// # Cases:
/// - 2 edges total: create triangle with artificial vertex on unbeveled edge
/// - 3+ edges: create poly/adj mesh with on-edge vertices
pub fn build_terminal_edge_boundary(
    edge_count: usize,
    bev_edge_idx: usize,
    edge_dirs: &[Vec3],    // Direction from vertex to each edge's other end
    face_normals: &[Vec3], // Face normal for each edge's face
    v_pos: Vec3,
    offset: f32,
    params: &BevelParams,
) -> TerminalEdgeResult {
    let mut result = TerminalEdgeResult::default();
    if edge_count < 2 || bev_edge_idx >= edge_count {
        return result;
    }

    let prev_idx = if bev_edge_idx == 0 {
        edge_count - 1
    } else {
        bev_edge_idx - 1
    };
    let next_idx = (bev_edge_idx + 1) % edge_count;

    let e_dir = edge_dirs.get(bev_edge_idx).copied().unwrap_or(Vec3::X);
    let prev_dir = edge_dirs.get(prev_idx).copied().unwrap_or(-Vec3::X);
    let next_dir = edge_dirs.get(next_idx).copied().unwrap_or(Vec3::Z);

    let prev_no = face_normals.get(prev_idx).copied().unwrap_or(Vec3::Y);
    let next_no = face_normals.get(next_idx).copied().unwrap_or(Vec3::Y);

    if edge_count == 2 {
        // Blender 2932-2966: 2 edges, create triangle
        // Left side of beveled edge
        let co_left = offset_in_plane(e_dir, v_pos, prev_no, offset, true);
        result.bnd_verts.push(BoundVertLite {
            pos: co_left,
            prev: 2,
            next: 1,
            index: 0,
            efirst: Some(bev_edge_idx),
            elast: Some(bev_edge_idx),
            ..Default::default()
        });

        // Right side of beveled edge
        let co_right = offset_in_plane(e_dir, v_pos, next_no, offset, false);
        result.bnd_verts.push(BoundVertLite {
            pos: co_right,
            prev: 0,
            next: 2,
            index: 1,
            efirst: Some(bev_edge_idx),
            elast: Some(bev_edge_idx),
            ..Default::default()
        });

        // Artificial point along unbeveled edge
        let co_slide = slide_dist(next_dir, v_pos, offset);
        result.bnd_verts.push(BoundVertLite {
            pos: co_slide,
            prev: 1,
            next: 0,
            index: 2,
            efirst: Some(next_idx),
            elast: Some(next_idx),
            ..Default::default()
        });

        result.mesh_kind = MeshKind::None; // Simple triangle, no mesh
    } else {
        // Blender 2968-3022: 3+ edges, create poly with on-edge verts
        let leg_slide = matches!(
            params.offset_type,
            OffsetType::Percent | OffsetType::Absolute
        );

        // Left side: meet or slide from prev edge
        let co_left = if leg_slide {
            slide_dist(prev_dir, v_pos, offset)
        } else {
            super::boundary::offset_meet(v_pos, prev_dir, e_dir, prev_no, offset, offset)
        };
        result.bnd_verts.push(BoundVertLite {
            pos: co_left,
            prev: edge_count - 1,
            next: 1,
            index: 0,
            efirst: Some(prev_idx),
            elast: Some(bev_edge_idx),
            ..Default::default()
        });

        // Right side: meet or slide to next edge
        let co_right = if leg_slide {
            slide_dist(next_dir, v_pos, offset)
        } else {
            super::boundary::offset_meet(v_pos, e_dir, next_dir, next_no, offset, offset)
        };
        result.bnd_verts.push(BoundVertLite {
            pos: co_right,
            prev: 0,
            next: 2,
            index: 1,
            efirst: Some(bev_edge_idx),
            elast: Some(next_idx),
            ..Default::default()
        });

        // Blender 3008-3022: slide along non-adjacent edges (wrapping iteration)
        let d = if params.profile_amount < 0.25 {
            offset * 2.0_f32.sqrt()
        } else {
            offset
        };
        let mut idx = 2usize;
        let mut ei = (next_idx + 1) % edge_count;
        while ei != prev_idx && ei != bev_edge_idx {
            let ed = edge_dirs.get(ei).copied().unwrap_or(Vec3::X);
            let co = slide_dist(ed, v_pos, d);
            result.bnd_verts.push(BoundVertLite {
                pos: co,
                prev: idx - 1,
                next: idx + 1,
                index: idx,
                efirst: Some(ei),
                elast: Some(ei),
                ..Default::default()
            });
            idx += 1;
            ei = (ei + 1) % edge_count;
        }

        // Fix up prev/next links
        let n = result.bnd_verts.len();
        for i in 0..n {
            result.bnd_verts[i].prev = if i == 0 { n - 1 } else { i - 1 };
            result.bnd_verts[i].next = (i + 1) % n;
        }

        // Blender 3036-3055: determine mesh kind
        result.mesh_kind = if n == 2 && edge_count == 3 {
            MeshKind::None
        } else if n == 3 {
            MeshKind::TriFan
        } else {
            MeshKind::Poly
        };
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_2_edges() {
        let result = build_terminal_edge_boundary(
            2,
            0,
            &[Vec3::X, Vec3::Z],
            &[Vec3::Y, Vec3::Y],
            Vec3::ZERO,
            0.1,
            &BevelParams::default(),
        );
        assert_eq!(result.bnd_verts.len(), 3);
        assert_eq!(result.mesh_kind, MeshKind::None);
    }

    #[test]
    fn test_terminal_3_edges() {
        let result = build_terminal_edge_boundary(
            3,
            0,
            &[Vec3::X, Vec3::Y, Vec3::Z],
            &[Vec3::Y, Vec3::Y, Vec3::Y],
            Vec3::ZERO,
            0.1,
            &BevelParams::default(),
        );
        assert!(result.bnd_verts.len() >= 2);
    }
}
