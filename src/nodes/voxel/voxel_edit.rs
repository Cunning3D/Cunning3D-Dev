//! VoxelEdit node: editor-only command list baked into a transient volume.
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use bevy::prelude::*;
use crate::mesh::{Attribute, GeoPrimitive, Geometry, PolygonPrim};
use crate::mesh::PointId;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::attrs;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use cunning_kernel::algorithms::algorithms_editor::voxel as vox;
use crate::volume::{VolumeHandle, CHUNK_SIZE};
use crate::register_node;

const PARAM_VOXEL_SIZE: &str = "voxel_size";
const PARAM_CMDS_JSON: &str = "cmds_json";
const PARAM_PALETTE_JSON: &str = "palette_json";
const PARAM_MASK_JSON: &str = "mask_json";
const PARAM_AI_STAMP_PROMPT: &str = "ai_stamp_prompt";
const PARAM_AI_STAMP_REF_IMAGE: &str = "ai_stamp_reference_image";
const PARAM_AI_STAMP_TILE_RES: &str = "ai_stamp_tile_res";
const PARAM_AI_STAMP_PAL_MAX: &str = "ai_stamp_palette_max";
const PARAM_AI_STAMP_DEPTH_EPS: &str = "ai_stamp_depth_eps";
const PARAM_VIEWPORT_DOCK_PRESET_JSON: &str = "viewport_coverlay_dock_preset_json";
const ATTR_BAKED_CURSOR: &str = "__voxel_baked_cursor";
const ATTR_BASE_FROM_INPUT: &str = "__voxel_base_from_input";
pub const ATTR_VOXEL_SRC_PRIM: &str = "__voxel_src";
pub const ATTR_VOXEL_SIZE_DETAIL: &str = "__voxel_size";
pub const ATTR_VOXEL_CELLS_I32: &str = "__voxel_cells_i32";
pub const ATTR_VOXEL_PI_U8: &str = "__voxel_pi_u8";
pub const ATTR_VOXEL_PALETTE_JSON: &str = "__voxel_palette_json";
pub const ATTR_VOXEL_MASK_CELLS_I32: &str = "__voxel_mask_cells_i32";
const ATTR_VOXEL_CHUNK_PRIM: &str = "__voxel_chunk";
const ATTR_VOXEL_BASE_SIG: &str = "__voxel_base_sig";

#[derive(Default)]
pub struct VoxelEditNode;

impl NodeParameters for VoxelEditNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(PARAM_VOXEL_SIZE, "Voxel Size", "Settings", ParameterValue::Float(0.1), ParameterUIType::FloatSlider { min: 0.001, max: 1000.0 }),
            Parameter::new(PARAM_CMDS_JSON, "CmdList (Internal)", "Internal", ParameterValue::String("{\"ops\":[],\"cursor\":0}".to_string()), ParameterUIType::Code),
            Parameter::new(PARAM_PALETTE_JSON, "Palette (Internal)", "Internal", ParameterValue::String("[]".to_string()), ParameterUIType::Code),
            Parameter::new(PARAM_MASK_JSON, "Mask (Internal)", "Internal", ParameterValue::String("[]".to_string()), ParameterUIType::Code),
            Parameter::new(PARAM_AI_STAMP_PROMPT, "AI Stamp Prompt (Internal)", "Internal", ParameterValue::String("A stylized but physically plausible material.".to_string()), ParameterUIType::Code),
            Parameter::new(PARAM_AI_STAMP_REF_IMAGE, "AI Stamp Reference Image (Internal)", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into(), "jpg".into(), "jpeg".into(), "webp".into()] }),
            Parameter::new(PARAM_AI_STAMP_TILE_RES, "AI Stamp Tile Resolution (Internal)", "Internal", ParameterValue::Int(384), ParameterUIType::IntSlider { min: 128, max: 1024 }),
            Parameter::new(PARAM_AI_STAMP_PAL_MAX, "AI Stamp Palette Max (Internal)", "Internal", ParameterValue::Int(64), ParameterUIType::IntSlider { min: 4, max: 200 }),
            Parameter::new(PARAM_AI_STAMP_DEPTH_EPS, "AI Stamp Depth Epsilon (Internal)", "Internal", ParameterValue::Float(0.02), ParameterUIType::FloatSlider { min: 0.0, max: 0.2 }),
            // Dispersed multi-panel default; palette is a narrow side strip.
            Parameter::new(PARAM_VIEWPORT_DOCK_PRESET_JSON, "Viewport Dock Preset (Internal)", "Internal", ParameterValue::String("{\"VoxelTools\":\"Right\",\"VoxelPalette\":\"Left\",\"VoxelDebug\":\"Bottom\",\"Import\":\"Right\",\"Export\":\"Right\",\"Anim\":\"Bottom\",\"Manager\":\"Right\",\"VoxelPaletteRatio\":0.03}".to_string()), ParameterUIType::Code),
        ]
    }
}

impl NodeOp for VoxelEditNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let input = inputs.first().map(|g| g.materialize()).unwrap_or_else(Geometry::new);
        let pm: HashMap<String, ParameterValue> = params.iter().map(|p| (p.name.clone(), p.value.clone())).collect();
        // NodeGraph has a faster cached path (node-id aware).
        Arc::new(compute_voxel_edit(None, &input, &pm))
    }
}

register_node!(
    "Voxel Edit",
    "Voxel",
    VoxelEditNode;
    coverlay: &[
        cunning_viewport::coverlay_dock::CoverlayPanelKind::VoxelTools,
        cunning_viewport::coverlay_dock::CoverlayPanelKind::VoxelPalette
    ]
);

#[inline]
fn has_meaningful_input(g: &Geometry) -> bool {
    if g.get_detail_attribute(ATTR_VOXEL_CELLS_I32).is_some() { return true; } // discrete payload
    if g.get_detail_attribute("__voxel_node").is_some() { return true; } // cache-linked voxel edit output
    !g.volumes.is_empty()
        || !g.primitives().is_empty()
        || g.get_point_attribute("@P")
            .and_then(|a| a.as_slice::<Vec3>())
            .map_or(false, |v| !v.is_empty())
}

#[inline]
fn read_baked_cursor(g: &Geometry) -> usize {
    g.get_detail_attribute(ATTR_BAKED_CURSOR).and_then(|a| a.as_slice::<i32>()).and_then(|v| v.first().copied()).unwrap_or(0).max(0) as usize
}

#[inline]
fn write_baked_cursor(g: &mut Geometry, v: usize) { g.set_detail_attribute(ATTR_BAKED_CURSOR, vec![v.min(i32::MAX as usize) as i32]); }

#[inline]
fn write_base_from_input(g: &mut Geometry, on: bool) { g.set_detail_attribute(ATTR_BASE_FROM_INPUT, vec![if on { 1.0f32 } else { 0.0f32 }]); }

#[inline]
fn read_voxel_node_id(g: &Geometry) -> Option<uuid::Uuid> {
    g.get_detail_attribute("__voxel_node")
        .and_then(|a| a.as_slice::<String>())
        .and_then(|v| v.first())
        .and_then(|s| uuid::Uuid::parse_str(s.trim()).ok())
}

#[inline]
fn read_base_grid_from_input(input: &Geometry, voxel_size: f32) -> Option<vox::DiscreteVoxelGrid> {
    read_discrete_payload(input, voxel_size).or_else(|| read_voxel_node_id(input).and_then(|id| cunning_kernel::nodes::voxel::voxel_edit::voxel_render_get_grid(id)))
}

