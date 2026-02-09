use crate::libs::geometry::mesh::Geometry;
use crate::libs::geometry::ids::{AttributeId, AttributeDomain};
use bevy::math::Vec3;
use rayon::prelude::*;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

/// Guard that holds the runtime borrow locks for the duration of the parallel iteration.
/// When dropped, it releases the locks.
pub struct QueryGuard<'a, I> {
    iter: Option<I>,
    locks_ref: Arc<Mutex<HashMap<(AttributeDomain, AttributeId), i32>>>,
    borrowed: Vec<(AttributeDomain, AttributeId, bool)>, // (Domain, ID, is_write)
    _marker: std::marker::PhantomData<&'a mut Geometry>,
}

impl<'a, I> Drop for QueryGuard<'a, I> {
    fn drop(&mut self) {
        if let Ok(mut locks) = self.locks_ref.lock() {
            for (domain, id, is_write) in &self.borrowed {
                if let Some(count) = locks.get_mut(&(*domain, *id)) {
                    if *is_write {
                        // Writer release: must be -1
                        if *count == -1 {
                            *count = 0;
                        } else {
                            eprintln!("QueryGuard Drop: State corruption for write lock {:?} {:?}", domain, id);
                        }
                    } else {
                        // Reader release: decrement
                        if *count > 0 {
                            *count -= 1;
                        } else {
                            eprintln!("QueryGuard Drop: State corruption for read lock {:?} {:?}", domain, id);
                        }
                    }
                }
            }
        }
    }
}

// Delegate ParallelIterator to the inner iterator
impl<'a, I> ParallelIterator for QueryGuard<'a, I>
where I: ParallelIterator
{
    type Item = I::Item;
    
    fn drive_unindexed<C>(mut self, consumer: C) -> C::Result
    where C: rayon::iter::plumbing::UnindexedConsumer<Self::Item>
    {
        let iter = self.iter.take().expect("Iterator already consumed");
        iter.drive_unindexed(consumer)
    }
    
    fn opt_len(&self) -> Option<usize> {
        self.iter.as_ref()?.opt_len()
    }
}

impl<'a, I> IndexedParallelIterator for QueryGuard<'a, I>
where I: IndexedParallelIterator
{
    fn len(&self) -> usize {
        self.iter.as_ref().map(|i| i.len()).unwrap_or(0)
    }
    
    fn drive<C>(mut self, consumer: C) -> C::Result
    where C: rayon::iter::plumbing::Consumer<Self::Item>
    {
        let iter = self.iter.take().expect("Iterator already consumed");
        iter.drive(consumer)
    }
    
    fn with_producer<CB>(mut self, callback: CB) -> CB::Output
    where CB: rayon::iter::plumbing::ProducerCallback<Self::Item>
    {
        let iter = self.iter.take().expect("Iterator already consumed");
        iter.with_producer(callback)
    }
}

/// Helper to attempt locking an attribute.
fn try_lock_attribute(
    locks: &mut HashMap<(AttributeDomain, AttributeId), i32>,
    domain: AttributeDomain,
    name: &str,
    is_write: bool
) -> Option<AttributeId> {
    let id = AttributeId::from(name);
    let key = (domain, id);
    let state = locks.entry(key).or_insert(0);
    
    if is_write {
        // Writer needs 0 (free)
        if *state != 0 { return None; }
        *state = -1;
    } else {
        // Reader needs >= 0 (not writer)
        if *state == -1 { return None; }
        *state += 1;
    }
    Some(id)
}

/// Trait to extract a parallel iterator from Geometry for a specific type.
pub trait GeoAttributeQuery<'a> {
    type Item: Send;
    type Iter: IndexedParallelIterator<Item = Self::Item>;
    
    fn is_write() -> bool;
    
    /// Extract the iterator. Unsafe because it creates aliased mutable references if misused.
    /// The caller must ensure uniqueness of attribute access via locks.
    unsafe fn extract(geo: &'a Geometry, domain: AttributeDomain, name: &str) -> Option<Self::Iter>;
}

// --- Implementations for immutable references ---

