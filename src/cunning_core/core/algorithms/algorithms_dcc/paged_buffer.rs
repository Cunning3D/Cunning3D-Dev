//! PagedBuffer: page-level COW buffer, designed to outperform Houdini's UT_PageArray
//! Features: page-table COW + zero-value optimization + small-value inlining + constant-page compression + SIMD alignment

use std::sync::Arc;
use std::mem::{size_of, MaybeUninit};
use std::marker::PhantomData;
use rayon::prelude::*;
use serde::{Serialize, Deserialize, Serializer, Deserializer};
use std::any::Any;
use crate::libs::geometry::mesh::AttributeStorage;

/// Page size: 1024 elements, SIMD-friendly (multiple of 64-byte alignment)
pub const PAGE_SIZE: usize = 1024;
/// Inline threshold: types ≤ 8 bytes can be stored directly in the enum
const INLINE_THRESHOLD: usize = 8;

/// Page pointer: supports four modes: zero/inline/constant/shared
#[derive(Debug, Clone)]
pub enum PagePtr<T: Clone + Send + Sync + 'static> {
    /// All-zero page (virtual page; no allocation)
    Zero,
    /// Inline value: constant page for types ≤ 8 bytes; stored directly with no heap allocation
    Inline(InlineValue<T>),
    /// Constant page: all elements identical (for types > 8 bytes)
    Constant(Arc<T>),
    /// Allocated page: real data, COW-shared
    Allocated(Arc<AlignedPage<T>>),
}

/// Inline value: up to 8 bytes, stored directly on the stack
#[derive(Clone, Copy)]
#[repr(C)]
pub struct InlineValue<T> {
    data: [u8; INLINE_THRESHOLD],
    _marker: PhantomData<T>,
}

impl<T: Clone + Send + Sync + 'static> std::fmt::Debug for InlineValue<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InlineValue").finish()
    }
}

impl<T: Clone + Send + Sync + 'static> InlineValue<T> {
    /// Create inline storage from a value (only when size_of::<T>() <= 8)
    #[inline]
    pub fn new(value: &T) -> Self {
        debug_assert!(size_of::<T>() <= INLINE_THRESHOLD);
        let mut data = [0u8; INLINE_THRESHOLD];
        unsafe {
            std::ptr::copy_nonoverlapping(value as *const T as *const u8, data.as_mut_ptr(), size_of::<T>());
        }
        Self { data, _marker: PhantomData }
    }

    /// Read the inline value
    #[inline]
    pub fn get(&self) -> T {
        debug_assert!(size_of::<T>() <= INLINE_THRESHOLD);
        unsafe {
            let mut val = MaybeUninit::<T>::uninit();
            std::ptr::copy_nonoverlapping(self.data.as_ptr(), val.as_mut_ptr() as *mut u8, size_of::<T>());
            val.assume_init()
        }
    }
}

/// Aligned page: 64-byte aligned, SIMD-friendly
#[derive(Debug, Clone)]
#[repr(C, align(64))]
pub struct AlignedPage<T> {
    pub data: Vec<T>,
}

impl<T: Clone> AlignedPage<T> {
    #[inline]
    pub fn new() -> Self { Self { data: Vec::with_capacity(PAGE_SIZE) } }
    
    #[inline]
    pub fn from_vec(v: Vec<T>) -> Self { Self { data: v } }
    
    #[inline]
    pub fn from_fill(val: T, count: usize) -> Self {
        Self { data: vec![val; count] }
    }
}

/// Page table: shareable (COW)
#[derive(Debug, Clone)]
pub struct PageTable<T: Clone + Send + Sync + 'static> {
    pub pages: Vec<PagePtr<T>>,
}

impl<T: Clone + Send + Sync + 'static> PageTable<T> {
    #[inline]
    pub fn new() -> Self { Self { pages: Vec::new() } }
}

