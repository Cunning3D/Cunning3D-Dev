//! Voxel editor (CPU MVP): translate viewport interactions into DiscreteVoxelCmdList ops.
use bevy::math::primitives::InfinitePlane3d;
use bevy::prelude::*;
use bevy_ecs::system::SystemParam;
use std::collections::{HashMap, HashSet};

use crate::camera::ViewportInteractionState;
use crate::coverlay_bevy_ui::{read_voxel_cmds, resolve_voxel_edit_target, voxel_size_for_target, write_voxel_cmds, write_voxel_mask, CoverlayUiWantsInput, VoxelAddType, VoxelBrushShape, VoxelEditTarget, VoxelHudInfo, VoxelOverlaySettings, VoxelPaintType, VoxelSelectType, VoxelToolMode, VoxelToolState};
use crate::input::NavigationInput;
use crate::nodes::{port_key, NodeGraphResource, NodeId, PortId};
use crate::tabs_system::viewport_3d::ViewportLayout;
use crate::ui::UiState;
use crate::GraphChanged;

use cunning_kernel::algorithms::algorithms_editor::voxel::{self as vox, DiscreteVoxelCmdList, DiscreteVoxelOp};

pub mod volume;
pub mod gpu_raycast;
pub mod gpu_brush;
use gpu_raycast::VoxelHitResult;
use crate::nodes::gpu::runtime::GpuRuntime;
use gpu_brush::GpuBrush;
use volume::{VoxelChanges, VoxelUndoStack};

#[derive(Clone, Copy)]
struct BrushStamp {
    target: VoxelEditTarget,
    subtract: bool,
    palette_index: u8,
    sym_x: bool,
    sym_y: bool,
    sym_z: bool,
    brush_offset: IVec3,
    voxel_size: f32,
}

#[derive(Default)]
struct BrushAsyncState {
    pending: HashMap<u64, BrushStamp>,
    ready: Vec<gpu_brush::BrushAsyncResult>,
}

#[derive(Default)]
struct RaycastDdaState {
    key_origin: Vec3,
    key_dir: Vec3,
    key_vs: f32,
    active: bool,
    done: bool,
    o: Vec3,
    d: Vec3,
    cell: IVec3,
    step: IVec3,
    t_max: Vec3,
    t_delta: Vec3,
    max_t: f32,
    t: f32,
    n: IVec3,
    last: Option<(f32, IVec3, IVec3, Vec3)>,
}

impl RaycastDdaState {
    #[inline]
    fn ensure(&mut self, grid: &vox::DiscreteSdfGrid, origin: Vec3, dir: Vec3, voxel_size: f32, max_dist_world: f32) {
        let vs = voxel_size.max(0.001);
        if !self.active
            || self.key_vs != vs
            || self.key_origin != origin
            || self.key_dir != dir
        {
            self.key_origin = origin;
            self.key_dir = dir;
            self.key_vs = vs;
            self.last = None;
            self.done = false;
            self.active = false;
            if grid.voxels.is_empty() { return; }
            let d = dir.normalize_or_zero();
            if d.length_squared() <= 1.0e-12 { return; }
            let o = origin / vs;
            let cell = o.floor().as_ivec3();
            let step = IVec3::new(if d.x >= 0.0 { 1 } else { -1 }, if d.y >= 0.0 { 1 } else { -1 }, if d.z >= 0.0 { 1 } else { -1 });
            let inv = Vec3::new(
                if d.x.abs() < 1.0e-12 { f32::INFINITY } else { 1.0 / d.x.abs() },
                if d.y.abs() < 1.0e-12 { f32::INFINITY } else { 1.0 / d.y.abs() },
                if d.z.abs() < 1.0e-12 { f32::INFINITY } else { 1.0 / d.z.abs() },
            );
            let next_boundary = |c: i32, dc: f32, s: i32| -> f32 { if s > 0 { (c as f32 + 1.0 - dc) } else { (dc - c as f32) } };
            self.o = o;
            self.d = d;
            self.cell = cell;
            self.step = step;
            self.t_max = Vec3::new(next_boundary(cell.x, o.x, step.x) * inv.x, next_boundary(cell.y, o.y, step.y) * inv.y, next_boundary(cell.z, o.z, step.z) * inv.z);
            self.t_delta = inv;
            self.max_t = (max_dist_world / vs).max(0.0);
            self.t = 0.0;
            self.n = IVec3::Y;
            self.active = true;
        }
    }

    #[inline]
    fn step(&mut self, grid: &vox::DiscreteSdfGrid, max_steps: u32) -> Option<(f32, IVec3, IVec3, Vec3)> {
        if !self.active || self.done { return None; }
        for _ in 0..max_steps {
            if self.t > self.max_t { self.done = true; break; }
            if grid.is_solid(self.cell.x, self.cell.y, self.cell.z) {
                let vs = self.key_vs.max(0.001);
                let h = (self.o + self.d * self.t) * vs;
                return Some((self.t * vs, self.cell, self.n, h));
            }
            if self.t_max.x < self.t_max.y {
                if self.t_max.x < self.t_max.z { self.cell.x += self.step.x; self.t = self.t_max.x; self.t_max.x += self.t_delta.x; self.n = IVec3::new(-self.step.x, 0, 0); }
                else { self.cell.z += self.step.z; self.t = self.t_max.z; self.t_max.z += self.t_delta.z; self.n = IVec3::new(0, 0, -self.step.z); }
            } else {
                if self.t_max.y < self.t_max.z { self.cell.y += self.step.y; self.t = self.t_max.y; self.t_max.y += self.t_delta.y; self.n = IVec3::new(0, -self.step.y, 0); }
                else { self.cell.z += self.step.z; self.t = self.t_max.z; self.t_max.z += self.t_delta.z; self.n = IVec3::new(0, 0, -self.step.z); }
            }
        }
        None
    }
}

#[derive(Resource, Default)]
pub struct VoxelSelection { pub cells: HashSet<IVec3> }

#[derive(Resource, Default)]
struct StrokeState {
    last_cell: Option<IVec3>,
    line_start: Option<IVec3>,
    region_start: Option<IVec3>,
    move_anchor: Option<IVec3>,
    drawing: bool,
}

#[derive(Default)]
struct VoxelCmdBakeCache {
    target: Option<VoxelEditTarget>,
    base_node: Option<uuid::Uuid>,
    voxel_size: f32,
    cmds_len: usize,
    cmds_cursor: usize,
    bake: vox::DiscreteBakeState,
    grid: vox::DiscreteSdfGrid,
    bounds: Option<(IVec3, IVec3)>,
    bounds_dirty: bool,
}

impl VoxelCmdBakeCache {
    #[inline]
    fn reset(&mut self, target: VoxelEditTarget, base_node: Option<uuid::Uuid>, base_grid: Option<vox::DiscreteSdfGrid>, voxel_size: f32) {
        let vs = voxel_size.max(0.001);
        self.target = Some(target);
        self.base_node = base_node;
        self.voxel_size = vs;
        self.cmds_len = 0;
        self.cmds_cursor = 0;
        self.bake = vox::DiscreteBakeState { baked_cursor: 0 };
        self.grid = base_grid.unwrap_or_else(|| vox::DiscreteSdfGrid::new(vs));
        self.bounds = self.grid.bounds();
        self.bounds_dirty = false;
    }

    #[inline]
    fn bounds_apply_fast(&mut self, op: &DiscreteVoxelOp) {
        let vs = self.voxel_size.max(0.001);
        let mut include = |mn: IVec3, mx: IVec3| {
            self.bounds = Some(if let Some((bmn, bmx)) = self.bounds { (bmn.min(mn), bmx.max(mx)) } else { (mn, mx) });
        };
        match op {
            DiscreteVoxelOp::SetVoxel { x, y, z, .. } | DiscreteVoxelOp::Paint { x, y, z, .. } => include(IVec3::new(*x, *y, *z), IVec3::new(*x, *y, *z)),
            DiscreteVoxelOp::BoxAdd { min, max, .. } | DiscreteVoxelOp::PerlinFill { min, max, .. } => include(*min, *max),
            DiscreteVoxelOp::SphereAdd { center, radius, .. } => {
                let c = (*center / vs).floor().as_ivec3();
                let r = (*radius / vs).ceil() as i32;
                include(c - IVec3::splat(r), c + IVec3::splat(r));
            }
            DiscreteVoxelOp::MoveSelected { cells, delta } => {
                if cells.is_empty() { return; }
                let mut mn = IVec3::splat(i32::MAX);
                let mut mx = IVec3::splat(i32::MIN);
                for c in cells.iter() { mn = mn.min(*c); mx = mx.max(*c); }
                include(mn, mx);
                include(mn + *delta, mx + *delta);
                self.bounds_dirty = true;
            }
            DiscreteVoxelOp::Extrude { cells, delta, .. } | DiscreteVoxelOp::CloneSelected { cells, delta, .. } => {
                if cells.is_empty() { return; }
                let mut mn = IVec3::splat(i32::MAX);
                let mut mx = IVec3::splat(i32::MIN);
                for c in cells.iter() { mn = mn.min(*c); mx = mx.max(*c); }
                include(mn + *delta, mx + *delta);
            }
            DiscreteVoxelOp::ClearAll => { self.bounds = None; self.bounds_dirty = false; }
            DiscreteVoxelOp::TrimToOrigin => self.bounds_dirty = true,
            DiscreteVoxelOp::RemoveVoxel { .. } | DiscreteVoxelOp::BoxRemove { .. } | DiscreteVoxelOp::SphereRemove { .. } => self.bounds_dirty = true,
        }
    }