impl<'a> GeoAttributeQuery<'a> for &'a Vec3 {
    type Item = &'a Vec3;
    type Iter = rayon::slice::Iter<'a, Vec3>;
    
    fn is_write() -> bool { false }
    
    unsafe fn extract(geo: &'a Geometry, domain: AttributeDomain, name: &str) -> Option<Self::Iter> {
        let attr = match domain {
            AttributeDomain::Point => geo.get_point_attribute(name),
            AttributeDomain::Vertex => geo.get_vertex_attribute(name),
            AttributeDomain::Primitive => geo.get_primitive_attribute(name),
            AttributeDomain::Edge => geo.get_edge_attribute(name),
            AttributeDomain::Detail => geo.get_detail_attribute(name),
        }?;
        
        attr.as_slice::<Vec3>().map(|vec| vec.par_iter())
    }
}

impl<'a> GeoAttributeQuery<'a> for &'a f32 {
    type Item = &'a f32;
    type Iter = rayon::slice::Iter<'a, f32>;
    
    fn is_write() -> bool { false }
    
    unsafe fn extract(geo: &'a Geometry, domain: AttributeDomain, name: &str) -> Option<Self::Iter> {
        let attr = match domain {
            AttributeDomain::Point => geo.get_point_attribute(name),
            AttributeDomain::Vertex => geo.get_vertex_attribute(name),
            AttributeDomain::Primitive => geo.get_primitive_attribute(name),
            AttributeDomain::Edge => geo.get_edge_attribute(name),
            AttributeDomain::Detail => geo.get_detail_attribute(name),
        }?;
        
        attr.as_slice::<f32>().map(|vec| vec.par_iter())
    }
}

// --- Implementations for mutable references ---

impl<'a> GeoAttributeQuery<'a> for &'a mut Vec3 {
    type Item = &'a mut Vec3;
    type Iter = rayon::slice::IterMut<'a, Vec3>;
    
    fn is_write() -> bool { true }
    
    #[allow(invalid_reference_casting)]
    unsafe fn extract(geo: &'a Geometry, domain: AttributeDomain, name: &str) -> Option<Self::Iter> {
        let ptr = geo as *const Geometry as *mut Geometry;
        let geo_mut = &mut *ptr;
        
        let attr = match domain {
            AttributeDomain::Point => geo_mut.get_point_attribute_mut(name),
            AttributeDomain::Vertex => geo_mut.get_vertex_attribute_mut(name),
            AttributeDomain::Primitive => geo_mut.get_primitive_attribute_mut(name),
            AttributeDomain::Edge => geo_mut.get_edge_attribute_mut(name),
            AttributeDomain::Detail => geo_mut.get_detail_attribute_mut(name),
        }?;
        
        attr.as_mut_slice::<Vec3>().map(|vec| vec.par_iter_mut())
    }
}

impl<'a> GeoAttributeQuery<'a> for &'a mut f32 {
    type Item = &'a mut f32;
    type Iter = rayon::slice::IterMut<'a, f32>;
    
    fn is_write() -> bool { true }
    
    #[allow(invalid_reference_casting)]
    unsafe fn extract(geo: &'a Geometry, domain: AttributeDomain, name: &str) -> Option<Self::Iter> {
        let ptr = geo as *const Geometry as *mut Geometry;
        let geo_mut = &mut *ptr;
        
        let attr = match domain {
            AttributeDomain::Point => geo_mut.get_point_attribute_mut(name),
            AttributeDomain::Vertex => geo_mut.get_vertex_attribute_mut(name),
            AttributeDomain::Primitive => geo_mut.get_primitive_attribute_mut(name),
            AttributeDomain::Edge => geo_mut.get_edge_attribute_mut(name),
            AttributeDomain::Detail => geo_mut.get_detail_attribute_mut(name),
        }?;
        
        attr.as_mut_slice::<f32>().map(|vec| vec.par_iter_mut())
    }
}

// --- The Query Interface ---

pub trait GeoQuery<'a> {
    type Item: Send;
    type Iter: IndexedParallelIterator<Item = Self::Item>;
    
    fn query(geo: &'a mut Geometry, domain: AttributeDomain, names: &[&str]) -> Option<QueryGuard<'a, Self::Iter>>;
}