#[inline]
fn payload_sig_sample(input: &Geometry) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let cells = input.get_detail_attribute(ATTR_VOXEL_CELLS_I32).and_then(|a| a.as_slice::<i32>())?;
    if cells.len() % 3 != 0 { return None; }
    let n = cells.len() / 3;
    let pis = input
        .get_detail_attribute(ATTR_VOXEL_PI_U8)
        .and_then(|a| a.as_storage::<crate::mesh::Bytes>())
        .map(|b| b.0.as_slice())?;
    if pis.len() != n { return None; }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    n.hash(&mut h);
    let k = 8usize.min(n.max(1));
    let step = (n / k).max(1);
    for i in 0..k {
        let idx = (i * step).min(n - 1);
        let base = idx * 3;
        cells[base].hash(&mut h);
        cells[base + 1].hash(&mut h);
        cells[base + 2].hash(&mut h);
        pis[idx].hash(&mut h);
    }
    Some(h.finish())
}

pub fn compute_voxel_edit(prev_cached: Option<&Geometry>, input: &Geometry, params: &HashMap<String, ParameterValue>) -> Geometry {
    puffin::profile_scope!("VoxelEdit::compute");
    let voxel_size = match params.get(PARAM_VOXEL_SIZE) { Some(ParameterValue::Float(v)) => *v, _ => 0.1 }.max(0.001);
    let cmds = {
        puffin::profile_scope!("VoxelEdit::parse_cmds");
        params
            .get(PARAM_CMDS_JSON)
            .and_then(|v| if let ParameterValue::String(s) = v { Some(s.as_str()) } else { None })
            .map(vox::DiscreteVoxelCmdList::from_json)
            .unwrap_or_default()
    };
    let palette_json = params.get(PARAM_PALETTE_JSON).and_then(|v| if let ParameterValue::String(s) = v { Some(s.as_str()) } else { None }).unwrap_or("[]");
    let mask_json = params.get(PARAM_MASK_JSON).and_then(|v| if let ParameterValue::String(s) = v { Some(s.as_str()) } else { None }).unwrap_or("[]");
    let want_input = has_meaningful_input(input);

    let (mut base, mut st) = if want_input {
        (read_base_grid_from_input(input, voxel_size).unwrap_or_else(|| implicit_discrete_from_input(input, voxel_size)), vox::DiscreteBakeState { baked_cursor: 0 })
    } else if let Some(prev) = prev_cached.and_then(|g| read_discrete_payload(g, voxel_size)) {
        (prev, vox::DiscreteBakeState { baked_cursor: prev_cached.map(read_baked_cursor).unwrap_or(0) })
    } else {
        (vox::DiscreteVoxelGrid::new(voxel_size), vox::DiscreteBakeState { baked_cursor: 0 })
    };
    {
        puffin::profile_scope!("VoxelEdit::parse_palette");
        if let Ok(p) = serde_json::from_str::<Vec<vox::discrete::PaletteEntry>>(palette_json) {
            if !p.is_empty() { for (i, e) in p.into_iter().enumerate() { if i < base.palette.len() { base.palette[i] = e; } } }
        }
    }
    {
        puffin::profile_scope!("VoxelEdit::bake_cmds_incremental");
        vox::discrete::bake_cmds_incremental(&mut base, &cmds, &mut st);
    }

    let mut out = {
        puffin::profile_scope!("VoxelEdit::surface_mesh");
        discrete_to_surface_mesh_with_filter(&base, None)
    };
    // Mark this geometry as voxel-derived on primitive domain so downstream ops (Merge/Blast/Separate)
    // can detect "pure voxel" vs "mixed" and pick the correct grid display mode.
    let prim_n = out.primitives().len();
    if prim_n > 0 { out.insert_primitive_attribute(ATTR_VOXEL_SRC_PRIM, Attribute::new(vec![true; prim_n])); }
    out.set_detail_attribute(ATTR_VOXEL_SIZE_DETAIL, vec![voxel_size]);
    write_baked_cursor(&mut out, st.baked_cursor);
    write_base_from_input(&mut out, want_input);
    // Optional editor selection mask: flat i32 list [x,y,z,x,y,z,...]
    if let Ok(v) = serde_json::from_str::<Vec<i32>>(mask_json) { if !v.is_empty() { out.set_detail_attribute(ATTR_VOXEL_MASK_CELLS_I32, v); } }
    {
        puffin::profile_scope!("VoxelEdit::write_payload");
        write_discrete_payload(&mut out, &base);
    }
    out
}

#[derive(Debug, Clone)]
struct VoxelEditCookCache {
    voxel_size: f32,
    want_input: bool,
    base_sig: u64,
    base_grid: vox::DiscreteVoxelGrid,
    palette_hash: u64,
    palette_dirty: bool,
    applied_cursor: usize,
    grid: vox::DiscreteVoxelGrid,
    chunk_solid: HashMap<IVec3, u32>,
    chunks: HashMap<IVec3, Vec<u8>>,
    dirty_chunks: HashSet<IVec3>,
    chunks_gen: u64,
}

impl VoxelEditCookCache {
    fn new(voxel_size: f32, want_input: bool) -> Self {
        let vs = voxel_size.max(0.001);
        Self {
            voxel_size: vs,
            want_input,
            base_sig: 0,
            base_grid: vox::DiscreteVoxelGrid::new(vs),
            palette_hash: 0,
            palette_dirty: true,
            applied_cursor: 0,
            grid: vox::DiscreteVoxelGrid::new(vs),
            chunk_solid: HashMap::new(),
            chunks: HashMap::new(),
            dirty_chunks: HashSet::new(),
            chunks_gen: 1,
        }
    }
    fn reset_from_grid(&mut self, voxel_size: f32, want_input: bool, grid: vox::DiscreteVoxelGrid) {
        let vs = voxel_size.max(0.001);
        self.voxel_size = vs;
        self.want_input = want_input;
        self.base_sig = 0;
        self.base_grid = grid.clone();
        self.palette_hash = 0;
        self.palette_dirty = true;
        self.applied_cursor = 0;
        self.grid = grid;
        self.chunk_solid.clear();
        self.chunks.clear();
        self.dirty_chunks.clear();
        self.chunks_gen = self.chunks_gen.wrapping_add(1);
        self.rebuild_chunks_from_grid();
    }

    #[inline]
    fn restore_working_from_base(&mut self) {
        self.grid = self.base_grid.clone();
        self.chunk_solid.clear();
        self.chunks.clear();
        self.dirty_chunks.clear();
        self.applied_cursor = 0;
        self.chunks_gen = self.chunks_gen.wrapping_add(1);
        self.rebuild_chunks_from_grid();
        self.palette_dirty = true;
    }

    fn rebuild_chunks_from_grid(&mut self) {
        puffin::profile_scope!("VoxelEdit::rebuild_chunks_from_grid");
        let cs = CHUNK_SIZE.max(4);
        let cs3 = (cs as usize) * (cs as usize) * (cs as usize);
        self.chunks.clear();
        self.chunk_solid.clear();
        for (vox::discrete::VoxelCoord(c), v) in self.grid.voxels.iter() {
            let pi = v.palette_index;
            if pi == 0 { continue; }
            let ck = chunk_coord(*c, cs);
            let lp = chunk_local(*c, cs);
            let buf = self.chunks.entry(ck).or_insert_with(|| vec![0u8; cs3]);
            let idx = chunk_idx(lp, cs);
            if buf[idx] == 0 {
                *self.chunk_solid.entry(ck).or_insert(0) += 1;
            }
            buf[idx] = pi;
        }
        // Full rebuild -> all chunks dirty for render.
        let keys: Vec<IVec3> = self.chunks.keys().copied().collect();
        for ck in keys { self.mark_dirty_chunk_and_neighbors(ck); }
        self.chunks_gen = self.chunks_gen.wrapping_add(1);
    }

