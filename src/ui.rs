//! The UI module, responsible for defining the global UI state and rendering the main UI.
#![allow(dead_code)]

use bevy::prelude::*;
use bevy_egui::egui;
use egui_dock::DockState;
use std::collections::HashSet;
use uuid::Uuid;

use crate::{
    nodes::{
        ConnectionId, NetworkBoxId, Node, NodeGraph, NodeId, NodeType, PromoteNoteId, StickyNoteId,
    },
    NodeGraphResource,
};

pub mod file_picker;
pub use file_picker::{FilePickerChosenEvent, FilePickerMode, FilePickerState, OpenFilePickerEvent};

#[derive(Clone, Debug)]
pub enum SettingsEdit {
    Set(String, crate::settings::SettingValue),
    Remove(String),
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentSelectionMode {
    #[default]
    Points,
    Vertices,
    Primitives,
    Edges,
}

#[derive(Default, Clone, Debug)]
pub struct ComponentSelection {
    pub mode: ComponentSelectionMode,
    pub indices: std::collections::HashSet<usize>,
}

#[derive(Default, Debug)]
pub struct RadialMenuState {
    pub node_id: Option<NodeId>,
}

// NOTE: Curve tool state lives in plugin node-local state (HostApi node_state_get/set). No UI-global state here.

#[derive(Debug, Clone, PartialEq, Eq, Copy, Default)]
pub enum LayoutMode {
    #[default]
    Desktop,
    Tablet,
    Phone,
}

#[derive(Debug, Clone, PartialEq, Eq, Copy, Default)]
pub enum MobileTab {
    #[default]
    Viewport,
    NodeGraph,
    Properties,
    Console,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ShelfTab {
    Create,
    Modify,
    Model,
    Polygon,
    Deform,
    Texture,
    Rigging,
    Empty, // Placeholder for new empty sets
}

#[derive(Clone, Debug)]
pub enum ShelfCommand {
    Add(ShelfTab, egui_dock::SurfaceIndex, egui_dock::NodeIndex),
    Remove(
        egui_dock::SurfaceIndex,
        egui_dock::NodeIndex,
        egui_dock::TabIndex,
    ),
    Toggle(ShelfTab, egui_dock::SurfaceIndex, egui_dock::NodeIndex),
    NewSet(egui_dock::SurfaceIndex, egui_dock::NodeIndex),
}

#[derive(Clone, Debug)]
pub enum PaneTabType {
    Viewport,
    NodeGraph,
    Properties,
    Spreadsheet,
    Outliner,
    Console,
    Codex,
    Coverlay,
    Timeline,
    Settings,
    NodeInfo,
    AiWorkspace,
}

#[derive(Message, Default, Clone)]
pub struct OpenSettingsWindowEvent;

/// Marker: initialize egui style/fonts for newly spawned windows before first `BeginFrame`.
#[derive(Component, Default, Clone, Copy)]
pub struct NeedsEguiFontsInit;

#[derive(Message, Default, Clone)]
pub struct OpenAiWorkspaceWindowEvent;

#[derive(Message, Default, Clone)]
pub struct OpenHotReloadWindowEvent;

#[derive(Message, Clone)]
pub struct OpenNodeInfoWindowEvent {
    pub node_id: NodeId,
    pub initial_rect: egui::Rect,
}

/// Unique identifier for a floating editor tab instance.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FloatingTabId(pub uuid::Uuid);

#[derive(Message, Clone)]
pub struct FloatTabToWindowEvent {
    pub title: String,
    pub initial_rect: egui::Rect,
    pub id: FloatingTabId,
}

/// Metadata for a single floating window.
#[derive(Clone, Debug)]
pub struct FloatingWindowEntry {
    pub title: String,
    pub id: FloatingTabId,
}

/// Marker component for custom chrome on floating native windows.
#[derive(Component, Clone, Debug)]
pub struct FloatingWindowChrome {
    pub title: String,
    pub id: FloatingTabId,
}

/// Runtime state for custom-chrome floating windows.
#[derive(Component, Clone, Debug, Default)]
pub struct FloatingWindowChromeState {
    pub pinned: bool,
    pub maximized: bool,
}

#[derive(Resource, Default)]
pub struct FloatingTabRegistry {
    /// Maps a Bevy Window Entity to the floating tab instance it displays.
    pub floating_windows: std::collections::HashMap<Entity, FloatingWindowEntry>,
}

#[derive(Debug)] // Cannot derive Clone because PaneTabType might not be enough if we wanted to pass data, but for now it's fine. Wait, PaneCommand usually just needs type.
pub enum PaneCommand {
    Add(PaneTabType, egui_dock::SurfaceIndex, egui_dock::NodeIndex),
    AddByName(String, egui_dock::SurfaceIndex, egui_dock::NodeIndex),
}

fn create_default_shelf() -> DockState<ShelfTab> {
    // Initial set (Left side: Modeling)
    let mut state = DockState::new(vec![
        ShelfTab::Create,
        ShelfTab::Modify,
        ShelfTab::Model,
        ShelfTab::Polygon,
    ]);

    // Split right (Right side: Simulation/Rigging)
    // The root node is always index 0 for a new DockState
    let root = egui_dock::NodeIndex::root();

    // split_right returns the indices of the two new nodes [left, right]
    // But we don't need to store them, just perform the split.
    state.main_surface_mut().split_right(
        root,
        0.55,
        vec![ShelfTab::Deform, ShelfTab::Rigging, ShelfTab::Texture],
    );

    state
}

// UI State Resource
#[derive(Resource)]
pub struct UiState {
    pub selected_nodes: HashSet<NodeId>,
    pub selected_connections: HashSet<ConnectionId>,
    pub selected_network_boxes: HashSet<NetworkBoxId>,
    pub selected_promote_notes: HashSet<PromoteNoteId>,
    pub selected_sticky_notes: HashSet<StickyNoteId>,
    pub last_selected_node_id: Option<NodeId>,
    pub radial_menu_state: RadialMenuState,
    pub dragged_node_id: Option<NodeId>,
    pub component_selection: ComponentSelection,
    // Layout Mode State
    pub layout_mode: LayoutMode,
    pub mobile_active_tab: MobileTab,
    // Shelf State
    pub shelf_dock_state: DockState<ShelfTab>,
    pub shelf_command_queue: Vec<ShelfCommand>,
    // Cache allows checking tab presence inside add_popup even when dock_state is borrowed/swapped
    pub shelf_tab_cache: std::collections::HashMap<
        (egui_dock::SurfaceIndex, egui_dock::NodeIndex),
        std::collections::HashSet<ShelfTab>,
    >,
    // Pane Command Queue
    pub pane_command_queue: Vec<PaneCommand>,
    // Settings edits staged by UI (applied after UI pass to avoid per-frame ResMut on SettingsStores)
    pub settings_edits: Vec<SettingsEdit>,
    // Flag to open AI Workspace GPUI window (set by pane command, consumed by system)
    pub pending_open_ai_workspace: bool,
    // Floating custom-chrome z-order (tail = frontmost).
    pub floating_window_chrome_order: Vec<FloatingTabId>,
}

impl Default for UiState {
    fn default() -> Self {
        let shelf_dock_state = create_default_shelf();

        Self {
            selected_nodes: Default::default(),
            selected_connections: Default::default(),
            selected_network_boxes: Default::default(),
            selected_promote_notes: Default::default(),
            selected_sticky_notes: Default::default(),
            last_selected_node_id: Default::default(),
            radial_menu_state: Default::default(),
            dragged_node_id: Default::default(),
            component_selection: Default::default(),
            layout_mode: Default::default(),
            mobile_active_tab: Default::default(),
            shelf_dock_state,
            shelf_command_queue: Default::default(),
            shelf_tab_cache: Default::default(),
            pane_command_queue: Default::default(),
            settings_edits: Default::default(),
            pending_open_ai_workspace: false,
            floating_window_chrome_order: Default::default(),
        }
    }
}

// Node Editor State
#[derive(Resource)]
pub struct NodeEditorState {
    pub pan: egui::Vec2,
    pub zoom: f32,
    pub target_pan: egui::Vec2,
    pub target_zoom: f32,
    pub zoom_center: Option<egui::Pos2>,
    pub cda_path: Vec<NodeId>, // Current CDA edit path (empty = main graph)
    pub undo_stack: crate::cunning_core::command::UndoStack,
}

impl Default for NodeEditorState {
    fn default() -> Self {
        Self {
            pan: egui::Vec2::ZERO,
            zoom: 1.0,
            target_pan: egui::Vec2::ZERO,
            target_zoom: 1.0,
            zoom_center: None,
            cda_path: Vec::new(),
            undo_stack: crate::cunning_core::command::UndoStack::new(),
        }
    }
}

#[derive(Resource)]
pub struct TimelineState {
    pub current_frame: f32,
    pub start_frame: f32,
    pub end_frame: f32,
    pub is_playing: bool,
    pub fps: f32,
    pub play_started_at: Option<f64>,
    pub play_started_frame: f32,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            current_frame: 1.0,
            start_frame: 1.0,
            end_frame: 240.0,
            is_playing: false,
            fps: 24.0,
            play_started_at: None,
            play_started_frame: 1.0,
        }
    }
}

