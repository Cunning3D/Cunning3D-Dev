//! Model API - ported from Oxide-Lab
pub mod model;
pub mod optimization;
pub use model::ModelBackend;
pub use optimization::{OptimizationConfig, WeightFormat};