    #[inline]
    fn mark_dirty_chunk_and_neighbors(&mut self, ck: IVec3) {
        self.dirty_chunks.insert(ck);
        self.dirty_chunks.insert(ck + IVec3::X);
        self.dirty_chunks.insert(ck + IVec3::NEG_X);
        self.dirty_chunks.insert(ck + IVec3::Y);
        self.dirty_chunks.insert(ck + IVec3::NEG_Y);
        self.dirty_chunks.insert(ck + IVec3::Z);
        self.dirty_chunks.insert(ck + IVec3::NEG_Z);
    }
}

static VOXEL_EDIT_COOK_CACHE: OnceLock<Mutex<HashMap<uuid::Uuid, VoxelEditCookCache>>> = OnceLock::new();

thread_local! { static VOXEL_CDA_INSTANCE_STACK: RefCell<Vec<uuid::Uuid>> = RefCell::new(Vec::new()); }
pub(crate) struct VoxelCdaInstanceGuard(uuid::Uuid);
impl VoxelCdaInstanceGuard {
    #[inline] pub fn push(inst_id: uuid::Uuid) -> Self { VOXEL_CDA_INSTANCE_STACK.with(|s| s.borrow_mut().push(inst_id)); Self(inst_id) }
}
impl Drop for VoxelCdaInstanceGuard {
    fn drop(&mut self) { VOXEL_CDA_INSTANCE_STACK.with(|s| { let mut st = s.borrow_mut(); if st.last().copied() == Some(self.0) { st.pop(); } }); }
}
#[inline]
fn voxel_current_cda_instance() -> Option<uuid::Uuid> { VOXEL_CDA_INSTANCE_STACK.with(|s| s.borrow().last().copied()) }

#[inline]
fn voxel_edit_cache_key(node_id: uuid::Uuid) -> uuid::Uuid {
    voxel_current_cda_instance().map_or(node_id, |inst| {
        let mut b = [0u8; 32];
        b[..16].copy_from_slice(inst.as_bytes());
        b[16..].copy_from_slice(node_id.as_bytes());
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, &b)
    })
}

/// Dirty snapshot for the RenderApp voxel pipeline.
#[derive(Debug, Clone)]
pub struct VoxelRenderDirty {
    pub voxel_size: f32,
    /// Optional palette update (RGBA floats).
    pub palette_rgba: Option<Vec<[f32; 4]>>,
    /// (chunk_key, raw voxels u32[ CHUNK_SIZE^3 ], solid_count)
    pub dirty_raw_chunks: Vec<(IVec3, Vec<u32>, u32)>,
    /// All chunk keys that currently exist (for neighbor lookup).
    pub all_chunk_keys: Vec<IVec3>,
}

/// Return all *existing* solid chunk keys for a VoxelEdit node.
pub fn voxel_render_all_chunk_keys(node_id: uuid::Uuid) -> Vec<IVec3> {
    let cache_map = VOXEL_EDIT_COOK_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let cache_map = cache_map.lock().unwrap();
    cache_map
        .get(&node_id)
        .map(|c| c.chunks.keys().copied().collect())
        .unwrap_or_default()
}

/// Chunk-key generation counter (increments when chunk set changes).
pub fn voxel_render_chunks_gen(node_id: uuid::Uuid) -> u64 {
    let cache_map = VOXEL_EDIT_COOK_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let cache_map = cache_map.lock().unwrap();
    cache_map.get(&node_id).map(|c| c.chunks_gen).unwrap_or(0)
}

#[inline]
fn build_raw_u32_from_chunk(chunk_data: &[u8]) -> Vec<u32> {
    chunk_data.iter().map(|&b| b as u32).collect()
}

/// Take & clear dirty-chunk updates for a given VoxelEdit node (RenderApp consumption).
pub fn voxel_render_take_dirty(node_id: uuid::Uuid) -> Option<VoxelRenderDirty> {
    let cache_map = VOXEL_EDIT_COOK_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache_map = cache_map.lock().unwrap();
    let cache = cache_map.get_mut(&node_id)?;
    if cache.dirty_chunks.is_empty() && !cache.palette_dirty {
        return None;
    }
    let mut dirty_raw_chunks = Vec::with_capacity(cache.dirty_chunks.len());
    for ck in cache.dirty_chunks.drain() {
        let solid = cache.chunk_solid.get(&ck).copied().unwrap_or(0);
        if let Some(chunk_data) = cache.chunks.get(&ck) {
            dirty_raw_chunks.push((ck, build_raw_u32_from_chunk(chunk_data), solid));
        }
    }

    let all_chunk_keys: Vec<IVec3> = cache.chunks.keys().copied().collect();

    let palette_rgba = if cache.palette_dirty {
        cache.palette_dirty = false;
        let mut pal = Vec::with_capacity(cache.grid.palette.len());
        for e in cache.grid.palette.iter() {
            let c = e.color;
            pal.push([
                c[0] as f32 / 255.0,
                c[1] as f32 / 255.0,
                c[2] as f32 / 255.0,
                c[3] as f32 / 255.0,
            ]);
        }
        Some(pal)
    } else {
        None
    };

    Some(VoxelRenderDirty {
        voxel_size: cache.voxel_size,
        palette_rgba,
        dirty_raw_chunks,
        all_chunk_keys,
    })
}

pub fn voxel_render_register_grid(
    node_id: uuid::Uuid,
    voxel_size: f32,
    grid: vox::DiscreteVoxelGrid,
) {
    let cache_map = VOXEL_EDIT_COOK_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache_map = cache_map.lock().unwrap();
    cache_map
        .entry(node_id)
        .or_insert_with(|| VoxelEditCookCache::new(voxel_size, false))
        .reset_from_grid(voxel_size, false, grid);
}

/// Register prebuilt voxel chunks directly for GPU voxel preview (dense/procedural generators).
/// This avoids building a `HashMap<VoxelCoord, ...>` for huge volumes.
pub fn voxel_render_register_chunks(
    node_id: uuid::Uuid,
    voxel_size: f32,
    palette: Vec<vox::PaletteEntry>,
    chunks: HashMap<IVec3, Vec<u8>>,
    chunk_solid: HashMap<IVec3, u32>,
) {
    let cache_map = VOXEL_EDIT_COOK_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache_map = cache_map.lock().unwrap();
    let cache = cache_map
        .entry(node_id)
        .or_insert_with(|| VoxelEditCookCache::new(voxel_size, false));
    cache.voxel_size = voxel_size.max(0.001);
    cache.want_input = false;
    cache.palette_hash = 0;
    cache.palette_dirty = true;
    cache.applied_cursor = 0;
    cache.grid = vox::DiscreteVoxelGrid::new(cache.voxel_size);
    if !palette.is_empty() {
        for (i, e) in palette.into_iter().enumerate() {
            if i < cache.grid.palette.len() {
                cache.grid.palette[i] = e;
            }
        }
    }
    cache.chunks = chunks;
    cache.chunk_solid = chunk_solid;
    cache.dirty_chunks.clear();
    let keys: Vec<IVec3> = cache.chunks.keys().copied().collect();
    for ck in keys {
        cache.mark_dirty_chunk_and_neighbors(ck);
    }
}

/// Clear all voxel render caches (used by Player when switching CDAs).
pub fn voxel_render_clear_all() {
    if let Some(m) = VOXEL_EDIT_COOK_CACHE.get() {
        if let Ok(mut g) = m.lock() {
            g.clear();
        }
    }
}

/// Compute the instance-salted cache key for a CDA-contained voxel node.
#[inline]
pub fn voxel_render_key_for_instance(inst_id: uuid::Uuid, node_id: uuid::Uuid) -> uuid::Uuid {
    let mut b = [0u8; 32];
    b[..16].copy_from_slice(inst_id.as_bytes());
    b[16..].copy_from_slice(node_id.as_bytes());
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, &b)
}