/// PagedBuffer: page-level COW buffer
#[derive(Debug, Clone)]
pub struct PagedBuffer<T: Clone + Send + Sync + 'static> {
    /// Page table (Arc-shared; supports page-table-level COW)
    table: Arc<PageTable<T>>,
    /// Logical length
    len: usize,
}

impl<T: Clone + Send + Sync + Default + 'static> Default for PagedBuffer<T> {
    fn default() -> Self { Self::new() }
}

impl<T: Clone + Send + Sync + 'static> PagedBuffer<T> {
    /// Create an empty buffer
    #[inline]
    pub fn new() -> Self {
        Self { table: Arc::new(PageTable::new()), len: 0 }
    }

    #[inline]
    pub fn len(&self) -> usize { self.len }

    #[inline]
    pub fn is_empty(&self) -> bool { self.len == 0 }

    /// Check whether inline optimization can be used
    #[inline]
    const fn can_inline() -> bool { size_of::<T>() <= INLINE_THRESHOLD }

    /// Get a mutable page table (COW: clone if shared)
    #[inline]
    fn table_mut(&mut self) -> &mut PageTable<T> {
        Arc::make_mut(&mut self.table)
    }

    /// Convert a value into the optimal page-pointer representation
    #[inline]
    fn optimal_page_ptr(val: &T) -> PagePtr<T> {
        if Self::can_inline() {
            PagePtr::Inline(InlineValue::new(val))
        } else {
            PagePtr::Constant(Arc::new(val.clone()))
        }
    }

    /// Push an element
    pub fn push(&mut self, value: T) {
        let page_idx = self.len / PAGE_SIZE;
        let offset = self.len % PAGE_SIZE;
        
        let table = self.table_mut();
        
        if offset == 0 {
            // Need a new page
            let mut page = AlignedPage::new();
            page.data.push(value);
            table.pages.push(PagePtr::Allocated(Arc::new(page)));
        } else {
            // Write into an existing page
            Self::harden_page_for_write(&mut table.pages[page_idx], PAGE_SIZE);
            if let PagePtr::Allocated(arc) = &mut table.pages[page_idx] {
                Arc::make_mut(arc).data.push(value);
            }
        }
        self.len += 1;
    }

    /// Pop the last element
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 { return None; }
        
        let page_idx = (self.len - 1) / PAGE_SIZE;
        let offset = (self.len - 1) % PAGE_SIZE;
        let new_len = self.len - 1;
        let should_remove_page = new_len % PAGE_SIZE == 0;
        
        let table = self.table_mut();
        Self::harden_page_for_write(&mut table.pages[page_idx], offset + 1);
        
        let val = if let PagePtr::Allocated(arc) = &mut table.pages[page_idx] {
            Arc::make_mut(arc).data.pop()
        } else {
            None
        };
        
        self.len = new_len;
        if should_remove_page && !self.table_mut().pages.is_empty() {
            self.table_mut().pages.pop();
        }
        
        val
    }

    /// Read an element (zero-copy; returns a reference or a cloned value)
    #[inline]
    pub fn get(&self, index: usize) -> Option<T> where T: Clone {
        if index >= self.len { return None; }
        let (page_idx, offset) = (index / PAGE_SIZE, index % PAGE_SIZE);
        
        match &self.table.pages[page_idx] {
            PagePtr::Zero => Some(unsafe { std::mem::zeroed() }),
            PagePtr::Inline(v) => Some(v.get()),
            PagePtr::Constant(v) => Some((**v).clone()),
            PagePtr::Allocated(page) => page.data.get(offset).cloned(),
        }
    }

    /// Read a reference (only valid for Allocated pages)
    #[inline]
    pub fn get_ref(&self, index: usize) -> Option<PageRef<'_, T>> {
        if index >= self.len { return None; }
        let (page_idx, offset) = (index / PAGE_SIZE, index % PAGE_SIZE);
        
        match &self.table.pages[page_idx] {
            PagePtr::Zero => Some(PageRef::Zero),
            PagePtr::Inline(v) => Some(PageRef::Inline(v.get())),
            PagePtr::Constant(v) => Some(PageRef::Borrowed(v)),
            PagePtr::Allocated(page) => page.data.get(offset).map(PageRef::Direct),
        }
    }

    /// Get a mutable reference (triggers COW)
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len { return None; }
        let (page_idx, offset) = (index / PAGE_SIZE, index % PAGE_SIZE);
        
        // Compute the page's actual length
        let page_len = if page_idx == self.table.pages.len() - 1 {
            let rem = self.len % PAGE_SIZE;
            if rem == 0 { PAGE_SIZE } else { rem }
        } else {
            PAGE_SIZE
        };
        
        let table = self.table_mut();
        Self::harden_page_for_write(&mut table.pages[page_idx], page_len);
        
        if let PagePtr::Allocated(arc) = &mut table.pages[page_idx] {
            Arc::make_mut(arc).data.get_mut(offset)
        } else {
            None
        }
    }

    /// Materialize a page (convert Zero/Inline/Constant into Allocated to support writes)
    fn harden_page_for_write(page: &mut PagePtr<T>, target_len: usize) where T: Clone {
        match page {
            PagePtr::Zero => {
                let data = vec![unsafe { std::mem::zeroed() }; target_len];
                *page = PagePtr::Allocated(Arc::new(AlignedPage::from_vec(data)));
            }
            PagePtr::Inline(v) => {
                let val = v.get();
                *page = PagePtr::Allocated(Arc::new(AlignedPage::from_fill(val, target_len)));
            }
            PagePtr::Constant(v) => {
                *page = PagePtr::Allocated(Arc::new(AlignedPage::from_fill((**v).clone(), target_len)));
            }
            PagePtr::Allocated(arc) => {
                // COW: clone if there are other references
                let _ = Arc::make_mut(arc);
            }
        }
    }

    /// Swap-remove
    pub fn swap_remove(&mut self, index: usize) where T: Clone {
        if index >= self.len { return; }
        if index == self.len - 1 { self.pop(); return; }
        
        if let Some(last_val) = self.pop() {
            if let Some(slot) = self.get_mut(index) {
                *slot = last_val;
            }
        }
    }

    /// Create from a Vec (with compression optimizations)
    pub fn from_vec(vec: Vec<T>) -> Self where T: Default + PartialEq {
        let chunks = vec.chunks(PAGE_SIZE);
        let mut table = PageTable::new();
        let mut total_len = 0;
        
        for chunk in chunks {
            total_len += chunk.len();
            
            // Detect all-zero pages
            let zero_val: T = unsafe { std::mem::zeroed() };
            if chunk.iter().all(|v| v == &zero_val) {
                table.pages.push(PagePtr::Zero);
                continue;
            }
            
            // Detect constant pages
            let first = &chunk[0];
            if chunk.iter().all(|v| v == first) {
                table.pages.push(Self::optimal_page_ptr(first));
                continue;
            }
            
            // Regular pages
            table.pages.push(PagePtr::Allocated(Arc::new(AlignedPage::from_vec(chunk.to_vec()))));
        }
        
        Self { table: Arc::new(table), len: total_len }
    }

    /// Create from a Vec (no compression; fewer constraints)
    pub fn from_vec_raw(vec: Vec<T>) -> Self {
        let chunks = vec.chunks(PAGE_SIZE);
        let mut table = PageTable::new();
        let mut total_len = 0;
        
        for chunk in chunks {
            total_len += chunk.len();
            table.pages.push(PagePtr::Allocated(Arc::new(AlignedPage::from_vec(chunk.to_vec()))));
        }
        
        Self { table: Arc::new(table), len: total_len }
    }

    /// Create from a slice (with compression)
    pub fn from_slice(slice: &[T]) -> Self where T: Default + PartialEq {
        Self::from_vec(slice.to_vec())
    }

    /// Create from a slice (no compression)
    pub fn from_slice_raw(slice: &[T]) -> Self {
        Self::from_vec_raw(slice.to_vec())
    }

    /// Flatten into a Vec
    pub fn flatten(&self) -> Vec<T> where T: Clone {
        self.iter().collect()
    }

    /// Flatten into an existing Vec (avoid reallocation)
    pub fn flatten_into(&self, out: &mut Vec<T>) where T: Clone {
        out.clear();
        out.reserve(self.len());
        out.extend(self.iter());
    }

    /// Iterator
    pub fn iter(&self) -> PagedBufferIter<'_, T> {
        PagedBufferIter { buffer: self, index: 0 }
    }

    /// Mutable iterator (materializes all pages)
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> where T: Clone {
        let len = self.len;
        let num_pages = self.table.pages.len();
        
        // Materialize all pages
        let table = self.table_mut();
        for i in 0..num_pages {
            let page_len = if i == num_pages - 1 {
                let rem = len % PAGE_SIZE;
                if rem == 0 { PAGE_SIZE } else { rem }
            } else {
                PAGE_SIZE
            };
            Self::harden_page_for_write(&mut table.pages[i], page_len);
        }
        
        self.table_mut().pages.iter_mut()
            .flat_map(|page| {
                if let PagePtr::Allocated(arc) = page {
                    Arc::make_mut(arc).data.iter_mut()
                } else {
                    unreachable!()
                }
            })
            .take(len)
    }

    /// Parallel iterator
    pub fn par_iter(&self) -> impl ParallelIterator<Item = T> + '_ where T: Clone {
        let total_len = self.len;
        let num_pages = self.table.pages.len();
        
        self.table.pages.par_iter().enumerate().flat_map(move |(idx, page)| {
            let take_count = if idx == num_pages - 1 {
                let rem = total_len % PAGE_SIZE;
                if rem == 0 { PAGE_SIZE } else { rem }
            } else {
                PAGE_SIZE
            };
            
            PageParIter { page, take_count, _marker: PhantomData }
        })
    }

    /// Append another buffer
    pub fn append(&mut self, other: &PagedBuffer<T>) where T: Clone {
        for item in other.iter() { self.push(item); }
    }

    /// Fill with a constant value (efficient: uses Inline/Constant pages)
    pub fn fill_constant(value: T, count: usize) -> Self {
        let full_pages = count / PAGE_SIZE;
        let remainder = count % PAGE_SIZE;
        
        let mut table = PageTable::new();
        let page_ptr = Self::optimal_page_ptr(&value);
        
        for _ in 0..full_pages {
            table.pages.push(page_ptr.clone());
        }
        
        if remainder > 0 {
            // The last page needs a real allocation to support partial fills
            table.pages.push(PagePtr::Allocated(Arc::new(AlignedPage::from_fill(value, remainder))));
        }
        
        Self { table: Arc::new(table), len: count }
    }

    /// Fill with zero values (efficient: uses the virtual Zero page)
    pub fn fill_zero(count: usize) -> Self {
        let full_pages = count / PAGE_SIZE;
        let remainder = count % PAGE_SIZE;
        
        let mut table = PageTable::new();
        
        for _ in 0..full_pages {
            table.pages.push(PagePtr::Zero);
        }
        
        if remainder > 0 {
            let data: Vec<T> = (0..remainder).map(|_| unsafe { std::mem::zeroed() }).collect();
            table.pages.push(PagePtr::Allocated(Arc::new(AlignedPage::from_vec(data))));
        }
        
        Self { table: Arc::new(table), len: count }
    }

    /// Compute memory usage
    pub fn memory_stats(&self) -> PagedBufferStats {
        let mut stats = PagedBufferStats::default();
        
        for page in &self.table.pages {
            match page {
                PagePtr::Zero => stats.zero_pages += 1,
                PagePtr::Inline(_) => stats.inline_pages += 1,
                PagePtr::Constant(_) => stats.constant_pages += 1,
                PagePtr::Allocated(arc) => {
                    stats.allocated_pages += 1;
                    stats.allocated_bytes += arc.data.len() * size_of::<T>();
                    if Arc::strong_count(arc) > 1 {
                        stats.shared_pages += 1;
                    }
                }
            }
        }
        
        stats.logical_elements = self.len;
        stats.element_size = size_of::<T>();
        stats
    }
}

