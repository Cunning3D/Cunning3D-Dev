use std::collections::HashMap;
use bevy::math::{Vec2, Vec3, Vec4, DVec2, DVec3, DVec4};
use crate::libs::geometry::mesh::{Attribute, Geometry};
use crate::libs::algorithms::algorithms_dcc::PagedBuffer;
use std::sync::Arc;

/// Strategy for interpolating attributes when creating new geometric elements
/// (e.g., points on an edge, points inside a triangle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeInterpolationMode {
    /// Linear interpolation (lerp).
    /// Used for continuous data like Position, UV, Color, Normal (with renormalization).
    Linear,
    
    /// Nearest neighbor.
    /// Used for discrete data like Integer IDs, Material Indices.
    Nearest,
    
    /// Always set to default/zero.
    ConstantZero,
    
    /// Don't interpolate (leave uninitialized or use specific fallback).
    None,
}

/// Strategy for handling attribute conflicts when merging geometries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeConflictStrategy {
    /// Keep value from the first geometry (A).
    KeepA,
    /// Keep value from the second geometry (B).
    KeepB,
    /// Blend values (if applicable).
    Blend,
}

/// Global configuration for the boolean operation's attribute behavior.
#[derive(Debug, Clone)]
pub struct BooleanConfig {
    /// Geometric tolerance for intersections (e.g., 1e-6).
    pub tolerance: f64,
    
    /// Default interpolation mode for float/vector attributes.
    pub default_float_interp: AttributeInterpolationMode,
    
    /// Default interpolation mode for integer/string attributes.
    pub default_discrete_interp: AttributeInterpolationMode,
    
    /// Per-attribute overrides (e.g., force "uv" to Linear, "id" to Nearest).
    pub attribute_overrides: HashMap<String, AttributeInterpolationMode>,
    
    /// Whether to transfer groups to the new geometry.
    pub transfer_groups: bool,
}

impl Default for BooleanConfig {
    fn default() -> Self {
        Self {
            tolerance: 1e-6,
            default_float_interp: AttributeInterpolationMode::Linear,
            default_discrete_interp: AttributeInterpolationMode::Nearest,
            attribute_overrides: HashMap::new(),
            transfer_groups: true,
        }
    }
}

/// Trait for types that can be interpolated.
pub trait Interpolatable: Clone + Sized {
    /// Linear interpolation: self * (1.0 - t) + other * t
    fn lerp(&self, other: &Self, t: f32) -> Self;
    
    /// Barycentric interpolation: self * u + b * v + c * w
    /// (Note: w is usually 1.0 - u - v, but passed explicitly for symmetry)
    fn barycentric(&self, b: &Self, c: &Self, u: f32, v: f32, w: f32) -> Self;
}

// --- Implementations for Standard Types ---

impl Interpolatable for f32 {
    #[inline]
    fn lerp(&self, other: &Self, t: f32) -> Self {
        self * (1.0 - t) + other * t
    }

    #[inline]
    fn barycentric(&self, b: &Self, c: &Self, u: f32, v: f32, w: f32) -> Self {
        self * u + b * v + c * w
    }
}

impl Interpolatable for f64 {
    #[inline]
    fn lerp(&self, other: &Self, t: f32) -> Self {
        self * (1.0 - t as f64) + other * (t as f64)
    }

    #[inline]
    fn barycentric(&self, b: &Self, c: &Self, u: f32, v: f32, w: f32) -> Self {
        self * (u as f64) + b * (v as f64) + c * (w as f64)
    }
}

impl Interpolatable for Vec2 {
    #[inline]
    fn lerp(&self, other: &Self, t: f32) -> Self {
        Vec2::lerp(*self, *other, t)
    }

    #[inline]
    fn barycentric(&self, b: &Self, c: &Self, u: f32, v: f32, w: f32) -> Self {
        *self * u + *b * v + *c * w
    }
}

impl Interpolatable for Vec3 {
    #[inline]
    fn lerp(&self, other: &Self, t: f32) -> Self {
        Vec3::lerp(*self, *other, t)
    }

    #[inline]
    fn barycentric(&self, b: &Self, c: &Self, u: f32, v: f32, w: f32) -> Self {
        *self * u + *b * v + *c * w
    }
}

