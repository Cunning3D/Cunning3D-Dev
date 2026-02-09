use bevy::math::{Vec2, Vec3, Vec4, DVec2, DVec3, DVec4, Quat};

/// Trait for types that can be interpolated (mixed).
/// Used for attribute interpolation during topological operations (split, smooth, etc.).
pub trait Interpolatable: Sized + Clone + Send + Sync + 'static {
    /// Linear interpolation between two values.
    /// t = 0.0 -> a, t = 1.0 -> b
    fn mix(a: &Self, b: &Self, t: f32) -> Self;

    /// Barycentric / Weighted interpolation from N values.
    /// Weights should sum to 1.0 (though implementations may not strictly enforce this for performance).
    fn mix_n(values: &[&Self], weights: &[f32]) -> Self;
}

// --- Implementations ---

macro_rules! impl_lerp_float {
    ($t:ty) => {
        impl Interpolatable for $t {
            #[inline]
            fn mix(a: &Self, b: &Self, t: f32) -> Self {
                let t = t as $t;
                a * (1.0 - t) + b * t
            }

            #[inline]
            fn mix_n(values: &[&Self], weights: &[f32]) -> Self {
                let mut sum = <$t>::default();
                for (v, w) in values.iter().zip(weights.iter()) {
                    sum += **v * (*w as $t);
                }
                sum
            }
        }
    };
}

impl_lerp_float!(f32);
impl_lerp_float!(f64);

macro_rules! impl_lerp_vec {
    ($t:ty) => {
        impl Interpolatable for $t {
            #[inline]
            fn mix(a: &Self, b: &Self, t: f32) -> Self {
                a.lerp(*b, t as _)
            }

            #[inline]
            fn mix_n(values: &[&Self], weights: &[f32]) -> Self {
                if values.is_empty() { return Self::default(); }
                let mut sum = Self::default();
                for (v, w) in values.iter().zip(weights.iter()) {
                    sum += **v * (*w as f32);
                }
                sum
            }
        }
    };
}

impl_lerp_vec!(Vec2);
impl_lerp_vec!(Vec3);
impl_lerp_vec!(Vec4);

macro_rules! impl_lerp_dvec {
    ($t:ty) => {
        impl Interpolatable for $t {
            #[inline]
            fn mix(a: &Self, b: &Self, t: f32) -> Self {
                a.lerp(*b, t as f64)
            }

            #[inline]
            fn mix_n(values: &[&Self], weights: &[f32]) -> Self {
                if values.is_empty() { return Self::default(); }
                let mut sum = Self::default();
                for (v, w) in values.iter().zip(weights.iter()) {
                    sum += **v * (*w as f64);
                }
                sum
            }
        }
    };
}

impl_lerp_dvec!(DVec2);
impl_lerp_dvec!(DVec3);
impl_lerp_dvec!(DVec4);

// Quaternion Slerp!
impl Interpolatable for Quat {
    #[inline]
    fn mix(a: &Self, b: &Self, t: f32) -> Self {
        a.slerp(*b, t)
    }

    #[inline]
    fn mix_n(values: &[&Self], weights: &[f32]) -> Self {
        let mut sum = Vec4::ZERO;
        for (v, w) in values.iter().zip(weights.iter()) {
            let q_vec = Vec4::new(v.x, v.y, v.z, v.w);
            sum += q_vec * *w;
        }
        if sum.length_squared() > 1e-6 {
            let n = sum.normalize();
            Quat::from_vec4(n)
        } else {
            *values[0]
        }
    }
}

// Integers and Bools: Nearest Neighbor
macro_rules! impl_nearest {
    ($t:ty) => {
        impl Interpolatable for $t {
            #[inline]
            fn mix(a: &Self, b: &Self, t: f32) -> Self {
                if t < 0.5 { a.clone() } else { b.clone() }
            }

            #[inline]
            fn mix_n(values: &[&Self], weights: &[f32]) -> Self {
                // Find index with max weight
                let mut max_w = -1.0;
                let mut best_idx = 0;
                for (i, w) in weights.iter().enumerate() {
                    if *w > max_w {
                        max_w = *w;
                        best_idx = i;
                    }
                }
                values[best_idx].clone()
            }
        }
    };
}

impl_nearest!(i32);
impl_nearest!(bool);
impl_nearest!(String);
impl_nearest!(crate::libs::geometry::ids::PointId); // IDs shouldn't be lerped
