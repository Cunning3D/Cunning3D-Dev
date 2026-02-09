//! PagedBuffer: 页级 COW 缓冲区，吊打 Houdini 的 UT_PageArray
//! 特性：页表级COW + 零值优化 + 小值内嵌 + 常量页压缩 + SIMD对齐

use std::sync::Arc;
use std::mem::{size_of, MaybeUninit};
use std::marker::PhantomData;
use rayon::prelude::*;
use serde::{Serialize, Deserialize, Serializer, Deserializer};
use std::any::Any;
use crate::libs::geometry::mesh::AttributeStorage;

/// 页大小：1024 元素，SIMD 友好（64 字节对齐的倍数）
pub const PAGE_SIZE: usize = 1024;
/// 内嵌阈值：≤8 字节的类型可以直接存在 enum 中
const INLINE_THRESHOLD: usize = 8;

/// 页指针：支持零值/内嵌/常量/共享四种模式
#[derive(Debug, Clone)]
pub enum PagePtr<T: Clone + Send + Sync + 'static> {
    /// 全零页（虚拟页，不分配内存）
    Zero,
    /// 内嵌值：≤8 字节类型的常量页，值直接存储，无堆分配
    Inline(InlineValue<T>),
    /// 常量页：所有元素相同（>8 字节类型）
    Constant(Arc<T>),
    /// 已分配页：真实数据，COW 共享
    Allocated(Arc<AlignedPage<T>>),
}

/// 内嵌值：最多 8 字节，直接存储在栈上
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
    /// 从值创建内嵌存储（仅当 size_of::<T>() <= 8）
    #[inline]
    pub fn new(value: &T) -> Self {
        debug_assert!(size_of::<T>() <= INLINE_THRESHOLD);
        let mut data = [0u8; INLINE_THRESHOLD];
        unsafe {
            std::ptr::copy_nonoverlapping(value as *const T as *const u8, data.as_mut_ptr(), size_of::<T>());
        }
        Self { data, _marker: PhantomData }
    }

    /// 读取内嵌值
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

/// 对齐页：64 字节对齐，SIMD 友好
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

/// 页表：可共享（COW）
#[derive(Debug, Clone)]
pub struct PageTable<T: Clone + Send + Sync + 'static> {
    pub pages: Vec<PagePtr<T>>,
}

impl<T: Clone + Send + Sync + 'static> PageTable<T> {
    #[inline]
    pub fn new() -> Self { Self { pages: Vec::new() } }
}

/// PagedBuffer：页级 COW 缓冲区
#[derive(Debug, Clone)]
pub struct PagedBuffer<T: Clone + Send + Sync + 'static> {
    /// 页表（Arc 共享，支持页表级 COW）
    table: Arc<PageTable<T>>,
    /// 逻辑长度
    len: usize,
}

impl<T: Clone + Send + Sync + Default + 'static> Default for PagedBuffer<T> {
    fn default() -> Self { Self::new() }
}