/// Incrementally sync a cmdlist into the voxel render cache (GPU preview path).
/// Returns the cache-keyed node id used by the render pipeline (instance-salted).
#[inline]
pub fn voxel_render_sync_cmds_for_instance(
    inst_id: uuid::Uuid,
    node_id: uuid::Uuid,
    voxel_size: f32,
    cmds: &vox::DiscreteVoxelCmdList,
) -> uuid::Uuid {
    let keyed = voxel_render_key_for_instance(inst_id, node_id);
    voxel_render_sync_cmds(keyed, voxel_size, cmds);
    keyed
}

/// Sync cmdlist into the voxel render cache keyed by `node_id`.
pub fn voxel_render_sync_cmds(
    node_id: uuid::Uuid,
    voxel_size: f32,
    cmds: &vox::DiscreteVoxelCmdList,
) {
    let vs = voxel_size.max(0.001);
    let cache_map = VOXEL_EDIT_COOK_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache_map = cache_map.lock().unwrap();
    let cache = cache_map
        .entry(node_id)
        .or_insert_with(|| VoxelEditCookCache::new(vs, false));

    if (cache.voxel_size - vs).abs() > 0.0 || cache.want_input {
        cache.reset_from_grid(vs, false, vox::DiscreteVoxelGrid::new(vs));
    }

    let cur_cursor = cmds.cursor.min(cmds.ops.len());
    puffin::profile_scope!("VoxelEdit::render_sync_cmds");
    if cur_cursor < cache.applied_cursor {
        cache.restore_working_from_base();
    }
    if cache.applied_cursor == 0 {
        for op in cmds.ops.iter().take(cur_cursor) {
            apply_op_cached(cache, op);
        }
        cache.applied_cursor = cur_cursor;
        let keys: Vec<IVec3> = cache.chunks.keys().copied().collect();
        for ck in keys { cache.mark_dirty_chunk_and_neighbors(ck); }
        return;
    }
    if cache.applied_cursor < cur_cursor {
        let cs = CHUNK_SIZE.max(4);
        let dc_opt = dirty_chunks_from_ops(cmds, cache.voxel_size, cs, cache.applied_cursor, cur_cursor);
        let full_dirty = dc_opt.is_none();
        if let Some(dc) = dc_opt {
            for ck in dc { cache.mark_dirty_chunk_and_neighbors(ck); }
        } else {
            cache.dirty_chunks.clear();
        }
        for op in cmds.ops[cache.applied_cursor..cur_cursor].iter() {
            apply_op_cached(cache, op);
        }
        cache.applied_cursor = cur_cursor;
        if full_dirty {
            let keys: Vec<IVec3> = cache.chunks.keys().copied().collect();
            for ck in keys { cache.mark_dirty_chunk_and_neighbors(ck); }
        }
    }
}

