//! Coverlay UI: HUD single-select + Coverlay multi-select for CDA/Voxel editing. (egui version)
use crate::cunning_core::cda::library::global_cda_library;
use crate::launcher::plugin::AppState;
use crate::nodes::parameter::{ParameterUIType, ParameterValue};
use crate::nodes::structs::{NodeGraph, NodeId, NodeType};
use crate::tabs_system::viewport_3d::ViewportLayout;
use crate::tabs_system::{EditorTab, EditorTabContext};
use crate::ui::UiState;
use crate::NodeGraphResource;
use bevy::prelude::*;
use bevy_ecs::system::SystemParam;
use bevy_egui::{egui, EguiContexts};
use egui_dock::{DockState, Node, NodeIndex, SurfaceIndex, TabViewer};
use cunning_kernel::algorithms::algorithms_editor::voxel::{DiscreteVoxelCmdList, DiscreteVoxelOp};
use bvh::aabb::{Aabb, Bounded};
use bvh::bounding_hierarchy::BHShape;
use bvh::bvh::Bvh;
use nalgebra::{Point3, Vector3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use cunning_viewport::coverlay_dock::{CoverlayDockPanel, CoverlayPanelKey, CoverlayPanelKind};
use cunning_viewport::voxel_coverlay_ui as vxui;
use cunning_overlay_widgets::overlay_widgets;

// overlay widgets are shared between editor/player

#[derive(Resource, Default)]
pub struct CoverlayUiWantsInput(pub bool);

pub use vxui::VoxelHudInfo;

pub use vxui::VoxelOverlaySettings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoxelEditTarget {
    Direct(NodeId),
    Cda { inst_id: NodeId, internal_id: NodeId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoverlayTarget { Cda(NodeId), DirectVoxel(NodeId), DirectNode(NodeId) }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum TopTab { #[default] Hud, Coverlay }

pub use vxui::{VoxelToolState, VoxelToolMode, VoxelAddType, VoxelSelectType, VoxelPaintType, VoxelBrushShape};

#[derive(Clone, Copy, Debug)]
struct PanelState { offset: egui::Vec2, drag_base: egui::Vec2, dragging: bool, z: u32 }

#[derive(Resource, Default)]
struct CoverlayPanelStateMap { z: u32, map: HashMap<CoverlayPanelKey, PanelState> }

#[derive(Resource, Default)]
struct CoverlayEguiState { top_tab: TopTab }

#[derive(Clone, Default)]
struct ImportPanelState { path: String, scale: f32, height: i32, mesh_max_voxels: i32 }

#[derive(Resource, Default)]
struct ImportPanelStateMap { map: HashMap<CoverlayPanelKey, ImportPanelState> }

#[derive(Clone, Default)]
struct ExportPanelState { path: String }

#[derive(Resource, Default)]
struct ExportPanelStateMap { map: HashMap<CoverlayPanelKey, ExportPanelState> }

type OpsPanelState = vxui::VoxelOpsPanelState;

#[derive(Resource, Default)]
struct OpsPanelStateMap { map: HashMap<CoverlayPanelKey, OpsPanelState> }

pub struct CoverlayUiPlugin;

impl Plugin for CoverlayUiPlugin {
    fn build(&self, app: &mut App) {
        // Dock-in-viewport is handled by `Viewport3DTab`; keep only shared editor resources here.
        app.init_resource::<CoverlayUiWantsInput>()
            .init_resource::<VoxelToolState>()
            .init_resource::<VoxelOverlaySettings>()
            .init_resource::<VoxelHudInfo>();
    }
}

/// Optional overlay-style Coverlay UI (for external engine integrations).
pub struct CoverlayOverlayUiPlugin;
impl Plugin for CoverlayOverlayUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CoverlayEguiState>()
            .init_resource::<CoverlayPanelStateMap>()
            .init_resource::<ImportPanelStateMap>()
            .init_resource::<ExportPanelStateMap>()
            .init_resource::<OpsPanelStateMap>()
            .add_systems(Update, coverlay_egui_system.run_if(in_state(AppState::Editor)));
    }
}

const PANEL_W: f32 = 280.0;
const PANEL_H_EST: f32 = 260.0;
const VIEWCUBE_SIZE: f32 = 130.0;
const VIEWCUBE_MARGIN_RIGHT: f32 = 75.0;
const VIEWCUBE_MARGIN_TOP: f32 = 20.0;
const VIEWCUBE_GAP_Y: f32 = 10.0;

const VOXEL_CMDS_KEY: &str = "cmds_json";
const VOXEL_PALETTE_KEY: &str = "palette_json";
const VOXEL_MASK_KEY: &str = "mask_json";
const VOXEL_AI_STAMP_PROMPT_KEY: &str = "ai_stamp_prompt";
const VOXEL_AI_STAMP_REF_IMAGE_KEY: &str = "ai_stamp_reference_image";
const VOXEL_AI_STAMP_TILE_RES_KEY: &str = "ai_stamp_tile_res";
const VOXEL_AI_STAMP_PAL_MAX_KEY: &str = "ai_stamp_palette_max";
const VOXEL_AI_STAMP_DEPTH_EPS_KEY: &str = "ai_stamp_depth_eps";

#[inline]
fn is_voxel_edit_node_type(ty: Option<&NodeType>) -> bool {
    ty.is_some_and(|t| t.type_id() == "cunning.voxel.edit")
}

#[inline]
fn read_target_string(g: &NodeGraph, target: VoxelEditTarget, key: &str, d: &str) -> String {
    match target {
        VoxelEditTarget::Direct(node_id) => g.nodes.get(&node_id).and_then(|n| n.parameters.iter().find(|p| p.name == key)).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None }).unwrap_or_else(|| d.to_string()),
        VoxelEditTarget::Cda { inst_id, internal_id } => {
            let Some(inst) = g.nodes.get(&inst_id) else { return d.to_string(); };
            let NodeType::CDA(data) = &inst.node_type else { return d.to_string(); };
            if let Some(s) = data.inner_param_overrides.get(&internal_id).and_then(|m| m.get(key)).and_then(|v| if let ParameterValue::String(s) = v { Some(s.clone()) } else { None }) { return s; }
            let Some(lib) = global_cda_library() else { return d.to_string(); };
            let _ = lib.ensure_loaded(&data.asset_ref);
            let Some(a) = lib.get(data.asset_ref.uuid) else { return d.to_string(); };
            a.inner_graph.nodes.get(&internal_id).and_then(|n| n.parameters.iter().find(|p| p.name == key)).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None }).unwrap_or_else(|| d.to_string())
        }
    }
}

#[inline]
fn read_target_i32(g: &NodeGraph, target: VoxelEditTarget, key: &str, d: i32) -> i32 {
    match target {
        VoxelEditTarget::Direct(node_id) => g.nodes.get(&node_id).and_then(|n| n.parameters.iter().find(|p| p.name == key)).and_then(|p| if let ParameterValue::Int(v) = &p.value { Some(*v) } else { None }).unwrap_or(d),
        VoxelEditTarget::Cda { inst_id, internal_id } => {
            let Some(inst) = g.nodes.get(&inst_id) else { return d; };
            let NodeType::CDA(data) = &inst.node_type else { return d; };
            if let Some(v) = data.inner_param_overrides.get(&internal_id).and_then(|m| m.get(key)).and_then(|v| if let ParameterValue::Int(v) = v { Some(*v) } else { None }) { return v; }
            let Some(lib) = global_cda_library() else { return d; };
            let _ = lib.ensure_loaded(&data.asset_ref);
            let Some(a) = lib.get(data.asset_ref.uuid) else { return d; };
            a.inner_graph.nodes.get(&internal_id).and_then(|n| n.parameters.iter().find(|p| p.name == key)).and_then(|p| if let ParameterValue::Int(v) = &p.value { Some(*v) } else { None }).unwrap_or(d)
        }
    }
}

#[inline]
fn read_target_f32(g: &NodeGraph, target: VoxelEditTarget, key: &str, d: f32) -> f32 {
    match target {
        VoxelEditTarget::Direct(node_id) => g.nodes.get(&node_id).and_then(|n| n.parameters.iter().find(|p| p.name == key)).and_then(|p| if let ParameterValue::Float(v) = &p.value { Some(*v) } else { None }).unwrap_or(d),
        VoxelEditTarget::Cda { inst_id, internal_id } => {
            let Some(inst) = g.nodes.get(&inst_id) else { return d; };
            let NodeType::CDA(data) = &inst.node_type else { return d; };
            if let Some(v) = data.inner_param_overrides.get(&internal_id).and_then(|m| m.get(key)).and_then(|v| if let ParameterValue::Float(v) = v { Some(*v) } else { None }) { return v; }
            let Some(lib) = global_cda_library() else { return d; };
            let _ = lib.ensure_loaded(&data.asset_ref);
            let Some(a) = lib.get(data.asset_ref.uuid) else { return d; };
            a.inner_graph.nodes.get(&internal_id).and_then(|n| n.parameters.iter().find(|p| p.name == key)).and_then(|p| if let ParameterValue::Float(v) = &p.value { Some(*v) } else { None }).unwrap_or(d)
        }
    }
}

#[inline]
fn write_target_value(g: &mut NodeGraph, target: VoxelEditTarget, key: &str, v: ParameterValue) -> Option<NodeId> {
    match target {
        VoxelEditTarget::Direct(node_id) => {
            let n = g.nodes.get_mut(&node_id)?;
            let p = n.parameters.iter_mut().find(|p| p.name == key)?;
            p.value = v;
            Some(node_id)
        }
        VoxelEditTarget::Cda { inst_id, internal_id } => {
            let inst = g.nodes.get_mut(&inst_id)?;
            let NodeType::CDA(data) = &mut inst.node_type else { return None; };
            data.inner_param_overrides.entry(internal_id).or_default().insert(key.to_string(), v);
            Some(inst_id)
        }
    }
}

pub fn resolve_voxel_edit_target(ui: &UiState, g: &NodeGraph) -> Option<VoxelEditTarget> {
    let id = ui
        .last_selected_node_id
        .or_else(|| ui.selected_nodes.iter().next().copied())?;
    let n = g.nodes.get(&id)?;
    match &n.node_type {
        NodeType::VoxelEdit => Some(VoxelEditTarget::Direct(id)),
        NodeType::CDA(data) => {
            let Some(lib) = global_cda_library() else { return None; };
            let _ = lib.ensure_loaded(&data.asset_ref);
            let a = lib.get(data.asset_ref.uuid)?;
            let internal_id = *data.coverlay_units.iter().find(|nid| {
                a.inner_graph
                    .nodes
                    .get(nid)
                    .map_or(false, |n| is_voxel_edit_node_type(Some(&n.node_type)))
            })?;
            Some(VoxelEditTarget::Cda { inst_id: id, internal_id })
        }
        _ => None,
    }
}

pub fn read_voxel_cmds(g: &NodeGraph, target: VoxelEditTarget) -> DiscreteVoxelCmdList {
    match target {
        VoxelEditTarget::Direct(node_id) => g
            .nodes
            .get(&node_id)
            .and_then(|n| n.parameters.iter().find(|p| p.name == VOXEL_CMDS_KEY))
            .and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.as_str()) } else { None })
            .map(DiscreteVoxelCmdList::from_json)
            .unwrap_or_default(),
        VoxelEditTarget::Cda { inst_id, internal_id } => {
            let Some(inst) = g.nodes.get(&inst_id) else { return default(); };
            let NodeType::CDA(data) = &inst.node_type else { return default(); };
            if let Some(s) = data
                .inner_param_overrides
                .get(&internal_id)
                .and_then(|m| m.get(VOXEL_CMDS_KEY))
                .and_then(|v| if let ParameterValue::String(s) = v { Some(s.as_str()) } else { None })
            {
                return DiscreteVoxelCmdList::from_json(s);
            }
            let Some(lib) = global_cda_library() else { return default(); };
            let _ = lib.ensure_loaded(&data.asset_ref);
            let Some(a) = lib.get(data.asset_ref.uuid) else { return default(); };
            a.inner_graph
                .nodes
                .get(&internal_id)
                .and_then(|n| n.parameters.iter().find(|p| p.name == VOXEL_CMDS_KEY))
                .and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.as_str()) } else { None })
                .map(DiscreteVoxelCmdList::from_json)
                .unwrap_or_default()
        }
    }
}

pub fn write_voxel_cmds(g: &mut NodeGraph, target: VoxelEditTarget, cmds: DiscreteVoxelCmdList) -> Option<NodeId> {
    match target {
        VoxelEditTarget::Direct(node_id) => {
            let n = g.nodes.get_mut(&node_id)?;
            if let Some(p) = n.parameters.iter_mut().find(|p| p.name == VOXEL_CMDS_KEY) {
                p.value = ParameterValue::String(cmds.to_json());
            }
            Some(node_id)
        }
        VoxelEditTarget::Cda { inst_id, internal_id } => {
            let inst = g.nodes.get_mut(&inst_id)?;
            let NodeType::CDA(data) = &mut inst.node_type else { return None; };
            data.inner_param_overrides
                .entry(internal_id)
                .or_default()
                .insert(VOXEL_CMDS_KEY.to_string(), ParameterValue::String(cmds.to_json()));
            Some(inst_id)
        }
    }
}

pub fn read_voxel_palette(g: &NodeGraph, target: VoxelEditTarget) -> String {
    match target {
        VoxelEditTarget::Direct(node_id) => g.nodes.get(&node_id).and_then(|n| n.parameters.iter().find(|p| p.name == VOXEL_PALETTE_KEY)).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None }).unwrap_or_else(|| "[]".to_string()),
        VoxelEditTarget::Cda { inst_id, internal_id } => {
            let Some(inst) = g.nodes.get(&inst_id) else { return "[]".to_string(); };
            let NodeType::CDA(data) = &inst.node_type else { return "[]".to_string(); };
            if let Some(s) = data.inner_param_overrides.get(&internal_id).and_then(|m| m.get(VOXEL_PALETTE_KEY)).and_then(|v| if let ParameterValue::String(s) = v { Some(s.clone()) } else { None }) { return s; }
            let Some(lib) = global_cda_library() else { return "[]".to_string(); };
            let _ = lib.ensure_loaded(&data.asset_ref);
            let Some(a) = lib.get(data.asset_ref.uuid) else { return "[]".to_string(); };
            a.inner_graph.nodes.get(&internal_id).and_then(|n| n.parameters.iter().find(|p| p.name == VOXEL_PALETTE_KEY)).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None }).unwrap_or_else(|| "[]".to_string())
        }
    }
}

