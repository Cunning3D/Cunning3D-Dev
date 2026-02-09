use bevy::math::{Vec3};
use crate::libs::geometry::mesh::Geometry;
use bvh::aabb::{Aabb, Bounded};
use bvh::bounding_hierarchy::BHShape;
use nalgebra::{Point3, Vector3};

/// A wrapper for a primitive index that holds its pre-computed AABB.
/// This allows us to use the `bvh` crate for high-performance spatial queries.
#[derive(Debug, Clone)]
pub struct PrimitiveShape {
    pub prim_index: usize,
    pub node_index: usize, // Index in the Geometry's primitives array
    pub aabb: Aabb<f32, 3>,    // Explicitly specify dimensions for bvh 0.12+
}

impl PrimitiveShape {
    /// Create a new PrimitiveShape.
    /// `pos_attr` should be the full slice of point positions.
    pub fn new(prim_idx: usize, geo: &Geometry, pos_attr: &[Vec3]) -> Self {
        let prim = &geo.primitives().values()[prim_idx];
        
        let mut min = Point3::new(f32::MAX, f32::MAX, f32::MAX);
        let mut max = Point3::new(f32::MIN, f32::MIN, f32::MIN);
        
        // We use the single-precision P for BVH (acceleration).
        // Actual intersection will use Double Precision if needed.
        for &v_id in prim.vertices() {
            if let Some(v) = geo.vertices().get(v_id.into()) {
                if let Some(pos_dense_idx) = geo.points().get_dense_index(v.point_id.into()) {
                    if let Some(pos) = pos_attr.get(pos_dense_idx) {
                        let p = Point3::new(pos.x, pos.y, pos.z);
                        // nalgebra inf/sup logic
                        min = min.inf(&p);
                        max = max.sup(&p);
                    }
                }
            }
        }
        
        // Add epsilon padding to avoid zero-volume boxes
        let eps = Vector3::new(1e-4, 1e-4, 1e-4);
        min -= eps;
        max += eps;

        Self {
            prim_index: prim_idx,
            node_index: prim_idx,
            aabb: Aabb::with_bounds(min, max),
        }
    }
}

impl Bounded<f32, 3> for PrimitiveShape {
    fn aabb(&self) -> Aabb<f32, 3> {
        self.aabb
    }
}

impl BHShape<f32, 3> for PrimitiveShape {
    fn set_bh_node_index(&mut self, index: usize) {
        self.node_index = index;
    }

    fn bh_node_index(&self) -> usize {
        self.node_index
    }
}