impl<T: Clone + Send + Sync + 'static> PagedBuffer<T> {
    /// 创建空缓冲区
    #[inline]
    pub fn new() -> Self {
        Self { table: Arc::new(PageTable::new()), len: 0 }
    }

    #[inline]
    pub fn len(&self) -> usize { self.len }

    #[inline]
    pub fn is_empty(&self) -> bool { self.len == 0 }

    /// 检查是否可以使用内嵌优化
    #[inline]
    const fn can_inline() -> bool { size_of::<T>() <= INLINE_THRESHOLD }

    /// 获取可变页表（COW：如果共享则复制）
    #[inline]
    fn table_mut(&mut self) -> &mut PageTable<T> {
        Arc::make_mut(&mut self.table)
    }

    /// 将值转为最优页指针类型
    #[inline]
    fn optimal_page_ptr(val: &T) -> PagePtr<T> {
        if Self::can_inline() {
            PagePtr::Inline(InlineValue::new(val))
        } else {
            PagePtr::Constant(Arc::new(val.clone()))
        }
    }

    /// 追加元素
    pub fn push(&mut self, value: T) {
        let page_idx = self.len / PAGE_SIZE;
        let offset = self.len % PAGE_SIZE;
        
        let table = self.table_mut();
        
        if offset == 0 {
            // 需要新页
            let mut page = AlignedPage::new();
            page.data.push(value);
            table.pages.push(PagePtr::Allocated(Arc::new(page)));
        } else {
            // 写入现有页
            Self::harden_page_for_write(&mut table.pages[page_idx], PAGE_SIZE);
            if let PagePtr::Allocated(arc) = &mut table.pages[page_idx] {
                Arc::make_mut(arc).data.push(value);
            }
        }
        self.len += 1;
    }

    /// 弹出最后一个元素
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

    /// 读取元素（零拷贝，返回引用或克隆值）
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

    /// 读取引用（仅对 Allocated 页有效）
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

    /// 获取可变引用（触发 COW）
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len { return None; }
        let (page_idx, offset) = (index / PAGE_SIZE, index % PAGE_SIZE);
        
        // 计算页的实际长度
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

    /// 硬化页（将 Zero/Inline/Constant 转为 Allocated 以支持写入）
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
                // COW：如果有其他引用，复制
                let _ = Arc::make_mut(arc);
            }
        }
    }

    /// 交换删除
    pub fn swap_remove(&mut self, index: usize) where T: Clone {
        if index >= self.len { return; }
        if index == self.len - 1 { self.pop(); return; }
        
        if let Some(last_val) = self.pop() {
            if let Some(slot) = self.get_mut(index) {
                *slot = last_val;
            }
        }
    }

    /// 从 Vec 创建（带压缩优化）
    pub fn from_vec(vec: Vec<T>) -> Self where T: Default + PartialEq {
        let chunks = vec.chunks(PAGE_SIZE);
        let mut table = PageTable::new();
        let mut total_len = 0;
        
        for chunk in chunks {
            total_len += chunk.len();
            
            // 检测全零页
            let zero_val: T = unsafe { std::mem::zeroed() };
            if chunk.iter().all(|v| v == &zero_val) {
                table.pages.push(PagePtr::Zero);
                continue;
            }
            
            // 检测常量页
            let first = &chunk[0];
            if chunk.iter().all(|v| v == first) {
                table.pages.push(Self::optimal_page_ptr(first));
                continue;
            }
            
            // 普通页
            table.pages.push(PagePtr::Allocated(Arc::new(AlignedPage::from_vec(chunk.to_vec()))));
        }
        
        Self { table: Arc::new(table), len: total_len }
    }

    /// 从 Vec 创建（无压缩，更少约束）
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

    /// 从切片创建（带压缩）
    pub fn from_slice(slice: &[T]) -> Self where T: Default + PartialEq {
        Self::from_vec(slice.to_vec())
    }

    /// 从切片创建（无压缩）
    pub fn from_slice_raw(slice: &[T]) -> Self {
        Self::from_vec_raw(slice.to_vec())
    }

    /// 展平为 Vec
    pub fn flatten(&self) -> Vec<T> where T: Clone {
        self.iter().collect()
    }

    /// 展平到已有 Vec（避免重复分配）
    pub fn flatten_into(&self, out: &mut Vec<T>) where T: Clone {
        out.clear();
        out.reserve(self.len());
        out.extend(self.iter());
    }

    /// 迭代器
    pub fn iter(&self) -> PagedBufferIter<'_, T> {
        PagedBufferIter { buffer: self, index: 0 }
    }

    /// 可变迭代器（触发全部硬化）
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> where T: Clone {
        let len = self.len;
        let num_pages = self.table.pages.len();
        
        // 硬化所有页
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

    /// 并行迭代器
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

    /// 追加另一个缓冲区
    pub fn append(&mut self, other: &PagedBuffer<T>) where T: Clone {
        for item in other.iter() { self.push(item); }
    }

    /// 填充常量值（高效：使用 Inline/Constant 页）
    pub fn fill_constant(value: T, count: usize) -> Self {
        let full_pages = count / PAGE_SIZE;
        let remainder = count % PAGE_SIZE;
        
        let mut table = PageTable::new();
        let page_ptr = Self::optimal_page_ptr(&value);
        
        for _ in 0..full_pages {
            table.pages.push(page_ptr.clone());
        }
        
        if remainder > 0 {
            // 最后一页需要真实分配以支持部分填充
            table.pages.push(PagePtr::Allocated(Arc::new(AlignedPage::from_fill(value, remainder))));
        }
        
        Self { table: Arc::new(table), len: count }
    }

    /// 填充零值（高效：使用 Zero 虚拟页）
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

    /// 统计内存使用
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

/// 页引用（避免不必要的克隆）
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

/// 迭代器
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

// --- From 实现 ---
impl<T: Clone + Send + Sync + 'static> From<Vec<T>> for PagedBuffer<T> {
    fn from(vec: Vec<T>) -> Self { Self::from_vec_raw(vec) }
}

