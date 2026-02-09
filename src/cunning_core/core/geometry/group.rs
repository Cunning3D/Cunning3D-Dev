use serde::{Deserialize, Serialize};

/// A highly efficient, dynamic bitset for managing element groups (Point/Primitive selections).
/// Internally uses `Vec<u64>` for storage, friendly to SIMD and cache.
/// 
/// # Invariants
/// - The logical length (`len`) matches the number of elements in the geometry owner.
/// - Bits beyond `len` in the last block are always 0 (padding).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ElementGroupMask {
    bits: Vec<u64>,
    len: usize,
}

impl ElementGroupMask {
    /// Create a new empty group mask with a specific capacity (in bits).
    pub fn new(len: usize) -> Self {
        let num_blocks = (len + 63) / 64;
        Self {
            bits: vec![0; num_blocks],
            len,
        }
    }

    /// Create a group mask from a boolean slice.
    pub fn from_bools(bools: &[bool]) -> Self {
        let mut mask = Self::new(bools.len());
        for (i, &b) in bools.iter().enumerate() {
            if b {
                mask.set_unchecked(i, true);
            }
        }
        mask
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn count_ones(&self) -> usize {
        let n = self.bits.len();
        if self.len == 0 || n == 0 { return 0; }
        let r = self.len & 63;
        let last_mask = if r == 0 { !0u64 } else { (1u64 << r) - 1 };
        let mut sum = 0usize;
        let mut i = 0usize;
        while i + 4 < n {
            sum += self.bits[i].count_ones() as usize;
            sum += self.bits[i + 1].count_ones() as usize;
            sum += self.bits[i + 2].count_ones() as usize;
            sum += self.bits[i + 3].count_ones() as usize;
            i += 4;
        }
        while i + 1 < n { sum += self.bits[i].count_ones() as usize; i += 1; }
        sum + (self.bits[n - 1] & last_mask).count_ones() as usize
    }

    /// Push a new bit at the end.
    pub fn push(&mut self, value: bool) {
        let new_len = self.len + 1;
        let needed_blocks = (new_len + 63) / 64;
        if needed_blocks > self.bits.len() {
            self.bits.push(0);
        }
        self.len = new_len;
        if value {
            self.set_unchecked(new_len - 1, true);
        }
    }

    #[inline]
    pub fn reserve_additional(&mut self, additional: usize) {
        if additional == 0 { return; }
        let new_len = self.len + additional;
        let need = (new_len + 63) / 64;
        if need > self.bits.len() { self.bits.reserve(need - self.bits.len()); }
    }

    /// Swap-remove: replace bit at `index` with the last bit, then shrink.
    /// This mirrors Vec::swap_remove for consistency with SparseSetArena.
    pub fn swap_remove(&mut self, index: usize) {
        if index >= self.len {
            return; // Out of bounds, no-op
        }
        if self.len == 0 {
            return;
        }
        
        let last_idx = self.len - 1;
        if index != last_idx {
            // Copy last bit to index
            let last_val = self.get_unchecked(last_idx);
            self.set_unchecked(index, last_val);
        }
        
        // Clear last bit and shrink
        self.set_unchecked(last_idx, false);
        self.len -= 1;
        
        // Shrink storage if possible
        let needed_blocks = (self.len + 63) / 64;
        self.bits.truncate(needed_blocks.max(1)); // Keep at least 1 block or 0 if empty
        if self.len == 0 {
            self.bits.clear();
        }
    }

    /// Resize the bitset. New bits are initialized to `value`.
    pub fn resize(&mut self, new_len: usize, value: bool) {
        if new_len == self.len {
            return;
        }

        let old_len = self.len;
        let new_num_blocks = (new_len + 63) / 64;
        
        // Resize the storage
        self.bits.resize(new_num_blocks, 0);
        self.len = new_len;

        if value && new_len > old_len {
            // Fill new bits with 1
            for i in old_len..new_len {
                self.set_unchecked(i, true);
            }
        }
        
        // Ensure padding bits are 0
             self.clear_trailing_bits();
    }

    #[inline]
    pub fn get(&self, index: usize) -> bool {
        if index >= self.len {
            false
        } else {
        self.get_unchecked(index)
    }
    }

    #[inline]
    pub fn get_unchecked(&self, index: usize) -> bool {
        let block = index / 64;
        let bit = index % 64;
            (self.bits[block] & (1 << bit)) != 0
    }

    #[inline]
    pub fn set(&mut self, index: usize, value: bool) {
        if index < self.len {
            self.set_unchecked(index, value);
        }
    }

    #[inline]
    fn set_unchecked(&mut self, index: usize, value: bool) {
        let block = index / 64;
        let bit = index % 64;
        if value {
            self.bits[block] |= 1 << bit;
        } else {
            self.bits[block] &= !(1 << bit);
        }
    }

    /// Clear all bits beyond `len` in the last block to ensure invariants.
    fn clear_trailing_bits(&mut self) {
        let active_bits_last_block = self.len % 64;
        if active_bits_last_block != 0 {
            let last_block_idx = self.len / 64;
            let mask = (1 << active_bits_last_block) - 1;
            if last_block_idx < self.bits.len() {
                self.bits[last_block_idx] &= mask;
        }
    }
    }

    pub fn iter_ones(&self) -> impl Iterator<Item = usize> + '_ {
        self.bits.iter().enumerate().flat_map(|(b_idx, &block)| {
            let mut block = block;
            let mut offset = 0;
            std::iter::from_fn(move || {
                if block == 0 {
                    return None;
                }
                let trailing = block.trailing_zeros();
                block >>= trailing;
                block >>= 1; // Shift off the bit we just found
                let bit_idx = b_idx * 64 + offset + trailing as usize;
                offset += trailing as usize + 1;
                Some(bit_idx)
            })
        }).take_while(move |&idx| idx < self.len)
}