pub fn create_node_instance(
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    name: &str,
    node_type: NodeType,
    position: egui::Pos2,
) -> Node {
    let node_id = Uuid::new_v4();
    let mut new_node = Node::new(node_id, name.to_string(), node_type, position);
    let size = crate::node_editor_settings::resolved_node_size(node_editor_settings);
    new_node.size = egui::vec2(size[0], size[1]);
    new_node
}

fn create_and_add_node(
    node_graph: &mut NodeGraph,
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    name: &str,
    node_type: NodeType,
    position: egui::Pos2,
) -> NodeId {
    let new_node = create_node_instance(node_editor_settings, name, node_type, position);
    let node_id = new_node.id;
    let is_foreach = matches!(&new_node.node_type, NodeType::Generic(s) if s == "ForEach Begin" || s == "ForEach End" || s == "ForEach Meta");
    node_graph.nodes.insert(new_node.id, new_node);
    if is_foreach {
        node_graph.rebuild_block_id_index();
    }
    node_id
}

pub fn create_cube_node(
    ui_state: &mut UiState,
    node_graph_res: &mut NodeGraphResource,
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    position: egui::Pos2,
) {
    let node_graph = &mut node_graph_res.0;
    let node_id = create_and_add_node(
        node_graph,
        node_editor_settings,
        "Create Cube",
        NodeType::CreateCube,
        position,
    );

    // Select the newly created node
    ui_state.selected_nodes.clear();
    ui_state.selected_nodes.insert(node_id);
    ui_state.last_selected_node_id = Some(node_id);
}

