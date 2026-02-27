use wasm_bindgen::prelude::*;
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::PrimitiveTopology;
use bevy_egui::{egui, EguiContexts, EguiPlugin};
use cunning_viewport::{CunningViewportPlugin, viewport_options::{DisplayMode, DisplayOptions}};
use egui_dock::{DockArea, DockState, TabViewer};
use cunning_viewport::voxel_coverlay_ui as vxui;

mod protocol;
mod worker;
mod wireframe;
mod overlay_render;
mod turntable;
mod worker_runtime;
use cunning_grid_plane::CunningGridPlanePlugin;
use cunning_voxel_faces::CunningVoxelFacesPlugin;
use protocol::{HostCommand, WorkerEvent};
use worker::ComputeWorker;
use cunning_kernel::{geometry::attrs, mesh::{GeoPrimitive, Geometry}, traits::parameter::ParameterValue};
use cunning_kernel::algorithms::algorithms_editor::voxel as vox;
use cunning_cda_runtime::asset::NodeId;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use wireframe::{PlayerWireframePlugin, WireframeMarker, WireframeTopology};
use overlay_render::{CunningNormalPlugin, CunningPointPlugin, NormalColor, NormalMarker, PointMarker};
use bevy::render::sync_world::SyncToRenderWorld;

#[derive(Component)]
struct OutputGeo;
#[derive(Component)]
struct OutputSurface;
#[derive(Component)]
struct OutputWire;
#[derive(Component)]
struct OutputPoints;
#[derive(Component)]
struct OutputVertexNormals;
#[derive(Component)]
struct OutputPrimitiveNormals;

use cunning_viewport::coverlay_dock::{
    apply_palette_ratio_once, build_default_viewport_dock_from_preset, coverlay_dock_sig,
    coverlay_strip_runtime, preset_palette_ratio, CoverlayDockPanel, CoverlayPanelKey,
    clamp_viewport_dock_fractions, CoverlayPanelKind, ViewportDockTab, VIEWPORT_KEEP_X, VIEWPORT_KEEP_Y,
};

#[derive(Clone, Debug)]
struct PlayerAnimState {
    enabled: bool,
    param_name: Option<String>,
    current_frame: f32,
    start_frame: f32,
    end_frame: f32,
    is_playing: bool,
    fps: f32,
    play_accum: f32,
    last_sent_frame: i32,
    last_send_s: f64,
}

impl Default for PlayerAnimState {
    fn default() -> Self {
        Self {
            enabled: false,
            param_name: None,
            current_frame: 1.0,
            start_frame: 1.0,
            end_frame: 240.0,
            is_playing: false,
            fps: 24.0,
            play_accum: 0.0,
            last_sent_frame: i32::MIN,
            last_send_s: 0.0,
        }
    }
}

#[derive(Resource)]
struct UiState {
    url: String,
    error: Option<String>,
    def: Option<Arc<cunning_cda_runtime::asset::RuntimeDefinition>>,
    output_ready: bool,
    overrides: HashMap<String, ParameterValue>,
    auto_cook: bool,
    pending_clear: bool,
    dock: DockState<ViewportDockTab>,
    dock_sig: u64,
    dock_owner: Option<uuid::Uuid>,
    viewport_rect: Option<egui::Rect>,
    // Runtime UI selection state (per loaded asset)
    active_hud_unit: Option<NodeId>,
    coverlay_enabled_units: HashSet<NodeId>,
    voxel: VoxelEditUiState,
    voxel_tools: vxui::VoxelToolState,
    voxel_ops: HashMap<CoverlayPanelKey, vxui::VoxelOpsPanelState>,
    voxel_overlay: vxui::VoxelOverlaySettings,
    voxel_hud: vxui::VoxelHudInfo,
    voxel_selection: Vec<IVec3>,
    voxel_subtract: bool,
    anim: PlayerAnimState,
    out_mesh: Handle<Mesh>,
    prim_normals_mesh: Handle<Mesh>,
    wf_topology: Handle<WireframeTopology>,
    perf: PerfStats,
    voxel_render_node_id: Option<uuid::Uuid>,
    voxel_render_last_cursor: usize,
}

#[derive(Default, Clone, Copy)]
struct PerfStats { show: bool, fps: f32, ms: f32, last_cook_ms: u32 }

#[derive(Clone, Debug)]
struct VoxelEditUiState {
    target: Option<NodeId>,
    voxel_size: f32,
    cmds: vox::DiscreteVoxelCmdList,
    bake_state: vox::discrete::DiscreteBakeState,
    grid: vox::DiscreteSdfGrid,
    drawing: bool,
    last_cell: Option<IVec3>,
    line_start: Option<IVec3>,
    region_start: Option<IVec3>,
    move_anchor: Option<IVec3>,
    last_send_s: f64,
}

impl Default for VoxelEditUiState {
    fn default() -> Self {
        Self {
            target: None,
            voxel_size: 0.1,
            cmds: vox::DiscreteVoxelCmdList::default(),
            bake_state: vox::discrete::DiscreteBakeState::default(),
            grid: vox::DiscreteSdfGrid::new(0.1),
            drawing: false,
            last_cell: None,
            line_start: None,
            region_start: None,
            move_anchor: None,
            last_send_s: 0.0,
        }
    }
}

impl VoxelEditUiState {
    fn reset_bake(&mut self) {
        self.bake_state = vox::discrete::DiscreteBakeState::default();
        self.grid = vox::DiscreteSdfGrid::new(self.voxel_size.max(0.001));
        vox::discrete::bake_cmds_incremental(&mut self.grid, &self.cmds, &mut self.bake_state);
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    #[cfg(target_arch = "wasm32")]
    {
        // If this wasm module is instantiated inside a WebWorker, do NOT start Bevy.
        // Instead, install the compute worker message loop and return.
        use wasm_bindgen::JsCast;
        if js_sys::global()
            .dyn_into::<web_sys::DedicatedWorkerGlobalScope>()
            .is_ok()
        {
            console_error_panic_hook::set_once();
            worker_runtime::install_worker_runtime();
            return;
        }
    }
    #[cfg(target_arch = "wasm32")]
    std::panic::set_hook(Box::new(|info| {
        console_error_panic_hook::hook(info);
        if let Some(w) = web_sys::window() {
            if let Some(d) = w.document() {
                let p = d.get_element_by_id("err").unwrap_or_else(|| {
                    let e = d.create_element("pre").unwrap();
                    e.set_id("err");
                    e.set_attribute("style", "position:fixed;left:0;top:0;right:0;max-height:50%;overflow:auto;background:rgba(0,0,0,.85);color:#f66;padding:12px;z-index:99999;white-space:pre-wrap;").ok();
                    d.body().unwrap().append_child(&e).ok();
                    e
                });
                p.set_text_content(Some(&format!("{info}")));
            }
        }
    }));
    #[cfg(not(target_arch = "wasm32"))]
    console_error_panic_hook::set_once();
    let render = bevy::render::RenderPlugin {
        render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(bevy::render::settings::WgpuSettings {
            priority: bevy::render::settings::WgpuSettingsPriority::Compatibility,
            features: bevy::render::settings::WgpuFeatures::empty(),
            ..Default::default()
        })),
        ..Default::default()
    };
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "Cunning Player".into(), fit_canvas_to_parent: true, prevent_default_event_handling: true, canvas: Some("#bevy".into()), ..default() }),
            ..default()
        }).set(render))
        .add_plugins(CunningPointPlugin)
        .add_plugins(CunningNormalPlugin)
        .add_plugins(CunningViewportPlugin)
        .add_plugins(CunningGridPlanePlugin)
        .add_plugins(CunningVoxelFacesPlugin)
        .add_plugins(PlayerWireframePlugin)
        .add_plugins(EguiPlugin)
        .insert_resource(cunning_viewport::ViewportUiMode::Embedded)
        .insert_non_send_resource(ComputeWorker::spawn())
        .insert_resource(ClearColor(Color::srgb(0.1, 0.2, 0.3)))
        .add_systems(Startup, setup)
        .add_systems(Update, apply_pending_clear_system.before(poll_worker))
        // NOTE: voxel gizmos must run AFTER voxel_input_system updates anchors/HUD.
        .add_systems(
            Update,
            (
                poll_worker,
                voxel_input_system,
                sync_player_voxel_faces_root_system.after(voxel_input_system),
                draw_player_voxel_gizmos_system.after(voxel_input_system),
                sync_display_mode,
                sync_overlay_visibility,
                ui,
            ),
        )
        .add_systems(Update, sync_player_camera_viewport.after(ui))
        .add_systems(PostUpdate, turntable::turntable_camera_system)
        .run();
}

#[inline]
fn gizmo_wire_box(g: &mut Gizmos, mn: Vec3, mx: Vec3, color: Color) {
    let a = Vec3::new(mn.x, mn.y, mn.z);
    let b = Vec3::new(mx.x, mn.y, mn.z);
    let c = Vec3::new(mx.x, mn.y, mx.z);
    let d = Vec3::new(mn.x, mn.y, mx.z);
    let e = Vec3::new(mn.x, mx.y, mn.z);
    let f = Vec3::new(mx.x, mx.y, mn.z);
    let h = Vec3::new(mx.x, mx.y, mx.z);
    let i = Vec3::new(mn.x, mx.y, mx.z);
    g.line(a, b, color);
    g.line(b, c, color);
    g.line(c, d, color);
    g.line(d, a, color);
    g.line(e, f, color);
    g.line(f, h, color);
    g.line(h, i, color);
    g.line(i, e, color);
    g.line(a, e, color);
    g.line(b, f, color);
    g.line(c, h, color);
    g.line(d, i, color);
}

