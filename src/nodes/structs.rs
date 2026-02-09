use crate::cunning_core::cda::utils::apply_channel_value;
use crate::cunning_core::graph::dirty_tracker::DirtyTracker;
use crate::cunning_core::profiling::ComputeRecord;
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::cunning_core::traits::node_interface::NodeParameters;
use crate::libs::geometry::geo_ref::{ForEachMeta, GeometryRef, GeometryView};
use crate::libs::geometry::group::ElementGroupMask;
use crate::mesh::{Attribute, Geometry};
use crate::nodes::attribute::attribute_promote::AttributePromoteParams;
use crate::nodes::parameter::{Parameter, ParameterValue};
use crate::nodes::port_key;
use bevy::prelude::Resource;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::hash_set::HashSet as StdHashSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Instant;
use ustr::Ustr;
use uuid::Uuid;

struct ParamIndex<'a>(HashMap<&'a str, &'a ParameterValue>);
impl<'a> ParamIndex<'a> {
    #[inline]
    fn new(ps: &'a [Parameter]) -> Self {
        let mut m = HashMap::with_capacity(ps.len());
        for p in ps {
            m.insert(p.name.as_str(), &p.value);
        }
        Self(m)
    }
    #[inline]
    fn int(&self, n: &str, d: i32) -> i32 {
        self.0
            .get(n)
            .and_then(|v| {
                if let ParameterValue::Int(x) = *v {
                    Some(*x)
                } else {
                    None
                }
            })
            .unwrap_or(d)
    }
    #[inline]
    fn bool(&self, n: &str, d: bool) -> bool {
        self.0
            .get(n)
            .and_then(|v| {
                if let ParameterValue::Bool(x) = *v {
                    Some(*x)
                } else {
                    None
                }
            })
            .unwrap_or(d)
    }
    #[inline]
    fn str(&self, n: &str, d: &str) -> String {
        self.0
            .get(n)
            .and_then(|v| {
                if let ParameterValue::String(x) = *v {
                    Some(x.clone())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| d.to_string())
    }
}

pub type NodeId = Uuid;
pub type ConnectionId = Uuid;
pub type NetworkBoxId = Uuid;
pub type StickyNoteId = Uuid;
pub type PromoteNoteId = Uuid;
pub type PortId = Ustr;
pub type NodeParamOverrides = HashMap<NodeId, Vec<(String, Option<usize>, f64)>>;

#[derive(Clone)]
pub(crate) enum GeoCacheRef {
    Geo(Arc<Geometry>),
    View(Arc<GeometryView>),
    Gpu(crate::nodes::gpu::runtime::GpuGeoHandle),
}

impl std::fmt::Debug for GeoCacheRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Geo(_) => "Geo",
            Self::View(_) => "View",
            Self::Gpu(_) => "Gpu",
        })
    }
}