/// Page reference (avoid unnecessary cloning)
pub enum PageRef<'a, T> {
    Zero,
    Inline(T),
    Borrowed(&'a T),
    Direct(&'a T),
}

impl<'a, T: Clone> PageRef<'a, T> {
    pub fn to_owned(self) -> T where T: Default {
        match self {
            PageRef::Zero => unsafe { std::mem::zeroed() },
            PageRef::Inline(v) => v,
            PageRef::Borrowed(v) | PageRef::Direct(v) => v.clone(),
        }
    }
}

/// Iterator
pub struct PagedBufferIter<'a, T: Clone + Send + Sync + 'static> {
    buffer: &'a PagedBuffer<T>,
    index: usize,
}

impl<'a, T: Clone + Send + Sync + 'static> Iterator for PagedBufferIter<'a, T> {
    type Item = T;
    
    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.buffer.len { return None; }
        let val = self.buffer.get(self.index);
        self.index += 1;
        val
    }
    
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.buffer.len - self.index;
        (remaining, Some(remaining))
    }
}

impl<'a, T: Clone + Send + Sync + 'static> ExactSizeIterator for PagedBufferIter<'a, T> {}

// --- From impls ---
impl<T: Clone + Send + Sync + 'static> From<Vec<T>> for PagedBuffer<T> {
    fn from(vec: Vec<T>) -> Self { Self::from_vec_raw(vec) }
}

