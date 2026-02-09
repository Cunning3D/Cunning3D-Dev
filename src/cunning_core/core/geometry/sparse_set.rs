use serde::{Deserialize, Serialize};
use std::slice::Iter;
use rayon::prelude::*;

/// A stable identifier combining an index into the slot array and a generation counter.
/// This guarantees safety against "ABA" problems where an index is reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArenaIndex {
    pub index: u32,      // Index into the 'sparse' (slots) array
    pub generation: u32, // Generation counter
}

impl ArenaIndex {
    pub fn from_raw(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }
    
    pub const INVALID: Self = Self { index: u32::MAX, generation: u32::MAX };
}

/// A slot in the sparse array.
/// Can either point to a valid index in the dense array, or be part of the free list.
#[derive(Debug, Clone, Serialize, Deserialize)]
enum Slot {
    Occupied {
        generation: u32,
        dense_index: u32, // Index into the `dense` array
    },
    Free {
        generation: u32,
        next_free: u32, // Index of next free slot, or u32::MAX
    },
}

/// A generational sparse-set arena.
/// 
/// Features:
/// - **O(1) Access**: Via dense array (internal) or ID (external).
/// - **O(1) Insert**: Reuses slots or appends.
/// - **O(1) Remove**: Swap-remove in dense array (changes physical order!).
/// - **Stable IDs**: IDs persist even when data moves in memory during compaction.
/// - **Dense Memory**: `dense` array is always packed, ideal for iteration and FFI.
/// - **Batch Optimization**: Support for O(N) batch removal to avoid swap-thrashing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparseSetArena<T> {
    /// The packed data storage. Iterate this for max performance.
    dense: Vec<T>,
    
    /// Maps Dense Index -> Slot Index.
    /// Used during swap-remove to update the slot of the swapped element.
    /// dense_to_slot[dense_idx] = slot_idx
    dense_to_slot: Vec<u32>,
    
    /// The sparse slots.
    /// slots[slot_idx] -> Dense Index or Free List
    slots: Vec<Slot>,
    
    /// Head of the free list (index into slots).
    free_head: u32,
}

impl<T> Default for SparseSetArena<T> {
    fn default() -> Self {
        Self {
            dense: Vec::new(),
            dense_to_slot: Vec::new(),
            slots: Vec::new(),
            free_head: u32::MAX,
        }
    }
}

impl<T> SparseSetArena<T> {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn reserve_additional(&mut self, additional: usize) {
        self.dense.reserve(additional);
        self.dense_to_slot.reserve(additional);
        self.slots.reserve(additional);
    }

    pub fn len(&self) -> usize {
        self.dense.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }
    
    pub fn capacity(&self) -> usize {
        self.dense.capacity()
    }

    /// Insert a value, returning a stable ID.
    pub fn insert(&mut self, value: T) -> ArenaIndex {
        let slot_idx;
        let gen;

        if self.free_head != u32::MAX {
            // Reuse free slot
            slot_idx = self.free_head;
            let slot = &mut self.slots[slot_idx as usize];
            
            match slot {
                Slot::Free { generation, next_free } => {
                    gen = *generation;
                    self.free_head = *next_free;
                },
                _ => unreachable!("Corrupted free list"),
            }
        } else {
            // New slot
            slot_idx = self.slots.len() as u32;
            gen = 0;
            self.slots.push(Slot::Free { generation: 0, next_free: u32::MAX }); // Placeholder
        }

        // Add to dense
        let dense_idx = self.dense.len() as u32;
        self.dense.push(value);
        self.dense_to_slot.push(slot_idx);

        // Update slot
        self.slots[slot_idx as usize] = Slot::Occupied {
            generation: gen,
            dense_index: dense_idx,
        };

        ArenaIndex { index: slot_idx, generation: gen }
    }

    /// Get a reference by ID. O(1).
    pub fn get(&self, id: ArenaIndex) -> Option<&T> {
        if let Some(Slot::Occupied { generation, dense_index }) = self.slots.get(id.index as usize) {
            if *generation == id.generation {
                return self.dense.get(*dense_index as usize);
            }
        }
        None
    }

    /// Get a mutable reference by ID. O(1).
    pub fn get_mut(&mut self, id: ArenaIndex) -> Option<&mut T> {
        if let Some(Slot::Occupied { generation, dense_index }) = self.slots.get(id.index as usize) {
            if *generation == id.generation {
                return self.dense.get_mut(*dense_index as usize);
            }
        }
        None
    }

    /// Convert a stable ID to a dense index (for looking up in parallel arrays).
    pub fn get_dense_index(&self, id: ArenaIndex) -> Option<usize> {
        if let Some(Slot::Occupied { generation, dense_index }) = self.slots.get(id.index as usize) {
            if *generation == id.generation {
                return Some(*dense_index as usize);
            }
        }
        None
    }
    
    /// Get the stable ID for a given dense index.
    pub fn get_id_from_dense(&self, dense_idx: usize) -> Option<ArenaIndex> {
        if dense_idx >= self.dense.len() { return None; }
        let slot_idx = self.dense_to_slot[dense_idx];
        match self.slots.get(slot_idx as usize) {
            Some(Slot::Occupied { generation, .. }) => {
                Some(ArenaIndex { index: slot_idx, generation: *generation })
            },
            _ => None,
        }
    }
    