fn draw_player_voxel_gizmos_system(
    s: Res<UiState>,
    mut g: Gizmos,
) {
    let Some(def_arc) = s.def.clone() else { return; };
    let def = def_arc.as_ref();
    let Some(_target) = resolve_voxel_target(def, s.active_hud_unit, &s.coverlay_enabled_units) else { return; };

    // Always show a hover cursor box while voxel coverlay is enabled.
    if s.voxel_hud.has_hit {
        let vs = s.voxel_hud.voxel_size.max(0.001);
        let c = s.voxel_hud.cell.as_vec3();
        let mn = c * vs;
        let mx = (c + Vec3::ONE) * vs;
        gizmo_wire_box(&mut g, mn, mx, Color::srgba(1.0, 1.0, 1.0, 0.85));
    }

    // Region/Rect live preview while dragging (WASM).
    if s.voxel.drawing {
        let tool = &s.voxel_tools;
        let Some(a) = s.voxel.region_start else { return; };
        let vs = s.voxel_hud.voxel_size.max(0.001);
        let end = match tool.mode {
            vxui::VoxelToolMode::Add if matches!(tool.add_type, vxui::VoxelAddType::Region) => {
                if s.voxel_subtract { s.voxel_hud.cell } else { s.voxel_hud.cell + s.voxel_hud.normal }
            }
            vxui::VoxelToolMode::Paint if matches!(tool.paint_type, vxui::VoxelPaintType::Region) => s.voxel_hud.cell,
            vxui::VoxelToolMode::Select if matches!(tool.select_type, vxui::VoxelSelectType::Region | vxui::VoxelSelectType::Rect) => s.voxel_hud.cell,
            _ => return,
        };
        let mn = a.min(end).as_vec3() * vs;
        let mx = (a.max(end).as_vec3() + Vec3::ONE) * vs;
        gizmo_wire_box(&mut g, mn, mx, Color::srgba(1.0, 0.85, 0.2, 0.95));
    }
}

fn detect_anim_param(def: &cunning_cda_runtime::asset::RuntimeDefinition) -> Option<String> {
    // Convention-based: any promoted parameter named like time/frame enables timeline transport.
    let mut cand: Vec<(u8, String)> = Vec::new();
    for p in &def.promoted_params {
        let name = p.name.to_ascii_lowercase();
        let group = p.group.to_ascii_lowercase();
        let score = if name == "frame" || name == "current_frame" { 0 }
            else if name == "time" || name == "t" { 1 }
            else if name.contains("frame") { 2 }
            else if name.contains("time") { 3 }
            else if group.contains("timeline") || group.contains("anim") { 4 }
            else { continue };
        cand.push((score, p.name.clone()));
    }
    cand.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    cand.first().map(|(_, n)| n.clone())
}

fn setup(worker: NonSend<ComputeWorker>, mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>, mut wireframe_topologies: ResMut<Assets<WireframeTopology>>) {
    commands.insert_resource(GlobalAmbientLight { brightness: 300.0, ..default() });
    commands.insert_resource(turntable::TurntableState::default());
    // NOTE: WebGL pipeline validation will panic if we spawn a renderable entity with an empty mesh layout.
    // Use a valid placeholder mesh but keep it hidden until we have cooked output.
    let out_mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let mut prim = Mesh::new(PrimitiveTopology::PointList, RenderAssetUsages::default());
    prim.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![Vec3::ZERO]);
    prim.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![Vec3::Y]);
    let prim_normals_mesh = meshes.add(prim);
    let wf_topology = wireframe_topologies.add(WireframeTopology::new(Vec::new()));
    commands.spawn((Name::new("OutputSurface"), Mesh3d(out_mesh.clone()), MeshMaterial3d(materials.add(StandardMaterial { base_color: Color::srgb(0.9, 0.9, 0.95), perceptual_roughness: 0.7, metallic: 0.05, cull_mode: None, double_sided: true, ..default() })), Transform::IDENTITY, Visibility::Hidden, OutputGeo, OutputSurface));
    commands.spawn((Name::new("OutputWireframe"), Mesh3d(out_mesh.clone()), WireframeMarker { topology: wf_topology.clone() }, Transform::IDENTITY, Visibility::Hidden, OutputGeo, OutputWire));
    commands.spawn((Name::new("OutputPoints"), PointMarker, SyncToRenderWorld, Mesh3d(out_mesh.clone()), Transform::IDENTITY, Visibility::Hidden, OutputGeo, OutputPoints));
    commands.spawn((Name::new("OutputVertexNormals"), NormalMarker, NormalColor(Color::srgb(0.5, 0.6, 0.2)), bevy::camera::visibility::NoFrustumCulling, SyncToRenderWorld, Mesh3d(out_mesh.clone()), Transform::IDENTITY, Visibility::Hidden, OutputGeo, OutputVertexNormals));
    commands.spawn((Name::new("OutputPrimitiveNormals"), NormalMarker, NormalColor(Color::srgb(0.6, 0.1, 0.4)), bevy::camera::visibility::NoFrustumCulling, SyncToRenderWorld, Mesh3d(prim_normals_mesh.clone()), Transform::IDENTITY, Visibility::Hidden, OutputGeo, OutputPrimitiveNormals));
    #[cfg(target_arch = "wasm32")]
    let url = initial_cda_url("");
    #[cfg(not(target_arch = "wasm32"))]
    let url = initial_cda_url("/Procedural%20Project/Assets/New%20CDA09.cda");
    let s = UiState {
        url,
        error: None,
        def: None,
        output_ready: false,
        overrides: HashMap::new(),
        auto_cook: true,
        pending_clear: false,
        dock: DockState::new(vec![ViewportDockTab::Viewport]),
        dock_sig: 0,
        dock_owner: None,
        viewport_rect: None,
        active_hud_unit: None,
        coverlay_enabled_units: HashSet::new(),
        voxel: VoxelEditUiState::default(),
        voxel_tools: vxui::VoxelToolState::default(),
        voxel_ops: HashMap::new(),
        voxel_overlay: vxui::VoxelOverlaySettings::default(),
        voxel_hud: vxui::VoxelHudInfo::default(),
        voxel_selection: Vec::new(),
        voxel_subtract: false,
        anim: PlayerAnimState::default(),
        out_mesh,
        prim_normals_mesh,
        wf_topology,
        perf: PerfStats::default(),
        voxel_render_node_id: None,
        voxel_render_last_cursor: 0,
    };
    if !s.url.is_empty() {
        worker.send(HostCommand::LoadCdaUrl { url: s.url.clone() });
    }
    commands.insert_resource(s);
}

fn initial_cda_url(default_url: &str) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        // Allow embedding host to select the initial asset:
        // - /index.html?cda=/examples/foo.cda
        // - /index.html?cda=https://.../foo.cda
        if let Some(w) = web_sys::window() {
            if let Ok(search) = w.location().search() {
                let s = search.trim_start_matches('?');
                for pair in s.split('&').filter(|p| !p.is_empty()) {
                    let mut it = pair.splitn(2, '=');
                    let k = it.next().unwrap_or("").trim();
                    let v = it.next().unwrap_or("").trim();
                    if k == "cda" || k == "url" {
                        let decoded = js_sys::decode_uri_component(v)
                            .ok()
                            .and_then(|v| v.as_string())
                            .unwrap_or_else(|| v.to_string());
                        let decoded = decoded.trim().to_string();
                        if !decoded.is_empty() {
                            return normalize_cda_input_to_url(&decoded);
                        }
                    }
                }
            }
        }
    }
    default_url.to_string()
}

#[inline]
fn clear_player_outputs(
    s: &mut UiState,
    q_vis: &mut Query<&mut Visibility, With<OutputGeo>>,
    meshes: &mut Assets<Mesh>,
    wireframe_topologies: &mut Assets<WireframeTopology>,
    tt: &mut turntable::TurntableState,
) {
    for mut v in q_vis.iter_mut() { *v = Visibility::Hidden; }
    if let Some(dst) = meshes.get_mut(&s.out_mesh) { *dst = Mesh::from(Cuboid::new(1.0, 1.0, 1.0)); }
    if let Some(t) = wireframe_topologies.get_mut(&s.wf_topology) { t.indices.clear(); }
    *tt = turntable::TurntableState::default();
    s.error = None;
    s.def = None;
    s.output_ready = false;
    s.overrides.clear();
    s.viewport_rect = None;
    s.active_hud_unit = None;
    s.coverlay_enabled_units.clear();
    s.voxel = VoxelEditUiState::default();
    s.voxel_render_node_id = None;
    s.voxel_render_last_cursor = 0;
    s.voxel_selection.clear();
    s.voxel_hud = vxui::VoxelHudInfo::default();
    s.voxel_subtract = false;
    s.anim = PlayerAnimState::default();
    s.dock_owner = None;
    s.dock_sig = 0;
    // Clear voxel render cache to enforce single-CDA + no wasted GPU/CPU memory.
    cunning_kernel::nodes::voxel::voxel_edit::voxel_render_clear_all();
}

fn apply_pending_clear_system(
    mut s: ResMut<UiState>,
    mut q_vis: Query<&mut Visibility, With<OutputGeo>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut wireframe_topologies: ResMut<Assets<WireframeTopology>>,
    mut tt: ResMut<turntable::TurntableState>,
) {
    if !s.pending_clear { return; }
    s.pending_clear = false;
    clear_player_outputs(&mut *s, &mut q_vis, &mut *meshes, &mut *wireframe_topologies, &mut *tt);
}

fn sync_player_voxel_faces_root_system(
    mut s: ResMut<UiState>,
    mut root: ResMut<cunning_voxel_faces::VoxelPreviewRoot>,
) {
    let Some(def_arc) = s.def.clone() else { root.node_id = None; root.voxel_size = 0.0; return; };
    let def = def_arc.as_ref();
    let Some(target) = resolve_voxel_target(def, s.active_hud_unit, &s.coverlay_enabled_units) else { root.node_id = None; root.voxel_size = 0.0; return; };
    if s.voxel.target != Some(target) { init_voxel_state_from_def(&mut s.voxel, def, target); }

    // Keep the GPU voxel cache in sync incrementally; avoid per-frame work if cmds didn't change.
    if s.voxel.cmds.cursor != s.voxel_render_last_cursor || s.voxel_render_node_id.is_none() {
        let keyed = cunning_kernel::nodes::voxel::voxel_edit::voxel_render_sync_cmds_for_instance(def.meta.uuid, target, s.voxel.voxel_size, &s.voxel.cmds);
        s.voxel_render_node_id = Some(keyed);
        s.voxel_render_last_cursor = s.voxel.cmds.cursor;
    }

    root.node_id = s.voxel_render_node_id;
    root.voxel_size = s.voxel.voxel_size.max(0.001);
}