pub fn create_generic_node(
    ui_state: &mut UiState,
    node_graph_res: &mut NodeGraphResource,
    node_registry: &crate::cunning_core::registries::node_registry::NodeRegistry,
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    position: egui::Pos2,
    node_name: &str,
) {
    info!("Creating generic node: {}", node_name);
    let node_graph = &mut node_graph_res.0;

    let node_type = match node_name {
        "Merge" => NodeType::Merge,
        "Curve" => NodeType::Generic("Curve".to_string()),
        "Spline" => NodeType::Spline,
        "Transform" => NodeType::Transform,
        "Create Cube" => NodeType::CreateCube,
        "Create Sphere" => NodeType::CreateSphere,
        "Boolean" => NodeType::Boolean,
        "PolyExtrude" | "Poly Extrude" => NodeType::PolyExtrude,
        "PolyBevel" | "Poly Bevel" => NodeType::PolyBevel,
        "CopyToPoints" | "Copy To Points" | "Copy to Points" => {
            NodeType::Generic("CopyToPoints".to_string())
        }
        "Fuse" => NodeType::Fuse,
        "FBX Importer" => NodeType::FbxImporter,
        "SDF From Polygons" | "SDF from Polygons" | "VDB From Polygons" | "VDB from Polygons" => {
            NodeType::VdbFromPolygons
        }
        "SDF To Polygons" | "SDF to Polygons" | "VDB To Polygons" | "VDB to Polygons" => {
            NodeType::VdbToPolygons
        }
        "Voxel Edit" | "VoxelEdit" => NodeType::VoxelEdit,
        "Group Create" => NodeType::GroupCreate,
        _ => NodeType::Generic(node_name.to_string()),
    };

    let node_id = create_and_add_node(
        node_graph,
        node_editor_settings,
        node_name,
        node_type,
        position,
    );
    info!("Node created in graph: {}", node_id);

    // If Generic, try to populate parameters from Registry
    if let Some(node) = node_graph.nodes.get_mut(&node_id) {
        if let NodeType::Generic(name) = &node.node_type {
            info!("Looking up descriptor for: {}", name);
            if let Some(descriptor) = node_registry.nodes.read().unwrap().get(name) {
                info!("Found descriptor. Calling parameters_factory...");
                node.parameters = (descriptor.parameters_factory)();
                info!("Parameters set. Count: {}", node.parameters.len());
            } else {
                warn!("No descriptor found for generic node: {}", name);
            }
        }
    }

    // Select the newly created node without altering display flags
    ui_state.selected_nodes.clear();
    ui_state.selected_nodes.insert(node_id);
    ui_state.last_selected_node_id = Some(node_id);

    info!("Node created and selected. Creation complete.");
}

