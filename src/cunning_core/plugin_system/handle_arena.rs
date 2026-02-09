use crate::libs::geometry::sparse_set::{ArenaIndex, SparseSetArena};
use crate::mesh::Geometry;
use cunning_plugin_sdk::c_api::GeoHandle;
use std::sync::Arc;

#[derive(Clone)]
pub enum GeoSlot {
    Read(Arc<Geometry>),
    Write(Box<Geometry>),
}

#[derive(Default)]
pub struct GeoArena {
    slots: SparseSetArena<GeoSlot>,
}

impl GeoArena {
    #[inline]
    fn pack(id: ArenaIndex) -> GeoHandle {
        ((id.generation as u64) << 32) | (id.index as u64)
    }
    #[inline]
    fn unpack(h: GeoHandle) -> ArenaIndex {
        ArenaIndex::from_raw(h as u32, (h >> 32) as u32)
    }

    #[inline]
    pub fn insert_read(&mut self, g: Arc<Geometry>) -> GeoHandle {
        Self::pack(self.slots.insert(GeoSlot::Read(g)))
    }
    #[inline]
    pub fn insert_write(&mut self, g: Geometry) -> GeoHandle {
        Self::pack(self.slots.insert(GeoSlot::Write(Box::new(g))))
    }

    #[inline]
    pub fn get_read(&self, h: GeoHandle) -> Option<&Arc<Geometry>> {
        match self.slots.get(Self::unpack(h))? {
            GeoSlot::Read(g) => Some(g),
            _ => None,
        }
    }
    #[inline]
    pub fn get_write(&mut self, h: GeoHandle) -> Option<&mut Geometry> {
        match self.slots.get_mut(Self::unpack(h))? {
            GeoSlot::Write(g) => Some(g.as_mut()),
            _ => None,
        }
    }

    #[inline]
    pub fn clone_to_write(&mut self, h: GeoHandle) -> Option<GeoHandle> {
        Some(self.insert_write(self.get_read(h)?.as_ref().clone()))
    }
    #[inline]
    pub fn take_write(&mut self, h: GeoHandle) -> Option<Geometry> {
        match self.slots.remove(Self::unpack(h))? {
            GeoSlot::Write(g) => Some(*g),
            _ => None,
        }
    }

    #[inline]
    pub fn point_count_any(&self, h: GeoHandle) -> u32 {
        self.slots
            .get(Self::unpack(h))
            .map(|s| match s {
                GeoSlot::Read(g) => g.points().len() as u32,
                GeoSlot::Write(g) => g.points().len() as u32,
            })
            .unwrap_or(0)
    }
    #[inline]
    pub fn prim_count_any(&self, h: GeoHandle) -> u32 {
        self.slots
            .get(Self::unpack(h))
            .map(|s| match s {
                GeoSlot::Read(g) => g.primitives().len() as u32,
                GeoSlot::Write(g) => g.primitives().len() as u32,
            })
            .unwrap_or(0)
    }
}