fn poll_worker(
    worker: NonSend<ComputeWorker>,
    mut s: ResMut<UiState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut q_vis: Query<&mut Visibility, With<OutputGeo>>,
    mut wireframe_topologies: ResMut<Assets<WireframeTopology>>,
    _display: Res<DisplayOptions>,
    mut tt: ResMut<turntable::TurntableState>,
) {
    while let Some(ev) = worker.try_recv() {
        match &ev {
            WorkerEvent::AssetReady { def } => {
                // Single-CDA invariant: switching assets clears everything.
                clear_player_outputs(&mut *s, &mut q_vis, &mut *meshes, &mut *wireframe_topologies, &mut *tt);
                s.def = Some(def.clone());
                s.overrides.clear();
                s.error = None;
                for pp in &def.promoted_params { s.overrides.insert(pp.name.clone(), pp.default_value.clone()); }

                // Initialize HUD/Coverlay selection from authoring defaults.
                s.active_hud_unit = def
                    .hud_units
                    .iter()
                    .find(|u| u.is_default)
                    .map(|u| u.node_id);
                s.coverlay_enabled_units = def
                    .coverlay_units
                    .iter()
                    .filter(|u| u.default_on)
                    .map(|u| u.node_id)
                    .collect();
                s.voxel = VoxelEditUiState::default();

                // Timeline transport: enable only when asset exposes a conventional time/frame parameter.
                s.anim = PlayerAnimState::default();
                s.anim.param_name = detect_anim_param(def.as_ref());
                s.anim.enabled = s.anim.param_name.is_some();
                if s.anim.enabled {
                    s.anim.last_sent_frame = i32::MIN;
                    // Push initial frame override once so nodes can render a stable first frame.
                    if let Some(n) = s.anim.param_name.clone() {
                        worker.send(HostCommand::SetOverride { name: n, value: ParameterValue::Float(s.anim.current_frame) });
                    }
                }

                if s.auto_cook { worker.send(HostCommand::Cook); }
            }
            WorkerEvent::Error { message } => {
                s.error = Some(message.clone());
            }
            WorkerEvent::CookFinished { duration_ms, outputs } => {
                s.perf.last_cook_ms = *duration_ms;
                let g = outputs.first().map(|g| g.as_ref());
                if let Some(g) = g {
                    if g.get_point_count() == 0 {
                        s.output_ready = false;
                        for mut v in &mut q_vis { *v = Visibility::Hidden; }
                        continue;
                    }
                    let mut m = g.to_bevy_mesh();
                    m.compute_smooth_normals();
                    if let Some(dst) = meshes.get_mut(&s.out_mesh) { *dst = m; }
                    if let Some(t) = wireframe_topologies.get_mut(&s.wf_topology) { t.indices = g.compute_wireframe_indices(); }
                    if let Some(dst) = meshes.get_mut(&s.prim_normals_mesh) { *dst = build_primitive_normals_mesh(g); }
                    let next_dirty = tt.dirty.wrapping_add(1);
                    turntable::update_bounds_from_geo(&mut tt, g, next_dirty);
                    s.output_ready = true;
                    for mut v in &mut q_vis { *v = Visibility::Visible; }
                } else {
                    s.output_ready = false;
                    for mut v in &mut q_vis { *v = Visibility::Hidden; }
                }
            }
        }
    }
}

fn sync_display_mode(
    s: Res<UiState>,
    display_options: Res<DisplayOptions>,
    mut q: Query<(&mut Visibility, Option<&OutputSurface>, Option<&OutputWire>), With<OutputGeo>>,
) {
    if !s.output_ready {
        for (mut v, _, _) in &mut q { *v = Visibility::Hidden; }
        return;
    }
    let show_surface = matches!(display_options.final_geometry_display_mode, DisplayMode::Shaded | DisplayMode::ShadedAndWireframe);
    let show_wire = matches!(display_options.final_geometry_display_mode, DisplayMode::Wireframe | DisplayMode::ShadedAndWireframe);
    for (mut v, is_surface, is_wire) in &mut q {
        if is_surface.is_some() { *v = if show_surface { Visibility::Visible } else { Visibility::Hidden }; }
        if is_wire.is_some() { *v = if show_wire { Visibility::Visible } else { Visibility::Hidden }; }
    }
}

fn sync_overlay_visibility(
    s: Res<UiState>,
    display_options: Res<DisplayOptions>,
    mut q: Query<(&mut Visibility, Option<&OutputPoints>, Option<&OutputVertexNormals>, Option<&OutputPrimitiveNormals>), With<OutputGeo>>,
) {
    if !s.output_ready {
        for (mut v, _, _, _) in &mut q { *v = Visibility::Hidden; }
        return;
    }
    let show_points = display_options.overlays.show_points;
    let show_vn = display_options.overlays.show_vertex_normals;
    let show_pn = display_options.overlays.show_primitive_normals;
    for (mut v, pts, vn, pn) in &mut q {
        if pts.is_some() { *v = if show_points { Visibility::Visible } else { Visibility::Hidden }; }
        if vn.is_some() { *v = if show_vn { Visibility::Visible } else { Visibility::Hidden }; }
        if pn.is_some() { *v = if show_pn { Visibility::Visible } else { Visibility::Hidden }; }
    }
}

fn sync_player_camera_viewport(
    s: Res<UiState>,
    primary_windows: Query<Entity, With<bevy::window::PrimaryWindow>>,
    mut viewport_layout: ResMut<cunning_viewport::layout::ViewportLayout>,
) {
    if viewport_layout.window_entity.is_none() {
        viewport_layout.window_entity = primary_windows.iter().next();
    }
    viewport_layout.logical_rect = s.viewport_rect;
}

fn is_voxel_edit_node(def: &cunning_cda_runtime::asset::RuntimeDefinition, node_id: NodeId) -> bool {
    def.nodes
        .iter()
        .find(|n| n.id == node_id)
        .is_some_and(|n| n.type_id == "cunning.voxel.edit")
}

fn resolve_voxel_target(
    def: &cunning_cda_runtime::asset::RuntimeDefinition,
    active_hud: Option<NodeId>,
    enabled: &HashSet<NodeId>,
) -> Option<NodeId> {
    // Keep HUD/coverlay targeting coherent:
    // 1) If active HUD unit is voxel, it is the active interaction target.
    // 2) Otherwise pick the first enabled VoxelEdit coverlay unit by authoring order.
    if let Some(hud_id) = active_hud {
        if is_voxel_edit_node(def, hud_id) {
            return Some(hud_id);
        }
    }

    let mut units = def.coverlay_units.clone();
    units.sort_by(|a, b| a.order.cmp(&b.order).then(a.label.cmp(&b.label)));
    for u in units {
        if enabled.contains(&u.node_id) && is_voxel_edit_node(def, u.node_id) {
            return Some(u.node_id);
        }
    }
    None
}

fn init_voxel_state_from_def(
    st: &mut VoxelEditUiState,
    def: &cunning_cda_runtime::asset::RuntimeDefinition,
    target: NodeId,
) {
    let Some(n) = def.nodes.iter().find(|n| n.id == target) else { return; };

    let voxel_size = n
        .params
        .get("voxel_size")
        .and_then(|v| if let ParameterValue::Float(x) = v { Some(*x) } else { None })
        .unwrap_or(0.1)
        .max(0.001);
    let cmds_json = n
        .params
        .get("cmds_json")
        .and_then(|v| if let ParameterValue::String(s) = v { Some(s.as_str()) } else { None })
        .unwrap_or("{\"ops\":[],\"cursor\":0}");

    st.target = Some(target);
    st.voxel_size = voxel_size;
    st.cmds = vox::DiscreteVoxelCmdList::from_json(cmds_json);
    st.reset_bake();
    st.drawing = false;
    st.last_cell = None;
    st.line_start = None;
    st.region_start = None;
    st.move_anchor = None;
}

#[inline]
fn sym_points(p: IVec3, sx: bool, sy: bool, sz: bool) -> impl Iterator<Item = IVec3> {
    let xs = if sx { [p.x, -p.x - 1] } else { [p.x, p.x] };
    let ys = if sy { [p.y, -p.y - 1] } else { [p.y, p.y] };
    let zs = if sz { [p.z, -p.z - 1] } else { [p.z, p.z] };
    let mut pts = [IVec3::ZERO; 8];
    let mut len = 0u8;
    for &x in xs.iter() {
        for &y in ys.iter() {
            for &z in zs.iter() {
                let q = IVec3::new(x, y, z);
                let mut dup = false;
                for i in 0..len as usize {
                    if pts[i] == q { dup = true; break; }
                }
                if !dup { pts[len as usize] = q; len += 1; }
            }
        }
    }
    let mut i = 0u8;
    std::iter::from_fn(move || {
        if i >= len { None } else { let v = pts[i as usize]; i += 1; Some(v) }
    })
}

#[inline]
fn bresenham_3d(mut a: IVec3, b: IVec3) -> Vec<IVec3> {
    let (mut x1, mut y1, mut z1) = (a.x, a.y, a.z);
    let (x2, y2, z2) = (b.x, b.y, b.z);
    let (dx, dy, dz) = ((x2 - x1).abs(), (y2 - y1).abs(), (z2 - z1).abs());
    let (xs, ys, zs) = ((if x2 > x1 { 1 } else { -1 }), (if y2 > y1 { 1 } else { -1 }), (if z2 > z1 { 1 } else { -1 }));
    let mut out = Vec::with_capacity((dx + dy + dz + 1).max(1) as usize);
    out.push(a);
    if dx >= dy && dx >= dz {
        let mut p1 = 2 * dy - dx;
        let mut p2 = 2 * dz - dx;
        while x1 != x2 {
            x1 += xs;
            if p1 >= 0 { y1 += ys; p1 -= 2 * dx; }
            if p2 >= 0 { z1 += zs; p2 -= 2 * dx; }
            p1 += 2 * dy;
            p2 += 2 * dz;
            a = IVec3::new(x1, y1, z1);
            out.push(a);
        }
    } else if dy >= dx && dy >= dz {
        let mut p1 = 2 * dx - dy;
        let mut p2 = 2 * dz - dy;
        while y1 != y2 {
            y1 += ys;
            if p1 >= 0 { x1 += xs; p1 -= 2 * dy; }
            if p2 >= 0 { z1 += zs; p2 -= 2 * dy; }
            p1 += 2 * dx;
            p2 += 2 * dz;
            a = IVec3::new(x1, y1, z1);
            out.push(a);
        }
    } else {
        let mut p1 = 2 * dy - dz;
        let mut p2 = 2 * dx - dz;
        while z1 != z2 {
            z1 += zs;
            if p1 >= 0 { y1 += ys; p1 -= 2 * dz; }
            if p2 >= 0 { x1 += xs; p2 -= 2 * dz; }
            p1 += 2 * dy;
            p2 += 2 * dx;
            a = IVec3::new(x1, y1, z1);
            out.push(a);
        }
    }
    out
}