pub fn write_voxel_palette(g: &mut NodeGraph, target: VoxelEditTarget, palette_json: String) -> Option<NodeId> {
    match target {
        VoxelEditTarget::Direct(node_id) => {
            let n = g.nodes.get_mut(&node_id)?;
            if let Some(p) = n.parameters.iter_mut().find(|p| p.name == VOXEL_PALETTE_KEY) { p.value = ParameterValue::String(palette_json); }
            Some(node_id)
        }
        VoxelEditTarget::Cda { inst_id, internal_id } => {
            let inst = g.nodes.get_mut(&inst_id)?;
            let NodeType::CDA(data) = &mut inst.node_type else { return None; };
            data.inner_param_overrides.entry(internal_id).or_default().insert(VOXEL_PALETTE_KEY.to_string(), ParameterValue::String(palette_json));
            Some(inst_id)
        }
    }
}

pub fn write_voxel_mask(g: &mut NodeGraph, target: VoxelEditTarget, mask_json: String) -> Option<NodeId> {
    match target {
        VoxelEditTarget::Direct(node_id) => {
            let n = g.nodes.get_mut(&node_id)?;
            if let Some(p) = n.parameters.iter_mut().find(|p| p.name == VOXEL_MASK_KEY) {
                p.value = ParameterValue::String(mask_json);
            } else {
                n.parameters.push(crate::nodes::parameter::Parameter::new(
                    VOXEL_MASK_KEY,
                    "Mask (Internal)",
                    "Internal",
                    ParameterValue::String(mask_json),
                    ParameterUIType::Code,
                ));
            }
            Some(node_id)
        }
        VoxelEditTarget::Cda { inst_id, internal_id } => {
            let inst = g.nodes.get_mut(&inst_id)?;
            let NodeType::CDA(data) = &mut inst.node_type else { return None; };
            data.inner_param_overrides.entry(internal_id).or_default().insert(VOXEL_MASK_KEY.to_string(), ParameterValue::String(mask_json));
            Some(inst_id)
        }
    }
}

pub fn voxel_size_for_target(g: &NodeGraph, target: VoxelEditTarget) -> f32 {
    match target {
        VoxelEditTarget::Direct(node_id) => g
            .nodes
            .get(&node_id)
            .and_then(|n| n.parameters.iter().find(|p| p.name == "voxel_size"))
            .and_then(|p| if let ParameterValue::Float(v) = &p.value { Some(*v) } else { None })
            .unwrap_or(0.1),
        VoxelEditTarget::Cda { inst_id, internal_id } => {
            let Some(inst) = g.nodes.get(&inst_id) else { return 0.1; };
            let NodeType::CDA(data) = &inst.node_type else { return 0.1; };
            if let Some(v) = data
                .inner_param_overrides
                .get(&internal_id)
                .and_then(|m| m.get("voxel_size"))
                .and_then(|v| if let ParameterValue::Float(v) = v { Some(*v) } else { None })
            {
                return v;
            }
            let Some(lib) = global_cda_library() else { return 0.1; };
            let _ = lib.ensure_loaded(&data.asset_ref);
            let Some(a) = lib.get(data.asset_ref.uuid) else { return 0.1; };
            a.inner_graph
                .nodes
                .get(&internal_id)
                .and_then(|n| n.parameters.iter().find(|p| p.name == "voxel_size"))
                .and_then(|p| if let ParameterValue::Float(v) = &p.value { Some(*v) } else { None })
                .unwrap_or(0.1)
        }
    }
}

fn pick_target(ui: &UiState, g: &NodeGraph) -> Option<CoverlayTarget> {
    let id = ui.last_selected_node_id.or_else(|| ui.selected_nodes.iter().next().copied())?;
    let n = g.nodes.get(&id)?;
    match &n.node_type {
        NodeType::CDA(_) => Some(CoverlayTarget::Cda(id)),
        NodeType::VoxelEdit => Some(CoverlayTarget::DirectVoxel(id)),
        NodeType::Generic(s) if s.trim() == "Coverlay Panel" => Some(CoverlayTarget::DirectNode(id)),
        _ => None,
    }
}

