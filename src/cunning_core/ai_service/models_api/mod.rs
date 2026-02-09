//! Model API - 移植自 Oxide-Lab
pub mod model;
pub mod optimization;
pub use model::ModelBackend;
pub use optimization::{OptimizationConfig, WeightFormat};
