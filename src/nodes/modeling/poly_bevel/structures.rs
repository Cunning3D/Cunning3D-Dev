use super::pipeline_v2::custom_profile::CustomProfile;
use crate::libs::geometry::ids::{EdgeId, HalfEdgeId, PointId, PrimId};
use bevy::prelude::*;

/// Object pool Workspace: prevents repetitive memory allocation, improves performance by 50%+
#[derive(Default)]
pub struct BevelWorkspace {
    pub out_points: Vec<Vec3>,
    pub out_polys: Vec<Vec<usize>>,
    // Dense index vec: corner_pt[dense_idx] = point index or u32::MAX if none
    pub corner_pt: Vec<u32>,
    pub face_normals_dense: Vec<Vec3>,
    pub bev_edge: Vec<bool>,
    pub any_bev_point: Vec<bool>,
    pub dense_to_pid: Vec<PointId>,
    pub dense_to_he: Vec<HalfEdgeId>,
    pub vmesh_buffer: Vec<Vec3>, // VMesh temporary buffer
}

impl BevelWorkspace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-allocate capacity based on input geometry
    pub fn prepare(
        &mut self,
        n_points: usize,
        n_prims: usize,
        n_halfedges: usize,
        n_graph_edges: usize,
    ) {
        let est_pts = n_points * 3; // bevel typically expands 2-4x
        let est_polys = n_prims * 3;

        self.out_points.clear();
        self.out_points
            .reserve(est_pts.saturating_sub(self.out_points.capacity()));
        self.out_polys.clear();
        self.out_polys
            .reserve(est_polys.saturating_sub(self.out_polys.capacity()));

        // Dense index vec for corner_pt: O(1) access via HalfEdgeId dense index
        self.corner_pt.clear();
        self.corner_pt.resize(n_halfedges, u32::MAX);

        // Reset vecs but keep capacity
        self.face_normals_dense.clear();
        self.face_normals_dense.resize(n_prims, Vec3::Y);
        self.bev_edge.clear();
        self.bev_edge.resize(n_halfedges, false);
        self.any_bev_point.clear();
        self.any_bev_point.resize(n_points, false);
        self.dense_to_pid.clear();
        self.dense_to_pid.resize(n_points, PointId::INVALID);
        self.dense_to_he.clear();
        self.dense_to_he.resize(n_halfedges, HalfEdgeId::INVALID);
        self.vmesh_buffer.clear();
        self.vmesh_buffer.reserve(n_graph_edges * 64); // estimate 64 pts per edge
    }

    /// Reset but keep capacity (for next bevel)
    pub fn reset(&mut self) {
        self.out_points.clear();
        self.out_polys.clear();
        self.corner_pt.fill(u32::MAX);
        self.face_normals_dense.fill(Vec3::Y);
        self.bev_edge.fill(false);
        self.any_bev_point.fill(false);
        self.vmesh_buffer.clear();
    }

    /// Set corner point index for a half-edge (O(1) via dense index)
    #[inline]
    pub fn set_corner_pt(&mut self, dense_idx: usize, pt_idx: usize) {
        if dense_idx < self.corner_pt.len() {
            self.corner_pt[dense_idx] = pt_idx as u32;
        }
    }

    /// Get corner point index for a half-edge (O(1) via dense index), returns None if not set
    #[inline]
    pub fn get_corner_pt(&self, dense_idx: usize) -> Option<usize> {
        self.corner_pt.get(dense_idx).and_then(|&v| {
            if v == u32::MAX {
                None
            } else {
                Some(v as usize)
            }
        })
    }
}

/// Affect type: edges or vertices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BevelAffect {
    #[default]
    Edges = 0,
    Vertices = 1,
}

/// Offset calculation method.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OffsetType {
    #[default]
    Offset = 0,
    Width = 1,
    Depth = 2,
    Percent = 3,
    Absolute = 4,
}

/// Profile shape type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProfileShape {
    #[default]
    Superellipse = 0,
    Custom = 1,
}

/// Miter type for outer/inner angles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MiterType {
    #[default]
    Sharp = 0,
    Patch = 1,
    Arc = 2,
}

/// VMesh generation method.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VMeshMethod {
    #[default]
    GridFill = 0,
    Cutoff = 1,
}

/// Face strength mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FaceStrength {
    #[default]
    None = 0,
    New = 1,
    Affected = 2,
    All = 3,
}