fn palette_color32(i: u8) -> egui::Color32 {
    if i == 0 { return egui::Color32::TRANSPARENT; }
    let h = (i as f32 * 0.618_033_988_75) % 1.0;
    let (s, v) = (0.65, 0.90);
    let h6 = h * 6.0;
    let c = v * s;
    let x = c * (1.0 - ((h6 % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h6 as i32 { 0 => (c, x, 0.0), 1 => (x, c, 0.0), 2 => (0.0, c, x), 3 => (0.0, x, c), 4 => (x, 0.0, c), _ => (c, 0.0, x) };
    egui::Color32::from_rgb(((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8)
}

#[derive(SystemParam)]
struct CoverlayEguiSystemParams<'w, 's> {
    node_graph_res: ResMut<'w, NodeGraphResource>,
    ui_state: Res<'w, UiState>,
    voxel_state: ResMut<'w, VoxelToolState>,
    voxel_overlay: ResMut<'w, VoxelOverlaySettings>,
    voxel_hud: Res<'w, VoxelHudInfo>,
    voxel_sel: Res<'w, crate::voxel_editor::VoxelSelection>,
    voxel_ai_stamp_queue: ResMut<'w, crate::voxel_editor::VoxelAiPromptStampQueue>,
    display_options: ResMut<'w, crate::viewport_options::DisplayOptions>,
    timeline: ResMut<'w, crate::ui::TimelineState>,
    egui_state: ResMut<'w, CoverlayEguiState>,
    panel_states: ResMut<'w, CoverlayPanelStateMap>,
    import_states: ResMut<'w, ImportPanelStateMap>,
    export_states: ResMut<'w, ExportPanelStateMap>,
    ops_states: ResMut<'w, OpsPanelStateMap>,
    wants_input: ResMut<'w, CoverlayUiWantsInput>,
    graph_changed: MessageWriter<'w, crate::GraphChanged>,
    viewport_layout: Res<'w, ViewportLayout>,
    _phantom: std::marker::PhantomData<&'s ()>,
}

fn coverlay_egui_system(mut ctx: EguiContexts, mut cx: CoverlayEguiSystemParams) {
    let Some(r) = cx.viewport_layout.logical_rect else { return; };
    let egui_ctx = if let Some(e) = cx.viewport_layout.window_entity {
        if let Some(c) = ctx.try_ctx_for_window_mut(e) { c } else { ctx.ctx_mut() }
    } else {
        ctx.ctx_mut()
    };
    let viewcube_right = r.max.x - VIEWCUBE_MARGIN_RIGHT;
    let viewcube_top = r.min.y + VIEWCUBE_MARGIN_TOP;
    let base_left = viewcube_right - PANEL_W;
    let base_top = viewcube_top + VIEWCUBE_SIZE + VIEWCUBE_GAP_Y;
    let frame = egui::Frame::window(&egui_ctx.style()).fill(egui::Color32::from_rgba_unmultiplied(26, 26, 26, 235)).corner_radius(egui::CornerRadius::same(10));
    let mut any_hovered = false;
    let mut any_dragging = false;

    let g = &cx.node_graph_res.0;
    let target = pick_target(&cx.ui_state, &g);
    let Some(target) = target else { cx.wants_input.0 = false; return; };

    struct PanelDesc { key: CoverlayPanelKey, title: String, kind: CoverlayPanelKind, order: i32 }
    let mut panels: Vec<PanelDesc> = Vec::new();

    match target {
        CoverlayTarget::DirectVoxel(node_id) => {
            panels.push(PanelDesc { key: CoverlayPanelKey::DirectVoxel { node_id, kind: CoverlayPanelKind::VoxelTools }, title: "Voxel Tools".to_string(), kind: CoverlayPanelKind::VoxelTools, order: 0 });
            panels.push(PanelDesc { key: CoverlayPanelKey::DirectVoxel { node_id, kind: CoverlayPanelKind::VoxelPalette }, title: "Palette".to_string(), kind: CoverlayPanelKind::VoxelPalette, order: 1 });
        }
        CoverlayTarget::DirectNode(node_id) => {
            panels.push(PanelDesc { key: CoverlayPanelKey::DirectNode { node_id }, title: "Controls".to_string(), kind: CoverlayPanelKind::NodeCoverlay, order: 0 });
        }
        CoverlayTarget::Cda(inst_id) => {
            let Some(inst) = g.nodes.get(&inst_id) else { cx.wants_input.0 = false; return; };
            let NodeType::CDA(data) = &inst.node_type else { cx.wants_input.0 = false; return; };
            panels.push(PanelDesc { key: CoverlayPanelKey::CdaManager { inst_id }, title: "Coverlay".to_string(), kind: CoverlayPanelKind::Manager, order: -1000 });
            let Some(lib) = global_cda_library() else { cx.wants_input.0 = false; return; };
            let Some(asset) = lib.get(data.asset_ref.uuid) else { cx.wants_input.0 = false; return; };
            for u in asset.coverlay_units.iter() {
                if !data.coverlay_units.contains(&u.node_id) { continue; }
                let label = if let Some(ic) = u.icon.as_deref() { format!("{} {}", ic, u.label) } else { u.label.clone() };
                let node_ty = asset.inner_graph.nodes.get(&u.node_id).map(|n| &n.node_type);
                // VoxelEdit exposes *two* coverlay panels (tools + palette).
                if is_voxel_edit_node_type(node_ty) {
                    panels.push(PanelDesc { key: CoverlayPanelKey::CdaVoxel { inst_id, internal_id: u.node_id, kind: CoverlayPanelKind::VoxelTools }, title: label.clone(), kind: CoverlayPanelKind::VoxelTools, order: u.order });
                    panels.push(PanelDesc { key: CoverlayPanelKey::CdaVoxel { inst_id, internal_id: u.node_id, kind: CoverlayPanelKind::VoxelPalette }, title: format!("{label} · Palette"), kind: CoverlayPanelKind::VoxelPalette, order: u.order.saturating_add(1) });
                    continue;
                }
                let kind = panel_kind_for_unit(&u.label, node_ty);
                panels.push(PanelDesc { key: CoverlayPanelKey::CdaUnit { inst_id, unit_id: u.node_id }, title: label, kind, order: u.order });
            }
        }
    }
    panels.sort_by_key(|p| {
        let z = cx.panel_states.map.get(&p.key).map(|s| s.z).unwrap_or(0);
        (z, p.order)
    });
    for (i, pdesc) in panels.iter().enumerate() {
        let mut dragging = false;
        let mut z_val = 0u32;
        let mut offset = cx.panel_states
            .map
            .get(&pdesc.key)
            .map(|s| s.offset)
            .unwrap_or_else(|| {
                let col = (i % 2) as f32;
                let row = (i / 2) as f32;
                egui::vec2((PANEL_W + 12.0) * col, (PANEL_H_EST + 12.0) * row)
            });
        let mut drag_base = offset;
        if let Some(s) = cx.panel_states.map.get(&pdesc.key) { dragging = s.dragging; z_val = s.z; drag_base = s.drag_base; if !dragging && s.offset == egui::Vec2::ZERO && i != 0 { let col = (i % 2) as f32; let row = (i / 2) as f32; offset = egui::vec2((PANEL_W + 12.0) * col, (PANEL_H_EST + 12.0) * row); drag_base = offset; } }
        let pos = egui::pos2(base_left + offset.x, base_top + offset.y);
        let mut open = true;
        let id = egui::Id::new(("coverlay_panel", pdesc.key));
        let mut hovered = false;
        egui::Window::new(&pdesc.title)
            .id(id)
            .frame(frame.clone())
            .fixed_pos(pos)
            .min_width(PANEL_W)
            .title_bar(false)
            .resizable(false)
            .open(&mut open)
            .show(egui_ctx, |ui| {
                hovered = ui.rect_contains_pointer(ui.max_rect());
                // Titlebar (drag + close)
                let bar_h = 22.0;
                let (bar, resp) = ui.allocate_exact_size(egui::vec2(PANEL_W - 10.0, bar_h), egui::Sense::drag());
                let close_rect = egui::Rect::from_min_size(egui::pos2(bar.max.x - 20.0, bar.min.y + 2.0), egui::vec2(18.0, bar_h - 4.0));
                let close_id = egui::Id::new(("coverlay_panel_close", pdesc.key));
                let close_resp = ui.interact(close_rect, close_id, egui::Sense::click());
                if close_resp.clicked() { ui.close(); }
                if resp.hovered() || close_resp.hovered() { ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand); }
                if resp.drag_started() { cx.panel_states.z = cx.panel_states.z.wrapping_add(1); z_val = cx.panel_states.z; }
                if resp.dragged() { offset += resp.drag_delta(); dragging = true; }
                if resp.drag_stopped() { dragging = false; }
                let bar_fill = if resp.dragged() { egui::Color32::from_rgba_unmultiplied(60, 60, 60, 160) } else if resp.hovered() { egui::Color32::from_rgba_unmultiplied(60, 60, 60, 110) } else { egui::Color32::from_rgba_unmultiplied(0, 0, 0, 0) };
                ui.painter().rect_filled(bar, egui::CornerRadius::same(6), bar_fill);
                ui.painter().text(bar.left_center() + egui::vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, format!("≡ {}", pdesc.title), egui::FontId::proportional(14.0), egui::Color32::from_gray(190));
                let x_col = if close_resp.hovered() { egui::Color32::WHITE } else { egui::Color32::from_gray(160) };
                ui.painter().text(close_rect.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(14.0), x_col);
                ui.separator();

                match pdesc.kind {
                    CoverlayPanelKind::Manager => draw_cda_manager(ui, &mut cx.node_graph_res, &cx.ui_state, &mut cx.egui_state, pdesc.key, &mut cx.graph_changed),
                    CoverlayPanelKind::VoxelTools => {
                        struct Backend<'a, 'w> {
                            ngr: &'a mut NodeGraphResource,
                            ui_state: &'a UiState,
                            key: CoverlayPanelKey,
                            gc: &'a mut MessageWriter<'w, crate::GraphChanged>,
                            sel_cells: Vec<IVec3>,
                        }
                        impl<'a, 'w> vxui::VoxelToolsBackend for Backend<'a, 'w> {
                            fn selection_cells(&self) -> &[IVec3] { &self.sel_cells }
                            fn undo(&mut self) {
                                let (cda_inst, direct_node) = match self.key {
                                    CoverlayPanelKey::DirectVoxel { node_id, .. } => (None, Some(node_id)),
                                    CoverlayPanelKey::CdaUnit { inst_id, .. } | CoverlayPanelKey::CdaVoxel { inst_id, .. } => (Some(inst_id), None),
                                    _ => (None, None),
                                };
                                do_undo_redo(self.ngr, self.gc, cda_inst, direct_node, true);
                            }
                            fn redo(&mut self) {
                                let (cda_inst, direct_node) = match self.key {
                                    CoverlayPanelKey::DirectVoxel { node_id, .. } => (None, Some(node_id)),
                                    CoverlayPanelKey::CdaUnit { inst_id, .. } | CoverlayPanelKey::CdaVoxel { inst_id, .. } => (Some(inst_id), None),
                                    _ => (None, None),
                                };
                                do_undo_redo(self.ngr, self.gc, cda_inst, direct_node, false);
                            }
                            fn push_op(&mut self, op: DiscreteVoxelOp) {
                                push_voxel_cmd(self.ngr, self.ui_state, self.key, self.gc, op);
                            }
                        }
                        let ops = cx.ops_states.map.entry(pdesc.key).or_default();
                        let mut backend = Backend {
                            ngr: &mut cx.node_graph_res,
                            ui_state: &cx.ui_state,
                            key: pdesc.key,
                            gc: &mut cx.graph_changed,
                            sel_cells: cx.voxel_sel.cells.iter().copied().collect(),
                        };
                        vxui::draw_voxel_tools_panel(ui, &mut cx.voxel_state, ops, pdesc.key, &mut backend, &mut cx.voxel_overlay, &cx.voxel_hud, &mut cx.display_options);
                    }
                    CoverlayPanelKind::VoxelPalette => vxui::draw_voxel_palette_panel(ui, &mut cx.voxel_state),
                    CoverlayPanelKind::VoxelDebug | CoverlayPanelKind::Import | CoverlayPanelKind::Export => {} // Merged into VoxelTools
                    CoverlayPanelKind::Anim => draw_anim_panel(ui, &mut cx.timeline),
                    CoverlayPanelKind::NodeCoverlay => draw_node_coverlay_panel(ui, &mut cx.node_graph_res, pdesc.key, &mut cx.graph_changed),
                    CoverlayPanelKind::Parameters => {} // Player-only
                }
            });
        if hovered { any_hovered = true; }
        if dragging { any_dragging = true; }
        if z_val != 0 { cx.panel_states.z = z_val; }
        cx.panel_states.map.insert(pdesc.key, PanelState { offset, drag_base, dragging, z: z_val });
        if !open { close_panel_key(&mut cx.node_graph_res, pdesc.key); }
    }
    cx.wants_input.0 = any_hovered || any_dragging;
}

fn panel_kind_for_unit(label: &str, node_type: Option<&NodeType>) -> CoverlayPanelKind {
    let l = label.to_ascii_lowercase();
    if l.contains("palette") { return CoverlayPanelKind::VoxelPalette; }
    if l.contains("anim") || l.contains("animation") { return CoverlayPanelKind::Anim; }
    // Debug/Import/Export merged into VoxelTools; "tools", "debug", "import", "export" -> VoxelTools
    if l.contains("tools") || l.contains("debug") || l.contains("import") || l.contains("export") { return CoverlayPanelKind::VoxelTools; }
    if is_voxel_edit_node_type(node_type) { return CoverlayPanelKind::VoxelTools; }
    if matches!(node_type, Some(NodeType::Generic(s)) if s.trim() == "Coverlay Panel") { return CoverlayPanelKind::NodeCoverlay; }
    CoverlayPanelKind::VoxelTools
}

fn close_panel_key(ngr: &mut NodeGraphResource, key: CoverlayPanelKey) {
    let (inst_id, unit_id) = match key {
        CoverlayPanelKey::CdaUnit { inst_id, unit_id } => (inst_id, unit_id),
        CoverlayPanelKey::CdaVoxel { inst_id, internal_id, .. } => (inst_id, internal_id),
        _ => return,
    };
    let g = &mut ngr.0;
    if let Some(n) = g.nodes.get_mut(&inst_id) {
        if let NodeType::CDA(d) = &mut n.node_type {
            d.coverlay_units.retain(|x| *x != unit_id);
            g.mark_dirty(inst_id);
        }
    }
}

const COVERLAY_BINDINGS_KEY: &str = "bindings_json";

#[derive(Clone, Debug, serde::Deserialize)]
struct CoverlayBindingsDoc { #[serde(default)] title: String, #[serde(default)] bindings: Vec<CoverlayBinding> }

#[derive(Clone, Debug, serde::Deserialize)]
struct CoverlayBinding {
    target_node: String,
    target_param: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    ui: CoverlayUiSpec,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
struct CoverlayUiSpec { #[serde(default)] kind: String } // "auto"

fn read_node_string_param(params: &[crate::nodes::parameter::Parameter], key: &str) -> Option<String> {
    params.iter().find(|p| p.name == key).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None })
}

fn parse_uuid(s: &str) -> Option<NodeId> { uuid::Uuid::parse_str(s.trim()).ok() }

fn read_coverlay_bindings_doc(g: &NodeGraph, inst_id: Option<NodeId>, unit_id: NodeId) -> Option<CoverlayBindingsDoc> {
    let raw = if let Some(inst_id) = inst_id {
        let inst = g.nodes.get(&inst_id)?;
        let NodeType::CDA(data) = &inst.node_type else { return None; };
        data.inner_param_overrides.get(&unit_id).and_then(|m| m.get(COVERLAY_BINDINGS_KEY)).and_then(|v| if let ParameterValue::String(s) = v { Some(s.clone()) } else { None }).unwrap_or_else(|| {
            let Some(lib) = global_cda_library() else { return String::new(); };
            let _ = lib.ensure_loaded(&data.asset_ref);
            let Some(a) = lib.get(data.asset_ref.uuid) else { return String::new(); };
            a.inner_graph.nodes.get(&unit_id).and_then(|n| read_node_string_param(&n.parameters, COVERLAY_BINDINGS_KEY)).unwrap_or_default()
        })
    } else {
        g.nodes.get(&unit_id).and_then(|n| read_node_string_param(&n.parameters, COVERLAY_BINDINGS_KEY)).unwrap_or_default()
    };
    let raw = raw.trim();
    if raw.is_empty() { return None; }
    serde_json::from_str::<CoverlayBindingsDoc>(raw).ok()
}

fn write_cda_override(g: &mut NodeGraph, inst_id: NodeId, target_node: NodeId, target_param: &str, v: ParameterValue) {
    let Some(inst) = g.nodes.get_mut(&inst_id) else { return; };
    let NodeType::CDA(data) = &mut inst.node_type else { return; };
    data.inner_param_overrides.entry(target_node).or_default().insert(target_param.to_string(), v);
}

fn draw_param_ui(ui: &mut egui::Ui, v: &mut ParameterValue, ui_ty: &ParameterUIType) -> bool {
    match (ui_ty, v) {
        (ParameterUIType::FloatSlider { min, max }, ParameterValue::Float(x)) => ui.add(egui::Slider::new(x, *min..=*max).show_value(true)).changed(),
        (ParameterUIType::IntSlider { min, max }, ParameterValue::Int(x)) => ui.add(egui::Slider::new(x, *min..=*max).show_value(true)).changed(),
        (ParameterUIType::Toggle, ParameterValue::Bool(x)) => ui.checkbox(x, "").changed(),
        (ParameterUIType::Vec2Drag, ParameterValue::Vec2(x)) => ui.horizontal(|ui| ui.add(egui::DragValue::new(&mut x.x).speed(0.01)).changed() | ui.add(egui::DragValue::new(&mut x.y).speed(0.01)).changed()).inner,
        (ParameterUIType::Vec3Drag, ParameterValue::Vec3(x)) => ui.horizontal(|ui| ui.add(egui::DragValue::new(&mut x.x).speed(0.01)).changed() | ui.add(egui::DragValue::new(&mut x.y).speed(0.01)).changed() | ui.add(egui::DragValue::new(&mut x.z).speed(0.01)).changed()).inner,
        (ParameterUIType::Vec4Drag, ParameterValue::Vec4(x)) => ui.horizontal(|ui| ui.add(egui::DragValue::new(&mut x.x).speed(0.01)).changed() | ui.add(egui::DragValue::new(&mut x.y).speed(0.01)).changed() | ui.add(egui::DragValue::new(&mut x.z).speed(0.01)).changed() | ui.add(egui::DragValue::new(&mut x.w).speed(0.01)).changed()).inner,
        (ParameterUIType::String, ParameterValue::String(s)) => ui.text_edit_singleline(s).changed(),
        (ParameterUIType::Dropdown { choices }, ParameterValue::Int(sel)) => {
            let cur = choices.iter().find(|(_, v)| v == sel).map(|(s, _)| s.as_str()).unwrap_or("?");
            let mut changed = false;
            egui::ComboBox::from_id_salt(ui.next_auto_id()).selected_text(cur).show_ui(ui, |ui| {
                for (label, v) in choices {
                    if ui.selectable_label(*sel == *v, label).clicked() { *sel = *v; changed = true; }
                }
            });
            changed
        }
        (ParameterUIType::Color { show_alpha }, ParameterValue::Color(rgb)) => {
            let mut c = [rgb.x, rgb.y, rgb.z];
            let r = ui.color_edit_button_rgb(&mut c).changed();
            if r { *rgb = bevy::prelude::Vec3::new(c[0], c[1], c[2]); }
            if *show_alpha { ui.label("Alpha not available"); }
            r
        }
        (ParameterUIType::Color { show_alpha }, ParameterValue::Color4(rgba)) => {
            let mut c = [rgba.x, rgba.y, rgba.z, rgba.w];
            let r = if *show_alpha { ui.color_edit_button_rgba_unmultiplied(&mut c).changed() } else { ui.color_edit_button_rgb(&mut [c[0], c[1], c[2]]).changed() };
            if r { *rgba = bevy::prelude::Vec4::new(c[0], c[1], c[2], if *show_alpha { c[3] } else { rgba.w }); }
            r
        }
        _ => { ui.label("(Unsupported)"); false }
    }
}

fn draw_node_coverlay_panel(ui: &mut egui::Ui, ngr: &mut NodeGraphResource, key: CoverlayPanelKey, gc: &mut MessageWriter<crate::GraphChanged>) {
    let (inst_id, unit_id) = match key { CoverlayPanelKey::DirectNode { node_id } => (None, node_id), CoverlayPanelKey::CdaUnit { inst_id, unit_id } => (Some(inst_id), unit_id), _ => return };
    let g = &mut ngr.0;
    let Some(doc) = read_coverlay_bindings_doc(g, inst_id, unit_id) else { ui.label("Missing or invalid bindings_json."); return; };
    if !doc.title.trim().is_empty() { ui.heading(doc.title.trim()); }
    for b in doc.bindings {
        let Some(tid) = parse_uuid(&b.target_node) else { ui.colored_label(egui::Color32::LIGHT_RED, format!("Bad target_node: {}", b.target_node)); continue; };
        let label = if b.label.trim().is_empty() { b.target_param.clone() } else { b.label.clone() };
        let ui_kind = b.ui.kind.trim();
        ui.horizontal(|ui| {
            ui.label(label);
            if !ui_kind.is_empty() && ui_kind != "auto" { ui.colored_label(egui::Color32::YELLOW, format!("ui.kind={} (unsupported)", ui_kind)); }
            if let Some(inst_id) = inst_id {
                let inst = g.nodes.get(&inst_id); let Some(inst) = inst else { ui.label("(missing inst)"); return; };
                let NodeType::CDA(data) = &inst.node_type else { ui.label("(not CDA)"); return; };
                let Some(lib) = global_cda_library() else { ui.label("(no CDA lib)"); return; };
                let Some(a) = lib.get(data.asset_ref.uuid) else { ui.label("(missing asset)"); return; };
                let Some(n) = a.inner_graph.nodes.get(&tid) else { ui.label("(missing node)"); return; };
                let Some(pdef) = n.parameters.iter().find(|p| p.name == b.target_param) else { ui.label("(missing param)"); return; };
                let cur = data.inner_param_overrides.get(&tid).and_then(|m| m.get(&b.target_param)).cloned().unwrap_or_else(|| pdef.value.clone());
                let mut v = cur;
                if draw_param_ui(ui, &mut v, &pdef.ui_type) {
                    write_cda_override(g, inst_id, tid, &b.target_param, v);
                    g.mark_dirty(inst_id);
                    gc.write_default();
                }
            } else {
                let Some(n) = g.nodes.get_mut(&tid) else { ui.label("(missing node)"); return; };
                let Some(p) = n.parameters.iter_mut().find(|p| p.name == b.target_param) else { ui.label("(missing param)"); return; };
                if draw_param_ui(ui, &mut p.value, &p.ui_type) { g.mark_dirty(tid); gc.write_default(); }
            }
        });
    }
}

fn draw_cda_manager(ui: &mut egui::Ui, ngr: &mut NodeGraphResource, ui_state: &UiState, egui_state: &mut CoverlayEguiState, key: CoverlayPanelKey, gc: &mut MessageWriter<crate::GraphChanged>) {
    let CoverlayPanelKey::CdaManager { inst_id } = key else { return; };
    let g = &ngr.0;
    let Some(inst) = g.nodes.get(&inst_id) else { return; };
    let NodeType::CDA(data) = &inst.node_type else { return; };
    let Some(lib) = global_cda_library() else { return; };
    let Some(asset) = lib.get(data.asset_ref.uuid) else { return; };
    let hud_units: Vec<_> = asset.hud_units.iter().cloned().collect();
    let coverlay_units: Vec<_> = asset.coverlay_units.iter().cloned().collect();
    let cur_hud = data.coverlay_hud;
    let cur_coverlays: Vec<NodeId> = data.coverlay_units.clone();
    let _ = ui_state;
    ui.horizontal(|ui| {
        if ui.selectable_label(egui_state.top_tab == TopTab::Hud, "HUD").clicked() { egui_state.top_tab = TopTab::Hud; }
        if ui.selectable_label(egui_state.top_tab == TopTab::Coverlay, "Coverlay").clicked() { egui_state.top_tab = TopTab::Coverlay; }
    });
    ui.separator();
    match egui_state.top_tab {
        TopTab::Hud => {
            let hud_txt = cur_hud.and_then(|id| hud_units.iter().find(|u| u.node_id == id).map(|u| u.label.as_str())).unwrap_or("(None)");
            ui.label(format!("Current HUD: {}", hud_txt));
            for u in &hud_units {
                let sel = Some(u.node_id) == cur_hud;
                if ui.radio(sel, &u.label).clicked() && !sel {
                    let g = &mut ngr.0;
                    if let Some(n) = g.nodes.get_mut(&inst_id) {
                        if let NodeType::CDA(d) = &mut n.node_type { d.coverlay_hud = Some(u.node_id); }
                    }
                    g.mark_dirty(inst_id);
                    gc.write_default();
                }
            }
        }
        TopTab::Coverlay => {
            for u in &coverlay_units {
                let mut on = cur_coverlays.iter().any(|x| *x == u.node_id);
                let txt = if let Some(ic) = u.icon.as_deref() { format!("{} {}", ic, u.label) } else { u.label.clone() };
                if ui.checkbox(&mut on, txt).changed() {
                    let g = &mut ngr.0;
                    if let Some(n) = g.nodes.get_mut(&inst_id) {
                        if let NodeType::CDA(d) = &mut n.node_type {
                            if !on { d.coverlay_units.retain(|x| *x != u.node_id); }
                            else if !d.coverlay_units.contains(&u.node_id) { d.coverlay_units.push(u.node_id); }
                        }
                    }
                    g.mark_dirty(inst_id);
                    gc.write_default();
                }
            }
        }
    }
}

fn draw_voxel_tools_panel(
    ui: &mut egui::Ui, st: &mut VoxelToolState, ngr: &mut NodeGraphResource, ui_state: &UiState,
    sel: &crate::voxel_editor::VoxelSelection, ai_q: &mut crate::voxel_editor::VoxelAiPromptStampQueue, ops: &mut OpsPanelStateMap, gc: &mut MessageWriter<crate::GraphChanged>,
    ov: &mut VoxelOverlaySettings, hud: &VoxelHudInfo, display_options: &mut crate::viewport_options::DisplayOptions,
    import_states: &mut ImportPanelStateMap, export_states: &mut ExportPanelStateMap, key: CoverlayPanelKey,
) {
    let (cda_inst, direct_node) = match key {
        CoverlayPanelKey::DirectVoxel { node_id, .. } => (None, Some(node_id)),
        CoverlayPanelKey::CdaUnit { inst_id, .. } | CoverlayPanelKey::CdaVoxel { inst_id, .. } => (Some(inst_id), None),
        _ => (None, None),
    };
    overlay_widgets::panel_frame(ui, |ui| {
        // Keep spacing theme-driven (no local overrides).
        ui.horizontal(|ui| {
            overlay_widgets::toolbar(ui, false, |ui| {
                overlay_widgets::icon_select(ui, &mut st.mode, VoxelToolMode::Add, "＋", "Tool: Add (1)");
                overlay_widgets::icon_select(ui, &mut st.mode, VoxelToolMode::Select, "▢", "Tool: Select (2)");
                overlay_widgets::icon_select(ui, &mut st.mode, VoxelToolMode::Move, "↔", "Tool: Move (3)");
                overlay_widgets::icon_select(ui, &mut st.mode, VoxelToolMode::Paint, "🖌", "Tool: Paint (4)");
                overlay_widgets::icon_select(ui, &mut st.mode, VoxelToolMode::Extrude, "⬆", "Tool: Extrude (5)");
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                draw_import_export_icons(ui, ngr, ui_state, key, import_states, export_states, gc);
            });
        });
        overlay_widgets::hsep(ui);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            overlay_widgets::group(ui, "Actions", true, |ui| {
                overlay_widgets::toolbar(ui, false, |ui| {
                    if overlay_widgets::icon_button(ui, "↶", false).on_hover_text("Undo (Z)").clicked() { do_undo_redo(ngr, gc, cda_inst, direct_node, true); }
                    if overlay_widgets::icon_button(ui, "↷", false).on_hover_text("Redo (Y)").clicked() { do_undo_redo(ngr, gc, cda_inst, direct_node, false); }
                    if overlay_widgets::icon_button(ui, "🗑", false).on_hover_text("Clear all voxels").clicked() { push_voxel_cmd(ngr, ui_state, key, gc, DiscreteVoxelOp::ClearAll); }
                    if overlay_widgets::icon_button(ui, "✂", false).on_hover_text("Trim bounds to origin").clicked() { push_voxel_cmd(ngr, ui_state, key, gc, DiscreteVoxelOp::TrimToOrigin); }
                });
            });

            overlay_widgets::group(ui, "Mode", true, |ui| {
                match st.mode {
                    VoxelToolMode::Add => {
                        overlay_widgets::toolbar(ui, false, |ui| {
                            overlay_widgets::icon_select(ui, &mut st.add_type, VoxelAddType::Point, "•", "Add: Point");
                            overlay_widgets::icon_select(ui, &mut st.add_type, VoxelAddType::Line, "╱", "Add: Line");
                            overlay_widgets::icon_select(ui, &mut st.add_type, VoxelAddType::Region, "▦", "Add: Region");
                            overlay_widgets::icon_select(ui, &mut st.add_type, VoxelAddType::Extrude, "⬆", "Add: Extrude");
                            overlay_widgets::icon_select(ui, &mut st.add_type, VoxelAddType::Clay, "☁", "Add: Clay");
                            overlay_widgets::icon_select(ui, &mut st.add_type, VoxelAddType::Smooth, "≈", "Add: Smooth");
                            overlay_widgets::icon_select(ui, &mut st.add_type, VoxelAddType::Clone, "⧉", "Add: Clone");
                        });
                        overlay_widgets::toggle_button(ui, &mut st.clone_overwrite, "Overwrite");
                    }
                    VoxelToolMode::Select => {
                        overlay_widgets::toolbar(ui, false, |ui| {
                            overlay_widgets::icon_select(ui, &mut st.select_type, VoxelSelectType::Point, "•", "Select: Point");
                            overlay_widgets::icon_select(ui, &mut st.select_type, VoxelSelectType::Line, "╱", "Select: Line");
                            overlay_widgets::icon_select(ui, &mut st.select_type, VoxelSelectType::Region, "▦", "Select: Region");
                            overlay_widgets::icon_select(ui, &mut st.select_type, VoxelSelectType::Face, "▣", "Select: Face");
                            overlay_widgets::icon_select(ui, &mut st.select_type, VoxelSelectType::Rect, "▭", "Select: Rect");
                            overlay_widgets::icon_select(ui, &mut st.select_type, VoxelSelectType::Color, "🎨", "Select: Color");
                        });
                    }
                    VoxelToolMode::Paint => {
                        overlay_widgets::toolbar(ui, false, |ui| {
                            overlay_widgets::icon_select(ui, &mut st.paint_type, VoxelPaintType::Point, "•", "Paint: Point");
                            overlay_widgets::icon_select(ui, &mut st.paint_type, VoxelPaintType::Line, "╱", "Paint: Line");
                            overlay_widgets::icon_select(ui, &mut st.paint_type, VoxelPaintType::Region, "▦", "Paint: Region");
                            overlay_widgets::icon_select(ui, &mut st.paint_type, VoxelPaintType::Face, "▣", "Paint: Face");
                            overlay_widgets::icon_select(ui, &mut st.paint_type, VoxelPaintType::ColorPick, "👁", "Paint: Pick");
                            overlay_widgets::icon_select(ui, &mut st.paint_type, VoxelPaintType::PromptStamp, "AI", "Paint: AI Stamp (selection-driven)");
                        });
                    }
                    _ => {}
                }
            });

            overlay_widgets::group(ui, "Brush", true, |ui| {
                overlay_widgets::toolbar(ui, false, |ui| {
                    overlay_widgets::icon_select(ui, &mut st.shape, VoxelBrushShape::Sphere, "⚪", "Shape: Sphere");
                    overlay_widgets::icon_select(ui, &mut st.shape, VoxelBrushShape::Cube, "⬜", "Shape: Cube");
                    overlay_widgets::icon_select(ui, &mut st.shape, VoxelBrushShape::Cylinder, "⬭", "Shape: Cylinder");
                    overlay_widgets::icon_select(ui, &mut st.shape, VoxelBrushShape::Cross, "✚", "Shape: Cross");
                    overlay_widgets::icon_select(ui, &mut st.shape, VoxelBrushShape::CrossWall, "╋", "Shape: Cross Wall");
                    overlay_widgets::icon_select(ui, &mut st.shape, VoxelBrushShape::Diamond, "◆", "Shape: Diamond");
                });
                overlay_widgets::axis_toggle(ui, &mut st.sym_x, &mut st.sym_y, &mut st.sym_z);
                overlay_widgets::styled_slider(ui, &mut st.brush_radius, 0.05..=5.0, "Radius");
            });

            if st.mode == VoxelToolMode::Paint && st.paint_type == VoxelPaintType::PromptStamp {
                overlay_widgets::group(ui, "AI Stamp", true, |ui| {
                    let g_ro = &ngr.0;
                    let target = panel_voxel_target(ui_state, &g_ro, key).or_else(|| resolve_voxel_edit_target(ui_state, &g_ro));
                    let Some(target) = target else { ui.label("Select a Voxel Edit target."); return; };
                    let mut prompt = read_target_string(&g_ro, target, VOXEL_AI_STAMP_PROMPT_KEY, "A stylized but physically plausible material.");
                    let mut ref_img = read_target_string(&g_ro, target, VOXEL_AI_STAMP_REF_IMAGE_KEY, "");
                    let mut tile_res = read_target_i32(&g_ro, target, VOXEL_AI_STAMP_TILE_RES_KEY, 384).clamp(128, 1024);
                    let mut pal_max = read_target_i32(&g_ro, target, VOXEL_AI_STAMP_PAL_MAX_KEY, 64).clamp(4, 200);
                    let mut depth_eps = read_target_f32(&g_ro, target, VOXEL_AI_STAMP_DEPTH_EPS_KEY, 0.02).clamp(0.0, 0.2);

                    overlay_widgets::label_secondary(ui, "Prompt");
                    let ch0 = ui.text_edit_multiline(&mut prompt).changed();
                    overlay_widgets::label_secondary(ui, "Reference Image (optional)");
                    let ch1 = ui.text_edit_singleline(&mut ref_img).changed();
                    ui.horizontal(|ui| {
                        overlay_widgets::label_secondary(ui, "Tile");
                        let ch2 = ui.add(egui::DragValue::new(&mut tile_res).speed(8).range(128..=1024)).changed();
                        overlay_widgets::label_secondary(ui, "PalMax");
                        let ch3 = ui.add(egui::DragValue::new(&mut pal_max).speed(1).range(4..=200)).changed();
                        let _ = (ch2, ch3);
                    });
                    ui.horizontal(|ui| {
                        overlay_widgets::label_secondary(ui, "DepthEps");
                        let ch4 = ui.add(egui::DragValue::new(&mut depth_eps).speed(0.005).range(0.0..=0.2)).changed();
                        let can = !sel.cells.is_empty();
                        let resp = overlay_widgets::action_button(ui, "Stamp");
                        if resp.clicked() {
                            if can {
                                let g = &mut ngr.0;
                                let _ = write_target_value(g, target, VOXEL_AI_STAMP_PROMPT_KEY, ParameterValue::String(prompt.clone())).map(|d| g.mark_dirty(d));
                                let _ = write_target_value(g, target, VOXEL_AI_STAMP_REF_IMAGE_KEY, ParameterValue::String(ref_img.clone())).map(|d| g.mark_dirty(d));
                                let _ = write_target_value(g, target, VOXEL_AI_STAMP_TILE_RES_KEY, ParameterValue::Int(tile_res)).map(|d| g.mark_dirty(d));
                                let _ = write_target_value(g, target, VOXEL_AI_STAMP_PAL_MAX_KEY, ParameterValue::Int(pal_max)).map(|d| g.mark_dirty(d));
                                let _ = write_target_value(g, target, VOXEL_AI_STAMP_DEPTH_EPS_KEY, ParameterValue::Float(depth_eps)).map(|d| g.mark_dirty(d));
                                let voxel_size = voxel_size_for_target(&g, target).max(0.001);
                                let cmds_json = read_voxel_cmds(&g, target).to_json();
                                let palette_json = read_voxel_palette(&g, target);
                                let mut cells: Vec<IVec3> = sel.cells.iter().copied().collect();
                                cells.sort_by(|a, b| (a.z, a.y, a.x).cmp(&(b.z, b.y, b.x)));
                                ai_q.queue.push(crate::voxel_editor::VoxelAiPromptStampRequest {
                                    target,
                                    voxel_size,
                                    cmds_json,
                                    palette_json,
                                    cells,
                                    prompt: prompt.clone(),
                                    reference_image: ref_img.clone(),
                                    tile_res,
                                    palette_max: pal_max,
                                    depth_eps,
                                });
                                gc.write_default();
                            }
                        }
                        if ch4 { let _ = ch4; }
                    });
                    if ch0 || ch1 {
                        let g = &mut ngr.0;
                        let _ = write_target_value(g, target, VOXEL_AI_STAMP_PROMPT_KEY, ParameterValue::String(prompt)).map(|d| g.mark_dirty(d));
                        let _ = write_target_value(g, target, VOXEL_AI_STAMP_REF_IMAGE_KEY, ParameterValue::String(ref_img)).map(|d| g.mark_dirty(d));
                        gc.write_default();
                    }
                    if sel.cells.is_empty() { ui.label("Tip: Select a region first (Select tool), then Stamp."); }
                });
            }

            let st_ops = ops.map.entry(key).or_insert_with(|| OpsPanelState { tab: 0, perlin_min: IVec3::ZERO, perlin_max: IVec3::new(32, 32, 32), perlin_scale: 0.08, perlin_threshold: 0.1, perlin_seed: 1, dup_delta: IVec3::new(1, 0, 0) });
            overlay_widgets::group(ui, "Generate", true, |ui| {
                overlay_widgets::segmented_tabs(ui, ("voxel_ops_tabs", key), &mut st_ops.tab, &["Perlin", "Duplicate"]);
                overlay_widgets::hsep(ui);
                match st_ops.tab {
                    0 => {
                        egui::Grid::new(ui.make_persistent_id(("perlin_grid", key))).num_columns(4).spacing(egui::vec2(6.0, 4.0)).show(ui, |ui| {
                            ui.label("Min"); ui.add(egui::DragValue::new(&mut st_ops.perlin_min.x)); ui.add(egui::DragValue::new(&mut st_ops.perlin_min.y)); ui.add(egui::DragValue::new(&mut st_ops.perlin_min.z)); ui.end_row();
                            ui.label("Max"); ui.add(egui::DragValue::new(&mut st_ops.perlin_max.x)); ui.add(egui::DragValue::new(&mut st_ops.perlin_max.y)); ui.add(egui::DragValue::new(&mut st_ops.perlin_max.z)); ui.end_row();
                            ui.label("Scale"); ui.add(egui::DragValue::new(&mut st_ops.perlin_scale).speed(0.01)); ui.label("Thr"); ui.add(egui::DragValue::new(&mut st_ops.perlin_threshold).speed(0.01)); ui.end_row();
                            ui.label("Seed"); ui.add(egui::DragValue::new(&mut st_ops.perlin_seed)); ui.label("Pal"); ui.add(egui::DragValue::new(&mut st.palette_index).range(1..=255)); ui.end_row();
                        });
                        if overlay_widgets::action_button(ui, "Apply").on_hover_text("Apply Perlin").clicked() {
                            let mn = st_ops.perlin_min.min(st_ops.perlin_max); let mx = st_ops.perlin_min.max(st_ops.perlin_max);
                            push_voxel_cmd(ngr, ui_state, key, gc, DiscreteVoxelOp::PerlinFill { min: mn, max: mx, scale: st_ops.perlin_scale, threshold: st_ops.perlin_threshold, palette_index: st.palette_index, seed: st_ops.perlin_seed });
                        }
                    }
                    _ => {
                        ui.horizontal(|ui| { ui.label("Delta"); ui.add(egui::DragValue::new(&mut st_ops.dup_delta.x)); ui.add(egui::DragValue::new(&mut st_ops.dup_delta.y)); ui.add(egui::DragValue::new(&mut st_ops.dup_delta.z)); });
                        overlay_widgets::toggle_button(ui, &mut st.clone_overwrite, "Overwrite");
                        if overlay_widgets::action_button(ui, "Duplicate").clicked() && !sel.cells.is_empty() {
                            push_voxel_cmd(ngr, ui_state, key, gc, DiscreteVoxelOp::CloneSelected { cells: sel.cells.iter().copied().collect(), delta: st_ops.dup_delta, overwrite: st.clone_overwrite });
                        }
                    }
                }
            });

            overlay_widgets::group(ui, "Overlays", false, |ui| {
                overlay_widgets::toolbar(ui, false, |ui| {
                    overlay_widgets::toggle_button(ui, &mut ov.show_volume_grid, "Volume");
                    overlay_widgets::toggle_button(ui, &mut ov.show_voxel_grid, "Voxel");
                    overlay_widgets::toggle_button(ui, &mut ov.show_volume_bound, "Bounds");
                });
                overlay_widgets::styled_slider(ui, &mut display_options.overlays.voxel_grid_line_px, 0.0..=4.0, "Grid px");
                overlay_widgets::toolbar(ui, false, |ui| {
                    overlay_widgets::toggle_button(ui, &mut ov.show_coordinates, "Coord");
                    overlay_widgets::toggle_button(ui, &mut ov.show_distance, "Dist");
                });
            });

            overlay_widgets::group(ui, "Raycast", false, |ui| {
                overlay_widgets::info_row(ui, "Hit", if hud.has_hit { "Yes" } else { "No" });
                overlay_widgets::info_row(ui, "Cell", &format!("{:?}", hud.cell));
                overlay_widgets::info_row(ui, "Normal", &format!("{:?}", hud.normal));
                if ov.show_distance { overlay_widgets::info_row(ui, "Distance", &format!("{:.3}", hud.distance)); }
                if ov.show_volume_bound && hud.has_bounds {
                    overlay_widgets::info_row(ui, "Min", &format!("{:?}", hud.bounds_min));
                    overlay_widgets::info_row(ui, "Max", &format!("{:?}", hud.bounds_max));
                }
            });
        });
    });
}

#[allow(deprecated)]
fn draw_import_export_icons(
    ui: &mut egui::Ui, ngr: &mut NodeGraphResource, ui_state: &UiState, key: CoverlayPanelKey,
    import_states: &mut ImportPanelStateMap, export_states: &mut ExportPanelStateMap, gc: &mut MessageWriter<crate::GraphChanged>,
) {
    let import_id = ui.make_persistent_id(("import_popup", key));
    let export_id = ui.make_persistent_id(("export_popup", key));
    let import_btn = overlay_widgets::icon_button(ui, "📥", false).on_hover_text("Import");
    if import_btn.clicked() { ui.memory_mut(|m| m.toggle_popup(import_id)); }
    let export_btn = overlay_widgets::icon_button(ui, "📤", false).on_hover_text("Export");
    if export_btn.clicked() { ui.memory_mut(|m| m.toggle_popup(export_id)); }
    egui::popup_below_widget(ui, import_id, &import_btn, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(260.0);
        draw_import_content(ui, ngr, ui_state, key, import_states, gc);
    });
    egui::popup_below_widget(ui, export_id, &export_btn, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(260.0);
        draw_export_content(ui, ngr, ui_state, key, export_states);
    });
}

fn draw_import_content(ui: &mut egui::Ui, ngr: &mut NodeGraphResource, ui_state: &UiState, key: CoverlayPanelKey, st_map: &mut ImportPanelStateMap, gc: &mut MessageWriter<crate::GraphChanged>) {
    let st = st_map.map.entry(key).or_default();
    if st.mesh_max_voxels <= 0 { st.mesh_max_voxels = 200_000; }
    let mut do_vox = false; let mut do_img = false; let mut do_hm = false; let mut do_mesh = false;
    overlay_widgets::label_secondary(ui, "Path");
    ui.text_edit_singleline(&mut st.path);
    ui.horizontal(|ui| { overlay_widgets::label_secondary(ui, "Scale"); ui.add(egui::DragValue::new(&mut st.scale).speed(0.1).range(0.01..=1000.0)); overlay_widgets::label_secondary(ui, "Height"); ui.add(egui::DragValue::new(&mut st.height).speed(1).range(1..=4096)); });
    ui.horizontal(|ui| { overlay_widgets::label_secondary(ui, "MeshMaxVox"); ui.add(egui::DragValue::new(&mut st.mesh_max_voxels).speed(100).range(1000..=20_000_000)); });
    overlay_widgets::toolbar(ui, false, |ui| { do_vox = overlay_widgets::action_button(ui, ".vox").clicked(); do_img = overlay_widgets::action_button(ui, "Img").clicked(); do_hm = overlay_widgets::action_button(ui, "Heightmap").clicked(); do_mesh = overlay_widgets::action_button(ui, "Mesh").clicked(); });
    if do_vox || do_img || do_hm || do_mesh { execute_import(ngr, ui_state, key, gc, st, do_vox, do_img, do_hm, do_mesh); }
}

fn draw_export_content(ui: &mut egui::Ui, ngr: &mut NodeGraphResource, ui_state: &UiState, key: CoverlayPanelKey, st_map: &mut ExportPanelStateMap) {
    let st = st_map.map.entry(key).or_default();
    let mut do_vox = false; let mut do_sm_opt = false; let mut do_sm = false; let mut do_patches = false;
    overlay_widgets::label_secondary(ui, "Path");
    ui.text_edit_singleline(&mut st.path);
    overlay_widgets::toolbar(ui, false, |ui| { do_vox = overlay_widgets::action_button(ui, ".vox").clicked(); do_patches = overlay_widgets::action_button(ui, "Patches").clicked(); });
    overlay_widgets::toolbar(ui, false, |ui| { do_sm_opt = overlay_widgets::action_button(ui, "Mesh Opt").clicked(); do_sm = overlay_widgets::action_button(ui, "Mesh").clicked(); });
    let do_add_asset = overlay_widgets::action_button(ui, "Add VoxyAsset").clicked();
    if do_vox || do_sm_opt || do_sm || do_patches || do_add_asset { execute_export(ngr, ui_state, key, st, do_vox, do_sm_opt, do_sm, do_patches, do_add_asset); }
}

fn execute_import(ngr: &mut NodeGraphResource, ui_state: &UiState, key: CoverlayPanelKey, gc: &mut MessageWriter<crate::GraphChanged>, st: &mut ImportPanelState, do_vox: bool, do_img: bool, do_hm: bool, do_mesh: bool) {
    let g = &mut ngr.0;
    let target = panel_voxel_target(ui_state, &g, key).or_else(|| resolve_voxel_edit_target(ui_state, &g));
    let Some(target) = target else { return; };
    let mut voxels: Vec<(IVec3, u8)> = Vec::new();
    let mut palette: Vec<cunning_kernel::algorithms::algorithms_editor::voxel::discrete::PaletteEntry> = vec![Default::default(); 256];
    palette[0] = cunning_kernel::algorithms::algorithms_editor::voxel::discrete::PaletteEntry { color: [0, 0, 0, 0], ..default() };
    let mut cmd_scale = st.scale.max(0.01);
    if do_vox { if let Ok(bytes) = std::fs::read(&st.path) { if let Some((vxs, pal)) = parse_magica_vox(&bytes) { voxels = vxs; palette = pal; } } }
    if do_img || do_hm {
        if let Ok(img) = image::open(&st.path) {
            if do_img {
                let rgba = img.to_rgba8(); let (w, h) = rgba.dimensions();
                let mut map: HashMap<[u8; 4], u8> = HashMap::new(); let mut next: u8 = 1;
                for y in 0..h { for x in 0..w {
                    let p = rgba.get_pixel(x, y).0; if p[3] == 0 { continue; }
                    let idx = *map.entry(p).or_insert_with(|| { let v = next; next = next.saturating_add(1); v });
                    if (idx as usize) < palette.len() { palette[idx as usize] = cunning_kernel::algorithms::algorithms_editor::voxel::discrete::PaletteEntry { color: p, ..default() }; }
                    voxels.push((IVec3::new(x as i32, 0, y as i32), idx));
                }}
            } else {
                let luma = img.to_luma8(); let (w, h) = luma.dimensions(); let maxh = st.height.max(1) as i32;
                for y in 0..h { for x in 0..w {
                    let v = luma.get_pixel(x, y).0[0] as f32 / 255.0; let hh = (v * (maxh as f32)).round() as i32;
                    for yy in 0..hh { voxels.push((IVec3::new(x as i32, yy, y as i32), 1)); }
                }}
            }
        }
    }
    if do_mesh { let vs = voxel_size_for_target(&g, target).max(0.001); cmd_scale = 1.0; if let Some(vxs) = voxelize_obj(&st.path, st.scale.max(0.01), vs, st.mesh_max_voxels.max(1000) as usize) { voxels = vxs; } }
    if voxels.is_empty() { return; }
    let mut cmds = DiscreteVoxelCmdList::default();
    for (p, pi) in voxels.into_iter() { cmds.push(DiscreteVoxelOp::SetVoxel { x: (p.x as f32 * cmd_scale).round() as i32, y: (p.y as f32 * cmd_scale).round() as i32, z: (p.z as f32 * cmd_scale).round() as i32, palette_index: pi.max(1) }); }
    let pal_json = serde_json::to_string(&palette).unwrap_or_else(|_| "[]".to_string());
    if let Some(d) = write_voxel_cmds(g, target, cmds) { g.mark_dirty(d); }
    if let Some(d) = write_voxel_palette(g, target, pal_json) { g.mark_dirty(d); }
    gc.write_default();
}

fn execute_export(ngr: &mut NodeGraphResource, ui_state: &UiState, key: CoverlayPanelKey, st: &mut ExportPanelState, do_vox: bool, do_sm_opt: bool, do_sm: bool, do_patches: bool, do_add_asset: bool) {
    let g = &mut ngr.0;
    let target = panel_voxel_target(ui_state, &g, key).or_else(|| resolve_voxel_edit_target(ui_state, &g));
    let Some(target) = target else {
        if do_add_asset { let id = NodeId::new_v4(); let node = crate::nodes::structs::Node::new(id, "Voxel Edit".to_string(), NodeType::VoxelEdit, egui::pos2(0.0, 0.0)); g.nodes.insert(id, node); g.mark_dirty(id); }
        return;
    };
    let voxel_size = voxel_size_for_target(&g, target).max(0.001);
    let cmds = read_voxel_cmds(&g, target);
    let pal_json = read_voxel_palette(&g, target);
    // Match legacy export behavior from `draw_export_panel` (single source of truth).
    let mut grid = cunning_kernel::algorithms::algorithms_editor::voxel::DiscreteVoxelGrid::new(voxel_size);
    if let Ok(p) = serde_json::from_str::<Vec<cunning_kernel::algorithms::algorithms_editor::voxel::discrete::PaletteEntry>>(&pal_json) {
        for (i, e) in p.into_iter().enumerate() { if i < grid.palette.len() { grid.palette[i] = e; } }
    }
    let mut bake = cunning_kernel::algorithms::algorithms_editor::voxel::DiscreteBakeState { baked_cursor: 0 };
    cunning_kernel::algorithms::algorithms_editor::voxel::discrete::bake_cmds_incremental(&mut grid, &cmds, &mut bake);
    if do_vox { if let Some(bytes) = write_magica_vox(&grid) { let _ = std::fs::write(&st.path, bytes); } }
    if do_sm_opt || do_sm {
        let mut pm: HashMap<String, ParameterValue> = HashMap::new();
        pm.insert("voxel_size".to_string(), ParameterValue::Float(voxel_size));
        pm.insert("cmds_json".to_string(), ParameterValue::String(cmds.to_json()));
        pm.insert("palette_json".to_string(), ParameterValue::String(pal_json.clone()));
        let geo = crate::nodes::voxel::voxel_edit::compute_voxel_edit(None, &crate::mesh::Geometry::new(), &pm);
        let _ = write_obj_geometry(&st.path, &geo);
    }
    if do_patches {
        let patch = serde_json::json!({ "voxel_size": voxel_size, "cmds_json": cmds.to_json(), "palette_json": pal_json });
        let _ = std::fs::write(&st.path, serde_json::to_string_pretty(&patch).unwrap_or_else(|_| "{}".to_string()));
    }
}

fn push_voxel_cmd(ngr: &mut NodeGraphResource, ui_state: &UiState, key: CoverlayPanelKey, gc: &mut MessageWriter<crate::GraphChanged>, op: DiscreteVoxelOp) {
    let g = &mut ngr.0;
    let target = panel_voxel_target(ui_state, &g, key).or_else(|| resolve_voxel_edit_target(ui_state, &g));
    let Some(target) = target else { return; };
    let mut cmds = read_voxel_cmds(&g, target);
    cmds.push(op);
    if let Some(d) = write_voxel_cmds(g, target, cmds) { g.mark_dirty(d); }
    gc.write_default();
}

fn draw_voxel_palette_panel(ui: &mut egui::Ui, st: &mut VoxelToolState) {
    overlay_widgets::panel_frame(ui, |ui| {
        let col = palette_color32(st.palette_index);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            ui.horizontal(|ui| {
                overlay_widgets::color_preview(ui, col, 28.0);
                overlay_widgets::label_primary(ui, "Palette");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    overlay_widgets::badge(ui, format!("{}", st.palette_index).as_str(), egui::Color32::from_black_alpha(90));
                });
            });

            // Fill-to-bottom palette area (no scroll, no dead space), HSV bars pinned to bottom.
            let bottom_h = 92.0;
            let strip_h = (ui.available_height() - bottom_h).max(40.0);
            ui.allocate_ui(egui::vec2(ui.available_width(), strip_h), |ui| {
                overlay_widgets::palette_grid_fill(ui, &mut st.palette_index, palette_color32);
            });

            overlay_widgets::hsep(ui);
            overlay_widgets::hsv_palette_bar(ui, "palette_hsv", &mut st.palette_index, palette_color32);
        });
    });
}