impl GeoCacheRef {
    #[inline]
    pub(crate) fn empty() -> Self {
        static EMPTY: OnceLock<Arc<Geometry>> = OnceLock::new();
        Self::Geo(EMPTY.get_or_init(|| Arc::new(Geometry::new())).clone())
    }
    #[inline]
    pub(crate) fn as_georef(&self) -> Arc<dyn GeometryRef> {
        self.as_geo() as Arc<dyn GeometryRef>
    }
    #[inline]
    pub(crate) fn as_geo(&self) -> Arc<Geometry> {
        match self {
            Self::Geo(g) => g.clone(),
            Self::View(v) => v.materialize_arc(),
            Self::Gpu(h) => h.download_blocking(),
        }
    }
    #[inline]
    pub(crate) fn as_gpu(&self) -> Option<crate::nodes::gpu::runtime::GpuGeoHandle> {
        if let Self::Gpu(h) = self {
            Some(h.clone())
        } else {
            None
        }
    }
    #[inline]
    pub(crate) fn is_empty_geo(&self) -> bool {
        match self {
            Self::Geo(g) => {
                g.get_point_count() == 0
                    && g.primitives().is_empty()
                    && g.vertices().is_empty()
                    && g.edges().is_empty()
            }
            Self::View(v) => {
                v.point_len() == 0 && v.prim_len() == 0 && v.vertex_len() == 0 && v.edge_len() == 0
            }
            Self::Gpu(h) => {
                h.cpu_base().get_point_count() == 0
                    && h.cpu_base().primitives().is_empty()
                    && h.cpu_base().vertices().is_empty()
                    && h.cpu_base().edges().is_empty()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GeoLevel {
    Point,
    Vertex,
    Primitive,
    Detail,
}

impl GeoLevel {
    pub fn from_i32(i: i32) -> Option<Self> {
        match i {
            0 => Some(Self::Point),
            1 => Some(Self::Vertex),
            2 => Some(Self::Primitive),
            3 => Some(Self::Detail),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputStyle {
    Individual,
    Collection,
    Bar, // Legacy support
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStyle {
    Normal,
    Large,
    Layered, // Legacy support
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NodeType {
    CreateCube,
    CreateSphere,
    Transform,
    Merge,
    Spline,
    AttributePromote(AttributePromoteParams),
    FbxImporter,
    VdbFromPolygons,
    VdbToPolygons,
    VoxelEdit,
    GroupCreate,
    GroupCombine,
    GroupPromote,
    GroupManage,
    GroupNormalize,
    Boolean,
    PolyExtrude,
    PolyBevel,
    Fuse,
    Generic(String),
    CDA(CDANodeData),  // CDA asset node
    CDAInput(String),  // CDA internal input node
    CDAOutput(String), // CDA internal output node
}

/// CDA node data: Embedded CDA asset + runtime parameter overrides
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CDANodeData {
    pub asset_ref: crate::cunning_core::cda::CdaAssetRef,
    #[serde(default)]
    pub name: String,
    // Coverlay runtime state (per instance): HUD is single-select, Coverlay is multi-select.
    #[serde(default)]
    pub coverlay_hud: Option<NodeId>,
    #[serde(default)]
    pub coverlay_units: Vec<NodeId>,
    // Future: per-instance internal node parameter overrides (e.g. spline snapshots) for exposed units.
    #[serde(default)]
    pub inner_param_overrides:
        HashMap<NodeId, HashMap<String, crate::nodes::parameter::ParameterValue>>,
}

impl NodeType {
    pub fn name(&self) -> &str {
        match self {
            NodeType::CreateCube => "Create Cube",
            NodeType::CreateSphere => "Create Sphere",
            NodeType::Transform => "Transform",
            NodeType::Merge => "Merge",
            NodeType::Spline => "Spline",
            NodeType::AttributePromote(_) => "Attribute Promote",
            NodeType::FbxImporter => "FBX Import",
            NodeType::VdbFromPolygons => "VDB From Polygons",
            NodeType::VdbToPolygons => "VDB To Polygons",
            NodeType::VoxelEdit => "Voxel Edit",
            NodeType::GroupCreate => "Group Create",
            NodeType::GroupCombine => "Group Combine",
            NodeType::GroupPromote => "Group Promote",
            NodeType::GroupManage => "Group Manage",
            NodeType::GroupNormalize => "Group Normalize",
            NodeType::Boolean => "Boolean",
            NodeType::PolyExtrude => "Poly Extrude",
            NodeType::PolyBevel => "Poly Bevel",
            NodeType::Fuse => "Fuse",
            NodeType::Generic(s) => s,
            NodeType::CDA(data) => &data.name,
            NodeType::CDAInput(name) => name,
            NodeType::CDAOutput(name) => name,
        }
    }

    /// Stable internal node type id for serialization/runtime (never use `name()` for this).
    pub fn type_id(&self) -> &str {
        match self {
            NodeType::CreateCube => "cunning.basic.create_cube",
            NodeType::CreateSphere => "cunning.basic.create_sphere",
            NodeType::Transform => "cunning.basic.transform",
            NodeType::Merge => "cunning.utility.merge",
            NodeType::Spline => "cunning.spline.unity",
            NodeType::AttributePromote(_) => "cunning.attribute.promote",
            NodeType::FbxImporter => "cunning.io.fbx_import",
            NodeType::VdbFromPolygons => "cunning.vdb.from_polygons",
            NodeType::VdbToPolygons => "cunning.vdb.to_polygons",
            NodeType::VoxelEdit => "cunning.voxel.edit",
            NodeType::GroupCreate => "cunning.group.create",
            NodeType::GroupCombine => "cunning.group.combine",
            NodeType::GroupPromote => "cunning.group.promote",
            NodeType::GroupManage => "cunning.group.manage",
            NodeType::GroupNormalize => "cunning.group.normalize",
            NodeType::Boolean => "cunning.modeling.boolean",
            NodeType::PolyExtrude => "cunning.modeling.poly_extrude",
            NodeType::PolyBevel => "cunning.modeling.poly_bevel",
            NodeType::Fuse => "cunning.modeling.fuse",
            // Generic nodes are registry-driven; map known UI names to stable ids for runtime export.
            NodeType::Generic(s) => match s.as_str() {
                "Boolean" => "cunning.modeling.boolean",
                "PolyExtrude" | "Poly Extrude" => "cunning.modeling.poly_extrude",
                "PolyBevel" | "Poly Bevel" => "cunning.modeling.poly_bevel",
                "Merge" => "cunning.utility.merge",
                "Fuse" => "cunning.modeling.fuse",
                "Voxel Edit" | "VoxelEdit" => "cunning.voxel.edit",
                _ => s,
            },
            // CDA instance node stores asset ref; serialized as cda://<uuid> elsewhere.
            NodeType::CDA(_) => "cunning.cda.instance",
            // CDA internal IO nodes
            NodeType::CDAInput(_) => "cunning.input",
            NodeType::CDAOutput(_) => "cunning.output",
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Connection {
    pub id: ConnectionId,
    pub from_node: NodeId,
    pub from_port: PortId,
    pub to_node: NodeId,
    pub to_port: PortId,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub waypoints: Vec<bevy_egui::egui::Pos2>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct NetworkBox {
    pub id: NetworkBoxId,
    pub rect: bevy_egui::egui::Rect,
    pub title: String,
    pub color: bevy_egui::egui::Color32,
    pub nodes_inside: HashSet<NodeId>,
    pub stickies_inside: HashSet<StickyNoteId>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct StickyNote {
    pub id: StickyNoteId,
    pub rect: bevy_egui::egui::Rect,
    pub title: String,
    pub content: String,
    pub color: bevy_egui::egui::Color32,
}

/// PromoteNote: voice/text input note for Copilot intent injection, auto-destroys 10s after use
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PromoteNote {
    pub id: PromoteNoteId,
    pub rect: bevy_egui::egui::Rect,
    pub content: String,
    pub color: bevy_egui::egui::Color32,
    #[serde(skip)]
    pub used_at: Option<std::time::Instant>,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    pub node_type: NodeType,
    pub position: bevy_egui::egui::Pos2,
    pub size: bevy_egui::egui::Vec2,
    pub parameters: Vec<Parameter>,
    pub is_template: bool,
    pub inputs: HashMap<PortId, ()>, // Only keys matter for interactions
    pub outputs: HashMap<PortId, ()>,

    // Legacy Fields for UI
    pub is_bypassed: bool,
    pub is_display_node: bool,
    pub is_locked: bool,
    pub input_style: InputStyle, // Assuming legacy default
    pub style: NodeStyle,
}

impl Node {
    pub fn new(
        id: NodeId,
        name: String,
        mut node_type: NodeType,
        position: bevy_egui::egui::Pos2,
    ) -> Self {
        // Patch: Convert Generic("Boolean") to typed NodeType::Boolean
        // This ensures it picks up the correct parameters and ports defined in structs.rs logic
        if let NodeType::Generic(ref s) = node_type {
            if s == "Boolean" {
                node_type = NodeType::Boolean;
            }
        }

        let parameters = match &node_type {
            NodeType::CreateCube => {
                crate::nodes::basic::create_cube::CreateCubeNode::define_parameters()
            }
            NodeType::CreateSphere => {
                crate::nodes::basic::create_sphere::CreateSphereNode::define_parameters()
            }
            NodeType::Generic(_) => Vec::new(),
            NodeType::Transform => {
                crate::nodes::basic_interaction::xform::TransformNode::define_parameters()
            }
            NodeType::Merge => crate::nodes::utility::merge::MergeNode::define_parameters(),
            NodeType::Spline => {
                crate::nodes::spline::spline_node::UnitySplineNode::define_parameters()
            }
            NodeType::AttributePromote(_) => {
                crate::nodes::attribute::attribute_promote::AttributePromoteNode::define_parameters(
                )
            }
            NodeType::FbxImporter => {
                crate::nodes::io::fbx_importer::FbxImporterNode::define_parameters()
            }
            NodeType::VdbFromPolygons => {
                crate::nodes::vdb::vdb_from_mesh::VdbFromPolygonsNode::define_parameters()
            }
            NodeType::VdbToPolygons => {
                crate::nodes::vdb::vdb_to_mesh::VdbToPolygonsNode::define_parameters()
            }
            NodeType::VoxelEdit => {
                crate::nodes::voxel::voxel_edit::VoxelEditNode::define_parameters()
            }
            NodeType::GroupCreate => {
                crate::nodes::group::group_create::GroupCreateNode::define_parameters()
            }
            NodeType::GroupCombine => {
                crate::nodes::group::group_combine::GroupCombineNode::define_parameters()
            }
            NodeType::GroupPromote => {
                crate::nodes::group::group_promote::GroupPromoteNode::define_parameters()
            }
            NodeType::GroupManage => {
                crate::nodes::group::group_manage::GroupManageNode::define_parameters()
            }
            NodeType::GroupNormalize => {
                crate::nodes::group::group_normalize::GroupNormalizeNode::define_parameters()
            }
            NodeType::Boolean => {
                crate::nodes::modeling::boolean::boolean_node::BooleanNode::define_parameters()
            }
            NodeType::PolyExtrude => {
                crate::nodes::modeling::poly_extrude::PolyExtrudeNode::define_parameters()
            }
            NodeType::PolyBevel => {
                crate::nodes::modeling::poly_bevel::PolyBevelNode::define_parameters()
            }
            NodeType::Fuse => crate::nodes::modeling::fuse_node::FuseNode::define_parameters(),
            NodeType::CDA(_) => Vec::new(),
            NodeType::CDAInput(_) => vec![],
            NodeType::CDAOutput(_) => vec![],
        };

        // Default ports (stable keys)
        let mut inputs = HashMap::new();
        let mut outputs = HashMap::new();
        inputs.insert(port_key::in0(), ());
        outputs.insert(port_key::out0(), ());

        let (input_style, style) = if matches!(node_type, NodeType::Merge) {
            (InputStyle::Bar, NodeStyle::Layered)
        } else {
            (InputStyle::Individual, NodeStyle::Normal)
        };

        let mut node = Self {
            id,
            name,
            node_type,
            position,
            size: bevy_egui::egui::Vec2::new(140.0, 60.0),
            parameters,
            is_template: false,
            inputs,
            outputs,
            is_bypassed: false,
            is_display_node: false,
            is_locked: false,
            input_style,
            style,
        };
        node.rebuild_ports();
        node
    }

    pub fn rebuild_ports(&mut self) {
        // Normalize UI style based on node type (important for multi-connect like Merge).
        if matches!(self.node_type, NodeType::Merge) {
            self.input_style = InputStyle::Bar;
            self.style = NodeStyle::Layered;
        } else if matches!(self.node_type, NodeType::CDA(_)) {
            self.input_style = InputStyle::Individual;
            self.style = NodeStyle::Normal;
        }
        // Default ports
        self.inputs.clear();
        self.outputs.clear();

        match &self.node_type {
            NodeType::CDA(data) => {
                if let Some(lib) = crate::cunning_core::cda::library::global_cda_library() {
                    if let Some(a) = lib.get(data.asset_ref.uuid) {
                        for input in &a.inputs {
                            self.inputs.insert(input.port_key(), ());
                        }
                        for output in &a.outputs {
                            self.outputs.insert(output.port_key(), ());
                        }
                        return;
                    }
                }
                self.inputs.insert(port_key::in0(), ());
                self.outputs.insert(port_key::out0(), ());
            }
            NodeType::CDAInput(_) => {
                self.outputs.insert(port_key::out0(), ());
            }
            NodeType::CDAOutput(_) => {
                self.inputs.insert(port_key::in0(), ());
            }
            NodeType::Boolean => {
                self.inputs.insert(port_key::in_a(), ());
                self.inputs.insert(port_key::in_b(), ());
                self.outputs.insert(port_key::out0(), ());
            }
            NodeType::GroupCreate => {
                self.inputs.insert(port_key::in0(), ());
                self.inputs.insert(port_key::in1(), ());
                self.outputs.insert(port_key::out0(), ());
            }
            NodeType::Generic(s) if s == "Group Create" => {
                self.inputs.insert(port_key::in0(), ());
                self.inputs.insert(port_key::in1(), ());
                self.outputs.insert(port_key::out0(), ());
            }
            NodeType::Generic(s) if s == "ForEach Begin" => {
                self.inputs.insert(port_key::in0(), ());
                self.outputs.insert(port_key::out0(), ());
            }
            NodeType::Generic(s) if s == "ForEach End" => {
                self.inputs.insert(port_key::in0(), ());
                self.inputs.insert(port_key::in1(), ());
                self.outputs.insert(port_key::out0(), ());
            }
            NodeType::Generic(s) if s == "ForEach Meta" => {
                self.outputs.insert(port_key::out0(), ());
            }
            NodeType::Generic(s) if s == crate::nodes::ai_texture::NODE_NANO_HEIGHTMAP => {
                self.outputs.insert(port_key::out0(), ());
            }
            _ => {
                self.inputs.insert(port_key::in0(), ());
                self.outputs.insert(port_key::out0(), ());
            }
        }
    }

    pub fn rebuild_parameters(&mut self) {
        if let NodeType::CDA(data) = &self.node_type {
            let new_params = crate::cunning_core::cda::library::global_cda_library()
                .and_then(|lib| lib.get(data.asset_ref.uuid))
                .map(|a| crate::nodes::cda::cda_node::build_cda_parameters(&a))
                .unwrap_or_default();

            // Map existing values by name to preserve user settings
            let mut existing_values: std::collections::HashMap<
                String,
                crate::nodes::parameter::ParameterValue,
            > = std::collections::HashMap::new();
            for p in &self.parameters {
                existing_values.insert(p.name.clone(), p.value.clone());
            }

            self.parameters = new_params;

            // Restore values
            for p in &mut self.parameters {
                if let Some(val) = existing_values.get(&p.name) {
                    p.value = val.clone();
                }
            }
        }
    }
}

#[derive(Resource)]
pub struct NodeGraphResource(pub NodeGraph);

impl Default for NodeGraphResource {
    fn default() -> Self {
        Self(NodeGraph::default())
    }
}

#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeGraph {
    pub nodes: HashMap<NodeId, Node>,
    pub connections: HashMap<ConnectionId, Connection>,
    #[serde(skip)]
    pub final_geometry: Arc<Geometry>,
    #[serde(skip)]
    pub geometry_cache: HashMap<NodeId, Arc<Geometry>>,
    #[serde(skip)]
    pub(crate) geometry_cache_lru: VecDeque<NodeId>,
    /// "Previous" geometry cache: kept even when a node is marked dirty, so some nodes
    /// (e.g. interactive ones like `VoxelEdit`) can do incremental updates instead of
    /// replaying all history from scratch.
    #[serde(skip)]
    pub(crate) prev_geometry_cache: HashMap<NodeId, Arc<Geometry>>,
    #[serde(skip)]
    pub(crate) prev_geometry_cache_lru: VecDeque<NodeId>,
    #[serde(skip)]
    pub(crate) port_geometry_cache: HashMap<(NodeId, PortId), Arc<Geometry>>,
    #[serde(skip)]
    pub(crate) port_ref_cache: HashMap<(NodeId, PortId), GeoCacheRef>,
    #[serde(skip)]
    pub(crate) foreach_piece_cache: HashMap<NodeId, (u64, Vec<GeoCacheRef>)>,
    #[serde(skip)]
    pub(crate) foreach_block_cache: HashMap<NodeId, (u64, Arc<Geometry>, Arc<Geometry>)>,
    #[serde(skip)]
    pub(crate) foreach_block_cache_ref: HashMap<NodeId, (u64, GeoCacheRef, GeoCacheRef)>,
    #[serde(skip)]
    pub(crate) foreach_compiled_cache: HashMap<
        NodeId,
        (
            u64,
            u64,
            Arc<crate::nodes::runtime::compiled_block::CompiledBlock>,
        ),
    >,
    #[serde(skip)]
    pub(crate) foreach_externals_cache: HashMap<NodeId, (u64, u64, Arc<Vec<GeoCacheRef>>)>,
    #[serde(skip)]
    pub(crate) foreach_reach_cache: HashMap<NodeId, (u64, Arc<std::collections::HashSet<NodeId>>)>,
    #[serde(skip)]
    pub(crate) graph_revision: u64,
    #[serde(skip)]
    pub(crate) param_revision: u64,
    #[serde(skip)]
    pub(crate) block_id_index: HashMap<String, (Option<NodeId>, Option<NodeId>, Option<NodeId>)>,
    #[serde(skip)]
    pub(crate) foreach_scope_nodes: HashSet<NodeId>,
    #[serde(skip)]
    pub(crate) foreach_scope_epoch: u64,
    #[serde(skip)]
    pub(crate) foreach_geo_epoch: HashMap<NodeId, u64>,
    #[serde(skip)]
    pub(crate) foreach_port_epoch: HashMap<(NodeId, PortId), u64>,
    #[serde(skip)]
    pub(crate) foreach_port_geo_epoch: HashMap<(NodeId, PortId), u64>,
    #[serde(skip)]
    pub dirty_tracker: DirtyTracker,
    #[serde(skip)]
    pub(crate) adjacency_out: Option<HashMap<NodeId, Vec<NodeId>>>, // O(1) lookup cache
    /// Optional shared cook-visualization state (authoritative during async cook).
    #[serde(skip)]
    pub cook_viz: Option<std::sync::Arc<crate::nodes::runtime::cook::CookVizShared>>,
    pub sticky_notes: HashMap<StickyNoteId, StickyNote>,
    pub sticky_note_draw_order: Vec<StickyNoteId>,
    pub network_boxes: HashMap<NetworkBoxId, NetworkBox>,
    pub network_box_draw_order: Vec<NetworkBoxId>,
    pub promote_notes: HashMap<PromoteNoteId, PromoteNote>,
    pub promote_note_draw_order: Vec<PromoteNoteId>,
    pub display_node: Option<NodeId>,
}

impl Clone for NodeGraph {
    fn clone(&self) -> Self {
        Self {
            nodes: self.nodes.clone(),
            connections: self.connections.clone(),
            final_geometry: self.final_geometry.clone(),
            geometry_cache: HashMap::new(),
            geometry_cache_lru: VecDeque::new(),
            prev_geometry_cache: HashMap::new(),
            prev_geometry_cache_lru: VecDeque::new(),
            port_geometry_cache: HashMap::new(),
            port_ref_cache: HashMap::new(),
            foreach_piece_cache: HashMap::new(),
            foreach_block_cache: HashMap::new(),
            foreach_block_cache_ref: HashMap::new(),
            foreach_compiled_cache: HashMap::new(),
            foreach_externals_cache: HashMap::new(),
            foreach_reach_cache: HashMap::new(),
            graph_revision: self.graph_revision,
            param_revision: self.param_revision,
            block_id_index: self.block_id_index.clone(),
            foreach_scope_nodes: HashSet::new(),
            foreach_scope_epoch: 0,
            foreach_geo_epoch: HashMap::new(),
            foreach_port_epoch: HashMap::new(),
            foreach_port_geo_epoch: HashMap::new(),
            dirty_tracker: self.dirty_tracker.clone(),
            adjacency_out: None, // Rebuild on demand
            cook_viz: self.cook_viz.clone(),
            sticky_notes: self.sticky_notes.clone(),
            sticky_note_draw_order: self.sticky_note_draw_order.clone(),
            network_boxes: self.network_boxes.clone(),
            network_box_draw_order: self.network_box_draw_order.clone(),
            promote_notes: self.promote_notes.clone(),
            promote_note_draw_order: self.promote_note_draw_order.clone(),
            display_node: self.display_node,
        }
    }
}

impl NodeGraph {
    pub fn new() -> Self {
        Self::default()
    }

    const GEOMETRY_CACHE_CAP: usize = 64;
    const PREV_GEOMETRY_CACHE_CAP: usize = 16;

    #[inline]
    fn geo_cache_touch(&mut self, id: NodeId) {
        if let Some(p) = self.geometry_cache_lru.iter().position(|x| *x == id) {
            self.geometry_cache_lru.remove(p);
        }
        self.geometry_cache_lru.push_back(id);
        while self.geometry_cache_lru.len() > Self::GEOMETRY_CACHE_CAP {
            if let Some(ev) = self.geometry_cache_lru.pop_front() {
                self.geometry_cache.remove(&ev);
            }
        }
    }

    #[inline]
    fn geo_cache_get(&mut self, id: NodeId) -> Option<Arc<Geometry>> {
        if self.foreach_scope_nodes.contains(&id) {
            if self.foreach_geo_epoch.get(&id) != Some(&self.foreach_scope_epoch) {
                return None;
            }
        }
        let v = self.geometry_cache.get(&id).cloned();
        if v.is_some() {
            self.geo_cache_touch(id);
        }
        v
    }

    #[inline]
    fn geo_cache_put(&mut self, id: NodeId, g: Arc<Geometry>) {
        self.geometry_cache.insert(id, g);
        if self.foreach_scope_nodes.contains(&id) {
            self.foreach_geo_epoch.insert(id, self.foreach_scope_epoch);
        }
        self.geo_cache_touch(id);
    }

    #[inline]
    fn geo_cache_remove(&mut self, id: NodeId) {
        self.geometry_cache.remove(&id);
        if let Some(p) = self.geometry_cache_lru.iter().position(|x| *x == id) {
            self.geometry_cache_lru.remove(p);
        }
    }

    #[inline]
    fn geo_cache_clear(&mut self) {
        self.geometry_cache.clear();
        self.geometry_cache_lru.clear();
        self.prev_geometry_cache.clear();
        self.prev_geometry_cache_lru.clear();
    }

    #[inline]
    fn prev_geo_cache_touch(&mut self, id: NodeId) {
        if let Some(p) = self.prev_geometry_cache_lru.iter().position(|x| *x == id) {
            self.prev_geometry_cache_lru.remove(p);
        }
        self.prev_geometry_cache_lru.push_back(id);
        while self.prev_geometry_cache_lru.len() > Self::PREV_GEOMETRY_CACHE_CAP {
            if let Some(ev) = self.prev_geometry_cache_lru.pop_front() {
                self.prev_geometry_cache.remove(&ev);
            }
        }
    }

    #[inline]
    fn prev_geo_cache_get(&mut self, id: NodeId) -> Option<Arc<Geometry>> {
        let v = self.prev_geometry_cache.get(&id).cloned();
        if v.is_some() {
            self.prev_geo_cache_touch(id);
        }
        v
    }

    #[inline]
    fn prev_geo_cache_put(&mut self, id: NodeId, g: Arc<Geometry>) {
        self.prev_geometry_cache.insert(id, g);
        self.prev_geo_cache_touch(id);
    }

    /// Invalidate adjacency cache when connections change
    #[inline]
    pub fn invalidate_adjacency(&mut self) {
        self.adjacency_out = None;
        self.foreach_reach_cache.clear();
        self.foreach_externals_cache.clear();
        self.graph_revision = self.graph_revision.wrapping_add(1);
    }

    #[inline]
    pub fn rebuild_block_id_index(&mut self) {
        self.block_id_index.clear();
        for (id, n) in &self.nodes {
            let is_foreach = matches!(&n.node_type, NodeType::Generic(s) if s == "ForEach Begin" || s == "ForEach End" || s == "ForEach Meta");
            if !is_foreach {
                continue;
            }
            let mut keys: Vec<String> = Vec::new();
            for name in ["block_uid", "block_id"] {
                if let Some(s) = n.parameters.iter().find(|p| p.name == name).and_then(|p| {
                    if let ParameterValue::String(v) = &p.value {
                        Some(v.as_str())
                    } else {
                        None
                    }
                }) {
                    let k = s.trim();
                    if !k.is_empty() {
                        keys.push(k.to_string());
                    }
                }
            }
            for k in keys {
                let entry = self.block_id_index.entry(k).or_insert((None, None, None));
                match &n.node_type {
                    NodeType::Generic(s) if s == "ForEach Begin" => {
                        if entry.0.is_none() {
                            entry.0 = Some(*id);
                        }
                    }
                    NodeType::Generic(s) if s == "ForEach End" => {
                        if entry.1.is_none() {
                            entry.1 = Some(*id);
                        }
                    }
                    NodeType::Generic(s) if s == "ForEach Meta" => {
                        if entry.2.is_none() {
                            entry.2 = Some(*id);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    #[inline]
    pub fn bump_param_revision(&mut self) {
        self.param_revision = self.param_revision.wrapping_add(1);
    }

    #[inline]
    pub fn foreach_scope_begin(&mut self, inside: &HashSet<NodeId>) {
        self.foreach_scope_nodes = inside.clone();
        self.foreach_scope_epoch = self.foreach_scope_epoch.wrapping_add(1);
    }

    #[inline]
    pub fn foreach_scope_bump(&mut self) {
        self.foreach_scope_epoch = self.foreach_scope_epoch.wrapping_add(1);
    }

    #[inline]
    pub fn foreach_scope_end(&mut self) {
        self.foreach_scope_nodes.clear();
    }

    /// Build adjacency list O(E), then propagate O(N) - total O(E+N) vs old O(E*N)
    pub fn mark_dirty(&mut self, node_id: NodeId) {
        let adj = self.adjacency_out.get_or_insert_with(|| {
            let mut m: HashMap<NodeId, Vec<NodeId>> =
                HashMap::with_capacity(self.connections.len());
            for c in self.connections.values() {
                m.entry(c.from_node).or_default().push(c.to_node);
            }
            m
        });
        // Take ownership temporarily to avoid borrow conflict
        let adj_ref = std::mem::take(adj);
        self.dirty_tracker.propagate_dirty_fast(node_id, &adj_ref);
        self.adjacency_out = Some(adj_ref);
    }

    /// Ensure this graph has a reasonable display node.
    /// Returns true if it changed `display_node`.
    pub fn ensure_display_node_default(&mut self) -> bool {
        if self.display_node.is_some() {
            return false;
        }
        // Prefer CDA definition outputs (semantic "final result" for a CDA inner graph).
        let mut cda_outs: Vec<(String, NodeId)> = self
            .nodes
            .iter()
            .filter_map(|(id, n)| {
                if matches!(n.node_type, NodeType::CDAOutput(_)) {
                    Some((n.name.clone(), *id))
                } else {
                    None
                }
            })
            .collect();
        if !cda_outs.is_empty() {
            cda_outs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            self.display_node = Some(cda_outs[0].1);
            return true;
        }
        // Otherwise, pick a deterministic sink node (no outgoing connections).
        let mut has_out: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
        for c in self.connections.values() {
            has_out.insert(c.from_node);
        }
        let mut sinks: Vec<NodeId> = self
            .nodes
            .keys()
            .filter(|id| !has_out.contains(id))
            .copied()
            .collect();
        if !sinks.is_empty() {
            sinks.sort();
            self.display_node = Some(sinks[0]);
            return true;
        }
        // Fallback: first node by id (deterministic).
        let mut ids: Vec<NodeId> = self.nodes.keys().copied().collect();
        ids.sort();
        self.display_node = ids.first().copied();
        self.display_node.is_some()
    }

    pub fn compute(
        &mut self,
        targets: &HashSet<NodeId>,
        registry: &NodeRegistry,
        perf_stats: Option<&mut HashMap<NodeId, ComputeRecord>>,
    ) {
        self.compute_ex(targets, registry, perf_stats, None);
    }

    pub fn compute_with_overrides(
        &mut self,
        targets: &HashSet<NodeId>,
        registry: &NodeRegistry,
        perf_stats: Option<&mut HashMap<NodeId, ComputeRecord>>,
        overrides: &NodeParamOverrides,
    ) {
        self.compute_ex(targets, registry, perf_stats, Some(overrides));
    }

    fn compute_ex(
        &mut self,
        targets: &HashSet<NodeId>,
        registry: &NodeRegistry,
        mut perf_stats: Option<&mut HashMap<NodeId, ComputeRecord>>,
        overrides: Option<&NodeParamOverrides>,
    ) {
        // INCREMENTAL COMPUTE:
        // 1. Invalidate cache for dirty nodes
        let dirty: Vec<NodeId> = self.dirty_tracker.dirty_nodes.iter().copied().collect();
        for dirty_id in dirty {
            // Preserve "previous output" for nodes that can update incrementally.
            // This lets them avoid O(history) replays when their own parameters change every frame.
            if matches!(
                self.nodes.get(&dirty_id).map(|n| &n.node_type),
                Some(NodeType::VoxelEdit)
            ) {
                if let Some(prev) = self.geometry_cache.get(&dirty_id).cloned() {
                    self.prev_geo_cache_put(dirty_id, prev);
                }
            }
            self.geo_cache_remove(dirty_id);
            self.port_geometry_cache
                .retain(|(nid, _), _| nid != &dirty_id);
            self.port_ref_cache.retain(|(nid, _), _| nid != &dirty_id);
            self.foreach_geo_epoch.remove(&dirty_id);
            self.foreach_port_epoch
                .retain(|(nid, _), _| nid != &dirty_id);
            self.foreach_port_geo_epoch
                .retain(|(nid, _), _| nid != &dirty_id);
            self.foreach_piece_cache.remove(&dirty_id);
            self.foreach_block_cache.remove(&dirty_id);
            self.foreach_block_cache_ref.remove(&dirty_id);
            self.foreach_compiled_cache.remove(&dirty_id);
            self.foreach_externals_cache.remove(&dirty_id);
            self.foreach_reach_cache.remove(&dirty_id);
        }

        // 2. Clear dirty tracker (we've handled them by removing from cache)
        self.dirty_tracker.clear();

        // Clear previous perf stats if monitor is active
        if let Some(stats) = &mut perf_stats {
            stats.clear();
        }

        let mut display_cooked = false;
        let mut visiting: StdHashSet<NodeId> = StdHashSet::new();

        for node_id in targets {
            let geometry = self.compute_node(
                *node_id,
                registry,
                &mut perf_stats,
                overrides,
                &mut visiting,
            );
            if Some(*node_id) == self.display_node {
                self.final_geometry = geometry.clone();
                display_cooked = true;
            }
        }

        match self.display_node {
            Some(display_id) if !display_cooked => {
                let geometry = self.compute_node(
                    display_id,
                    registry,
                    &mut perf_stats,
                    overrides,
                    &mut visiting,
                );
                self.final_geometry = geometry;
            }
            None if !display_cooked => {
                self.final_geometry = Arc::new(Geometry::new());
            }
            _ => {}
        }
    }

    #[inline]
    fn p_str(params: &[Parameter], n: &str, d: &str) -> String {
        params
            .iter()
            .find(|p| p.name == n)
            .and_then(|p| {
                if let ParameterValue::String(s) = &p.value {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| d.to_string())
    }
    #[inline]
    fn p_int(params: &[Parameter], n: &str, d: i32) -> i32 {
        params
            .iter()
            .find(|p| p.name == n)
            .and_then(|p| {
                if let ParameterValue::Int(v) = &p.value {
                    Some(*v)
                } else {
                    None
                }
            })
            .unwrap_or(d)
    }
    #[inline]
    fn p_bool(params: &[Parameter], n: &str, d: bool) -> bool {
        params
            .iter()
            .find(|p| p.name == n)
            .and_then(|p| {
                if let ParameterValue::Bool(v) = &p.value {
                    Some(*v)
                } else {
                    None
                }
            })
            .unwrap_or(d)
    }

    fn first_src(&self, to: NodeId, to_port: &PortId) -> Option<(NodeId, PortId)> {
        let mut srcs: Vec<(ConnectionId, NodeId, PortId)> = self
            .connections
            .values()
            .filter(|c| c.to_node == to && c.to_port.as_str() == to_port.as_str())
            .map(|c| (c.id, c.from_node, c.from_port.clone()))
            .collect();
        srcs.sort_by(|a, b| a.0.cmp(&b.0));
        srcs.into_iter().next().map(|(_, n, p)| (n, p))
    }

    #[inline]
    fn geo_fingerprint_hash(g: &Geometry) -> u64 {
        let fp = g.compute_fingerprint();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        fp.point_count.hash(&mut h);
        fp.primitive_count.hash(&mut h);
        if let Some(m) = fp.bbox_min {
            for v in m {
                v.to_bits().hash(&mut h);
            }
        }
        if let Some(m) = fp.bbox_max {
            for v in m {
                v.to_bits().hash(&mut h);
            }
        }
        h.finish()
    }

    #[inline]
    fn params_hash(params: &[Parameter]) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for p in params {
            p.name.hash(&mut h);
            match &p.value {
                ParameterValue::Int(v) => v.hash(&mut h),
                ParameterValue::Bool(v) => v.hash(&mut h),
                ParameterValue::Float(v) => v.to_bits().hash(&mut h),
                ParameterValue::String(s) => s.hash(&mut h),
                _ => {}
            }
        }
        h.finish()
    }

    #[inline]
    fn piece_key_hash(base_dirty: u64, params: &[Parameter]) -> u64 {
        let pi = ParamIndex::new(params);
        let domain = pi.int("piece_domain", 0);
        let method = pi.int("iteration_method", 0);
        let attr = pi.str("piece_attribute", "class");
        let count = pi.int("count", 1);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        base_dirty.hash(&mut h);
        domain.hash(&mut h);
        method.hash(&mut h);
        attr.hash(&mut h);
        count.hash(&mut h);
        h.finish()
    }

    fn plan_pieces(&self, base: Arc<Geometry>, params: &[Parameter]) -> Vec<GeoCacheRef> {
        let pi = ParamIndex::new(params);
        let domain = pi.int("piece_domain", 0);
        let method = pi.int("iteration_method", 0);
        let use_attr = pi.bool("use_piece_attribute", true);
        let mut attr = pi.str("piece_attribute", "class");
        let a_norm = attr.trim().trim_start_matches('@');
        let by_index = if domain == 1 { "ptnum" } else { "primnum" };
        if !use_attr || a_norm.is_empty() {
            attr = by_index.into();
        } else if !matches!(a_norm, "ptnum" | "pointnum" | "primnum" | "primitivenum") {
            let ok = if domain == 1 {
                base.get_point_attribute(a_norm).is_some()
            } else {
                base.get_primitive_attribute(a_norm).is_some()
            };
            if !ok {
                attr = by_index.into();
            }
        }
        let count = pi.int("count", 1).max(1) as usize;
        use crate::libs::algorithms::algorithms_runtime::foreach_pieces::{
            plan_foreach_pieces, ForeachPiecePlanItem as I, ForeachPiecePlanParams as P,
        };
        plan_foreach_pieces(
            base.clone(),
            P {
                domain,
                method,
                attr,
                count,
            },
        )
        .into_iter()
        .map(|it| match it {
            I::FullInput => GeoCacheRef::Geo(base.clone()),
            I::View(v) => GeoCacheRef::View(v),
        })
        .collect()
    }

    pub fn reset_foreach_cache_by_block_id(&mut self, block_id: &str) {
        let mut ids: Vec<NodeId> = Vec::new();
        for (id, n) in &self.nodes {
            let uid = Self::p_str(&n.parameters, "block_uid", "");
            let bid = Self::p_str(&n.parameters, "block_id", "");
            let ok = (!uid.is_empty() && uid == block_id) || (!bid.is_empty() && bid == block_id);
            if matches!(&n.node_type, NodeType::Generic(s) if (s == "ForEach Begin" || s == "ForEach End"))
                && ok
            {
                ids.push(*id);
            }
        }
        for id in ids {
            self.foreach_piece_cache.remove(&id);
            self.foreach_block_cache.remove(&id);
            self.foreach_block_cache_ref.remove(&id);
            self.foreach_compiled_cache.remove(&id);
            self.foreach_externals_cache.remove(&id);
            self.foreach_reach_cache.remove(&id);
            self.geo_cache_remove(id);
            self.port_geometry_cache.retain(|(nid, _), _| nid != &id);
            self.port_ref_cache.retain(|(nid, _), _| nid != &id);
            self.mark_dirty(id);
        }
    }

    fn foreach_run_end_ref(
        &mut self,
        end_id: NodeId,
        registry: &NodeRegistry,
        perf_stats: &mut Option<&mut HashMap<NodeId, ComputeRecord>>,
        overrides: Option<&NodeParamOverrides>,
        visiting: &mut StdHashSet<NodeId>,
    ) -> (GeoCacheRef, GeoCacheRef) {
        crate::nodes::runtime::graph_executor::GraphExecutor::new(self)
            .foreach_run_end(end_id, registry, perf_stats, overrides, visiting)
    }

    pub(crate) fn compute_output_ref(
        &mut self,
        node_id: NodeId,
        out_port: &PortId,
        registry: &NodeRegistry,
        perf_stats: &mut Option<&mut HashMap<NodeId, ComputeRecord>>,
        overrides: Option<&NodeParamOverrides>,
        visiting: &mut StdHashSet<NodeId>,
    ) -> GeoCacheRef {
        let key = (node_id, *out_port);
        if self.foreach_scope_nodes.contains(&node_id) {
            if self.foreach_port_epoch.get(&key) == Some(&self.foreach_scope_epoch) {
                if let Some(v) = self.port_ref_cache.get(&key) {
                    return v.clone();
                }
            }
        } else if let Some(v) = self.port_ref_cache.get(&key) {
            return v.clone();
        }

        // Debug UX (safe): only when Begin is a display node AND nothing already cached for this port (so foreach iteration injection still wins).
        if port_key::is_out0(out_port) {
            if let Some(node) = self.nodes.get(&node_id).cloned() {
                if node.is_display_node
                    && matches!(&node.node_type, NodeType::Generic(s) if s == "ForEach Begin")
                {
                    let uid = Self::p_str(&node.parameters, "block_uid", "");
                    let bid = Self::p_str(&node.parameters, "block_id", "");
                    let bkey = if !uid.is_empty() { uid } else { bid };
                    if let Some((_, Some(end_id), _)) = self.block_id_index.get(&bkey) {
                        if let Some(end) = self.nodes.get(end_id).cloned() {
                            if matches!(&end.node_type, NodeType::Generic(s) if s == "ForEach End")
                                && Self::p_bool(&end.parameters, "single_pass", false)
                            {
                                let base = self
                                    .first_src(node_id, &port_key::in0())
                                    .map(|(n, p)| {
                                        self.compute_output_ref(
                                            n, &p, registry, perf_stats, overrides, visiting,
                                        )
                                    })
                                    .unwrap_or_else(GeoCacheRef::empty)
                                    .as_geo();

                                let (domain, method, use_attr, mut attr, count) = (
                                    Self::p_int(&end.parameters, "piece_domain", 0),
                                    Self::p_int(&end.parameters, "iteration_method", 0),
                                    Self::p_bool(&end.parameters, "use_piece_attribute", true),
                                    Self::p_str(&end.parameters, "piece_attribute", "class")
                                        .trim()
                                        .to_string(),
                                    Self::p_int(&end.parameters, "count", 1).max(1) as usize,
                                );
                                let a_norm = attr.trim_start_matches('@');
                                let by_index = if domain == 1 { "ptnum" } else { "primnum" };
                                if !use_attr || attr.is_empty() {
                                    attr = by_index.into();
                                } else if !matches!(
                                    a_norm,
                                    "ptnum" | "pointnum" | "primnum" | "primitivenum"
                                ) {
                                    let ok = if domain == 1 {
                                        base.get_point_attribute(a_norm).is_some()
                                    } else {
                                        base.get_primitive_attribute(a_norm).is_some()
                                    };
                                    if !ok {
                                        attr = by_index.into();
                                    }
                                }
                                let pieces: Vec<GeoCacheRef> = {
                                    use crate::libs::algorithms::algorithms_runtime::foreach_pieces::{ForeachPiecePlanItem as I, ForeachPiecePlanParams as P, plan_foreach_pieces};
                                    plan_foreach_pieces(
                                        base.clone(),
                                        P {
                                            domain,
                                            method,
                                            attr,
                                            count,
                                        },
                                    )
                                    .into_iter()
                                    .map(|it| match it {
                                        I::FullInput => GeoCacheRef::Geo(base.clone()),
                                        I::View(v) => GeoCacheRef::View(v),
                                    })
                                    .collect()
                                };

                                let (sp_mode, sp_idx, sp_val) = (
                                    Self::p_int(&end.parameters, "single_pass_mode", 0),
                                    Self::p_int(&end.parameters, "single_pass_index", 0).max(0)
                                        as usize,
                                    Self::p_str(&end.parameters, "single_pass_value", "")
                                        .trim()
                                        .to_string(),
                                );
                                let sp_sel = if !pieces.is_empty() {
                                    if sp_mode == 1 && !sp_val.is_empty() {
                                        let iv = sp_val.parse::<i32>().ok();
                                        pieces
                                            .iter()
                                            .enumerate()
                                            .find_map(|(i, p)| match p {
                                                GeoCacheRef::View(v) => {
                                                    v.foreach_meta().and_then(|m| {
                                                        if m.value == sp_val || iv == Some(m.ivalue)
                                                        {
                                                            Some(i)
                                                        } else {
                                                            None
                                                        }
                                                    })
                                                }
                                                _ => None,
                                            })
                                            .unwrap_or_else(|| {
                                                sp_idx.min(pieces.len().saturating_sub(1))
                                            })
                                    } else {
                                        sp_idx.min(pieces.len().saturating_sub(1))
                                    }
                                } else {
                                    0
                                };
                                let piece = pieces
                                    .get(sp_sel)
                                    .cloned()
                                    .unwrap_or_else(GeoCacheRef::empty);
                                let begin_method = Self::p_int(&node.parameters, "method", 0);
                                return if begin_method == 3 {
                                    GeoCacheRef::Geo(base)
                                } else {
                                    piece
                                };
                            }
                        }
                    }
                }
            }
        }
        let node = match self.nodes.get(&node_id) {
            Some(n) => n.clone(),
            None => return GeoCacheRef::empty(),
        };
        if matches!(&node.node_type, NodeType::Generic(s) if s == "ForEach End")
            && port_key::is_out0(out_port)
        {
            let (out, fb) =
                self.foreach_run_end_ref(node_id, registry, perf_stats, overrides, visiting);
            self.port_ref_cache
                .insert((node_id, port_key::out0()), out.clone());
            if self.foreach_scope_nodes.contains(&node_id) {
                self.foreach_port_epoch
                    .insert((node_id, port_key::out0()), self.foreach_scope_epoch);
            }
            let _ = fb; // feedback is internal (no output port)
            return out;
        }
        if matches!(&node.node_type, NodeType::Generic(s) if s == "Attribute Kernel (GPU)")
            && port_key::is_out0(out_port)
        {
            let src = self
                .first_src(node_id, &port_key::in0())
                .map(|(n, p)| {
                    self.compute_output_ref(n, &p, registry, perf_stats, overrides, visiting)
                })
                .unwrap_or_else(GeoCacheRef::empty);
            let op = crate::nodes::gpu::ops::lower_attribute_kernel(&node.parameters);
            let crate::nodes::gpu::ops::GpuOp::AffineVec3 {
                domain,
                attr,
                mul,
                add,
            } = op
            else {
                return GeoCacheRef::Geo(src.as_geo());
            };
            if domain != 0 {
                return GeoCacheRef::Geo(src.as_geo());
            }
            let h = src.as_gpu().unwrap_or_else(|| {
                crate::nodes::gpu::runtime::GpuGeoHandle::from_cpu(src.as_geo())
            });
            let out = GeoCacheRef::Gpu(h.apply_affine_vec3(attr.as_str(), mul, add));
            self.port_ref_cache.insert(key, out.clone());
            if self.foreach_scope_nodes.contains(&node_id) {
                self.foreach_port_epoch
                    .insert(key, self.foreach_scope_epoch);
            }
            return out;
        }
        if let NodeType::CDA(data) = &node.node_type {
            if self.foreach_scope_nodes.contains(&node_id) {
                if self.foreach_port_geo_epoch.get(&key) == Some(&self.foreach_scope_epoch) {
                    if let Some(g) = self.port_geometry_cache.get(&key) {
                        return GeoCacheRef::Geo(g.clone());
                    }
                }
            } else if let Some(g) = self.port_geometry_cache.get(&key) {
                return GeoCacheRef::Geo(g.clone());
            }
            let Some(lib) = crate::cunning_core::cda::library::global_cda_library() else {
                return GeoCacheRef::empty();
            };
            let _ = lib.ensure_loaded(&data.asset_ref);
            let Some(a) = lib.get(data.asset_ref.uuid) else {
                return GeoCacheRef::empty();
            };
            let input_names: Vec<PortId> = a
                .inputs
                .iter()
                .map(|p| PortId::from(p.port_key().as_str()))
                .collect::<Vec<_>>();
            let input_names = if input_names.is_empty() {
                vec![port_key::in0()]
            } else {
                input_names
            };
            let mut inputs: Vec<Arc<dyn GeometryRef>> = Vec::new();
            for name in input_names {
                inputs.push(
                    self.first_src(node_id, &name)
                        .map(|(n, p)| {
                            self.compute_output_ref(
                                n, &p, registry, perf_stats, overrides, visiting,
                            )
                            .as_georef()
                        })
                        .unwrap_or_else(|| Arc::new(Geometry::new()) as Arc<dyn GeometryRef>),
                );
            }
            let mut ps = node.parameters.clone();
            if let Some(ov) = overrides.and_then(|m| m.get(&node_id)) {
                for (name, ch, v) in ov {
                    if let Some(p) = ps.iter_mut().find(|p| p.name == *name) {
                        apply_channel_value(&mut p.value, *ch, *v);
                    }
                }
            }
            let pv = convert_params(&ps);
            let mut ch: HashMap<String, Vec<f64>> = HashMap::new();
            for (k, v) in &pv {
                let c = crate::cunning_core::cda::utils::value_to_channels(v);
                if !c.is_empty() {
                    ch.insert(k.clone(), c);
                }
            }
            let mats: Vec<Arc<Geometry>> =
                inputs.iter().map(|g| Arc::new(g.materialize())).collect();
            let iv = if data.inner_param_overrides.is_empty() {
                None
            } else {
                Some(&data.inner_param_overrides)
            };
            let outs =
                a.evaluate_outputs_cached_with_value_overrides(&ch, mats.as_slice(), registry, iv);
            for (i, iface) in a.outputs.iter().enumerate() {
                let g = outs
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| Arc::new(Geometry::new()));
                self.port_geometry_cache
                    .insert((node_id, PortId::from(iface.port_key().as_str())), g);
                if self.foreach_scope_nodes.contains(&node_id) {
                    self.foreach_port_geo_epoch.insert(
                        (node_id, PortId::from(iface.port_key().as_str())),
                        self.foreach_scope_epoch,
                    );
                }
            }
            return self
                .port_geometry_cache
                .get(&key)
                .cloned()
                .map(GeoCacheRef::Geo)
                .unwrap_or_else(GeoCacheRef::empty);
        }
        GeoCacheRef::Geo(self.compute_node(node_id, registry, perf_stats, overrides, visiting))
    }

    fn compute_node(
        &mut self,
        node_id: NodeId,
        registry: &NodeRegistry,
        perf_stats: &mut Option<&mut HashMap<NodeId, ComputeRecord>>,
        overrides: Option<&NodeParamOverrides>,
        visiting: &mut StdHashSet<NodeId>,
    ) -> Arc<Geometry> {
        puffin::profile_function!();

        #[inline]
        fn cook_state_set(g: &NodeGraph, id: NodeId, s: crate::nodes::runtime::cook::NodeCookState) {
            if let Some(v) = g.cook_viz.as_ref() {
                if v.active.load(std::sync::atomic::Ordering::Relaxed) {
                    v.set_state(id, s);
                }
            }
        }

        #[inline]
        fn cook_failed(g: &NodeGraph, id: NodeId, msg: String) {
            if let Some(v) = g.cook_viz.as_ref() {
                if v.active.load(std::sync::atomic::Ordering::Relaxed) {
                    v.set_failed(id, msg);
                }
            }
        }

        // Check cache first
        if let Some(geo) = self.geo_cache_get(node_id) {
            cook_state_set(self, node_id, crate::nodes::runtime::cook::NodeCookState::Idle);
            return geo;
        }

        // We need to clone the node to avoid borrowing issues while computing inputs (recursive calls)
        let node = if let Some(n) = self.nodes.get(&node_id) {
            n.clone()
        } else {
            return Arc::new(Geometry::new());
        };

        if !visiting.insert(node_id) {
            return Arc::new(Geometry::new());
        }
        struct VisitGuard {
            set: *mut StdHashSet<NodeId>,
            id: NodeId,
        }
        impl Drop for VisitGuard {
            fn drop(&mut self) {
                unsafe {
                    (*self.set).remove(&self.id);
                }
            }
        }
        let _guard = VisitGuard {
            set: visiting as *mut _,
            id: node_id,
        };
        // perf: keep hot path silent

        puffin::profile_scope!("compute_node", node.name.as_str());

        // Find inputs
        // For Boolean, we need specific named inputs mapped to indices [0, 1]
        // Standard logic collects all inputs.
        // We need to ensure order if named ports are used.
        // But `compute_boolean` expects a slice.
        // The `connections` map gives us edges.
        // We iterate `self.connections`.

        // Better logic: Iterate node.inputs keys (ports) and find connections for each.
        // For Boolean: "Geometry A" -> idx 0, "Geometry B" -> idx 1.

        let mut input_vals: Vec<GeoCacheRef> = Vec::new();

        if let NodeType::CDA(data) = &node.node_type {
            let input_names: Vec<PortId> = crate::cunning_core::cda::library::global_cda_library()
                .and_then(|lib| lib.get(data.asset_ref.uuid))
                .map(|a| {
                    a.inputs
                        .iter()
                        .map(|p| PortId::from(p.port_key().as_str()))
                        .collect()
                })
                .unwrap_or_else(|| vec![PortId::from("Input")]);
            for name in input_names {
                input_vals.push(
                    self.first_src(node_id, &name)
                        .map(|(n, p)| {
                            self.compute_output_ref(
                                n, &p, registry, perf_stats, overrides, visiting,
                            )
                        })
                        .unwrap_or_else(GeoCacheRef::empty),
                );
            }
        } else if matches!(node.node_type, NodeType::Boolean) {
            // Specific handling for Boolean ports
            let ports = [
                crate::nodes::port_key::in_a(),
                crate::nodes::port_key::in_b(),
            ];
            for port_name in ports {
                input_vals.push(
                    self.first_src(node_id, &port_name)
                        .map(|(n, p)| {
                            self.compute_output_ref(
                                n, &p, registry, perf_stats, overrides, visiting,
                            )
                        })
                        .unwrap_or_else(GeoCacheRef::empty),
                );
            }
        } else if matches!(&node.node_type, NodeType::Generic(s) if s == "ForEach Begin") {
            input_vals.push(
                self.first_src(node_id, &crate::nodes::port_key::in0())
                    .map(|(n, p)| {
                        self.compute_output_ref(n, &p, registry, perf_stats, overrides, visiting)
                    })
                    .unwrap_or_else(GeoCacheRef::empty),
            );
        } else {
            // Standard single input logic (or multi-input merge which uses dynamic ports in future)
            // For now, just collect all connections to this node.
            // Note: This doesn't guarantee order for Merge node if it relies on port names.
            // But Merge node usually just takes all inputs.
            // Current implementation of input_ids was:
            let mut conns: Vec<(PortId, i32, NodeId, PortId, uuid::Uuid)> = self
                .connections
                .values()
                .filter(|c| c.to_node == node_id)
                .map(|c| {
                    (
                        c.to_port.clone(),
                        c.order,
                        c.from_node,
                        c.from_port.clone(),
                        c.id,
                    )
                })
                .collect();
            conns.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.4.cmp(&b.4)));
            for (_to_port, _ord, from_node, from_port, _id) in conns {
                input_vals.push(self.compute_output_ref(
                    from_node, &from_port, registry, perf_stats, overrides, visiting,
                ));
            }
        }

        enum Params<'a> {
            B(&'a [Parameter]),
            O(Vec<Parameter>),
        }
        impl<'a> Params<'a> {
            #[inline]
            fn as_slice(&self) -> &[Parameter] {
                match self {
                    Self::B(p) => p,
                    Self::O(v) => v,
                }
            }
        }
        let params = if let Some(ov) = overrides.and_then(|m| m.get(&node_id)) {
            let mut ps = node.parameters.clone();
            for (name, ch, v) in ov {
                if let Some(p) = ps.iter_mut().find(|p| p.name == *name) {
                    apply_channel_value(&mut p.value, *ch, *v);
                }
            }
            Params::O(ps)
        } else {
            Params::B(&node.parameters)
        };
        let ps = params.as_slice();

        // Check Bypass
        if node.is_bypassed {
            let result = input_vals
                .first()
                .map(|g| g.as_geo())
                .unwrap_or_else(|| Arc::new(Geometry::new()));
            self.geo_cache_put(node_id, result.clone());
            return result;
        }

        // Start Timer
        let start_time = Instant::now();

        let input_refs: Vec<Arc<dyn GeometryRef>> =
            input_vals.iter().map(|g| g.as_georef()).collect();

        cook_state_set(self, node_id, crate::nodes::runtime::cook::NodeCookState::Running);

        // Display UX: allow ForEach Begin display-node to show single_pass preview (compute_output_ref handles safe gating).
        if node.is_display_node
            && matches!(&node.node_type, NodeType::Generic(s) if s == "ForEach Begin")
        {
            return self
                .compute_output_ref(
                    node_id,
                    &port_key::out0(),
                    registry,
                    perf_stats,
                    overrides,
                    visiting,
                )
                .as_geo();
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &node.node_type {
            NodeType::Generic(s) if s == "ForEach Meta" => {
                let uid = Self::p_str(ps, "block_uid", "");
                let bid = Self::p_str(ps, "block_id", "");
                let key = if !uid.is_empty() { uid } else { bid };
                let m = if !key.is_empty() {
                    crate::nodes::runtime::foreach_tls::find_rev(key.as_str())
                } else {
                    crate::nodes::runtime::foreach_tls::last().map(|(_, m)| m)
                }
                .unwrap_or_default();
                let mut g = Geometry::new();
                g.insert_detail_attribute("iteration", Attribute::new(vec![m.iteration]));
                g.insert_detail_attribute("numiterations", Attribute::new(vec![m.numiterations]));
                g.insert_detail_attribute("value", Attribute::new(vec![m.value]));
                g.insert_detail_attribute("ivalue", Attribute::new(vec![m.ivalue]));
                Arc::new(g)
            }
            NodeType::Generic(s) if s == "ForEach End" => self
                .compute_output_ref(
                    node_id,
                    &port_key::out0(),
                    registry,
                    perf_stats,
                    overrides,
                    visiting,
                )
                .as_geo(),
            NodeType::Generic(name) => registry
                .create_op(name)
                .map(|op| op.compute(ps, &input_refs))
                .unwrap_or_else(|| Arc::new(Geometry::new())),
            NodeType::CreateCube => {
                let geo =
                    crate::nodes::basic::create_cube::create_cube_geometry(&convert_params(ps));
                Arc::new(geo)
            }
            NodeType::CreateSphere => Arc::new(
                crate::nodes::basic::create_sphere::create_sphere_geometry(&convert_params(ps)),
            ),
            NodeType::Transform => {
                if let Some(input) = input_vals.first() {
                    let g = input.as_geo();
                    if let Ok(res) = crate::nodes::basic_interaction::xform::apply_transform(
                        &g,
                        &convert_params(ps),
                    ) {
                        Arc::new(res)
                    } else {
                        g
                    }
                } else {
                    Arc::new(Geometry::new())
                }
            }
            NodeType::Merge => {
                let geos: Vec<Arc<Geometry>> = input_vals.iter().map(|g| g.as_geo()).collect();
                Arc::new(crate::libs::algorithms::merge::merge_geometry_arcs(&geos))
            }
            NodeType::Spline => {
                let op = crate::nodes::spline::spline_node::UnitySplineNode;
                crate::cunning_core::traits::node_interface::NodeOp::compute(&op, ps, &input_refs)
            }
            NodeType::AttributePromote(_) => {
                if let Some(input) = input_vals.first() {
                    let g = input.as_geo();
                    if let Ok(res) =
                        crate::nodes::attribute::attribute_promote::promote(&g, &convert_params(ps))
                    {
                        Arc::new(res)
                    } else {
                        g
                    }
                } else {
                    Arc::new(Geometry::new())
                }
            }
            NodeType::FbxImporter => Arc::new(crate::nodes::io::fbx_importer::compute_fbx_import(
                &convert_params(ps),
            )),
            NodeType::VdbFromPolygons => {
                if let Some(input) = input_vals.first() {
                    Arc::new(crate::nodes::vdb::vdb_from_mesh::compute_vdb_from_mesh(
                        &input.as_geo(),
                        &convert_params(ps),
                    ))
                } else {
                    Arc::new(Geometry::new())
                }
            }
            NodeType::VdbToPolygons => {
                if let Some(input) = input_vals.first() {
                    Arc::new(crate::nodes::vdb::vdb_to_mesh::compute_vdb_to_mesh(
                        &input.as_geo(),
                        &convert_params(ps),
                    ))
                } else {
                    Arc::new(Geometry::new())
                }
            }
            NodeType::VoxelEdit => {
                let input = input_vals
                    .first()
                    .map(|g| g.as_geo())
                    .unwrap_or_else(|| Arc::new(Geometry::new()));
                let pv = convert_params(ps);
                // Note: the normal geometry cache is cleared when a node is marked dirty.
                // For interactive nodes (like VoxelEdit) we keep a separate "previous" cache
                // so we can do incremental baking instead of replaying the full cmd history.
                let prev = self
                    .geo_cache_get(node_id)
                    .or_else(|| self.prev_geo_cache_get(node_id));
                Arc::new(crate::nodes::voxel::voxel_edit::compute_voxel_edit_cached(
                    node_id,
                    prev,
                    &input,
                    &pv,
                ))
            }
            NodeType::GroupCreate => {
                // GroupCreate is an op that expects a slice of inputs.
                // It can handle 1 or 2 inputs.
                let op = crate::nodes::group::group_create::GroupCreateNode;
                // We can't call compute directly if we need to follow NodeOp trait strictly with Arc slice,
                // but here we have Arc<Geometry> in input_geometries which matches.
                use crate::cunning_core::traits::node_interface::NodeOp;
                op.compute(ps, &input_refs)
            }
            NodeType::GroupCombine => {
                let op = crate::nodes::group::group_combine::GroupCombineNode;
                use crate::cunning_core::traits::node_interface::NodeOp;
                op.compute(ps, &input_refs)
            }
            NodeType::GroupPromote => {
                let op = crate::nodes::group::group_promote::GroupPromoteNode;
                use crate::cunning_core::traits::node_interface::NodeOp;
                op.compute(ps, &input_refs)
            }
            NodeType::GroupManage => {
                let op = crate::nodes::group::group_manage::GroupManageNode;
                use crate::cunning_core::traits::node_interface::NodeOp;
                op.compute(ps, &input_refs)
            }
            NodeType::GroupNormalize => {
                let op = crate::nodes::group::group_normalize::GroupNormalizeNode;
                use crate::cunning_core::traits::node_interface::NodeOp;
                op.compute(ps, &input_refs)
            }
            NodeType::Boolean => {
                let params = convert_params(ps);
                let geos: Vec<Arc<Geometry>> = input_vals.iter().map(|g| g.as_geo()).collect();
                Arc::new(
                    crate::nodes::modeling::boolean::boolean_node::compute_boolean(&geos, &params),
                )
            }
            NodeType::PolyExtrude => {
                let op = crate::nodes::modeling::poly_extrude::PolyExtrudeNode;
                use crate::cunning_core::traits::node_interface::NodeOp;
                op.compute(ps, &input_refs)
            }
            NodeType::PolyBevel => {
                let op = crate::nodes::modeling::poly_bevel::PolyBevelNode;
                use crate::cunning_core::traits::node_interface::NodeOp;
                op.compute(ps, &input_refs)
            }
            NodeType::Fuse => {
                let op = crate::nodes::modeling::fuse_node::FuseNode;
                use crate::cunning_core::traits::node_interface::NodeOp;
                op.compute(ps, &input_refs)
            }
            NodeType::CDA(data) => {
                if let Some(lib) = crate::cunning_core::cda::library::global_cda_library() {
                    let _ = lib.ensure_loaded(&data.asset_ref);
                    if let Some(a) = lib.get(data.asset_ref.uuid) {
                        let pv = convert_params(ps);
                        let mut ch: HashMap<String, Vec<f64>> = HashMap::new();
                        for (k, v) in &pv {
                            let c = crate::cunning_core::cda::utils::value_to_channels(v);
                            if !c.is_empty() {
                                ch.insert(k.clone(), c);
                            }
                        }
                        let mats: Vec<Arc<Geometry>> = input_refs
                            .iter()
                            .map(|g| Arc::new(g.materialize()))
                            .collect();
                        let iv = if data.inner_param_overrides.is_empty() {
                            None
                        } else {
                            Some(&data.inner_param_overrides)
                        };
                        let _inst_guard =
                            crate::nodes::voxel::voxel_edit::VoxelCdaInstanceGuard::push(node_id);
                        a.evaluate_outputs_cached_with_value_overrides(
                            &ch,
                            mats.as_slice(),
                            registry,
                            iv,
                        )
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| Arc::new(Geometry::new()))
                    } else {
                        Arc::new(Geometry::new())
                    }
                } else {
                    Arc::new(Geometry::new())
                }
            }
            NodeType::CDAInput(_) => {
                // Input node returns cached input geometry directly (pre-filled during evaluate)
                input_vals
                    .first()
                    .map(|g| g.as_geo())
                    .unwrap_or_else(|| Arc::new(Geometry::new()))
            }
            NodeType::CDAOutput(_) => {
                // Output node passes input through directly
                input_vals
                    .first()
                    .map(|g| g.as_geo())
                    .unwrap_or_else(|| Arc::new(Geometry::new()))
            }
        }));
        let result = match result {
            Ok(v) => v,
            Err(_) => {
                cook_failed(self, node_id, format!("panic: node_id={}", node_id));
                Arc::new(Geometry::new())
            }
        };
        // perf: keep hot path silent

        // End Timer & Record
        let end_time = Instant::now();
        let duration = end_time.duration_since(start_time);

        if let Some(stats) = perf_stats {
            stats.insert(
                node_id,
                ComputeRecord {
                    duration,
                    start_time,
                    end_time,
                    thread_name: format!("{:?}", thread::current().id()),
                },
            );
        }

        self.geo_cache_put(node_id, result.clone());
        cook_state_set(self, node_id, crate::nodes::runtime::cook::NodeCookState::Idle);
        result
    }

    pub(crate) fn compute_output(
        &mut self,
        node_id: NodeId,
        out_port: &PortId,
        registry: &NodeRegistry,
        perf_stats: &mut Option<&mut HashMap<NodeId, ComputeRecord>>,
        overrides: Option<&NodeParamOverrides>,
        visiting: &mut StdHashSet<NodeId>,
    ) -> Arc<Geometry> {
        let key = (node_id, out_port.clone());
        if self.foreach_scope_nodes.contains(&node_id) {
            if self.foreach_port_geo_epoch.get(&key) == Some(&self.foreach_scope_epoch) {
                if let Some(g) = self.port_geometry_cache.get(&key) {
                    return g.clone();
                }
            }
        } else if let Some(g) = self.port_geometry_cache.get(&key) {
            return g.clone();
        }
        let node = match self.nodes.get(&node_id) {
            Some(n) => n.clone(),
            None => return Arc::new(Geometry::new()),
        };
        if let NodeType::CDA(data) = &node.node_type {
            // Gather inputs for the CDA node (same logic as compute_node would)
            let mut input_geometries: Vec<Arc<Geometry>> = Vec::new();
            let Some(lib) = crate::cunning_core::cda::library::global_cda_library() else {
                return Arc::new(Geometry::new());
            };
            let _ = lib.ensure_loaded(&data.asset_ref);
            let Some(a) = lib.get(data.asset_ref.uuid) else {
                return Arc::new(Geometry::new());
            };
            let input_names: Vec<PortId> = a
                .inputs
                .iter()
                .map(|p| PortId::from(p.port_key().as_str()))
                .collect::<Vec<_>>();
            let input_names = if input_names.is_empty() {
                vec![PortId::from("Input")]
            } else {
                input_names
            };
            for name in input_names {
                let mut srcs: Vec<(uuid::Uuid, NodeId, PortId)> = self
                    .connections
                    .values()
                    .filter(|c| c.to_node == node_id && c.to_port == name)
                    .map(|c| (c.id, c.from_node, c.from_port.clone()))
                    .collect();
                srcs.sort_by(|a, b| a.0.cmp(&b.0));
                if srcs.len() > 1 {
                    if let Some(c) = crate::console::global_console() {
                        c.warning(format!(
                            "CDA input multi-wire: inst_node={} port={} count={}",
                            node_id,
                            name.as_str(),
                            srcs.len()
                        ));
                    }
                }
                if let Some((_id, from_node, from_port)) = srcs.into_iter().next() {
                    input_geometries.push(self.compute_output(
                        from_node, &from_port, registry, perf_stats, overrides, visiting,
                    ));
                } else {
                    input_geometries.push(Arc::new(Geometry::new()));
                }
            }
            let mut ps = node.parameters.clone();
            if let Some(ov) = overrides.and_then(|m| m.get(&node_id)) {
                for (name, ch, v) in ov {
                    if let Some(p) = ps.iter_mut().find(|p| p.name == *name) {
                        apply_channel_value(&mut p.value, *ch, *v);
                    }
                }
            }
            let pv = convert_params(&ps);
            let mut ch: HashMap<String, Vec<f64>> = HashMap::new();
            for (k, v) in &pv {
                let c = crate::cunning_core::cda::utils::value_to_channels(v);
                if !c.is_empty() {
                    ch.insert(k.clone(), c);
                }
            }
            let iv = if data.inner_param_overrides.is_empty() {
                None
            } else {
                Some(&data.inner_param_overrides)
            };
            let _inst_guard =
                crate::nodes::voxel::voxel_edit::VoxelCdaInstanceGuard::push(node_id);
            let outs = a.evaluate_outputs_cached_with_value_overrides(
                &ch,
                input_geometries.as_slice(),
                registry,
                iv,
            );
            for (i, iface) in a.outputs.iter().enumerate() {
                let g = outs
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| Arc::new(Geometry::new()));
                self.port_geometry_cache
                    .insert((node_id, PortId::from(iface.port_key().as_str())), g);
                if self.foreach_scope_nodes.contains(&node_id) {
                    self.foreach_port_geo_epoch.insert(
                        (node_id, PortId::from(iface.port_key().as_str())),
                        self.foreach_scope_epoch,
                    );
                }
            }
            return self
                .port_geometry_cache
                .get(&key)
                .cloned()
                .unwrap_or_else(|| {
                    self.geo_cache_get(node_id)
                        .unwrap_or_else(|| Arc::new(Geometry::new()))
                });
        }
        self.compute_node(node_id, registry, perf_stats, overrides, visiting)
    }

    /// Compute geometry for a specific output port (no perf stats / no overrides).
    pub(crate) fn compute_output_simple(
        &mut self,
        node_id: NodeId,
        out_port: &PortId,
        registry: &NodeRegistry,
    ) -> Arc<Geometry> {
        let mut visiting: StdHashSet<NodeId> = StdHashSet::default();
        let mut stats: Option<&mut HashMap<NodeId, ComputeRecord>> = None;
        self.compute_output(node_id, out_port, registry, &mut stats, None, &mut visiting)
    }

    // (removed) compute_cda_outputs_cached: CDA executes via runtime VM.
}

fn convert_params(params: &[Parameter]) -> HashMap<String, ParameterValue> {
    params
        .iter()
        .map(|p| (p.name.clone(), p.value.clone()))
        .collect()
}