    #[inline]
    fn ensure_baked(&mut self, target: VoxelEditTarget, base_node: Option<uuid::Uuid>, base_grid: Option<vox::DiscreteSdfGrid>, voxel_size: f32, cmds: &DiscreteVoxelCmdList) {
        let vs = voxel_size.max(0.001);
        if self.target != Some(target) || self.base_node != base_node || (self.voxel_size - vs).abs() > 0.0 {
            self.reset(target, base_node, base_grid, vs);
        }
        let cur = cmds.cursor.min(cmds.ops.len());
        if self.cmds_len == cmds.ops.len() && self.cmds_cursor == cur { return; }
        if cur < self.cmds_cursor { self.bounds_dirty = true; }
        if cur > self.cmds_cursor {
            for op in cmds.ops[self.cmds_cursor..cur].iter() { self.bounds_apply_fast(op); }
        }
        vox::discrete::bake_cmds_incremental(&mut self.grid, cmds, &mut self.bake);
        self.cmds_len = cmds.ops.len();
        self.cmds_cursor = cur;
    }

    #[inline]
    fn ensure_bounds_exact(&mut self) {
        if !self.bounds_dirty { return; }
        self.bounds = self.grid.bounds();
        self.bounds_dirty = false;
    }
}

#[inline]
fn first_src(graph: &crate::nodes::NodeGraph, to: NodeId, to_port: &PortId) -> Option<(NodeId, PortId)> {
    let mut srcs: Vec<(crate::nodes::ConnectionId, NodeId, PortId)> = graph
        .connections
        .values()
        .filter(|c| c.to_node == to && c.to_port.as_str() == to_port.as_str())
        .map(|c| (c.id, c.from_node, c.from_port.clone()))
        .collect();
    srcs.sort_by(|a, b| a.0.cmp(&b.0));
    srcs.into_iter().next().map(|(_, n, p)| (n, p))
}

#[inline]
fn cached_output_geo(graph: &crate::nodes::NodeGraph, nid: NodeId, port: &PortId) -> Option<std::sync::Arc<crate::mesh::Geometry>> {
    if port_key::is_cda_port_key(port) { graph.port_geometry_cache.get(&(nid, port.clone())).cloned() } else { graph.geometry_cache.get(&nid).cloned() }
}

#[inline]
fn base_grid_for_target(graph: &crate::nodes::NodeGraph, target: VoxelEditTarget) -> (Option<uuid::Uuid>, Option<vox::DiscreteSdfGrid>) {
    let VoxelEditTarget::Direct(node_id) = target else { return (None, None); };
    let in0 = port_key::in0();
    let Some((src_n, src_p)) = first_src(graph, node_id, &in0) else { return (None, None); };
    let Some(g) = cached_output_geo(graph, src_n, &src_p) else { return (None, None); };
    let Some(nid) = g
        .get_detail_attribute("__voxel_node")
        .and_then(|a| a.as_slice::<String>())
        .and_then(|v| v.first())
        .and_then(|s| uuid::Uuid::parse_str(s.trim()).ok())
    else { return (None, None); };
    (Some(nid), cunning_kernel::nodes::voxel::voxel_edit::voxel_render_get_grid(nid))
}

pub struct VoxelEditorPlugin;

impl Plugin for VoxelEditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StrokeState>()
            .init_resource::<VoxelSelection>()
            .init_resource::<VoxelHitResult>()
            .init_resource::<VoxelUndoStack>()
            .init_resource::<VoxelHudInfo>()
            .init_resource::<VoxelOverlaySettings>()
            .init_resource::<VoxelAiPromptStampQueue>()
            .init_resource::<VoxelAiPromptStampJobs>()
            .add_systems(Update, (voxel_ai_prompt_stamp_system, voxel_editor_input_system, voxel_editor_preview_system));
    }
}

/// Coverlay-only request queue for AI Prompt Stamp.
#[derive(Resource, Default)]
pub struct VoxelAiPromptStampQueue { pub queue: Vec<VoxelAiPromptStampRequest> }

#[derive(Clone, Debug)]
pub struct VoxelAiPromptStampRequest {
    pub target: crate::coverlay_bevy_ui::VoxelEditTarget,
    pub voxel_size: f32,
    pub cmds_json: String,
    pub palette_json: String,
    pub cells: Vec<IVec3>,
    pub prompt: String,
    pub reference_image: String,
    pub tile_res: i32,
    pub palette_max: i32,
    pub depth_eps: f32,
}

#[derive(Resource, Default)]
struct VoxelAiPromptStampJobs { map: HashMap<String, JobState> }

#[derive(Default)]
struct JobState { inflight: bool, rx: Option<crossbeam_channel::Receiver<JobMsg>> }

#[derive(Clone, Debug)]
enum JobMsg { Done(JobOut), Fail(String) }

#[derive(Clone, Debug)]
struct JobOut {
    target: crate::coverlay_bevy_ui::VoxelEditTarget,
    palette_json: String,
    paint_ops: Vec<(IVec3, u8)>,
}

fn target_key(t: crate::coverlay_bevy_ui::VoxelEditTarget) -> String {
    match t {
        crate::coverlay_bevy_ui::VoxelEditTarget::Direct(id) => format!("direct:{id}"),
        crate::coverlay_bevy_ui::VoxelEditTarget::Cda { inst_id, internal_id } => format!("cda:{inst_id}:{internal_id}"),
    }
}

fn voxel_ai_prompt_stamp_system(
    mut q: ResMut<VoxelAiPromptStampQueue>,
    mut jobs: ResMut<VoxelAiPromptStampJobs>,
    mut ngr: ResMut<crate::nodes::NodeGraphResource>,
    mut graph_changed: MessageWriter<'_, crate::GraphChanged>,
) {
    // Start at most one job per frame (avoid UI hitch).
    if let Some(req) = q.queue.pop() {
        let key = target_key(req.target);
        let st = jobs.map.entry(key.clone()).or_default();
        if !st.inflight {
            st.inflight = true;
            st.rx = Some(spawn_prompt_stamp_job(req));
        }
    }

    // Poll jobs.
    let keys: Vec<String> = jobs.map.keys().cloned().collect();
    for k in keys {
        let Some(st) = jobs.map.get_mut(&k) else { continue; };
        if !st.inflight { continue; }
        let msg = match st.rx.as_ref() {
            Some(rx) => rx.try_recv().ok(),
            None => None,
        };
        let Some(m) = msg else { continue; };
        st.inflight = false;
        st.rx = None;
        match m {
            JobMsg::Fail(_e) => {
                // UI can surface status later; keep minimal side effects.
            }
            JobMsg::Done(o) => {
                let g = &mut ngr.0;
                let mut cmds = crate::coverlay_bevy_ui::read_voxel_cmds(g, o.target);
                for (p, pi) in o.paint_ops {
                    cmds.push(cunning_kernel::algorithms::algorithms_editor::voxel::DiscreteVoxelOp::Paint { x: p.x, y: p.y, z: p.z, palette_index: pi.max(1) });
                }
                if let Some(d) = crate::coverlay_bevy_ui::write_voxel_cmds(g, o.target, cmds) { g.mark_dirty(d); }
                if let Some(d) = crate::coverlay_bevy_ui::write_voxel_palette(g, o.target, o.palette_json) { g.mark_dirty(d); }
                graph_changed.write_default();
            }
        }
    }
}