#[allow(dead_code)]
fn draw_voxel_debug_panel(ui: &mut egui::Ui, st: &mut VoxelToolState, ov: &mut VoxelOverlaySettings, hud: &VoxelHudInfo, display_options: &mut crate::viewport_options::DisplayOptions) {
    let _ = st;
    overlay_widgets::panel_frame(ui, |ui| {
        overlay_widgets::group(ui, "Overlays", true, |ui| {
            overlay_widgets::toolbar(ui, false, |ui| {
                overlay_widgets::toggle_button(ui, &mut ov.show_volume_grid, "Volume");
                overlay_widgets::toggle_button(ui, &mut ov.show_voxel_grid, "Voxel");
                overlay_widgets::toggle_button(ui, &mut ov.show_volume_bound, "Bounds");
            });
            overlay_widgets::styled_slider(ui, &mut display_options.overlays.voxel_grid_line_px, 0.0..=4.0, "Grid px");
            overlay_widgets::toolbar(ui, false, |ui| {
                overlay_widgets::toggle_button(ui, &mut ov.show_coordinates, "Coord");
                overlay_widgets::toggle_button(ui, &mut ov.show_distance, "Dist");
            });
        });
        overlay_widgets::group(ui, "Raycast", true, |ui| {
            overlay_widgets::info_row(ui, "Hit", if hud.has_hit { "Yes" } else { "No" });
            overlay_widgets::info_row(ui, "Cell", &format!("{:?}", hud.cell));
            overlay_widgets::info_row(ui, "Normal", &format!("{:?}", hud.normal));
            if ov.show_distance { overlay_widgets::info_row(ui, "Distance", &format!("{:.3}", hud.distance)); }
            if ov.show_volume_bound && hud.has_bounds {
                overlay_widgets::info_row(ui, "Min", &format!("{:?}", hud.bounds_min));
                overlay_widgets::info_row(ui, "Max", &format!("{:?}", hud.bounds_max));
            }
        });
    });
}

