//! Voxel volume + undo stack (CPU): Voxy-style contiguous data + changes.
use bevy::prelude::*;

#[derive(Clone)]
pub struct VoxelVolume {
    pub size3d: IVec3,
    pub origin: IVec3,
    pub voxel_size: f32,
    pub data: Vec<u8>,
}

impl VoxelVolume {
    pub fn new(size3d: IVec3, origin: IVec3, voxel_size: f32) -> Self {
        let len = (size3d.x.max(0) as usize)
            * (size3d.y.max(0) as usize)
            * (size3d.z.max(0) as usize);
        Self { size3d, origin, voxel_size: voxel_size.max(0.001), data: vec![0; len] }
    }

    #[inline]
    pub fn idx(&self, p: IVec3) -> Option<usize> {
        let lp = p - self.origin;
        if lp.x < 0 || lp.y < 0 || lp.z < 0 { return None; }
        if lp.x >= self.size3d.x || lp.y >= self.size3d.y || lp.z >= self.size3d.z { return None; }
        let (sx, sy) = (self.size3d.x as usize, self.size3d.y as usize);
        Some((lp.z as usize) * sx * sy + (lp.y as usize) * sx + (lp.x as usize))
    }

    #[inline]
    pub fn get(&self, p: IVec3) -> u8 { self.idx(p).and_then(|i| self.data.get(i).copied()).unwrap_or(0) }

    #[inline]
    pub fn set(&mut self, p: IVec3, v: u8) { if let Some(i) = self.idx(p) { self.data[i] = v; } }
}

#[derive(Clone)]
pub struct VoxelChanges {
    pub min: IVec3,
    pub max: IVec3,
    pub coords: Vec<IVec3>,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

impl VoxelChanges {
    pub fn apply(&self, vol: &mut VoxelVolume) { for (p, v) in self.coords.iter().zip(self.after.iter()) { vol.set(*p, *v); } }
    pub fn undo(&self, vol: &mut VoxelVolume) { for (p, v) in self.coords.iter().zip(self.before.iter()) { vol.set(*p, *v); } }
}

#[derive(Resource, Default)]
pub struct VoxelUndoStack {
    pub cursor: usize,
    pub max_undo: usize,
    pub changes: Vec<VoxelChanges>,
}

impl VoxelUndoStack {
    pub fn push(&mut self, c: VoxelChanges) {
        if self.cursor < self.changes.len() { self.changes.truncate(self.cursor); }
        self.changes.push(c);
        self.cursor = self.changes.len();
        if self.max_undo > 0 && self.changes.len() > self.max_undo {
            let drop_n = self.changes.len() - self.max_undo;
            self.changes.drain(0..drop_n);
            self.cursor = self.cursor.saturating_sub(drop_n);
        }
    }
    pub fn undo(&mut self, vol: &mut VoxelVolume) -> bool {
        if self.cursor == 0 { return false; }
        self.cursor -= 1;
        self.changes[self.cursor].undo(vol);
        true
    }
    pub fn redo(&mut self, vol: &mut VoxelVolume) -> bool {
        if self.cursor >= self.changes.len() { return false; }
        self.changes[self.cursor].apply(vol);
        self.cursor += 1;
        true
    }
}

