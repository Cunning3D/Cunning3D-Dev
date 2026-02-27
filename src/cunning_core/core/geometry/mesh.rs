use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, MeshVertexAttribute};
use bevy::render::render_resource::{PrimitiveTopology, VertexFormat};
use bevy::math::{DVec2, DVec3, DVec4};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock, atomic::{AtomicU64, Ordering}};
use serde::{Serialize, Deserialize, Serializer, Deserializer};
use rayon::prelude::*;
use std::any::Any;

use crate::libs::geometry::group::{ElementGroupMask, GroupPromoteMode};
use crate::libs::geometry::topology::Topology;
use crate::libs::geometry::edge_cache::EdgeCache;
use crate::libs::geometry::spatial::PrimitiveShape;
use crate::libs::geometry::attrs;
use crate::libs::geometry::interpolation::Interpolatable;
use crate::libs::algorithms::algorithms_dcc::PagedBuffer;

pub use crate::libs::geometry::ids::{AttributeId, AttributeDomain, PointId, VertexId, PrimId, EdgeId};
use crate::libs::geometry::sparse_set::SparseSetArena;

/// Adaptive threshold: auto-switch to PagedBuffer above this count
pub const PAGED_THRESHOLD: usize = 65536;

/// Hot attributes: NEVER use PagedBuffer (frequent random access)
const FORCE_VEC_ATTRS: &[&str] = &["@P", "@N", "@uv", "@uv2", "@Cd", "@v", "@knot_tangent_in", "@knot_tangent_out", "@knot_rot"];

/// Sparse-friendly attributes: allow PagedBuffer when large (constant/repetitive data)
const ALLOW_PAGED_ATTRS: &[&str] = &["@class", "@name", "@id", "@shop_materialpath", "@knot_mode", "@knot_link_id"];

use bvh::bvh::Bvh;
use kiddo::ImmutableKdTree;

pub const ATTRIBUTE_INSTANCE_POSITION: MeshVertexAttribute = MeshVertexAttribute::new("InstanceP", 9001, VertexFormat::Float32x3);
pub const ATTRIBUTE_INSTANCE_NORMAL: MeshVertexAttribute = MeshVertexAttribute::new("InstanceN", 9002, VertexFormat::Float32x3);

static GEOMETRY_DIRTY_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

pub fn new_dirty_id() -> u64 {
    GEOMETRY_DIRTY_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

// --- Attribute System (Type-Erased) ---

pub trait AttributeStorage: Send + Sync + 'static + std::fmt::Debug {
    fn len(&self) -> usize;
    fn swap_remove(&mut self, index: usize);
    fn push_default(&mut self);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn clone_box(&self) -> Box<dyn AttributeStorage>;
}

impl<T: Clone + Default + Send + Sync + 'static + std::fmt::Debug> AttributeStorage for Vec<T> {
    fn len(&self) -> usize { self.len() }
    fn swap_remove(&mut self, index: usize) { if index < self.len() { self.swap_remove(index); } }
    fn push_default(&mut self) { self.push(T::default()); }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
    fn clone_box(&self) -> Box<dyn AttributeStorage> { Box::new(self.clone()) }
}

#[derive(Clone, Debug, Default)]
pub struct Bytes(pub Vec<u8>);
impl AttributeStorage for Bytes {
    fn len(&self) -> usize { self.0.len() }
    fn swap_remove(&mut self, index: usize) { if index < self.0.len() { self.0.swap_remove(index); } }
    fn push_default(&mut self) { self.0.push(0); }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
    fn clone_box(&self) -> Box<dyn AttributeStorage> { Box::new(self.clone()) }
}

#[derive(Debug)]
pub struct Attribute {
    data: Box<dyn AttributeStorage>,
}

impl Clone for Attribute {
    fn clone(&self) -> Self {
        Self { data: self.data.clone_box() }
    }
}

impl Attribute {
    pub fn new<T: AttributeStorage>(data: T) -> Self {
        Self { data: Box::new(data) }
    }

    /// Smart constructor: chooses Vec or PagedBuffer based on attribute name and size
    /// - Hot attrs (@P, @N, @uv, etc.) → always Vec
    /// - Sparse-friendly attrs (@class, @name, etc.) + large → PagedBuffer
    /// - Others → Vec (conservative)
    pub fn new_for<T: Clone + Default + PartialEq + Send + Sync + 'static + std::fmt::Debug>(name: &str, data: Vec<T>) -> Self {
        // Rule 1: Hot attributes → always Vec
        if FORCE_VEC_ATTRS.iter().any(|&n| name == n) {
            return Self { data: Box::new(data) };
        }
        // Rule 2: Small data → Vec
        if data.len() <= PAGED_THRESHOLD {
            return Self { data: Box::new(data) };
        }
        // Rule 3: Sparse-friendly + large → PagedBuffer with compression
        if ALLOW_PAGED_ATTRS.iter().any(|&n| name == n) {
            return Self { data: Box::new(PagedBuffer::from_vec(data)) };
        }
        // Rule 4: Unknown large attr → Vec (conservative)
        Self { data: Box::new(data) }
    }

    /// Legacy: auto-upgrade based on size only (no name awareness)
    pub fn new_auto<T: Clone + Default + PartialEq + Send + Sync + 'static + std::fmt::Debug>(data: Vec<T>) -> Self {
        if data.len() > PAGED_THRESHOLD {
            Self { data: Box::new(PagedBuffer::from_vec_raw(data)) }
        } else {
            Self { data: Box::new(data) }
        }
    }

    /// Auto-construct with compression: detect zero/constant pages for better compression
    pub fn new_auto_compressed<T: Clone + Default + PartialEq + Send + Sync + 'static + std::fmt::Debug>(data: Vec<T>) -> Self {
        if data.len() > PAGED_THRESHOLD {
            Self { data: Box::new(PagedBuffer::from_vec(data)) }
        } else {
            Self { data: Box::new(data) }
        }
    }

    /// Force PagedBuffer (for explicit large data scenarios)
    pub fn new_paged<T: Clone + Default + PartialEq + Send + Sync + 'static + std::fmt::Debug>(data: Vec<T>) -> Self {
        Self { data: Box::new(PagedBuffer::from_vec_raw(data)) }
    }

    /// Check if using PagedBuffer backend
    pub fn is_paged(&self) -> bool {
        let type_name = std::any::type_name_of_val(&*self.data);
        type_name.contains("PagedBuffer")
    }

    // Back-compat constructors (many legacy call sites use `Attribute::Vec3(...)` style).
    #[inline] pub fn F32<T: AttributeStorage>(data: T) -> Self { Self::new(data) }
    #[inline] pub fn F64<T: AttributeStorage>(data: T) -> Self { Self::new(data) }
    #[inline] pub fn Vec2<T: AttributeStorage>(data: T) -> Self { Self::new(data) }
    #[inline] pub fn Vec3<T: AttributeStorage>(data: T) -> Self { Self::new(data) }
    #[inline] pub fn Vec4<T: AttributeStorage>(data: T) -> Self { Self::new(data) }
    #[inline] pub fn DVec2<T: AttributeStorage>(data: T) -> Self { Self::new(data) }
    #[inline] pub fn DVec3<T: AttributeStorage>(data: T) -> Self { Self::new(data) }
    #[inline] pub fn DVec4<T: AttributeStorage>(data: T) -> Self { Self::new(data) }
    #[inline] pub fn I32<T: AttributeStorage>(data: T) -> Self { Self::new(data) }
    #[inline] pub fn Bool<T: AttributeStorage>(data: T) -> Self { Self::new(data) }
    #[inline] pub fn String<T: AttributeStorage>(data: T) -> Self { Self::new(data) }

    pub fn len(&self) -> usize { self.data.len() }
    pub fn swap_remove(&mut self, index: usize) { self.data.swap_remove(index); }
    pub fn push_default(&mut self) { self.data.push_default(); }

    /// Read slice (compatible with Vec backend)
    pub fn as_slice<T: 'static>(&self) -> Option<&[T]> {
        self.data.as_any().downcast_ref::<Vec<T>>().map(|v| v.as_slice())
    }

    /// Read mutable slice (compatible with Vec backend)
    pub fn as_mut_slice<T: 'static>(&mut self) -> Option<&mut [T]> {
        self.data.as_any_mut().downcast_mut::<Vec<T>>().map(|v| v.as_mut_slice())
    }

    /// Read PagedBuffer (if backend is PagedBuffer)
    pub fn as_paged<T: Clone + Send + Sync + 'static>(&self) -> Option<&PagedBuffer<T>> {
        self.data.as_any().downcast_ref::<PagedBuffer<T>>()
    }

    /// Read mutable PagedBuffer
    pub fn as_paged_mut<T: Clone + Send + Sync + 'static>(&mut self) -> Option<&mut PagedBuffer<T>> {
        self.data.as_any_mut().downcast_mut::<PagedBuffer<T>>()
    }

    /// Unified read: return Vec (may copy) regardless of backend (Vec or PagedBuffer)
    pub fn to_vec<T: Clone + Send + Sync + 'static>(&self) -> Option<Vec<T>> {
        if let Some(v) = self.as_slice::<T>() {
            Some(v.to_vec())
        } else if let Some(pb) = self.as_paged::<T>() {
            Some(pb.flatten())
        } else {
            None
        }
    }
    
    /// Access the underlying storage directly if it matches type S.
    pub fn as_storage<S: 'static>(&self) -> Option<&S> {
        self.data.as_any().downcast_ref::<S>()
    }

    pub fn as_storage_mut<S: 'static>(&mut self) -> Option<&mut S> {
        self.data.as_any_mut().downcast_mut::<S>()
    }
    
    // Helpers for common types
    pub fn as_vec3(&self) -> Option<&[Vec3]> { self.as_slice::<Vec3>() }
    pub fn as_vec3_mut(&mut self) -> Option<&mut [Vec3]> { self.as_mut_slice::<Vec3>() }
    pub fn as_f32(&self) -> Option<&[f32]> { self.as_slice::<f32>() }
    pub fn as_f32_mut(&mut self) -> Option<&mut [f32]> { self.as_mut_slice::<f32>() }
}