fn spawn_prompt_stamp_job(req: VoxelAiPromptStampRequest) -> crossbeam_channel::Receiver<JobMsg> {
    use cunning_kernel::algorithms::algorithms_editor::voxel as vox;
    let (tx, rx) = crossbeam_channel::unbounded();
    std::thread::spawn(move || {
        let r = (|| -> Result<JobOut, String> {
            if req.cells.is_empty() { return Err("Empty selection.".to_string()); }
            let voxel_size = req.voxel_size.max(0.001);
            let cmds = vox::DiscreteVoxelCmdList::from_json(&req.cmds_json);
            let pal_json = req.palette_json;
            let mut grid = vox::DiscreteSdfGrid::new(voxel_size);
            if let Ok(p) = serde_json::from_str::<Vec<vox::discrete::PaletteEntry>>(&pal_json) {
                for (i, e) in p.into_iter().enumerate() { if i < grid.palette.len() { grid.palette[i] = e; } }
            }
            let mut bake = vox::DiscreteBakeState { baked_cursor: 0 };
            vox::discrete::bake_cmds_incremental(&mut grid, &cmds, &mut bake);

            // Build mask set.
            let mask: std::collections::HashSet<IVec3> = req.cells.iter().copied().collect();

            // Generate guide+base atlas.
            let tile = req.tile_res.clamp(128, 1024) as u32;
            let (guide_rgba, _cams) = crate::nodes::ai_texture::nano_voxel_painter::guide_atlas_rgba_gpu(&grid, voxel_size, tile, 250_000)
                .map_err(|e| format!("Guide capture: {e}"))?;
            let w = tile * 3;
            let h = tile * 2;
            let guide_img = image::RgbaImage::from_raw(w, h, guide_rgba).ok_or("Guide image buffer")?;
            let guide_dyn = image::DynamicImage::ImageRgba8(guide_img.clone());

            let api_key = crate::nodes::ai_texture::nano_voxel_painter::load_gemini_key();
            if api_key.trim().is_empty() { return Err("Missing Gemini API key (GEMINI_API_KEY or settings/ai/providers.json)".to_string()); }
            let model = crate::nodes::ai_texture::nano_voxel_painter::load_gemini_model_image();
            let mut refs: Vec<crate::nodes::ai_texture::nano_voxel_painter::ImageBlob> = Vec::new();
            let mut guide_png: Vec<u8> = Vec::new();
            {
                use std::io::Write;
                let mut c = std::io::Cursor::new(&mut guide_png);
                guide_dyn
                    .write_to(&mut c, image::ImageFormat::Png)
                    .map_err(|e| format!("Guide PNG encode: {e}"))?;
                let _ = c.flush();
            }
            refs.push(crate::nodes::ai_texture::nano_voxel_painter::ImageBlob { mime: "image/png".to_string(), bytes: guide_png });
            if !req.reference_image.trim().is_empty() {
                // Best-effort: treat as path to file and attach bytes.
                let p = req.reference_image.trim();
                if let Ok(bytes) = std::fs::read(p) {
                    let ext = std::path::Path::new(p).extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
                    let mime = match ext.as_str() {
                        "jpg" | "jpeg" => "image/jpeg",
                        "webp" => "image/webp",
                        "png" => "image/png",
                        _ => "application/octet-stream",
                    }
                    .to_string();
                    refs.push(crate::nodes::ai_texture::nano_voxel_painter::ImageBlob { mime, bytes });
                }
            }
            let base = crate::nodes::ai_texture::nano_voxel_painter::gemini_generate_image(60, &api_key, &model, crate::nodes::ai_texture::nano_voxel_painter::SYS_ATLAS, &req.prompt, &refs, w, h)?;
            let rgba = image::load_from_memory(&base.bytes).map(|i| i.to_rgba8()).map_err(|e| e.to_string())?;
            let rgba = image::imageops::resize(&rgba, w, h, image::imageops::FilterType::Lanczos3);
            let base_dyn = image::DynamicImage::ImageRgba8(rgba);

            // Recolor with mask (selection-driven).
            let new_grid = crate::nodes::ai_texture::nano_voxel_painter::recolor_grid_from_atlas(
                &grid,
                &base_dyn,
                Some(&guide_dyn),
                req.palette_max.clamp(4, 200) as usize,
                Some(&mask),
                req.depth_eps.clamp(0.0, 0.2),
                250_000,
            );

            let mut paint_ops: Vec<(IVec3, u8)> = Vec::new();
            for c in req.cells.iter() {
                let a = grid.get(c.x, c.y, c.z).map(|v| v.palette_index).unwrap_or(0);
                let b = new_grid.get(c.x, c.y, c.z).map(|v| v.palette_index).unwrap_or(0);
                if a != 0 && b != 0 && a != b { paint_ops.push((*c, b)); }
            }
            let palette_json = serde_json::to_string(&new_grid.palette).unwrap_or_else(|_| "[]".to_string());
            Ok(JobOut { target: req.target, palette_json, paint_ops })
        })();
        let _ = tx.send(match r { Ok(o) => JobMsg::Done(o), Err(e) => JobMsg::Fail(e) });
    });
    rx
}

fn voxel_editor_preview_system(
    mut gizmos: Gizmos,
    interaction: Res<ViewportInteractionState>,
    coverlay_wants: Res<CoverlayUiWantsInput>,
    nav: Res<NavigationInput>,
    tool: Res<VoxelToolState>,
    st: Res<StrokeState>,
    hit: Res<VoxelHitResult>,
    sel: Res<VoxelSelection>,
    ov: Res<VoxelOverlaySettings>,
    hud: Res<VoxelHudInfo>,
) {
    puffin::profile_function!();
    if coverlay_wants.0 || nav.active || !interaction.is_hovered { return; }
    let voxel_size = hud.voxel_size.max(0.001);
    let c = hit.hit_cell;
    let n = hit.hit_normal;
    let base = if tool.mode == VoxelToolMode::Add && hit.has_hit { c + n } else { c };
    let col = match tool.mode {
        VoxelToolMode::Add => Color::srgb(1.0, 1.0, 1.0),
        VoxelToolMode::Paint => Color::srgb(0.2, 1.0, 0.2),
        VoxelToolMode::Select => Color::srgb(1.0, 1.0, 0.2),
        VoxelToolMode::Move | VoxelToolMode::Extrude => Color::srgb(0.2, 1.0, 1.0),
    };
    if ov.show_voxel_grid {
        let aabb = bevy::math::bounding::Aabb3d::new(
            Vec3::new((base.x as f32 + 0.5) * voxel_size, (base.y as f32 + 0.5) * voxel_size, (base.z as f32 + 0.5) * voxel_size),
            Vec3::splat(voxel_size * 0.5),
        );
        gizmos.aabb_3d(aabb, Transform::IDENTITY, col);
    }

    if ov.show_volume_grid && hud.has_bounds {
        let step = 4i32.max(1);
        let mn = hud.bounds_min;
        let mx = hud.bounds_max + IVec3::ONE;
        let y = mn.y;
        let col = Color::srgba(0.7, 0.7, 0.7, 0.35);
        for x in (mn.x..=mx.x).step_by(step as usize) {
            let p0 = Vec3::new(x as f32 * voxel_size, y as f32 * voxel_size, mn.z as f32 * voxel_size);
            let p1 = Vec3::new(x as f32 * voxel_size, y as f32 * voxel_size, mx.z as f32 * voxel_size);
            gizmos.line(p0, p1, col);
        }
        for z in (mn.z..=mx.z).step_by(step as usize) {
            let p0 = Vec3::new(mn.x as f32 * voxel_size, y as f32 * voxel_size, z as f32 * voxel_size);
            let p1 = Vec3::new(mx.x as f32 * voxel_size, y as f32 * voxel_size, z as f32 * voxel_size);
            gizmos.line(p0, p1, col);
        }
    }

    if let Some(a) = st.region_start {
        let mn = a.min(c);
        let mx = a.max(c) + IVec3::ONE;
        let cen = Vec3::new((mn.x as f32 + mx.x as f32) * 0.5 * voxel_size, (mn.y as f32 + mx.y as f32) * 0.5 * voxel_size, (mn.z as f32 + mx.z as f32) * 0.5 * voxel_size);
        let half = Vec3::new((mx.x - mn.x) as f32 * 0.5 * voxel_size, (mx.y - mn.y) as f32 * 0.5 * voxel_size, (mx.z - mn.z) as f32 * 0.5 * voxel_size);
        gizmos.aabb_3d(bevy::math::bounding::Aabb3d::new(cen, half), Transform::IDENTITY, Color::srgb(1.0, 0.5, 0.1));
    }

    if ov.show_volume_bound && hud.has_bounds {
        let mn = hud.bounds_min;
        let mx = hud.bounds_max + IVec3::ONE;
        let cen = Vec3::new((mn.x as f32 + mx.x as f32) * 0.5 * voxel_size, (mn.y as f32 + mx.y as f32) * 0.5 * voxel_size, (mn.z as f32 + mx.z as f32) * 0.5 * voxel_size);
        let half = Vec3::new((mx.x - mn.x) as f32 * 0.5 * voxel_size, (mx.y - mn.y) as f32 * 0.5 * voxel_size, (mx.z - mn.z) as f32 * 0.5 * voxel_size);
        gizmos.aabb_3d(bevy::math::bounding::Aabb3d::new(cen, half), Transform::IDENTITY, Color::srgb(0.8, 0.8, 0.8));
    }

    if ov.show_coordinates {
        let o = Vec3::new((c.x as f32 + 0.5) * voxel_size, (c.y as f32 + 0.5) * voxel_size, (c.z as f32 + 0.5) * voxel_size);
        let l = voxel_size * 3.0;
        gizmos.line(o, o + Vec3::X * l, Color::srgb(1.0, 0.2, 0.2));
        gizmos.line(o, o + Vec3::Y * l, Color::srgb(0.2, 1.0, 0.2));
        gizmos.line(o, o + Vec3::Z * l, Color::srgb(0.2, 0.6, 1.0));
    }

    if ov.show_distance {
        if let Some(a) = st.last_cell.or(st.move_anchor).or(st.region_start) {
            let p0 = Vec3::new((a.x as f32 + 0.5) * voxel_size, (a.y as f32 + 0.5) * voxel_size, (a.z as f32 + 0.5) * voxel_size);
            let p1 = Vec3::new((c.x as f32 + 0.5) * voxel_size, (c.y as f32 + 0.5) * voxel_size, (c.z as f32 + 0.5) * voxel_size);
            gizmos.line(p0, p1, Color::srgb(1.0, 1.0, 1.0));
        }
    }

    let mut drawn = 0usize;
    for p in sel.cells.iter() {
        if drawn >= 2048 { break; }
        let aabb = bevy::math::bounding::Aabb3d::new(
            Vec3::new((p.x as f32 + 0.5) * voxel_size, (p.y as f32 + 0.5) * voxel_size, (p.z as f32 + 0.5) * voxel_size),
            Vec3::splat(voxel_size * 0.5),
        );
        gizmos.aabb_3d(aabb, Transform::IDENTITY, Color::srgba(1.0, 1.0, 0.0, 0.6));
        drawn += 1;
    }
}