/// Parallel iterator (page-level)
struct PageParIter<'a, T: Clone + Send + Sync + 'static> {
    page: &'a PagePtr<T>,
    take_count: usize,
    _marker: PhantomData<T>,
}

impl<'a, T: Clone + Send + Sync + 'static> ParallelIterator for PageParIter<'a, T> {
    type Item = T;
    
    fn drive_unindexed<C>(self, consumer: C) -> C::Result
    where C: rayon::iter::plumbing::UnindexedConsumer<Self::Item>
    {
        match self.page {
            PagePtr::Zero => {
                rayon::iter::repeat_n(unsafe { std::mem::zeroed() }, self.take_count).drive_unindexed(consumer)
            }
            PagePtr::Inline(v) => {
                rayon::iter::repeat_n(v.get(), self.take_count).drive_unindexed(consumer)
            }
            PagePtr::Constant(v) => {
                rayon::iter::repeat_n((**v).clone(), self.take_count).drive_unindexed(consumer)
            }
            PagePtr::Allocated(page) => {
                page.data.par_iter().take(self.take_count).cloned().drive_unindexed(consumer)
            }
        }
    }
}

/// Memory stats
#[derive(Debug, Default, Clone)]
pub struct PagedBufferStats {
    pub logical_elements: usize,
    pub element_size: usize,
    pub zero_pages: usize,
    pub inline_pages: usize,
    pub constant_pages: usize,
    pub allocated_pages: usize,
    pub shared_pages: usize,
    pub allocated_bytes: usize,
}