fn voxel_input_system(
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut egui_ctx: EguiContexts,
    windows: Query<&Window>,
    cam_q: Query<(&Camera, &GlobalTransform), With<cunning_viewport::MainCamera>>,
    worker: NonSend<ComputeWorker>,
    mut s: ResMut<UiState>,
) {
    let Some(def_arc) = s.def.clone() else { return; };
    let def = def_arc.as_ref();
    let Some(target) = resolve_voxel_target(def, s.active_hud_unit, &s.coverlay_enabled_units) else { return; };

    if s.voxel.target != Some(target) {
        init_voxel_state_from_def(&mut s.voxel, def, target);
    }

    let Ok((camera, camera_tfm)) = cam_q.single() else { return; };
    let Ok(win) = windows.single() else { return; };
    let Some(cursor) = win.cursor_position() else { return; };
    let in_viewport = s
        .viewport_rect
        .map(|r| r.contains(egui::pos2(cursor.x, cursor.y)))
        .unwrap_or(true);

    // Avoid painting while the UI is actively using input, unless we are in the 3D viewport region.
    let wants_pointer = egui_ctx.ctx_mut().wants_pointer_input();
    let wants_keyboard = egui_ctx.ctx_mut().wants_keyboard_input();
    if (wants_pointer || wants_keyboard) && !in_viewport {
        s.voxel.drawing = false;
        s.voxel.last_cell = None;
        return;
    }
    if !in_viewport { return; }
    let Ok(ray) = camera.viewport_to_world(camera_tfm, cursor) else { return; };

    let voxel_size = s.voxel.voxel_size.max(0.001);

    // Hit: CPU DDA raycast against baked discrete grid; fallback to ground plane.
    let (hit_world, hit_cell, hit_nrm, hit_dist) = if let Some((t, c, n, h)) =
        raycast_discrete_dda(&s.voxel.grid, ray.origin, *ray.direction, voxel_size, 10000.0)
    {
        (h, c, n, t)
    } else {
        let Some(dist) = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y)) else {
            return;
        };
        let h = ray.get_point(dist);
        (h, (h / voxel_size).floor().as_ivec3(), IVec3::Y, dist)
    };

    // Update HUD (shared UI expects this).
    {
        let bounds = s.voxel.grid.bounds();
        s.voxel_hud = vxui::VoxelHudInfo {
            has_hit: true,
            cell: hit_cell,
            normal: hit_nrm,
            voxel_size,
            has_bounds: bounds.is_some(),
            bounds_min: bounds.map(|b| b.0).unwrap_or(IVec3::ZERO),
            bounds_max: bounds.map(|b| b.1).unwrap_or(IVec3::ZERO),
            distance: hit_dist,
        };
    }

    let just_down = mouse.just_pressed(MouseButton::Left);
    let down = mouse.pressed(MouseButton::Left);
    let just_up = mouse.just_released(MouseButton::Left);

    if just_down {
        s.voxel.drawing = true;
        s.voxel.last_cell = None;
        s.voxel.line_start = None;
        s.voxel.region_start = None;
        s.voxel.move_anchor = None;
    }

    if just_up {
        s.voxel.drawing = false;
    }

    let subtract = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    s.voxel_subtract = subtract;
    let tool = s.voxel_tools.clone();
    let (sym_x, sym_y, sym_z) = (tool.sym_x, tool.sym_y, tool.sym_z);
    let palette_index = tool.palette_index.max(1);
    let radius = tool.brush_radius.max(voxel_size * 0.25);
    let place_cell = if subtract { hit_cell } else { hit_cell + hit_nrm };
    let center = (place_cell.as_vec3() + Vec3::splat(0.5)) * voxel_size;

    // Helper: push ops + update local baked grid.
    let mut push_ops = |ops: &[vox::DiscreteVoxelOp], v: &mut VoxelEditUiState| {
        for op in ops { v.cmds.push(op.clone()); }
        vox::discrete::bake_cmds_incremental(&mut v.grid, &v.cmds, &mut v.bake_state);
    };

    // Selection helpers (player-only, but must exist for Clone/Move/Extrude UI to work).
    let mut sel_changed = false;
    let mut sel_set: std::collections::HashSet<IVec3> = s.voxel_selection.iter().copied().collect();
    let mut set_sel = |p: IVec3| {
        if subtract { sel_changed |= sel_set.remove(&p); } else { sel_changed |= sel_set.insert(p); }
    };

    // Region/Line anchors.
    if just_down {
        match tool.mode {
            vxui::VoxelToolMode::Add => match tool.add_type { vxui::VoxelAddType::Line => s.voxel.line_start = Some(place_cell), vxui::VoxelAddType::Region => s.voxel.region_start = Some(place_cell), _ => {} },
            vxui::VoxelToolMode::Paint => match tool.paint_type { vxui::VoxelPaintType::Line => s.voxel.line_start = Some(hit_cell), vxui::VoxelPaintType::Region => s.voxel.region_start = Some(hit_cell), _ => {} },
            vxui::VoxelToolMode::Select => match tool.select_type { vxui::VoxelSelectType::Line => s.voxel.line_start = Some(hit_cell), vxui::VoxelSelectType::Region | vxui::VoxelSelectType::Rect => s.voxel.region_start = Some(hit_cell), _ => {} },
            vxui::VoxelToolMode::Move | vxui::VoxelToolMode::Extrude => s.voxel.move_anchor = Some(hit_cell),
        }
    }

    // Continuous point stroke runs while down (Point-only; Line/Region/etc finalize on release).
    let allow_stroke = match tool.mode {
        vxui::VoxelToolMode::Add => matches!(tool.add_type, vxui::VoxelAddType::Point),
        vxui::VoxelToolMode::Paint => matches!(tool.paint_type, vxui::VoxelPaintType::Point),
        vxui::VoxelToolMode::Select => matches!(tool.select_type, vxui::VoxelSelectType::Point),
        _ => false,
    };
    if allow_stroke && down && s.voxel.drawing && !just_up {
        if s.voxel.last_cell == Some(hit_cell) { /* skip */ } else {
            s.voxel.last_cell = Some(hit_cell);
            match tool.mode {
                vxui::VoxelToolMode::Add => {
                    let op = match tool.shape {
                        vxui::VoxelBrushShape::Cube => {
                            let r = (radius / voxel_size).ceil().max(1.0) as i32;
                            let mn = place_cell - IVec3::splat(r);
                            let mx = place_cell + IVec3::splat(r);
                            if subtract { vox::DiscreteVoxelOp::BoxRemove { min: mn, max: mx } } else { vox::DiscreteVoxelOp::BoxAdd { min: mn, max: mx, palette_index } }
                        }
                        _ => {
                            if subtract { vox::DiscreteVoxelOp::SphereRemove { center, radius } } else { vox::DiscreteVoxelOp::SphereAdd { center, radius, palette_index } }
                        }
                    };
                    push_ops(&[op], &mut s.voxel);
                }
                vxui::VoxelToolMode::Paint => match tool.paint_type {
                    vxui::VoxelPaintType::Point => {
                        for p in sym_points(hit_cell, sym_x, sym_y, sym_z) {
                            let b0 = s.voxel.grid.get(p.x, p.y, p.z).map(|v| v.palette_index).unwrap_or(0);
                            if b0 != 0 && b0 != palette_index {
                                push_ops(&[vox::DiscreteVoxelOp::Paint { x: p.x, y: p.y, z: p.z, palette_index }], &mut s.voxel);
                            }
                        }
                    }
                    vxui::VoxelPaintType::ColorPick | vxui::VoxelPaintType::PromptStamp => {}
                    _ => {}
                },
                vxui::VoxelToolMode::Select => match tool.select_type {
                    vxui::VoxelSelectType::Point => for p in sym_points(hit_cell, sym_x, sym_y, sym_z) { set_sel(p); },
                    _ => {}
                },
                _ => {}
            }
        }
    }

    // Finalize actions on mouse release (Line/Region/Move/Extrude/Select Color/Face/Rect).
    if just_up {
        let mut ops: Vec<vox::DiscreteVoxelOp> = Vec::new();
        match tool.mode {
            vxui::VoxelToolMode::Add => match tool.add_type {
                vxui::VoxelAddType::Line => if let Some(a) = s.voxel.line_start.take() {
                    let pts = bresenham_3d(a, place_cell);
                    for c in pts {
                        let cc = (c.as_vec3() + Vec3::splat(0.5)) * voxel_size;
                        ops.push(if subtract { vox::DiscreteVoxelOp::SphereRemove { center: cc, radius } } else { vox::DiscreteVoxelOp::SphereAdd { center: cc, radius, palette_index } });
                    }
                },
                vxui::VoxelAddType::Region => if let Some(a) = s.voxel.region_start.take() {
                    let mn = a.min(place_cell);
                    let mx = a.max(place_cell);
                    ops.push(if subtract { vox::DiscreteVoxelOp::BoxRemove { min: mn, max: mx } } else { vox::DiscreteVoxelOp::BoxAdd { min: mn, max: mx, palette_index } });
                },
                _ => {}
            },
            vxui::VoxelToolMode::Paint => match tool.paint_type {
                vxui::VoxelPaintType::Line => if let Some(a) = s.voxel.line_start.take() {
                    for c in bresenham_3d(a, hit_cell) {
                        for p in sym_points(c, sym_x, sym_y, sym_z) {
                            let b0 = s.voxel.grid.get(p.x, p.y, p.z).map(|v| v.palette_index).unwrap_or(0);
                            if b0 != 0 && b0 != palette_index { ops.push(vox::DiscreteVoxelOp::Paint { x: p.x, y: p.y, z: p.z, palette_index }); }
                        }
                    }
                },
                vxui::VoxelPaintType::Region => if let Some(a) = s.voxel.region_start.take() {
                    let mn = a.min(hit_cell);
                    let mx = a.max(hit_cell);
                    for x in mn.x..=mx.x { for y in mn.y..=mx.y { for z in mn.z..=mx.z {
                        for p in sym_points(IVec3::new(x, y, z), sym_x, sym_y, sym_z) {
                            let b0 = s.voxel.grid.get(p.x, p.y, p.z).map(|v| v.palette_index).unwrap_or(0);
                            if b0 != 0 && b0 != palette_index { ops.push(vox::DiscreteVoxelOp::Paint { x: p.x, y: p.y, z: p.z, palette_index }); }
                        }
                    }}}
                },
                vxui::VoxelPaintType::Face => {
                    // Simple face paint: paint all voxels on the face plane that share the same axis coordinate as the hit.
                    let axis = if hit_nrm.x != 0 { 0 } else if hit_nrm.y != 0 { 1 } else { 2 };
                    for (c, vv) in s.voxel.grid.voxels.iter() {
                        let p = c.0;
                        if (axis == 0 && p.x != hit_cell.x) || (axis == 1 && p.y != hit_cell.y) || (axis == 2 && p.z != hit_cell.z) { continue; }
                        if vv.palette_index != 0 && vv.palette_index != palette_index {
                            for sp in sym_points(p, sym_x, sym_y, sym_z) { ops.push(vox::DiscreteVoxelOp::Paint { x: sp.x, y: sp.y, z: sp.z, palette_index }); }
                        }
                    }
                }
                vxui::VoxelPaintType::ColorPick | vxui::VoxelPaintType::PromptStamp => {}
                _ => {}
            },
            vxui::VoxelToolMode::Select => match tool.select_type {
                vxui::VoxelSelectType::Color => {
                    if let Some(vv) = s.voxel.grid.get(hit_cell.x, hit_cell.y, hit_cell.z) {
                        let pi = vv.palette_index;
                        if !subtract { sel_set.clear(); sel_changed = true; }
                        for (c, v2) in s.voxel.grid.voxels.iter() { if v2.palette_index == pi { sel_changed |= sel_set.insert(c.0); } }
                    }
                }
                vxui::VoxelSelectType::Line => if let Some(a) = s.voxel.line_start.take() { for p in bresenham_3d(a, hit_cell) { for sp in sym_points(p, sym_x, sym_y, sym_z) { set_sel(sp); } } },
                vxui::VoxelSelectType::Region | vxui::VoxelSelectType::Rect => if let Some(a) = s.voxel.region_start.take() {
                    let mn = a.min(hit_cell); let mx = a.max(hit_cell);
                    for x in mn.x..=mx.x { for y in mn.y..=mx.y { for z in mn.z..=mx.z { for sp in sym_points(IVec3::new(x, y, z), sym_x, sym_y, sym_z) { set_sel(sp); }}}}
                },
                vxui::VoxelSelectType::Face => {
                    let axis = if hit_nrm.x != 0 { 0 } else if hit_nrm.y != 0 { 1 } else { 2 };
                    for sp in sym_points(hit_cell, sym_x, sym_y, sym_z) { set_sel(sp); }
                    // Keep face selection simple on web: same-plane voxels only when Color/Region are used.
                    let _ = axis;
                }
                _ => {}
            },
            vxui::VoxelToolMode::Move => if let Some(a) = s.voxel.move_anchor.take() {
                if !s.voxel_selection.is_empty() {
                    let delta = hit_cell - a;
                    ops.push(vox::DiscreteVoxelOp::MoveSelected { cells: s.voxel_selection.clone(), delta });
                }
            },
            vxui::VoxelToolMode::Extrude => if let Some(a) = s.voxel.move_anchor.take() {
                if !s.voxel_selection.is_empty() {
                    let delta = hit_cell - a;
                    ops.push(vox::DiscreteVoxelOp::Extrude { cells: s.voxel_selection.clone(), delta, palette_index });
                }
            },
        }
        if !ops.is_empty() { push_ops(&ops, &mut s.voxel); }

        if sel_changed {
            let mut v: Vec<IVec3> = sel_set.into_iter().collect();
            v.sort_by(|a, b| (a.x, a.y, a.z).cmp(&(b.x, b.y, b.z)));
            s.voxel_selection = v;
            let mut flat: Vec<i32> = Vec::with_capacity(s.voxel_selection.len() * 3);
            for c in s.voxel_selection.iter() { flat.extend_from_slice(&[c.x, c.y, c.z]); }
            if let Ok(json) = serde_json::to_string(&flat) {
                worker.send(HostCommand::SetInternalOverride { node: target, param: "mask_json".to_string(), value: ParameterValue::String(json) });
            }
        }
    }

    if just_up {
        s.voxel.last_cell = None;
        worker.send(HostCommand::Cook);
        return;
    }

    // Write cmds_json into the VoxelEdit node and cook with throttling.
    worker.send(HostCommand::SetInternalOverride {
        node: target,
        param: "cmds_json".to_string(),
        value: ParameterValue::String(s.voxel.cmds.to_json()),
    });

    // No cook throttling: keep desktop-like high-frequency updates while editing.
    s.voxel.last_send_s = time.elapsed_secs_f64();
    worker.send(HostCommand::Cook);

    let _ = hit_world; // reserved for future HUD/preview
}