fn panel_voxel_target(_ui: &UiState, g: &NodeGraph, key: CoverlayPanelKey) -> Option<VoxelEditTarget> {
    match key {
        CoverlayPanelKey::DirectVoxel { node_id, .. } => Some(VoxelEditTarget::Direct(node_id)),
        CoverlayPanelKey::DirectNode { .. } => None,
        CoverlayPanelKey::CdaManager { inst_id } => {
            let inst = g.nodes.get(&inst_id)?;
            let NodeType::CDA(data) = &inst.node_type else { return None; };
            let Some(lib) = global_cda_library() else { return None; };
            let _ = lib.ensure_loaded(&data.asset_ref);
            let a = lib.get(data.asset_ref.uuid)?;
            let internal_id = *data.coverlay_units.iter().find(|nid| a.inner_graph.nodes.get(nid).map_or(false, |n| matches!(n.node_type, NodeType::VoxelEdit)))?;
            Some(VoxelEditTarget::Cda { inst_id, internal_id })
        }
        CoverlayPanelKey::CdaUnit { inst_id, unit_id } => {
            let inst = g.nodes.get(&inst_id)?;
            let NodeType::CDA(data) = &inst.node_type else { return None; };
            let Some(lib) = global_cda_library() else { return None; };
            let _ = lib.ensure_loaded(&data.asset_ref);
            let a = lib.get(data.asset_ref.uuid)?;
            if a.inner_graph.nodes.get(&unit_id).map_or(false, |n| is_voxel_edit_node_type(Some(&n.node_type))) { return Some(VoxelEditTarget::Cda { inst_id, internal_id: unit_id }); }
            let internal_id = *data.coverlay_units.iter().find(|nid| a.inner_graph.nodes.get(nid).map_or(false, |n| is_voxel_edit_node_type(Some(&n.node_type))))?;
            Some(VoxelEditTarget::Cda { inst_id, internal_id })
        }
        CoverlayPanelKey::CdaVoxel { inst_id, internal_id, .. } => Some(VoxelEditTarget::Cda { inst_id, internal_id }),
    }
}