    /// Remove an element by ID.
    /// Uses Swap-Remove: Moves the last element into the hole.
    /// Returns the removed value.
    pub fn remove(&mut self, id: ArenaIndex) -> Option<T> {
        let dense_idx = self.get_dense_index(id)?;
        let slot_idx = id.index as usize;
        
        // 1. Extract value (swap remove from dense)
        let removed_val = self.dense.swap_remove(dense_idx);
        self.dense_to_slot.swap_remove(dense_idx); // Remove mapping for the deleted element

        // 2. Update the slot of the moved element (if we didn't remove the last one)
        if dense_idx < self.dense.len() {
            // The element that was at 'last' is now at 'dense_idx'.
            // Its slot index is stored in dense_to_slot[dense_idx].
            let moved_slot_idx = self.dense_to_slot[dense_idx];
            
            let slot = &mut self.slots[moved_slot_idx as usize];
            if let Slot::Occupied { dense_index, .. } = slot {
                *dense_index = dense_idx as u32;
            } else {
                unreachable!("Swapped element slot must be occupied");
            }
        }

        // 3. Mark current slot as free
        let current_gen = id.generation;
        self.slots[slot_idx] = Slot::Free {
            generation: current_gen.wrapping_add(1), // Increment generation
            next_free: self.free_head,
        };
        self.free_head = slot_idx as u32;

        Some(removed_val)
    }

    /// Batch remove.
    /// Optimizes multiple deletions by using a single compaction pass instead of multiple swaps.
    /// This is O(N) where N is total elements (linear scan), but much faster than K * O(1) swaps if K is large.
    /// 
    /// Note: This is a simplified implementation. A full implementation would use a bitset or marker for O(1) checks.
    /// For now, we assume `ids` is reasonable size.
    pub fn remove_batch(&mut self, ids: &[ArenaIndex]) {
        if ids.is_empty() || self.dense.is_empty() { return; }
        let mut dens: Vec<usize> = ids.iter().filter_map(|&id| self.get_dense_index(id)).collect();
        if dens.is_empty() { return; }
        dens.sort_unstable();
        dens.dedup();
        let old_len = self.dense.len();
        if dens.len() >= old_len { self.clear(); return; }
        let mut rm = vec![false; old_len];
        for &i in &dens { if i < old_len { rm[i] = true; } }
        let mut old_dense = std::mem::take(&mut self.dense);
        let old_d2s = std::mem::take(&mut self.dense_to_slot);
        let mut new_dense: Vec<T> = Vec::with_capacity(old_len - dens.len());
        let mut new_d2s: Vec<u32> = Vec::with_capacity(old_len - dens.len());
        for (i, v) in old_dense.drain(..).enumerate() {
            let slot_idx = old_d2s[i] as usize;
            if rm[i] {
                let gen = match self.slots[slot_idx] {
                    Slot::Occupied { generation, .. } => generation,
                    Slot::Free { generation, .. } => generation,
                };
                self.slots[slot_idx] = Slot::Free { generation: gen.wrapping_add(1), next_free: self.free_head };
                self.free_head = slot_idx as u32;
                continue;
            }
            let new_i = new_dense.len() as u32;
            new_dense.push(v);
            new_d2s.push(slot_idx as u32);
            if let Slot::Occupied { dense_index, .. } = &mut self.slots[slot_idx] { *dense_index = new_i; }
        }
        self.dense = new_dense;
        self.dense_to_slot = new_d2s;
    }

    #[inline]
    pub fn clear(&mut self) {
        self.dense.clear();
        self.dense_to_slot.clear();
        self.slots.clear();
        self.free_head = u32::MAX;
    }
    
    // --- Direct Access for FFI / Iteration ---

    pub fn iter(&self) -> Iter<'_, T> {
        self.dense.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.dense.iter_mut()
    }
    
    /// Iterate with both the stable ArenaIndex and the value.
    /// Useful when you need to know "which ID" each element belongs to.
    pub fn iter_enumerated(&self) -> impl Iterator<Item = (ArenaIndex, &T)> {
        self.dense.iter().enumerate().map(move |(dense_idx, val)| {
            let slot_idx = self.dense_to_slot[dense_idx];
            let gen = match &self.slots[slot_idx as usize] {
                Slot::Occupied { generation, .. } => *generation,
                _ => unreachable!("Dense array points to non-occupied slot"),
            };
            (ArenaIndex { index: slot_idx, generation: gen }, val)
        })
    }
    
    pub fn values(&self) -> &[T] {
        &self.dense
    }

    /// Get raw pointer to dense data.
    pub fn as_ptr(&self) -> *const T {
        self.dense.as_ptr()
    }
}

impl<T: Sync + Send> SparseSetArena<T> {
    pub fn par_iter(&self) -> rayon::slice::Iter<'_, T> {
        self.dense.par_iter()
    }
}
