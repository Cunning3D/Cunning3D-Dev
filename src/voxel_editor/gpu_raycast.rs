//! Voxel raycast (CPU-first): avoid GPU readback stalls on interactive frames.
use bevy::prelude::*;
use crate::nodes::gpu::runtime::GpuRuntime;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VoxelAabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl VoxelAabb {
    #[inline]
    pub fn new(min: [f32; 3], max: [f32; 3]) -> Self { Self { min, max } }
}

pub struct GpuVoxelRaycaster;

#[derive(Resource, Default, Clone, Copy)]
pub struct VoxelHitResult {
    pub has_hit: bool,
    pub hit_t: f32,
    pub hit_idx: u32,
    pub hit_cell: IVec3,
    pub hit_normal: IVec3,
}

impl GpuVoxelRaycaster {
    #[inline]
    pub fn new(_rt: &GpuRuntime) -> Self { Self }

    pub fn raycast(&self, _rt: &GpuRuntime, voxels: &[VoxelAabb], origin: [f32; 3], dir: [f32; 3], _half: [f32; 3]) -> Option<(u32, f32)> {
        if voxels.is_empty() { return None; }
        let o = Vec3::from_array(origin);
        let d = Vec3::from_array(dir);
        let inv = Vec3::new(
            if d.x != 0.0 { 1.0 / d.x } else { f32::INFINITY },
            if d.y != 0.0 { 1.0 / d.y } else { f32::INFINITY },
            if d.z != 0.0 { 1.0 / d.z } else { f32::INFINITY },
        );
        let mut best_t = f32::INFINITY;
        let mut best_i: u32 = u32::MAX;
        for (i, a) in voxels.iter().enumerate() {
            let bmin = Vec3::from_array(a.min);
            let bmax = Vec3::from_array(a.max);
            let t0 = (bmin - o) * inv;
            let t1 = (bmax - o) * inv;
            let tmin = t0.min(t1);
            let tmax = t0.max(t1);
            let enter = tmin.x.max(tmin.y).max(tmin.z).max(0.0);
            let exit = tmax.x.min(tmax.y).min(tmax.z);
            if exit < 0.0 || enter > exit { continue; }
            if enter < best_t {
                best_t = enter;
                best_i = i as u32;
            }
        }
        if best_i == u32::MAX { None } else { Some((best_i, best_t)) }
    }
}

