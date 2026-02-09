use crate::cunning_core::cda::library::{global_cda_library, CdaAssetRef};
use crate::cunning_core::cda::CDAAsset;
use crate::nodes::structs::{
    Connection, ConnectionId, NetworkBox, NetworkBoxId, Node, NodeId, PromoteNote, PromoteNoteId,
    StickyNote, StickyNoteId,
};
use crate::nodes::NodeGraph;
use crate::nodes::NodeType;
use crate::ui::UiState;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use bevy_egui::egui;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectHeader {
    pub version: String,
    pub app_version: String,
    pub uuid: Uuid,
    pub author: String,
    pub created_at: String,
    // adding project header
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectFile {
    pub header: ProjectHeader,
    pub graph: GraphData,
    pub cda_defs: HashMap<Uuid, CDAAsset>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphData {
    pub nodes: HashMap<NodeId, Node>,
    pub connections: HashMap<ConnectionId, Connection>,
    pub sticky_notes: HashMap<StickyNoteId, StickyNote>,
    pub sticky_note_draw_order: Vec<StickyNoteId>,
    pub network_boxes: HashMap<NetworkBoxId, NetworkBox>,
    pub network_box_draw_order: Vec<NetworkBoxId>,
    #[serde(default)]
    pub promote_notes: HashMap<PromoteNoteId, PromoteNote>,
    #[serde(default)]
    pub promote_note_draw_order: Vec<PromoteNoteId>,
    pub display_node: Option<NodeId>,
    pub ui_state: UiStateData,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UiStateData {
    pub pan: Vec2,
    pub zoom: f32,
    // Add more UI state here if needed (e.g. selected nodes, layout mode)
}

impl ProjectFile {
    #[allow(unused_variables)]
    pub fn new(node_graph: &NodeGraph, _ui_state: &UiState) -> Self {
        fn collect_deps(asset: &CDAAsset, out: &mut HashMap<Uuid, CDAAsset>) {
            for n in asset.inner_graph.nodes.values() {
                let NodeType::CDA(d) = &n.node_type else {
                    continue;
                };
                if out.contains_key(&d.asset_ref.uuid) {
                    continue;
                }
                if let Some(lib) = global_cda_library() {
                    if lib.ensure_loaded(&d.asset_ref).is_ok() {
                        if let Some(a) = lib.get(d.asset_ref.uuid) {
                            out.insert(a.id, a.clone());
                            collect_deps(&a, out);
                        }
                    }
                }
            }
        }

        let mut cda_defs: HashMap<Uuid, CDAAsset> = HashMap::new();
        if let Some(lib) = global_cda_library() {
            for n in node_graph.nodes.values() {
                let NodeType::CDA(d) = &n.node_type else {
                    continue;
                };
                if cda_defs.contains_key(&d.asset_ref.uuid) {
                    continue;
                }
                if lib.ensure_loaded(&d.asset_ref).is_ok() {
                    if let Some(a) = lib.get(d.asset_ref.uuid) {
                        cda_defs.insert(a.id, a.clone());
                        collect_deps(&a, &mut cda_defs);
                    }
                }
            }
        }
        Self {
            header: ProjectHeader {
                version: "1.1.0".to_string(),
                app_version: "0.10.0".to_string(),
                uuid: Uuid::new_v4(),
                author: "User".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            },
            graph: GraphData {
                nodes: node_graph.nodes.clone(),
                connections: node_graph.connections.clone(),
                sticky_notes: node_graph.sticky_notes.clone(),
                sticky_note_draw_order: node_graph.sticky_note_draw_order.clone(),
                network_boxes: node_graph.network_boxes.clone(),
                network_box_draw_order: node_graph.network_box_draw_order.clone(),
                promote_notes: node_graph.promote_notes.clone(),
                promote_note_draw_order: node_graph.promote_note_draw_order.clone(),
                display_node: node_graph.display_node,
                ui_state: UiStateData {
                    // EditorTabContext has pan/zoom. UiState has selection.
                    // We need to pass pan/zoom separately or restructure.
                    // Let's check where pan/zoom is stored. It's in NodeEditorTab struct.
                    pan: Vec2::ZERO,
                    zoom: 1.0,
                },
            },
            cda_defs,
        }
    }
}

impl From<GraphData> for NodeGraph {
    fn from(data: GraphData) -> Self {
        let mut g = Self {
            nodes: data.nodes,
            connections: data.connections,
            sticky_notes: data.sticky_notes,
            sticky_note_draw_order: data.sticky_note_draw_order,
            network_boxes: data.network_boxes,
            network_box_draw_order: data.network_box_draw_order,
            promote_notes: data.promote_notes,
            promote_note_draw_order: data.promote_note_draw_order,
            display_node: data.display_node,
            final_geometry: std::sync::Arc::new(crate::mesh::Geometry::new()),
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
            graph_revision: 0,
            param_revision: 0,
            block_id_index: HashMap::new(),
            foreach_scope_nodes: HashSet::new(),
            foreach_scope_epoch: 0,
            foreach_geo_epoch: HashMap::new(),
            foreach_port_epoch: HashMap::new(),
            foreach_port_geo_epoch: HashMap::new(),
            dirty_tracker: Default::default(),
            adjacency_out: None,
            cook_viz: None,
        };
        for n in g.nodes.values_mut() {
            n.rebuild_ports();
        }
        g.rebuild_block_id_index();
        g
    }
}

pub fn save_project(node_graph: &NodeGraph, ui_state: &UiState) {
    let _ = (node_graph, ui_state);
    // Deprecated: blocking OS dialog + I/O. Use AppJobs + in-app file picker instead.
}

pub fn load_project() -> Option<(NodeGraph, UiStateData)> {
    // Deprecated: blocking OS dialog + I/O. Use AppJobs + in-app file picker instead.
    None
}

// -----------------------------
// Non-blocking save/open (Job friendly)
// -----------------------------

pub fn save_project_to_path(
    path: &Path,
    project: &ProjectFile,
) -> Result<(), String> {
    let json = serde_json::to_string_pretty(project)
        .map_err(|e| format!("serialize project failed: {e}"))?;
    atomic_write(path, json.as_bytes())
}

pub fn load_project_from_path(path: &Path) -> Result<ProjectFile, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("read failed: {e}"))?;
    serde_json::from_str(&json).map_err(|e| format!("deserialize failed: {e}"))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project.c3d");
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        file_name,
        std::process::id()
    ));
    std::fs::write(&tmp, bytes).map_err(|e| format!("write tmp failed: {e}"))?;
    // Best-effort replace on Windows: remove then rename.
    let _ = std::fs::remove_file(path);
    std::fs::rename(&tmp, path).map_err(|e| format!("rename failed: {e}"))?;
    Ok(())
}