impl PagedBufferStats {
    /// Theoretical memory (no compression)
    pub fn theoretical_bytes(&self) -> usize {
        self.logical_elements * self.element_size
    }
    
    /// Actual memory (after compression)
    pub fn actual_bytes(&self) -> usize {
        self.allocated_bytes + self.constant_pages * self.element_size + self.inline_pages * 8
    }
    
    /// Compression ratio
    pub fn compression_ratio(&self) -> f64 {
        if self.allocated_bytes == 0 { return 0.0; }
        self.theoretical_bytes() as f64 / self.actual_bytes() as f64
    }
}

// --- Index impls ---
impl<T: Clone + Send + Sync + 'static> std::ops::Index<usize> for PagedBuffer<T> {
    type Output = T;
    fn index(&self, _index: usize) -> &Self::Output {
        panic!("PagedBuffer does not support direct indexing; use get() instead")
    }
}

// --- Serde impls ---
impl<T: Clone + Send + Sync + Serialize + 'static> Serialize for PagedBuffer<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.flatten().serialize(serializer)
    }
}

impl<'de, T: Clone + Send + Sync + Default + PartialEq + Deserialize<'de> + 'static> Deserialize<'de> for PagedBuffer<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let vec = Vec::<T>::deserialize(deserializer)?;
        Ok(Self::from_vec(vec))
    }
}

