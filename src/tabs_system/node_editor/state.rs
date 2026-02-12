use crate::nodes::{
    ConnectionId, InputStyle, NetworkBoxId, Node, NodeId, NodeStyle, PortId, PromoteNoteId,
    StickyNoteId,
};
use bevy_egui::egui::{Pos2, Rect, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

pub use super::cda::editor_state::CDAEditorState;

#[derive(Clone)]
pub struct NodeAnimation {
    pub start_pos: Pos2,
    pub target_pos: Pos2,
    pub start_time: f64,
    pub duration: f64,
}

#[derive(Clone, Default)]
pub struct ElasticState {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuButton {
    Bypass,
    Blame,
    Information,
    Visible,
    Temp,
    Lock,
    Parm,
}

#[derive(Clone, Debug)]
pub struct ButtonGeometry {
    pub button: MenuButton,
    pub points: Vec<Pos2>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RadialMenuAction {
    ToggleBypass,
    ToggleDisplay,
    ToggleTemplate,
    ToggleLock,
    ToggleInfo,
}

#[derive(Clone, Debug)]
pub struct NodeInteraction {
    pub hovered: bool,
    pub clicked: bool,
    pub clicked_on_output_port: Option<(NodeId, PortId)>,
    pub dragged_on_output_port: Option<(NodeId, PortId)>,
    pub parameter_changed: bool,
}

#[derive(Clone)]
pub struct NodeSnapshot {
    pub id: NodeId,
    pub name: String,
    pub position: Pos2,
    pub size: Vec2,
    pub header_h: f32,
    pub input_style: InputStyle,
    pub style: NodeStyle,
    pub is_template: bool,
    pub is_bypassed: bool,
    pub is_display_node: bool,
    pub is_locked: bool,
    pub inputs: Vec<PortId>,
    pub outputs: Vec<PortId>,
}

#[derive(Clone, Default)]
pub struct GhostGraph {
    pub nodes: Vec<NodeSnapshot>,
    /// Directed edges between ghost nodes (by id).
    pub links: Vec<(NodeId, NodeId)>,
}

impl std::fmt::Debug for GhostGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GhostGraph")
            .field("nodes", &self.nodes.len())
            .field("links", &self.links.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopilotRelayStatus {
    Generating,
    Ready,
    Error,
}

#[derive(Clone, Debug)]
pub struct CopilotRelaySession {
    pub session_id: Uuid,
    pub backend: CopilotBackend,
    pub sources: Vec<(NodeId, PortId)>,
    pub anchor_graph_pos: Pos2,
    pub request_id: String,
    pub status: CopilotRelayStatus,
    pub target_nodes: usize,
    pub created_at: std::time::Instant,
    pub ghost: Option<GhostGraph>,
    pub ghost_params: HashMap<String, String>,
    pub reason_title: Option<String>,
    pub reason: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopilotRelayActionKind {
    Apply,
    Reroll,
    Cancel,
}

#[derive(Clone, Copy, Debug)]
pub struct CopilotRelayAction {
    pub session_id: Uuid,
    pub kind: CopilotRelayActionKind,
}

#[derive(Clone)]
pub struct NodeHit {
    pub id: NodeId,
    pub rect: Rect,         // Graph-space visual rect (expanded) for hit testing
    pub logical_rect: Rect, // Graph-space body rect for snapping/selection
    pub input_style: InputStyle,
    pub inputs: Vec<PortId>,
    pub outputs: Vec<PortId>,
}

#[derive(Clone)]
pub struct HitCache {
    pub nodes: Vec<NodeHit>,
    pub buckets: std::collections::HashMap<(i32, i32), Vec<usize>>,
    pub bucket_size: f32,
}

impl Default for HitCache {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            buckets: std::collections::HashMap::new(),
            bucket_size: 256.0,
        }
    }
}

#[derive(Clone, Default)]
pub struct InfoPanelState {
    pub active_node_id: Option<NodeId>,
    // Prevent "open then immediately close" when the click that opens the panel
    // is also interpreted as an outside click in the same frame.
    pub active_open_frame: u64,
    pub pinned_nodes: std::collections::HashSet<NodeId>,
}

// Single menu state machine - completely eliminate multiple menu system conflicts
#[derive(Clone, Debug, Default)]
pub enum MenuState {
    #[default]
    None,
    /// Node search menu (triggered by double-click/right-click on blank space), open_frame prevents closing in the same frame
    /// context: If triggered from dragging a connection, stores the source port (NodeId, PortId)
    Search {
        pos: Pos2,
        open_frame: u64,
        context: Option<(NodeId, PortId)>,
    },
    /// Node context menu (triggered by right-click on a node, node_id locked when opened)
    Node {
        node_id: NodeId,
        pos: Pos2,
        open_frame: u64,
    },
    /// CDA specific context menu (CDA node / CDA IO node)
    CdaNode {
        node_id: NodeId,
        pos: Pos2,
        open_frame: u64,
    },
}

#[derive(Clone)]
pub struct NodeEditorTab {
    pub tab_id: Uuid,
    pub pan: Vec2,
    pub zoom: f32,
    pub target_pan: Vec2,
    pub target_zoom: f32,
    pub zoom_center: Option<Pos2>,

    pub selection_start: Option<Pos2>,
    pub radial_menu_anim_start_time: Option<f64>,
    pub is_menu_opening: bool,
    pub snap_lines: Vec<(Pos2, Pos2)>,
    pub drag_history: Vec<(f64, Pos2)>,
    pub shaken_node_id: Option<NodeId>,
    pub dragged_node_screen_pos: Option<(NodeId, Rect)>,
    pub drag_pointer_last: Option<Pos2>,
    pub drag_start_positions: HashMap<NodeId, Pos2>,
    /// Accumulated node-move delta during a drag gesture. We update visuals via `cached_nodes`
    /// every frame, and commit this delta into the real `NodeGraph` only on release to avoid
    /// per-frame mutex contention (which can cause global stutter).
    pub pending_node_move_delta: Vec2,
    pub sticky_rect_start: HashMap<StickyNoteId, Rect>,
    pub box_rect_start: HashMap<NetworkBoxId, Rect>,
    pub port_locations: HashMap<(NodeId, PortId), (Pos2, Option<f32>)>,
    pub hit_cache_key: u64,
    pub hit_cache: HitCache,
    pub pending_connection_from: Option<(NodeId, PortId)>,
    pub pending_connections_from: Vec<(NodeId, PortId)>,
    pub pending_wire_waypoints: Vec<Pos2>,
    /// When true, a single-wire drag was released on blank canvas after placing waypoints.
    /// We keep the dashed preview + waypoint "dot" so the user can resume dragging from it.
    pub pending_wire_parked: bool,
    pub single_alt_was_down: bool,
    pub selected_waypoint: Option<(ConnectionId, usize)>,
    pub waypoint_drag_old: Option<(ConnectionId, Vec<Pos2>)>,
    pub copilot_relays: HashMap<Uuid, CopilotRelaySession>,
    pub copilot_relays_cancelled: std::collections::HashSet<String>,
    pub copilot_relay_selected: Option<Uuid>,
    pub copilot_relay_actions: Vec<CopilotRelayAction>,
    pub copilot_backend: CopilotBackend,
    pub gemini_cloud_model: GeminiCloudModel,
    pub copilot_inflight_backend: Option<CopilotBackend>,
    pub copilot_retry_count: u8, // Auto-retry counter (0 = first attempt)
    pub copilot_request_start: Option<std::time::Instant>, // For timeout detection
    pub copilot_cloud_disabled_until: Option<std::time::Instant>,
    pub ghost_multi_agg_name: Option<String>,
    pub snapped_to_port: Option<(NodeId, PortId)>,
    pub did_start_connection_this_frame: bool,
    pub insertion_target: Option<ConnectionId>,
    pub geometry_rev: u64,
    pub is_cutting: bool,
    pub cut_path: Vec<Pos2>,
    pub copied_nodes: Vec<Node>,
    pub node_animations: HashMap<NodeId, NodeAnimation>,
    pub last_clicked_port: Option<(PortId, f64)>,
    pub ghost_tab_last_time: Option<std::time::Instant>,
    pub ghost_tab_burst: u32,
    pub ghost_target_nodes: Option<usize>,
    pub multi_alt_was_down: bool,
    pub merge_target_node: Option<NodeId>,
    pub search_text: String,
    pub create_network_box_request: bool,
    pub elastic_state: Option<ElasticState>,
    pub editing_box_title_id: Option<NetworkBoxId>,
    pub create_sticky_note_request: bool,
    pub editing_sticky_note_title_id: Option<StickyNoteId>,
    pub editing_sticky_note_content_id: Option<StickyNoteId>,
    pub create_promote_note_request: bool,
    pub editing_promote_note_id: Option<PromoteNoteId>,
    pub promote_note_rect_start: HashMap<PromoteNoteId, Rect>,
    pub promote_note_recording_id: Option<PromoteNoteId>,

    // Unified menu state (single source of truth, no more conflicts)
    pub menu_state: MenuState,
    pub menu_search_text: String,
    pub menu_search_cached_categories: Vec<String>,
    pub menu_search_cached_query: String,
    pub menu_search_cached_results: Vec<(String, String)>,

    // Info Panel State
    pub info_panel_state: InfoPanelState,

    // Lightweight cache to avoid re-locking the graph for immutable per-node data each frame.
    pub cached_nodes_rev: u64,
    pub cached_topo_key: u64,
    pub cached_nodes: Vec<NodeSnapshot>,
    pub cached_cda_depth: usize, // Track CDA edit depth changes

    // Interaction feedback
    pub hovered_port: Option<(NodeId, PortId, Rect)>, // Updated per-frame

    // CDA editing state
    pub cda_state: CDAEditorState,
    pub create_cda_request: bool,
    pub ghost_graph: Option<GhostGraph>,
    pub ghost_request_id: Option<String>,
    pub ghost_anchor_graph_pos: Option<Pos2>,
    pub ghost_from: Option<(NodeId, PortId)>,
    pub ghost_dialog: String,
    pub ghost_turns: u32,
    pub ghost_pending_user: Option<String>,
    pub ghost_reason_title: Option<String>,
    pub ghost_reason: Option<String>,
    pub ghost_params: HashMap<String, String>, // AI suggested params: "NodeName.param" -> "value"
    pub box_note_request_id: Option<String>,
    pub box_note_inflight_backend: Option<CopilotBackend>,
    pub box_note_pending_nodes: Vec<NodeId>,
    pub box_note_pending_stickies: Vec<StickyNoteId>,
    // Deep mode (Skill-based multi-turn)
    pub deep_mode: bool,
    pub deep_skill_turns: usize,
    pub deep_history: String,
    // Grid ripple for deep thinking visualization
    pub grid_ripple_start: Option<f32>,
    // Graph explain feature (` key trigger)
    pub explain_request_id: Option<String>,
    pub explain_inflight_backend: Option<CopilotBackend>,
    pub explain_pending_nodes: Vec<NodeId>,
    pub explain_result: Option<(String, String)>, // (title, explanation)
    pub explain_show_pos: Option<Pos2>,

    // Coverlay Panel (Gemini) generation
    pub coverlay_gen_open: bool,
    pub coverlay_gen_anchor: Option<Pos2>,
    pub coverlay_gen_prompt: String,
    pub coverlay_gen_request_id: Option<String>,
    pub coverlay_gen_error: Option<String>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CopilotBackend {
    LocalTiny,
    LocalThink,
    Gemini,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum GeminiCloudModel { Fast, Pro }

impl GeminiCloudModel {
    pub fn model_name(&self) -> &'static str {
        match self {
            Self::Fast => "gemini-3-flash-preview",
            Self::Pro => "gemini-3-pro-preview",
        }
    }
}

impl Default for NodeEditorTab {
    fn default() -> Self {
        Self {
            tab_id: Uuid::new_v4(),
            pan: Vec2::ZERO,
            zoom: 1.0,
            target_pan: Vec2::ZERO,
            target_zoom: 1.0,
            zoom_center: None,

            selection_start: None,
            radial_menu_anim_start_time: None,
            is_menu_opening: false,
            snap_lines: Vec::new(),
            drag_history: Vec::new(),
            shaken_node_id: None,
            dragged_node_screen_pos: None,
            drag_pointer_last: None,
            drag_start_positions: HashMap::new(),
            pending_node_move_delta: Vec2::ZERO,
            sticky_rect_start: HashMap::new(),
            box_rect_start: HashMap::new(),
            port_locations: HashMap::new(),
            hit_cache_key: 0,
            hit_cache: HitCache::default(),
            pending_connection_from: None,
            pending_connections_from: Vec::new(),
            pending_wire_waypoints: Vec::new(),
            pending_wire_parked: false,
            single_alt_was_down: false,
            selected_waypoint: None,
            waypoint_drag_old: None,
            copilot_relays: HashMap::new(),
            copilot_relays_cancelled: std::collections::HashSet::new(),
            copilot_relay_selected: None,
            copilot_relay_actions: Vec::new(),
            copilot_backend: CopilotBackend::LocalTiny,
            gemini_cloud_model: GeminiCloudModel::Fast,
            copilot_inflight_backend: None,
            copilot_retry_count: 0,
            copilot_request_start: None,
            copilot_cloud_disabled_until: None,
            ghost_multi_agg_name: None,
            snapped_to_port: None,
            did_start_connection_this_frame: false,
            insertion_target: None,
            geometry_rev: 0,
            is_cutting: false,
            cut_path: Vec::new(),
            copied_nodes: Vec::new(),
            node_animations: HashMap::new(),
            last_clicked_port: None,
            ghost_tab_last_time: None,
            ghost_tab_burst: 0,
            ghost_target_nodes: None,
            multi_alt_was_down: false,
            merge_target_node: None,
            search_text: "".to_string(),
            create_network_box_request: false,
            elastic_state: None,
            editing_box_title_id: None,
            create_sticky_note_request: false,
            editing_sticky_note_title_id: None,
            editing_sticky_note_content_id: None,
            create_promote_note_request: false,
            editing_promote_note_id: None,
            promote_note_rect_start: HashMap::new(),
            promote_note_recording_id: None,

            menu_state: MenuState::None,
            menu_search_text: String::new(),
            menu_search_cached_categories: Vec::new(),
            menu_search_cached_query: String::new(),
            menu_search_cached_results: Vec::new(),

            info_panel_state: InfoPanelState::default(),

            cached_nodes_rev: 0,
            cached_topo_key: 0,
            cached_nodes: Vec::new(),
            cached_cda_depth: 0,
            hovered_port: None,

            cda_state: CDAEditorState::default(),
            create_cda_request: false,
            ghost_graph: None,
            ghost_request_id: None,
            ghost_anchor_graph_pos: None,
            ghost_from: None,
            ghost_dialog: String::new(),
            ghost_turns: 0,
            ghost_pending_user: None,
            ghost_reason_title: None,
            ghost_reason: None,
            ghost_params: HashMap::new(),
            box_note_request_id: None,
            box_note_inflight_backend: None,
            box_note_pending_nodes: Vec::new(),
            box_note_pending_stickies: Vec::new(),
            deep_mode: false,
            deep_skill_turns: 0,
            deep_history: String::new(),
            grid_ripple_start: None,
            explain_request_id: None,
            explain_inflight_backend: None,
            explain_pending_nodes: Vec::new(),
            explain_result: None,
            explain_show_pos: None,

            coverlay_gen_open: false,
            coverlay_gen_anchor: None,
            coverlay_gen_prompt: String::new(),
            coverlay_gen_request_id: None,
            coverlay_gen_error: None,
        }
    }
}

impl crate::ui::NodeEditorState {
    pub fn execute(
        &mut self,
        mut command: Box<dyn crate::cunning_core::command::Command>,
        graph: &mut crate::nodes::NodeGraph,
    ) {
        command.apply(graph);
        self.undo_stack.push(command, self.cda_path.clone());
    }

    pub fn record(&mut self, command: Box<dyn crate::cunning_core::command::Command>) {
        self.undo_stack.push(command, self.cda_path.clone());
    }

    pub fn undo(&mut self, root_graph: &mut crate::nodes::NodeGraph) {
        if let Some(entry) = self.undo_stack.pop_undo() {
            let mut cmd = entry.cmd;
            crate::tabs_system::node_editor::cda::navigation::with_graph_by_path_mut(
                root_graph,
                &entry.path,
                |g| cmd.revert(g),
            );
            self.undo_stack
                .push_redo(crate::cunning_core::command::StackEntry {
                    cmd,
                    path: entry.path,
                });
        }
    }

    pub fn redo(&mut self, root_graph: &mut crate::nodes::NodeGraph) {
        if let Some(entry) = self.undo_stack.pop_redo() {
            let mut cmd = entry.cmd;
            crate::tabs_system::node_editor::cda::navigation::with_graph_by_path_mut(
                root_graph,
                &entry.path,
                |g| cmd.apply(g),
            );
            self.undo_stack
                .push_undo(crate::cunning_core::command::StackEntry {
                    cmd,
                    path: entry.path,
                });
        }
    }
}