#[inline]
fn reserve_attr_handle(handle: &mut AttributeHandle, additional: usize) {
    if additional == 0 { return; }
    let a = handle.get_mut();
    macro_rules! r { ($t:ty) => { if let Some(v) = a.as_storage_mut::<Vec<$t>>() { v.reserve(additional); return; } }; }
    r!(f32); r!(Vec2); r!(Vec3); r!(Vec4); r!(f64); r!(DVec2); r!(DVec3); r!(DVec4); r!(i32); r!(bool); r!(String);
    if let Some(b) = a.as_storage_mut::<Bytes>() { b.0.reserve(additional); }
}

// --- Attribute Serialization Helper ---
// Keeps compatibility with old enum format while allowing extensibility
#[derive(Serialize, Deserialize)]
enum AttributeDataEnum {
    F32(Vec<f32>),
    Vec2(Vec<Vec2>),
    Vec3(Vec<Vec3>),
    Vec4(Vec<Vec4>),
    F64(Vec<f64>),
    DVec2(Vec<DVec2>),
    DVec3(Vec<DVec3>),
    DVec4(Vec<DVec4>),
    I32(Vec<i32>),
    Bool(Vec<bool>),
    String(Vec<String>),
    Bytes(Vec<u8>),
    // Future: Custom(Box<serde_json::value::RawValue>)
}

impl Serialize for Attribute {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer {
        // Try to downcast to known types
        let data = if let Some(v) = self.as_slice::<f32>() { AttributeDataEnum::F32(v.to_vec()) }
        else if let Some(v) = self.as_slice::<Vec2>() { AttributeDataEnum::Vec2(v.to_vec()) }
        else if let Some(v) = self.as_slice::<Vec3>() { AttributeDataEnum::Vec3(v.to_vec()) }
        else if let Some(v) = self.as_slice::<Vec4>() { AttributeDataEnum::Vec4(v.to_vec()) }
        else if let Some(v) = self.as_slice::<f64>() { AttributeDataEnum::F64(v.to_vec()) }
        else if let Some(v) = self.as_slice::<DVec2>() { AttributeDataEnum::DVec2(v.to_vec()) }
        else if let Some(v) = self.as_slice::<DVec3>() { AttributeDataEnum::DVec3(v.to_vec()) }
        else if let Some(v) = self.as_slice::<DVec4>() { AttributeDataEnum::DVec4(v.to_vec()) }
        else if let Some(v) = self.as_slice::<i32>() { AttributeDataEnum::I32(v.to_vec()) }
        else if let Some(v) = self.as_slice::<bool>() { AttributeDataEnum::Bool(v.to_vec()) }
        else if let Some(v) = self.as_slice::<String>() { AttributeDataEnum::String(v.to_vec()) }
        else if let Some(v) = self.as_storage::<Bytes>() { AttributeDataEnum::Bytes(v.0.clone()) }
        else {
            // For unknown types, we currently fail or skip. 
            // In a real implementation, we'd need a registry.
            // For now, let's serialize as empty F32 to avoid crash, or error.
            return Err(serde::ser::Error::custom("Unsupported attribute type for serialization"));
        };
        data.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Attribute {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: Deserializer<'de> {
        let data = AttributeDataEnum::deserialize(deserializer)?;
        Ok(match data {
            AttributeDataEnum::F32(v) => Attribute::new(v),
            AttributeDataEnum::Vec2(v) => Attribute::new(v),
            AttributeDataEnum::Vec3(v) => Attribute::new(v),
            AttributeDataEnum::Vec4(v) => Attribute::new(v),
            AttributeDataEnum::F64(v) => Attribute::new(v),
            AttributeDataEnum::DVec2(v) => Attribute::new(v),
            AttributeDataEnum::DVec3(v) => Attribute::new(v),
            AttributeDataEnum::DVec4(v) => Attribute::new(v),
            AttributeDataEnum::I32(v) => Attribute::new(v),
            AttributeDataEnum::Bool(v) => Attribute::new(v),
            AttributeDataEnum::String(v) => Attribute::new(v),
            AttributeDataEnum::Bytes(v) => Attribute::new(Bytes(v)),
        })
    }
}

/// A thread-safe, versioned handle to an attribute buffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributeHandle {
    pub data: std::sync::Arc<Attribute>,
    pub version: u64,
}

impl AttributeHandle {
    pub fn new(attr: Attribute) -> Self {
        Self {
            data: std::sync::Arc::new(attr),
            version: new_dirty_id(),
        }
    }

    pub fn get_mut(&mut self) -> &mut Attribute {
        self.version = new_dirty_id();
        std::sync::Arc::make_mut(&mut self.data)
    }
}

