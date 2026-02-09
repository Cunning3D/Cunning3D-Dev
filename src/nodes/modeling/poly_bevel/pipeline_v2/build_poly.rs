//! Polygon building: port of Blender bevel_build_poly / bevel_build_trifan / bevel_vert_two_edges.
use super::super::structures::Profile;
use super::vmesh::VMeshGrid;
use super::Poly;
use bevy::prelude::*;

/// Build a simple polygon from boundary vertices (Blender bevel_build_poly 5891-5968).
/// Returns vertex indices for the polygon face.
pub fn build_poly(
    vm: &VMeshGrid,
    ns: usize,
    boundary_ids: &[Vec<usize>], // boundary_ids[i] = vertex IDs for boundvert i's arc
) -> Vec<usize> {
    let n = vm.n;
    let mut verts: Vec<usize> = Vec::new();

    for i in 0..n {
        // Add first vertex of each boundary arc
        if let Some(arc) = boundary_ids.get(i) {
            if let Some(&first) = arc.first() {
                verts.push(first);
            }
            // Add intermediate vertices along the arc (if seg > 1)
            for k in 1..ns {
                if let Some(&v) = arc.get(k) {
                    verts.push(v);
                }
            }
        }
    }

    verts
}

/// Build a triangle fan from a polygon (Blender bevel_build_trifan 5970-6021).
/// Takes a polygon and splits it into triangles radiating from a center vertex.
pub fn build_trifan(poly_verts: &[usize], out_polys: &mut Vec<Poly>) {
    let n = poly_verts.len();
    if n < 3 {
        return;
    }

    // Use first vertex as fan center
    let center = poly_verts[0];

    for i in 1..(n - 1) {
        let v1 = poly_verts[i];
        let v2 = poly_verts[i + 1];
        out_polys.push(vec![center, v1, v2]);
    }
}

/// Handle special case: vertex bevel with only two boundary verts (Blender bevel_vert_two_edges 6028-6077).
/// Creates a curved edge between the two boundary vertices.
pub fn bevel_vert_two_edges(
    vm: &VMeshGrid,
    ns: usize,
    weld1_idx: usize,
    weld2_idx: usize,
    add_point: &mut dyn FnMut(Vec3) -> usize,
    out_polys: &mut Vec<Poly>,
) -> Vec<usize> {
    let mut edge_verts: Vec<usize> = Vec::new();

    // Get positions along the two profiles
    for k in 0..=ns {
        let p1 = vm.get(weld1_idx, 0, k);
        let p2 = vm.get(weld2_idx, 0, ns - k);
        // Use midpoint for welded edge
        let mid = (p1 + p2) * 0.5;
        let idx = add_point(mid);
        edge_verts.push(idx);
    }

    // Create edge faces (quads along the edge)
    // This is typically handled by the edge polygon builder
    let _ = out_polys;

    edge_verts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_trifan() {
        let poly = vec![0, 1, 2, 3, 4];
        let mut prims = Vec::new();
        build_trifan(&poly, &mut prims);
        assert_eq!(prims.len(), 3); // n-2 triangles for n vertices
    }
}
