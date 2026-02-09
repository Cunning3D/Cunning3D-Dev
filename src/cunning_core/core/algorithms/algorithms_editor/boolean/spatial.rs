use bevy::math::Vec3;
use crate::libs::geometry::mesh::Geometry;
use bvh::aabb::{Aabb, Bounded};
use bvh::bounding_hierarchy::BHShape;
use bvh::bvh::Bvh as BvhImpl;
use nalgebra::{Point3, Vector3};
use rayon::prelude::*;

/// A wrapper for a primitive index that holds its pre-computed AABB.
/// This allows us to use the `bvh` crate for high-performance spatial queries.
#[derive(Debug, Clone)]
pub struct PrimitiveShape {
    pub prim_index: usize,
    pub node_index: usize, // Index in the Geometry's primitives array
    pub aabb: Aabb<f32, 3>,    // Explicitly specify dimensions for bvh 0.12+
}

impl PrimitiveShape {
    pub fn new(prim_idx: usize, geo: &Geometry, positions: &[Vec3]) -> Self {
        let prim = &geo.primitives().values()[prim_idx];
        
        let mut min = Point3::new(f32::MAX, f32::MAX, f32::MAX);
        let mut max = Point3::new(f32::MIN, f32::MIN, f32::MIN);
        
        // We use the single-precision P for BVH (acceleration).
        // Actual intersection will use Double Precision.
        for &v_idx in prim.vertices() {
            if let Some(v) = geo.vertices().get(v_idx.into()) {
                if let Some(p_idx) = geo.points().get_dense_index(v.point_id.into()) {
                    if let Some(pos) = positions.get(p_idx) {
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

/// High-performance BVH wrapper using the `bvh` crate.
pub struct SpatialIndex {
    bvh: BvhImpl<f32, 3>,
    shapes: Vec<PrimitiveShape>,
}

impl SpatialIndex {
    pub fn build(geo: &Geometry) -> Option<Self> {
        let positions = geo.get_point_position_attribute()?;
        let mut shapes: Vec<PrimitiveShape> = (0..geo.primitives().len())
            .into_par_iter()
            .map(|i| PrimitiveShape::new(i, geo, positions))
            .collect();

        if shapes.is_empty() {
            return None;
        }

        // Build the BVH (Surface Area Heuristic)
        // Note: build sorts the shapes in-place.
        let bvh = BvhImpl::build(&mut shapes);

        Some(Self { bvh, shapes })
    }

    /// Find all primitives that potentially intersect with the given AABB.
    /// 
    /// `aabb_min`, `aabb_max`: World space bounds of the query object.
    pub fn query_aabb(&self, aabb_min: Vec3, aabb_max: Vec3) -> Vec<usize> {
        let min = Point3::new(aabb_min.x, aabb_min.y, aabb_min.z);
        let max = Point3::new(aabb_max.x, aabb_max.y, aabb_max.z);
        let query_aabb = Aabb::with_bounds(min, max);

        let mut results = Vec::new();
        // Root node is always at index 0 for bvh implementation
        let mut stack = vec![0]; 

        let nodes = &self.bvh.nodes;
        
        while let Some(node_idx) = stack.pop() {
            let node = &nodes[node_idx];
            
            match node {
                bvh::bvh::BvhNode::Node { child_l_index, child_l_aabb, child_r_index, child_r_aabb, .. } => {
                    if child_l_aabb.intersects_aabb(&query_aabb) {
                        stack.push(*child_l_index);
                    }
                    if child_r_aabb.intersects_aabb(&query_aabb) {
                        stack.push(*child_r_index);
                    }
                }
                bvh::bvh::BvhNode::Leaf { shape_index, .. } => {
                    // shape_index is the index in the *reordered* shapes vector
                    if let Some(shape) = self.shapes.get(*shape_index) {
                        // Check shape AABB just in case (Leaf node doesn't store it in 0.12? It does not.)
                        if shape.aabb.intersects_aabb(&query_aabb) {
                            results.push(shape.prim_index);
                        }
                    }
                }
            }
        }
        
        results
    }
}