fn ui(
    mut ctx: EguiContexts,
    worker: NonSend<ComputeWorker>,
    voxel_stats: Res<cunning_voxel_faces::VoxelFacesStatsShared>,
    mut s: ResMut<UiState>,
    mut display_options: ResMut<cunning_viewport::viewport_options::DisplayOptions>,
) {
    let voxel_stats = voxel_stats.0.lock().ok().map(|g| g.clone()).unwrap_or_default();
    let dt = ctx.ctx_mut().input(|i| i.unstable_dt).max(1e-6);
    let fps = 1.0 / dt;
    let k = 1.0 - (-dt * 2.0).exp();
    s.perf.fps = if s.perf.fps <= 0.0 { fps } else { s.perf.fps + (fps - s.perf.fps) * k };
    s.perf.ms = if s.perf.ms <= 0.0 { dt * 1000.0 } else { s.perf.ms + (dt * 1000.0 - s.perf.ms) * k };

    // Timeline transport tick (WASM-first): advance frame and push override+cook when changed.
    if s.anim.enabled && s.anim.is_playing {
        let fps = s.anim.fps.max(1.0).min(240.0);
        s.anim.play_accum += dt;
        let step = (s.anim.play_accum * fps).floor() as i32;
        if step > 0 {
            s.anim.play_accum -= (step as f32) / fps;
            s.anim.current_frame = (s.anim.current_frame + (step as f32)).min(s.anim.end_frame);
            if s.anim.current_frame >= s.anim.end_frame {
                // stop at end (simple transport; looping can be added later)
                s.anim.is_playing = false;
                s.anim.play_accum = 0.0;
            }
        }
    }
    if s.anim.enabled {
        let cf = s.anim.current_frame.round().clamp(s.anim.start_frame, s.anim.end_frame);
        s.anim.current_frame = cf;
        let frame_i = cf as i32;
        if frame_i != s.anim.last_sent_frame {
            s.anim.last_sent_frame = frame_i;
            if let Some(n) = s.anim.param_name.clone() {
                worker.send(HostCommand::SetOverride { name: n, value: ParameterValue::Float(cf) });
                // throttle cooks slightly to avoid flooding slow graphs
                let now_s = ctx.ctx_mut().input(|i| i.time);
                if now_s - s.anim.last_send_s >= (1.0 / s.anim.fps.max(1.0).min(60.0) as f64).max(0.02) {
                    s.anim.last_send_s = now_s;
                    worker.send(HostCommand::Cook);
                }
            }
        }
    }

    fn local_storage() -> Option<web_sys::Storage> {
        #[cfg(target_arch = "wasm32")]
        { web_sys::window().and_then(|w| w.local_storage().ok().flatten()) }
        #[cfg(not(target_arch = "wasm32"))]
        { None }
    }
    fn ls_key(owner: uuid::Uuid, suffix: &str) -> String {
        format!("c3d_player.viewport_dock.{}.{}", owner, suffix)
    }
    fn ls_get(owner: uuid::Uuid, suffix: &str) -> Option<String> {
        local_storage()?.get_item(&ls_key(owner, suffix)).ok().flatten()
    }
    fn ls_set(owner: uuid::Uuid, suffix: &str, value: &str) {
        let _ = local_storage().and_then(|ls| ls.set_item(&ls_key(owner, suffix), value).ok());
    }

    fn side_icon_button(ui: &mut egui::Ui, id: &str, icon: egui::ImageSource<'static>, selected: bool) -> egui::Response {
        let button_size = egui::vec2(24.0, 24.0);
        let (rect, _) = ui.allocate_exact_size(button_size, egui::Sense::click());
        let response = ui.interact(rect, ui.make_persistent_id(id), egui::Sense::click());
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, ""));
        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact_selectable(&response, selected);
            if selected || response.hovered() {
                ui.painter().rect(
                    rect.expand(visuals.expansion),
                    visuals.corner_radius,
                    visuals.bg_fill,
                    visuals.bg_stroke,
                    egui::StrokeKind::Inside,
                );
            }
            egui::Image::new(icon)
                .tint(visuals.text_color())
                .paint_at(ui, egui::Rect::from_center_size(rect.center(), button_size * 0.8));
        }
        response
    }

    let owner = s.def.as_deref().map(|d| d.meta.uuid);
    if s.dock_owner != owner {
        s.dock_owner = owner;
        s.dock_sig = 0;
    }
    let owner_for_keys = owner.unwrap_or(uuid::Uuid::nil());
    let mut panels: Vec<CoverlayDockPanel> = vec![
        CoverlayDockPanel {
            key: CoverlayPanelKey::DirectVoxel { node_id: owner_for_keys, kind: CoverlayPanelKind::Parameters },
            title: "Parameters".to_string(),
            kind: CoverlayPanelKind::Parameters,
        },
        CoverlayDockPanel {
            key: CoverlayPanelKey::CdaManager { inst_id: owner_for_keys },
            title: "Coverlay".to_string(),
            kind: CoverlayPanelKind::Manager,
        },
    ];
    if let Some(def) = s.def.as_deref() {
        if let Some(target) = resolve_voxel_target(def, s.active_hud_unit, &s.coverlay_enabled_units) {
            panels.push(CoverlayDockPanel {
                key: CoverlayPanelKey::CdaVoxel { inst_id: def.meta.uuid, internal_id: target, kind: CoverlayPanelKind::VoxelTools },
                title: "Voxel Tools".to_string(),
                kind: CoverlayPanelKind::VoxelTools,
            });
            panels.push(CoverlayDockPanel {
                key: CoverlayPanelKey::CdaVoxel { inst_id: def.meta.uuid, internal_id: target, kind: CoverlayPanelKind::VoxelPalette },
                title: "Palette".to_string(),
                kind: CoverlayPanelKind::VoxelPalette,
            });
        }
    }
    if s.anim.enabled {
        panels.push(CoverlayDockPanel {
            key: CoverlayPanelKey::DirectVoxel { node_id: owner_for_keys, kind: CoverlayPanelKind::Anim },
            title: "Anim".to_string(),
            kind: CoverlayPanelKind::Anim,
        });
    }

    let mut desired: Vec<ViewportDockTab> = vec![ViewportDockTab::Viewport];
    for p in panels.iter().cloned() {
        desired.push(ViewportDockTab::Coverlay(p));
    }
    let sig = coverlay_dock_sig(owner, &desired);
    if s.dock_sig != sig {
        s.dock_sig = sig;
        let preset = owner.and_then(|o| ls_get(o, "preset_json"));
        let pal_ratio = preset_palette_ratio(preset.as_deref());
        let mut map: HashMap<CoverlayPanelKey, CoverlayDockPanel> = HashMap::new();
        for t in desired.iter() {
            if let ViewportDockTab::Coverlay(p) = t {
                map.insert(p.key, p.clone());
            }
        }
        let restored = owner
            .and_then(|o| ls_get(o, "layout_json"))
            .and_then(|s| serde_json::from_str::<DockState<ViewportDockTab>>(&s).ok())
            .map(|st| {
                st.filter_map_tabs(|t| match t {
                    ViewportDockTab::Viewport => Some(ViewportDockTab::Viewport),
                    ViewportDockTab::Coverlay(p) => map.get(&p.key).cloned().map(ViewportDockTab::Coverlay),
                })
            });
        let mut st = if let Some(st) = restored {
            let want: std::collections::HashSet<CoverlayPanelKey> = map.keys().copied().collect();
            let have: std::collections::HashSet<CoverlayPanelKey> = st
                .iter_all_tabs()
                .filter_map(|(_, t)| match t { ViewportDockTab::Coverlay(p) => Some(p.key), _ => None })
                .collect();
            if want.is_subset(&have) {
                st
            } else {
                build_default_viewport_dock_from_preset(preset, map.values().cloned().collect())
            }
        } else {
            build_default_viewport_dock_from_preset(preset, map.values().cloned().collect())
        };
        if let Some(r) = pal_ratio {
            apply_palette_ratio_once(&mut st, r);
        }
        // Keep the dock within bounds and avoid off-screen panels (desktop + wasm).
        clamp_viewport_dock_fractions(&mut st, VIEWPORT_KEEP_X, VIEWPORT_KEEP_Y);
        s.dock = st;
    }

    #[cfg(target_arch = "wasm32")]
    fn browse_local_cda(worker: &ComputeWorker) {
        use wasm_bindgen::JsCast;
        let Some(w) = web_sys::window() else { return; };
        let Some(d) = w.document() else { return; };
        let Ok(el) = d.create_element("input") else { return; };
        let Ok(input) = el.dyn_into::<web_sys::HtmlInputElement>() else { return; };
        input.set_type("file");
        input.set_accept(".cda");
        let worker2 = worker.clone();
        let input2 = input.clone();
        let onchange = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            let Some(files) = input2.files() else { return; };
            let Some(file) = files.item(0) else { return; };
            let Ok(url) = web_sys::Url::create_object_url_with_blob(&file) else { return; };
            if url.is_empty() { return; }
            worker2.send(HostCommand::LoadCdaUrl { url });
        });
        input.set_onchange(Some(onchange.as_ref().unchecked_ref()));
        onchange.forget();
        input.click();
    }

    fn ui_parameters(
        ui: &mut egui::Ui,
        s: &mut UiState,
        worker: &ComputeWorker,
        voxel_stats: &cunning_voxel_faces::VoxelFacesStats,
        display_options: &mut DisplayOptions,
    ) {
        ui.heading("Cunning Player");
        if let Some(e) = &s.error { ui.colored_label(egui::Color32::RED, e); }
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.perf.show, "Perf");
            if s.perf.show {
                ui.monospace(format!("fps {:.0}  {:.2}ms  q {}  cook {}ms", s.perf.fps, s.perf.ms, worker.queue_len(), s.perf.last_cook_ms));
                ui.monospace(format!("voxel_chunks {}  dirty_up {}  atlas_kb {:.1}", voxel_stats.visible_chunks, voxel_stats.dirty_chunks_uploaded, voxel_stats.atlas_upload_bytes as f64 / 1024.0));
            }
        });
        ui.checkbox(&mut s.auto_cook, "Auto cook");
        ui.horizontal(|ui| {
            ui.label("CDA");
            #[cfg(target_arch = "wasm32")]
            if ui.button("Browse").clicked() { s.pending_clear = true; browse_local_cda(worker); }
            if ui.button("Load").clicked() { s.pending_clear = true; s.url = normalize_cda_input_to_url(&s.url); worker.send(HostCommand::LoadCdaUrl { url: s.url.clone() }); }
            if ui.button("Cook").clicked() { worker.send(HostCommand::Cook); }
        });
        ui.add(egui::TextEdit::singleline(&mut s.url).desired_width(f32::INFINITY));
        ui.separator();
        ui.collapsing("Turntable", |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut display_options.turntable.enabled, "Enabled");
                ui.label("Speed");
                ui.add(egui::DragValue::new(&mut display_options.turntable.speed_deg_per_sec).speed(1.0).range(0.0..=360.0));
            });
            ui.horizontal(|ui| {
                ui.label("Elevation");
                ui.add(egui::DragValue::new(&mut display_options.turntable.elevation_deg).speed(1.0).range(-80.0..=80.0));
                ui.label("Distance");
                ui.add(egui::DragValue::new(&mut display_options.turntable.distance_factor).speed(0.02).range(0.1..=10.0));
            });
        });
        if let Some(def) = &s.def {
            ui.separator();
            ui.label(format!("Asset: {} ({})", def.meta.name, def.meta.uuid));
            ui.separator();
            ui.label(format!("Parameters ({})", def.promoted_params.len()));
            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut pps = def.promoted_params.clone();
                pps.sort_by(|a, b| a.group.cmp(&b.group).then(a.order.cmp(&b.order)).then(a.name.cmp(&b.name)));
                let mut cur_group = String::new();
                for pp in pps {
                    if pp.group != cur_group { cur_group = pp.group.clone(); if !cur_group.is_empty() { ui.label(egui::RichText::new(&cur_group).strong()); } }
                    let label = if pp.label.is_empty() { &pp.name } else { &pp.label };
                    let name = pp.name.clone();
                    let out_val = {
                        let v = s.overrides.entry(name.clone()).or_insert(pp.default_value.clone());
                        if param_widget(ui, label, v) { Some(v.clone()) } else { None }
                    };
                    if let Some(value) = out_val { worker.send(HostCommand::SetOverride { name, value }); if s.auto_cook { worker.send(HostCommand::Cook); } }
                }
            });
        } else {
            ui.separator();
            ui.weak("Loading CDA...");
        }
    }

    fn ui_coverlay_manager(ui: &mut egui::Ui, s: &mut UiState) {
        let Some(def) = s.def.as_deref() else { ui.weak("No CDA loaded."); return; };
        let mut hud = def.hud_units.clone();
        let mut cov = def.coverlay_units.clone();
        hud.sort_by(|a, b| a.order.cmp(&b.order).then(a.label.cmp(&b.label)));
        cov.sort_by(|a, b| a.order.cmp(&b.order).then(a.label.cmp(&b.label)));
        let mut next_active_hud = s.active_hud_unit;
        let mut next_coverlay = s.coverlay_enabled_units.clone();
        ui.label("HUD (single-select)");
        if hud.is_empty() { ui.weak("No HUD units exposed by this CDA."); } else {
            for u in &hud { if ui.radio(next_active_hud == Some(u.node_id), &u.label).clicked() { next_active_hud = Some(u.node_id); } }
            if ui.button("Clear HUD").clicked() { next_active_hud = None; }
        }
        ui.separator();
        ui.label("Panels (multi-select)");
        if cov.is_empty() { ui.weak("No coverlay panels exposed by this CDA."); } else {
            for u in &cov {
                let mut on = next_coverlay.contains(&u.node_id);
                let label = u.icon.as_ref().map(|ic| format!("{}  {}", ic, u.label)).unwrap_or_else(|| u.label.clone());
                if ui.checkbox(&mut on, label).changed() {
                    if on {
                        next_coverlay.insert(u.node_id);
                        if hud.iter().any(|h| h.node_id == u.node_id) {
                            next_active_hud = Some(u.node_id);
                        }
                    } else {
                        next_coverlay.remove(&u.node_id);
                    }
                }
            }
        }
        if let Some(hud_id) = next_active_hud {
            if cov.iter().any(|u| u.node_id == hud_id) {
                next_coverlay.insert(hud_id);
            }
        }
        s.active_hud_unit = next_active_hud;
        s.coverlay_enabled_units = next_coverlay;
    }

    fn ui_voxel_tools(ui: &mut egui::Ui, s: &mut UiState, worker: &ComputeWorker, display_options: &mut DisplayOptions, key: CoverlayPanelKey) {
        let Some(def_arc) = s.def.clone() else { ui.weak("No CDA loaded."); return; };
        let def = def_arc.as_ref();
        let target = match key {
            CoverlayPanelKey::CdaVoxel { internal_id, .. } | CoverlayPanelKey::CdaUnit { unit_id: internal_id, .. } => Some(internal_id),
            CoverlayPanelKey::DirectVoxel { node_id, .. } => Some(node_id),
            _ => resolve_voxel_target(def, s.active_hud_unit, &s.coverlay_enabled_units),
        };
        let Some(target) = target else { ui.weak("Enable a VoxelEdit coverlay unit to edit."); return; };
        if s.voxel.target != Some(target) { init_voxel_state_from_def(&mut s.voxel, def, target); }
        struct Backend<'a> { voxel: &'a mut VoxelEditUiState, worker: &'a ComputeWorker, target: NodeId, selection: &'a [IVec3] }
        impl<'a> vxui::VoxelToolsBackend for Backend<'a> {
            fn selection_cells(&self) -> &[IVec3] { self.selection }
            fn undo(&mut self) {
                if self.voxel.cmds.undo() {
                    self.voxel.reset_bake();
                    self.worker.send(HostCommand::SetInternalOverride { node: self.target, param: "cmds_json".to_string(), value: ParameterValue::String(self.voxel.cmds.to_json()) });
                    self.worker.send(HostCommand::Cook);
                }
            }
            fn redo(&mut self) {
                if self.voxel.cmds.redo() {
                    self.voxel.reset_bake();
                    self.worker.send(HostCommand::SetInternalOverride { node: self.target, param: "cmds_json".to_string(), value: ParameterValue::String(self.voxel.cmds.to_json()) });
                    self.worker.send(HostCommand::Cook);
                }
            }
            fn push_op(&mut self, op: vox::DiscreteVoxelOp) {
                self.voxel.cmds.push(op);
                self.voxel.reset_bake();
                self.worker.send(HostCommand::SetInternalOverride { node: self.target, param: "cmds_json".to_string(), value: ParameterValue::String(self.voxel.cmds.to_json()) });
                self.worker.send(HostCommand::Cook);
            }
        }
        let mut ops = s.voxel_ops.remove(&key).unwrap_or_default();
        {
            let mut backend = Backend { voxel: &mut s.voxel, worker, target, selection: &s.voxel_selection };
            vxui::draw_voxel_tools_panel(ui, &mut s.voxel_tools, &mut ops, key, &mut backend, &mut s.voxel_overlay, &s.voxel_hud, display_options);
        }
        s.voxel_ops.insert(key, ops);
    }

    fn ui_voxel_palette(ui: &mut egui::Ui, s: &mut UiState) {
        vxui::draw_voxel_palette_panel(ui, &mut s.voxel_tools);
    }

    fn ui_anim(ui: &mut egui::Ui, s: &mut UiState) {
        if !s.anim.enabled { ui.weak("This asset does not expose a timeline parameter."); return; }
        ui.horizontal(|ui| {
            if ui.button("⏮").clicked() { s.anim.current_frame = s.anim.start_frame; s.anim.is_playing = false; s.anim.play_accum = 0.0; }
            if ui.button("◀").clicked() { s.anim.current_frame = (s.anim.current_frame - 1.0).max(s.anim.start_frame); s.anim.is_playing = false; s.anim.play_accum = 0.0; }
            let play = if s.anim.is_playing { "⏸" } else { "▶" };
            if ui.button(play).clicked() { s.anim.is_playing = !s.anim.is_playing; s.anim.play_accum = 0.0; }
            if ui.button("▶").clicked() { s.anim.current_frame = (s.anim.current_frame + 1.0).min(s.anim.end_frame); s.anim.is_playing = false; s.anim.play_accum = 0.0; }
            if ui.button("⏭").clicked() { s.anim.current_frame = s.anim.end_frame; s.anim.is_playing = false; s.anim.play_accum = 0.0; }
        });
        ui.separator();
        if let Some(n) = &s.anim.param_name { ui.weak(format!("Param: {n}")); }
        ui.horizontal(|ui| {
            ui.label("Frame");
            ui.add(egui::DragValue::new(&mut s.anim.current_frame).speed(1.0).range(s.anim.start_frame..=s.anim.end_frame));
            ui.label("FPS");
            ui.add(egui::DragValue::new(&mut s.anim.fps).speed(0.5).range(1.0..=240.0));
        });
        ui.horizontal(|ui| {
            ui.label("Start");
            ui.add(egui::DragValue::new(&mut s.anim.start_frame).speed(1.0));
            ui.label("End");
            ui.add(egui::DragValue::new(&mut s.anim.end_frame).speed(1.0));
        });
        if s.anim.end_frame < s.anim.start_frame { std::mem::swap(&mut s.anim.start_frame, &mut s.anim.end_frame); }
    }

    struct Viewer<'a> {
        s: &'a mut UiState,
        worker: &'a ComputeWorker,
        voxel_stats: &'a cunning_voxel_faces::VoxelFacesStats,
        display_options: &'a mut DisplayOptions,
    }
    impl<'a> TabViewer for Viewer<'a> {
        type Tab = ViewportDockTab;
        fn title(&mut self, t: &mut Self::Tab) -> egui::WidgetText {
            match t {
                ViewportDockTab::Viewport => "".into(),
                ViewportDockTab::Coverlay(p) => p.title.clone().into(),
            }
        }
        fn ui(&mut self, ui: &mut egui::Ui, t: &mut Self::Tab) {
            match t {
                ViewportDockTab::Viewport => {
                    let r = ui.max_rect();
                    self.s.viewport_rect = Some(r);
                    ui.allocate_space(ui.available_size());
                }
                ViewportDockTab::Coverlay(p) => {
                    let fill = ui.style().visuals.panel_fill;
                    egui::Frame::NONE.fill(fill).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                        match p.kind {
                            CoverlayPanelKind::Parameters => ui_parameters(ui, self.s, self.worker, self.voxel_stats, self.display_options),
                            CoverlayPanelKind::Manager => ui_coverlay_manager(ui, self.s),
                            CoverlayPanelKind::VoxelTools => ui_voxel_tools(ui, self.s, self.worker, self.display_options, p.key),
                            CoverlayPanelKind::VoxelPalette => ui_voxel_palette(ui, self.s),
                            CoverlayPanelKind::Anim => ui_anim(ui, self.s),
                            _ => { ui.weak("Panel not implemented in Player."); }
                        }
                    });
                }
            }
        }
        fn clear_background(&self, t: &Self::Tab) -> bool { !matches!(t, ViewportDockTab::Viewport) }
        fn allowed_in_windows(&self, _tab: &mut Self::Tab) -> bool { false }
        fn closeable(&mut self, _tab: &mut Self::Tab) -> bool { false }
    }

    // Important: keep the center background transparent so Bevy's 3D pass stays visible.
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 0)))
        .show(ctx.ctx_mut(), |ui| {
        // Top/side chrome (Player variant, aligned with editor layout).
        egui::TopBottomPanel::top("viewport_3d_header")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)).fill(ui.style().visuals.panel_fill.linear_multiply(0.8)))
            .resizable(false)
            .min_height(32.0)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("3D Viewport").strong().color(egui::Color32::from_white_alpha(100)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.separator();
                        let m = &mut display_options.final_geometry_display_mode;
                        use cunning_viewport::viewport_options::DisplayMode;
                        if side_icon_button(ui, "shaded_wire", cunning_viewport::icons::ViewportIcons::SHADED_WIRE, *m == DisplayMode::ShadedAndWireframe).on_hover_text("Shaded + Wireframe").clicked() { *m = DisplayMode::ShadedAndWireframe; }
                        if side_icon_button(ui, "wireframe", cunning_viewport::icons::ViewportIcons::WIREFRAME, *m == DisplayMode::Wireframe).on_hover_text("Wireframe").clicked() { *m = DisplayMode::Wireframe; }
                        if side_icon_button(ui, "shaded", cunning_viewport::icons::ViewportIcons::SHADED, *m == DisplayMode::Shaded).on_hover_text("Shaded").clicked() { *m = DisplayMode::Shaded; }
                        ui.separator();
                        use cunning_viewport::viewport_options::ViewportLightingMode;
                        egui::ComboBox::from_id_salt("lighting_mode")
                            .selected_text(match display_options.lighting_mode { ViewportLightingMode::HeadlightOnly => "Headlight", ViewportLightingMode::FullLighting => "Full", ViewportLightingMode::FullLightingWithShadow => "Full+Shadow" })
                            .width(80.0).show_ui(ui, |ui| {
                                ui.selectable_value(&mut display_options.lighting_mode, ViewportLightingMode::HeadlightOnly, "Headlight Only");
                                ui.selectable_value(&mut display_options.lighting_mode, ViewportLightingMode::FullLighting, "Full Lighting");
                                ui.selectable_value(&mut display_options.lighting_mode, ViewportLightingMode::FullLightingWithShadow, "Full + Shadow");
                            });
                        ui.separator();
                        ui.toggle_value(&mut display_options.wireframe_ghost_mode, "Ghost").on_hover_text("Ghost Wireframe");
                        ui.toggle_value(&mut display_options.turntable.enabled, "Turntable").on_hover_text("Auto-frame + rotate (disables manual camera controls)");
                        ui.add_enabled_ui(display_options.turntable.enabled, |ui| {
                            ui.add(egui::DragValue::new(&mut display_options.turntable.speed_deg_per_sec).speed(1.0).range(0.0..=360.0)).on_hover_text("Turntable speed (deg/sec)");
                        });
                        ui.separator();
                    });
                });
            });

        egui::SidePanel::left("handle_controls_panel")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 8)).fill(ui.style().visuals.panel_fill.linear_multiply(0.8)))
            .resizable(false)
            .width_range(if display_options.is_handle_controls_collapsed { 12.0..=12.0 } else { 32.0..=32.0 })
            .show_inside(ui, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    if display_options.is_handle_controls_collapsed {
                        if side_icon_button(ui, "expand_controls", cunning_viewport::icons::ViewportIcons::EXPAND, false).on_hover_text("Show Controls").clicked() {
                            display_options.is_handle_controls_collapsed = false;
                        }
                    } else {
                        if side_icon_button(ui, "collapse_controls", cunning_viewport::icons::ViewportIcons::COLLAPSE, false).on_hover_text("Hide Controls").clicked() {
                            display_options.is_handle_controls_collapsed = true;
                        }
                        ui.separator();
                        ui.add(egui::DragValue::new(&mut display_options.camera_speed).speed(0.1).range(0.1..=100.0)).on_hover_text("Camera Speed (Drag)");
                    }
                });
            });

        let mut dock_state = std::mem::replace(&mut s.dock, DockState::new(vec![ViewportDockTab::Viewport]));
        {
            let mut viewer = Viewer { s: &mut *s, worker: &worker, voxel_stats: &voxel_stats, display_options: &mut *display_options };
            egui_dock::CoverlayDockArea::new(&mut dock_state)
                .id(egui::Id::new("cunning_player_viewport_dock"))
                .show_inside(ui, &mut viewer);
        }
        s.dock = dock_state;
    });

    // Persist layout (per asset uuid) when not dragging.
    if let Some(owner) = owner {
        if !ctx.ctx_mut().input(|i| i.pointer.any_down()) {
            if let Ok(json) = serde_json::to_string(&coverlay_strip_runtime(s.dock.clone())) {
                ls_set(owner, "layout_json", &json);
            }
        }
    }

    // HUD (single-select): lightweight overlay area (node-type specific if known).
    if let (Some(def_arc), Some(hud_id)) = (s.def.clone(), s.active_hud_unit) {
        let def = def_arc.as_ref();
        let hud_label = def
            .hud_units
            .iter()
            .find(|u| u.node_id == hud_id)
            .map(|u| u.label.clone())
            .unwrap_or_else(|| "HUD".to_string());
        let type_id = def
            .nodes
            .iter()
            .find(|n| n.id == hud_id)
            .map(|n| n.type_id.clone())
            .unwrap_or_default();

        let base = s.viewport_rect.map(|r| r.min).unwrap_or_else(|| egui::pos2(0.0, 0.0));
        egui::Area::new("cunning_player_hud".into())
            .fixed_pos(base + egui::vec2(10.0, 10.0))
            .show(ctx.ctx_mut(), |ui| {
                let frame = egui::Frame::default()
                    .fill(egui::Color32::from_black_alpha(160))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_white_alpha(50)))
                    .rounding(egui::Rounding::same(6u8))
                    .inner_margin(egui::Margin::same(8i8));
                frame.show(ui, |ui| {
                    ui.label(egui::RichText::new(hud_label).strong());
                    ui.label(egui::RichText::new(type_id.clone()).small().weak());

                    if type_id == "cunning.voxel.edit" {
                        ui.separator();
                        ui.label(format!("Palette: {}", s.voxel_tools.palette_index.max(1)));
                        ui.label(format!("Brush: {:.3}", s.voxel_tools.brush_radius));
                        if let Some(c) = s.voxel.last_cell {
                            ui.label(format!("Cell: {}, {}, {}", c.x, c.y, c.z));
                        } else {
                            ui.label("Cell: (hover)".to_string());
                        }
                        ui.weak("LMB paint, Shift subtract");
                    } else {
                        ui.separator();
                        ui.weak("HUD not implemented for this node type (yet).");
                    }
                });
            });
    }
}