/// Complete bevel parameters matching Blender's BevelParams.
#[derive(Clone, Debug)]
pub struct BevelParams {
    // Main
    pub affect: BevelAffect,
    pub offset_type: OffsetType,
    pub offset: f32,
    pub seg: usize,
    // Profile
    pub profile_shape: ProfileShape,
    pub profile_type: i32,   // 0=Circle, 1=SquareIn, 2=SquareOut
    pub profile_amount: f32, // 0.0-1.0 superellipse control
    pub pro_super_r: f32,    // Computed from profile_type
    pub custom_profile: Option<CustomProfile>,
    // Miter
    pub miter_outer: MiterType,
    pub miter_inner: MiterType,
    pub spread: f32,
    // Geometry
    pub clamp_overlap: bool,
    pub loop_slide: bool,
    // Weights
    pub use_weights: bool,
    pub vertex_group: String,
    pub bweight_offset_vert: bool,
    pub bweight_offset_edge: bool,
    // Shading
    pub harden_normals: bool,
    pub mark_seam: bool,
    pub mark_sharp: bool,
    // Intersection
    pub vmesh_method: VMeshMethod,
    pub face_strength: FaceStrength,
    pub material: i32,
    // Debug
    pub invert_profile: bool,
    pub corner_scale: f32,
}

impl Default for BevelParams {
    fn default() -> Self {
        Self {
            affect: BevelAffect::Edges,
            offset_type: OffsetType::Offset,
            offset: 0.1,
            seg: 1,
            profile_shape: ProfileShape::Superellipse,
            profile_type: 0,
            profile_amount: 0.5,
            pro_super_r: 0.0, // Circle
            custom_profile: None,
            miter_outer: MiterType::Sharp,
            miter_inner: MiterType::Sharp,
            spread: 0.1,
            clamp_overlap: true,
            loop_slide: true,
            use_weights: false,
            vertex_group: String::new(),
            bweight_offset_vert: false,
            bweight_offset_edge: false,
            harden_normals: false,
            mark_seam: false,
            mark_sharp: false,
            vmesh_method: VMeshMethod::GridFill,
            face_strength: FaceStrength::None,
            material: -1,
            invert_profile: false,
            corner_scale: 1.0,
        }
    }
}

impl BevelParams {
    /// Create from node parameter values.
    pub fn from_node_params(
        affect: i32,
        offset_type: i32,
        distance: f32,
        divisions: usize,
        profile_type: i32,
        profile_amount: f32,
        custom_profile: String,
        miter_outer: i32,
        miter_inner: i32,
        spread: f32,
        clamp_overlap: bool,
        loop_slide: bool,
        use_weights: bool,
        vertex_group: String,
        bweight_offset_vert: bool,
        bweight_offset_edge: bool,
        harden_normals: bool,
        mark_seam: bool,
        mark_sharp: bool,
        vmesh_method: i32,
        face_strength: i32,
        material: i32,
        invert_profile: bool,
        corner_scale: f32,
    ) -> Self {
        use super::pipeline_v2::math::{PRO_CIRCLE_R, PRO_SQUARE_IN_R, PRO_SQUARE_R};

        // Blender style: profile_type=0 is Superellipse (slider), profile_type=1 is Custom
        // Slider: 0.0=SquareIn, 0.5=Circle, 1.0=SquareOut
        let (profile_shape, custom_profile_obj, pro_super_r) = if profile_type == 1 {
            let s = custom_profile.trim();
            let cp = if s.is_empty() {
                None
            } else {
                parse_custom_profile(s)
            };
            (ProfileShape::Custom, cp, PRO_CIRCLE_R)
        } else {
            // Superellipse: interpolate based on profile_amount (0.0-1.0)
            let amt = profile_amount.clamp(0.0, 1.0);
            let r = if amt <= 0.001 {
                PRO_SQUARE_IN_R
            } else if amt >= 0.999 {
                PRO_SQUARE_R
            } else if amt < 0.5 {
                let t = amt * 2.0;
                PRO_SQUARE_IN_R * (1.0 - t) + PRO_CIRCLE_R * t
            } else {
                let t = (amt - 0.5) * 2.0;
                let (lc, ls) = (PRO_CIRCLE_R.ln(), PRO_SQUARE_R.ln());
                (lc + (ls - lc) * t).exp()
            };
            (ProfileShape::Superellipse, None, r)
        };

        Self {
            affect: if affect == 1 {
                BevelAffect::Vertices
            } else {
                BevelAffect::Edges
            },
            offset_type: match offset_type {
                1 => OffsetType::Width,
                2 => OffsetType::Depth,
                3 => OffsetType::Percent,
                4 => OffsetType::Absolute,
                _ => OffsetType::Offset,
            },
            offset: distance,
            seg: divisions.max(1),
            profile_shape,
            profile_type,
            profile_amount,
            pro_super_r,
            custom_profile: custom_profile_obj,
            miter_outer: match miter_outer {
                1 => MiterType::Patch,
                2 => MiterType::Arc,
                _ => MiterType::Sharp,
            },
            miter_inner: match miter_inner {
                1 => MiterType::Arc,
                _ => MiterType::Sharp,
            },
            spread,
            clamp_overlap,
            loop_slide,
            use_weights,
            vertex_group,
            bweight_offset_vert,
            bweight_offset_edge,
            harden_normals,
            mark_seam,
            mark_sharp,
            vmesh_method: if vmesh_method == 1 {
                VMeshMethod::Cutoff
            } else {
                VMeshMethod::GridFill
            },
            face_strength: match face_strength {
                1 => FaceStrength::New,
                2 => FaceStrength::Affected,
                3 => FaceStrength::All,
                _ => FaceStrength::None,
            },
            material,
            invert_profile,
            corner_scale,
        }
    }
}