// -----------------------------
// AppJobs integration
// -----------------------------

pub struct ProjectIoPlugin;

impl Plugin for ProjectIoPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                project_file_picker_to_jobs_system,
                apply_completed_project_jobs_system,
            ),
        );
    }
}

#[derive(Debug)]
pub struct ProjectOpenOutput {
    pub graph: NodeGraph,
    pub ui: UiStateData,
}

#[derive(Debug)]
pub struct ProjectSaveOutput {
    pub path: PathBuf,
}

struct SaveProjectJob {
    path: PathBuf,
    snapshot: Arc<crate::nodes::graph_model::NodeGraphSnapshot>,
    view: UiStateData,
}

impl crate::app_jobs::JobRunnable for SaveProjectJob {
    fn title(&self) -> String {
        "Save Project".to_string()
    }
    fn kind(&self) -> &'static str {
        "save_project"
    }
    fn pool(&self) -> crate::app_jobs::JobPool {
        crate::app_jobs::JobPool::Io
    }
    fn start(
        self: Box<Self>,
        cx: crate::app_jobs::JobContext,
    ) -> bevy::tasks::Task<Result<crate::app_jobs::JobOutput, crate::app_jobs::JobError>> {
        let path = self.path.clone();
        let snapshot = self.snapshot.clone();
        let view = self.view.clone();
        IoTaskPool::get().spawn(async move {
            let _ = cx.progress.send(crate::app_jobs::JobProgress {
                fraction: 0.02,
                message: "Snapshotting...".into(),
            });

            if cx.cancel.is_cancelled() {
                return Err("Cancelled".into());
            }

            let graph_data = GraphData {
                nodes: snapshot.nodes.clone(),
                connections: snapshot.connections.clone(),
                sticky_notes: snapshot.sticky_notes.clone(),
                sticky_note_draw_order: snapshot.sticky_note_draw_order.clone(),
                network_boxes: snapshot.network_boxes.clone(),
                network_box_draw_order: snapshot.network_box_draw_order.clone(),
                promote_notes: snapshot.promote_notes.clone(),
                promote_note_draw_order: snapshot.promote_note_draw_order.clone(),
                display_node: snapshot.display_node,
                ui_state: view,
            };
            let cda_refs = collect_cda_refs_from_nodes(&snapshot.nodes);

            let _ = cx.progress.send(crate::app_jobs::JobProgress {
                fraction: 0.10,
                message: "Collecting CDA dependencies...".into(),
            });

            let mut cda_defs: HashMap<Uuid, CDAAsset> = HashMap::new();
            if let Some(lib) = global_cda_library() {
                // Ensure loaded in background (may touch disk).
                let mut q: VecDeque<CdaAssetRef> = cda_refs.into_iter().collect();
                let mut seen: HashSet<Uuid> = HashSet::new();
                while let Some(r) = q.pop_front() {
                    if !seen.insert(r.uuid) {
                        continue;
                    }
                    if cx.cancel.is_cancelled() {
                        return Err("Cancelled".into());
                    }
                    let _ = lib.ensure_loaded(&r);
                    if let Some(a) = lib.get(r.uuid) {
                        // Enqueue nested deps
                        for n in a.inner_graph.nodes.values() {
                            let NodeType::CDA(d) = &n.node_type else { continue; };
                            q.push_back(d.asset_ref.clone());
                        }
                        cda_defs.insert(a.id, a);
                    }
                }
            }

            let project = ProjectFile {
                header: ProjectHeader {
                    version: "1.1.0".to_string(),
                    app_version: "0.10.0".to_string(),
                    uuid: Uuid::new_v4(),
                    author: "User".to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                },
                graph: graph_data,
                cda_defs,
            };

            let _ = cx.progress.send(crate::app_jobs::JobProgress {
                fraction: 0.70,
                message: "Writing to disk...".into(),
            });

            save_project_to_path(&path, &project).map_err(crate::app_jobs::JobError::from)?;

            let _ = cx.progress.send(crate::app_jobs::JobProgress {
                fraction: 1.0,
                message: "Done".into(),
            });

            Ok(Box::new(ProjectSaveOutput { path }) as crate::app_jobs::JobOutput)
        })
    }
}

