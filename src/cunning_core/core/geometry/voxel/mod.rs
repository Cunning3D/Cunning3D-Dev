//! Voxel (discrete palette-index) domain.
//!
//! This module is intentionally separate from the generic `Geometry` container:
//! - `Geometry` is the universal exchange/authoring format for procedural modeling.
//! - Voxel editing needs chunked storage, dirty tracking, and mesh caches.
//!
//! The intended usage is:
//! - Edit in voxel domain (`VoxelVolume`) with dirty chunks.
//! - Mesh dirty chunks into renderable meshes (CPU now, GPU later).
//! - Only materialize to `Geometry` when needed for non-voxel nodes/export.

pub mod types;
pub mod chunk;
pub mod volume;
pub mod dirty;
pub mod ops;
pub mod meshing;

pub use types::{PaletteEntry, Voxel, VoxelPalette, VoxelPi, CHUNK_SIZE, CHUNK_SIZE_USIZE};
pub use chunk::VoxelChunk;
pub use volume::{VoxelVolume, VoxelChunkKey};
pub use dirty::{DirtyChunks, DirtyAabb};
pub use ops::VoxelOp;
pub use meshing::{ChunkMesh, mesh_chunk_greedy, mesh_dirty_chunks_greedy, volume_to_geometry_greedy};