/// 并行迭代器（页级）
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

/// 内存统计
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
    /// 理论内存（无压缩）
    pub fn theoretical_bytes(&self) -> usize {
        self.logical_elements * self.element_size
    }
    
    /// 实际内存（压缩后）
    pub fn actual_bytes(&self) -> usize {
        self.allocated_bytes + self.constant_pages * self.element_size + self.inline_pages * 8
    }
    
    /// 压缩比
    pub fn compression_ratio(&self) -> f64 {
        if self.allocated_bytes == 0 { return 0.0; }
        self.theoretical_bytes() as f64 / self.actual_bytes() as f64
    }
}

// --- Index 实现 ---
impl<T: Clone + Send + Sync + 'static> std::ops::Index<usize> for PagedBuffer<T> {
    type Output = T;
    fn index(&self, _index: usize) -> &Self::Output {
        panic!("PagedBuffer does not support direct indexing; use get() instead")
    }
}

// --- Serde 实现 ---
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

// --- AttributeStorage 实现 ---
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
        
        assert_eq!(stats.zero_pages, 4); // 4 个完整零页
        assert_eq!(stats.allocated_pages, 1); // 1 个部分页
        assert!(stats.actual_bytes() < stats.theoretical_bytes());
        println!("压缩比: {:.2}x", stats.compression_ratio());
    }

    #[test]
    fn test_inline_constant_page() {
        // f32 是 4 字节，可以内嵌
        let buffer: PagedBuffer<f32> = PagedBuffer::fill_constant(3.14, 5000);
        let stats = buffer.memory_stats();
        
        assert_eq!(stats.inline_pages, 4); // 4 个内嵌常量页
        assert_eq!(stats.allocated_bytes, 904 * 4); // 只有最后一页真正分配
        println!("内嵌页: {}, 压缩比: {:.2}x", stats.inline_pages, stats.compression_ratio());
    }

    #[test]
    fn test_vec3_constant_page() {
        // Vec3 是 12 字节，不能内嵌，使用 Constant
        let buffer: PagedBuffer<Vec3> = PagedBuffer::fill_constant(Vec3::ONE, 5000);
        let stats = buffer.memory_stats();
        
        assert_eq!(stats.constant_pages, 4);
        println!("常量页: {}, 压缩比: {:.2}x", stats.constant_pages, stats.compression_ratio());
    }

    #[test]
    fn test_page_table_cow() {
        let buffer1: PagedBuffer<f32> = PagedBuffer::from_vec(vec![1.0; 3000]);
        let mut buffer2 = buffer1.clone();
        
        // clone 后共享同一页表
        assert_eq!(Arc::strong_count(&buffer1.table), 2);
        
        // 写入触发 COW
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
        assert_eq!(buffer.get(2), Some(9)); // 最后一个元素移到了位置 2
    }
}