fn parse_custom_profile(s: &str) -> Option<CustomProfile> {
    let mut v = Vec::new();
    for p in s.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let mut it = p.split(',').map(str::trim);
        let (Some(x), Some(y)) = (it.next(), it.next()) else {
            continue;
        };
        if let (Ok(x), Ok(y)) = (x.parse::<f32>(), y.parse::<f32>()) {
            v.push(Vec2::new(x, y));
        }
    }
    if v.len() < 2 {
        None
    } else {
        Some(CustomProfile::new(v))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum MeshKind {
    #[default]
    None,
    Poly,
    Adj, // Grid fill
    TriFan,
    Cutoff,
}

#[derive(Clone)]
pub struct BevVert {
    pub p_id: PointId, // Original point
    pub edge_count: usize,
    pub edges: Vec<usize>, // Indices into global EdgeHalf list (spokes)
    pub vmesh: Option<VMesh>,
    pub offset: f32, // For vertex-only bevel
}

#[derive(Clone)]
pub struct EdgeHalf {
    pub e_id: EdgeId,           // Original edge
    pub he_id: HalfEdgeId,      // Original half-edge
    pub pair_index: usize,      // Index of other EdgeHalf in global list
    pub next_index: usize,      // Next around BevVert (in spoke list)
    pub prev_index: usize,      // Prev around BevVert
    pub origin_bev_vert: usize, // Index of BevVert

    pub offset_l: f32,
    pub offset_r: f32,
    pub seg: usize,
    pub is_bev: bool,
    pub is_seam: bool,

    // Boundary verts indices in BevelGraph.bound_verts
    pub left_v: Option<usize>,
    pub right_v: Option<usize>,
}

#[derive(Clone, Default)]
pub struct ProfileSpacing {
    pub xvals: Vec<f64>,
    pub yvals: Vec<f64>,
    pub xvals_2: Vec<f64>,
    pub yvals_2: Vec<f64>,
    pub seg_2: usize,
    pub fullness: f32,
}

#[derive(Clone)]
pub struct Profile {
    pub super_r: f32,
    pub start: Vec3,
    pub middle: Vec3,
    pub end: Vec3,
    pub plane_no: Vec3,
    pub plane_co: Vec3,
    pub proj_dir: Vec3,
    pub height: f32,
    pub prof_co: Vec<Vec3>,
    pub prof_co_2: Vec<Vec3>,
    pub ids: Vec<usize>,
    pub face_start: PrimId,
    pub face_end: PrimId,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            super_r: 0.0,
            start: Vec3::ZERO,
            middle: Vec3::ZERO,
            end: Vec3::ZERO,
            plane_no: Vec3::ZERO,
            plane_co: Vec3::ZERO,
            proj_dir: Vec3::ZERO,
            height: 0.0,
            prof_co: Vec::new(),
            prof_co_2: Vec::new(),
            ids: Vec::new(),
            face_start: PrimId::INVALID,
            face_end: PrimId::INVALID,
        }
    }
}

#[derive(Clone)]
pub struct BoundVert {
    pub pos: Vec3,
    pub next: usize, // Index in linked list
    pub prev: usize,
    pub index: usize, // Index in the VMesh boundary loop
    pub is_arc_start: bool,
    pub is_patch_start: bool,
    pub efirst: Option<usize>, // Index into graph.edges (global)
    pub elast: Option<usize>,  // Index into graph.edges (global)
    pub eon: Option<usize>,    // Index into graph.edges (global)
    pub sinratio: f32,
    pub profile: Profile,
    pub e_idx: Option<usize>, // Index into graph.edges
}

#[derive(Clone)]
pub struct VMesh {
    pub bound_start: usize, // Index of first BoundVert in BevelGraph.bound_verts
    pub count: usize,       // Number of boundary verts
    pub seg: usize,
    pub kind: MeshKind,
    pub mesh_verts: Vec<Vec3>, // New internal vertices created for this corner
}