pub fn create_sphere_node(
    ui_state: &mut UiState,
    node_graph_res: &mut NodeGraphResource,
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    pos: egui::Pos2,
) {
    let node_graph = &mut node_graph_res.0;
    let node_id = create_and_add_node(
        node_graph,
        node_editor_settings,
        "Sphere",
        NodeType::CreateSphere,
        pos,
    );

    // Select the newly created node
    ui_state.selected_nodes.clear();
    ui_state.selected_nodes.insert(node_id);
    ui_state.last_selected_node_id = Some(node_id);
}

pub fn prepare_generic_node(
    node_registry: &crate::cunning_core::registries::node_registry::NodeRegistry,
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    position: egui::Pos2,
    node_name: &str,
) -> Node {
    let node_type = match node_name {
        "Merge" => NodeType::Merge,
        "Curve" => NodeType::Generic("Curve".to_string()),
        "Spline" => NodeType::Spline,
        "Transform" => NodeType::Transform,
        "Create Cube" => NodeType::CreateCube,
        "Create Sphere" => NodeType::CreateSphere,
        "Boolean" => NodeType::Boolean,
        "PolyExtrude" | "Poly Extrude" => NodeType::PolyExtrude,
        "PolyBevel" | "Poly Bevel" => NodeType::PolyBevel,
        "CopyToPoints" | "Copy To Points" | "Copy to Points" => {
            NodeType::Generic("CopyToPoints".to_string())
        }
        "Fuse" => NodeType::Fuse,
        "FBX Importer" => NodeType::FbxImporter,
        "SDF From Polygons" | "SDF from Polygons" | "VDB From Polygons" | "VDB from Polygons" => {
            NodeType::VdbFromPolygons
        }
        "SDF To Polygons" | "SDF to Polygons" | "VDB To Polygons" | "VDB to Polygons" => {
            NodeType::VdbToPolygons
        }
        "Voxel Edit" | "VoxelEdit" => NodeType::VoxelEdit,
        "Group Create" => NodeType::GroupCreate,
        _ => NodeType::Generic(node_name.to_string()),
    };
    let mut node = create_node_instance(node_editor_settings, node_name, node_type, position);
    if let NodeType::Generic(name) = &node.node_type {
        if let Some(desc) = node_registry.nodes.read().unwrap().get(name) {
            node.parameters = (desc.parameters_factory)();
        }
    }
    node
}

pub fn create_generic_node_in_graph(
    ui_state: &mut UiState,
    node_graph: &mut NodeGraph,
    node_registry: &crate::cunning_core::registries::node_registry::NodeRegistry,
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    position: egui::Pos2,
    node_name: &str,
) -> NodeId {
    let node = prepare_generic_node(node_registry, node_editor_settings, position, node_name);
    let node_id = node.id;
    let is_foreach = matches!(&node.node_type, NodeType::Generic(s) if s == "ForEach Begin" || s == "ForEach End" || s == "ForEach Meta");
    node_graph.nodes.insert(node.id, node);
    if is_foreach {
        node_graph.rebuild_block_id_index();
    }
    ui_state.selected_nodes.clear();
    ui_state.selected_nodes.insert(node_id);
    ui_state.last_selected_node_id = Some(node_id);
    node_id
}