#[allow(dead_code)]
fn draw_import_panel(ui: &mut egui::Ui, ngr: &mut NodeGraphResource, ui_state: &UiState, key: CoverlayPanelKey, st_map: &mut ImportPanelStateMap, gc: &mut MessageWriter<crate::GraphChanged>) {
    let st = st_map.map.entry(key).or_default();
    if st.mesh_max_voxels <= 0 { st.mesh_max_voxels = 200_000; }
    let mut do_vox = false;
    let mut do_img = false;
    let mut do_hm = false;
    let mut do_mesh = false;
    overlay_widgets::panel_frame(ui, |ui| {
        overlay_widgets::group(ui, "Import", true, |ui| {
            overlay_widgets::label_secondary(ui, "Path");
            ui.text_edit_singleline(&mut st.path);
            ui.horizontal(|ui| {
                overlay_widgets::label_secondary(ui, "Scale");
                ui.add(egui::DragValue::new(&mut st.scale).speed(0.1).range(0.01..=1000.0));
                overlay_widgets::label_secondary(ui, "Height");
                ui.add(egui::DragValue::new(&mut st.height).speed(1).range(1..=4096));
            });
            ui.horizontal(|ui| {
                overlay_widgets::label_secondary(ui, "MeshMaxVox");
                ui.add(egui::DragValue::new(&mut st.mesh_max_voxels).speed(100).range(1000..=20_000_000));
            });
            overlay_widgets::toolbar(ui, false, |ui| {
                do_vox = overlay_widgets::action_button(ui, "Import .vox").clicked();
                do_img = overlay_widgets::action_button(ui, "Voxelize Image").clicked();
                do_hm = overlay_widgets::action_button(ui, "Import Heightmap").clicked();
                do_mesh = overlay_widgets::action_button(ui, "Voxelize Mesh (OBJ)").clicked();
            });
        });
    });
    if !(do_vox || do_img || do_hm || do_mesh) { return; }

    let mut g = &mut ngr.0;
    let target = panel_voxel_target(ui_state, &g, key).or_else(|| resolve_voxel_edit_target(ui_state, &g));
    let Some(target) = target else { return; };

    let mut voxels: Vec<(IVec3, u8)> = Vec::new();
    let mut palette: Vec<cunning_kernel::algorithms::algorithms_editor::voxel::discrete::PaletteEntry> = vec![Default::default(); 256];
    palette[0] = cunning_kernel::algorithms::algorithms_editor::voxel::discrete::PaletteEntry { color: [0, 0, 0, 0], ..default() };
    let mut cmd_scale = st.scale.max(0.01);

    if do_vox {
        if let Ok(bytes) = std::fs::read(&st.path) {
            if let Some((vxs, pal)) = parse_magica_vox(&bytes) {
                voxels = vxs;
                palette = pal;
            }
        }
    }
    if do_img || do_hm {
        if let Ok(img) = image::open(&st.path) {
            if do_img {
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                let mut map: HashMap<[u8; 4], u8> = HashMap::new();
                let mut next: u8 = 1;
                for y in 0..h {
                    for x in 0..w {
                        let p = rgba.get_pixel(x, y).0;
                        if p[3] == 0 { continue; }
                        let idx = *map.entry(p).or_insert_with(|| { let v = next; next = next.saturating_add(1); v });
                        if (idx as usize) < palette.len() { palette[idx as usize] = cunning_kernel::algorithms::algorithms_editor::voxel::discrete::PaletteEntry { color: p, ..default() }; }
                        voxels.push((IVec3::new(x as i32, 0, y as i32), idx));
                    }
                }
            } else {
                let luma = img.to_luma8();
                let (w, h) = luma.dimensions();
                let maxh = st.height.max(1) as i32;
                for y in 0..h {
                    for x in 0..w {
                        let v = luma.get_pixel(x, y).0[0] as f32 / 255.0;
                        let hh = (v * (maxh as f32)).round() as i32;
                        for yy in 0..hh {
                            voxels.push((IVec3::new(x as i32, yy, y as i32), 1));
                        }
                    }
                }
            }
        }
    }
    if do_mesh {
        let vs = voxel_size_for_target(&g, target).max(0.001);
        cmd_scale = 1.0;
        if let Some(vxs) = voxelize_obj(&st.path, st.scale.max(0.01), vs, st.mesh_max_voxels.max(1000) as usize) {
            voxels = vxs;
        }
    }

    if voxels.is_empty() { return; }
    let mut cmds = DiscreteVoxelCmdList::default();
    for (p, pi) in voxels.into_iter() {
        cmds.push(DiscreteVoxelOp::SetVoxel { x: (p.x as f32 * cmd_scale).round() as i32, y: (p.y as f32 * cmd_scale).round() as i32, z: (p.z as f32 * cmd_scale).round() as i32, palette_index: pi.max(1) });
    }
    let pal_json = serde_json::to_string(&palette).unwrap_or_else(|_| "[]".to_string());
    if let Some(d) = write_voxel_cmds(&mut g, target, cmds) { g.mark_dirty(d); }
    if let Some(d) = write_voxel_palette(&mut g, target, pal_json) { g.mark_dirty(d); }
    gc.write_default();
}

#[derive(Clone)]
#[allow(dead_code)]
struct TriShape { prim_index: usize, node_index: usize, aabb: Aabb<f32, 3>, p0: Vec3, p1: Vec3, p2: Vec3 }

impl TriShape {
    fn new(i: usize, p0: Vec3, p1: Vec3, p2: Vec3) -> Self {
        let mut min = Point3::new(f32::MAX, f32::MAX, f32::MAX);
        let mut max = Point3::new(f32::MIN, f32::MIN, f32::MIN);
        for p in [p0, p1, p2] {
            let q = Point3::new(p.x, p.y, p.z);
            min = min.inf(&q);
            max = max.sup(&q);
        }
        let eps = Vector3::new(1e-4, 1e-4, 1e-4);
        min -= eps;
        max += eps;
        Self { prim_index: i, node_index: i, aabb: Aabb::with_bounds(min, max), p0, p1, p2 }
    }
}

impl Bounded<f32, 3> for TriShape { fn aabb(&self) -> Aabb<f32, 3> { self.aabb } }
impl BHShape<f32, 3> for TriShape { fn set_bh_node_index(&mut self, index: usize) { self.node_index = index; } fn bh_node_index(&self) -> usize { self.node_index } }

fn voxelize_obj(path: &str, scale: f32, voxel_size: f32, max_voxels: usize) -> Option<Vec<(IVec3, u8)>> {
    let (models, _) = tobj::load_obj(path, &tobj::LoadOptions { single_index: true, triangulate: true, ..Default::default() }).ok()?;
    let mut tris: Vec<TriShape> = Vec::new();
    let mut mn = Vec3::splat(f32::MAX);
    let mut mx = Vec3::splat(f32::MIN);
    for m in models.iter() {
        let mesh = &m.mesh;
        let v = &mesh.positions;
        for p in v.chunks_exact(3) {
            let q = Vec3::new(p[0], p[1], p[2]) * scale;
            mn = mn.min(q);
            mx = mx.max(q);
        }
        for idx in mesh.indices.chunks_exact(3) {
            let a = idx[0] as usize; let b = idx[1] as usize; let c = idx[2] as usize;
            let p0 = Vec3::new(v[a * 3], v[a * 3 + 1], v[a * 3 + 2]) * scale;
            let p1 = Vec3::new(v[b * 3], v[b * 3 + 1], v[b * 3 + 2]) * scale;
            let p2 = Vec3::new(v[c * 3], v[c * 3 + 1], v[c * 3 + 2]) * scale;
            tris.push(TriShape::new(tris.len(), p0, p1, p2));
        }
    }
    if tris.is_empty() { return None; }
    let vs = voxel_size.max(0.0001);
    let size = ((mx - mn) / vs).ceil().as_ivec3().max(IVec3::ONE);
    let total = (size.x as i64) * (size.y as i64) * (size.z as i64);
    if total <= 0 || total as usize > max_voxels { return None; }
    let mut shapes = tris;
    let bvh = Bvh::build(&mut shapes);

    let mut out: Vec<(IVec3, u8)> = Vec::new();
    let pad = vs * 4.0;
    for z in 0..size.z {
        for y in 0..size.y {
            let cy = mn.y + (y as f32 + 0.5) * vs;
            let cz = mn.z + (z as f32 + 0.5) * vs;
            let qmin = Vec3::new(mn.x - pad, cy - 1e-4, cz - 1e-4);
            let qmax = Vec3::new(mx.x + pad, cy + 1e-4, cz + 1e-4);
            let cand = query_bvh_aabb(&bvh, &shapes, qmin, qmax);
            if cand.is_empty() { continue; }
            for x in 0..size.x {
                let cx = mn.x + (x as f32 + 0.5) * vs;
                let ro = Vec3::new(mn.x - pad, cy, cz);
                let rd = Vec3::X;
                let mut hits = 0u32;
                for &ti in cand.iter() {
                    let t = ray_tri(ro, rd, shapes[ti].p0, shapes[ti].p1, shapes[ti].p2);
                    if let Some(t) = t { if t > 0.0 && ro.x + rd.x * t <= cx { hits += 1; } }
                }
                if hits & 1 == 1 { out.push((IVec3::new(x, y, z), 1)); }
            }
        }
    }
    Some(out)
}

fn query_bvh_aabb(bvh: &Bvh<f32, 3>, shapes: &[TriShape], aabb_min: Vec3, aabb_max: Vec3) -> Vec<usize> {
    let min = Point3::new(aabb_min.x, aabb_min.y, aabb_min.z);
    let max = Point3::new(aabb_max.x, aabb_max.y, aabb_max.z);
    let query = Aabb::with_bounds(min, max);
    let mut out: Vec<usize> = Vec::new();
    let mut stack = vec![0usize];
    while let Some(i) = stack.pop() {
        match &bvh.nodes[i] {
            bvh::bvh::BvhNode::Node { child_l_index, child_l_aabb, child_r_index, child_r_aabb, .. } => {
                if child_l_aabb.intersects_aabb(&query) { stack.push(*child_l_index); }
                if child_r_aabb.intersects_aabb(&query) { stack.push(*child_r_index); }
            }
            bvh::bvh::BvhNode::Leaf { shape_index, .. } => {
                if let Some(s) = shapes.get(*shape_index) {
                    if s.aabb.intersects_aabb(&query) { out.push(*shape_index); }
                }
            }
        }
    }
    out
}

fn ray_tri(ro: Vec3, rd: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f32> {
    let e1 = v1 - v0;
    let e2 = v2 - v0;
    let p = rd.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-8 { return None; }
    let inv = 1.0 / det;
    let tvec = ro - v0;
    let u = tvec.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) { return None; }
    let q = tvec.cross(e1);
    let v = rd.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 { return None; }
    let t = e2.dot(q) * inv;
    Some(t)
}

fn parse_magica_vox(bytes: &[u8]) -> Option<(Vec<(IVec3, u8)>, Vec<cunning_kernel::algorithms::algorithms_editor::voxel::discrete::PaletteEntry>)> {
    use byteorder::{LittleEndian, ReadBytesExt};
    use std::io::Read;
    use std::io::Cursor;
    let mut cur = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cur.read_exact(&mut magic).ok()?;
    if &magic != b"VOX " { return None; }
    let _ver = cur.read_u32::<LittleEndian>().ok()?;
    let mut voxels: Vec<(IVec3, u8)> = Vec::new();
    let mut palette: Vec<cunning_kernel::algorithms::algorithms_editor::voxel::discrete::PaletteEntry> = vec![Default::default(); 256];
    palette[0] = cunning_kernel::algorithms::algorithms_editor::voxel::discrete::PaletteEntry { color: [0, 0, 0, 0], ..default() };
    while (cur.position() as usize) + 12 <= bytes.len() {
        let mut id = [0u8; 4];
        cur.read_exact(&mut id).ok()?;
        let content = cur.read_u32::<LittleEndian>().ok()? as usize;
        let children = cur.read_u32::<LittleEndian>().ok()? as usize;
        let start = cur.position() as usize;
        if &id == b"XYZI" {
            let n = cur.read_u32::<LittleEndian>().ok()? as usize;
            for _ in 0..n {
                let x = cur.read_u8().ok()? as i32;
                let y = cur.read_u8().ok()? as i32;
                let z = cur.read_u8().ok()? as i32;
                let i = cur.read_u8().ok()?;
                voxels.push((IVec3::new(x, z, y), i.max(1)));
            }
        } else if &id == b"RGBA" {
            for i in 1..=255usize {
                let r = cur.read_u8().ok()?;
                let g = cur.read_u8().ok()?;
                let b = cur.read_u8().ok()?;
                let a = cur.read_u8().ok()?;
                palette[i] = cunning_kernel::algorithms::algorithms_editor::voxel::discrete::PaletteEntry { color: [r, g, b, a], ..default() };
            }
        }
        let _ = children;
        cur.set_position((start + content) as u64);
    }
    Some((voxels, palette))
}