#[derive(Default)]
pub struct BevelGraph {
    pub verts: Vec<BevVert>,
    pub edges: Vec<EdgeHalf>, // All EdgeHalves (2 per beveled edge, 1 per non-beveled spoke)
    pub bound_verts: Vec<BoundVert>,

    // SoA (Structure of Arrays) for hot edge fields - cache-friendly parallel access
    pub edge_offset_l: Vec<f32>,
    pub edge_offset_r: Vec<f32>,
    pub edge_is_bev: Vec<bool>,
    pub edge_seg: Vec<usize>,
    pub edge_he_id: Vec<HalfEdgeId>,
    pub edge_pair_idx: Vec<usize>,
    pub edge_prev_idx: Vec<usize>,
    pub edge_next_idx: Vec<usize>,
    pub edge_origin_vert: Vec<usize>,

    // SoA for hot vert fields
    pub vert_p_id: Vec<PointId>,
    pub vert_offset: Vec<f32>,
}

impl BevelGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-allocate capacity, reducing realloc
    pub fn with_capacity(verts: usize, edges: usize) -> Self {
        Self {
            verts: Vec::with_capacity(verts),
            edges: Vec::with_capacity(edges),
            bound_verts: Vec::with_capacity(verts * 4),
            edge_offset_l: Vec::with_capacity(edges),
            edge_offset_r: Vec::with_capacity(edges),
            edge_is_bev: Vec::with_capacity(edges),
            edge_seg: Vec::with_capacity(edges),
            edge_he_id: Vec::with_capacity(edges),
            edge_pair_idx: Vec::with_capacity(edges),
            edge_prev_idx: Vec::with_capacity(edges),
            edge_next_idx: Vec::with_capacity(edges),
            edge_origin_vert: Vec::with_capacity(edges),
            vert_p_id: Vec::with_capacity(verts),
            vert_offset: Vec::with_capacity(verts),
        }
    }

    /// Sync SoA arrays from AoS edges (call after building edges)
    pub fn sync_edge_soa(&mut self) {
        let n = self.edges.len();
        self.edge_offset_l.clear();
        self.edge_offset_l.reserve(n);
        self.edge_offset_r.clear();
        self.edge_offset_r.reserve(n);
        self.edge_is_bev.clear();
        self.edge_is_bev.reserve(n);
        self.edge_seg.clear();
        self.edge_seg.reserve(n);
        self.edge_he_id.clear();
        self.edge_he_id.reserve(n);
        self.edge_pair_idx.clear();
        self.edge_pair_idx.reserve(n);
        self.edge_prev_idx.clear();
        self.edge_prev_idx.reserve(n);
        self.edge_next_idx.clear();
        self.edge_next_idx.reserve(n);
        self.edge_origin_vert.clear();
        self.edge_origin_vert.reserve(n);
        for e in &self.edges {
            self.edge_offset_l.push(e.offset_l);
            self.edge_offset_r.push(e.offset_r);
            self.edge_is_bev.push(e.is_bev);
            self.edge_seg.push(e.seg);
            self.edge_he_id.push(e.he_id);
            self.edge_pair_idx.push(e.pair_index);
            self.edge_prev_idx.push(e.prev_index);
            self.edge_next_idx.push(e.next_index);
            self.edge_origin_vert.push(e.origin_bev_vert);
        }
    }

    /// Sync SoA arrays from AoS verts (call after building verts)
    pub fn sync_vert_soa(&mut self) {
        let n = self.verts.len();
        self.vert_p_id.clear();
        self.vert_p_id.reserve(n);
        self.vert_offset.clear();
        self.vert_offset.reserve(n);
        for v in &self.verts {
            self.vert_p_id.push(v.p_id);
            self.vert_offset.push(v.offset);
        }
    }

    /// Sync all SoA arrays
    #[inline]
    pub fn sync_soa(&mut self) {
        self.sync_edge_soa();
        self.sync_vert_soa();
    }

    /// Fast O(1) access to edge bevel status
    #[inline]
    pub fn is_edge_bev(&self, idx: usize) -> bool {
        self.edge_is_bev.get(idx).copied().unwrap_or(false)
    }

    /// Fast O(1) access to edge offset
    #[inline]
    pub fn get_edge_offset_l(&self, idx: usize) -> f32 {
        self.edge_offset_l.get(idx).copied().unwrap_or(0.0)
    }

    /// Fast O(1) access to edge half-edge ID
    #[inline]
    pub fn get_edge_he_id(&self, idx: usize) -> HalfEdgeId {
        self.edge_he_id
            .get(idx)
            .copied()
            .unwrap_or(HalfEdgeId::INVALID)
    }
}
