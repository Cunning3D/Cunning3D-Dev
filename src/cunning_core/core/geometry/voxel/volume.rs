use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::chunk::VoxelChunk;
use super::dirty::{chunk_coord, DirtyChunks};
use super::ops::VoxelOp;
use super::types::{PaletteEntry, VoxelPalette, VoxelPi, CHUNK_SIZE};

pub type VoxelChunkKey = IVec3;

/// Discrete voxel volume (palette indices), chunked for editing + meshing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoxelVolume {
    pub voxel_size: f32,
    pub palette: VoxelPalette,
    pub chunks: HashMap<VoxelChunkKey, VoxelChunk>,
}

impl Default for VoxelVolume {
    fn default() -> Self {
        let mut palette = vec![PaletteEntry::default(); 256];
        palette[0] = PaletteEntry { color: [0, 0, 0, 0], ..Default::default() };
        Self { voxel_size: 0.1, palette, chunks: HashMap::new() }
    }
}

impl VoxelVolume {
    pub fn new(voxel_size: f32) -> Self {
        Self { voxel_size: voxel_size.max(0.001), ..Default::default() }
    }

    #[inline]
    pub fn get_pi(&self, p: IVec3) -> VoxelPi {
        let (ck, lp) = self.chunk_key_local(p);
        self.chunks.get(&ck).map(|c| c.get(lp)).unwrap_or(0)
    }

    #[inline]
    pub fn is_solid(&self, p: IVec3) -> bool {
        self.get_pi(p) != 0
    }

    #[inline]
    pub fn set_pi(&mut self, p: IVec3, pi: VoxelPi) {
        let (ck, lp) = self.chunk_key_local(p);
        if pi == 0 {
            if let Some(ch) = self.chunks.get_mut(&ck) {
                ch.set(lp, 0);
                if ch.solid_count == 0 {
                    self.chunks.remove(&ck);
                }
            }
        } else {
            let ch = self.chunks.entry(ck).or_insert_with(VoxelChunk::new);
            ch.set(lp, pi.max(1));
        }
    }

    #[inline]
    pub fn paint_pi(&mut self, p: IVec3, pi: VoxelPi) {
        if pi == 0 { return; }
        let (ck, lp) = self.chunk_key_local(p);
        let Some(ch) = self.chunks.get_mut(&ck) else { return; };
        if ch.get(lp) == 0 { return; }
        ch.set(lp, pi.max(1));
    }

    pub fn clear_all(&mut self) {
        self.chunks.clear();
    }

    /// Apply an edit op and mark dirty chunks (for meshing).
    pub fn apply_op(&mut self, op: &VoxelOp, dirty: &mut DirtyChunks) {
        if let Some(aabb) = op.dirty_aabb(self.voxel_size) {
            dirty.mark_aabb_with_neighbors(aabb);
        } else {
            // Full-volume change (ClearAll): mark everything dirty.
            // Keep it simple: clear chunks and let caller decide how to remesh.
        }

        match op {
            VoxelOp::Set { p, pi } => self.set_pi(*p, *pi),
            VoxelOp::Remove { p } => self.set_pi(*p, 0),
            VoxelOp::Paint { p, pi } => self.paint_pi(*p, *pi),
            VoxelOp::BoxAdd { min, max, pi } => {
                let mn = min.min(*max);
                let mx = min.max(*max);
                let pi = (*pi).max(1);
                for z in mn.z..=mx.z { for y in mn.y..=mx.y { for x in mn.x..=mx.x {
                    self.set_pi(IVec3::new(x, y, z), pi);
                }}}
            }
            VoxelOp::BoxRemove { min, max } => {
                let mn = min.min(*max);
                let mx = min.max(*max);
                for z in mn.z..=mx.z { for y in mn.y..=mx.y { for x in mn.x..=mx.x {
                    self.set_pi(IVec3::new(x, y, z), 0);
                }}}
            }
            VoxelOp::SphereAdd { center, radius_world, pi } => {
                let vs = self.voxel_size.max(0.001);
                let r = (*radius_world / vs).ceil() as i32;
                let c = (*center / vs).floor().as_ivec3();
                let r2 = (radius_world / vs) * (radius_world / vs);
                let pi = (*pi).max(1);
                for z in (c.z - r)..=(c.z + r) { for y in (c.y - r)..=(c.y + r) { for x in (c.x - r)..=(c.x + r) {
                    let d = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5) - (*center / vs);
                    if d.length_squared() <= r2 { self.set_pi(IVec3::new(x, y, z), pi); }
                }}}
            }
            VoxelOp::SphereRemove { center, radius_world } => {
                let vs = self.voxel_size.max(0.001);
                let r = (*radius_world / vs).ceil() as i32;
                let c = (*center / vs).floor().as_ivec3();
                let r2 = (radius_world / vs) * (radius_world / vs);
                for z in (c.z - r)..=(c.z + r) { for y in (c.y - r)..=(c.y + r) { for x in (c.x - r)..=(c.x + r) {
                    let d = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5) - (*center / vs);
                    if d.length_squared() <= r2 { self.set_pi(IVec3::new(x, y, z), 0); }
                }}}
            }
            VoxelOp::ClearAll => self.clear_all(),
        }
    }

    #[inline]
    fn chunk_key_local(&self, p: IVec3) -> (IVec3, IVec3) {
        let ck = chunk_coord(p);
        let lp = IVec3::new(p.x.rem_euclid(CHUNK_SIZE), p.y.rem_euclid(CHUNK_SIZE), p.z.rem_euclid(CHUNK_SIZE));
        (ck, lp)
    }
}