#[inline]
fn hash64(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn op_aabb_vox(op: &vox::DiscreteVoxelOp, voxel_size: f32) -> Option<(IVec3, IVec3)> {
    let vs = voxel_size.max(0.001);
    match op {
        vox::DiscreteVoxelOp::SetVoxel { x, y, z, .. }
        | vox::DiscreteVoxelOp::RemoveVoxel { x, y, z }
        | vox::DiscreteVoxelOp::Paint { x, y, z, .. } => {
            let p = IVec3::new(*x, *y, *z);
            Some((p, p))
        }
        vox::DiscreteVoxelOp::BoxAdd { min, max, .. }
        | vox::DiscreteVoxelOp::BoxRemove { min, max }
        | vox::DiscreteVoxelOp::PerlinFill { min, max, .. } => Some((*min, *max)),
        vox::DiscreteVoxelOp::SphereAdd { center, radius, .. }
        | vox::DiscreteVoxelOp::SphereRemove { center, radius } => {
            let c = (*center / vs).floor().as_ivec3();
            let r = (*radius / vs).ceil() as i32;
            Some((c - IVec3::splat(r), c + IVec3::splat(r)))
        }
        vox::DiscreteVoxelOp::MoveSelected { cells, delta } => {
            if cells.is_empty() { return None; }
            let mut mn = IVec3::splat(i32::MAX);
            let mut mx = IVec3::splat(i32::MIN);
            for c in cells.iter() { mn = mn.min(*c); mx = mx.max(*c); }
            let mn2 = mn.min(mn + *delta);
            let mx2 = mx.max(mx + *delta);
            Some((mn2, mx2))
        }
        vox::DiscreteVoxelOp::Extrude { cells, delta, .. }
        | vox::DiscreteVoxelOp::CloneSelected { cells, delta, .. } => {
            if cells.is_empty() { return None; }
            let mut mn = IVec3::splat(i32::MAX);
            let mut mx = IVec3::splat(i32::MIN);
            for c in cells.iter() { mn = mn.min(*c); mx = mx.max(*c); }
            Some((mn + *delta, mx + *delta))
        }
        vox::DiscreteVoxelOp::ClearAll | vox::DiscreteVoxelOp::TrimToOrigin => None,
    }
}

fn dirty_chunks_from_ops(
    cmds: &vox::DiscreteVoxelCmdList,
    voxel_size: f32,
    cs: i32,
    prev_cursor: usize,
    cur_cursor: usize,
) -> Option<HashSet<IVec3>> {
    if cur_cursor < prev_cursor {
        return None; // undo/rewind -> full rebuild
    }
    if prev_cursor >= cur_cursor || prev_cursor >= cmds.ops.len() {
        return Some(HashSet::new());
    }
    let end = cur_cursor.min(cmds.ops.len());
    let mut mn = IVec3::splat(i32::MAX);
    let mut mx = IVec3::splat(i32::MIN);
    let mut any = false;
    for op in cmds.ops[prev_cursor..end].iter() {
        let Some((a, b)) = op_aabb_vox(op, voxel_size) else {
            // ClearAll / TrimToOrigin etc -> topology-wide change.
            return None;
        };
        mn = mn.min(a);
        mx = mx.max(b);
        any = true;
    }
    if !any {
        return Some(HashSet::new());
    }
    // Expand by 1 voxel for adjacency, then map to chunk coords.
    let mn = mn - IVec3::ONE;
    let mx = mx + IVec3::ONE;
    let c0 = chunk_coord(mn, cs);
    let c1 = chunk_coord(mx, cs);
    let mut out: HashSet<IVec3> = HashSet::new();
    for z in c0.z..=c1.z {
        for y in c0.y..=c1.y {
            for x in c0.x..=c1.x {
                out.insert(IVec3::new(x, y, z));
            }
        }
    }
    // Boundary faces depend on immediate neighbors.
    let dirs = [
        IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z,
    ];
    let base: Vec<IVec3> = out.iter().copied().collect();
    for c in base {
        for d in dirs { out.insert(c + d); }
    }
    Some(out)
}

#[inline]
fn mark_dirty_aabb_chunks(cache: &mut VoxelEditCookCache, mn: IVec3, mx: IVec3) {
    let cs = CHUNK_SIZE.max(4);
    // Expand by 1 voxel for adjacency and face dependencies.
    let mn = mn - IVec3::ONE;
    let mx = mx + IVec3::ONE;
    let c0 = chunk_coord(mn, cs);
    let c1 = chunk_coord(mx, cs);
    for z in c0.z..=c1.z {
        for y in c0.y..=c1.y {
            for x in c0.x..=c1.x {
                cache.mark_dirty_chunk_and_neighbors(IVec3::new(x, y, z));
            }
        }
    }
}

pub fn compute_voxel_edit_cached(
    node_id: uuid::Uuid,
    prev_cached: Option<Arc<Geometry>>,
    input: &Geometry,
    params: &HashMap<String, ParameterValue>,
) -> Geometry {
    puffin::profile_scope!("VoxelEdit::compute_cached");
    let node_id = voxel_edit_cache_key(node_id);
    let voxel_size = match params.get(PARAM_VOXEL_SIZE) { Some(ParameterValue::Float(v)) => *v, _ => 0.1 }.max(0.001);
    let cmds = {
        puffin::profile_scope!("VoxelEdit::parse_cmds");
        params
            .get(PARAM_CMDS_JSON)
            .and_then(|v| if let ParameterValue::String(s) = v { Some(s.as_str()) } else { None })
            .map(vox::DiscreteVoxelCmdList::from_json)
            .unwrap_or_default()
    };
    let palette_json = params
        .get(PARAM_PALETTE_JSON)
        .and_then(|v| if let ParameterValue::String(s) = v { Some(s.as_str()) } else { None })
        .unwrap_or("[]");
    let mask_json = params.get(PARAM_MASK_JSON).and_then(|v| if let ParameterValue::String(s) = v { Some(s.as_str()) } else { None }).unwrap_or("[]");
    let want_input = has_meaningful_input(input);

    // Voxel base from upstream (payload or cache-linked VoxelEdit). Non-voxel geometry keeps legacy path.
    let base_node = read_voxel_node_id(input);
    let base_cursor = input
        .get_detail_attribute(ATTR_BAKED_CURSOR)
        .and_then(|a| a.as_slice::<i32>())
        .and_then(|v| v.first().copied())
        .unwrap_or(0)
        .max(0) as u64;
    let base_sig = if let Some(id) = base_node {
        hash64(&format!("node:{id}:{base_cursor}"))
    } else if let Some(s) = payload_sig_sample(input) {
        hash64(&format!("payload:{s}"))
    } else {
        0
    };
    let base_grid = if base_node.is_some() || input.get_detail_attribute(ATTR_VOXEL_CELLS_I32).is_some() {
        read_base_grid_from_input(input, voxel_size)
    } else {
        None
    };
    if want_input && base_grid.is_none() {
        // Input-driven voxelization is not the interactive hot path; keep legacy behavior for correctness.
        return compute_voxel_edit(prev_cached.as_deref(), input, params);
    }

    // --- Kernel-side voxel render cache (single source of truth for viewport rendering) ---
    let prev_base_sig = prev_cached
        .as_deref()
        .and_then(|g| g.get_detail_attribute(ATTR_VOXEL_BASE_SIG))
        .and_then(|a| a.as_slice::<String>())
        .and_then(|v| v.first())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let pal_h = hash64(palette_json);
    let prev_pal_h = prev_cached
        .as_deref()
        .and_then(|g| g.get_detail_attribute("__voxel_pal_hash"))
        .and_then(|a| a.as_slice::<String>())
        .and_then(|v| v.first())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    // Seed/refresh base grid when upstream changes.
    if base_sig != prev_base_sig {
        if let Some(bg) = base_grid.clone() {
            cunning_kernel::nodes::voxel::voxel_edit::voxel_render_register_grid(node_id, voxel_size, bg);
        }
    }
    // Apply palette edits.
    if pal_h != prev_pal_h {
        cunning_kernel::nodes::voxel::voxel_edit::voxel_render_set_palette_from_json(node_id, palette_json);
    }
    // Apply cmdlist edits (incremental).
    cunning_kernel::nodes::voxel::voxel_edit::voxel_render_sync_cmds(node_id, voxel_size, &cmds);

    // Extreme path: skip CPU surface meshing entirely (hot path).
    // Viewport uses GPU chunk rendering. Downstream geometry nodes should use a dedicated "Voxel To Surface" node later.
    let cur_cursor = cmds.cursor.min(cmds.ops.len());
    let mut out = Geometry::new();
    out.set_detail_attribute(ATTR_VOXEL_SIZE_DETAIL, vec![voxel_size]);
    out.set_detail_attribute("__voxel_pure", vec![1.0f32]);
    out.set_detail_attribute("__voxel_node", vec![node_id.to_string()]);
    out.set_detail_attribute("__voxel_pal_hash", vec![pal_h.to_string()]);
    out.set_detail_attribute(ATTR_VOXEL_BASE_SIG, vec![base_sig.to_string()]);
    write_baked_cursor(&mut out, cur_cursor);
    write_base_from_input(&mut out, want_input);
    if let Ok(v) = serde_json::from_str::<Vec<i32>>(mask_json) { if !v.is_empty() { out.set_detail_attribute(ATTR_VOXEL_MASK_CELLS_I32, v); } }
    out
}

/// Read the authoritative discrete voxel grid from the voxel render cache (GPU preview path).
pub fn voxel_render_get_grid(node_id: uuid::Uuid) -> Option<vox::DiscreteVoxelGrid> {
    let cache_map = VOXEL_EDIT_COOK_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let cache_map = cache_map.lock().ok()?;
    cache_map.get(&node_id).map(|c| c.grid.clone())
}

/// Apply palette edits (JSON) into the voxel render cache, marking palette as dirty for RenderApp upload.
pub fn voxel_render_set_palette_from_json(node_id: uuid::Uuid, palette_json: &str) {
    let cache_map = VOXEL_EDIT_COOK_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache_map = cache_map.lock().unwrap();
    let cache = cache_map.entry(node_id).or_insert_with(|| VoxelEditCookCache::new(0.1, false));
    let pal_h = hash64(palette_json);
    if cache.palette_hash == pal_h {
        return;
    }
    if let Ok(p) = serde_json::from_str::<Vec<vox::discrete::PaletteEntry>>(palette_json) {
        if !p.is_empty() {
            for (i, e) in p.into_iter().enumerate() {
                if i < cache.grid.palette.len() {
                    cache.grid.palette[i] = e;
                }
            }
        }
    }
    cache.palette_hash = pal_h;
    cache.palette_dirty = true;
}

/// Read the authoritative discrete voxel grid from the VoxelEdit cook cache (interactive path).
pub(crate) fn voxel_edit_cache_get_grid(node_id: uuid::Uuid) -> Option<vox::DiscreteVoxelGrid> {
    let cache_map = VOXEL_EDIT_COOK_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let cache_map = cache_map.lock().ok()?;
    cache_map.get(&node_id).map(|c| c.grid.clone())
}

#[inline]
fn set_pi_cached(cache: &mut VoxelEditCookCache, p: IVec3, pi: u8) {
    let cs = CHUNK_SIZE.max(4);
    let cs3 = (cs as usize) * (cs as usize) * (cs as usize);
    let ck = chunk_coord(p, cs);
    let lp = chunk_local(p, cs);
    let idx = chunk_idx(lp, cs);

    if pi == 0 {
        cache.grid.voxels.remove(&vox::discrete::VoxelCoord(p));
        if let Some(buf) = cache.chunks.get_mut(&ck) {
            let old = *buf.get(idx).unwrap_or(&0);
            if old != 0 {
                buf[idx] = 0;
                if let Some(c) = cache.chunk_solid.get_mut(&ck) {
                    *c = c.saturating_sub(1);
                    if *c == 0 {
                        cache.chunk_solid.remove(&ck);
                        cache.chunks.remove(&ck);
                        cache.chunks_gen = cache.chunks_gen.wrapping_add(1);
                    }
                }
            }
        }
        return;
    }

    cache.grid.voxels.insert(
        vox::discrete::VoxelCoord(p),
        vox::DiscreteVoxel {
            palette_index: pi,
            color_override: None,
        },
    );
    let created = !cache.chunks.contains_key(&ck);
    let buf = cache.chunks.entry(ck).or_insert_with(|| vec![0u8; cs3]);
    let old = buf[idx];
    if old == 0 {
        *cache.chunk_solid.entry(ck).or_insert(0) += 1;
    }
    buf[idx] = pi;
    if created {
        cache.chunks_gen = cache.chunks_gen.wrapping_add(1);
    }
}

#[inline]
fn paint_pi_cached(cache: &mut VoxelEditCookCache, p: IVec3, pi: u8) {
    if pi == 0 { return; }
    let Some(v) = cache.grid.voxels.get_mut(&vox::discrete::VoxelCoord(p)) else { return; };
    v.palette_index = pi;
    // Update chunk buffer too (solid count unchanged).
    let cs = CHUNK_SIZE.max(4);
    let ck = chunk_coord(p, cs);
    let lp = chunk_local(p, cs);
    let idx = chunk_idx(lp, cs);
    if let Some(buf) = cache.chunks.get_mut(&ck) {
        if idx < buf.len() {
            buf[idx] = pi;
        }
    }
}

#[inline]
fn hash_u32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

#[inline]
fn rand01(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    let h = hash_u32(
        (x as u32).wrapping_mul(73856093)
            ^ (y as u32).wrapping_mul(19349663)
            ^ (z as u32).wrapping_mul(83492791)
            ^ seed,
    );
    (h as f32) * (1.0 / (u32::MAX as f32))
}

#[inline]
fn smooth(t: f32) -> f32 { t * t * (3.0 - 2.0 * t) }

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }

