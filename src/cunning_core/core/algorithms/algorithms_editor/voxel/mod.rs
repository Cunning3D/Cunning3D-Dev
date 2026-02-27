//! Editor-only voxel editing algorithms (command list + incremental bake).
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use crate::mesh::Geometry;
use crate::sdf::{SdfChunk, SdfGrid, SdfHandle};

pub mod discrete;
pub use discrete::{DiscreteSdfGrid, DiscreteVoxelOp, DiscreteVoxelCmdList, DiscreteVoxel, PaletteEntry, DiscreteBakeState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VoxelOp {
    Sphere { center_world: Vec3, radius_world: f32, sdf: f32 },
    Box { min_world: Vec3, max_world: Vec3, sdf: f32 },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VoxelCmdList { pub ops: Vec<VoxelOp>, #[serde(default)] pub cursor: usize }

impl VoxelCmdList {
    #[inline] pub fn to_json(&self) -> String { serde_json::to_string(self).unwrap_or_else(|_| "{\"ops\":[],\"cursor\":0}".to_string()) }
    #[inline] pub fn from_json(s: &str) -> Self { serde_json::from_str(s).unwrap_or_default() }
    #[inline] pub fn push(&mut self, op: VoxelOp) { if self.cursor < self.ops.len() { self.ops.truncate(self.cursor); } self.ops.push(op); self.cursor = self.ops.len(); }
    #[inline] pub fn undo(&mut self) -> bool { if self.cursor == 0 { false } else { self.cursor -= 1; true } }
    #[inline] pub fn redo(&mut self) -> bool { if self.cursor >= self.ops.len() { false } else { self.cursor += 1; true } }
    #[inline] pub fn active_ops(&self) -> &[VoxelOp] { &self.ops[..self.cursor.min(self.ops.len())] }
}

#[derive(Debug, Clone, Default)]
pub struct VoxelBakeState { pub baked_cursor: usize }

#[inline]
pub fn blank_edit_volume(voxel_size: f32) -> SdfHandle { SdfHandle::new(SdfGrid::new(voxel_size.max(0.01), 0.0)) }

pub fn implicit_edit_volume(input: Option<&Geometry>, voxel_size: f32) -> SdfHandle {
    let Some(input) = input else { return blank_edit_volume(voxel_size); };

    // 1) Prefer existing volume.
    if let Some(v) = input.sdfs.first() {
        let g = v.grid.read().unwrap();
        let mut grid = SdfGrid::new(g.voxel_size.max(0.001), g.background_value);
        grid.chunks = g.chunks.clone();
        return SdfHandle::new(grid).with_transform(v.transform);
    }

    // 2) Point cloud -> sparse write into new grid.
    let vs = voxel_size.max(0.01);
    let mut grid = SdfGrid::new(vs, 0.0);
    if let Some(ps) = input.get_point_attribute("@P").and_then(|a| a.as_slice::<Vec3>()) {
        for p in ps {
            let v = *p / vs;
            grid.set_voxel(v.x.floor() as i32, v.y.floor() as i32, v.z.floor() as i32, -1.0);
        }
    }
    SdfHandle::new(grid)
}

pub fn bake_cmds_full(handle: &SdfHandle, cmds: &VoxelCmdList) {
    let mut grid = handle.grid.write().unwrap();
    let (vs, bg) = (grid.voxel_size, grid.background_value);
    *grid = SdfGrid::new(vs, bg);
    for op in cmds.active_ops().iter() { apply_op(&mut *grid, handle.transform, op); }
}

pub fn bake_cmds_incremental(handle: &SdfHandle, cmds: &VoxelCmdList, st: &mut VoxelBakeState) {
    let cur = cmds.cursor.min(cmds.ops.len());
    if cur < st.baked_cursor { st.baked_cursor = 0; }
    if st.baked_cursor == 0 { bake_cmds_full(handle, cmds); st.baked_cursor = cur; return; }
    if st.baked_cursor >= cur { return; }
    let mut grid = handle.grid.write().unwrap();
    for op in cmds.ops[st.baked_cursor..cur].iter() { apply_op(&mut *grid, handle.transform, op); }
    st.baked_cursor = cur;
}

fn apply_op(grid: &mut SdfGrid, xf: Mat4, op: &VoxelOp) {
    match op {
        VoxelOp::Sphere { center_world, radius_world, sdf } => {
            let vs = grid.voxel_size.max(0.001);
            let c_local = xf.inverse().transform_point3(*center_world) / vs;
            let r = (radius_world.max(0.0) / vs).ceil() as i32;
            let cx = c_local.x.floor() as i32;
            let cy = c_local.y.floor() as i32;
            let cz = c_local.z.floor() as i32;
            let r2 = (radius_world / vs) * (radius_world / vs);
            for z in (cz - r)..=(cz + r) {
                for y in (cy - r)..=(cy + r) {
                    for x in (cx - r)..=(cx + r) {
                        let dx = (x as f32 + 0.5) - c_local.x;
                        let dy = (y as f32 + 0.5) - c_local.y;
                        let dz = (z as f32 + 0.5) - c_local.z;
                        if dx * dx + dy * dy + dz * dz > r2 { continue; }
                        let cur = grid.get_voxel(x, y, z);
                        let next = if *sdf < 0.0 { cur.min(*sdf) } else { cur.max(*sdf) };
                        grid.set_voxel(x, y, z, next);
                    }
                }
            }
        }
        VoxelOp::Box { min_world, max_world, sdf } => {
            let vs = grid.voxel_size.max(0.001);
            let a = xf.inverse().transform_point3(*min_world) / vs;
            let b = xf.inverse().transform_point3(*max_world) / vs;
            let min = IVec3::new(a.x.floor() as i32, a.y.floor() as i32, a.z.floor() as i32).min(IVec3::new(b.x.floor() as i32, b.y.floor() as i32, b.z.floor() as i32));
            let max = IVec3::new(a.x.ceil() as i32, a.y.ceil() as i32, a.z.ceil() as i32).max(IVec3::new(b.x.ceil() as i32, b.y.ceil() as i32, b.z.ceil() as i32));
            for z in min.z..=max.z {
                for y in min.y..=max.y {
                    for x in min.x..=max.x {
                        let cur = grid.get_voxel(x, y, z);
                        let next = if *sdf < 0.0 { cur.min(*sdf) } else { cur.max(*sdf) };
                        grid.set_voxel(x, y, z, next);
                    }
                }
            }
        }
    }
}