fn draw_anim_panel(ui: &mut egui::Ui, st: &mut crate::ui::TimelineState) {
    ui.horizontal(|ui| {
        if ui.button("⏮").clicked() { st.current_frame = st.start_frame; st.play_started_at = None; }
        if ui.button("◀").clicked() { st.current_frame = (st.current_frame - 1.0).max(st.start_frame); st.play_started_at = None; }
        let play = if st.is_playing { "⏸" } else { "▶" };
        if ui.button(play).clicked() { st.is_playing = !st.is_playing; st.play_started_at = None; }
        if ui.button("▶").clicked() { st.current_frame = (st.current_frame + 1.0).min(st.end_frame); st.play_started_at = None; }
        if ui.button("⏭").clicked() { st.current_frame = st.end_frame; st.play_started_at = None; }
    });
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Frame");
        ui.add(egui::DragValue::new(&mut st.current_frame).speed(1.0).range(st.start_frame..=st.end_frame));
        ui.label("FPS");
        ui.add(egui::DragValue::new(&mut st.fps).speed(0.5).range(1.0..=240.0));
    });
    ui.horizontal(|ui| {
        ui.label("Start");
        ui.add(egui::DragValue::new(&mut st.start_frame).speed(1.0));
        ui.label("End");
        ui.add(egui::DragValue::new(&mut st.end_frame).speed(1.0));
    });
}

#[allow(dead_code)]
fn draw_export_panel(ui: &mut egui::Ui, ngr: &mut NodeGraphResource, ui_state: &UiState, key: CoverlayPanelKey, st_map: &mut ExportPanelStateMap) {
    let st = st_map.map.entry(key).or_default();
    let mut do_vox = false;
    let mut do_sm_opt = false;
    let mut do_sm = false;
    let mut do_patches = false;
    let mut do_add_asset = false;
    overlay_widgets::panel_frame(ui, |ui| {
        overlay_widgets::group(ui, "Export", true, |ui| {
            overlay_widgets::label_secondary(ui, "Path");
            ui.text_edit_singleline(&mut st.path);
            overlay_widgets::toolbar(ui, false, |ui| {
                do_vox = overlay_widgets::action_button(ui, "Export .vox").clicked();
                do_patches = overlay_widgets::action_button(ui, "Patches (.json)").clicked();
            });
            overlay_widgets::toolbar(ui, false, |ui| {
                do_sm_opt = overlay_widgets::action_button(ui, "StaticMesh Opt (.obj)").clicked();
                do_sm = overlay_widgets::action_button(ui, "StaticMesh (.obj)").clicked();
            });
            do_add_asset = overlay_widgets::action_button(ui, "Add VoxyAsset").clicked();
        });
    });
    if !(do_vox || do_sm_opt || do_sm || do_patches || do_add_asset) { return; }
    let g = &mut ngr.0;
    let target = panel_voxel_target(ui_state, &g, key).or_else(|| resolve_voxel_edit_target(ui_state, &g));
    let Some(target) = target else {
        if do_add_asset {
            let id = NodeId::new_v4();
            let node = crate::nodes::structs::Node::new(id, "Voxel Edit".to_string(), NodeType::VoxelEdit, egui::pos2(0.0, 0.0));
            g.nodes.insert(id, node);
            g.mark_dirty(id);
        }
        return;
    };
    let voxel_size = voxel_size_for_target(&g, target).max(0.001);
    let cmds = read_voxel_cmds(&g, target);
    let pal_json = read_voxel_palette(&g, target);
    let mut grid = cunning_kernel::algorithms::algorithms_editor::voxel::DiscreteVoxelGrid::new(voxel_size);
    if let Ok(p) = serde_json::from_str::<Vec<cunning_kernel::algorithms::algorithms_editor::voxel::discrete::PaletteEntry>>(&pal_json) {
        for (i, e) in p.into_iter().enumerate() { if i < grid.palette.len() { grid.palette[i] = e; } }
    }
    let mut bake = cunning_kernel::algorithms::algorithms_editor::voxel::DiscreteBakeState { baked_cursor: 0 };
    cunning_kernel::algorithms::algorithms_editor::voxel::discrete::bake_cmds_incremental(&mut grid, &cmds, &mut bake);
    if do_vox { if let Some(bytes) = write_magica_vox(&grid) { let _ = std::fs::write(&st.path, bytes); } }
    if do_sm_opt || do_sm {
        let mut pm: HashMap<String, ParameterValue> = HashMap::new();
        pm.insert("voxel_size".to_string(), ParameterValue::Float(voxel_size));
        pm.insert("cmds_json".to_string(), ParameterValue::String(cmds.to_json()));
        pm.insert("palette_json".to_string(), ParameterValue::String(pal_json.clone()));
        let geo = crate::nodes::voxel::voxel_edit::compute_voxel_edit(None, &crate::mesh::Geometry::new(), &pm);
        let _ = write_obj_geometry(&st.path, &geo);
    }
    if do_patches {
        let patch = serde_json::json!({ "voxel_size": voxel_size, "cmds_json": cmds.to_json(), "palette_json": pal_json });
        let _ = std::fs::write(&st.path, serde_json::to_string_pretty(&patch).unwrap_or_else(|_| "{}".to_string()));
    }
}

fn write_magica_vox(g: &cunning_kernel::algorithms::algorithms_editor::voxel::DiscreteVoxelGrid) -> Option<Vec<u8>> {
    use byteorder::{LittleEndian, WriteBytesExt};
    let mut mn = IVec3::splat(i32::MAX);
    let mut mx = IVec3::splat(i32::MIN);
    for (c, _) in g.voxels.iter() { mn = mn.min(c.0); mx = mx.max(c.0); }
    if mn.x == i32::MAX { return None; }
    let sz = (mx - mn + IVec3::ONE).max(IVec3::ONE);
    let mut voxels: Vec<(u8, u8, u8, u8)> = Vec::with_capacity(g.voxels.len());
    for (c, v) in g.voxels.iter() {
        let p = c.0 - mn;
        let (x, y, z) = (p.x.clamp(0, 255) as u8, p.z.clamp(0, 255) as u8, p.y.clamp(0, 255) as u8);
        voxels.push((x, y, z, v.palette_index.max(1)));
    }
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"VOX ");
    out.write_u32::<LittleEndian>(150).ok()?;
    out.extend_from_slice(b"MAIN");
    out.write_u32::<LittleEndian>(0).ok()?;
    let main_children_pos = out.len();
    out.write_u32::<LittleEndian>(0).ok()?;
    let start_children = out.len();

    let mut size: Vec<u8> = Vec::new();
    size.write_i32::<LittleEndian>(sz.x).ok()?;
    size.write_i32::<LittleEndian>(sz.z).ok()?;
    size.write_i32::<LittleEndian>(sz.y).ok()?;
    write_chunk(&mut out, *b"SIZE", &size, &[])?;

    let mut xyzi: Vec<u8> = Vec::new();
    xyzi.write_u32::<LittleEndian>(voxels.len() as u32).ok()?;
    for (x, y, z, i) in voxels.into_iter() { xyzi.extend_from_slice(&[x, y, z, i]); }
    write_chunk(&mut out, *b"XYZI", &xyzi, &[])?;

    let mut rgba: Vec<u8> = Vec::new();
    for i in 1..=255usize {
        let c = g.palette.get(i).map(|e| e.color).unwrap_or([0, 0, 0, 255]);
        rgba.extend_from_slice(&c);
    }
    write_chunk(&mut out, *b"RGBA", &rgba, &[])?;

    let children_len = (out.len() - start_children) as u32;
    out[main_children_pos..main_children_pos + 4].copy_from_slice(&children_len.to_le_bytes());
    Some(out)
}

fn write_chunk(out: &mut Vec<u8>, id: [u8; 4], content: &[u8], children: &[u8]) -> Option<()> {
    use byteorder::{LittleEndian, WriteBytesExt};
    out.extend_from_slice(&id);
    out.write_u32::<LittleEndian>(content.len() as u32).ok()?;
    out.write_u32::<LittleEndian>(children.len() as u32).ok()?;
    out.extend_from_slice(content);
    out.extend_from_slice(children);
    Some(())
}

fn write_obj_geometry(path: &str, geo: &crate::mesh::Geometry) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    let Some(pos) = geo.get_point_attribute("@P").and_then(|a| a.as_slice::<Vec3>()) else { return Ok(()); };
    for p in pos.iter() { writeln!(f, "v {} {} {}", p.x, p.y, p.z)?; }
    for prim in geo.primitives().values().iter() {
        let mut idxs: Vec<usize> = Vec::new();
        for &vid in prim.vertices() {
            if let Some(v) = geo.vertices().get(vid.into()) {
                if let Some(di) = geo.points().get_dense_index(v.point_id.into()) { idxs.push(di + 1); }
            }
        }
        if idxs.len() >= 3 {
            write!(f, "f")?;
            for i in idxs.iter() { write!(f, " {}", i)?; }
            writeln!(f)?;
        }
    }
    Ok(())
}

fn do_undo_redo(ngr: &mut NodeGraphResource, gc: &mut MessageWriter<crate::GraphChanged>, cda_inst: Option<NodeId>, direct_node: Option<NodeId>, undo: bool) {
    const KEY: &str = "cmds_json";
    let g = &mut ngr.0;
    if let Some(inst_id) = cda_inst {
        let Some(inst) = g.nodes.get_mut(&inst_id) else { return; };
        let NodeType::CDA(data) = &mut inst.node_type else { return; };
        let Some(lib) = global_cda_library() else { return; };
        let Some(a) = lib.get(data.asset_ref.uuid) else { return; };
        let Some(internal_id) = data.coverlay_units.iter().find(|id| a.inner_graph.nodes.get(id).map_or(false, |n| matches!(n.node_type, NodeType::VoxelEdit))).copied() else { return; };
        let s = data.inner_param_overrides.get(&internal_id).and_then(|m| m.get(KEY)).and_then(|v| if let ParameterValue::String(s) = v { Some(s.as_str()) } else { None })
            .or_else(|| a.inner_graph.nodes.get(&internal_id).and_then(|n| n.parameters.iter().find(|p| p.name == KEY)).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.as_str()) } else { None }))
            .unwrap_or("{\"ops\":[],\"cursor\":0}");
        let mut c = DiscreteVoxelCmdList::from_json(s);
        if undo { let _ = c.undo(); } else { let _ = c.redo(); }
        data.inner_param_overrides.entry(internal_id).or_default().insert(KEY.to_string(), ParameterValue::String(c.to_json()));
        g.mark_dirty(inst_id);
        gc.write_default();
    } else if let Some(node_id) = direct_node {
        let Some(n) = g.nodes.get_mut(&node_id) else { return; };
        if !matches!(n.node_type, NodeType::VoxelEdit) { return; }
        let Some(p) = n.parameters.iter_mut().find(|p| p.name == KEY) else { return; };
        let ParameterValue::String(s) = &p.value else { return; };
        let mut c = DiscreteVoxelCmdList::from_json(s);
        if undo { let _ = c.undo(); } else { let _ = c.redo(); }
        p.value = ParameterValue::String(c.to_json());
        g.mark_dirty(node_id);
        gc.write_default();
    }
}

pub struct CoverlayDockTab {
    egui_state: CoverlayEguiState,
    import_states: ImportPanelStateMap,
    export_states: ExportPanelStateMap,
    ops_states: OpsPanelStateMap,
    dock_state: DockState<CoverlayDockPanel>,
    dock_sig: u64,
    dock_owner: Option<NodeId>,
}

impl Default for CoverlayDockTab {
    fn default() -> Self {
        Self {
            egui_state: Default::default(),
            import_states: Default::default(),
            export_states: Default::default(),
            ops_states: Default::default(),
            dock_state: DockState::new(Vec::new()),
            dock_sig: 0,
            dock_owner: None,
        }
    }
}

pub(crate) fn coverlay_collect_panels(ui: &UiState, g: &NodeGraph) -> Option<(NodeId, Vec<CoverlayDockPanel>)> {
    let target = pick_target(ui, g)?;
    let owner = match target { CoverlayTarget::DirectVoxel(id) => id, CoverlayTarget::DirectNode(id) => id, CoverlayTarget::Cda(id) => id };
    struct PanelDesc { key: CoverlayPanelKey, title: String, kind: CoverlayPanelKind, order: i32 }
    let mut panels: Vec<PanelDesc> = Vec::new();
    match target {
        CoverlayTarget::DirectVoxel(node_id) => {
            panels.push(PanelDesc { key: CoverlayPanelKey::DirectVoxel { node_id, kind: CoverlayPanelKind::VoxelTools }, title: "Voxel Tools".to_string(), kind: CoverlayPanelKind::VoxelTools, order: 0 });
            panels.push(PanelDesc { key: CoverlayPanelKey::DirectVoxel { node_id, kind: CoverlayPanelKind::VoxelPalette }, title: "Palette".to_string(), kind: CoverlayPanelKind::VoxelPalette, order: 1 });
        }
        CoverlayTarget::DirectNode(node_id) => {
            panels.push(PanelDesc { key: CoverlayPanelKey::DirectNode { node_id }, title: "Controls".to_string(), kind: CoverlayPanelKind::NodeCoverlay, order: 0 });
        }
        CoverlayTarget::Cda(inst_id) => {
            let inst = g.nodes.get(&inst_id)?;
            let NodeType::CDA(data) = &inst.node_type else { return None; };
            panels.push(PanelDesc { key: CoverlayPanelKey::CdaManager { inst_id }, title: "Coverlay".to_string(), kind: CoverlayPanelKind::Manager, order: -1000 });
            let lib = global_cda_library()?;
            let asset = lib.get(data.asset_ref.uuid)?;
            for u in asset.coverlay_units.iter() {
                if !data.coverlay_units.contains(&u.node_id) { continue; }
                let label = if let Some(ic) = u.icon.as_deref() { format!("{} {}", ic, u.label) } else { u.label.clone() };
                let node_ty = asset.inner_graph.nodes.get(&u.node_id).map(|n| &n.node_type);
                if is_voxel_edit_node_type(node_ty) {
                    panels.push(PanelDesc { key: CoverlayPanelKey::CdaVoxel { inst_id, internal_id: u.node_id, kind: CoverlayPanelKind::VoxelTools }, title: label.clone(), kind: CoverlayPanelKind::VoxelTools, order: u.order });
                    panels.push(PanelDesc { key: CoverlayPanelKey::CdaVoxel { inst_id, internal_id: u.node_id, kind: CoverlayPanelKind::VoxelPalette }, title: format!("{label} · Palette"), kind: CoverlayPanelKind::VoxelPalette, order: u.order.saturating_add(1) });
                    continue;
                }
                let kind = panel_kind_for_unit(&u.label, node_ty);
                panels.push(PanelDesc { key: CoverlayPanelKey::CdaUnit { inst_id, unit_id: u.node_id }, title: label, kind, order: u.order });
            }
        }
    }
    panels.sort_by_key(|p| p.order);
    Some((owner, panels.into_iter().map(|p| CoverlayDockPanel { key: p.key, title: p.title, kind: p.kind }).collect()))
}

