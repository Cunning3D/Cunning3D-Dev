//! Discrete voxel grid (Voxy-style): each voxel = palette index + optional color override.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const PALETTE_SIZE: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct VoxelCoord(pub IVec3);
impl VoxelCoord { #[inline] pub fn new(x: i32, y: i32, z: i32) -> Self { Self(IVec3::new(x, y, z)) } }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscreteVoxel {
    pub palette_index: u8,
    #[serde(skip_serializing_if = "Option::is_none", default)] pub color_override: Option<[u8; 4]>,
}

impl Default for DiscreteVoxel { fn default() -> Self { Self { palette_index: 1, color_override: None } } }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaletteEntry {
    pub color: [u8; 4],
    #[serde(default)] pub roughness: f32,
    #[serde(default)] pub metallic: f32,
    #[serde(default)] pub emissive: f32,
}

impl Default for PaletteEntry { fn default() -> Self { Self { color: [255, 255, 255, 255], roughness: 0.5, metallic: 0.0, emissive: 0.0 } } }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscreteVoxelGrid {
    pub voxels: HashMap<VoxelCoord, DiscreteVoxel>,
    pub palette: Vec<PaletteEntry>,
    #[serde(default)] pub voxel_size: f32,
}

impl Default for DiscreteVoxelGrid {
    fn default() -> Self {
        let mut palette = vec![PaletteEntry::default(); PALETTE_SIZE];
        palette[0] = PaletteEntry { color: [0, 0, 0, 0], ..default() }; // 0 = empty/transparent
        palette[1] = PaletteEntry { color: [200, 200, 200, 255], ..default() }; // 1 = default gray
        for i in 2..PALETTE_SIZE {
            let c = palette_color(i as u8);
            palette[i] = PaletteEntry { color: c, ..default() };
        }
        Self { voxels: HashMap::new(), palette, voxel_size: 1.0 }
    }
}

#[inline]
fn palette_color(i: u8) -> [u8; 4] {
    if i == 0 { return [0, 0, 0, 0]; }
    let h = (i as f32 * 0.618_033_988_75) % 1.0;
    let s = 0.65;
    let v = 0.90;
    let h6 = h * 6.0;
    let c = v * s;
    let x = c * (1.0 - ((h6 % 2.0) - 1.0).abs());
    let m = v - c;
    let (r1, g1, b1) = match h6 as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [((r1 + m) * 255.0) as u8, ((g1 + m) * 255.0) as u8, ((b1 + m) * 255.0) as u8, 255]
}