struct OpenProjectJob {
    path: PathBuf,
}

impl crate::app_jobs::JobRunnable for OpenProjectJob {
    fn title(&self) -> String {
        "Open Project".to_string()
    }
    fn kind(&self) -> &'static str {
        "open_project"
    }
    fn pool(&self) -> crate::app_jobs::JobPool {
        crate::app_jobs::JobPool::Io
    }
    fn start(
        self: Box<Self>,
        cx: crate::app_jobs::JobContext,
    ) -> bevy::tasks::Task<Result<crate::app_jobs::JobOutput, crate::app_jobs::JobError>> {
        let path = self.path.clone();
        IoTaskPool::get().spawn(async move {
            let _ = cx.progress.send(crate::app_jobs::JobProgress {
                fraction: 0.05,
                message: "Reading file...".into(),
            });
            if cx.cancel.is_cancelled() {
                return Err("Cancelled".into());
            }
            let project = load_project_from_path(&path).map_err(crate::app_jobs::JobError::from)?;
            if let Some(lib) = global_cda_library() {
                for a in project.cda_defs.values() {
                    lib.put(a.clone());
                }
            }
            let ui = project.graph.ui_state.clone();
            let graph = NodeGraph::from(project.graph);
            let _ = cx.progress.send(crate::app_jobs::JobProgress {
                fraction: 1.0,
                message: "Done".into(),
            });
            Ok(Box::new(ProjectOpenOutput { graph, ui }) as crate::app_jobs::JobOutput)
        })
    }
}

