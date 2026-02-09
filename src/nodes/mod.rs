pub mod attribute;
pub mod ai_texture;
pub mod basic;
pub mod basic_interaction;
pub mod cda;
pub mod curve;
pub mod flow;
pub mod gpu;
pub mod group;
pub mod io;
pub mod material;
pub mod modeling;
pub mod port_key;
pub mod runtime;
pub mod spline;
pub mod utility;
pub mod uv;
pub mod vdb;
pub mod voxel;
pub mod test_assets;
pub mod graph_model;

// Legacy definitions to support migration
pub mod structs;
pub use structs::*;

// Re-export parameter module as many files expect crate::nodes::parameter
pub use crate::cunning_core::traits::parameter;
pub use parameter::Parameter;

// Legacy alias for volume
pub use vdb as volume;