#[inline]
fn perlin3(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let zi = z.floor() as i32;
    let xf = smooth(x - xi as f32);
    let yf = smooth(y - yi as f32);
    let zf = smooth(z - zi as f32);
    let v000 = rand01(xi, yi, zi, seed);
    let v100 = rand01(xi + 1, yi, zi, seed);
    let v010 = rand01(xi, yi + 1, zi, seed);
    let v110 = rand01(xi + 1, yi + 1, zi, seed);
    let v001 = rand01(xi, yi, zi + 1, seed);
    let v101 = rand01(xi + 1, yi, zi + 1, seed);
    let v011 = rand01(xi, yi + 1, zi + 1, seed);
    let v111 = rand01(xi + 1, yi + 1, zi + 1, seed);
    let x00 = lerp(v000, v100, xf);
    let x10 = lerp(v010, v110, xf);
    let x01 = lerp(v001, v101, xf);
    let x11 = lerp(v011, v111, xf);
    let y0 = lerp(x00, x10, yf);
    let y1 = lerp(x01, x11, yf);
    lerp(y0, y1, zf) * 2.0 - 1.0
}

fn apply_op_cached(cache: &mut VoxelEditCookCache, op: &vox::DiscreteVoxelOp) {
    puffin::profile_scope!("VoxelEdit::apply_op");
    let vs = cache.voxel_size.max(0.001);
    match op {
        vox::DiscreteVoxelOp::SetVoxel { x, y, z, palette_index } => {
            set_pi_cached(cache, IVec3::new(*x, *y, *z), (*palette_index).max(1));
        }
        vox::DiscreteVoxelOp::RemoveVoxel { x, y, z } => {
            set_pi_cached(cache, IVec3::new(*x, *y, *z), 0);
        }
        vox::DiscreteVoxelOp::Paint { x, y, z, palette_index } => {
            paint_pi_cached(cache, IVec3::new(*x, *y, *z), (*palette_index).max(1));
        }
        vox::DiscreteVoxelOp::SphereAdd { center, radius, palette_index } => {
            let r = (*radius / vs).ceil() as i32;
            let c = (*center / vs).floor().as_ivec3();
            let r2 = (radius / vs) * (radius / vs);
            let pi = (*palette_index).max(1);
            for z in (c.z - r)..=(c.z + r) {
                for y in (c.y - r)..=(c.y + r) {
                    for x in (c.x - r)..=(c.x + r) {
                        let d = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5)
                            - (*center / vs);
                        if d.length_squared() <= r2 {
                            set_pi_cached(cache, IVec3::new(x, y, z), pi);
                        }
                    }
                }
            }
        }
        vox::DiscreteVoxelOp::SphereRemove { center, radius } => {
            let r = (*radius / vs).ceil() as i32;
            let c = (*center / vs).floor().as_ivec3();
            let r2 = (radius / vs) * (radius / vs);
            for z in (c.z - r)..=(c.z + r) {
                for y in (c.y - r)..=(c.y + r) {
                    for x in (c.x - r)..=(c.x + r) {
                        let d = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5)
                            - (*center / vs);
                        if d.length_squared() <= r2 {
                            set_pi_cached(cache, IVec3::new(x, y, z), 0);
                        }
                    }
                }
            }
        }
        vox::DiscreteVoxelOp::BoxAdd { min, max, palette_index } => {
            let pi = (*palette_index).max(1);
            for z in min.z..=max.z {
                for y in min.y..=max.y {
                    for x in min.x..=max.x {
                        set_pi_cached(cache, IVec3::new(x, y, z), pi);
                    }
                }
            }
        }
        vox::DiscreteVoxelOp::BoxRemove { min, max } => {
            for z in min.z..=max.z {
                for y in min.y..=max.y {
                    for x in min.x..=max.x {
                        set_pi_cached(cache, IVec3::new(x, y, z), 0);
                    }
                }
            }
        }
        vox::DiscreteVoxelOp::MoveSelected { cells, delta } => {
            let mut tmp: Vec<(IVec3, vox::DiscreteVoxel)> = Vec::with_capacity(cells.len());
            for c in cells.iter() {
                if let Some(v) = cache.grid.voxels.remove(&vox::discrete::VoxelCoord(*c)) {
                    set_pi_cached(cache, *c, 0);
                    tmp.push((*c + *delta, v));
                }
            }
            for (p, v) in tmp {
                set_pi_cached(cache, p, v.palette_index.max(1));
            }
        }
        vox::DiscreteVoxelOp::Extrude { cells, delta, palette_index } => {
            let pi = (*palette_index).max(1);
            for c in cells.iter() {
                set_pi_cached(cache, *c + *delta, pi);
            }
        }
        vox::DiscreteVoxelOp::CloneSelected { cells, delta, overwrite } => {
            let mut src: Vec<(IVec3, u8)> = Vec::with_capacity(cells.len());
            for c in cells.iter() {
                if let Some(v) = cache.grid.voxels.get(&vox::discrete::VoxelCoord(*c)) {
                    src.push((*c, v.palette_index.max(1)));
                }
            }
            for (c, pi) in src {
                let p = c + *delta;
                if !*overwrite && cache.grid.voxels.contains_key(&vox::discrete::VoxelCoord(p)) {
                    continue;
                }
                set_pi_cached(cache, p, pi);
            }
        }
        vox::DiscreteVoxelOp::ClearAll => {
            cache.grid.voxels.clear();
            cache.chunks.clear();
            cache.chunk_solid.clear();
        }
        vox::DiscreteVoxelOp::TrimToOrigin => {
            // Rare, but keep correct: shift all voxels and rebuild chunk buffers.
            if let Some((mn, _mx)) = cache.grid.bounds() {
                if mn != IVec3::ZERO {
                    let mut new_map: std::collections::HashMap<vox::discrete::VoxelCoord, vox::DiscreteVoxel> =
                        std::collections::HashMap::with_capacity(cache.grid.voxels.len());
                    for (vox::discrete::VoxelCoord(c), v) in cache.grid.voxels.drain() {
                        new_map.insert(vox::discrete::VoxelCoord(c - mn), v);
                    }
                    cache.grid.voxels = new_map;
                    cache.rebuild_chunks_from_grid();
                }
            }
        }
        vox::DiscreteVoxelOp::PerlinFill { min, max, scale, threshold, palette_index, seed } => {
            let sc = scale.max(0.0001);
            let thr = threshold.clamp(-1.0, 1.0);
            let pi = (*palette_index).max(1);
            for z in min.z..=max.z {
                for y in min.y..=max.y {
                    for x in min.x..=max.x {
                        let v = perlin3(x as f32 * sc, y as f32 * sc, z as f32 * sc, *seed);
                        if v >= thr {
                            set_pi_cached(cache, IVec3::new(x, y, z), pi);
                        }
                    }
                }
            }
        }
    }
}

