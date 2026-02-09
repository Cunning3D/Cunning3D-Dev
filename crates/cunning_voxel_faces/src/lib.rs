//! Shared GPU voxel face renderer (Editor + Player).
//!
//! This crate intentionally compiles the single source-of-truth implementation from
//! `Cunning3D_1.0/src/render/voxel_faces.rs` so we do not maintain two render paths.

#[path = "../../../src/render/voxel_faces.rs"]
mod voxel_faces;

pub use voxel_faces::*;