impl DiscreteVoxelGrid {
    pub fn new(voxel_size: f32) -> Self { Self { voxel_size: voxel_size.max(0.01), ..default() } }
    #[inline] pub fn get(&self, x: i32, y: i32, z: i32) -> Option<&DiscreteVoxel> { self.voxels.get(&VoxelCoord::new(x, y, z)) }
    #[inline] pub fn set(&mut self, x: i32, y: i32, z: i32, v: DiscreteVoxel) { self.voxels.insert(VoxelCoord::new(x, y, z), v); }
    #[inline] pub fn remove(&mut self, x: i32, y: i32, z: i32) -> Option<DiscreteVoxel> { self.voxels.remove(&VoxelCoord::new(x, y, z)) }
    #[inline] pub fn is_solid(&self, x: i32, y: i32, z: i32) -> bool { self.voxels.contains_key(&VoxelCoord::new(x, y, z)) }
    pub fn get_color(&self, x: i32, y: i32, z: i32) -> Option<[u8; 4]> {
        let v = self.get(x, y, z)?;
        Some(v.color_override.unwrap_or_else(|| self.palette.get(v.palette_index as usize).map(|p| p.color).unwrap_or([255, 255, 255, 255])))
    }
    pub fn bounds(&self) -> Option<(IVec3, IVec3)> {
        if self.voxels.is_empty() { return None; }
        let mut mn = IVec3::splat(i32::MAX);
        let mut mx = IVec3::splat(i32::MIN);
        for VoxelCoord(c) in self.voxels.keys() { mn = mn.min(*c); mx = mx.max(*c); }
        Some((mn, mx))
    }
    pub fn voxel_count(&self) -> usize { self.voxels.len() }
    pub fn to_json(&self) -> String { serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string()) }
    pub fn from_json(s: &str) -> Self { serde_json::from_str(s).unwrap_or_default() }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiscreteVoxelOp {
    SetVoxel { x: i32, y: i32, z: i32, palette_index: u8 },
    RemoveVoxel { x: i32, y: i32, z: i32 },
    SphereAdd { center: Vec3, radius: f32, palette_index: u8 },
    SphereRemove { center: Vec3, radius: f32 },
    BoxAdd { min: IVec3, max: IVec3, palette_index: u8 },
    BoxRemove { min: IVec3, max: IVec3 },
    Paint { x: i32, y: i32, z: i32, palette_index: u8 },
    MoveSelected { cells: Vec<IVec3>, delta: IVec3 },
    Extrude { cells: Vec<IVec3>, delta: IVec3, palette_index: u8 },
    CloneSelected { cells: Vec<IVec3>, delta: IVec3, overwrite: bool },
    ClearAll,
    TrimToOrigin,
    PerlinFill { min: IVec3, max: IVec3, scale: f32, threshold: f32, palette_index: u8, seed: u32 },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscreteVoxelCmdList { pub ops: Vec<DiscreteVoxelOp>, #[serde(default)] pub cursor: usize }

impl DiscreteVoxelCmdList {
    #[inline] pub fn push(&mut self, op: DiscreteVoxelOp) { if self.cursor < self.ops.len() { self.ops.truncate(self.cursor); } self.ops.push(op); self.cursor = self.ops.len(); }
    #[inline] pub fn undo(&mut self) -> bool { if self.cursor == 0 { false } else { self.cursor -= 1; true } }
    #[inline] pub fn redo(&mut self) -> bool { if self.cursor >= self.ops.len() { false } else { self.cursor += 1; true } }
    #[inline] pub fn active_ops(&self) -> &[DiscreteVoxelOp] { &self.ops[..self.cursor.min(self.ops.len())] }
    pub fn to_json(&self) -> String { serde_json::to_string(self).unwrap_or_else(|_| "{\"ops\":[],\"cursor\":0}".to_string()) }
    pub fn from_json(s: &str) -> Self { serde_json::from_str(s).unwrap_or_default() }
}

pub fn apply_op(grid: &mut DiscreteVoxelGrid, op: &DiscreteVoxelOp) {
    match op {
        DiscreteVoxelOp::SetVoxel { x, y, z, palette_index } => { grid.set(*x, *y, *z, DiscreteVoxel { palette_index: *palette_index, color_override: None }); }
        DiscreteVoxelOp::RemoveVoxel { x, y, z } => { grid.remove(*x, *y, *z); }
        DiscreteVoxelOp::Paint { x, y, z, palette_index } => { if let Some(v) = grid.voxels.get_mut(&VoxelCoord::new(*x, *y, *z)) { v.palette_index = *palette_index; } }
        DiscreteVoxelOp::SphereAdd { center, radius, palette_index } => {
            let vs = grid.voxel_size.max(0.01);
            let r = (*radius / vs).ceil() as i32;
            let c = (*center / vs).floor().as_ivec3();
            let r2 = (radius / vs) * (radius / vs);
            for z in (c.z - r)..=(c.z + r) { for y in (c.y - r)..=(c.y + r) { for x in (c.x - r)..=(c.x + r) {
                let d = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5) - (*center / vs);
                if d.length_squared() <= r2 { grid.set(x, y, z, DiscreteVoxel { palette_index: *palette_index, color_override: None }); }
            }}}
        }
        DiscreteVoxelOp::SphereRemove { center, radius } => {
            let vs = grid.voxel_size.max(0.01);
            let r = (*radius / vs).ceil() as i32;
            let c = (*center / vs).floor().as_ivec3();
            let r2 = (radius / vs) * (radius / vs);
            for z in (c.z - r)..=(c.z + r) { for y in (c.y - r)..=(c.y + r) { for x in (c.x - r)..=(c.x + r) {
                let d = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5) - (*center / vs);
                if d.length_squared() <= r2 { grid.remove(x, y, z); }
            }}}
        }
        DiscreteVoxelOp::BoxAdd { min, max, palette_index } => { for z in min.z..=max.z { for y in min.y..=max.y { for x in min.x..=max.x { grid.set(x, y, z, DiscreteVoxel { palette_index: *palette_index, color_override: None }); }}}}
        DiscreteVoxelOp::BoxRemove { min, max } => { for z in min.z..=max.z { for y in min.y..=max.y { for x in min.x..=max.x { grid.remove(x, y, z); }}}}
        DiscreteVoxelOp::MoveSelected { cells, delta } => {
            let mut tmp: Vec<(IVec3, DiscreteVoxel)> = cells.iter().filter_map(|c| grid.remove(c.x, c.y, c.z).map(|v| (*c + *delta, v))).collect();
            for (p, v) in tmp.drain(..) { grid.set(p.x, p.y, p.z, v); }
        }
        DiscreteVoxelOp::Extrude { cells, delta, palette_index } => {
            for c in cells.iter() { let np = *c + *delta; grid.set(np.x, np.y, np.z, DiscreteVoxel { palette_index: *palette_index, color_override: None }); }
        }
        DiscreteVoxelOp::CloneSelected { cells, delta, overwrite } => {
            let mut src: Vec<(IVec3, u8)> = Vec::with_capacity(cells.len());
            for c in cells.iter() { if let Some(v) = grid.get(c.x, c.y, c.z) { src.push((*c, v.palette_index)); } }
            for (c, pi) in src.into_iter() {
                let p = c + *delta;
                if !*overwrite && grid.is_solid(p.x, p.y, p.z) { continue; }
                grid.set(p.x, p.y, p.z, DiscreteVoxel { palette_index: pi, color_override: None });
            }
        }
        DiscreteVoxelOp::ClearAll => { grid.voxels.clear(); }
        DiscreteVoxelOp::TrimToOrigin => {
            let Some((mn, _mx)) = grid.bounds() else { return; };
            if mn == IVec3::ZERO { return; }
            let mut new_map: HashMap<VoxelCoord, DiscreteVoxel> = HashMap::with_capacity(grid.voxels.len());
            for (VoxelCoord(c), v) in grid.voxels.drain() { new_map.insert(VoxelCoord(c - mn), v); }
            grid.voxels = new_map;
        }
        DiscreteVoxelOp::PerlinFill { min, max, scale, threshold, palette_index, seed } => {
            let sc = scale.max(0.0001);
            let thr = threshold.clamp(-1.0, 1.0);
            for z in min.z..=max.z { for y in min.y..=max.y { for x in min.x..=max.x {
                let v = perlin3(x as f32 * sc, y as f32 * sc, z as f32 * sc, *seed);
                if v >= thr { grid.set(x, y, z, DiscreteVoxel { palette_index: *palette_index, color_override: None }); }
            }}}
        }
    }
}