#[inline]
pub fn read_discrete_payload(input: &Geometry, voxel_size: f32) -> Option<vox::DiscreteVoxelGrid> {
    let cells = input.get_detail_attribute(ATTR_VOXEL_CELLS_I32).and_then(|a| a.as_slice::<i32>())?;
    if cells.len() % 3 != 0 { return None; }
    let n = cells.len() / 3;
    let pis = input.get_detail_attribute(ATTR_VOXEL_PI_U8).and_then(|a| a.as_storage::<crate::mesh::Bytes>()).map(|b| b.0.as_slice())?;
    if pis.len() != n { return None; }
    let mut g = vox::DiscreteVoxelGrid::new(voxel_size);
    if let Some(pal_s) = input.get_detail_attribute(ATTR_VOXEL_PALETTE_JSON).and_then(|a| a.as_slice::<String>()).and_then(|v| v.first()).map(|s| s.as_str()) {
        if let Ok(p) = serde_json::from_str::<Vec<vox::discrete::PaletteEntry>>(pal_s) {
            if !p.is_empty() { for (i, e) in p.into_iter().enumerate() { if i < g.palette.len() { g.palette[i] = e; } } }
        }
    }
    for i in 0..n {
        let x = cells[i * 3 + 0];
        let y = cells[i * 3 + 1];
        let z = cells[i * 3 + 2];
        let pi = pis[i].max(1);
        g.set(x, y, z, vox::DiscreteVoxel { palette_index: pi, color_override: None });
    }
    Some(g)
}

#[inline]
pub fn write_discrete_payload(out: &mut Geometry, g: &vox::DiscreteVoxelGrid) {
    let mut cells: Vec<i32> = Vec::with_capacity(g.voxels.len() * 3);
    let mut pis: Vec<u8> = Vec::with_capacity(g.voxels.len());
    for (vox::discrete::VoxelCoord(c), v) in g.voxels.iter() {
        cells.extend_from_slice(&[c.x, c.y, c.z]);
        pis.push(v.palette_index.max(1));
    }
    out.set_detail_attribute(ATTR_VOXEL_CELLS_I32, cells);
    out.set_detail_attribute(ATTR_VOXEL_PI_U8, crate::mesh::Bytes(pis));
    let pal = serde_json::to_string(&g.palette).unwrap_or_else(|_| "[]".to_string());
    out.set_detail_attribute(ATTR_VOXEL_PALETTE_JSON, vec![pal]);
}

#[inline]
fn emit_face(geo: &mut Geometry, ps: &mut Vec<Vec3>, prim_cd: &mut Vec<Vec3>, cd: Vec3, p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3) {
    let a = geo.add_point(); ps.push(p0);
    let b = geo.add_point(); ps.push(p1);
    let c = geo.add_point(); ps.push(p2);
    let d = geo.add_point(); ps.push(p3);
    let v0 = geo.add_vertex(a);
    let v1 = geo.add_vertex(b);
    let v2 = geo.add_vertex(c);
    let v3 = geo.add_vertex(d);
    geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: vec![v0, v1, v2, v3] }));
    prim_cd.push(cd);
}

#[inline]
fn cd_from_pi(palette: &[vox::PaletteEntry], pi: u8) -> Vec3 {
    let c = palette.get(pi as usize).map(|p| p.color).unwrap_or([255, 255, 255, 255]);
    Vec3::new(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0)
}

#[inline]
fn chunk_coord(p: IVec3, cs: i32) -> IVec3 { IVec3::new(p.x.div_euclid(cs), p.y.div_euclid(cs), p.z.div_euclid(cs)) }

#[inline]
fn chunk_local(p: IVec3, cs: i32) -> IVec3 { IVec3::new(p.x.rem_euclid(cs), p.y.rem_euclid(cs), p.z.rem_euclid(cs)) }

#[inline]
fn chunk_idx(lp: IVec3, cs: i32) -> usize { (lp.z as usize) * (cs as usize) * (cs as usize) + (lp.y as usize) * (cs as usize) + (lp.x as usize) }

#[inline]
fn get_pi(chunks: &HashMap<IVec3, Vec<u8>>, cs: i32, p: IVec3) -> u8 {
    let ck = chunk_coord(p, cs);
    let lp = chunk_local(p, cs);
    chunks.get(&ck).and_then(|v| v.get(chunk_idx(lp, cs)).copied()).unwrap_or(0)
}

#[inline]
fn point_cached(geo: &mut Geometry, cache: &mut HashMap<IVec3, PointId>, ps: &mut Vec<Vec3>, p: IVec3, vs: f32) -> PointId {
    if let Some(id) = cache.get(&p).copied() { return id; }
    let id = geo.add_point();
    cache.insert(p, id);
    ps.push(Vec3::new(p.x as f32 * vs, p.y as f32 * vs, p.z as f32 * vs));
    id
}

#[inline]
fn emit_quad_cached(geo: &mut Geometry, cache: &mut HashMap<IVec3, PointId>, ps: &mut Vec<Vec3>, prim_cd: &mut Vec<Vec3>, cd: Vec3, p0: IVec3, p1: IVec3, p2: IVec3, p3: IVec3, vs: f32) {
    let a = point_cached(geo, cache, ps, p0, vs);
    let b = point_cached(geo, cache, ps, p1, vs);
    let c = point_cached(geo, cache, ps, p2, vs);
    let d = point_cached(geo, cache, ps, p3, vs);
    let v0 = geo.add_vertex(a);
    let v1 = geo.add_vertex(b);
    let v2 = geo.add_vertex(c);
    let v3 = geo.add_vertex(d);
    geo.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: vec![v0, v1, v2, v3] }));
    prim_cd.push(cd);
}

#[inline]
fn greedy_mesh_mask(mask: &mut [i32], w: usize, h: usize, mut emit: impl FnMut(usize, usize, usize, usize, i32)) {
    for y in 0..h {
        let mut x = 0usize;
        while x < w {
            let v = mask[y * w + x];
            if v == 0 { x += 1; continue; }
            let mut ww = 1usize;
            while x + ww < w && mask[y * w + x + ww] == v { ww += 1; }
            let mut hh = 1usize;
            'outer: while y + hh < h {
                for xx in 0..ww {
                    if mask[(y + hh) * w + x + xx] != v { break 'outer; }
                }
                hh += 1;
            }
            emit(x, y, ww, hh, v);
            for yy in 0..hh { for xx in 0..ww { mask[(y + yy) * w + x + xx] = 0; } }
            x += ww;
        }
    }
}

