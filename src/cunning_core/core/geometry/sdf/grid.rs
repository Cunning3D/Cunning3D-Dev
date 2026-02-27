use bevy::prelude::*;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::{Arc, RwLock};

pub const CHUNK_SIZE: i32 = 16;
pub const CHUNK_SIZE_F32: f32 = 16.0;

/// A thread-safe handle to an SdfGrid, friendly for the Node System (PartialEq, Serialize).
/// This replaces the old SdfHandle/SdfGrid naming to clearly distinguish it from discrete voxels.
#[derive(Clone, Debug)]
pub struct SdfHandle {
    pub grid: Arc<RwLock<SdfGrid>>,
    pub transform: Mat4,
}

impl SdfHandle {
    pub fn new(grid: SdfGrid) -> Self {
        Self {
            grid: Arc::new(RwLock::new(grid)),
            transform: Mat4::IDENTITY,
        }
    }

    pub fn with_transform(mut self, transform: Mat4) -> Self {
        self.transform = transform;
        self
    }

    pub fn world_to_voxel(&self, world_pos: Vec3) -> IVec3 {
        let grid = self.grid.read().unwrap();
        let local_pos = self.transform.inverse().transform_point3(world_pos);
        let v = local_pos / grid.voxel_size;
        IVec3::new(v.x.floor() as i32, v.y.floor() as i32, v.z.floor() as i32)
    }

    pub fn voxel_to_world(&self, voxel_idx: IVec3) -> Vec3 {
        let grid = self.grid.read().unwrap();
        let local_pos = Vec3::new(
            voxel_idx.x as f32 + 0.5,
            voxel_idx.y as f32 + 0.5,
            voxel_idx.z as f32 + 0.5,
        ) * grid.voxel_size;
        self.transform.transform_point3(local_pos)
    }
}

impl PartialEq for SdfHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.grid, &other.grid) && self.transform == other.transform
    }
}

impl Serialize for SdfHandle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_none()
    }
}

impl<'de> Deserialize<'de> for SdfHandle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let _ = Option::<()>::deserialize(deserializer)?;
        Ok(SdfHandle::new(SdfGrid::new(0.1, 0.0)))
    }
}

#[derive(Clone, Debug)]
pub struct SdfChunk {
    pub data: Vec<f32>,
}

impl SdfChunk {
    pub fn new(default_value: f32) -> Self {
        Self {
            data: vec![default_value; (CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE) as usize],
        }
    }

    pub fn get(&self, x: i32, y: i32, z: i32) -> f32 {
        self.data[Self::index(x, y, z)]
    }

    pub fn set(&mut self, x: i32, y: i32, z: i32, val: f32) {
        let idx = Self::index(x, y, z);
        self.data[idx] = val;
    }

    #[inline]
    fn index(x: i32, y: i32, z: i32) -> usize {
        (x + y * CHUNK_SIZE + z * CHUNK_SIZE * CHUNK_SIZE) as usize
    }
}

/// A Sparse Hash Grid representing a Signed Distance Field.
/// This will eventually be backed heavily by GPU buffers for JFA / Marching Cubes updates.
#[derive(Clone, Debug)]
pub struct SdfGrid {
    pub voxel_size: f32,
    pub chunks: FxHashMap<IVec3, SdfChunk>,
    pub background_value: f32, // The limit value or distance of completely empty space
}

impl SdfGrid {
    pub fn new(voxel_size: f32, background_value: f32) -> Self {
        Self {
            voxel_size,
            chunks: FxHashMap::default(),
            background_value,
        }
    }

    pub fn get_voxel(&self, x: i32, y: i32, z: i32) -> f32 {
        let (chunk_pos, local_pos) = self.get_chunk_coord(x, y, z);
        self.chunks
            .get(&chunk_pos)
            .map(|c| c.get(local_pos.x, local_pos.y, local_pos.z))
            .unwrap_or(self.background_value)
    }

    pub fn set_voxel(&mut self, x: i32, y: i32, z: i32, val: f32) {
        let (chunk_pos, local_pos) = self.get_chunk_coord(x, y, z);
        // Optimize: If value equals background and cluster doesn't exist, ignore allocation.
        if (val - self.background_value).abs() < f32::EPSILON {
            if let Some(chunk) = self.chunks.get_mut(&chunk_pos) {
                chunk.set(local_pos.x, local_pos.y, local_pos.z, val);
            }
            return;
        }
        self.chunks
            .entry(chunk_pos)
            .or_insert_with(|| SdfChunk::new(self.background_value))
            .set(local_pos.x, local_pos.y, local_pos.z, val);
    }

    fn get_chunk_coord(&self, x: i32, y: i32, z: i32) -> (IVec3, IVec3) {
        let (cx, cy, cz) = (
            x.div_euclid(CHUNK_SIZE),
            y.div_euclid(CHUNK_SIZE),
            z.div_euclid(CHUNK_SIZE),
        );
        let (lx, ly, lz) = (
            x.rem_euclid(CHUNK_SIZE),
            y.rem_euclid(CHUNK_SIZE),
            z.rem_euclid(CHUNK_SIZE),
        );
        (IVec3::new(cx, cy, cz), IVec3::new(lx, ly, lz))
    }

    pub fn get_active_voxels(&self) -> Vec<Vec3> {
        let mut points = Vec::new();
        for (chunk_pos, chunk) in &self.chunks {
            let chunk_origin = *chunk_pos * CHUNK_SIZE;
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    for x in 0..CHUNK_SIZE {
                        let val = chunk.get(x, y, z);
                        if (val - self.background_value).abs() > f32::EPSILON {
                            let (gx, gy, gz) =
                                (chunk_origin.x + x, chunk_origin.y + y, chunk_origin.z + z);
                            points.push(
                                Vec3::new(gx as f32 + 0.5, gy as f32 + 0.5, gz as f32 + 0.5)
                                    * self.voxel_size,
                            );
                        }
                    }
                }
            }
        }
        points
    }
}