#[inline]
fn hash_u32(mut x: u32) -> u32 { x ^= x >> 16; x = x.wrapping_mul(0x7feb_352d); x ^= x >> 15; x = x.wrapping_mul(0x846c_a68b); x ^= x >> 16; x }

#[inline]
fn rand01(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    let h = hash_u32((x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663) ^ (z as u32).wrapping_mul(83492791) ^ seed);
    (h as f32) * (1.0 / (u32::MAX as f32))
}

#[inline]
fn smooth(t: f32) -> f32 { t * t * (3.0 - 2.0 * t) }

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }

#[inline]
fn perlin3(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let xi = x.floor() as i32; let yi = y.floor() as i32; let zi = z.floor() as i32;
    let xf = smooth(x - xi as f32); let yf = smooth(y - yi as f32); let zf = smooth(z - zi as f32);
    let v000 = rand01(xi, yi, zi, seed); let v100 = rand01(xi + 1, yi, zi, seed);
    let v010 = rand01(xi, yi + 1, zi, seed); let v110 = rand01(xi + 1, yi + 1, zi, seed);
    let v001 = rand01(xi, yi, zi + 1, seed); let v101 = rand01(xi + 1, yi, zi + 1, seed);
    let v011 = rand01(xi, yi + 1, zi + 1, seed); let v111 = rand01(xi + 1, yi + 1, zi + 1, seed);
    let x00 = lerp(v000, v100, xf); let x10 = lerp(v010, v110, xf);
    let x01 = lerp(v001, v101, xf); let x11 = lerp(v011, v111, xf);
    let y0 = lerp(x00, x10, yf); let y1 = lerp(x01, x11, yf);
    lerp(y0, y1, zf) * 2.0 - 1.0
}

pub fn bake_cmds_full(grid: &mut DiscreteVoxelGrid, cmds: &DiscreteVoxelCmdList) {
    grid.voxels.clear();
    for op in cmds.active_ops() { apply_op(grid, op); }
}

#[derive(Debug, Clone, Default)]
pub struct DiscreteBakeState { pub baked_cursor: usize }

pub fn bake_cmds_incremental(grid: &mut DiscreteVoxelGrid, cmds: &DiscreteVoxelCmdList, st: &mut DiscreteBakeState) {
    let cur = cmds.cursor.min(cmds.ops.len());
    if cur < st.baked_cursor { st.baked_cursor = 0; grid.voxels.clear(); }
    if st.baked_cursor == 0 { bake_cmds_full(grid, cmds); st.baked_cursor = cur; return; }
    if st.baked_cursor >= cur { return; }
    for op in cmds.ops[st.baked_cursor..cur].iter() { apply_op(grid, op); }
    st.baked_cursor = cur;
}
