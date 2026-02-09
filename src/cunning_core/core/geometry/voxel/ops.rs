use bevy::prelude::*;

use super::dirty::{DirtyAabb};
use super::types::VoxelPi;

/// Editing operations in voxel domain.
#[derive(Debug, Clone)]
pub enum VoxelOp {
    Set { p: IVec3, pi: VoxelPi },
    Remove { p: IVec3 },
    Paint { p: IVec3, pi: VoxelPi }, // only if exists
    BoxAdd { min: IVec3, max: IVec3, pi: VoxelPi },
    BoxRemove { min: IVec3, max: IVec3 },
    SphereAdd { center: Vec3, radius_world: f32, pi: VoxelPi },
    SphereRemove { center: Vec3, radius_world: f32 },
    ClearAll,
}

impl VoxelOp {
    /// Conservative dirty bounds in voxel coordinates (inclusive).
    /// Returns None for full-volume changes.
    pub fn dirty_aabb(&self, voxel_size: f32) -> Option<DirtyAabb> {
        let vs = voxel_size.max(0.001);
        match self {
            Self::Set { p, .. } | Self::Remove { p } | Self::Paint { p, .. } => Some(DirtyAabb::new(*p, *p)),
            Self::BoxAdd { min, max, .. } | Self::BoxRemove { min, max } => Some(DirtyAabb::new(*min, *max)),
            Self::SphereAdd { center, radius_world, .. } | Self::SphereRemove { center, radius_world } => {
                let c = (*center / vs).floor().as_ivec3();
                let r = (*radius_world / vs).ceil() as i32;
                Some(DirtyAabb::new(c - IVec3::splat(r), c + IVec3::splat(r)))
            }
            Self::ClearAll => None,
        }
    }
}