    #[inline]
    pub fn ones_vec(&self) -> Vec<usize> {
        let mut v = Vec::with_capacity(self.count_ones());
        v.extend(self.iter_ones());
        #[cfg(debug_assertions)]
        {
            self.debug_check();
            debug_assert_eq!(v.len(), self.count_ones());
            debug_assert!(v.windows(2).all(|w| w[0] < w[1]));
            debug_assert!(v.last().copied().map_or(true, |i| i < self.len));
        }
        v
    }

    #[cfg(debug_assertions)]
    #[inline]
    pub fn debug_check(&self) {
        let n = self.bits.len();
        if self.len == 0 { debug_assert_eq!(n, 0); return; }
        debug_assert!(n == (self.len + 63) / 64);
        let r = self.len & 63;
        if r != 0 { debug_assert_eq!(self.bits[n - 1] & (!((1u64 << r) - 1)), 0); }
        debug_assert_eq!(self.count_ones(), self.iter_ones().count());
    }

    // --- Boolean Operations ---

    /// Union (OR): self |= other
    pub fn union_with(&mut self, other: &Self) {
        assert_eq!(self.len, other.len, "Group lengths must match for boolean ops");
        let a = &mut self.bits;
        let b = &other.bits;
        let n = a.len().min(b.len());
        let mut i = 0usize;
        while i + 4 <= n {
            a[i] |= b[i];
            a[i + 1] |= b[i + 1];
            a[i + 2] |= b[i + 2];
            a[i + 3] |= b[i + 3];
            i += 4;
        }
        while i < n { a[i] |= b[i]; i += 1; }
    }

    /// Intersect (AND): self &= other
    pub fn intersect_with(&mut self, other: &Self) {
        assert_eq!(self.len, other.len, "Group lengths must match for boolean ops");
        let a = &mut self.bits;
        let b = &other.bits;
        let n = a.len().min(b.len());
        let mut i = 0usize;
        while i + 4 <= n {
            a[i] &= b[i];
            a[i + 1] &= b[i + 1];
            a[i + 2] &= b[i + 2];
            a[i + 3] &= b[i + 3];
            i += 4;
        }
        while i < n { a[i] &= b[i]; i += 1; }
    }

    /// Difference (AND NOT): self &= !other
    pub fn difference_with(&mut self, other: &Self) {
        assert_eq!(self.len, other.len, "Group lengths must match for boolean ops");
        let a = &mut self.bits;
        let b = &other.bits;
        let n = a.len().min(b.len());
        let mut i = 0usize;
        while i + 4 <= n {
            a[i] &= !b[i];
            a[i + 1] &= !b[i + 1];
            a[i + 2] &= !b[i + 2];
            a[i + 3] &= !b[i + 3];
            i += 4;
        }
        while i < n { a[i] &= !b[i]; i += 1; }
    }

    /// Symmetric Difference (XOR): self ^= other
    pub fn symmetric_difference_with(&mut self, other: &Self) {
        assert_eq!(self.len, other.len, "Group lengths must match for boolean ops");
        let a = &mut self.bits;
        let b = &other.bits;
        let n = a.len().min(b.len());
        let mut i = 0usize;
        while i + 4 <= n {
            a[i] ^= b[i];
            a[i + 1] ^= b[i + 1];
            a[i + 2] ^= b[i + 2];
            a[i + 3] ^= b[i + 3];
            i += 4;
        }
        while i < n { a[i] ^= b[i]; i += 1; }
    }

    /// Invert all bits
    pub fn invert(&mut self) {
        let a = &mut self.bits;
        let n = a.len();
        let mut i = 0usize;
        while i + 4 <= n {
            a[i] = !a[i];
            a[i + 1] = !a[i + 1];
            a[i + 2] = !a[i + 2];
            a[i + 3] = !a[i + 3];
            i += 4;
        }
        while i < n { a[i] = !a[i]; i += 1; }
        self.clear_trailing_bits(); // Clear padding bits that became 1
    }
}

/// A group that maintains the order of elements.
/// Useful for modeling operations where selection order matters (e.g. Loft, Curve from Selection).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrderedElementGroup {
    /// List of indices in order. Note: These are Dense Indices usually.
    pub indices: Vec<u32>,
}

impl OrderedElementGroup {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn from_indices(indices: Vec<u32>) -> Self {
        Self { indices }
    }
    
    /// Convert to unordered mask (loses order).
    pub fn to_mask(&self, total_len: usize) -> ElementGroupMask {
        let mut mask = ElementGroupMask::new(total_len);
        for &idx in &self.indices {
            mask.set(idx as usize, true);
        }
        mask
    }
    
    /// Create from mask (order is implicitly index-order).
    pub fn from_mask(mask: &ElementGroupMask) -> Self {
        Self {
            indices: mask.iter_ones().map(|i| i as u32).collect(),
        }
    }
    
    pub fn push(&mut self, idx: u32) {
        self.indices.push(idx);
    }
    
    pub fn len(&self) -> usize {
        self.indices.len()
    }
    
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupPromoteMode {
    Any, // Or
    All, // And
}