fn param_widget(ui: &mut egui::Ui, label: &str, v: &mut ParameterValue) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        match v {
            
            ParameterValue::Float(x) => { changed |= ui.add(egui::DragValue::new(x).speed(0.01)).changed(); }
            ParameterValue::Int(x) => { changed |= ui.add(egui::DragValue::new(x).speed(1)).changed(); }
            ParameterValue::Bool(x) => { changed |= ui.checkbox(x, "").changed(); }
            ParameterValue::String(s) => { changed |= ui.text_edit_singleline(s).changed(); }
            ParameterValue::Vec2(p) => {
                changed |= ui.add(egui::DragValue::new(&mut p.x).speed(0.01)).changed();
                changed |= ui.add(egui::DragValue::new(&mut p.y).speed(0.01)).changed();
            }

            ParameterValue::Vec3(p) => {
                changed |= ui.add(egui::DragValue::new(&mut p.x).speed(0.01)).changed();
                changed |= ui.add(egui::DragValue::new(&mut p.y).speed(0.01)).changed();
                changed |= ui.add(egui::DragValue::new(&mut p.z).speed(0.01)).changed();
            }

            ParameterValue::Vec4(p) => {
                changed |= ui.add(egui::DragValue::new(&mut p.x).speed(0.01)).changed();
                changed |= ui.add(egui::DragValue::new(&mut p.y).speed(0.01)).changed();
                changed |= ui.add(egui::DragValue::new(&mut p.z).speed(0.01)).changed();
                changed |= ui.add(egui::DragValue::new(&mut p.w).speed(0.01)).changed();
            }

            ParameterValue::Color(rgb) => {
                let mut c = [rgb.x, rgb.y, rgb.z];
                changed |= ui.color_edit_button_rgb(&mut c).changed();
                if changed { *rgb = Vec3::new(c[0], c[1], c[2]); }
            }
            ParameterValue::Color4(rgba) => {
                let mut c = [rgba.x, rgba.y, rgba.z, rgba.w];
                changed |= ui.color_edit_button_rgba_unmultiplied(&mut c).changed();
                if changed { *rgba = Vec4::new(c[0], c[1], c[2], c[3]); }
            }
            _ => { ui.label("(unsupported in player)"); }
        }
    });
    changed
}