// Impl for single component
impl<'a, T> GeoQuery<'a> for T 
where T: GeoAttributeQuery<'a> 
{
    type Item = T::Item;
    type Iter = T::Iter;
    
    fn query(geo: &'a mut Geometry, domain: AttributeDomain, names: &[&str]) -> Option<QueryGuard<'a, Self::Iter>> {
        if names.len() != 1 { return None; }
        
        let mut locks = geo.attribute_locks.lock().ok()?;
        let mut borrowed = Vec::new();
        
        let name = names[0];
        let is_write = T::is_write();
        
        if let Some(id) = try_lock_attribute(&mut locks, domain, name, is_write) {
            borrowed.push((domain, id, is_write));
        } else {
            return None; // Lock conflict
        }
        
        unsafe {
            let iter = T::extract(geo, domain, name)?;
            Some(QueryGuard {
                iter: Some(iter),
                locks_ref: geo.attribute_locks.clone(),
                borrowed,
                _marker: std::marker::PhantomData,
            })
        }
    }
}

// Impl for tuple (A, B)
impl<'a, A, B> GeoQuery<'a> for (A, B)
where 
    A: GeoAttributeQuery<'a>,
    B: GeoAttributeQuery<'a>,
{
    type Item = (A::Item, B::Item);
    type Iter = rayon::iter::Zip<A::Iter, B::Iter>;
    
    fn query(geo: &'a mut Geometry, domain: AttributeDomain, names: &[&str]) -> Option<QueryGuard<'a, Self::Iter>> {
        if names.len() != 2 { return None; }
        // Simple check for duplicate names (basic aliasing)
        if names[0] == names[1] { return None; }
        
        let mut locks = geo.attribute_locks.lock().ok()?;
        let mut borrowed = Vec::new();
        
        // Try Lock A
        if let Some(id) = try_lock_attribute(&mut locks, domain, names[0], A::is_write()) {
            borrowed.push((domain, id, A::is_write()));
        } else {
            // Revert: The easiest way is to track what we added and revert.
            // But locks are held.
            // Correct approach: check all locks first? Or rollback on fail.
            return None; 
        }
        
        // Try Lock B
        if let Some(id) = try_lock_attribute(&mut locks, domain, names[1], B::is_write()) {
            borrowed.push((domain, id, B::is_write()));
        } else {
            // Rollback A
            let (d, id, w) = borrowed[0];
            let c = locks.get_mut(&(d, id)).unwrap();
            if w { *c = 0; } else { *c -= 1; }
            return None;
        }
        
        unsafe {
            let iter_a = A::extract(geo, domain, names[0]);
            let iter_b = B::extract(geo, domain, names[1]);
            
            if iter_a.is_none() || iter_b.is_none() {
                // Cleanup locks
                for (d, id, w) in &borrowed {
                    let c = locks.get_mut(&(*d, *id)).unwrap();
                    if *w { *c = 0; } else { *c -= 1; }
                }
                return None;
            }
            
            Some(QueryGuard {
                iter: Some(iter_a.unwrap().zip(iter_b.unwrap())),
                locks_ref: geo.attribute_locks.clone(),
                borrowed,
                _marker: std::marker::PhantomData,
            })
        }
    }
}

// This helper trait is needed to pass variadic names cleanly
pub trait QueryNames {
    fn as_slice(&self) -> Vec<&str>;
}

impl QueryNames for &str {
    fn as_slice(&self) -> Vec<&str> { vec![self] }
}

impl QueryNames for (&str, &str) {
    fn as_slice(&self) -> Vec<&str> { vec![self.0, self.1] }
}

impl Geometry {
    pub fn query_points<'a, Q>(&'a mut self, names: impl QueryNames) -> Option<QueryGuard<'a, Q::Iter>>
    where Q: GeoQuery<'a> 
    {
        Q::query(self, AttributeDomain::Point, &names.as_slice())
    }

    pub fn query_vertices<'a, Q>(&'a mut self, names: impl QueryNames) -> Option<QueryGuard<'a, Q::Iter>>
    where Q: GeoQuery<'a> 
    {
        Q::query(self, AttributeDomain::Vertex, &names.as_slice())
    }

    pub fn query_primitives<'a, Q>(&'a mut self, names: impl QueryNames) -> Option<QueryGuard<'a, Q::Iter>>
    where Q: GeoQuery<'a> 
    {
        Q::query(self, AttributeDomain::Primitive, &names.as_slice())
    }

    pub fn query_edges<'a, Q>(&'a mut self, names: impl QueryNames) -> Option<QueryGuard<'a, Q::Iter>>
    where Q: GeoQuery<'a> 
    {
        Q::query(self, AttributeDomain::Edge, &names.as_slice())
    }

    pub fn query_detail<'a, Q>(&'a mut self, names: impl QueryNames) -> Option<QueryGuard<'a, Q::Iter>>
    where Q: GeoQuery<'a>
    {
        Q::query(self, AttributeDomain::Detail, &names.as_slice())
    }
}
