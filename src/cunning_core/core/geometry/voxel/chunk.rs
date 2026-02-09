use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::types::{VoxelPi, CHUNK_SIZE, CHUNK_SIZE_USIZE};

/// Dense chunk storing palette indices (0 = empty).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoxelChunk {
    pub data: Vec<VoxelPi>,
    pub solid_count: u32,
}

impl VoxelChunk {
    #[inline]
    pub fn new() -> Self {
        let n = CHUNK_SIZE_USIZE * CHUNK_SIZE_USIZE * CHUNK_SIZE_USIZE;
        Self { data: vec![0; n], solid_count: 0 }
    }

    #[inline]
    pub fn index(lp: IVec3) -> usize {
        debug_assert!((0..CHUNK_SIZE).contains(&lp.x));
        debug_assert!((0..CHUNK_SIZE).contains(&lp.y));
        debug_assert!((0..CHUNK_SIZE).contains(&lp.z));
        (lp.z as usize) * CHUNK_SIZE_USIZE * CHUNK_SIZE_USIZE
            + (lp.y as usize) * CHUNK_SIZE_USIZE
            + (lp.x as usize)
    }

    #[inline]
    pub fn get(&self, lp: IVec3) -> VoxelPi {
        let idx = Self::index(lp);
        *self.data.get(idx).unwrap_or(&0)
    }

    #[inline]
    pub fn set(&mut self, lp: IVec3, pi: VoxelPi) {
        let idx = Self::index(lp);
        let old = self.data[idx];
        if old == 0 && pi != 0 {
            self.solid_count += 1;
        } else if old != 0 && pi == 0 {
            self.solid_count = self.solid_count.saturating_sub(1);
        }
        self.data[idx] = pi;
    }
}

