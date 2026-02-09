use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::types::CHUNK_SIZE;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DirtyAabb {
    pub min: IVec3,
    pub max: IVec3,
}

impl DirtyAabb {
    #[inline]
    pub fn new(min: IVec3, max: IVec3) -> Self {
        Self { min: min.min(max), max: min.max(max) }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DirtyChunks {
    pub chunks: HashSet<IVec3>,
}

impl DirtyChunks {
    #[inline]
    pub fn clear(&mut self) {
        self.chunks.clear();
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    #[inline]
    pub fn mark_chunk(&mut self, ck: IVec3) {
        self.chunks.insert(ck);
    }

    /// Mark chunks overlapped by an AABB in voxel coordinates.
    /// Includes 6-neighbor expansion so boundary faces update correctly.
    pub fn mark_aabb_with_neighbors(&mut self, aabb: DirtyAabb) {
        let mn = aabb.min - IVec3::ONE;
        let mx = aabb.max + IVec3::ONE;
        let c0 = chunk_coord(mn);
        let c1 = chunk_coord(mx);
        for z in c0.z..=c1.z {
            for y in c0.y..=c1.y {
                for x in c0.x..=c1.x {
                    self.chunks.insert(IVec3::new(x, y, z));
                }
            }
        }
        // Neighbor expansion
        let base: Vec<IVec3> = self.chunks.iter().copied().collect();
        for c in base {
            self.chunks.insert(c + IVec3::X);
            self.chunks.insert(c + IVec3::NEG_X);
            self.chunks.insert(c + IVec3::Y);
            self.chunks.insert(c + IVec3::NEG_Y);
            self.chunks.insert(c + IVec3::Z);
            self.chunks.insert(c + IVec3::NEG_Z);
        }
    }
}

#[inline]
pub fn chunk_coord(p: IVec3) -> IVec3 {
    IVec3::new(
        p.x.div_euclid(CHUNK_SIZE),
        p.y.div_euclid(CHUNK_SIZE),
        p.z.div_euclid(CHUNK_SIZE),
    )
}