fn normalize_cda_input_to_url(s: &str) -> String {
    let t = s.trim();
    if t.starts_with("http://") || t.starts_with("https://") { return t.into(); }
    let p = t.replace('\\', "/");
    let p = p.strip_prefix("file:///").unwrap_or(&p);
    let p = if let Some(i) = p.find("/ghost1.0/") { &p[i + "/ghost1.0".len()..] } else { p };
    let p = if p.starts_with('/') { p.to_string() } else { format!("/{p}") };
    p.replace(' ', "%20")
}

fn build_primitive_normals_mesh(g: &Geometry) -> Mesh {
    let mut pos: Vec<Vec3> = Vec::new();
    let mut nor: Vec<Vec3> = Vec::new();
    let positions = g.get_point_attribute(attrs::P).and_then(|a| a.as_slice::<Vec3>()).map(|s| s.to_vec()).unwrap_or_default();
    for p in g.primitives().values().iter() {
        let GeoPrimitive::Polygon(poly) = p else { continue; };
        if poly.vertices.len() < 3 { continue; }
        let p0 = g.get_pos_by_vertex(poly.vertices[0], &positions);
        let p1 = g.get_pos_by_vertex(poly.vertices[1], &positions);
        let p2 = g.get_pos_by_vertex(poly.vertices[2], &positions);
        let n = (p1 - p0).cross(p2 - p0);
        let n = if n.length_squared() > 1e-12 { n.normalize() } else { Vec3::Y };
        let mut c = Vec3::ZERO;
        for &v in &poly.vertices { c += g.get_pos_by_vertex(v, &positions); }
        c /= poly.vertices.len() as f32;
        pos.push(c);
        nor.push(n);
    }
    let mut m = Mesh::new(PrimitiveTopology::PointList, RenderAssetUsages::default());
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
    m
}

