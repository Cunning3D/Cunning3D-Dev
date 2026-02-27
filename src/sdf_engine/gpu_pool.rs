//! GPU-first storage for chunked SDF data (brick pool) + work-item batching.
//!
//! Design goals:
//! - Keep SDF chunks resident in a large GPU storage buffer (`chunk_pool`).
//! - Batch compute work per-frame with a compact `work_items` buffer.
//! - Avoid per-chunk pipeline rebuild / bindgroup churn: one pool, many dispatches.

use bevy::prelude::IVec3;
use bytemuck::{Pod, Zeroable};
use rustc_hash::FxHashMap;

use crate::cunning_core::core::geometry::sdf::{SdfChunk, SdfGrid, CHUNK_SIZE};
use crate::nodes::gpu::runtime::GpuRuntime;

pub const CHUNK_VOXELS: usize = (CHUNK_SIZE as usize) * (CHUNK_SIZE as usize) * (CHUNK_SIZE as usize);
pub const MISSING_SLOT: u32 = u32::MAX;

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct SdfWorkItem {
    /// Chunk key in chunk-space (not voxel-space): grid chunk coordinates.
    pub chunk_key: [i32; 4],
    /// Slot into `chunk_pool`.
    pub slot: u32,
    pub _pad0: [u32; 3],
    /// Neighbor slots in a fixed order (3x3x3, dx/dy/dz in [-1..1]).
    pub neighbor_slots: [u32; 27],
    pub _pad1: [u32; 1],
}