// --- AttributeStorage impls ---
impl<T: Clone + Default + PartialEq + Send + Sync + 'static + std::fmt::Debug> AttributeStorage for PagedBuffer<T> {
    fn len(&self) -> usize { self.len }
    fn swap_remove(&mut self, index: usize) { self.swap_remove(index); }
    fn push_default(&mut self) { self.push(T::default()); }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
    fn clone_box(&self) -> Box<dyn AttributeStorage> { Box::new(self.clone()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Vec3;

    #[test]
    fn test_zero_page_optimization() {
        let buffer: PagedBuffer<f32> = PagedBuffer::fill_zero(5000);
        let stats = buffer.memory_stats();
        
        assert_eq!(stats.zero_pages, 4); // 4 full zero pages
        assert_eq!(stats.allocated_pages, 1); // 1 partial page
        assert!(stats.actual_bytes() < stats.theoretical_bytes());
        println!("Compression ratio: {:.2}x", stats.compression_ratio());
    }

    #[test]
    fn test_inline_constant_page() {
        // f32 is 4 bytes, so it can be inlined
        let buffer: PagedBuffer<f32> = PagedBuffer::fill_constant(3.14, 5000);
        let stats = buffer.memory_stats();
        
        assert_eq!(stats.inline_pages, 4); // 4 inline constant pages
        assert_eq!(stats.allocated_bytes, 904 * 4); // Only the last page is actually allocated
        println!(
            "Inline pages: {}, compression ratio: {:.2}x",
            stats.inline_pages,
            stats.compression_ratio()
        );
    }

    #[test]
    fn test_vec3_constant_page() {
        // Vec3 is 12 bytes, so it can't be inlined; use Constant
        let buffer: PagedBuffer<Vec3> = PagedBuffer::fill_constant(Vec3::ONE, 5000);
        let stats = buffer.memory_stats();
        
        assert_eq!(stats.constant_pages, 4);
        println!(
            "Constant pages: {}, compression ratio: {:.2}x",
            stats.constant_pages,
            stats.compression_ratio()
        );
    }

    #[test]
    fn test_page_table_cow() {
        let buffer1: PagedBuffer<f32> = PagedBuffer::from_vec(vec![1.0; 3000]);
        let mut buffer2 = buffer1.clone();
        
        // After clone, they share the same page table
        assert_eq!(Arc::strong_count(&buffer1.table), 2);
        
        // Writing triggers COW
        buffer2.push(99.0);
        assert_eq!(Arc::strong_count(&buffer1.table), 1);
        assert_eq!(Arc::strong_count(&buffer2.table), 1);
    }

    #[test]
    fn test_basic_operations() {
        let mut buffer = PagedBuffer::new();
        for i in 0..2500 { buffer.push(i as f32); }
        
        assert_eq!(buffer.len(), 2500);
        assert_eq!(buffer.get(0), Some(0.0));
        assert_eq!(buffer.get(1023), Some(1023.0));
        assert_eq!(buffer.get(1024), Some(1024.0));
        assert_eq!(buffer.get(2499), Some(2499.0));
        assert_eq!(buffer.get(2500), None);
    }

    #[test]
    fn test_swap_remove() {
        let mut buffer: PagedBuffer<i32> = PagedBuffer::from_vec((0..10).collect());
        buffer.swap_remove(2);
        
        assert_eq!(buffer.len(), 9);
        assert_eq!(buffer.get(2), Some(9)); // The last element moved to position 2
    }
}
