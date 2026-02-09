use bevy::math::{DVec3, DVec2};
use robust::{orient3d, orient2d, Coord3D, Coord};

/// A robust geometric kernel wrapper.
/// Uses exact predicates for critical geometric tests to avoid floating point issues.
pub struct GeoKernel;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Orientation {
    Positive,
    Negative,
    Zero, // Coplanar or Collinear
}

impl GeoKernel {
    /// Robust 3D orientation test.
    /// Returns Positive if point d is below plane abc (right-hand rule),
    /// Negative if above, Zero if coplanar.
    /// Uses Shewchuk's predicates via the `robust` crate.
    #[inline]
    pub fn orient3d(a: DVec3, b: DVec3, c: DVec3, d: DVec3) -> Orientation {
        // robust::Coord3D is a generic struct Coord3D<T>
        let p_a = Coord3D { x: a.x, y: a.y, z: a.z };
        let p_b = Coord3D { x: b.x, y: b.y, z: b.z };
        let p_c = Coord3D { x: c.x, y: c.y, z: c.z };
        let p_d = Coord3D { x: d.x, y: d.y, z: d.z };

        let res = orient3d(p_a, p_b, p_c, p_d);
        
        if res > 0.0 {
            Orientation::Positive
        } else if res < 0.0 {
            Orientation::Negative
        } else {
            Orientation::Zero
        }
    }

    /// Check if a point P is inside triangle ABC (assuming P is already known to be on the plane).
    /// Uses robust orientation tests on 2D projections or 3D edge planes.
    pub fn point_in_triangle_3d(a: DVec3, b: DVec3, c: DVec3, p: DVec3, normal: DVec3) -> bool {
        // To be robust, we check orientation of P relative to edges AB, BC, CA.
        // We use a helper point (normal + p) to define the "positive" side of the plane.
        // Actually, for point-in-triangle on 3D plane, consistent winding check is better.
        
        // Edge AB
        let o1 = Self::orient3d(a, b, a + normal, p);
        // Edge BC
        let o2 = Self::orient3d(b, c, b + normal, p);
        // Edge CA
        let o3 = Self::orient3d(c, a, c + normal, p);

        // If all match the normal orientation (or are zero), it's inside.
        // Orientation depends on winding. Assuming CCW.
        // If normal is (A->B) x (A->C), then (A, B, A+N, P) should be... wait.
        // The standard "Same Side" test.
        
        // Simplified Robust Approach:
        // Project to 2D plane with largest normal component (to avoid precision loss).
        // Then do 2D orient2d checks. This is standard stable approach.
        
        let abs_n = normal.abs();
        let (u, v) = if abs_n.x >= abs_n.y && abs_n.x >= abs_n.z {
            (1, 2) // Project to YZ
        } else if abs_n.y >= abs_n.x && abs_n.y >= abs_n.z {
            (0, 2) // Project to XZ
        } else {
            (0, 1) // Project to XY
        };
        
        let project = |v: DVec3| -> Coord<f64> {
            if abs_n.x >= abs_n.y && abs_n.x >= abs_n.z {
                Coord { x: v.y, y: v.z }
            } else if abs_n.y >= abs_n.x && abs_n.y >= abs_n.z {
                Coord { x: v.x, y: v.z }
            } else {
                Coord { x: v.x, y: v.y }
            }
        };
        
        let pa = project(a);
        let pb = project(b);
        let pc = project(c);
        let pp = project(p);
        
        let res1 = orient2d(pa, pb, pp);
        let res2 = orient2d(pb, pc, pp);
        let res3 = orient2d(pc, pa, pp);
        
        // Check if all have same sign (or zero)
        let has_pos = res1 > 0.0 || res2 > 0.0 || res3 > 0.0;
        let has_neg = res1 < 0.0 || res2 < 0.0 || res3 < 0.0;
        
        !(has_pos && has_neg)
    }
}

/// Represents an intersection between a segment and a triangle.
#[derive(Debug, Clone)]
pub struct Intersection {
    pub point: DVec3,
    pub t: f64,          // Parameter on segment (0.0..=1.0)
    pub uv: DVec2,       // Barycentric coords on triangle (u, v)
    pub is_proper: bool, // True if strictly interior, False if on edge/vertex
}