#[derive(SystemParam)]
struct VoxelEditorParams<'w, 's> {
    mouse: Res<'w, ButtonInput<MouseButton>>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    nav: Res<'w, NavigationInput>,
    viewport_layout: Res<'w, ViewportLayout>,
    windows: Query<'w, 's, (Entity, &'static Window)>,
    cam_q: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<crate::MainCamera>>,
    interaction: Res<'w, ViewportInteractionState>,
    coverlay_wants: Res<'w, CoverlayUiWantsInput>,
    st: ResMut<'w, StrokeState>,
    tool: ResMut<'w, VoxelToolState>,
    sel: ResMut<'w, VoxelSelection>,
    undo_stack: ResMut<'w, VoxelUndoStack>,
    ui_state: Res<'w, UiState>,
    node_graph_res: ResMut<'w, NodeGraphResource>,
    graph_changed: MessageWriter<'w, GraphChanged>,
    hit_res: ResMut<'w, VoxelHitResult>,
    hud: ResMut<'w, VoxelHudInfo>,
    ov: Res<'w, VoxelOverlaySettings>,
}

fn voxel_editor_input_system(
    mut p: VoxelEditorParams,
    mut brush: Local<Option<GpuBrush>>,
    mut brush_async: Local<BrushAsyncState>,
    mut raycast_dda: Local<RaycastDdaState>,
    mut bake_cache: Local<VoxelCmdBakeCache>,
    mut scratch: Local<VoxelStrokeScratch>,
) {
    puffin::profile_function!();
    let mouse = &p.mouse;
    let keys = &p.keys;
    let viewport_layout = &p.viewport_layout;
    let windows = &p.windows;
    let cam_q = &p.cam_q;
    let nav = &p.nav;
    let interaction = &p.interaction;
    let coverlay_wants = &p.coverlay_wants;
    let st = &mut p.st;
    let tool: &mut VoxelToolState = &mut *p.tool;
    let sel = &mut p.sel;
    let undo_stack = &mut p.undo_stack;
    let ui_state = &p.ui_state;
    let node_graph_res = &mut p.node_graph_res;
    let graph_changed = &mut p.graph_changed;
    let hit_res = &mut p.hit_res;
    let hud = &mut p.hud;
    let ov = &p.ov;
    if coverlay_wants.0 || nav.active || !interaction.is_hovered { return; }
    let Ok((camera, camera_tfm)) = cam_q.single() else { return; };
    let cursor = viewport_layout
        .window_entity
        .and_then(|e| windows.get(e).ok().and_then(|(_, w)| w.cursor_position()))
        .unwrap_or(Vec2::new(-99999.0, -99999.0));
    let Ok(ray) = camera.viewport_to_world(camera_tfm, cursor) else { return; };

    // Shortcut keys: 1-Add 2-Select 3-Move 4-Paint 5-Extrude
    if keys.just_pressed(KeyCode::Digit1) { tool.mode = VoxelToolMode::Add; }
    if keys.just_pressed(KeyCode::Digit2) { tool.mode = VoxelToolMode::Select; }
    if keys.just_pressed(KeyCode::Digit3) { tool.mode = VoxelToolMode::Move; }
    if keys.just_pressed(KeyCode::Digit4) { tool.mode = VoxelToolMode::Paint; }
    if keys.just_pressed(KeyCode::Digit5) { tool.mode = VoxelToolMode::Extrude; }
    if keys.just_pressed(KeyCode::BracketLeft) { tool.brush_radius = (tool.brush_radius * 0.9).max(0.01); }
    if keys.just_pressed(KeyCode::BracketRight) { tool.brush_radius = (tool.brush_radius * 1.1).min(1000.0); }

    // Undo/Redo (Z/Y): keep cmdlist as the authoritative history, also mirror changes into undo_stack.
    if keys.just_pressed(KeyCode::KeyZ) || keys.just_pressed(KeyCode::KeyY) {
        let Some(t) = resolve_voxel_edit_target(ui_state, &node_graph_res.0) else { return; };
        let mut c = read_voxel_cmds(&node_graph_res.0, t);
        if keys.just_pressed(KeyCode::KeyZ) { let _ = c.undo(); }
        if keys.just_pressed(KeyCode::KeyY) { let _ = c.redo(); }
        if let Some(dirty) = write_voxel_cmds(&mut node_graph_res.0, t, c) { node_graph_res.0.mark_dirty(dirty); }
        graph_changed.write_default();
        return;
    }

    let target: Option<VoxelEditTarget> = resolve_voxel_edit_target(ui_state, &node_graph_res.0);
    let Some(target) = target else { return; };

    let voxel_size = voxel_size_for_target(&node_graph_res.0, target).max(0.001);
    let palette_index = tool.palette_index.max(1);
    let subtract = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    let (sym_x, sym_y, sym_z) = (tool.sym_x, tool.sym_y, tool.sym_z);
    hud.voxel_size = voxel_size;

    let cmds_snapshot = {
        read_voxel_cmds(&node_graph_res.0, target)
    };
    let (base_node, base_grid) = base_grid_for_target(&node_graph_res.0, target);
    bake_cache.ensure_baked(target, base_node, base_grid, voxel_size, &cmds_snapshot);
    if (ov.show_volume_grid || ov.show_volume_bound) && bake_cache.bounds_dirty { bake_cache.ensure_bounds_exact(); }

    // Hit: CPU DDA raycast against baked discrete grid, fallback to ground plane.
    let (hit, cell, nrm) = {
        let mut hit_world: Option<Vec3> = None;
        let mut hit_cell: Option<IVec3> = None;
        let mut hit_nrm: IVec3 = IVec3::Y;
        let do_raycast = matches!(tool.mode, VoxelToolMode::Add | VoxelToolMode::Paint | VoxelToolMode::Select | VoxelToolMode::Move | VoxelToolMode::Extrude);
        if do_raycast {
            hud.has_bounds = !bake_cache.grid.voxels.is_empty();
            if hud.has_bounds {
                if (ov.show_volume_grid || ov.show_volume_bound) && bake_cache.bounds.is_some() {
                    let (mn, mx) = bake_cache.bounds.unwrap();
                    hud.bounds_min = mn;
                    hud.bounds_max = mx;
                }
            }
            // Non-blocking stepping: amortize long rays across frames.
            raycast_dda.ensure(&bake_cache.grid, ray.origin, *ray.direction, voxel_size, 10000.0);
            if let Some((t, c, n, h)) = raycast_dda.step(&bake_cache.grid, 512) {
                raycast_dda.last = Some((t, c, n, h));
            }
            if let Some((t, c, n, h)) = raycast_dda.last {
                hit_res.has_hit = true;
                hit_res.hit_t = t;
                hit_res.hit_idx = 0;
                hit_world = Some(h);
                hit_cell = Some(c);
                hit_nrm = n;
            } else {
                hit_res.has_hit = false;
                hit_res.hit_t = 0.0;
                hit_res.hit_idx = u32::MAX;
            }
        }
        if let (Some(h), Some(c)) = (hit_world, hit_cell) { (h, c, hit_nrm) }
        else {
            hit_res.has_hit = false;
            hit_res.hit_t = 0.0;
            hit_res.hit_idx = u32::MAX;
            let Some(dist) = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y)) else { return; };
            let h = ray.get_point(dist);
            (h, (h / voxel_size).floor().as_ivec3(), IVec3::Y)
        }
    };
    hit_res.hit_cell = cell;
    hit_res.hit_normal = nrm;
    hud.has_hit = hit_res.has_hit;
    hud.cell = cell;
    hud.normal = nrm;
    hud.distance = st.last_cell.map(|a| (cell - a).as_vec3().length() * voxel_size).unwrap_or(0.0);

    let just_down = mouse.just_pressed(MouseButton::Left);
    let down = mouse.pressed(MouseButton::Left);
    let just_up = mouse.just_released(MouseButton::Left);

    let want_region = alt
        || (tool.mode == VoxelToolMode::Add && matches!(tool.add_type, VoxelAddType::Region))
        || (tool.mode == VoxelToolMode::Select && matches!(tool.select_type, VoxelSelectType::Region | VoxelSelectType::Rect))
        || (tool.mode == VoxelToolMode::Paint && matches!(tool.paint_type, VoxelPaintType::Region));
    let want_line = (tool.mode == VoxelToolMode::Add && matches!(tool.add_type, VoxelAddType::Line))
        || (tool.mode == VoxelToolMode::Select && matches!(tool.select_type, VoxelSelectType::Line))
        || (tool.mode == VoxelToolMode::Paint && matches!(tool.paint_type, VoxelPaintType::Line))
        || keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if just_down {
        st.drawing = true; st.last_cell = None; st.line_start = None;
        st.region_start = if want_region { Some(cell) } else { None };
        st.move_anchor = if matches!(tool.mode, VoxelToolMode::Move | VoxelToolMode::Extrude) { Some(cell) } else { None };
    }
    if just_up {
        st.line_start = None;
        // Region（Alt+Drag）：commit by mode on release
        if let Some(a) = st.region_start.take() {
            let mn = a.min(cell); let mx = a.max(cell);
            if tool.mode == VoxelToolMode::Select && matches!(tool.select_type, VoxelSelectType::Region | VoxelSelectType::Rect) {
                let additive = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
                if !subtract && !additive { sel.cells.clear(); }
                for z in mn.z..=mx.z { for y in mn.y..=mx.y { for x in mn.x..=mx.x {
                    let p = IVec3::new(x, y, z);
                    if subtract { sel.cells.remove(&p); } else { sel.cells.insert(p); }
                }}}
                let mut flat: Vec<i32> = Vec::with_capacity(sel.cells.len() * 3);
                for c in sel.cells.iter() { flat.extend_from_slice(&[c.x, c.y, c.z]); }
                if let Ok(json) = serde_json::to_string(&flat) {
                    if let Some(dirty) = write_voxel_mask(&mut node_graph_res.0, target, json) { node_graph_res.0.mark_dirty(dirty); }
                    graph_changed.write_default();
                }
            } else {
                let mut cmds = read_voxel_cmds(&node_graph_res.0, target);
                if tool.mode == VoxelToolMode::Add {
                    cmds.push(if subtract { DiscreteVoxelOp::BoxRemove { min: mn, max: mx } } else { DiscreteVoxelOp::BoxAdd { min: mn, max: mx, palette_index } });
                } else {
                    let grid = &bake_cache.grid;
                    let mut coords: Vec<IVec3> = Vec::new();
                    let mut before: Vec<u8> = Vec::new();
                    let mut after: Vec<u8> = Vec::new();
                    for z in mn.z..=mx.z { for y in mn.y..=mx.y { for x in mn.x..=mx.x {
                        let p = IVec3::new(x, y, z);
                        let b = grid.get(x, y, z).map(|v| v.palette_index).unwrap_or(0);
                        if tool.mode == VoxelToolMode::Paint && tool.paint_type == VoxelPaintType::Region {
                            if b == 0 || b == palette_index { continue; }
                            coords.push(p); before.push(b); after.push(palette_index);
                            cmds.push(DiscreteVoxelOp::Paint { x, y, z, palette_index });
                            continue;
                        }
                        let a = if subtract { 0 } else { palette_index };
                        if b != a { coords.push(p); before.push(b); after.push(a); cmds.push(if a == 0 { DiscreteVoxelOp::RemoveVoxel { x, y, z } } else { DiscreteVoxelOp::SetVoxel { x, y, z, palette_index: a } }); }
                    }}}
                    if !coords.is_empty() { undo_stack.push(VoxelChanges { min: mn, max: mx, coords, before, after }); }
                }
                if let Some(dirty) = write_voxel_cmds(&mut node_graph_res.0, target, cmds) { node_graph_res.0.mark_dirty(dirty); }
                graph_changed.write_default();
            }
        }
        // Move: commit MoveSelected on release
        if let Some(anchor) = st.move_anchor.take() {
            if tool.mode == VoxelToolMode::Move && !sel.cells.is_empty() {
                let delta = cell - anchor;
                if delta != IVec3::ZERO {
                    let cells_vec: Vec<IVec3> = sel.cells.iter().copied().collect();
                    let op = DiscreteVoxelOp::MoveSelected { cells: cells_vec.clone(), delta };
                    let mut cmds = read_voxel_cmds(&node_graph_res.0, target);
                    cmds.push(op);
                    if let Some(dirty) = write_voxel_cmds(&mut node_graph_res.0, target, cmds) { node_graph_res.0.mark_dirty(dirty); }
                    // Update selection to new positions
                    sel.cells = cells_vec.iter().map(|c| *c + delta).collect();
                    graph_changed.write_default();
                }
            }
            // Extrude: commit Extrude on release (copies selection, leaving originals)
            if tool.mode == VoxelToolMode::Extrude && !sel.cells.is_empty() {
                let delta = cell - anchor;
                if delta != IVec3::ZERO {
                    let cells_vec: Vec<IVec3> = sel.cells.iter().copied().collect();
                    let op = DiscreteVoxelOp::Extrude { cells: cells_vec.clone(), delta, palette_index };
                    let mut cmds = read_voxel_cmds(&node_graph_res.0, target);
                    cmds.push(op);
                    if let Some(dirty) = write_voxel_cmds(&mut node_graph_res.0, target, cmds) { node_graph_res.0.mark_dirty(dirty); }
                    // Selection follows the extruded part
                    sel.cells = cells_vec.iter().map(|c| *c + delta).collect();
                    graph_changed.write_default();
                }
            }
        }
        st.drawing = false; st.last_cell = None;
        return;
    }
    if !down || !st.drawing { return; }
    if want_region { return; }
    if st.last_cell == Some(cell) { return; }

    // Line
    scratch.cells.clear();
    if want_line {
        if let Some(prev) = st.last_cell {
            scratch.cells.extend(bresenham_3d(prev, cell));
        } else {
            scratch.cells.push(cell);
        }
    } else {
        scratch.cells.push(cell);
    }
    st.last_cell = Some(cell);

    let mut cmds = read_voxel_cmds(&node_graph_res.0, target);
    let grid = &bake_cache.grid;
    let rt = GpuRuntime::get_blocking();
    let b = brush.get_or_insert_with(|| GpuBrush::new(rt));
    let max_out = 65536u32;
    let brush_offset = if tool.mode == VoxelToolMode::Add
        && hit_res.has_hit
        && !subtract
        && matches!(tool.add_type, VoxelAddType::Extrude | VoxelAddType::Point | VoxelAddType::Line | VoxelAddType::Region | VoxelAddType::Clay | VoxelAddType::Smooth | VoxelAddType::Clone)
    { nrm } else { IVec3::ZERO };

    // Poll async brush results (non-blocking); apply with captured stamp.
    while let Some(r) = b.poll() { brush_async.ready.push(r); }
    if !brush_async.ready.is_empty() {
        // Group by target without HashMap (VoxelEditTarget may not be Hash).
        let mut per_target: Vec<(VoxelEditTarget, Vec<(gpu_brush::BrushAsyncResult, BrushStamp)>)> = Vec::new();
        let ready = std::mem::take(&mut brush_async.ready);
        for r in ready {
            let Some(stamp) = brush_async.pending.remove(&r.seq) else { continue; };
            if let Some((_, v)) = per_target.iter_mut().find(|(t, _)| *t == stamp.target) {
                v.push((r, stamp));
            } else {
                per_target.push((stamp.target, vec![(r, stamp)]));
            }
        }
        for (tgt, rs) in per_target {
            let mut cmds2 = read_voxel_cmds(&node_graph_res.0, tgt);
            for (r, stamp) in rs {
                scratch.coords.clear();
                scratch.before.clear();
                scratch.after.clear();
                for c in r.cells.iter() {
                    for sp in sym_points(IVec3::new(c.x, c.y, c.z), stamp.sym_x, stamp.sym_y, stamp.sym_z) {
                        let q = sp + stamp.brush_offset;
                        let (x, y, z) = (q.x, q.y, q.z);
                        let b0 = grid.get(x, y, z).map(|v| v.palette_index).unwrap_or(0);
                        let a0 = if stamp.subtract { 0 } else { stamp.palette_index };
                        if b0 == a0 { continue; }
                        scratch.coords.push(q);
                        scratch.before.push(b0);
                        scratch.after.push(a0);
                        cmds2.push(if a0 == 0 {
                            DiscreteVoxelOp::RemoveVoxel { x, y, z }
                        } else {
                            DiscreteVoxelOp::SetVoxel { x, y, z, palette_index: a0 }
                        });
                    }
                }
                if !scratch.coords.is_empty() {
                    let (mut mn, mut mx) = (IVec3::splat(i32::MAX), IVec3::splat(i32::MIN));
                    for c in scratch.coords.iter() { mn = mn.min(*c); mx = mx.max(*c); }
                    undo_stack.push(VoxelChanges {
                        min: mn,
                        max: mx,
                        coords: std::mem::take(&mut scratch.coords),
                        before: std::mem::take(&mut scratch.before),
                        after: std::mem::take(&mut scratch.after),
                    });
                }
            }
            if let Some(dirty) = write_voxel_cmds(&mut node_graph_res.0, tgt, cmds2) { node_graph_res.0.mark_dirty(dirty); }
            graph_changed.write_default();
        }
    }

    let n_cells = scratch.cells.len();
    for i in 0..n_cells {
        let p = scratch.cells[i];
        match tool.mode {
            VoxelToolMode::Add => {
                let rad = tool.brush_radius.max(voxel_size * 0.5);
                if tool.add_type != VoxelAddType::Smooth && (tool.shape == VoxelBrushShape::Sphere || tool.shape == VoxelBrushShape::Cube) {
                    let base = p + brush_offset;
                    if tool.shape == VoxelBrushShape::Sphere {
                        let w = (Vec3::new(base.x as f32 + 0.5, base.y as f32 + 0.5, base.z as f32 + 0.5) * voxel_size);
                        for sp in sym_points(base, sym_x, sym_y, sym_z).into_iter() {
                            let c = (Vec3::new(sp.x as f32 + 0.5, sp.y as f32 + 0.5, sp.z as f32 + 0.5) * voxel_size);
                            cmds.push(if subtract { DiscreteVoxelOp::SphereRemove { center: c, radius: rad } } else { DiscreteVoxelOp::SphereAdd { center: c, radius: rad, palette_index } });
                        }
                    } else {
                        let r = (rad / voxel_size).ceil() as i32;
                        for sp in sym_points(base, sym_x, sym_y, sym_z).into_iter() {
                            let mn = sp - IVec3::splat(r);
                            let mx = sp + IVec3::splat(r);
                            cmds.push(if subtract { DiscreteVoxelOp::BoxRemove { min: mn, max: mx } } else { DiscreteVoxelOp::BoxAdd { min: mn, max: mx, palette_index } });
                        }
                    }
                    continue;
                }
                // Fallback: GPU brush emits per-voxel ops (used by Smooth/advanced shapes).
                let center = (Vec3::new(p.x as f32 + 0.5, p.y as f32 + 0.5, p.z as f32 + 0.5) * voxel_size).to_array();
                // GPU-first async (no stall): request and return; results applied when ready.
                if tool.shape == VoxelBrushShape::Sphere {
                    let seq = b.gen_sphere_cells_async(rt, center, rad, voxel_size, max_out);
                    brush_async.pending.insert(seq, BrushStamp {
                        target,
                        subtract,
                        palette_index,
                        sym_x,
                        sym_y,
                        sym_z,
                        brush_offset,
                        voxel_size,
                    });
                    continue;
                }
                let out = match tool.shape {
                    VoxelBrushShape::Cube => b.gen_box_cells(rt, center, [rad; 3], voxel_size, max_out),
                    _ => b.gen_sphere_cells(rt, center, rad, voxel_size, max_out),
                };
                scratch.coords.clear();
                scratch.before.clear();
                scratch.after.clear();
                for c in out.iter() {
                    let base = IVec3::new(c.x, c.y, c.z);
                    for sp in sym_points(base, sym_x, sym_y, sym_z).into_iter() {
                        let q = sp + brush_offset;
                        let (x, y, z) = (q.x, q.y, q.z);
                        if tool.add_type == VoxelAddType::Smooth {
                            continue;
                        }
                        let b0 = grid.get(x, y, z).map(|v| v.palette_index).unwrap_or(0);
                        let a0 = if subtract { 0 } else { palette_index };
                        if b0 == a0 { continue; }
                        scratch.coords.push(q); scratch.before.push(b0); scratch.after.push(a0);
                        cmds.push(if a0 == 0 { DiscreteVoxelOp::RemoveVoxel { x, y, z } } else { DiscreteVoxelOp::SetVoxel { x, y, z, palette_index: a0 } });
                    }
                }
                if tool.add_type == VoxelAddType::Smooth {
                    scratch.coords2.clear();
                    scratch.before2.clear();
                    scratch.after2.clear();
                    for c in out.iter() {
                        for sp in sym_points(IVec3::new(c.x, c.y, c.z), sym_x, sym_y, sym_z).into_iter() {
                            let q = sp + brush_offset;
                            let (x, y, z) = (q.x, q.y, q.z);
                            let cur = grid.get(x, y, z).map(|v| v.palette_index).unwrap_or(0);
                            let mut solid_n = 0u8;
                            for d in [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z] {
                                if grid.is_solid(x + d.x, y + d.y, z + d.z) { solid_n = solid_n.saturating_add(1); }
                            }
                            let next = if cur != 0 {
                                if solid_n <= 1 { 0 } else { cur }
                            } else {
                                if solid_n >= 5 { palette_index } else { 0 }
                            };
                            if next == cur { continue; }
                            scratch.coords2.push(q); scratch.before2.push(cur); scratch.after2.push(next);
                            cmds.push(if next == 0 { DiscreteVoxelOp::RemoveVoxel { x, y, z } } else { DiscreteVoxelOp::SetVoxel { x, y, z, palette_index: next } });
                        }
                    }
                    if !scratch.coords2.is_empty() {
                        let (mut mn, mut mx) = (IVec3::splat(i32::MAX), IVec3::splat(i32::MIN));
                        for c in scratch.coords2.iter() { mn = mn.min(*c); mx = mx.max(*c); }
                        undo_stack.push(VoxelChanges { min: mn, max: mx, coords: std::mem::take(&mut scratch.coords2), before: std::mem::take(&mut scratch.before2), after: std::mem::take(&mut scratch.after2) });
                    }
                }
                if !scratch.coords.is_empty() {
                    let (mut mn, mut mx) = (IVec3::splat(i32::MAX), IVec3::splat(i32::MIN));
                    for c in scratch.coords.iter() { mn = mn.min(*c); mx = mx.max(*c); }
                    undo_stack.push(VoxelChanges { min: mn, max: mx, coords: std::mem::take(&mut scratch.coords), before: std::mem::take(&mut scratch.before), after: std::mem::take(&mut scratch.after) });
                }
            }
            VoxelToolMode::Paint => {
                // ColorPick mode: pick color from clicked voxel
                if tool.paint_type == VoxelPaintType::ColorPick && hit_res.has_hit {
                    if let Some(v) = grid.get(cell.x, cell.y, cell.z) { tool.palette_index = v.palette_index; }
                    continue;
                }

                // All Paint modes: use IsPainting=1 logic (paint existing voxels only, don't create)
                match tool.paint_type {
                    // Point mode: Voxy-style with BrushSize + Spacing + ShapeType, IsPainting=1
                    VoxelPaintType::Point => {
                        if hit_res.has_hit {
                            // Voxy: Check spacing to control point density
                            let should_paint = if let Some(last_pos) = st.last_cell {
                                let dist = (cell - last_pos).as_vec3().length() * voxel_size;
                                dist >= tool.spacing.max(voxel_size * 0.5) // Voxy GetSpacing()
                            } else {
                                true
                            };

                            if should_paint {
                                scratch.coords.clear();
                                scratch.before.clear();
                                scratch.after.clear();
                                scratch.seen.clear();

                                // Voxy: Use Brush with BrushSize and ShapeType
                                if tool.brush_radius > voxel_size * 0.5 {
                                    // Generate brush cells based on shape
                                    let brush_cells = match tool.shape {
                                        VoxelBrushShape::Sphere => {
                                            let mut cells = Vec::new();
                                            let r = (tool.brush_radius / voxel_size).ceil() as i32;
                                            let r2 = (tool.brush_radius / voxel_size) * (tool.brush_radius / voxel_size);
                                            for dz in -r..=r {
                                                for dy in -r..=r {
                                                    for dx in -r..=r {
                                                        let d2 = (dx*dx + dy*dy + dz*dz) as f32;
                                                        if d2 <= r2 {
                                                            cells.push((dx, dy, dz));
                                                        }
                                                    }
                                                }
                                            }
                                            cells
                                        },
                                        VoxelBrushShape::Cube => {
                                            let mut cells = Vec::new();
                                            let r = (tool.brush_radius / voxel_size).ceil() as i32;
                                            for dz in -r..=r {
                                                for dy in -r..=r {
                                                    for dx in -r..=r {
                                                        cells.push((dx, dy, dz));
                                                    }
                                                }
                                            }
                                            cells
                                        },
                                        _ => {
                                            // Default to sphere for other shapes
                                            let mut cells = Vec::new();
                                            let r = (tool.brush_radius / voxel_size).ceil() as i32;
                                            let r2 = (tool.brush_radius / voxel_size) * (tool.brush_radius / voxel_size);
                                            for dz in -r..=r {
                                                for dy in -r..=r {
                                                    for dx in -r..=r {
                                                        let d2 = (dx*dx + dy*dy + dz*dz) as f32;
                                                        if d2 <= r2 {
                                                            cells.push((dx, dy, dz));
                                                        }
                                                    }
                                                }
                                            }
                                            cells
                                        }
                                    };

                                    // Process brush cells with symmetry
                                    for (dx, dy, dz) in brush_cells {
                                        let p = IVec3::new(
                                            cell.x + dx,
                                            cell.y + dy,
                                            cell.z + dz
                                        );

                                        for sym_p in sym_points(p, sym_x, sym_y, sym_z) {
                                            if !scratch.seen.insert(sym_p) { continue; }

                                            let b0 = grid.get(sym_p.x, sym_p.y, sym_p.z)
                                                .map(|v| v.palette_index)
                                                .unwrap_or(0);

                                            // Voxy: IsPainting=1 - only paint existing voxels, don't create
                                            if b0 != 0 && b0 != palette_index {
                                                scratch.coords.push(sym_p);
                                                scratch.before.push(b0);
                                                scratch.after.push(palette_index);
                                                cmds.push(DiscreteVoxelOp::Paint {
                                                    x: sym_p.x,
                                                    y: sym_p.y,
                                                    z: sym_p.z,
                                                    palette_index,
                                                });
                                            }
                                        }
                                    }
                                } else {
                                    // BrushSize very small ≈ single voxel
                                    let b0 = grid.get(cell.x, cell.y, cell.z)
                                        .map(|v| v.palette_index)
                                        .unwrap_or(0);

                                    if b0 != 0 && b0 != palette_index {
                                        for p in sym_points(cell, sym_x, sym_y, sym_z) {
                                            let b0_sym = grid.get(p.x, p.y, p.z)
                                                .map(|v| v.palette_index)
                                                .unwrap_or(0);

                                            if b0_sym != 0 && b0_sym != palette_index {
                                                scratch.coords.push(p);
                                                scratch.before.push(b0_sym);
                                                scratch.after.push(palette_index);
                                                cmds.push(DiscreteVoxelOp::Paint {
                                                    x: p.x, y: p.y, z: p.z,
                                                    palette_index,
                                                });
                                            }
                                        }
                                    }
                                }

                                if !scratch.coords.is_empty() {
                                    let (mut mn, mut mx) = (IVec3::splat(i32::MAX), IVec3::splat(i32::MIN));
                                    for c in scratch.coords.iter() { mn = mn.min(*c); mx = mx.max(*c); }
                                    undo_stack.push(VoxelChanges { min: mn, max: mx, coords: std::mem::take(&mut scratch.coords), before: std::mem::take(&mut scratch.before), after: std::mem::take(&mut scratch.after) });
                                }

                                // Voxy: update last_cell for spacing calculations
                                st.last_cell = Some(cell);
                            }
                        }
                    }

                    // Line mode: Voxy-style with Bresenham + BrushSize + ShapeType, IsPainting=1
                    VoxelPaintType::Line => {
                        if hit_res.has_hit {
                            // Voxy: Record line start on first point
                            if st.line_start.is_none() {
                                st.line_start = Some(cell);
                            }

                            let start = st.line_start.unwrap();
                            let end = cell;

                            // Voxy: Use Bresenham algorithm to generate line points
                            let line_points = bresenham_3d(start, end);

                            scratch.coords.clear();
                            scratch.before.clear();
                            scratch.after.clear();
                            scratch.seen.clear();

                            // Voxy: For each point on the line, optionally apply Brush
                            for line_point in line_points {
                                // Voxy: Use Brush if BrushSize > voxel_size
                                if tool.brush_radius > voxel_size {
                                    let brush_offset = line_point.as_vec3() * voxel_size;
                                    let brush_cells = match tool.shape {
                                        VoxelBrushShape::Sphere => {
                                            let mut cells = Vec::new();
                                            let r = (tool.brush_radius / voxel_size).ceil() as i32;
                                            let r2 = (tool.brush_radius / voxel_size).powi(2);
                                            for dz in -r..=r {
                                                for dy in -r..=r {
                                                    for dx in -r..=r {
                                                        if (dx*dx + dy*dy + dz*dz) as f32 <= r2 {
                                                            cells.push((dx, dy, dz));
                                                        }
                                                    }
                                                }
                                            }
                                            cells
                                        },
                                        VoxelBrushShape::Cube => {
                                            let mut cells = Vec::new();
                                            let r = (tool.brush_radius / voxel_size).ceil() as i32;
                                            for dz in -r..=r {
                                                for dy in -r..=r {
                                                    for dx in -r..=r {
                                                        cells.push((dx, dy, dz));
                                                    }
                                                }
                                            }
                                            cells
                                        },
                                        _ => {
                                            // Default to sphere
                                            let mut cells = Vec::new();
                                            let r = (tool.brush_radius / voxel_size).ceil() as i32;
                                            let r2 = (tool.brush_radius / voxel_size).powi(2);
                                            for dz in -r..=r {
                                                for dy in -r..=r {
                                                    for dx in -r..=r {
                                                        if (dx*dx + dy*dy + dz*dz) as f32 <= r2 {
                                                            cells.push((dx, dy, dz));
                                                        }
                                                    }
                                                }
                                            }
                                            cells
                                        }
                                    };

                                    // Process brush cells with symmetry (Voxy-style)
                                    for (dx, dy, dz) in brush_cells {
                                        let p = IVec3::new(
                                            line_point.x + dx,
                                            line_point.y + dy,
                                            line_point.z + dz
                                        );

                                        for sym_p in sym_points(p, sym_x, sym_y, sym_z) {
                                            if !scratch.seen.insert(sym_p) { continue; }

                                            let b0 = grid.get(sym_p.x, sym_p.y, sym_p.z)
                                                .map(|v| v.palette_index)
                                                .unwrap_or(0);

                                            // Voxy: IsPainting=1 - only paint existing voxels
                                            if b0 != 0 && b0 != palette_index {
                                                scratch.coords.push(sym_p);
                                                scratch.before.push(b0);
                                                scratch.after.push(palette_index);
                                                cmds.push(DiscreteVoxelOp::Paint {
                                                    x: sym_p.x, y: sym_p.y, z: sym_p.z,
                                                    palette_index,
                                                });
                                            }
                                        }
                                    }
                                } else {
                                    // BrushSize small ≈ single voxel
                                    let b0 = grid.get(line_point.x, line_point.y, line_point.z)
                                        .map(|v| v.palette_index)
                                        .unwrap_or(0);

                                    if b0 != 0 && b0 != palette_index {
                                        for p in sym_points(line_point, sym_x, sym_y, sym_z) {
                                            let b0_sym = grid.get(p.x, p.y, p.z)
                                                .map(|v| v.palette_index)
                                                .unwrap_or(0);

                                            if b0_sym != 0 && b0_sym != palette_index {
                                                scratch.coords.push(p);
                                                scratch.before.push(b0_sym);
                                                scratch.after.push(palette_index);
                                                cmds.push(DiscreteVoxelOp::Paint {
                                                    x: p.x, y: p.y, z: p.z,
                                                    palette_index,
                                                });
                                            }
                                        }
                                    }
                                }
                            }

                            if !scratch.coords.is_empty() {
                                let (mut mn, mut mx) = (IVec3::splat(i32::MAX), IVec3::splat(i32::MIN));
                                for c in scratch.coords.iter() { mn = mn.min(*c); mx = mx.max(*c); }
                                undo_stack.push(VoxelChanges { min: mn, max: mx, coords: std::mem::take(&mut scratch.coords), before: std::mem::take(&mut scratch.before), after: std::mem::take(&mut scratch.after) });
                            }

                            // Voxy: Update last_cell for spacing calculation
                            st.last_cell = Some(cell);
                        }
                    }

                    // Region mode: drag-to-select region (like Select Region but with Paint)
                    VoxelPaintType::Region => {
                        // Region mode is handled on mouse release (like Voxy's PaintRegionTool)
                        // During drag, we show preview
                        // On release, we paint the entire region

                        if just_down {
                            st.region_start = Some(cell);
                        } else if down && st.region_start.is_some() {
                            // Show preview during drag (this is handled by the UI preview system)
                            // The actual painting happens on mouse up
                        } else if just_up {
                            if let Some(start) = st.region_start.take() {
                                let end = cell;
                                let min = IVec3::new(
                                    start.x.min(end.x),
                                    start.y.min(end.y),
                                    start.z.min(end.z),
                                );
                                let max = IVec3::new(
                                    start.x.max(end.x),
                                    start.y.max(end.y),
                                    start.z.max(end.z),
                                );

                                // Paint all voxels in the region
                                scratch.coords.clear();
                                scratch.before.clear();
                                scratch.after.clear();

                                for x in min.x..=max.x {
                                    for y in min.y..=max.y {
                                        for z in min.z..=max.z {
                                            let pos = IVec3::new(x, y, z);

                                            // Apply symmetry
                                            for sym_pos in sym_points(pos, sym_x, sym_y, sym_z) {
                                                let b0 = grid.get(sym_pos.x, sym_pos.y, sym_pos.z)
                                                    .map(|v| v.palette_index)
                                                    .unwrap_or(0);

                                                // Only paint existing voxels
                                                if b0 != 0 && b0 != palette_index {
                                                    scratch.coords.push(sym_pos);
                                                    scratch.before.push(b0);
                                                    scratch.after.push(palette_index);
                                                    cmds.push(DiscreteVoxelOp::Paint {
                                                        x: sym_pos.x,
                                                        y: sym_pos.y,
                                                        z: sym_pos.z,
                                                        palette_index,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }

                                if !scratch.coords.is_empty() {
                                    let (mut mn, mut mx) = (IVec3::splat(i32::MAX), IVec3::splat(i32::MIN));
                                    for c in scratch.coords.iter() { mn = mn.min(*c); mx = mx.max(*c); }
                                    undo_stack.push(VoxelChanges { min: mn, max: mx, coords: std::mem::take(&mut scratch.coords), before: std::mem::take(&mut scratch.before), after: std::mem::take(&mut scratch.after) });
                                }
                            }
                        }
                    }

                    // ColorPick handled above; keep arm for exhaustiveness
                    VoxelPaintType::ColorPick => {}

                    // AI Stamp is Coverlay-driven (button + selection); ignore direct stroke paint.
                    VoxelPaintType::PromptStamp => {}

                    // Face mode: select entire face based on normal direction
                    VoxelPaintType::Face => {
                        if hit_res.has_hit {
                            // Find all connected voxels on the same face (same normal)
                            let start = cell;
                            let normal = nrm;

                            // Determine which axis the normal points to
                            let axis = if normal.x.abs() > normal.y.abs() && normal.x.abs() > normal.z.abs() {
                                0 // X axis
                            } else if normal.y.abs() > normal.z.abs() {
                                1 // Y axis
                            } else {
                                2 // Z axis
                            };

                            // Flood fill to find all connected voxels on the same face
                            let mut to_process = vec![start];
                            let mut processed = HashSet::new();
                            let mut face_voxels = Vec::new();

                            while let Some(current) = to_process.pop() {
                                if !processed.insert(current) { continue; }

                                // Check if this voxel exists
                                if let Some(voxel) = grid.get(current.x, current.y, current.z) {
                                    face_voxels.push((current, voxel.palette_index));

                                    // Check neighbors on the same face plane
                                    let neighbors = match axis {
                                        0 => vec![ // X axis, check YZ plane neighbors
                                            IVec3::new(current.x, current.y + 1, current.z),
                                            IVec3::new(current.x, current.y - 1, current.z),
                                            IVec3::new(current.x, current.y, current.z + 1),
                                            IVec3::new(current.x, current.y, current.z - 1),
                                        ],
                                        1 => vec![ // Y axis, check XZ plane neighbors
                                            IVec3::new(current.x + 1, current.y, current.z),
                                            IVec3::new(current.x - 1, current.y, current.z),
                                            IVec3::new(current.x, current.y, current.z + 1),
                                            IVec3::new(current.x, current.y, current.z - 1),
                                        ],
                                        _ => vec![ // Z axis, check XY plane neighbors
                                            IVec3::new(current.x + 1, current.y, current.z),
                                            IVec3::new(current.x - 1, current.y, current.z),
                                            IVec3::new(current.x, current.y + 1, current.z),
                                            IVec3::new(current.x, current.y - 1, current.z),
                                        ],
                                    };

                                    for neighbor in neighbors {
                                        if !processed.contains(&neighbor) {
                                            to_process.push(neighbor);
                                        }
                                    }
                                }
                            }

                            // Paint all voxels on the face
                            let mut coords: Vec<IVec3> = Vec::new();
                            let mut before: Vec<u8> = Vec::new();
                            let mut after: Vec<u8> = Vec::new();

                            for (pos, b0) in face_voxels {
                                // Apply symmetry
                                for sym_pos in sym_points(pos, sym_x, sym_y, sym_z) {
                                    let b0_sym = grid.get(sym_pos.x, sym_pos.y, sym_pos.z)
                                        .map(|v| v.palette_index)
                                        .unwrap_or(0);

                                    if b0_sym != 0 && b0_sym != palette_index {
                                        coords.push(sym_pos);
                                        before.push(b0_sym);
                                        after.push(palette_index);
                                        cmds.push(DiscreteVoxelOp::Paint {
                                            x: sym_pos.x,
                                            y: sym_pos.y,
                                            z: sym_pos.z,
                                            palette_index,
                                        });
                                    }
                                }
                            }

                            if !coords.is_empty() {
                                let (mut mn, mut mx) = (IVec3::splat(i32::MAX), IVec3::splat(i32::MIN));
                                for c in coords.iter() { mn = mn.min(*c); mx = mx.max(*c); }
                                undo_stack.push(VoxelChanges { min: mn, max: mx, coords, before, after });
                            }
                        }
                    }
                }
            }
            VoxelToolMode::Select => {
                if tool.select_type == VoxelSelectType::Color && hit_res.has_hit {
                    let Some(v) = grid.get(cell.x, cell.y, cell.z) else { continue; };
                    let pi = v.palette_index;
                    if !subtract { sel.cells.clear(); }
                    for (c, vv) in grid.voxels.iter() { if vv.palette_index == pi { if subtract { sel.cells.remove(&c.0); } else { sel.cells.insert(c.0); } } }
                    let mut flat: Vec<i32> = Vec::with_capacity(sel.cells.len() * 3);
                    for c in sel.cells.iter() { flat.extend_from_slice(&[c.x, c.y, c.z]); }
                    if let Ok(json) = serde_json::to_string(&flat) {
                        if let Some(dirty) = write_voxel_mask(&mut node_graph_res.0, target, json) { node_graph_res.0.mark_dirty(dirty); }
                        graph_changed.write_default();
                    }
                    continue;
                }
                let out = match tool.shape {
                    VoxelBrushShape::Sphere => b.gen_sphere_cells(rt, hit.to_array(), tool.brush_radius.max(voxel_size * 0.5), voxel_size, max_out),
                    VoxelBrushShape::Cube => b.gen_box_cells(rt, hit.to_array(), [tool.brush_radius; 3], voxel_size, max_out),
                    _ => b.gen_sphere_cells(rt, hit.to_array(), tool.brush_radius.max(voxel_size * 0.5), voxel_size, max_out),
                };
                for c in out.iter() {
                    let iv0 = IVec3::new(c.x, c.y, c.z);
                    if tool.select_type == VoxelSelectType::Face && hit_res.has_hit {
                        let axis = if nrm.x != 0 { 0 } else if nrm.y != 0 { 1 } else { 2 };
                        for iv in sym_points(iv0, sym_x, sym_y, sym_z).into_iter().filter(|p| match axis { 0 => p.x == cell.x, 1 => p.y == cell.y, _ => p.z == cell.z }) {
                            if subtract { sel.cells.remove(&iv); } else { sel.cells.insert(iv); }
                        }
                    } else {
                        for iv in sym_points(iv0, sym_x, sym_y, sym_z) {
                            if subtract { sel.cells.remove(&iv); } else { sel.cells.insert(iv); }
                        }
                    }
                }
                let mut flat: Vec<i32> = Vec::with_capacity(sel.cells.len() * 3);
                for c in sel.cells.iter() { flat.extend_from_slice(&[c.x, c.y, c.z]); }
                if let Ok(json) = serde_json::to_string(&flat) {
                    if let Some(dirty) = write_voxel_mask(&mut node_graph_res.0, target, json) { node_graph_res.0.mark_dirty(dirty); }
                    graph_changed.write_default();
                }
            }
            VoxelToolMode::Move | VoxelToolMode::Extrude => {
                // Move/Extrude are handled on mouse release (above)
            }
        }
    }
    // Commit only when data actually changed
    if let Some(dirty) = write_voxel_cmds(&mut node_graph_res.0, target, cmds) {
        node_graph_res.0.mark_dirty(dirty);
        graph_changed.write_default();
    }
}

#[derive(Default)]
struct VoxelStrokeScratch {
    cells: Vec<IVec3>,
    coords: Vec<IVec3>,
    before: Vec<u8>,
    after: Vec<u8>,
    coords2: Vec<IVec3>,
    before2: Vec<u8>,
    after2: Vec<u8>,
    seen: HashSet<IVec3>,
}

// Simple 3D Bresenham, returns discrete grid points sequence including endpoints
fn bresenham_3d(a: IVec3, b: IVec3) -> Vec<IVec3> {
    let (mut x1, mut y1, mut z1) = (a.x, a.y, a.z);
    let (x2, y2, z2) = (b.x, b.y, b.z);
    let (dx, dy, dz) = ((x2 - x1).abs(), (y2 - y1).abs(), (z2 - z1).abs());
    let (xs, ys, zs) = ((x2 - x1).signum(), (y2 - y1).signum(), (z2 - z1).signum());
    let mut out: Vec<IVec3> = Vec::new();
    out.push(IVec3::new(x1, y1, z1));
    if dx >= dy && dx >= dz {
        let mut p1 = 2 * dy - dx;
        let mut p2 = 2 * dz - dx;
        while x1 != x2 {
            x1 += xs;
            if p1 >= 0 { y1 += ys; p1 -= 2 * dx; }
            if p2 >= 0 { z1 += zs; p2 -= 2 * dx; }
            p1 += 2 * dy;
            p2 += 2 * dz;
            out.push(IVec3::new(x1, y1, z1));
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
            out.push(IVec3::new(x1, y1, z1));
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
            out.push(IVec3::new(x1, y1, z1));
        }
    }
    out
}

#[inline]
fn sym_points(p: IVec3, sx: bool, sy: bool, sz: bool) -> SymPoints { SymPoints::new(p, sx, sy, sz) }

#[derive(Clone, Copy)]
struct SymPoints { pts: [IVec3; 8], len: u8 }

impl SymPoints {
    #[inline]
    fn new(p: IVec3, sx: bool, sy: bool, sz: bool) -> Self {
        let xs = if sx { [p.x, -p.x - 1] } else { [p.x, p.x] };
        let ys = if sy { [p.y, -p.y - 1] } else { [p.y, p.y] };
        let zs = if sz { [p.z, -p.z - 1] } else { [p.z, p.z] };
        let mut pts = [IVec3::ZERO; 8];
        let mut len = 0u8;
        for &x in xs.iter() { for &y in ys.iter() { for &z in zs.iter() {
            let q = IVec3::new(x, y, z);
            let mut dup = false;
            for i in 0..len as usize { if pts[i] == q { dup = true; break; } }
            if !dup { pts[len as usize] = q; len += 1; }
        }}}
        Self { pts, len }
    }
}

struct SymPointsIter { pts: [IVec3; 8], idx: u8, len: u8 }
impl Iterator for SymPointsIter {
    type Item = IVec3;
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.len { return None; }
        let i = self.idx as usize;
        self.idx += 1;
        Some(self.pts[i])
    }
}
impl IntoIterator for SymPoints {
    type Item = IVec3;
    type IntoIter = SymPointsIter;
    #[inline]
    fn into_iter(self) -> Self::IntoIter { SymPointsIter { pts: self.pts, idx: 0, len: self.len } }
}

#[inline]
fn aabb_face_normal(hit: Vec3, bmin: Vec3, bmax: Vec3) -> IVec3 {
    let e = 1.0e-4;
    let dx0 = (hit.x - bmin.x).abs();
    let dx1 = (bmax.x - hit.x).abs();
    let dy0 = (hit.y - bmin.y).abs();
    let dy1 = (bmax.y - hit.y).abs();
    let dz0 = (hit.z - bmin.z).abs();
    let dz1 = (bmax.z - hit.z).abs();
    let mut best = dx0;
    let mut n = IVec3::NEG_X;
    if dx1 < best { best = dx1; n = IVec3::X; }
    if dy0 < best { best = dy0; n = IVec3::NEG_Y; }
    if dy1 < best { best = dy1; n = IVec3::Y; }
    if dz0 < best { best = dz0; n = IVec3::NEG_Z; }
    if dz1 < best { best = dz1; n = IVec3::Z; }
    if best > e { n } else { n }
}

#[inline]
fn raycast_discrete_dda(grid: &vox::DiscreteSdfGrid, origin: Vec3, dir: Vec3, voxel_size: f32, max_dist_world: f32) -> Option<(f32, IVec3, IVec3, Vec3)> {
    if grid.voxels.is_empty() { return None; }
    let vs = voxel_size.max(0.001);
    let d = dir.normalize_or_zero();
    if d.length_squared() <= 1.0e-12 { return None; }
    let o = origin / vs;
    let mut cell = o.floor().as_ivec3();
    let step = IVec3::new(if d.x >= 0.0 { 1 } else { -1 }, if d.y >= 0.0 { 1 } else { -1 }, if d.z >= 0.0 { 1 } else { -1 });
    let inv = Vec3::new(if d.x.abs() < 1.0e-12 { f32::INFINITY } else { 1.0 / d.x.abs() }, if d.y.abs() < 1.0e-12 { f32::INFINITY } else { 1.0 / d.y.abs() }, if d.z.abs() < 1.0e-12 { f32::INFINITY } else { 1.0 / d.z.abs() });
    let next_boundary = |c: i32, dc: f32, s: i32| -> f32 { if s > 0 { (c as f32 + 1.0 - dc) } else { (dc - c as f32) } };
    let mut t_max = Vec3::new(next_boundary(cell.x, o.x, step.x) * inv.x, next_boundary(cell.y, o.y, step.y) * inv.y, next_boundary(cell.z, o.z, step.z) * inv.z);
    let t_delta = inv;
    let max_t = (max_dist_world / vs).max(0.0);
    let mut t = 0.0f32;
    let mut n = IVec3::Y;
    for _ in 0..8192 {
        if t > max_t { break; }
        if grid.is_solid(cell.x, cell.y, cell.z) {
            let h = (o + d * t) * vs;
            return Some((t * vs, cell, n, h));
        }
        if t_max.x < t_max.y {
            if t_max.x < t_max.z { cell.x += step.x; t = t_max.x; t_max.x += t_delta.x; n = IVec3::new(-step.x, 0, 0); }
            else { cell.z += step.z; t = t_max.z; t_max.z += t_delta.z; n = IVec3::new(0, 0, -step.z); }
        } else {
            if t_max.y < t_max.z { cell.y += step.y; t = t_max.y; t_max.y += t_delta.y; n = IVec3::new(0, -step.y, 0); }
            else { cell.z += step.z; t = t_max.z; t_max.z += t_delta.z; n = IVec3::new(0, 0, -step.z); }
        }
    }
    None
}
