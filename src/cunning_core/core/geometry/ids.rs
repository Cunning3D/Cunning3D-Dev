use ustr::Ustr;
use std::marker::PhantomData;
use serde::{Serialize, Deserialize};
use crate::libs::geometry::sparse_set::ArenaIndex;

/// A lightweight handle to an interned attribute name string.
/// This allows O(1) comparison and hashing instead of O(N) string operations.
/// It wraps a `ustr::Ustr`, which is a global string interner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AttributeId(#[serde(with = "attribute_id_serde")] Ustr);

impl AttributeId {
    /// Create a new AttributeId from a string slice.
    pub fn new(name: &str) -> Self {
        Self(Ustr::from(name))
    }

    /// Get the underlying string slice.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Display for AttributeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<&str> for AttributeId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for AttributeId {
    fn from(s: String) -> Self {
        Self::new(&s)
    }
}

// Custom Serde implementation to serialize as String, not internal ID
mod attribute_id_serde {
    use super::*;
    use serde::{Serializer, Deserializer};

    pub fn serialize<S>(id: &Ustr, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(id.as_str())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Ustr, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Ustr::from(&s))
    }
}

// --- Generational ID System ---
// This replaces the old simple index-based ID system to support safe deletion and O(1) checks.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GenerationalId<Tag> {
    pub index: u32,
    pub generation: u32,
    #[serde(skip)]
    pub _marker: PhantomData<Tag>,
}

impl<Tag> GenerationalId<Tag> {
    pub const INVALID: Self = Self { 
        index: u32::MAX, 
        generation: u32::MAX, 
        _marker: PhantomData 
    };

    #[inline(always)]
    pub fn from_raw(index: u32, generation: u32) -> Self {
        Self { index, generation, _marker: PhantomData }
    }

    #[inline(always)]
    pub fn is_valid(&self) -> bool {
        self.index != u32::MAX
    }

    #[inline(always)]
    pub fn index(&self) -> usize {
        self.index as usize
    }
}

// Bridge between ArenaIndex and Typed GenerationalId
impl<Tag> From<ArenaIndex> for GenerationalId<Tag> {
    fn from(idx: ArenaIndex) -> Self {
        Self::from_raw(idx.index, idx.generation)
    }
}

impl<Tag> From<GenerationalId<Tag>> for ArenaIndex {
    fn from(id: GenerationalId<Tag>) -> Self {
        ArenaIndex { index: id.index, generation: id.generation }
    }
}

// --- Legacy / Raw Index Support ---
// Used for internal array indexing where generation checks are already passed or unnecessary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ElementIndex<Tag>(pub u32, #[serde(skip)] pub PhantomData<Tag>);

impl<Tag> ElementIndex<Tag> {
    #[inline(always)]
    pub fn new(index: u32) -> Self {
        Self(index, PhantomData)
    }

    #[inline(always)]
    pub fn index(&self) -> usize {
        self.0 as usize
    }
}

// --- Tags for Element Types ---
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PointTag;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct VertexTag;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PrimTag;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EdgeTag;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HalfEdgeTag;

// --- Type Aliases (Upgraded to GenerationalId) ---
pub type PointId = GenerationalId<PointTag>;
pub type VertexId = GenerationalId<VertexTag>;
pub type PrimId = GenerationalId<PrimTag>;
pub type EdgeId = GenerationalId<EdgeTag>;
pub type HalfEdgeId = GenerationalId<HalfEdgeTag>;

// --- Raw Index Aliases ---
pub type PointIndex = ElementIndex<PointTag>;
pub type VertexIndex = ElementIndex<VertexTag>;
pub type PrimIndex = ElementIndex<PrimTag>;
pub type EdgeIndex = ElementIndex<EdgeTag>;
pub type HalfEdgeIndex = ElementIndex<HalfEdgeTag>;

// --- Attribute Domains ---
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AttributeDomain {
    Point,
    Vertex,
    Primitive,
    Edge,
    Detail,
}