impl GeoKernel {
    /// Intersects a segment P0->P1 with triangle T0-T1-T2.
    /// Returns None if no intersection or parallel/coplanar.
    /// 
    /// This implementation uses signed volumes (orient3d) to check straddle,
    /// ensuring strict robustness against epsilon issues.
    pub fn intersect_segment_triangle(
        p0: DVec3, p1: DVec3,
        t0: DVec3, t1: DVec3, t2: DVec3
    ) -> Option<Intersection> {
        // 1. Check if Segment straddles the Triangle Plane
        let o0 = Self::orient3d(t0, t1, t2, p0);
        let o1 = Self::orient3d(t0, t1, t2, p1);

        // If both points are on the same side, no intersection.
        if o0 == o1 && o0 != Orientation::Zero {
            return None;
        }
        
        // If both zero, segment is coplanar.
        // We handle coplanar cases separately (usually return None here and let coplanar handler deal with it).
        if o0 == Orientation::Zero && o1 == Orientation::Zero {
            return None; 
        }

        // 2. Check if Triangle edges straddle the Segment (conceptually)
        // Effectively, we check if the intersection point lies inside the triangle edges.
        // We do this by checking the orientation of P0->P1 against tetrahedrons formed by edges.
        
        // Use Coord3D struct explicitly for robust crate 1.2.0+
        let tp0 = Coord3D { x: p0.x, y: p0.y, z: p0.z };
        let tp1 = Coord3D { x: p1.x, y: p1.y, z: p1.z };
        let tt0 = Coord3D { x: t0.x, y: t0.y, z: t0.z };
        let tt1 = Coord3D { x: t1.x, y: t1.y, z: t1.z };
        let tt2 = Coord3D { x: t2.x, y: t2.y, z: t2.z };
        
        // Note: orient3d(a, b, c, d)
        let vol01_01 = robust::orient3d(tp0, tp1, tt0, tt1);
        let vol01_12 = robust::orient3d(tp0, tp1, tt1, tt2);
        let vol01_20 = robust::orient3d(tp0, tp1, tt2, tt0);

        // Map f64 result to Orientation enum manually
        // We re-implement mapping here to avoid borrow checker issues with Self::orient3d inside this method if any
        // or just for clarity.
        // Actually, let's use a helper or map directly.
        let map_orient = |val: f64| {
            if val > 0.0 { Orientation::Positive }
            else if val < 0.0 { Orientation::Negative }
            else { Orientation::Zero }
        };
        
        let vol01_01 = map_orient(vol01_01);
        let vol01_12 = map_orient(vol01_12);
        let vol01_20 = map_orient(vol01_20);

        // For intersection, the winding must be consistent.
        // i.e., all must be Positive/Zero or all Negative/Zero.
        
        let has_pos = vol01_01 == Orientation::Positive || vol01_12 == Orientation::Positive || vol01_20 == Orientation::Positive;
        let has_neg = vol01_01 == Orientation::Negative || vol01_12 == Orientation::Negative || vol01_20 == Orientation::Negative;
        
        if has_pos && has_neg {
            return None; // Outside triangle
        }

        // 3. Compute exact intersection point
        // If we are here, there is an intersection. We use standard ray-tri math for the coordinate,
        // knowing it IS valid.
        
        let edge1 = t1 - t0;
        let edge2 = t2 - t0;
        let h = (p1 - p0).cross(edge2);
        let a = edge1.dot(h);

        // Parallel check (should have been caught by orient3d, but good for safety)
        if a.abs() < 1e-14 {
            return None; 
        }
        
        let f = 1.0 / a;
        let s = p0 - t0;
        let u = f * s.dot(h);
        
        let q = s.cross(edge1);
        let v = f * (p1 - p0).dot(q);
        let t = f * edge2.dot(q);

        // Clamp due to float precision, relying on orient3d for the "truth" of intersection.
        let t_clamped = t.clamp(0.0, 1.0);

        // Proper intersection means not on boundary
        let is_proper = o0 != Orientation::Zero && o1 != Orientation::Zero 
                        && vol01_01 != Orientation::Zero && vol01_12 != Orientation::Zero && vol01_20 != Orientation::Zero;

        Some(Intersection {
            point: p0 + (p1 - p0) * t_clamped,
            t: t_clamped,
            uv: DVec2::new(u, v),
            is_proper
        })
    }

    /// Intersects a Ray (Origin, Dir) with Triangle T0-T1-T2.
    /// Returns t (distance) if intersection occurs, None otherwise.
    /// Uses Möller–Trumbore algorithm.
    pub fn intersect_ray_triangle(
        origin: DVec3, dir: DVec3,
        t0: DVec3, t1: DVec3, t2: DVec3
    ) -> Option<f64> {
        let edge1 = t1 - t0;
        let edge2 = t2 - t0;
        let h = dir.cross(edge2);
        let a = edge1.dot(h);

        if a > -1e-7 && a < 1e-7 {
            return None; // Parallel
        }

        let f = 1.0 / a;
        let s = origin - t0;
        let u = f * s.dot(h);

        if u < 0.0 || u > 1.0 {
            return None;
        }

        let q = s.cross(edge1);
        let v = f * dir.dot(q);

        if v < 0.0 || u + v > 1.0 {
            return None;
        }

        let t = f * edge2.dot(q);

        if t > 1e-7 {
            Some(t)
        } else {
            None
        }
    }
}