#[inline]
fn raycast_discrete_dda(
    grid: &vox::DiscreteSdfGrid,
    origin: Vec3,
    dir: Vec3,
    voxel_size: f32,
    max_dist_world: f32,
) -> Option<(f32, IVec3, IVec3, Vec3)> {
    if grid.voxels.is_empty() {
        return None;
    }
    let vs = voxel_size.max(0.001);
    let d = dir.normalize_or_zero();
    if d.length_squared() <= 1.0e-12 {
        return None;
    }
    let o = origin / vs;
    let mut cell = o.floor().as_ivec3();
    let step = IVec3::new(
        if d.x >= 0.0 { 1 } else { -1 },
        if d.y >= 0.0 { 1 } else { -1 },
        if d.z >= 0.0 { 1 } else { -1 },
    );
    let inv = Vec3::new(
        if d.x.abs() < 1.0e-12 { f32::INFINITY } else { 1.0 / d.x.abs() },
        if d.y.abs() < 1.0e-12 { f32::INFINITY } else { 1.0 / d.y.abs() },
        if d.z.abs() < 1.0e-12 { f32::INFINITY } else { 1.0 / d.z.abs() },
    );
    let next_boundary = |c: i32, dc: f32, s: i32| -> f32 {
        if s > 0 {
            c as f32 + 1.0 - dc
        } else {
            dc - c as f32
        }
    };
    let mut t_max = Vec3::new(
        next_boundary(cell.x, o.x, step.x) * inv.x,
        next_boundary(cell.y, o.y, step.y) * inv.y,
        next_boundary(cell.z, o.z, step.z) * inv.z,
    );
    let t_delta = inv;
    let max_t = (max_dist_world / vs).max(0.0);
    let mut t = 0.0f32;
    let mut n = IVec3::Y;
    for _ in 0..8192 {
        if t > max_t {
            break;
        }
        if grid.is_solid(cell.x, cell.y, cell.z) {
            let h = (o + d * t) * vs;
            return Some((t * vs, cell, n, h));
        }
        if t_max.x < t_max.y {
            if t_max.x < t_max.z {
                cell.x += step.x;
                t = t_max.x;
                t_max.x += t_delta.x;
                n = IVec3::new(-step.x, 0, 0);
            } else {
                cell.z += step.z;
                t = t_max.z;
                t_max.z += t_delta.z;
                n = IVec3::new(0, 0, -step.z);
            }
        } else if t_max.y < t_max.z {
            cell.y += step.y;
            t = t_max.y;
            t_max.y += t_delta.y;
            n = IVec3::new(0, -step.y, 0);
        } else {
            cell.z += step.z;
            t = t_max.z;
            t_max.z += t_delta.z;
            n = IVec3::new(0, 0, -step.z);
        }
    }
    None
}
