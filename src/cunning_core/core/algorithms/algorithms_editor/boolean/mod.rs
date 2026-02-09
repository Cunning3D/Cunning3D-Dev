pub mod attributes;
pub mod kernel;
pub mod spatial;
pub mod spatial_truth;
pub mod graph;
pub mod ops;
pub mod manifold_backend;

// Re-export key types
pub use attributes::{AttributeInterpolationMode, AttributeConflictStrategy, BooleanConfig, Interpolatable};
pub use ops::{mesh_boolean, BooleanOperation};