fn collect_cda_refs_from_graph(g: &NodeGraph) -> Vec<CdaAssetRef> {
    let mut out = Vec::new();
    for n in g.nodes.values() {
        let NodeType::CDA(d) = &n.node_type else { continue; };
        out.push(d.asset_ref.clone());
    }
    out
}

fn collect_cda_refs_from_nodes(nodes: &HashMap<NodeId, Node>) -> Vec<CdaAssetRef> {
    let mut out = Vec::new();
    for n in nodes.values() {
        let NodeType::CDA(d) = &n.node_type else { continue; };
        out.push(d.asset_ref.clone());
    }
    out
}

fn project_file_picker_to_jobs_system(
    mut chosen: MessageReader<crate::ui::FilePickerChosenEvent>,
    mut jobs: ResMut<crate::app_jobs::AppJobs>,
    snapshot: Res<crate::nodes::graph_model::NodeGraphSnapshotRes>,
    node_editor_state: Res<crate::ui::NodeEditorState>,
) {
    for ev in chosen.read() {
        match ev.mode {
            crate::ui::FilePickerMode::SaveProject | crate::ui::FilePickerMode::SaveProjectAs => {
                let view = UiStateData {
                    pan: Vec2::new(node_editor_state.pan.x, node_editor_state.pan.y),
                    zoom: node_editor_state.zoom,
                };
                jobs.enqueue(Box::new(SaveProjectJob {
                    path: ev.path.clone(),
                    snapshot: snapshot.0.clone(),
                    view,
                }));
            }
            crate::ui::FilePickerMode::OpenProject => {
                jobs.enqueue(Box::new(OpenProjectJob { path: ev.path.clone() }));
            }
        }
    }
}

fn apply_completed_project_jobs_system(
    mut jobs: ResMut<crate::app_jobs::AppJobs>,
    mut node_graph_res: ResMut<crate::NodeGraphResource>,
    mut node_editor_state: ResMut<crate::ui::NodeEditorState>,
    mut graph_changed: MessageWriter<crate::GraphChanged>,
    mut geometry_changed: MessageWriter<crate::GeometryChanged>,
) {
    // Drain completed IDs; apply only project outputs.
    let mut to_apply: Vec<crate::app_jobs::JobId> = Vec::new();
    while let Some(id) = jobs.completed_queue().pop_front() {
        to_apply.push(id);
    }
    for id in to_apply {
        let Some(out) = jobs.take_output(id) else {
            continue;
        };
        if let Ok(o) = out.downcast::<ProjectOpenOutput>() {
            node_graph_res.0 = o.graph;
            // Restore view (best-effort)
            node_editor_state.pan = egui::vec2(o.ui.pan.x, o.ui.pan.y);
            node_editor_state.zoom = o.ui.zoom;
            node_editor_state.target_pan = egui::vec2(o.ui.pan.x, o.ui.pan.y);
            node_editor_state.target_zoom = o.ui.zoom;
            graph_changed.write_default();
            geometry_changed.write_default();
        }
    }
}