fn chunks_to_surface_mesh(
    chunks: &HashMap<IVec3, Vec<u8>>,
    palette: &[vox::PaletteEntry],
    voxel_size: f32,
    filter_chunks: Option<&HashSet<IVec3>>,
) -> Geometry {
    let vs = voxel_size.max(0.001);
    let mut out = Geometry::new();
    if chunks.is_empty() { return out; }

    let cs = CHUNK_SIZE.max(4);
    let mut ps: Vec<Vec3> = Vec::new();
    let mut cds_prim: Vec<Vec3> = Vec::new();
    let mut ns_prim: Vec<Vec3> = Vec::new();
    let mut ns_vert: Vec<Vec3> = Vec::new();
    let mut chunk_prim: Vec<IVec3> = Vec::new();

    let mut point_cache: HashMap<IVec3, PointId> = HashMap::new();
    let mut mask: Vec<i32> = vec![0; (cs as usize) * (cs as usize)];
    for (&ck, _) in chunks.iter() {
        if let Some(f) = filter_chunks {
            if !f.contains(&ck) { continue; }
        }
        let base = ck * cs;
        for axis in 0..3 {
            for s in 0..=cs {
                mask.fill(0);
                for v in 0..cs {
                    for u in 0..cs {
                        let (a, b) = match axis {
                            0 => (get_pi(chunks, cs, base + IVec3::new(s - 1, u, v)), get_pi(chunks, cs, base + IVec3::new(s, u, v))),
                            1 => (get_pi(chunks, cs, base + IVec3::new(u, s - 1, v)), get_pi(chunks, cs, base + IVec3::new(u, s, v))),
                            _ => (get_pi(chunks, cs, base + IVec3::new(u, v, s - 1)), get_pi(chunks, cs, base + IVec3::new(u, v, s))),
                        };
                        let val = if a != 0 && b == 0 { a as i32 } else if a == 0 && b != 0 { -(b as i32) } else { 0 };
                        mask[(v as usize) * (cs as usize) + (u as usize)] = val;
                    }
                }

                greedy_mesh_mask(&mut mask, cs as usize, cs as usize, |u0, v0, uw, vh, val| {
                    let pi = val.unsigned_abs() as u8;
                    let cd = cd_from_pi(palette, pi);
                    let u1 = u0 as i32 + uw as i32;
                    let v1 = v0 as i32 + vh as i32;
                    let u0 = u0 as i32;
                    let v0 = v0 as i32;
                    let s = s as i32;
                    let n = match axis {
                        0 => Vec3::X,
                        1 => Vec3::Y,
                        _ => Vec3::Z,
                    } * if val > 0 { 1.0 } else { -1.0 };
                    match axis {
                        0 => {
                            let x = base.x + s;
                            let y0 = base.y + u0; let y1 = base.y + u1;
                            let z0 = base.z + v0; let z1 = base.z + v1;
                            if val > 0 { emit_quad_cached(&mut out, &mut point_cache, &mut ps, &mut cds_prim, cd, IVec3::new(x, y0, z0), IVec3::new(x, y1, z0), IVec3::new(x, y1, z1), IVec3::new(x, y0, z1), vs); }
                            else { emit_quad_cached(&mut out, &mut point_cache, &mut ps, &mut cds_prim, cd, IVec3::new(x, y0, z0), IVec3::new(x, y0, z1), IVec3::new(x, y1, z1), IVec3::new(x, y1, z0), vs); }
                        }
                        1 => {
                            let y = base.y + s;
                            let x0 = base.x + u0; let x1 = base.x + u1;
                            let z0 = base.z + v0; let z1 = base.z + v1;
                            if val > 0 { emit_quad_cached(&mut out, &mut point_cache, &mut ps, &mut cds_prim, cd, IVec3::new(x0, y, z0), IVec3::new(x0, y, z1), IVec3::new(x1, y, z1), IVec3::new(x1, y, z0), vs); }
                            else { emit_quad_cached(&mut out, &mut point_cache, &mut ps, &mut cds_prim, cd, IVec3::new(x0, y, z0), IVec3::new(x1, y, z0), IVec3::new(x1, y, z1), IVec3::new(x0, y, z1), vs); }
                        }
                        _ => {
                            let z = base.z + s;
                            let x0 = base.x + u0; let x1 = base.x + u1;
                            let y0 = base.y + v0; let y1 = base.y + v1;
                            if val > 0 { emit_quad_cached(&mut out, &mut point_cache, &mut ps, &mut cds_prim, cd, IVec3::new(x0, y0, z), IVec3::new(x1, y0, z), IVec3::new(x1, y1, z), IVec3::new(x0, y1, z), vs); }
                            else { emit_quad_cached(&mut out, &mut point_cache, &mut ps, &mut cds_prim, cd, IVec3::new(x0, y0, z), IVec3::new(x0, y1, z), IVec3::new(x1, y1, z), IVec3::new(x1, y0, z), vs); }
                        }
                    }
                    ns_prim.push(n);
                    chunk_prim.push(ck);
                    ns_vert.extend_from_slice(&[n, n, n, n]);
                });
            }
        }
    }

    if !ps.is_empty() { out.insert_point_attribute(attrs::P, Attribute::new(ps)); }
    if !cds_prim.is_empty() { out.insert_primitive_attribute(attrs::CD, Attribute::new(cds_prim)); }
    if !ns_prim.is_empty() { out.insert_primitive_attribute(attrs::N, Attribute::new(ns_prim)); }
    if !ns_vert.is_empty() { out.insert_vertex_attribute(attrs::N, Attribute::new(ns_vert)); }
    if !chunk_prim.is_empty() { out.insert_primitive_attribute(ATTR_VOXEL_CHUNK_PRIM, Attribute::new(chunk_prim)); }
    out
}

pub(crate) fn discrete_to_surface_mesh_with_filter(g: &vox::DiscreteVoxelGrid, filter_chunks: Option<&HashSet<IVec3>>) -> Geometry {
    if g.voxels.is_empty() { return Geometry::new(); }
    let cs = CHUNK_SIZE.max(4);
    let cs3 = (cs as usize) * (cs as usize) * (cs as usize);
    let mut chunks: HashMap<IVec3, Vec<u8>> = HashMap::new();
    for (vox::discrete::VoxelCoord(c), v) in g.voxels.iter() {
        let pi = v.palette_index;
        if pi == 0 { continue; }
        let ck = chunk_coord(*c, cs);
        let lp = chunk_local(*c, cs);
        let buf = chunks.entry(ck).or_insert_with(|| vec![0u8; cs3]);
        buf[chunk_idx(lp, cs)] = pi;
    }
    chunks_to_surface_mesh(&chunks, &g.palette, g.voxel_size, filter_chunks)
}

/// Back-compat helper used by other nodes/importers.
pub(crate) fn discrete_to_surface_mesh(g: &vox::DiscreteVoxelGrid) -> Geometry {
    discrete_to_surface_mesh_with_filter(g, None)
}

/// Public helper for runtime/player voxel previews.
pub fn voxel_discrete_to_surface_mesh(g: &vox::DiscreteVoxelGrid) -> Geometry { discrete_to_surface_mesh_with_filter(g, None) }

fn implicit_discrete_from_input(input: &Geometry, voxel_size: f32) -> vox::DiscreteVoxelGrid {
    if let Some(v) = input.volumes.first() { return discrete_from_volume(v); }
    let h = vox::implicit_edit_volume(Some(input), voxel_size);
    discrete_from_volume(&h)
}

fn discrete_from_volume(h: &VolumeHandle) -> vox::DiscreteVoxelGrid {
    let g = h.grid.read().unwrap();
    let mut out = vox::DiscreteVoxelGrid::new(g.voxel_size.max(0.001));
    for (ck, c) in g.chunks.iter() {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    let v = c.get(x, y, z);
                    if v < 0.0 {
                        let p = *ck * CHUNK_SIZE + IVec3::new(x, y, z);
                        out.set(p.x, p.y, p.z, vox::DiscreteVoxel { palette_index: 1, color_override: None });
                    }
                }
            }
        }
    }
    out
}