impl Interpolatable for Vec4 {
    #[inline]
    fn lerp(&self, other: &Self, t: f32) -> Self {
        Vec4::lerp(*self, *other, t)
    }

    #[inline]
    fn barycentric(&self, b: &Self, c: &Self, u: f32, v: f32, w: f32) -> Self {
        *self * u + *b * v + *c * w
    }
}

// Double precision vectors
impl Interpolatable for DVec2 {
    #[inline]
    fn lerp(&self, other: &Self, t: f32) -> Self {
        DVec2::lerp(*self, *other, t as f64)
    }

    #[inline]
    fn barycentric(&self, b: &Self, c: &Self, u: f32, v: f32, w: f32) -> Self {
        *self * (u as f64) + *b * (v as f64) + *c * (w as f64)
    }
}

impl Interpolatable for DVec3 {
    #[inline]
    fn lerp(&self, other: &Self, t: f32) -> Self {
        DVec3::lerp(*self, *other, t as f64)
    }

    #[inline]
    fn barycentric(&self, b: &Self, c: &Self, u: f32, v: f32, w: f32) -> Self {
        *self * (u as f64) + *b * (v as f64) + *c * (w as f64)
    }
}

impl Interpolatable for DVec4 {
    #[inline]
    fn lerp(&self, other: &Self, t: f32) -> Self {
        DVec4::lerp(*self, *other, t as f64)
    }

    #[inline]
    fn barycentric(&self, b: &Self, c: &Self, u: f32, v: f32, w: f32) -> Self {
        *self * (u as f64) + *b * (v as f64) + *c * (w as f64)
    }
}

// Discrete types (Nearest Neighbor)
impl Interpolatable for i32 {
    #[inline]
    fn lerp(&self, other: &Self, t: f32) -> Self {
        if t < 0.5 { *self } else { *other }
    }
    #[inline]
    fn barycentric(&self, b: &Self, c: &Self, u: f32, v: f32, w: f32) -> Self {
        // Simple max weight wins
        if u >= v && u >= w { *self }
        else if v >= u && v >= w { *b }
        else { *c }
    }
}

impl Interpolatable for bool {
    #[inline]
    fn lerp(&self, other: &Self, t: f32) -> Self {
        if t < 0.5 { *self } else { *other }
    }
    #[inline]
    fn barycentric(&self, b: &Self, c: &Self, u: f32, v: f32, w: f32) -> Self {
        if u >= v && u >= w { *self }
        else if v >= u && v >= w { *b }
        else { *c }
    }
}

impl Interpolatable for String {
    #[inline]
    fn lerp(&self, other: &Self, t: f32) -> Self {
        if t < 0.5 { self.clone() } else { other.clone() }
    }
    #[inline]
    fn barycentric(&self, b: &Self, c: &Self, u: f32, v: f32, w: f32) -> Self {
         if u >= v && u >= w { self.clone() }
        else if v >= u && v >= w { b.clone() }
        else { c.clone() }
    }
}

// --- Batch Operations Helper ---

impl AttributeInterpolationMode {
    pub fn interpolate_paged<T: Interpolatable + Clone + Send + Sync + 'static>(
        &self,
        buffer: &mut PagedBuffer<T>,
        src_buffer: &PagedBuffer<T>,
        idx_a: usize,
        idx_b: usize,
        t: f32
    ) {
        match self {
            AttributeInterpolationMode::Linear | AttributeInterpolationMode::Nearest => {
                 if let (Some(val_a), Some(val_b)) = (src_buffer.get(idx_a), src_buffer.get(idx_b)) {
                     let result = val_a.lerp(&val_b, t);
                     buffer.push(result);
                 } else {
                     if let Some(val) = src_buffer.get(idx_a) { buffer.push(val); }
                     else if let Some(val) = src_buffer.get(idx_b) { buffer.push(val); }
                 }
            },
            AttributeInterpolationMode::ConstantZero => {
                 // Requires T::default(), but T is generic. 
                 // In practice, we will match on Attribute Enum and know the type to create Zero.
                 // This generic helper might be too generic for Zero case without Default trait.
                 // We'll handle Zero at the caller match level.
            },
            AttributeInterpolationMode::None => {
                // Do nothing? Or push uninit? 
                // PagedBuffer expects values. 
            }
        }
    }
}