// Stats structs...
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vec3Stats {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub avg: [f32; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vec2Stats {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub avg: [f32; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TopologySummary {
    pub boundary_edge_count: usize,
    pub non_manifold_edge_count: usize,
    pub has_open_boundary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeometryFingerprint {
    pub point_count: usize,
    pub primitive_count: usize,
    pub bbox_min: Option<[f32; 3]>,
    pub bbox_max: Option<[f32; 3]>,
    pub position_stats: Option<Vec3Stats>,
    pub normal_stats: Option<Vec3Stats>,
    pub color_stats: Option<Vec3Stats>,
    pub uv_stats: Option<Vec2Stats>,
    pub topology: TopologySummary,
}

/// Explicit Edge representation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeoEdge {
    pub p0: PointId,
    pub p1: PointId,
}

/// Vertex referring to a point.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeoVertex {
    pub point_id: PointId, 
}

// --- Primitives (Extensible & Unity Compatible) ---

/// Unity Spline compatible Bezier Knot
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BezierKnot {
    pub position: Vec3,
    pub tangent_in: Vec3,
    pub tangent_out: Vec3,
    pub rotation: Quat,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TangentMode {
    AutoSmooth,
    Mirrored,
    Continuous,
    Broken,
    Linear,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PolygonPrim {
    pub vertices: Vec<VertexId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolylinePrim {
    pub vertices: Vec<VertexId>,
    pub closed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BezierCurvePrim {
    pub vertices: Vec<VertexId>,
    pub closed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GeoPrimitive {
    Polygon(PolygonPrim),
    Polyline(PolylinePrim),
    BezierCurve(BezierCurvePrim),
    // Future: NurbsCurve, Volume, PackedGeo
}

// Legacy primitive kind (kept for old nodes/UI/FFI code paths)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrimitiveType { Polygon, Polyline, BezierCurve }

impl Default for GeoPrimitive {
    fn default() -> Self {
        GeoPrimitive::Polygon(PolygonPrim::default())
    }
}

// Helpers to access vertices if the primitive supports them
impl GeoPrimitive {
    pub fn vertices(&self) -> &[VertexId] {
        match self {
            GeoPrimitive::Polygon(p) => &p.vertices,
            GeoPrimitive::Polyline(p) => &p.vertices,
            GeoPrimitive::BezierCurve(p) => &p.vertices,
        }
    }
}

// --- Geometry Core ---

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Geometry {
    #[serde(skip)]
    pub dirty_id: u64,
    
    // Core Elements
    pub(crate) points: SparseSetArena<()>, 
    pub(crate) vertices: SparseSetArena<GeoVertex>,
    pub(crate) primitives: SparseSetArena<GeoPrimitive>,
    pub(crate) edges: SparseSetArena<GeoEdge>, 
    
    // Attributes
    pub point_attributes: HashMap<AttributeId, AttributeHandle>,
    pub vertex_attributes: HashMap<AttributeId, AttributeHandle>,
    pub primitive_attributes: HashMap<AttributeId, AttributeHandle>,
    pub edge_attributes: HashMap<AttributeId, AttributeHandle>,
    pub detail_attributes: HashMap<AttributeId, AttributeHandle>,
    
    // Groups
    pub point_groups: HashMap<AttributeId, ElementGroupMask>,
    pub vertex_groups: HashMap<AttributeId, ElementGroupMask>,
    pub primitive_groups: HashMap<AttributeId, ElementGroupMask>,
    
    // Explicit Edge Groups (Lazy / Optional)
    // Only allocated when user explicitly creates edge groups or path tools need them.
    // This keeps the structure small for the 90% case, but powerful for the 10%.
    pub edge_groups: Option<HashMap<AttributeId, ElementGroupMask>>,
    
    #[serde(skip)]
    pub sdfs: Vec<crate::sdf::SdfHandle>,
    
    #[serde(skip)]
    pub(crate) attribute_locks: Arc<Mutex<HashMap<(AttributeDomain, AttributeId), i32>>>,
    
    #[serde(skip)]
    pub(crate) topology_cache: Arc<RwLock<Option<Arc<Topology>>>>,
    
    #[serde(skip)]
    pub(crate) edge_cache: Arc<RwLock<Option<Arc<EdgeCache>>>>,

    // Cache stores (BVH, position_attribute_version)
    #[serde(skip)]
    pub(crate) prim_bvh_cache: Arc<RwLock<Option<(Arc<Bvh<f32, 3>>, u64)>>>,
    
    // KDTree cache for point nearest-neighbor queries
    #[serde(skip)]
    pub(crate) point_kdtree_cache: Arc<RwLock<Option<(Arc<ImmutableKdTree<f32, 3>>, u64)>>>,
    
    // Bevy Mesh cache (positions, normals, uvs, indices) - version based on P + N + UV + topology
    #[serde(skip)]
    pub(crate) bevy_mesh_cache: Arc<RwLock<Option<BevyMeshCache>>>,
    
    // Wireframe indices cache - version based on topology only
    #[serde(skip)]
    pub(crate) wireframe_cache: Arc<RwLock<Option<WireframeCache>>>,
}

/// Cached Bevy mesh data to avoid per-frame recomputation
#[derive(Clone, Debug)]
pub struct BevyMeshCache {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Option<Vec<[f32; 2]>>,
    pub colors: Option<Vec<[f32; 4]>>,
    pub indices: Vec<u32>,
    pub version_p: u64,
    pub version_n: u64,
    pub version_uv: u64,
    pub version_cd: u64,
    pub prim_count: usize,
}

/// Cached wireframe indices
#[derive(Clone, Debug)]
pub struct WireframeCache {
    pub indices: Vec<u32>,
    pub prim_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CdSource { None, Detail, Vertex, Point, Primitive }

#[inline]
fn mix_version(tag: u64, ver: u64) -> u64 { ver ^ tag.wrapping_mul(0x9E37_79B9_7F4A_7C15) }

impl Geometry {
    pub fn new() -> Self {
        Self {
            dirty_id: new_dirty_id(),
            attribute_locks: Arc::new(Mutex::new(HashMap::new())),
            topology_cache: Arc::new(RwLock::new(None)),
            edge_cache: Arc::new(RwLock::new(None)),
            prim_bvh_cache: Arc::new(RwLock::new(None)),
            point_kdtree_cache: Arc::new(RwLock::new(None)),
            bevy_mesh_cache: Arc::new(RwLock::new(None)),
            wireframe_cache: Arc::new(RwLock::new(None)),
            ..Default::default()
        }
    }

    pub fn fork(&self) -> Self {
        let mut new_geo = self.clone();
        new_geo.dirty_id = new_dirty_id();
        new_geo.attribute_locks = Arc::new(Mutex::new(HashMap::new()));
        
        let topo = self.topology_cache.read().unwrap().clone();
        new_geo.topology_cache = Arc::new(RwLock::new(topo));
        
        let ec = self.edge_cache.read().unwrap().clone();
        new_geo.edge_cache = Arc::new(RwLock::new(ec));

        let bvh = self.prim_bvh_cache.read().unwrap().clone();
        new_geo.prim_bvh_cache = Arc::new(RwLock::new(bvh));
        
        let kdtree = self.point_kdtree_cache.read().unwrap().clone();
        new_geo.point_kdtree_cache = Arc::new(RwLock::new(kdtree));
        
        let mesh_cache = self.bevy_mesh_cache.read().unwrap().clone();
        new_geo.bevy_mesh_cache = Arc::new(RwLock::new(mesh_cache));
        
        let wf_cache = self.wireframe_cache.read().unwrap().clone();
        new_geo.wireframe_cache = Arc::new(RwLock::new(wf_cache));
        
        new_geo
    }

    pub fn invalidate_topology(&self) {
        if let Ok(mut cache) = self.topology_cache.write() { *cache = None; }
        if let Ok(mut cache) = self.edge_cache.write() { *cache = None; }
        if let Ok(mut cache) = self.wireframe_cache.write() { *cache = None; }
        if let Ok(mut cache) = self.bevy_mesh_cache.write() { *cache = None; }
        self.invalidate_spatial_caches();
    }
    
    pub fn invalidate_spatial_caches(&self) {
        if let Ok(mut cache) = self.prim_bvh_cache.write() { *cache = None; }
        if let Ok(mut cache) = self.point_kdtree_cache.write() { *cache = None; }
    }

    pub fn get_topology(&self) -> Arc<Topology> {
        if let Ok(cache) = self.topology_cache.read() {
            if let Some(topo) = cache.as_ref() {
                return topo.clone();
            }
        }
        
        let topo = Arc::new(Topology::build_from(self));
        
        if let Ok(mut cache) = self.topology_cache.write() {
            *cache = Some(topo.clone());
        }
        topo
    }

    pub fn get_edge_cache(&self) -> Arc<EdgeCache> {
        if let Ok(cache) = self.edge_cache.read() {
            if let Some(ec) = cache.as_ref() {
                return ec.clone();
            }
        }
        
        let ec = Arc::new(EdgeCache::build(self));
        
        if let Ok(mut cache) = self.edge_cache.write() {
            *cache = Some(ec.clone());
        }
        ec
    }

    pub fn get_prim_bvh(&self) -> Arc<Bvh<f32, 3>> {
        puffin::profile_function!();
        // Check current position version
        let current_p_version = self.point_attributes.get(&attrs::P.into())
            .map(|h| h.version).unwrap_or(0);

        if let Ok(cache) = self.prim_bvh_cache.read() {
            if let Some((bvh, cached_version)) = cache.as_ref() {
                if *cached_version == current_p_version {
                    return bvh.clone();
                }
            }
        }

        // Build BVH
        puffin::profile_scope!("geometry_build_prim_bvh");
        // 1. Get position attribute
        let pos_attr = self.get_point_attribute(attrs::P)
            .and_then(|a| a.as_slice::<Vec3>())
            .unwrap_or(&[]); // Empty if no P attribute, will result in empty BVH

        // 2. Create shapes
        let mut shapes: Vec<PrimitiveShape> = (0..self.primitives.len())
            .into_par_iter()
            .map(|i| PrimitiveShape::new(i, self, pos_attr))
            .collect();

        // 3. Build BVH
        let bvh = Arc::new(Bvh::build(&mut shapes));
        
        if let Ok(mut cache) = self.prim_bvh_cache.write() {
            *cache = Some((bvh.clone(), current_p_version));
        }
        bvh
    }

    /// Get or build a KDTree for point nearest-neighbor queries.
    /// Automatically rebuilds when @P attribute changes.
    pub fn get_point_kdtree(&self) -> Arc<ImmutableKdTree<f32, 3>> {
        puffin::profile_function!();
        let current_p_version = self.point_attributes.get(&attrs::P.into()).map(|h| h.version).unwrap_or(0);

        if let Ok(cache) = self.point_kdtree_cache.read() {
            if let Some((tree, cached_version)) = cache.as_ref() {
                if *cached_version == current_p_version {
                    return tree.clone();
                }
            }
        }

        // Build KDTree from point positions
        puffin::profile_scope!("geometry_build_point_kdtree");
        let positions = self.get_point_attribute(attrs::P)
            .and_then(|a| a.as_slice::<Vec3>())
            .unwrap_or(&[]);

        let entries: Vec<[f32; 3]> = positions.par_iter().map(|p| [p.x, p.y, p.z]).collect();
        let tree = Arc::new(ImmutableKdTree::new_from_slice(&entries));

        if let Ok(mut cache) = self.point_kdtree_cache.write() {
            *cache = Some((tree.clone(), current_p_version));
        }
        tree
    }

    #[inline]
    pub fn structural_edit<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.invalidate_topology();
        f(self)
    }

    // --- Accessors ---

    #[inline] pub fn points(&self) -> &SparseSetArena<()> { &self.points }
    #[inline] pub fn vertices(&self) -> &SparseSetArena<GeoVertex> { &self.vertices }
    #[inline] pub fn primitives(&self) -> &SparseSetArena<GeoPrimitive> { &self.primitives }
    #[inline] pub fn edges(&self) -> &SparseSetArena<GeoEdge> { &self.edges }

    #[inline]
    pub fn points_mut(&mut self) -> &mut SparseSetArena<()> {
        self.invalidate_topology();
        &mut self.points
    }

    #[inline]
    pub fn vertices_mut(&mut self) -> &mut SparseSetArena<GeoVertex> {
        self.invalidate_topology();
        &mut self.vertices
    }

    #[inline]
    pub fn primitives_mut(&mut self) -> &mut SparseSetArena<GeoPrimitive> {
        self.invalidate_topology();
        &mut self.primitives
    }

    #[inline]
    pub fn edges_mut(&mut self) -> &mut SparseSetArena<GeoEdge> {
        self.invalidate_topology();
        &mut self.edges
    }

    #[inline]
    pub(crate) fn set_vertex_point_no_invalidate(&mut self, vid: VertexId, pid: PointId) {
        if let Some(v) = self.vertices.get_mut(vid.into()) {
            v.point_id = pid;
        }
    }

    #[inline]
    pub fn set_vertex_point(&mut self, vid: VertexId, pid: PointId) {
        self.invalidate_topology();
        self.set_vertex_point_no_invalidate(vid, pid);
    }

    #[inline]
    pub(crate) fn set_primitive_vertices_no_invalidate(&mut self, prim_id: PrimId, vertices: Vec<VertexId>) {
        if let Some(p) = self.primitives.get_mut(prim_id.into()) {
            match p {
                GeoPrimitive::Polygon(poly) => poly.vertices = vertices,
                GeoPrimitive::Polyline(line) => line.vertices = vertices,
                _ => {} // Bezier doesn't support vertex setting this way
            }
        }
    }

    #[inline]
    pub fn set_primitive_vertices(&mut self, prim_id: PrimId, vertices: Vec<VertexId>) {
        self.invalidate_topology();
        self.set_primitive_vertices_no_invalidate(prim_id, vertices);
    }
    
    pub fn get_vertex_id_at_dense_index(&self, idx: usize) -> Option<VertexId> {
        self.vertices.get_id_from_dense(idx).map(VertexId::from)
    }

    pub fn clean(&mut self) {
        let mut dead_verts = Vec::new();
        for (v_id, v) in self.vertices.iter_enumerated() {
            if self.points.get(v.point_id.into()).is_none() {
                dead_verts.push(VertexId::from(v_id));
            }
        }
        for v in dead_verts { self.remove_vertex(v); }
        
        let mut dead_prims = Vec::new();
        for (p_id, prim) in self.primitives.iter_enumerated() {
            for v_id in prim.vertices() {
                if self.vertices.get((*v_id).into()).is_none() {
                    dead_prims.push(PrimId::from(p_id));
                    break;
                }
            }
        }
        for p in dead_prims { self.remove_primitive(p); }
        
        let mut dead_edges = Vec::new();
        for (e_id, edge) in self.edges.iter_enumerated() {
            if self.points.get(edge.p0.into()).is_none() || self.points.get(edge.p1.into()).is_none() {
                dead_edges.push(EdgeId::from(e_id));
            }
        }
        for e in dead_edges { self.remove_edge(e); }
    }

    pub fn get_detail_attribute(&self, name: &str) -> Option<&Attribute> {
        self.detail_attributes.get(&AttributeId::from(name)).map(|h| &*h.data)
    }

    pub fn get_detail_attribute_mut(&mut self, name: &str) -> Option<&mut Attribute> {
        self.detail_attributes.get_mut(&AttributeId::from(name)).map(|h| Arc::make_mut(&mut h.data))
    }

    pub fn set_detail_attribute<T: AttributeStorage>(&mut self, name: &str, value: T) {
        let attr = Attribute::new(value);
        let id = AttributeId::from(name);
        self.detail_attributes.insert(id, AttributeHandle::new(attr));
    }
    
    pub fn remove_detail_attribute(&mut self, name: &str) {
        self.detail_attributes.remove(&AttributeId::from(name));
    }

    pub fn ensure_point_group(&mut self, name: &str) -> &mut ElementGroupMask {
        self.point_groups.entry(AttributeId::from(name)).or_insert_with(|| {
            let mut mask = ElementGroupMask::new(self.points.len());
            while mask.len() < self.points.len() { mask.push(false); }
            mask
        })
    }

    pub fn ensure_vertex_group(&mut self, name: &str) -> &mut ElementGroupMask {
        self.vertex_groups.entry(AttributeId::from(name)).or_insert_with(|| {
            let mut mask = ElementGroupMask::new(self.vertices.len());
            while mask.len() < self.vertices.len() { mask.push(false); }
            mask
        })
    }

    pub fn ensure_primitive_group(&mut self, name: &str) -> &mut ElementGroupMask {
        self.primitive_groups.entry(AttributeId::from(name)).or_insert_with(|| {
            let mut mask = ElementGroupMask::new(self.primitives.len());
            while mask.len() < self.primitives.len() { mask.push(false); }
            mask
        })
    }

    pub fn get_point_group_mut(&mut self, name: &str) -> Option<&mut ElementGroupMask> {
        self.point_groups.get_mut(&AttributeId::from(name))
    }

    pub fn get_point_group(&self, name: &str) -> Option<&ElementGroupMask> {
        self.point_groups.get(&AttributeId::from(name))
    }

    pub fn get_vertex_group_mut(&mut self, name: &str) -> Option<&mut ElementGroupMask> {
        self.vertex_groups.get_mut(&AttributeId::from(name))
    }

    pub fn get_vertex_group(&self, name: &str) -> Option<&ElementGroupMask> {
        self.vertex_groups.get(&AttributeId::from(name))
    }

    pub fn get_primitive_group_mut(&mut self, name: &str) -> Option<&mut ElementGroupMask> {
        self.primitive_groups.get_mut(&AttributeId::from(name))
    }

    pub fn get_primitive_group(&self, name: &str) -> Option<&ElementGroupMask> {
        self.primitive_groups.get(&AttributeId::from(name))
    }
    
    // --- Edge Group Accessors (Lazy) ---

    pub fn ensure_edge_group(&mut self, name: &str) -> &mut ElementGroupMask {
        let groups = self.edge_groups.get_or_insert_with(HashMap::new);
        groups.entry(AttributeId::from(name)).or_insert_with(|| {
            let mut mask = ElementGroupMask::new(self.edges.len());
            while mask.len() < self.edges.len() { mask.push(false); }
            mask
        })
    }

    pub fn get_edge_group_mut(&mut self, name: &str) -> Option<&mut ElementGroupMask> {
        self.edge_groups.as_mut()?.get_mut(&AttributeId::from(name))
    }

    pub fn get_edge_group(&self, name: &str) -> Option<&ElementGroupMask> {
        self.edge_groups.as_ref()?.get(&AttributeId::from(name))
    }
    
    pub fn build_topology(&self) -> Arc<Topology> {
        self.get_topology()
    }

    // --- Element Management ---
    
    #[inline]
    pub(crate) fn add_point_no_invalidate(&mut self) -> PointId {
        let idx = self.points.insert(());
        let id = PointId::from(idx);
        for handle in self.point_attributes.values_mut() { handle.get_mut().push_default(); }
        for mask in self.point_groups.values_mut() { mask.push(false); }
        id
    }

    #[inline]
    pub(crate) fn add_vertex_no_invalidate(&mut self, point_id: PointId) -> VertexId {
        let idx = self.vertices.insert(GeoVertex { point_id });
        let id = VertexId::from(idx);
        for handle in self.vertex_attributes.values_mut() { handle.get_mut().push_default(); }
        for mask in self.vertex_groups.values_mut() { mask.push(false); }
        id
    }

    #[inline]
    pub(crate) fn add_primitive_no_invalidate(&mut self, primitive: GeoPrimitive) -> PrimId {
        let idx = self.primitives.insert(primitive);
        let id = PrimId::from(idx);
        for handle in self.primitive_attributes.values_mut() { handle.get_mut().push_default(); }
        for mask in self.primitive_groups.values_mut() { mask.push(false); }
        id
    }

    #[inline]
    pub fn add_points_batch(&mut self, count: usize) {
        if count == 0 { return; }
        self.invalidate_topology();
        self.points.reserve_additional(count);
        for h in self.point_attributes.values_mut() { reserve_attr_handle(h, count); }
        for m in self.point_groups.values_mut() { m.reserve_additional(count); }
        for _ in 0..count { let _ = self.add_point_no_invalidate(); }
    }

    #[inline]
    pub fn add_vertices_batch(&mut self, count: usize, point_id: PointId) {
        if count == 0 { return; }
        self.invalidate_topology();
        self.vertices.reserve_additional(count);
        for h in self.vertex_attributes.values_mut() { reserve_attr_handle(h, count); }
        for m in self.vertex_groups.values_mut() { m.reserve_additional(count); }
        for _ in 0..count { let _ = self.add_vertex_no_invalidate(point_id); }
    }

    #[inline]
    pub fn add_primitives_batch(&mut self, count: usize, primitive: &GeoPrimitive) {
        if count == 0 { return; }
        self.invalidate_topology();
        self.primitives.reserve_additional(count);
        for h in self.primitive_attributes.values_mut() { reserve_attr_handle(h, count); }
        for m in self.primitive_groups.values_mut() { m.reserve_additional(count); }
        for _ in 0..count { let _ = self.add_primitive_no_invalidate(primitive.clone()); }
    }

    pub fn add_point(&mut self) -> PointId {
        self.invalidate_topology();
        self.add_point_no_invalidate()
    }
    
    pub fn remove_point(&mut self, id: PointId) {
        self.invalidate_topology();
        if let Some(dense_idx) = self.points.get_dense_index(id.into()) {
            self.points.remove(id.into());
            for handle in self.point_attributes.values_mut() {
                handle.get_mut().swap_remove(dense_idx);
            }
            for mask in self.point_groups.values_mut() {
                mask.swap_remove(dense_idx);
            }
        }
    }
    
    pub fn add_vertex(&mut self, point_id: PointId) -> VertexId {
        self.invalidate_topology();
        self.add_vertex_no_invalidate(point_id)
    }
    
    pub fn remove_vertex(&mut self, id: VertexId) {
        self.invalidate_topology();
        if let Some(dense_idx) = self.vertices.get_dense_index(id.into()) {
            self.vertices.remove(id.into());
            for handle in self.vertex_attributes.values_mut() {
                handle.get_mut().swap_remove(dense_idx);
            }
            for mask in self.vertex_groups.values_mut() {
                mask.swap_remove(dense_idx);
            }
        }
    }

    pub fn add_primitive(&mut self, primitive: GeoPrimitive) -> PrimId {
        self.invalidate_topology();
        self.add_primitive_no_invalidate(primitive)
    }
    
    pub fn remove_primitive(&mut self, id: PrimId) {
        self.invalidate_topology();
        if let Some(dense_idx) = self.primitives.get_dense_index(id.into()) {
            self.primitives.remove(id.into());
            for handle in self.primitive_attributes.values_mut() {
                handle.get_mut().swap_remove(dense_idx);
            }
             for mask in self.primitive_groups.values_mut() {
                mask.swap_remove(dense_idx);
            }
        }
    }

    pub fn add_edge(&mut self, p0: PointId, p1: PointId) -> EdgeId {
        let idx = self.edges.insert(GeoEdge { p0, p1 });
        let id = EdgeId::from(idx);
        for handle in self.edge_attributes.values_mut() {
            handle.get_mut().push_default();
        }
        
        // Sync edge groups if they exist
        if let Some(groups) = &mut self.edge_groups {
            for mask in groups.values_mut() {
                mask.push(false);
            }
        }
        id
    }
    
    pub fn remove_edge(&mut self, id: EdgeId) {
        if let Some(dense_idx) = self.edges.get_dense_index(id.into()) {
            self.edges.remove(id.into());
            for handle in self.edge_attributes.values_mut() {
                handle.get_mut().swap_remove(dense_idx);
            }
            
            // Sync edge groups if they exist
            if let Some(groups) = &mut self.edge_groups {
                for mask in groups.values_mut() {
                    mask.swap_remove(dense_idx);
                }
            }
        }
    }

    // --- Attribute Accessors ---

    pub fn get_point_attribute(&self, name: impl Into<AttributeId>) -> Option<&Attribute> {
        self.point_attributes.get(&name.into()).map(|h| &*h.data)
    }

    pub fn get_vertex_attribute(&self, name: impl Into<AttributeId>) -> Option<&Attribute> {
        self.vertex_attributes.get(&name.into()).map(|h| &*h.data)
    }

    pub fn get_primitive_attribute(&self, name: impl Into<AttributeId>) -> Option<&Attribute> {
        self.primitive_attributes.get(&name.into()).map(|h| &*h.data)
    }

    pub fn get_point_attribute_mut(&mut self, name: impl Into<AttributeId>) -> Option<&mut Attribute> {
        self.point_attributes.get_mut(&name.into()).map(|h| h.get_mut())
    }

    pub fn get_vertex_attribute_mut(&mut self, name: impl Into<AttributeId>) -> Option<&mut Attribute> {
        self.vertex_attributes.get_mut(&name.into()).map(|h| h.get_mut())
    }

    pub fn get_primitive_attribute_mut(&mut self, name: impl Into<AttributeId>) -> Option<&mut Attribute> {
        self.primitive_attributes.get_mut(&name.into()).map(|h| h.get_mut())
    }

    pub fn get_edge_attribute(&self, name: impl Into<AttributeId>) -> Option<&Attribute> {
        self.edge_attributes.get(&name.into()).map(|h| &*h.data)
    }

    pub fn get_edge_attribute_mut(&mut self, name: impl Into<AttributeId>) -> Option<&mut Attribute> {
        self.edge_attributes.get_mut(&name.into()).map(|h| h.get_mut())
    }

    #[inline] pub fn remove_point_attribute(&mut self, name: impl Into<AttributeId>) { self.point_attributes.remove(&name.into()); }
    #[inline] pub fn remove_vertex_attribute(&mut self, name: impl Into<AttributeId>) { self.vertex_attributes.remove(&name.into()); }
    #[inline] pub fn remove_primitive_attribute(&mut self, name: impl Into<AttributeId>) { self.primitive_attributes.remove(&name.into()); }
    #[inline] pub fn remove_edge_attribute(&mut self, name: impl Into<AttributeId>) { self.edge_attributes.remove(&name.into()); }
    #[inline] pub fn remove_detail_attribute_id(&mut self, name: impl Into<AttributeId>) { self.detail_attributes.remove(&name.into()); }

    pub fn insert_edge_attribute(&mut self, name: impl Into<AttributeId>, mut attr: Attribute) {
        while attr.len() < self.edges.len() { attr.push_default(); }
        self.edge_attributes.insert(name.into(), AttributeHandle::new(attr));
    }
    
    pub fn insert_point_attribute(&mut self, name: impl Into<AttributeId>, mut attr: Attribute) {
        while attr.len() < self.points.len() { attr.push_default(); }
        self.point_attributes.insert(name.into(), AttributeHandle::new(attr));
    }

    pub fn insert_vertex_attribute(&mut self, name: impl Into<AttributeId>, mut attr: Attribute) {
        while attr.len() < self.vertices.len() { attr.push_default(); }
        self.vertex_attributes.insert(name.into(), AttributeHandle::new(attr));
    }
    
    pub fn insert_primitive_attribute(&mut self, name: impl Into<AttributeId>, mut attr: Attribute) {
        while attr.len() < self.primitives.len() { attr.push_default(); }
        self.primitive_attributes.insert(name.into(), AttributeHandle::new(attr));
    }
    
    pub fn insert_detail_attribute(&mut self, name: impl Into<AttributeId>, mut attr: Attribute) {
        while attr.len() < 1 { attr.push_default(); }
        self.detail_attributes.insert(name.into(), AttributeHandle::new(attr));
    }

    pub fn get_point_count(&self) -> usize {
        self.points.len()
    }

    pub fn get_point_position_attribute(&self) -> Option<&[Vec3]> {
        self.get_point_attribute(attrs::P)?.as_slice()
    }

    // --- Queries ---

    pub fn compute_bounds(&self) -> Option<(Vec3, Vec3)> {
        let positions = self.get_point_position_attribute()?;
        
        let mut min_v = Vec3::splat(f32::INFINITY);
        let mut max_v = Vec3::splat(f32::NEG_INFINITY);
        
        if positions.len() == 0 { return None; }
        
        for pos in positions.iter() {
            min_v = min_v.min(*pos);
            max_v = max_v.max(*pos);
        }
        
        if min_v.x.is_infinite() { None } else { Some((min_v, max_v)) }
    }

    pub fn compute_fingerprint(&self) -> GeometryFingerprint {
        let (min, max) = self.compute_bounds().unwrap_or((Vec3::ZERO, Vec3::ZERO));
        
        let pos_stats = self.get_point_attribute(attrs::P)
            .and_then(|a| a.as_slice::<Vec3>())
            .map(|d| compute_vec3_stats(&d.to_vec()));

        let norm_stats = self.get_vertex_attribute(attrs::N)
            .and_then(|a| a.as_slice::<Vec3>())
            .map(|d| compute_vec3_stats(&d.to_vec()));

        GeometryFingerprint {
            point_count: self.get_point_count(),
            primitive_count: self.primitives.len(),
            bbox_min: Some(min.to_array()),
            bbox_max: Some(max.to_array()),
            position_stats: pos_stats,
            normal_stats: norm_stats,
            color_stats: None,
            uv_stats: None,
            topology: TopologySummary {
                boundary_edge_count: 0,
                non_manifold_edge_count: 0,
                has_open_boundary: false,
            },
        }
    }

    pub fn calculate_flat_normals(&mut self) {
        // 1. Get positions (ReadOnly borrow)
        let positions = if let Some(p) = self.get_point_attribute(attrs::P) {
            if let Some(v) = p.as_slice::<Vec3>() { v } else { return; }
                } else {
            return;
        };

        // 2. Capture read-only references to arenas for parallel closure
        let primitives_arena = &self.primitives;
        let vertices_arena = &self.vertices;
        let points_arena = &self.points;

        // 3. Compute normals in parallel
        let normals: Vec<Vec3> = primitives_arena
            .par_iter()
            .map(|prim| {
                if let GeoPrimitive::Polygon(poly) = prim {
                    if poly.vertices.len() < 3 { return Vec3::Y; }

                    let get_pos = |vid: VertexId| -> Vec3 {
                        if let Some(v) = vertices_arena.get(vid.into()) {
                            if let Some(pidx) = points_arena.get_dense_index(v.point_id.into()) {
                                return positions.get(pidx).copied().unwrap_or(Vec3::ZERO);
                            }
                        }
                        Vec3::ZERO
                    };

                    let p0 = get_pos(poly.vertices[0]);
                    let p1 = get_pos(poly.vertices[1]);
                    let p2 = get_pos(poly.vertices[2]);

                    (p1 - p0).cross(p2 - p0).normalize_or_zero()
                } else {
                    Vec3::Y
                }
            })
            .collect();
        
        // 4. Write back (primitive + vertex domains)
        self.insert_primitive_attribute(attrs::N, Attribute::new(normals.clone()));
        let mut vnorm = vec![Vec3::Y; self.vertices.len()];
        for (prim_idx, prim) in self.primitives.iter().enumerate() {
            let n = *normals.get(prim_idx).unwrap_or(&Vec3::Y);
            for &vid in prim.vertices() {
                if let Some(vd) = self.vertices.get_dense_index(vid.into()) { if let Some(s) = vnorm.get_mut(vd) { *s = n; } }
            }
        }
        self.insert_vertex_attribute(attrs::N, Attribute::new(vnorm));
    }
    
    pub fn calculate_smooth_normals(&mut self) {
        self.calculate_flat_normals();
    }
    
    pub fn get_pos_by_vertex(&self, vid: VertexId, positions: &[Vec3]) -> Vec3 {
         if let Some(v) = self.vertices.get(vid.into()) {
             if let Some(pidx) = self.points.get_dense_index(v.point_id.into()) {
                 return positions.get(pidx).copied().unwrap_or(Vec3::ZERO);
             }
         }
         Vec3::ZERO
    }

    pub fn to_bevy_mesh(&self) -> Mesh {
        // Polyline/Curve path (no caching needed - rare case)
        let has_polyline = self.primitives.iter().any(|p| matches!(p, GeoPrimitive::Polyline(_)));
        let has_curve = self.primitives.iter().any(|p| matches!(p, GeoPrimitive::BezierCurve(_)));
        let has_polygon = self.primitives.iter().any(|p| matches!(p, GeoPrimitive::Polygon(_)));
        if (has_polyline || has_curve) && !has_polygon {
            return self.build_polyline_mesh();
        }
        
        // Get current attribute versions for cache validation
        let ver_p = self.point_attributes.get(&attrs::P.into()).map(|h| h.version).unwrap_or(0);
        let ver_n = self.vertex_attributes.get(&attrs::N.into()).map(|h| h.version).unwrap_or(0);
        let ver_uv = self.vertex_attributes.get(&attrs::UV.into()).map(|h| h.version)
            .or_else(|| self.point_attributes.get(&attrs::UV.into()).map(|h| h.version)).unwrap_or(0);
        let cd_id: AttributeId = attrs::CD.into();
        let cd_source = if self.detail_attributes.contains_key(&cd_id) { CdSource::Detail }
            else if self.vertex_attributes.contains_key(&cd_id) { CdSource::Vertex }
            else if self.point_attributes.contains_key(&cd_id) { CdSource::Point }
            else if self.primitive_attributes.contains_key(&cd_id) { CdSource::Primitive }
            else { CdSource::None };
        let (cd_tag, ver_cd_raw) = match cd_source {
            CdSource::None => (0, 0),
            CdSource::Detail => (1, self.detail_attributes.get(&cd_id).map(|h| h.version).unwrap_or(0)),
            CdSource::Vertex => (2, self.vertex_attributes.get(&cd_id).map(|h| h.version).unwrap_or(0)),
            CdSource::Point => (3, self.point_attributes.get(&cd_id).map(|h| h.version).unwrap_or(0)),
            CdSource::Primitive => (4, self.primitive_attributes.get(&cd_id).map(|h| h.version).unwrap_or(0)),
        };
        let ver_cd = mix_version(cd_tag, ver_cd_raw);
        let prim_count = self.primitives.len();
        
        // Check cache validity
        if let Ok(cache_guard) = self.bevy_mesh_cache.read() {
            if let Some(ref c) = *cache_guard {
                if c.version_p == ver_p && c.version_n == ver_n && c.version_uv == ver_uv && c.version_cd == ver_cd && c.prim_count == prim_count {
                    return self.build_mesh_from_cache(c);
                }
            }
        }
        
        // Cache miss: rebuild
        let (pos, nrm, uv0, cd0, idx) = self.build_mesh_data_internal(cd_source);
        
        // Store in cache
        if let Ok(mut cache_guard) = self.bevy_mesh_cache.write() {
            *cache_guard = Some(BevyMeshCache {
                positions: pos.clone(), normals: nrm.clone(), uvs: uv0.clone(), colors: cd0.clone(),
                indices: idx.clone(),
                version_p: ver_p, version_n: ver_n, version_uv: ver_uv, version_cd: ver_cd, prim_count,
            });
        }
        
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
        if let Some(uv) = uv0 { mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv); }
        if let Some(cd) = cd0 { mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, cd); }
        mesh.insert_indices(Indices::U32(idx));
        mesh
    }
    
    #[inline]
    fn build_mesh_from_cache(&self, c: &BevyMeshCache) -> Mesh {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, c.positions.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, c.normals.clone());
        if let Some(ref uv) = c.uvs { mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv.clone()); }
        if let Some(ref cd) = c.colors { mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, cd.clone()); }
        mesh.insert_indices(Indices::U32(c.indices.clone()));
        mesh
    }
    
    #[inline]
    fn build_polyline_mesh(&self) -> Mesh {
        let cap = self.vertices.len().max(self.points.len());
        let mut pos = Vec::with_capacity(cap);
        let mut nrm = Vec::with_capacity(cap);
        let p_attr = self.get_point_attribute(attrs::P);
        let p_slice = p_attr.and_then(|a| a.as_slice::<Vec3>());
        let p_paged = p_attr.and_then(|a| a.as_paged::<Vec3>());
        let get_p = |i: usize| p_slice.and_then(|s| s.get(i).copied()).or_else(|| p_paged.and_then(|pb| pb.get(i))).unwrap_or(Vec3::ZERO);
        for (_, v) in self.vertices.iter_enumerated() {
            let pd = self.points.get_dense_index(v.point_id.into()).unwrap_or(0);
            pos.push(get_p(pd).to_array());
            nrm.push(Vec3::Y.to_array());
        }
        let mut mesh = Mesh::new(PrimitiveTopology::PointList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
        mesh
    }
    
    fn build_mesh_data_internal(&self, cd_source: CdSource) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Option<Vec<[f32; 2]>>, Option<Vec<[f32; 4]>>, Vec<u32>) {
        let est_verts = self.vertices.len();
        let est_prims = self.primitives.len();
        let est_out_verts = if cd_source == CdSource::Primitive {
            self.primitives.iter().filter_map(|p| match p { GeoPrimitive::Polygon(poly) => Some(poly.vertices.len()), _ => None }).sum::<usize>()
        } else { est_verts };
        let mut pos = Vec::with_capacity(est_out_verts);
        let mut nrm = Vec::with_capacity(est_out_verts);
        let mut uv0: Option<Vec<[f32; 2]>> = None;
        let mut cd0: Option<Vec<[f32; 4]>> = None;
        let mut idx = Vec::with_capacity(est_prims * 3);
        let mut remap: Vec<u32> = vec![u32::MAX; est_verts];
        
        let p_attr = self.get_point_attribute(attrs::P);
        let n_attr = self.get_vertex_attribute(attrs::N);
        let uv_attr_v = self.get_vertex_attribute(attrs::UV);
        let uv_attr_p = self.get_point_attribute(attrs::UV);
        let cd_attr_d = if cd_source == CdSource::Detail { self.get_detail_attribute(attrs::CD) } else { None };
        let cd_attr_v = if cd_source == CdSource::Vertex { self.get_vertex_attribute(attrs::CD) } else { None };
        let cd_attr_p = if cd_source == CdSource::Point { self.get_point_attribute(attrs::CD) } else { None };
        let cd_attr_prim = if cd_source == CdSource::Primitive { self.get_primitive_attribute(attrs::CD) } else { None };
        let p_slice = p_attr.and_then(|a| a.as_slice::<Vec3>());
        let p_paged = p_attr.and_then(|a| a.as_paged::<Vec3>());
        let n_slice = n_attr.and_then(|a| a.as_slice::<Vec3>());
        let n_paged = n_attr.and_then(|a| a.as_paged::<Vec3>());
        let uv_v_slice = uv_attr_v.and_then(|a| a.as_slice::<Vec2>());
        let uv_v_paged = uv_attr_v.and_then(|a| a.as_paged::<Vec2>());
        let uv_p_slice = uv_attr_p.and_then(|a| a.as_slice::<Vec2>());
        let uv_p_paged = uv_attr_p.and_then(|a| a.as_paged::<Vec2>());
        let has_uv = uv_v_slice.is_some() || uv_v_paged.is_some() || uv_p_slice.is_some() || uv_p_paged.is_some();
        let get_cd_from_attr = |a: Option<&Attribute>, i: usize| -> Option<[f32; 4]> {
            if let Some(s) = a.and_then(|a| a.as_slice::<Vec4>()).and_then(|s| s.get(i)).copied() { return Some([s.x, s.y, s.z, s.w]); }
            if let Some(s) = a.and_then(|a| a.as_paged::<Vec4>()).and_then(|pb| pb.get(i)) { return Some([s.x, s.y, s.z, s.w]); }
            if let Some(s) = a.and_then(|a| a.as_slice::<Vec3>()).and_then(|s| s.get(i)).copied() { return Some([s.x, s.y, s.z, 1.0]); }
            if let Some(s) = a.and_then(|a| a.as_paged::<Vec3>()).and_then(|pb| pb.get(i)) { return Some([s.x, s.y, s.z, 1.0]); }
            None
        };
        let cd_detail = if cd_source == CdSource::Detail { get_cd_from_attr(cd_attr_d, 0).unwrap_or([1.0, 1.0, 1.0, 1.0]) } else { [1.0, 1.0, 1.0, 1.0] };
        let has_cd = cd_source != CdSource::None;
        
        let get_p = |i: usize| p_slice.and_then(|s| s.get(i).copied()).or_else(|| p_paged.and_then(|pb| pb.get(i))).unwrap_or(Vec3::ZERO);
        let get_n = |i: usize| n_slice.and_then(|s| s.get(i).copied()).or_else(|| n_paged.and_then(|pb| pb.get(i))).unwrap_or(Vec3::Y);
        let get_uv_v = |i: usize| uv_v_slice.and_then(|s| s.get(i).copied()).or_else(|| uv_v_paged.and_then(|pb| pb.get(i)));
        let get_uv_p = |i: usize| uv_p_slice.and_then(|s| s.get(i).copied()).or_else(|| uv_p_paged.and_then(|pb| pb.get(i)));
        let get_cd = |v_dense: usize, p_dense: usize, prim_dense: usize| -> [f32; 4] {
            match cd_source {
                CdSource::None => [1.0, 1.0, 1.0, 1.0],
                CdSource::Detail => cd_detail,
                CdSource::Vertex => get_cd_from_attr(cd_attr_v, v_dense).unwrap_or([1.0, 1.0, 1.0, 1.0]),
                CdSource::Point => get_cd_from_attr(cd_attr_p, p_dense).unwrap_or([1.0, 1.0, 1.0, 1.0]),
                CdSource::Primitive => get_cd_from_attr(cd_attr_prim, prim_dense).unwrap_or([1.0, 1.0, 1.0, 1.0]),
            }
        };

        let share_vertices = cd_source != CdSource::Primitive;
        for (prim_i, prim) in self.primitives.iter().enumerate() {
            let GeoPrimitive::Polygon(poly) = prim else { continue; };
            let mut poly_idx = Vec::with_capacity(poly.vertices.len());
            for &v_id in &poly.vertices {
                let Some(v_dense) = self.vertices.get_dense_index(v_id.into()) else { continue; };
                let r = if share_vertices && remap[v_dense] != u32::MAX { remap[v_dense] } else {
                    let r = pos.len() as u32;
                    if share_vertices { remap[v_dense] = r; }
                    let v = self.vertices.get(v_id.into()).unwrap();
                    let pd = self.points.get_dense_index(v.point_id.into()).unwrap_or(0);
                    pos.push(get_p(pd).to_array());
                    nrm.push(get_n(v_dense).normalize_or_zero().to_array());
                    if has_uv {
                        let uv = get_uv_v(v_dense).or_else(|| get_uv_p(pd)).unwrap_or(Vec2::ZERO);
                        uv0.get_or_insert_with(|| Vec::with_capacity(est_out_verts)).push(uv.to_array());
                    }
                    if has_cd {
                        cd0.get_or_insert_with(|| Vec::with_capacity(est_out_verts)).push(get_cd(v_dense, pd, prim_i));
                    }
                    r
                };
                poly_idx.push(r);
            }
            if poly_idx.len() >= 3 {
                let v0 = poly_idx[0];
                for i in 1..poly_idx.len() - 1 { idx.extend_from_slice(&[v0, poly_idx[i], poly_idx[i + 1]]); }
            }
        }
        (pos, nrm, uv0, cd0, idx)
    }

    // --- Interpolation ---

    pub fn add_point_from_mix(&mut self, p0_dense_idx: usize, p1_dense_idx: usize, t: f32) -> PointId {
        let new_id = self.add_point();
        let target_idx = self.points.len() - 1; 
        
        // Helper macro to handle known types via trait
        macro_rules! try_mix {
            ($attr:expr, $t:ty) => {
                if let Some(buf) = $attr.as_mut_slice::<$t>() {
                    let v0 = *buf.get(p0_dense_idx).unwrap_or(&<$t>::default());
                    let v1 = *buf.get(p1_dense_idx).unwrap_or(&<$t>::default());
                    if let Some(val) = buf.get_mut(target_idx) { 
                        *val = <$t>::mix(&v0, &v1, t); 
                    }
                continue;
                }
            }
        }

        let keys: Vec<_> = self.point_attributes.keys().copied().collect();
        for key in keys {
            if let Some(attr_handle) = self.point_attributes.get_mut(&key) {
                let attr = attr_handle.get_mut();
                
                try_mix!(attr, f32);
                try_mix!(attr, f64);
                try_mix!(attr, Vec2);
                try_mix!(attr, Vec3);
                try_mix!(attr, Vec4);
                try_mix!(attr, DVec2);
                try_mix!(attr, DVec3);
                try_mix!(attr, DVec4);
                try_mix!(attr, Quat);
                
                // For integers/bools, mix does nearest neighbor
                try_mix!(attr, i32);
                try_mix!(attr, bool);
            }
        }
        new_id
    }

    pub fn add_vertex_from_mix(&mut self, v0_dense_idx: usize, v1_dense_idx: usize, t: f32, target_point: PointId) -> VertexId {
        let new_id = self.add_vertex(target_point);
        let target_idx = self.vertices.len() - 1; 

        macro_rules! try_mix {
            ($attr:expr, $t:ty) => {
                if let Some(buf) = $attr.as_mut_slice::<$t>() {
                    let v0 = *buf.get(v0_dense_idx).unwrap_or(&<$t>::default());
                    let v1 = *buf.get(v1_dense_idx).unwrap_or(&<$t>::default());
                    if let Some(val) = buf.get_mut(target_idx) { 
                        *val = <$t>::mix(&v0, &v1, t); 
                    }
                continue;
                }
            }
        }

        let keys: Vec<_> = self.vertex_attributes.keys().copied().collect();
        for key in keys {
            if let Some(attr_handle) = self.vertex_attributes.get_mut(&key) {
                let attr = attr_handle.get_mut();
                
                try_mix!(attr, f32);
                try_mix!(attr, f64);
                try_mix!(attr, Vec2);
                try_mix!(attr, Vec3);
                try_mix!(attr, Vec4);
                try_mix!(attr, DVec2);
                try_mix!(attr, DVec3);
                try_mix!(attr, DVec4);
                try_mix!(attr, Quat);
                try_mix!(attr, i32);
                try_mix!(attr, bool);
            }
        }
        new_id
    }
    
    // --- Group Promotion (Basic) ---
    
    pub fn promote_point_to_vertex_group(&self, point_group: &ElementGroupMask) -> ElementGroupMask {
        let mut vertex_group = ElementGroupMask::new(self.vertices.len());
        for (v_idx, v) in self.vertices.iter().enumerate() {
            if let Some(p_idx) = self.points.get_dense_index(v.point_id.into()) {
                if point_group.get(p_idx) {
                    vertex_group.set(v_idx, true);
                }
            }
        }
        vertex_group
    }

    pub fn promote_vertex_to_point_group(&self, vertex_group: &ElementGroupMask) -> ElementGroupMask {
        let mut point_group = ElementGroupMask::new(self.points.len());
        for (v_idx, v) in self.vertices.iter().enumerate() {
            if vertex_group.get(v_idx) {
                if let Some(p_idx) = self.points.get_dense_index(v.point_id.into()) {
                    point_group.set(p_idx, true);
                }
            }
        }
        point_group
    }

    pub fn promote_primitive_to_point_group(&self, prim_group: &ElementGroupMask) -> ElementGroupMask {
        let mut point_group = ElementGroupMask::new(self.points.len());
        for (prim_idx, prim) in self.primitives.iter().enumerate() {
            if prim_group.get(prim_idx) {
                for &vid in prim.vertices() {
                    if let Some(v) = self.vertices.get(vid.into()) {
                        if let Some(pidx) = self.points.get_dense_index(v.point_id.into()) {
                            point_group.set(pidx, true);
                        }
                    }
                }
            }
        }
        point_group
    }

    pub fn promote_point_to_primitive_group(&self, point_group: &ElementGroupMask, mode: GroupPromoteMode) -> ElementGroupMask {
        let mut prim_group = ElementGroupMask::new(self.primitives.len());
        for (prim_idx, prim) in self.primitives.iter().enumerate() {
            let mut match_count = 0;
            let mut total_points = 0;
            for &vid in prim.vertices() {
                if let Some(v) = self.vertices.get(vid.into()) {
                    total_points += 1;
                    if let Some(pidx) = self.points.get_dense_index(v.point_id.into()) {
                        if point_group.get(pidx) { match_count += 1; }
                    }
                }
            }
            let selected = match mode {
                GroupPromoteMode::Any => match_count > 0,
                GroupPromoteMode::All => match_count > 0 && match_count == total_points,
            };
            if selected { prim_group.set(prim_idx, true); }
        }
        prim_group
    }
    
    pub fn compute_wireframe_indices(&self) -> Vec<u32> {
        let prim_count = self.primitives.len();
        
        // Check cache
        if let Ok(cache_guard) = self.wireframe_cache.read() {
            if let Some(ref c) = *cache_guard {
                if c.prim_count == prim_count { return c.indices.clone(); }
            }
        }
        
        // Cache miss: rebuild
        let est_edges = prim_count * 4;
        let mut out = Vec::with_capacity(est_edges * 2);
        let mut edges: HashSet<u64> = HashSet::with_capacity(est_edges);
        let mut remap: Vec<u32> = vec![u32::MAX; self.vertices.len()];
        let mut next = 0u32;
        
        for prim in self.primitives.iter() {
            let GeoPrimitive::Polygon(poly) = prim else { continue; };
            let mut poly_idx = Vec::with_capacity(poly.vertices.len());
            for &v_id in &poly.vertices {
                let Some(v_dense) = self.vertices.get_dense_index(v_id.into()) else { continue; };
                let r = if remap[v_dense] != u32::MAX { remap[v_dense] } else { let r = next; next += 1; remap[v_dense] = r; r };
                poly_idx.push(r);
            }
            let n = poly_idx.len();
            if n < 2 { continue; }
            for i in 0..n {
                let a = poly_idx[i];
                let b = poly_idx[(i + 1) % n];
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                if edges.insert(((lo as u64) << 32) | (hi as u64)) { out.extend_from_slice(&[lo, hi]); }
            }
        }
        
        // Store in cache
        if let Ok(mut cache_guard) = self.wireframe_cache.write() {
            *cache_guard = Some(WireframeCache { indices: out.clone(), prim_count });
        }
        out
    }

    pub fn compute_polyline_indices(&self) -> Vec<u32> {
        let mut out = Vec::new();
        let mut remap: HashMap<u32, u32> = HashMap::new();
        let mut next = 0u32;
        for prim in self.primitives.iter() {
            let (verts, closed) = match prim { GeoPrimitive::Polyline(l) => (&l.vertices, l.closed), GeoPrimitive::BezierCurve(c) => (&c.vertices, c.closed), _ => continue };
            let mut vs = Vec::with_capacity(verts.len());
            for &v_id in verts {
                let Some(v_dense) = self.vertices.get_dense_index(v_id.into()) else { continue; };
                let r = *remap.entry(v_dense as u32).or_insert_with(|| { let r = next; next += 1; r });
                vs.push(r);
            }
            if vs.len() < 2 { continue; }
            for i in 0..vs.len() - 1 { out.extend_from_slice(&[vs[i], vs[i + 1]]); }
            if closed { out.extend_from_slice(&[*vs.last().unwrap(), vs[0]]); }
        }
        out
    }
    pub fn build_edge_to_primitive_map(&self) -> HashMap<(PointId, PointId), Vec<PrimId>> { HashMap::new() }
    pub fn update_bevy_mesh(&self, mesh: &mut Mesh) { *mesh = self.to_bevy_mesh(); }
}

fn compute_vec3_stats(data: &Vec<Vec3>) -> Vec3Stats {
    let mut min_v = Vec3::splat(f32::INFINITY);
    let mut max_v = Vec3::splat(f32::NEG_INFINITY);
    let mut sum = Vec3::ZERO;
    
    for v in data.iter() {
        min_v = min_v.min(*v);
        max_v = max_v.max(*v);
        sum += *v;
    }
    
    let count = data.len() as f32;
    let avg = if count > 0.0 { sum / count } else { Vec3::ZERO };
    
    Vec3Stats { min: min_v.to_array(), max: max_v.to_array(), avg: avg.to_array() }
}

pub fn create_point_cloud_mesh(points: &[Vec3]) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::PointList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, points.to_vec());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![Vec3::Y; points.len()]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![Vec2::ZERO; points.len()]);
    mesh
}