impl CoverlayDockTab {
    pub(crate) fn draw_panel(&mut self, ui: &mut egui::Ui, cx: &mut EditorTabContext, p: &mut CoverlayDockPanel) {
        let mut viewer = CoverlayDockViewer { egui_state: &mut self.egui_state, import_states: &mut self.import_states, export_states: &mut self.export_states, ops_states: &mut self.ops_states, cx };
        viewer.ui(ui, p);
    }
}

const COVERLAY_LAYOUT_KEY: &str = "coverlay_layout_json";

fn coverlay_dock_sig(panels: &[CoverlayDockPanel]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    let mut h = DefaultHasher::new();
    panels.len().hash(&mut h);
    for p in panels {
        p.key.hash(&mut h);
        p.kind.hash(&mut h);
        p.title.hash(&mut h);
    }
    h.finish()
}

fn coverlay_strip_runtime(mut d: DockState<CoverlayDockPanel>) -> DockState<CoverlayDockPanel> {
    for (_si, n) in d.iter_all_nodes_mut() {
        match n {
            Node::Leaf { rect, viewport, scroll, .. } => { *rect = egui::Rect::NOTHING; *viewport = egui::Rect::NOTHING; *scroll = 0.0; }
            Node::Vertical { rect, .. } | Node::Horizontal { rect, .. } => { *rect = egui::Rect::NOTHING; }
            Node::Empty => {}
        }
    }
    d
}

fn coverlay_read_layout(g: &NodeGraph, owner: NodeId) -> Option<String> {
    let n = g.nodes.get(&owner)?;
    n.parameters.iter().find(|p| p.name == COVERLAY_LAYOUT_KEY).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None })
}

fn coverlay_write_layout(g: &mut NodeGraph, owner: NodeId, json: String) {
    let Some(n) = g.nodes.get_mut(&owner) else { return; };
    if let Some(p) = n.parameters.iter_mut().find(|p| p.name == COVERLAY_LAYOUT_KEY) {
        p.value = ParameterValue::String(json);
    } else {
        n.parameters.push(crate::nodes::parameter::Parameter::new(
            COVERLAY_LAYOUT_KEY,
            COVERLAY_LAYOUT_KEY,
            "Internal",
            ParameterValue::String(json),
            crate::nodes::parameter::ParameterUIType::Code,
        ));
    }
}

fn coverlay_apply_saved_layout(saved_json: Option<String>, panels: Vec<CoverlayDockPanel>) -> DockState<CoverlayDockPanel> {
    if panels.is_empty() { return DockState::new(Vec::new()); }
    let mut m: HashMap<CoverlayPanelKey, CoverlayDockPanel> = HashMap::with_capacity(panels.len());
    for p in panels { m.insert(p.key, p); }
    let mut st = saved_json.as_deref().and_then(|s| serde_json::from_str::<DockState<CoverlayDockPanel>>(s).ok())
        .map(|loaded| loaded.filter_map_tabs(|t| m.get(&t.key).cloned()))
        .filter(|st| st.iter_all_tabs().next().is_some())
        .unwrap_or_else(|| coverlay_default_dock(m.values().cloned().collect()));
    let mut have: std::collections::HashSet<CoverlayPanelKey> = std::collections::HashSet::new();
    for (_, t) in st.iter_all_tabs() { have.insert(t.key); }
    for p in m.values() { if !have.contains(&p.key) { st.push_to_focused_leaf(p.clone()); } }
    st
}

fn coverlay_default_dock(mut panels: Vec<CoverlayDockPanel>) -> DockState<CoverlayDockPanel> {
    if panels.is_empty() { return DockState::new(Vec::new()); }
    fn take(panels: &mut Vec<CoverlayDockPanel>, k: CoverlayPanelKind) -> Option<CoverlayDockPanel> {
        panels.iter().position(|p| p.kind == k).map(|i| panels.remove(i))
    }
    let main = if let Some(p) = take(&mut panels, CoverlayPanelKind::VoxelTools) {
        p
    } else if let Some(p) = take(&mut panels, CoverlayPanelKind::Manager) {
        p
    } else {
        panels.remove(0)
    };
    let palette = take(&mut panels, CoverlayPanelKind::VoxelPalette);
    let debug = take(&mut panels, CoverlayPanelKind::VoxelDebug);
    let mut dock = DockState::new(vec![main]);
    let mut parent = NodeIndex::root();
    if let Some(p) = palette { let [top, _] = dock.main_surface_mut().split_below(parent, 0.72, vec![p]); parent = top; }
    if let Some(d) = debug { let [left, _] = dock.main_surface_mut().split_right(parent, 0.72, vec![d]); parent = left; }
    if !panels.is_empty() {
        if let Node::Leaf { tabs, .. } = &mut dock[SurfaceIndex::main()][parent] { tabs.extend(panels); }
    }
    dock
}

struct CoverlayDockViewer<'a, 'b> {
    egui_state: &'a mut CoverlayEguiState,
    import_states: &'a mut ImportPanelStateMap,
    export_states: &'a mut ExportPanelStateMap,
    ops_states: &'a mut OpsPanelStateMap,
    cx: &'a mut EditorTabContext<'b>,
}

impl<'a, 'b> TabViewer for CoverlayDockViewer<'a, 'b> {
    type Tab = CoverlayDockPanel;

    fn ui(&mut self, ui: &mut egui::Ui, p: &mut Self::Tab) {
        match p.kind {
            CoverlayPanelKind::Manager => draw_cda_manager(ui, self.cx.node_graph_res, self.cx.ui_state, self.egui_state, p.key, self.cx.graph_changed_writer),
            CoverlayPanelKind::VoxelTools => {
                struct Backend<'a, 'b> {
                    ngr: &'a mut NodeGraphResource,
                    ui_state: &'a UiState,
                    key: CoverlayPanelKey,
                    gc: &'a mut MessageWriter<'b, crate::GraphChanged>,
                    sel_cells: Vec<IVec3>,
                }
                impl<'a, 'b> vxui::VoxelToolsBackend for Backend<'a, 'b> {
                    fn selection_cells(&self) -> &[IVec3] { &self.sel_cells }
                    fn undo(&mut self) {
                        let (cda_inst, direct_node) = match self.key {
                            CoverlayPanelKey::DirectVoxel { node_id, .. } => (None, Some(node_id)),
                            CoverlayPanelKey::CdaUnit { inst_id, .. } | CoverlayPanelKey::CdaVoxel { inst_id, .. } => (Some(inst_id), None),
                            _ => (None, None),
                        };
                        do_undo_redo(self.ngr, self.gc, cda_inst, direct_node, true);
                    }
                    fn redo(&mut self) {
                        let (cda_inst, direct_node) = match self.key {
                            CoverlayPanelKey::DirectVoxel { node_id, .. } => (None, Some(node_id)),
                            CoverlayPanelKey::CdaUnit { inst_id, .. } | CoverlayPanelKey::CdaVoxel { inst_id, .. } => (Some(inst_id), None),
                            _ => (None, None),
                        };
                        do_undo_redo(self.ngr, self.gc, cda_inst, direct_node, false);
                    }
                    fn push_op(&mut self, op: DiscreteVoxelOp) {
                        push_voxel_cmd(self.ngr, self.ui_state, self.key, self.gc, op);
                    }
                }
                let ops = self.ops_states.map.entry(p.key).or_default();
                let mut backend = Backend {
                    ngr: self.cx.node_graph_res,
                    ui_state: self.cx.ui_state,
                    key: p.key,
                    gc: self.cx.graph_changed_writer,
                    sel_cells: self.cx.voxel_selection.cells.iter().copied().collect(),
                };
                vxui::draw_voxel_tools_panel(ui, self.cx.voxel_tool_state, ops, p.key, &mut backend, self.cx.voxel_overlay_settings, self.cx.voxel_hud_info, self.cx.display_options);
            }
            CoverlayPanelKind::VoxelPalette => vxui::draw_voxel_palette_panel(ui, self.cx.voxel_tool_state),
            CoverlayPanelKind::VoxelDebug | CoverlayPanelKind::Import | CoverlayPanelKind::Export => {} // Merged into VoxelTools
            CoverlayPanelKind::Anim => draw_anim_panel(ui, self.cx.timeline_state),
            CoverlayPanelKind::NodeCoverlay => draw_node_coverlay_panel(ui, self.cx.node_graph_res, p.key, self.cx.graph_changed_writer),
            CoverlayPanelKind::Parameters => {} // Player-only
        }
    }

    fn title(&mut self, p: &mut Self::Tab) -> egui::WidgetText {
        p.title.clone().into()
    }

    fn allowed_in_windows(&self, _tab: &mut Self::Tab) -> bool { false }
}

impl EditorTab for CoverlayDockTab {
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut EditorTabContext) {
        let g = &cx.node_graph_res.0;
        let target = pick_target(cx.ui_state, &g);
        let Some(target) = target else {
            ui.label("Select a node with coverlay.");
            return;
        };
        let owner = match target { CoverlayTarget::DirectVoxel(id) => id, CoverlayTarget::DirectNode(id) => id, CoverlayTarget::Cda(id) => id };
        if self.dock_owner != Some(owner) { self.dock_owner = Some(owner); self.dock_sig = 0; }

        struct PanelDesc { key: CoverlayPanelKey, title: String, kind: CoverlayPanelKind, order: i32 }
        let mut panels: Vec<PanelDesc> = Vec::new();
        match target {
            CoverlayTarget::DirectVoxel(node_id) => {
                panels.push(PanelDesc { key: CoverlayPanelKey::DirectVoxel { node_id, kind: CoverlayPanelKind::VoxelTools }, title: "Voxel Tools".to_string(), kind: CoverlayPanelKind::VoxelTools, order: 0 });
                panels.push(PanelDesc { key: CoverlayPanelKey::DirectVoxel { node_id, kind: CoverlayPanelKind::VoxelPalette }, title: "Palette".to_string(), kind: CoverlayPanelKind::VoxelPalette, order: 1 });
            }
            CoverlayTarget::DirectNode(node_id) => {
                panels.push(PanelDesc { key: CoverlayPanelKey::DirectNode { node_id }, title: "Controls".to_string(), kind: CoverlayPanelKind::NodeCoverlay, order: 0 });
            }
            CoverlayTarget::Cda(inst_id) => {
                let Some(inst) = g.nodes.get(&inst_id) else { ui.label("Missing CDA instance."); return; };
                let NodeType::CDA(data) = &inst.node_type else { ui.label("Not a CDA instance."); return; };
                panels.push(PanelDesc { key: CoverlayPanelKey::CdaManager { inst_id }, title: "Coverlay".to_string(), kind: CoverlayPanelKind::Manager, order: -1000 });
                let Some(lib) = global_cda_library() else { ui.label("Missing CDA library."); return; };
                let Some(asset) = lib.get(data.asset_ref.uuid) else { ui.label("Missing CDA asset."); return; };
                for u in asset.coverlay_units.iter() {
                    if !data.coverlay_units.contains(&u.node_id) { continue; }
                    let label = if let Some(ic) = u.icon.as_deref() { format!("{} {}", ic, u.label) } else { u.label.clone() };
                    let node_ty = asset.inner_graph.nodes.get(&u.node_id).map(|n| &n.node_type);
                    if is_voxel_edit_node_type(node_ty) {
                        panels.push(PanelDesc { key: CoverlayPanelKey::CdaVoxel { inst_id, internal_id: u.node_id, kind: CoverlayPanelKind::VoxelTools }, title: label.clone(), kind: CoverlayPanelKind::VoxelTools, order: u.order });
                        panels.push(PanelDesc { key: CoverlayPanelKey::CdaVoxel { inst_id, internal_id: u.node_id, kind: CoverlayPanelKind::VoxelPalette }, title: format!("{label} · Palette"), kind: CoverlayPanelKind::VoxelPalette, order: u.order.saturating_add(1) });
                        continue;
                    }
                    let kind = panel_kind_for_unit(&u.label, node_ty);
                    panels.push(PanelDesc { key: CoverlayPanelKey::CdaUnit { inst_id, unit_id: u.node_id }, title: label, kind, order: u.order });
                }
            }
        }
        panels.sort_by_key(|p| p.order);

        let dock_panels: Vec<CoverlayDockPanel> = panels
            .into_iter()
            .map(|p| CoverlayDockPanel { key: p.key, title: p.title, kind: p.kind })
            .collect();
        let sig = coverlay_dock_sig(&dock_panels);
        if self.dock_sig != sig {
            self.dock_sig = sig;
            self.dock_state = coverlay_apply_saved_layout(coverlay_read_layout(&cx.node_graph_res.0, owner), dock_panels);
        }

        let mut viewer = CoverlayDockViewer {
            egui_state: &mut self.egui_state,
            import_states: &mut self.import_states,
            export_states: &mut self.export_states,
            ops_states: &mut self.ops_states,
            cx,
        };
        egui_dock::CoverlayDockArea::new(&mut self.dock_state)
            .id(egui::Id::new("coverlay_dock_panels"))
            .show_inside(ui, &mut viewer);

        // Persist layout on release (avoid cook/dirty; this is UI metadata).
        if !ui.ctx().input(|i| i.pointer.any_down()) {
            if let Ok(json) = serde_json::to_string(&coverlay_strip_runtime(self.dock_state.clone())) {
                let cur = coverlay_read_layout(&cx.node_graph_res.0, owner).unwrap_or_default();
                if cur != json {
                    coverlay_write_layout(&mut cx.node_graph_res.0, owner, json);
                    cx.ui_changed_writer.write_default();
                }
            }
        }
    }

    fn title(&self) -> egui::WidgetText { "Coverlay".into() }
    fn as_any(&self) -> &dyn std::any::Any { self }
}