#[inline]
pub fn neighbor_index(dx: i32, dy: i32, dz: i32) -> usize {
    debug_assert!((-1..=1).contains(&dx));
    debug_assert!((-1..=1).contains(&dy));
    debug_assert!((-1..=1).contains(&dz));
    ((dz + 1) * 9 + (dy + 1) * 3 + (dx + 1)) as usize
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SdfGpuPoolStats {
    pub cap_chunks: u32,
    pub live_chunks: u32,
    pub evictions: u64,
    pub uploads: u64,
}

#[derive(Clone, Copy, Debug)]
struct SlotMeta {
    key: Option<IVec3>,
    last_used: u64,
    gen: u32,
}

impl Default for SlotMeta {
    fn default() -> Self {
        Self { key: None, last_used: 0, gen: 0 }
    }
}

/// A GPU pool that stores many `16^3` SDF chunks in one large storage buffer.
///
/// Slots are indexed by `u32` and stable until evicted.
pub struct SdfGpuPool {
    pub chunk_pool: wgpu::Buffer,
    pub work_items: wgpu::Buffer,

    cap_chunks: u32,
    work_items_cap: u32,

    next_slot: u32,
    slots: Vec<SlotMeta>,
    map: FxHashMap<IVec3, u32>,
    free: Vec<u32>,

    seq: u64,
    evictions: u64,
    uploads: u64,
}

impl SdfGpuPool {
    pub fn new(rt: &GpuRuntime, cap_chunks: u32, work_items_cap: u32) -> Self {
        let dev = rt.device();
        let cap_chunks = cap_chunks.max(1);
        let work_items_cap = work_items_cap.max(1);

        let chunk_pool = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_chunk_pool"),
            size: (cap_chunks as u64) * (CHUNK_VOXELS as u64) * 4u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let work_items = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_work_items"),
            size: (work_items_cap as u64) * (std::mem::size_of::<SdfWorkItem>() as u64),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            chunk_pool,
            work_items,
            cap_chunks,
            work_items_cap,
            next_slot: 0,
            slots: vec![SlotMeta::default(); cap_chunks as usize],
            map: FxHashMap::default(),
            free: Vec::new(),
            seq: 0,
            evictions: 0,
            uploads: 0,
        }
    }

    #[inline]
    pub fn stats(&self) -> SdfGpuPoolStats {
        SdfGpuPoolStats {
            cap_chunks: self.cap_chunks,
            live_chunks: self.map.len() as u32,
            evictions: self.evictions,
            uploads: self.uploads,
        }
    }

    /// Returns the chunk key currently stored in a slot (if any).
    #[inline]
    pub fn slot_key(&self, slot: u32) -> Option<IVec3> {
        self.slots.get(slot as usize).and_then(|m| m.key)
    }

    #[inline]
    fn touch(&mut self, slot: u32) {
        if let Some(m) = self.slots.get_mut(slot as usize) {
            m.last_used = self.seq;
        }
    }

    fn evict_one(&mut self) -> Option<u32> {
        let mut best_slot = None;
        let mut best_time = u64::MAX;
        for (i, meta) in self.slots.iter().enumerate() {
            if meta.key.is_none() { continue; }
            if meta.last_used < best_time {
                best_time = meta.last_used;
                best_slot = Some(i as u32);
            }
        }
        let slot = best_slot?;
        let key = self.slots[slot as usize].key.take();
        if let Some(k) = key {
            self.map.remove(&k);
        }
        self.slots[slot as usize].gen = self.slots[slot as usize].gen.wrapping_add(1);
        self.evictions += 1;
        Some(slot)
    }

    fn alloc_slot(&mut self, key: IVec3) -> u32 {
        if let Some(slot) = self.free.pop() {
            self.slots[slot as usize].key = Some(key);
            self.touch(slot);
            self.map.insert(key, slot);
            return slot;
        }
        if self.next_slot < self.cap_chunks {
            let slot = self.next_slot;
            self.next_slot += 1;
            self.slots[slot as usize].key = Some(key);
            self.touch(slot);
            self.map.insert(key, slot);
            return slot;
        }
        // Pool full: evict LRU.
        let slot = self.evict_one().unwrap_or(0);
        self.slots[slot as usize].key = Some(key);
        self.touch(slot);
        self.map.insert(key, slot);
        slot
    }

    /// Returns an existing slot for `chunk_key`, or allocates (possibly evicting) a new one.
    #[inline]
    pub fn get_or_alloc_slot(&mut self, chunk_key: IVec3) -> u32 {
        if let Some(&slot) = self.map.get(&chunk_key) {
            self.touch(slot);
            return slot;
        }
        self.alloc_slot(chunk_key)
    }

    #[inline]
    fn slot_byte_offset(slot: u32) -> u64 {
        (slot as u64) * (CHUNK_VOXELS as u64) * 4u64
    }

    /// Upload a single chunk into its slot. Caller guarantees slot is owned by `chunk_key`.
    #[inline]
    pub fn upload_chunk_to_slot(&mut self, rt: &GpuRuntime, slot: u32, chunk: &SdfChunk) {
        debug_assert_eq!(chunk.data.len(), CHUNK_VOXELS);
        let q = rt.queue();
        q.write_buffer(&self.chunk_pool, Self::slot_byte_offset(slot), bytemuck::cast_slice(&chunk.data));
        self.uploads += 1;
    }

    /// Ensure a set of chunks exist in the pool, returning their slots.
    /// Uploads chunk data for any newly-allocated slots.
    pub fn ensure_chunks<'a>(
        &mut self,
        rt: &GpuRuntime,
        grid: &'a SdfGrid,
        chunk_keys: impl IntoIterator<Item = IVec3>,
    ) -> Vec<(IVec3, u32)> {
        self.seq = self.seq.wrapping_add(1);
        let mut out = Vec::new();
        for ck in chunk_keys {
            let slot = self.get_or_alloc_slot(ck);
            if let Some(chunk) = grid.chunks.get(&ck) {
                // Upload every time for now (correctness). Higher-level code can avoid reuploading
                // by tracking dirty bits and calling upload selectively.
                self.upload_chunk_to_slot(rt, slot, chunk);
            } else {
                // Missing chunk: upload a constant background chunk lazily? For now leave stale data;
                // shader must treat missing neighbors as background via `MISSING_SLOT`.
            }
            out.push((ck, slot));
        }
        out
    }

    /// Build and upload a work-item buffer for a batch of chunk keys.
    ///
    /// Returns the number of items written.
    pub fn upload_work_items(
        &mut self,
        rt: &GpuRuntime,
        chunk_keys: &[IVec3],
    ) -> u32 {
        self.seq = self.seq.wrapping_add(1);
        let n = (chunk_keys.len() as u32).min(self.work_items_cap);
        if n == 0 { return 0; }

        let mut items = Vec::with_capacity(n as usize);
        for &ck in chunk_keys.iter().take(n as usize) {
            let slot = self.get_or_alloc_slot(ck);
            let mut wi = SdfWorkItem::default();
            wi.chunk_key = [ck.x, ck.y, ck.z, 0];
            wi.slot = slot;
            wi.neighbor_slots.fill(MISSING_SLOT);
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let nk = ck + IVec3::new(dx, dy, dz);
                        let ns = self.map.get(&nk).copied().unwrap_or(MISSING_SLOT);
                        wi.neighbor_slots[neighbor_index(dx, dy, dz)] = ns;
                    }
                }
            }
            items.push(wi);
        }

        let q = rt.queue();
        q.write_buffer(&self.work_items, 0, bytemuck::cast_slice(&items));
        n
    }
}