pub fn create_transform_node(
    ui_state: &mut UiState,
    node_graph_res: &mut NodeGraphResource,
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    pos: egui::Pos2,
) {
    let node_graph = &mut node_graph_res.0;
    let node_id = create_and_add_node(
        node_graph,
        node_editor_settings,
        "Transform",
        NodeType::Transform,
        pos,
    );

    // Select the newly created node
    ui_state.selected_nodes.clear();
    ui_state.selected_nodes.insert(node_id);
    ui_state.last_selected_node_id = Some(node_id);
}

pub fn create_attribute_promote_node(
    ui_state: &mut UiState,
    node_graph_res: &mut NodeGraphResource,
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    pos: egui::Pos2,
) {
    let node_graph = &mut node_graph_res.0;
    let node_id = create_and_add_node(
        node_graph,
        node_editor_settings,
        "Attribute Promote",
        NodeType::AttributePromote(Default::default()),
        pos,
    );

    // Select the newly created node
    ui_state.selected_nodes.clear();
    ui_state.selected_nodes.insert(node_id);
    ui_state.last_selected_node_id = Some(node_id);
}

pub fn create_merge_node(
    ui_state: &mut UiState,
    node_graph_res: &mut NodeGraphResource,
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    pos: egui::Pos2,
) {
    let node_graph = &mut node_graph_res.0;
    let node_id = create_and_add_node(
        node_graph,
        node_editor_settings,
        "Merge",
        NodeType::Merge,
        pos,
    );

    // Select the newly created node
    ui_state.selected_nodes.clear();
    ui_state.selected_nodes.insert(node_id);
    ui_state.last_selected_node_id = Some(node_id);
}

pub fn create_fbx_importer_node(
    ui_state: &mut UiState,
    node_graph_res: &mut NodeGraphResource,
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    pos: egui::Pos2,
) {
    let node_graph = &mut node_graph_res.0;
    let node_id = create_and_add_node(
        node_graph,
        node_editor_settings,
        "FBX Importer",
        NodeType::FbxImporter,
        pos,
    );

    // Select the newly created node
    ui_state.selected_nodes.clear();
    ui_state.selected_nodes.insert(node_id);
    ui_state.last_selected_node_id = Some(node_id);
}

pub fn create_sdf_from_polygons_node(
    ui_state: &mut UiState,
    node_graph_res: &mut NodeGraphResource,
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    pos: egui::Pos2,
) {
    let node_graph = &mut node_graph_res.0;
    let node_id = create_and_add_node(
        node_graph,
        node_editor_settings,
        "SDF From Polygons",
        NodeType::VdbFromPolygons,
        pos,
    );

    // Select the newly created node
    ui_state.selected_nodes.clear();
    ui_state.selected_nodes.insert(node_id);
    ui_state.last_selected_node_id = Some(node_id);
}

pub fn create_sdf_to_polygons_node(
    ui_state: &mut UiState,
    node_graph_res: &mut NodeGraphResource,
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    pos: egui::Pos2,
) {
    let node_graph = &mut node_graph_res.0;
    let node_id = create_and_add_node(
        node_graph,
        node_editor_settings,
        "SDF To Polygons",
        NodeType::VdbToPolygons,
        pos,
    );

    // Select the newly created node
    ui_state.selected_nodes.clear();
    ui_state.selected_nodes.insert(node_id);
    ui_state.last_selected_node_id = Some(node_id);
}

// Back-compat helpers (older callers / UI actions).
pub fn create_vdb_from_polygons_node(
    ui_state: &mut UiState,
    node_graph_res: &mut NodeGraphResource,
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    pos: egui::Pos2,
) {
    create_sdf_from_polygons_node(ui_state, node_graph_res, node_editor_settings, pos);
}

pub fn create_vdb_to_polygons_node(
    ui_state: &mut UiState,
    node_graph_res: &mut NodeGraphResource,
    node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    pos: egui::Pos2,
) {
    create_sdf_to_polygons_node(ui_state, node_graph_res, node_editor_settings, pos);
}
