use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use dashmap::DashMap;
use crossbeam_channel::{unbounded, Sender, Receiver};

extern crate cunning_core as cunning_kernel;
use cunning_cda_runtime as cda_rt;

use bevy::prelude::{Vec2, Vec3};
use cunning_kernel::libs::geometry::{attrs, ids::{PointId, PrimId}, sparse_set::ArenaIndex};
use cunning_kernel::libs::geometry::primitives;
use cunning_kernel::libs::geometry::mesh::{Attribute, GeoPrimitive, Geometry, PolygonPrim, new_dirty_id};
use cunning_kernel::libs::geometry::mesh::{BezierCurvePrim, PolylinePrim};
use cunning_kernel::traits::parameter::ParameterValue;
use cunning_kernel::io::blob_store::{BlobStore, global_store};
use bevy::math::{Mat4, Mat3, Quat};

// Runtime CDA Asset (Unity-facing): eval via shared runtime core.

pub type Handle = u64;
pub type InstanceId = u64;
pub type JobId = u64;

// --- CDA Container (DccChunk/GameEngineChunk) ---
const CDA_MAGIC: &[u8; 4] = b"CDA\0";
const CHUNK_GAME: u32 = u32::from_le_bytes(*b"GAME");

#[derive(Clone, Copy)]
struct ChunkEntry { id: u32, off: u64, size: u64 }

fn read_u32_le(data: &[u8], at: usize) -> Option<u32> { Some(u32::from_le_bytes(data.get(at..at+4)?.try_into().ok()?)) }
fn read_u64_le(data: &[u8], at: usize) -> Option<u64> { Some(u64::from_le_bytes(data.get(at..at+8)?.try_into().ok()?)) }

fn load_game_chunk(path: &str) -> Option<String> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => { *last_cda_err().lock().unwrap() = format!("CDA read failed: {} ({})", e, path); return None; }
    };
    if bytes.get(0..4) != Some(CDA_MAGIC) { *last_cda_err().lock().unwrap() = format!("CDA invalid magic: {}", path); return None; } // no fallback by design
    let ver = match read_u32_le(&bytes, 4) { Some(v) => v, None => { *last_cda_err().lock().unwrap() = format!("CDA invalid header(version): {}", path); return None; } };
    if ver != 1 { *last_cda_err().lock().unwrap() = format!("CDA version mismatch: {} (ver={})", path, ver); return None; }
    let n = match read_u32_le(&bytes, 8) { Some(v) => v as usize, None => { *last_cda_err().lock().unwrap() = format!("CDA invalid header(chunk_count): {}", path); return None; } };
    let mut at = 12usize;
    let mut entries: Vec<ChunkEntry> = Vec::with_capacity(n);
    for _ in 0..n {
        let id = match read_u32_le(&bytes, at) { Some(v) => v, None => { *last_cda_err().lock().unwrap() = format!("CDA invalid chunk table(id): {}", path); return None; } }; at += 4;
        let off = match read_u64_le(&bytes, at) { Some(v) => v, None => { *last_cda_err().lock().unwrap() = format!("CDA invalid chunk table(off): {}", path); return None; } }; at += 8;
        let size = match read_u64_le(&bytes, at) { Some(v) => v, None => { *last_cda_err().lock().unwrap() = format!("CDA invalid chunk table(size): {}", path); return None; } }; at += 8;
        entries.push(ChunkEntry { id, off, size });
    }
    let e = match entries.iter().find(|e| e.id == CHUNK_GAME) {
        Some(v) => v,
        None => { *last_cda_err().lock().unwrap() = format!("CDA missing GAME chunk: {}", path); return None; }
    };
    let s = e.off as usize;
    let eend = match s.checked_add(e.size as usize) { Some(v) => v, None => { *last_cda_err().lock().unwrap() = format!("CDA invalid GAME range: {}", path); return None; } };
    let payload = match bytes.get(s..eend) { Some(v) => v, None => { *last_cda_err().lock().unwrap() = format!("CDA invalid GAME slice: {}", path); return None; } };
    match std::str::from_utf8(payload) {
        Ok(s) => Some(s.to_string()),
        Err(_) => { *last_cda_err().lock().unwrap() = format!("CDA GAME is not utf8: {}", path); None }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus { Pending = 0, Running = 1, Ready = 2, Cancelled = 3, Failed = 4 }

impl Default for JobStatus { fn default() -> Self { Self::Pending } }

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct JobStats {
    pub status: JobStatus,
    pub submitted_ms: u64,
    pub started_ms: u64,
    pub finished_ms: u64,
    pub compute_ms: u64,
    pub out_handle: Handle,
}

struct JobRequest {
    job_id: JobId,
    instance_id: InstanceId,
    gen: u64,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    cda_id: u64,
    params_json: String,
    inputs: Vec<Handle>,
}

struct JobRuntime {
    tx: Sender<JobRequest>,
    latest_gen: DashMap<InstanceId, u64>,
    cancel_map: DashMap<JobId, Arc<std::sync::atomic::AtomicBool>>,
    stats: DashMap<JobId, JobStats>,
    outputs: DashMap<JobId, Vec<Handle>>,
    errors: DashMap<JobId, String>,
}

static JOB_RT: OnceCell<JobRuntime> = OnceCell::new();
static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);
static START_EPOCH: OnceCell<Instant> = OnceCell::new();
static LAST_CDA_ERROR: OnceCell<Mutex<String>> = OnceCell::new();
static BLOB_REGISTRY: OnceCell<Mutex<HashMap<u64, Vec<u8>>>> = OnceCell::new();
static NEXT_BLOB_ID: AtomicU64 = AtomicU64::new(1);
static SESSION_PATH: OnceCell<Mutex<Option<String>>> = OnceCell::new();
static LATEST_REG: OnceCell<Mutex<HashMap<u64, u64>>> = OnceCell::new();

fn ms_now() -> u64 { START_EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64 }
fn last_cda_err() -> &'static Mutex<String> { LAST_CDA_ERROR.get_or_init(|| Mutex::new(String::new())) }
fn blob_reg() -> &'static Mutex<HashMap<u64, Vec<u8>>> { BLOB_REGISTRY.get_or_init(|| Mutex::new(HashMap::new())) }
fn session_path() -> &'static Mutex<Option<String>> { SESSION_PATH.get_or_init(|| Mutex::new(None)) }
fn latest_reg() -> &'static Mutex<HashMap<u64, u64>> { LATEST_REG.get_or_init(|| Mutex::new(HashMap::new())) }

fn session_store() -> Option<std::sync::MutexGuard<'static, BlobStore>> { global_store() }

#[inline]
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for &b in bytes { h ^= b as u64; h = h.wrapping_mul(1099511628211); }
    h
}

fn job_rt() -> &'static JobRuntime {
    JOB_RT.get_or_init(|| {
        let (tx, rx) = unbounded::<JobRequest>();
        let rt = JobRuntime {
            tx,
            latest_gen: DashMap::new(),
            cancel_map: DashMap::new(),
            stats: DashMap::new(),
            outputs: DashMap::new(),
            errors: DashMap::new(),
        };
        std::thread::spawn(move || worker_loop(rx));
        rt
    })
}

pub type CdaAsset = cda_rt::RuntimeDefinition;

#[derive(Clone)]
struct CdaEntry { def: CdaAsset, plan: cda_rt::compiler::ExecutionPlan }

static CDA_RT_REG: OnceCell<cda_rt::registry::RuntimeRegistry> = OnceCell::new();
fn cda_registry() -> &'static cda_rt::registry::RuntimeRegistry { CDA_RT_REG.get_or_init(|| cda_rt::registry::RuntimeRegistry::new_default()) }

static CDA_REGISTRY: OnceCell<Mutex<HashMap<u64, CdaEntry>>> = OnceCell::new();
static NEXT_CDA_ID: AtomicU64 = AtomicU64::new(1);

fn cda_reg() -> &'static Mutex<HashMap<u64, CdaEntry>> {
    CDA_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_geo_handle(geo: Arc<Geometry>) -> Handle {
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    {
        let mut reg = get_geo_registry().lock().unwrap();
        reg.insert(handle, geo.clone());
    }
    {
        let mut caches = get_cache_map().lock().unwrap();
        let mut cache = ExportCache::default();
        cache.sync(&geo);
        caches.insert(handle, cache);
    }
    handle
}

fn worker_loop(rx: Receiver<JobRequest>) {
    while let Ok(req) = rx.recv() {
        let rt = job_rt();
        // drop outdated jobs immediately
        if rt.latest_gen.get(&req.instance_id).map(|v| *v).unwrap_or(req.gen) > req.gen {
            let mut st = rt.stats.get(&req.job_id).map(|v| *v.value()).unwrap_or_default();
            st.status = JobStatus::Cancelled;
            st.finished_ms = ms_now();
            rt.stats.insert(req.job_id, st);
            continue;
        }

        {
            let mut st = rt.stats.get(&req.job_id).map(|v| *v.value()).unwrap_or_default();
            st.status = JobStatus::Running;
            st.started_ms = ms_now();
            rt.stats.insert(req.job_id, st);
        }

        // Real compute: CDA runtime evaluate (multi-input/multi-output).
        let t0 = Instant::now();
        let mut out0: Handle = 0;
        let mut outs: Vec<Handle> = Vec::new();
        let ok = catch_unwind(AssertUnwindSafe(|| {
            if req.cancel.load(std::sync::atomic::Ordering::Relaxed) { return None; }
            let entry = { cda_reg().lock().unwrap().get(&req.cda_id).cloned()? };
            let params: HashMap<String, ParameterValue> = serde_json::from_str(&req.params_json).ok().unwrap_or_default();
            let mut inputs: Vec<Arc<Geometry>> = Vec::new();
            { let reg = get_geo_registry().lock().unwrap(); for h in &req.inputs { inputs.push(reg.get(h).cloned().unwrap_or_else(|| Arc::new(Geometry::new()))); } }
            let geos = match cda_rt::execute(&entry.plan, &entry.def, cda_registry(), &inputs, &params, &req.cancel) {
                Ok(v) => v,
                Err(e) => { rt.errors.insert(req.job_id, format!("{:?}", e)); return None; }
            };
            for g in geos { outs.push(register_geo_handle(g)); }
            out0 = outs.get(0).copied().unwrap_or(0);
            Some(())
        })).ok().flatten().is_some();
        let compute_ms = t0.elapsed().as_millis() as u64;

        let mut st = rt.stats.get(&req.job_id).map(|v| *v.value()).unwrap_or_default();
        st.compute_ms = compute_ms;
        st.finished_ms = ms_now();
        st.out_handle = out0;
        st.status = if req.cancel.load(std::sync::atomic::Ordering::Relaxed) { JobStatus::Cancelled } else if ok { JobStatus::Ready } else { JobStatus::Failed };
        rt.stats.insert(req.job_id, st);
        if !outs.is_empty() { rt.outputs.insert(req.job_id, outs); }
        if ok { rt.errors.remove(&req.job_id); }
        rt.cancel_map.remove(&req.job_id);
    }
}

// --- Graph Asset (*.cgraph) ---
pub type NodeId = Uuid;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphAsset {
    pub meta: GraphMeta,
    pub logic: GraphLogic,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<GraphEditorState>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphMeta {
    pub format_version: u32,
    pub min_engine_version: String,
    pub uuid: Uuid,
    pub name: String,
    pub author: Option<String>,
    pub license: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphLogic {
    pub nodes: HashMap<NodeId, NodeAssetData>,
    pub connections: Vec<ConnectionAssetData>,
    #[serde(default)]
    pub inputs: Vec<GraphPortDef>,
    #[serde(default)]
    pub outputs: Vec<GraphPortDef>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NodeAssetData {
    pub type_id: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub params: HashMap<String, ParameterValue>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConnectionAssetData {
    pub from_node: NodeId,
    pub from_socket: String,
    pub to_node: NodeId,
    pub to_socket: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphPortDef {
    pub name: String,
    pub data_type: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GraphEditorState {
    pub node_positions: HashMap<NodeId, Vec2>,
}

#[derive(Clone)]
struct NodeEval {
    type_id: String,
    params: HashMap<String, ParameterValue>,
    inputs: Vec<NodeId>,
}

// --- Engine Context ---
struct EngineContext {
    asset: Option<GraphAsset>,
    output_geo: Arc<Geometry>,
}

static ENGINE_CONTEXT: OnceCell<Mutex<EngineContext>> = OnceCell::new();

fn get_engine_context() -> &'static Mutex<EngineContext> {
    ENGINE_CONTEXT.get_or_init(|| Mutex::new(EngineContext { asset: None, output_geo: Arc::new(Geometry::new()) }))
}

// --- CDA Asset Load / Submit ---
#[no_mangle]
pub extern "C" fn cunning_cda_load(path: *const c_char) -> u64 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if path.is_null() { *last_cda_err().lock().unwrap() = "CDA load failed: null path".to_string(); return None; }
        let c_str = unsafe { CStr::from_ptr(path) };
        let p = match c_str.to_str() {
            Ok(v) => v,
            Err(_) => { *last_cda_err().lock().unwrap() = "CDA load failed: path not utf8".to_string(); return None; }
        };
        let json = load_game_chunk(p)?;
        let def: CdaAsset = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(e) => { *last_cda_err().lock().unwrap() = format!("CDA parse failed: {}", e); return None; }
        };
        let plan = match cda_rt::compile(&def, cda_registry()) {
            Ok(p) => p,
            Err(e) => { *last_cda_err().lock().unwrap() = format!("CDA compile failed: {:?}", e); return None; }
        };
        let id = NEXT_CDA_ID.fetch_add(1, Ordering::Relaxed);
        cda_reg().lock().unwrap().insert(id, CdaEntry { def, plan });
        Some(id)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_cda_get_last_error(out_buf: *mut c_char, cap: u32) -> u32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if out_buf.is_null() || cap == 0 { return None; }
        let msg = last_cda_err().lock().unwrap().clone();
        let bytes = msg.as_bytes();
        let n = bytes.len().min(cap.saturating_sub(1) as usize);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf as *mut u8, n);
            *out_buf.add(n) = 0;
        }
        Some(1u32)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_cda_submit(cda_id: u64, instance_id: InstanceId, gen: u64, params_json: *const c_char, input_handles: *const Handle, input_count: u32) -> JobId {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let rt = job_rt();
        rt.latest_gen.insert(instance_id, gen);
        let job_id = NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        rt.cancel_map.insert(job_id, cancel.clone());
        rt.stats.insert(job_id, JobStats { status: JobStatus::Pending, submitted_ms: ms_now(), ..Default::default() });

        let json = if params_json.is_null() { "{}".to_string() } else { unsafe { CStr::from_ptr(params_json) }.to_str().unwrap_or("{}").to_string() };
        let inputs = if input_handles.is_null() || input_count == 0 { Vec::new() } else { unsafe { std::slice::from_raw_parts(input_handles, input_count as usize) }.to_vec() };

        let _ = rt.tx.send(JobRequest { job_id, instance_id, gen, cancel, cda_id, params_json: json, inputs });
        Some(job_id)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_job_cancel(job_id: JobId) -> u32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let rt = job_rt();
        if let Some(c) = rt.cancel_map.get(&job_id) { c.value().store(true, std::sync::atomic::Ordering::Relaxed); }
        Some(1u32)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_job_poll(job_id: JobId) -> JobStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let rt = job_rt();
        rt.stats.get(&job_id).map(|s| s.status).unwrap_or(JobStatus::Failed)
    }));
    result.unwrap_or(JobStatus::Failed)
}

#[no_mangle]
pub extern "C" fn cunning_job_get_stats(job_id: JobId, out_stats: *mut JobStats) -> u32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if out_stats.is_null() { return None; }
        let rt = job_rt();
        let st = rt.stats.get(&job_id).map(|v| *v.value()).unwrap_or(JobStats { status: JobStatus::Failed, ..Default::default() });
        unsafe { *out_stats = st; }
        Some(1u32)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_job_get_output_count(job_id: JobId) -> u32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let rt = job_rt();
        rt.outputs.get(&job_id).map(|v| v.len() as u32).unwrap_or(0)
    }));
    result.unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_job_get_output_handle(job_id: JobId, index: u32) -> Handle {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let rt = job_rt();
        rt.outputs.get(&job_id).and_then(|v| v.get(index as usize).copied()).unwrap_or(0)
    }));
    result.unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_job_get_error(job_id: JobId, out_buf: *mut c_char, cap: u32) -> u32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if out_buf.is_null() || cap == 0 { return None; }
        let rt = job_rt();
        let msg = rt.errors.get(&job_id).map(|s| s.value().clone()).unwrap_or_default();
        let bytes = msg.as_bytes();
        let n = bytes.len().min(cap.saturating_sub(1) as usize);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf as *mut u8, n);
            *out_buf.add(n) = 0;
        }
        Some(1u32)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

fn cook_asset(asset: &GraphAsset) -> Option<Arc<Geometry>> {
    let mut incoming: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for c in &asset.logic.connections {
        incoming.entry(c.to_node).or_default().push(c.from_node);
    }

    let mut nodes: HashMap<NodeId, NodeEval> = HashMap::new();
    for (id, n) in &asset.logic.nodes {
        nodes.insert(*id, NodeEval { type_id: n.type_id.clone(), params: n.params.clone(), inputs: incoming.remove(id).unwrap_or_default() });
    }

    // MVP: choose a deterministic output node. Prefer a node named "Output" if present, else last key.
    let out_id = nodes.iter().find_map(|(id, n)| (n.type_id == "Output").then_some(*id)).or_else(|| nodes.keys().copied().last())?;

    fn eval(id: NodeId, nodes: &HashMap<NodeId, NodeEval>, cache: &mut HashMap<NodeId, Arc<Geometry>>) -> Option<Arc<Geometry>> {
        if let Some(g) = cache.get(&id) { return Some(g.clone()); }
        let n = nodes.get(&id)?;
        let mut ins: Vec<Arc<Geometry>> = Vec::new();
        for &iid in &n.inputs {
            if let Some(g) = eval(iid, nodes, cache) { ins.push(g); }
        }

        let out = match n.type_id.as_str() {
            "Boolean" | "boolean" | "cunning.modeling.boolean" => {
                use cunning_kernel::nodes::modeling::boolean::BooleanNode;
                use cunning_kernel::cunning_core::traits::node_interface::NodeParameters;
                let mut p: HashMap<String, ParameterValue> = HashMap::new();
                for dp in <BooleanNode as NodeParameters>::define_parameters() { p.insert(dp.name, dp.value); }
                for (k, v) in &n.params { p.insert(k.clone(), v.clone()); }
                Arc::new(cunning_kernel::nodes::modeling::boolean::compute_boolean(&ins, &p))
            }
            "Poly Extrude" | "PolyExtrude" | "poly_extrude" | "cunning.modeling.poly_extrude" => {
                use cunning_kernel::nodes::modeling::poly_extrude::PolyExtrudeNode;
                use cunning_kernel::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
                let mut params = <PolyExtrudeNode as NodeParameters>::define_parameters();
                for (k, v) in &n.params {
                    if let Some(p) = params.iter_mut().find(|p| p.name == *k) { p.value = v.clone(); }
                }
                PolyExtrudeNode::default().compute(&params, &ins)
            }
            _ => Arc::new(Geometry::new()),
        };

        cache.insert(id, out.clone());
        Some(out)
    }

    let mut cache: HashMap<NodeId, Arc<Geometry>> = HashMap::new();
    eval(out_id, &nodes, &mut cache)
}

// --- Export Cache (Visualization) ---
#[derive(Default)]
struct ExportCache {
    dirty_id: u64,
    point_positions: Vec<[f32; 3]>,
    positions: Vec<[f32; 3]>,
    prim_ids: Vec<u64>,
    prim_point_offsets: Vec<u32>,
    prim_point_indices: Vec<u32>,
    tri_indices: Vec<u32>,
    line_indices: Vec<u32>,
}

impl ExportCache {
    fn sync(&mut self, geo: &Geometry) {
        if self.dirty_id == geo.dirty_id { return; }
        self.dirty_id = geo.dirty_id;
        self.point_positions.clear();
        self.positions.clear();
        self.prim_ids.clear();
        self.prim_point_offsets.clear();
        self.prim_point_indices.clear();
        self.tri_indices.clear();
        self.line_indices.clear();

        if let Some(p_attr) = geo.get_point_attribute(cunning_kernel::libs::geometry::attrs::P) {
            if let Some(p) = p_attr.as_slice::<Vec3>() {
                self.point_positions.reserve(p.len());
                for v in p.iter() { self.point_positions.push(v.to_array()); }
            } else if let Some(pb) = p_attr.as_paged::<Vec3>() {
                self.point_positions.reserve(pb.len());
                for v in pb.iter() { self.point_positions.push(v.to_array()); }
            }
        }

        let verts = geo.vertices();
        let points = geo.points();
        self.positions.reserve(verts.len());
        for v in verts.iter() {
            let pd = points.get_dense_index(v.point_id.into());
            let p = pd.and_then(|i| self.point_positions.get(i)).copied().unwrap_or_default();
            self.positions.push(p);
        }

        let prims = geo.primitives();
        self.prim_ids.reserve(prims.len());
        for di in 0..prims.len() { self.prim_ids.push(di as u64); }

        self.prim_point_offsets.reserve(prims.len() + 1);
        self.prim_point_offsets.push(0);
        for prim in prims.iter() {
            if let GeoPrimitive::Polygon(poly) = prim {
                let mut idx = Vec::with_capacity(poly.vertices.len());
                for &vid in poly.vertices.iter() {
                    let Some(vd) = verts.get_dense_index(vid.into()) else { idx.clear(); break; };
                    idx.push(vd as u32);
                }
                if idx.len() >= 3 {
                    let v0 = idx[0];
                    for i in 1..idx.len() - 1 { self.tri_indices.extend_from_slice(&[v0, idx[i], idx[i + 1]]); }
                }

                if idx.len() >= 2 {
                    for i in 0..idx.len() {
                        let v0 = idx[i];
                        let v1 = idx[(i + 1) % idx.len()];
                        self.line_indices.push(v0);
                        self.line_indices.push(v1);
                    }
                }

                for &vid in poly.vertices.iter() {
                    if let Some(vref) = verts.get(vid.into()) {
                        if let Some(pd) = points.get_dense_index(vref.point_id.into()) { self.prim_point_indices.push(pd as u32); }
        }
    }
            }
            self.prim_point_offsets.push(self.prim_point_indices.len() as u32);
        }
    }
}

fn cook_cda(asset: &CdaAsset, promoted_values: &HashMap<String, ParameterValue>, inputs: &[Arc<Geometry>], cancel: &std::sync::atomic::AtomicBool) -> Option<Vec<Arc<Geometry>>> {
    cda_rt::cook(asset, inputs, promoted_values, cancel).ok()
}

static CACHES: OnceCell<Mutex<HashMap<u64, ExportCache>>> = OnceCell::new();
static GEO_REGISTRY: OnceCell<Mutex<HashMap<u64, Arc<Geometry>>>> = OnceCell::new();
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn get_cache_map() -> &'static Mutex<HashMap<u64, ExportCache>> { CACHES.get_or_init(|| Mutex::new(HashMap::new())) }
fn get_geo_registry() -> &'static Mutex<HashMap<u64, Arc<Geometry>>> { GEO_REGISTRY.get_or_init(|| Mutex::new(HashMap::new())) }

#[inline] fn pack_ai(ai: ArenaIndex) -> u64 { ((ai.generation as u64) << 32) | (ai.index as u64) }
#[inline] fn unpack_ai(id: u64) -> ArenaIndex { ArenaIndex::from_raw(id as u32, (id >> 32) as u32) }
#[inline] fn pack_point(id: PointId) -> u64 { pack_ai(id.into()) }
#[inline] fn unpack_point(id: u64) -> PointId { PointId::from(unpack_ai(id)) }
#[inline] fn pack_prim(id: PrimId) -> u64 { pack_ai(id.into()) }

// --- FFI Functions ---
#[no_mangle]
pub extern "C" fn cunning_init() { get_engine_context(); }

// --- Debug Bridge (redb-backed capture store) ---
#[no_mangle]
pub extern "C" fn cunning_bridge_open(path: *const c_char, create: u32) -> u32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if path.is_null() { return None; }
        let p = unsafe { CStr::from_ptr(path) }.to_str().ok()?.to_string();
        if let Some(existing) = session_path().lock().unwrap().clone() { if existing != p { *last_cda_err().lock().unwrap() = format!("Session already open: {existing}"); return None; } return Some(1u32); }
        // Initialize global store in cunning_core (OnceLock). We allow only one session per process.
        if session_store().is_none() {
            if create != 0 { cunning_kernel::io::blob_store::init_global_store(p.clone().into()); }
            else { cunning_kernel::io::blob_store::init_global_store_existing(p.clone().into()); }
        }
        *session_path().lock().unwrap() = Some(p);
        Some(1u32)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_bridge_put_blob(ptr: *const u8, len: u32) -> u64 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if ptr.is_null() || len == 0 { return None; }
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
        if let Some(store) = session_store() { return store.insert_alloc(bytes).ok(); }
        let id = NEXT_BLOB_ID.fetch_add(1, Ordering::Relaxed);
        blob_reg().lock().unwrap().insert(id, bytes.to_vec());
        Some(id)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_bridge_get_blob_size(blob_id: u64) -> u32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if let Some(store) = session_store() { return store.get(blob_id).ok().flatten().map(|b| b.len().min(u32::MAX as usize) as u32); }
        blob_reg().lock().unwrap().get(&blob_id).map(|b| b.len().min(u32::MAX as usize) as u32)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_bridge_copy_blob(blob_id: u64, out_ptr: *mut u8, out_cap: u32) -> u32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if out_ptr.is_null() || out_cap == 0 { return None; }
        let data = if let Some(store) = session_store() { store.get(blob_id).ok().flatten()? } else { blob_reg().lock().unwrap().get(&blob_id).cloned()? };
        let n = data.len().min(out_cap as usize);
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), out_ptr, n); }
        Some(n.min(u32::MAX as usize) as u32)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

// --- Bridge State Channels (latest pointer per key) ---
// key is utf8 bytes hashed by FNV-1a 64. We store: key_hash -> latest_blob_id.

#[no_mangle]
pub extern "C" fn cunning_state_get_latest(key_ptr: *const u8, key_len: u32) -> u64 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if key_ptr.is_null() || key_len == 0 { return None; }
        let k = unsafe { std::slice::from_raw_parts(key_ptr, key_len as usize) };
        let kh = fnv1a64(k);
        if let Some(store) = session_store() { return store.get_latest(kh).ok(); }
        latest_reg().lock().unwrap().get(&kh).copied()
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_state_set_latest(key_ptr: *const u8, key_len: u32, blob_id: u64) -> u32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if key_ptr.is_null() || key_len == 0 { return None; }
        let k = unsafe { std::slice::from_raw_parts(key_ptr, key_len as usize) };
        let kh = fnv1a64(k);
        if let Some(store) = session_store() { return store.set_latest(kh, blob_id).ok().map(|_| 1u32); }
        latest_reg().lock().unwrap().insert(kh, blob_id);
        Some(1u32)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_state_put_latest(key_ptr: *const u8, key_len: u32, ptr: *const u8, len: u32) -> u64 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if key_ptr.is_null() || key_len == 0 || ptr.is_null() || len == 0 { return None; }
        let k = unsafe { std::slice::from_raw_parts(key_ptr, key_len as usize) };
        let kh = fnv1a64(k);
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
        if let Some(store) = session_store() {
            let id = store.insert_alloc(bytes).ok()?;
            let _ = store.set_latest(kh, id);
            return Some(id);
        }
        let id = NEXT_BLOB_ID.fetch_add(1, Ordering::Relaxed);
        blob_reg().lock().unwrap().insert(id, bytes.to_vec());
        latest_reg().lock().unwrap().insert(kh, id);
        Some(id)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

// --- Spline Snapshot (FBS -> Geometry Handle) ---

#[inline] fn rd_u16(b: &[u8], at: usize) -> Option<u16> { Some(u16::from_le_bytes(b.get(at..at+2)?.try_into().ok()?)) }
#[inline] fn rd_u32(b: &[u8], at: usize) -> Option<u32> { Some(u32::from_le_bytes(b.get(at..at+4)?.try_into().ok()?)) }
#[inline] fn rd_i32(b: &[u8], at: usize) -> Option<i32> { Some(i32::from_le_bytes(b.get(at..at+4)?.try_into().ok()?)) }
#[inline] fn rd_f32(b: &[u8], at: usize) -> Option<f32> { Some(f32::from_le_bytes(b.get(at..at+4)?.try_into().ok()?)) }
#[inline] fn vtbl_off(b: &[u8], obj: usize, voff: usize) -> Option<usize> {
    let vt_rel = rd_i32(b, obj)? as isize;
    let vt = (obj as isize - vt_rel) as usize;
    let idx = (voff.saturating_sub(4)) / 2;
    let entry = rd_u16(b, vt + 4 + idx * 2)? as usize;
    if entry == 0 { None } else { Some(obj + entry) }
}
#[inline] fn obj_u32(b: &[u8], obj: usize, voff: usize, def: u32) -> u32 { vtbl_off(b, obj, voff).and_then(|p| rd_u32(b, p)).unwrap_or(def) }
#[inline] fn obj_u8(b: &[u8], obj: usize, voff: usize, def: u8) -> u8 { vtbl_off(b, obj, voff).and_then(|p| b.get(p).copied()).unwrap_or(def) }
#[inline] fn obj_f32(b: &[u8], obj: usize, voff: usize, def: f32) -> f32 { vtbl_off(b, obj, voff).and_then(|p| rd_f32(b, p)).unwrap_or(def) }
#[inline] fn obj_struct(b: &[u8], obj: usize, voff: usize, size: usize) -> Option<&[u8]> { let p = vtbl_off(b, obj, voff)?; b.get(p..p+size) }
#[inline] fn obj_tbl(b: &[u8], obj: usize, voff: usize) -> Option<usize> { let p = vtbl_off(b, obj, voff)?; Some(p + rd_u32(b, p)? as usize) }
#[inline] fn obj_vec(b: &[u8], obj: usize, voff: usize) -> Option<(usize, usize)> {
    let p = vtbl_off(b, obj, voff)?;
    let v = p + rd_u32(b, p)? as usize;
    Some((v + 4, rd_u32(b, v)? as usize))
}
#[inline] fn vec_tbl_at(b: &[u8], base: usize, i: usize) -> Option<usize> { let p = base + i * 4; Some(p + rd_u32(b, p)? as usize) }

#[inline] fn read_vec3(s: &[u8]) -> Option<Vec3> { Some(Vec3::new(rd_f32(s, 0)?, rd_f32(s, 4)?, rd_f32(s, 8)?)) }
#[inline] fn read_quat(s: &[u8]) -> Option<Quat> { Some(Quat::from_xyzw(rd_f32(s, 0)?, rd_f32(s, 4)?, rd_f32(s, 8)?, rd_f32(s, 12)?)) }
#[inline] fn read_mat4(s: &[u8]) -> Option<Mat4> {
    let mut a = [0.0f32; 16];
    for i in 0..16 { a[i] = rd_f32(s, i * 4)?; }
    Some(Mat4::from_cols_array(&a))
}

#[inline] fn obj_u8_req(b: &[u8], obj: usize, voff: usize) -> Option<u8> { vtbl_off(b, obj, voff).and_then(|p| b.get(p).copied()) }
#[inline] fn obj_f32_req(b: &[u8], obj: usize, voff: usize) -> Option<f32> { vtbl_off(b, obj, voff).and_then(|p| rd_f32(b, p)) }

#[no_mangle]
pub extern "C" fn cunning_spline_snapshot_fbs_to_geo(fbs_ptr: *const u8, fbs_len: u32, source_basis: u32) -> Handle {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if fbs_ptr.is_null() || fbs_len < 8 { return None; }
        let b = unsafe { std::slice::from_raw_parts(fbs_ptr, fbs_len as usize) };
        let root = rd_u32(b, 0)? as usize;
        if root >= b.len() { return None; }

        let l2w0 = read_mat4(obj_struct(b, root, 6, 64)?)?;
        let map = if source_basis == 1 { cunning_kernel::coord::basis::map(cunning_kernel::coord::basis::BasisId::Unity, cunning_kernel::coord::basis::BasisId::InternalBevy) } else { None };
        let l2w = map.map(|m| m.map_m4(l2w0)).unwrap_or(l2w0);

        let (sv_base, sv_len) = obj_vec(b, root, 8)?;
        let lv = obj_vec(b, root, 10);
        let mut geo = Geometry::new();
        let mut all_p: Vec<Vec3> = Vec::new();
        let mut all_tin: Vec<Vec3> = Vec::new();
        let mut all_tout: Vec<Vec3> = Vec::new();
        let mut all_rot: Vec<Quat> = Vec::new();
        let mut all_mode: Vec<i32> = Vec::new();
        let mut all_tension: Vec<f32> = Vec::new();
        let mut all_link: Vec<i32> = Vec::new();
        let has_links = lv.is_some();

        // Preserve exact homologous snapshot bytes (post basis transform) for round-trip export back to Unity.
        geo.set_detail_attribute("__cunning.spline_snapshot.fbs", cunning_kernel::libs::geometry::mesh::Bytes(b.to_vec()));
        geo.set_detail_attribute("__cunning.spline_snapshot.basis", vec![(if source_basis == 1 { "Unity" } else { "InternalBevy" }).to_string()]);
        geo.set_detail_attribute("__cunning.spline_snapshot.basis_to_internal", vec![source_basis == 1]);

        // Build link id map: (spline,knot) -> link_id (dense by discovery order, deterministic by group sort).
        let mut link_id_by_key: HashMap<(i32, i32), i32> = HashMap::new();
        if let Some((lv_base, lv_len)) = lv {
            for gi in 0..lv_len {
                let g = vec_tbl_at(b, lv_base, gi)?;
                let (kv, kl) = obj_vec(b, g, 4)?;
                if kl < 2 { continue; }
                let link_id = gi as i32;
                for ki in 0..kl {
                    let s = b.get(kv + ki * 8..kv + ki * 8 + 8)?;
                    let Some(sp) = rd_i32(s, 0) else { continue; };
                    let Some(kn) = rd_i32(s, 4) else { continue; };
                    link_id_by_key.insert((sp, kn), link_id);
                }
            }
        }

        for si in 0..sv_len {
            let spline = vec_tbl_at(b, sv_base, si)?;
        let closed = obj_u8_req(b, spline, 4)? != 0;
            let (kv_base, kv_len) = obj_vec(b, spline, 6)?;
            let mut curve_verts = Vec::with_capacity(kv_len);

            for ki in 0..kv_len {
                let knot = vec_tbl_at(b, kv_base, ki)?;
                let mut p = read_vec3(obj_struct(b, knot, 4, 12)?)?;
                let mut tin = read_vec3(obj_struct(b, knot, 6, 12)?)?;
                let mut tout = read_vec3(obj_struct(b, knot, 8, 12)?)?;
                let mut q = read_quat(obj_struct(b, knot, 10, 16)?)?;
                if let Some(m) = map { p = m.map_v3(p); tin = m.map_v3(tin); tout = m.map_v3(tout); q = m.map_q(q); }

                // Apply local_to_world (position + tangent vectors) while keeping tangents expressed in knot-local space.
                let (_s, mrot, _t) = l2w.to_scale_rotation_translation();
                let rot = mrot * q;
                let inv = rot.inverse();
                let tin_w = q.mul_vec3(tin);
                let tout_w = q.mul_vec3(tout);
                let kp = cunning_kernel::libs::geometry::mesh::BezierKnot {
                    position: l2w.transform_point3(p),
                    tangent_in: inv.mul_vec3(l2w.transform_vector3(tin_w)),
                    tangent_out: inv.mul_vec3(l2w.transform_vector3(tout_w)),
                    rotation: rot,
                };

                let mode = match obj_u8_req(b, knot, 12)? {
                    0 => cunning_kernel::libs::geometry::mesh::TangentMode::AutoSmooth,
                    1 => cunning_kernel::libs::geometry::mesh::TangentMode::Linear,
                    2 => cunning_kernel::libs::geometry::mesh::TangentMode::Mirrored,
                    3 => cunning_kernel::libs::geometry::mesh::TangentMode::Continuous,
                    _ => cunning_kernel::libs::geometry::mesh::TangentMode::Broken,
                };
                let tension = obj_f32_req(b, knot, 14)?;
                if has_links {
                    // -1 explicitly means "unlinked"
                    all_link.push(*link_id_by_key.get(&(si as i32, ki as i32)).unwrap_or(&-1));
                }

                let pid = geo.add_point();
                all_p.push(kp.position);
                all_tin.push(kp.tangent_in);
                all_tout.push(kp.tangent_out);
                all_rot.push(kp.rotation);
                all_mode.push(match mode { cunning_kernel::libs::geometry::mesh::TangentMode::AutoSmooth => 0, cunning_kernel::libs::geometry::mesh::TangentMode::Linear => 1, cunning_kernel::libs::geometry::mesh::TangentMode::Mirrored => 2, cunning_kernel::libs::geometry::mesh::TangentMode::Continuous => 3, cunning_kernel::libs::geometry::mesh::TangentMode::Broken => 4 });
                all_tension.push(tension);
                let vid = geo.add_vertex(pid);
                curve_verts.push(vid);
            }

            geo.add_primitive(GeoPrimitive::BezierCurve(BezierCurvePrim { vertices: curve_verts, closed }));
        }

        geo.insert_point_attribute(attrs::P, Attribute::new(all_p));
        geo.insert_point_attribute(cunning_kernel::libs::geometry::attrs::KNOT_TIN, Attribute::new(all_tin));
        geo.insert_point_attribute(cunning_kernel::libs::geometry::attrs::KNOT_TOUT, Attribute::new(all_tout));
        geo.insert_point_attribute(cunning_kernel::libs::geometry::attrs::KNOT_ROT, Attribute::new(all_rot));
        geo.insert_point_attribute(cunning_kernel::libs::geometry::attrs::KNOT_MODE, Attribute::new(all_mode));
        geo.insert_point_attribute(cunning_kernel::libs::geometry::attrs::KNOT_TENSION, Attribute::new(all_tension));
        if has_links { geo.insert_point_attribute(cunning_kernel::libs::geometry::attrs::KNOT_LINK_ID, Attribute::new(all_link)); }
        geo.dirty_id = new_dirty_id();
        Some(register_geo(geo))
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_geo_create() -> Handle {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut geo = Geometry::new();
        geo.insert_point_attribute(attrs::P, Attribute::new(Vec::<Vec3>::new()));
        let geo = Arc::new(geo);
        let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        { let mut reg = get_geo_registry().lock().unwrap(); reg.insert(handle, geo.clone()); }
        { let mut caches = get_cache_map().lock().unwrap(); let mut c = ExportCache::default(); c.sync(&geo); caches.insert(handle, c); }
        handle
    }));
    result.unwrap_or(0)
}

fn register_geo(geo: Geometry) -> Handle {
    let geo = Arc::new(geo);
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    { let mut reg = get_geo_registry().lock().unwrap(); reg.insert(handle, geo.clone()); }
    { let mut caches = get_cache_map().lock().unwrap(); let mut c = ExportCache::default(); c.sync(&geo); caches.insert(handle, c); }
    handle
}

#[no_mangle]
pub extern "C" fn cunning_geo_create_cube(size: f32, div_x: u32, div_y: u32, div_z: u32) -> Handle {
    let result = catch_unwind(AssertUnwindSafe(|| register_geo(primitives::create_cube(size, bevy::math::UVec3::new(div_x, div_y, div_z)))));
    result.unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_geo_create_sphere(radius: f32, rings: u32, segments: u32) -> Handle {
    let result = catch_unwind(AssertUnwindSafe(|| register_geo(primitives::create_sphere(radius, rings as usize, segments as usize))));
    result.unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_geo_add_point(handle: Handle, x: f32, y: f32, z: f32) -> u64 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut reg = get_geo_registry().lock().unwrap();
        let geo = reg.get_mut(&handle)?;
        let g = Arc::make_mut(geo);
        if g.get_point_attribute(attrs::P).is_none() { g.insert_point_attribute(attrs::P, Attribute::new(Vec::<Vec3>::new())); }
        let pid = g.add_point();
        let dense = g.points().get_dense_index(pid.into())?;
        if let Some(a) = g.get_point_attribute_mut(attrs::P) {
            if let Some(s) = a.as_mut_slice::<Vec3>() { if dense < s.len() { s[dense] = Vec3::new(x, y, z); } }
            else if let Some(pb) = a.as_paged_mut::<Vec3>() { if let Some(v) = pb.get_mut(dense) { *v = Vec3::new(x, y, z); } }
        }
        g.dirty_id = new_dirty_id();
        drop(reg);
        { let reg = get_geo_registry().lock().unwrap(); let geo = reg.get(&handle)?; let mut caches = get_cache_map().lock().unwrap(); if let Some(c) = caches.get_mut(&handle) { c.sync(geo); } }
        Some(pack_point(pid))
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_geo_add_poly(handle: Handle, point_ids: *const u64, count: u32) -> u64 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if point_ids.is_null() || count < 3 { return None; }
        let ids = unsafe { std::slice::from_raw_parts(point_ids, count as usize) };
        let mut reg = get_geo_registry().lock().unwrap();
        let geo = reg.get_mut(&handle)?;
        let g = Arc::make_mut(geo);
        let mut verts = Vec::with_capacity(ids.len());
        for &pid in ids.iter() { verts.push(g.add_vertex(unpack_point(pid))); }
        let prim = g.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: verts }));
        g.dirty_id = new_dirty_id();
        drop(reg);
        { let reg = get_geo_registry().lock().unwrap(); let geo = reg.get(&handle)?; let mut caches = get_cache_map().lock().unwrap(); if let Some(c) = caches.get_mut(&handle) { c.sync(geo); } }
        Some(pack_prim(prim))
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_load_graph_asset(path: *const c_char) -> u32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let c_str = unsafe { CStr::from_ptr(path) };
        let path_str = c_str.to_str().ok()?;
        let json = std::fs::read_to_string(path_str).ok()?;
        let asset: GraphAsset = serde_json::from_str(&json).ok()?;
        let mut ctx = get_engine_context().lock().unwrap();
        ctx.asset = Some(asset);
        Some(1u32)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_cook_async() {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let mut ctx = get_engine_context().lock().unwrap();
        let Some(asset) = ctx.asset.clone() else { return; };
        if let Some(out) = cook_asset(&asset) { ctx.output_geo = out; }
    }));
}

#[no_mangle]
pub extern "C" fn cunning_get_output_geo() -> Handle {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let ctx = get_engine_context().lock().unwrap();
        let geo = ctx.output_geo.clone();
        let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        {
            let mut caches = get_cache_map().lock().unwrap();
            let mut cache = ExportCache::default();
            cache.sync(&geo);
            caches.insert(handle, cache);
        }
        {
            let mut reg = get_geo_registry().lock().unwrap();
            reg.insert(handle, geo);
        }
        handle
    }));
    result.unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_op_boolean(handle_a: Handle, handle_b: Handle, op_type: u32) -> Handle {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let reg = get_geo_registry().lock().unwrap();
        let geo_a = reg.get(&handle_a)?;
        let geo_b = reg.get(&handle_b)?;

        let mut params: HashMap<String, ParameterValue> = HashMap::new();
        params.insert("operation".to_string(), ParameterValue::Int(op_type as i32));
        params.insert("tolerance".to_string(), ParameterValue::Float(1e-4));
        let inputs = vec![geo_a.clone(), geo_b.clone()];
        let result_arc = Arc::new(cunning_kernel::nodes::modeling::boolean::compute_boolean(&inputs, &params));

        drop(reg);
        let out_handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        {
            let mut reg = get_geo_registry().lock().unwrap();
            reg.insert(out_handle, result_arc.clone());
        }
        {
            let mut caches = get_cache_map().lock().unwrap();
            let mut cache = ExportCache::default();
            cache.sync(&result_arc);
            caches.insert(out_handle, cache);
        }
        Some(out_handle)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_op_poly_extrude(handle: Handle, distance: f32, inset: f32) -> Handle {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let reg = get_geo_registry().lock().unwrap();
        let geo = reg.get(&handle)?;

        use cunning_kernel::cunning_core::traits::node_interface::NodeOp;
        use cunning_kernel::nodes::modeling::poly_extrude::PolyExtrudeNode;
        use cunning_kernel::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};

        let params = vec![
            Parameter::new("distance", "", "", ParameterValue::Float(distance), ParameterUIType::FloatSlider { min: -1000.0, max: 1000.0 }),
            Parameter::new("inset", "", "", ParameterValue::Float(inset), ParameterUIType::FloatSlider { min: -1000.0, max: 1000.0 }),
            Parameter::new("divisions", "", "", ParameterValue::Int(1), ParameterUIType::IntSlider { min: 1, max: 128 }),
            Parameter::new("output_front", "", "", ParameterValue::Bool(true), ParameterUIType::Toggle),
            Parameter::new("output_back", "", "", ParameterValue::Bool(false), ParameterUIType::Toggle),
            Parameter::new("output_side", "", "", ParameterValue::Bool(true), ParameterUIType::Toggle),
        ];

        let inputs = vec![geo.clone()];
        let result_arc = PolyExtrudeNode::default().compute(&params, &inputs);

        drop(reg);
        let out_handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        {
            let mut reg = get_geo_registry().lock().unwrap();
            reg.insert(out_handle, result_arc.clone());
        }
        {
            let mut caches = get_cache_map().lock().unwrap();
            let mut cache = ExportCache::default();
            cache.sync(&result_arc);
            caches.insert(out_handle, cache);
        }
        Some(out_handle)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_op_poly_extrude_prim(handle: Handle, prim_index: u32, distance: f32, inset: f32, divisions: u32) -> Handle {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let reg = get_geo_registry().lock().unwrap();
        let geo = reg.get(&handle)?;

        use cunning_kernel::cunning_core::traits::node_interface::NodeOp;
        use cunning_kernel::nodes::modeling::poly_extrude::PolyExtrudeNode;
        use cunning_kernel::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};

        let params = vec![
            Parameter::new("group", "", "", ParameterValue::String(prim_index.to_string()), ParameterUIType::String),
            Parameter::new("split", "", "", ParameterValue::Int(0), ParameterUIType::Dropdown { choices: vec![("Connected Components".into(), 0), ("Individual Elements".into(), 1)] }),
            Parameter::new("distance", "", "", ParameterValue::Float(distance), ParameterUIType::FloatSlider { min: -1000.0, max: 1000.0 }),
            Parameter::new("inset", "", "", ParameterValue::Float(inset), ParameterUIType::FloatSlider { min: -1000.0, max: 1000.0 }),
            Parameter::new("divisions", "", "", ParameterValue::Int(divisions.max(1) as i32), ParameterUIType::IntSlider { min: 1, max: 128 }),
            Parameter::new("output_front", "", "", ParameterValue::Bool(true), ParameterUIType::Toggle),
            Parameter::new("output_back", "", "", ParameterValue::Bool(false), ParameterUIType::Toggle),
            Parameter::new("output_side", "", "", ParameterValue::Bool(true), ParameterUIType::Toggle),
            Parameter::new("front_grp", "", "", ParameterValue::String("extrudeFront".into()), ParameterUIType::String),
            Parameter::new("back_grp", "", "", ParameterValue::String("extrudeBack".into()), ParameterUIType::String),
            Parameter::new("side_grp", "", "", ParameterValue::String("extrudeSide".into()), ParameterUIType::String),
        ];

        let inputs = vec![geo.clone()];
        let result_arc = PolyExtrudeNode::default().compute(&params, &inputs);

        drop(reg);
        let out_handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        {
            let mut reg = get_geo_registry().lock().unwrap();
            reg.insert(out_handle, result_arc.clone());
        }
        {
            let mut caches = get_cache_map().lock().unwrap();
            let mut cache = ExportCache::default();
            cache.sync(&result_arc);
            caches.insert(out_handle, cache);
        }
        Some(out_handle)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_op_poly_bevel(handle: Handle, distance: f32, divisions: u32) -> Handle {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let reg = get_geo_registry().lock().unwrap();
        let geo = reg.get(&handle)?;

        use cunning_kernel::cunning_core::traits::node_interface::NodeOp;
        use cunning_kernel::nodes::modeling::poly_bevel::PolyBevelNode;
        use cunning_kernel::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};

        let params = vec![
            Parameter::new("group", "", "", ParameterValue::String("".into()), ParameterUIType::String),
            Parameter::new("affect", "", "", ParameterValue::Int(0), ParameterUIType::Dropdown { choices: vec![("Edges".into(), 0), ("Vertices".into(), 1)] }),
            Parameter::new("offset_type", "", "", ParameterValue::Int(0), ParameterUIType::Dropdown { choices: vec![("Offset".into(), 0), ("Width".into(), 1), ("Depth".into(), 2), ("Percent".into(), 3), ("Absolute".into(), 4)] }),
            Parameter::new("distance", "", "", ParameterValue::Float(distance), ParameterUIType::FloatSlider { min: 0.0, max: 1000.0 }),
            Parameter::new("divisions", "", "", ParameterValue::Int(divisions.max(1) as i32), ParameterUIType::IntSlider { min: 1, max: 16 }),
        ];

        let inputs = vec![geo.clone()];
        let result_arc = PolyBevelNode::default().compute(&params, &inputs);

        drop(reg);
        let out_handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        {
            let mut reg = get_geo_registry().lock().unwrap();
            reg.insert(out_handle, result_arc.clone());
        }
        {
            let mut caches = get_cache_map().lock().unwrap();
            let mut cache = ExportCache::default();
            cache.sync(&result_arc);
            caches.insert(out_handle, cache);
        }
        Some(out_handle)
    }));
    result.unwrap_or(None).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_geo_get_point_count(handle: Handle) -> u32 {
    let caches = get_cache_map().lock().unwrap();
    caches.get(&handle).map(|c| c.point_positions.len() as u32).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_geo_get_vertex_count(handle: Handle) -> u32 {
    let caches = get_cache_map().lock().unwrap();
    caches.get(&handle).map(|c| c.positions.len() as u32).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_geo_get_prim_count(handle: Handle) -> u32 {
    let caches = get_cache_map().lock().unwrap();
    caches.get(&handle).map(|c| c.prim_ids.len() as u32).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_geo_copy_points(handle: Handle, out_ptr: *mut f32) {
    let caches = get_cache_map().lock().unwrap();
    if let Some(c) = caches.get(&handle) {
        unsafe { if !out_ptr.is_null() { std::ptr::copy_nonoverlapping(c.point_positions.as_ptr() as *const f32, out_ptr, c.point_positions.len() * 3); } }
    }
}

#[no_mangle]
pub extern "C" fn cunning_geo_copy_vertices(handle: Handle, out_ptr: *mut f32) {
    let caches = get_cache_map().lock().unwrap();
    if let Some(c) = caches.get(&handle) {
        unsafe { if !out_ptr.is_null() { std::ptr::copy_nonoverlapping(c.positions.as_ptr() as *const f32, out_ptr, c.positions.len() * 3); } }
    }
}

#[no_mangle]
pub extern "C" fn cunning_geo_copy_indices(handle: Handle, out_ptr: *mut u32) -> u32 {
    let caches = get_cache_map().lock().unwrap();
    if let Some(c) = caches.get(&handle) {
        unsafe { if !out_ptr.is_null() { std::ptr::copy_nonoverlapping(c.tri_indices.as_ptr(), out_ptr, c.tri_indices.len()); } }
        c.tri_indices.len() as u32
    } else { 0 }
}

#[no_mangle]
pub extern "C" fn cunning_geo_copy_lines(handle: Handle, out_ptr: *mut u32) -> u32 {
    let caches = get_cache_map().lock().unwrap();
    if let Some(c) = caches.get(&handle) {
        unsafe { if !out_ptr.is_null() { std::ptr::copy_nonoverlapping(c.line_indices.as_ptr(), out_ptr, c.line_indices.len()); } }
        c.line_indices.len() as u32
    } else { 0 }
}

#[no_mangle]
pub extern "C" fn cunning_geo_get_dirty_id(handle: Handle) -> u64 {
    let caches = get_cache_map().lock().unwrap();
    caches.get(&handle).map(|c| c.dirty_id).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_geo_copy_prim_point_offsets(handle: Handle, out_ptr: *mut u32) -> u32 {
    let caches = get_cache_map().lock().unwrap();
    if let Some(c) = caches.get(&handle) {
        unsafe { if !out_ptr.is_null() { std::ptr::copy_nonoverlapping(c.prim_point_offsets.as_ptr(), out_ptr, c.prim_point_offsets.len()); } }
        c.prim_point_offsets.len() as u32
    } else { 0 }
}

#[no_mangle]
pub extern "C" fn cunning_geo_copy_prim_point_indices(handle: Handle, out_ptr: *mut u32) -> u32 {
    let caches = get_cache_map().lock().unwrap();
    if let Some(c) = caches.get(&handle) {
        unsafe { if !out_ptr.is_null() { std::ptr::copy_nonoverlapping(c.prim_point_indices.as_ptr(), out_ptr, c.prim_point_indices.len()); } }
        c.prim_point_indices.len() as u32
    } else { 0 }
}

#[no_mangle]
pub extern "C" fn cunning_release_handle(handle: Handle) {
    { let mut caches = get_cache_map().lock().unwrap(); caches.remove(&handle); }
    { let mut reg = get_geo_registry().lock().unwrap(); reg.remove(&handle); }
}

// --- Zstd (Binary Transport) ---
// Two-phase API (Unity-friendly, no native allocations):
// 1) get bound: allocate managed byte[] of that size
// 2) compress into provided buffer, return actual bytes written (0 = error)

#[no_mangle]
pub extern "C" fn cunning_zstd_compress_bound(src_len: u32) -> u32 {
    zstd_safe::compress_bound(src_len as usize).min(u32::MAX as usize) as u32
}

#[no_mangle]
pub extern "C" fn cunning_zstd_compress(src_ptr: *const u8, src_len: u32, level: i32, out_ptr: *mut u8, out_cap: u32) -> u32 {
    if src_ptr.is_null() || out_ptr.is_null() { return 0; }
    let src = unsafe { std::slice::from_raw_parts(src_ptr, src_len as usize) };
    let dst = unsafe { std::slice::from_raw_parts_mut(out_ptr, out_cap as usize) };
    match zstd::bulk::compress_to_buffer(src, dst, level) {
        Ok(n) => n.min(u32::MAX as usize) as u32,
        Err(_) => 0,
    }
}



// --- Spline Snapshot (JSON -> FlatBuffers) ---

#[no_mangle]
pub extern "C" fn cunning_spline_snapshot_json_to_fbs(json_ptr: *const u8, json_len: u32) -> u64 {
    if json_ptr.is_null() { return 0; }
    let json = unsafe { std::slice::from_raw_parts(json_ptr, json_len as usize) };
    let snap: cunning_kernel::algorithms::algorithms_runtime::unity_spline::editor::harness::SplineContainerSnapshot = match serde_json::from_slice(json) { Ok(v) => v, Err(_) => return 0 };
    let bytes = cunning_kernel::spline_snapshot_fbs::encode_snapshot_fbs(&snap);
    if let Some(store) = session_store() { store.insert_alloc(&bytes).ok().unwrap_or(0) } else { let id = NEXT_BLOB_ID.fetch_add(1, Ordering::Relaxed); blob_reg().lock().unwrap().insert(id, bytes); id }
}

// --- Spline Snapshot (JSON -> FlatBuffers -> Zstd) ---

#[no_mangle]
pub extern "C" fn cunning_blob_get_size(blob_handle: u64) -> u32 {
    if let Some(store) = session_store() { return store.get(blob_handle).ok().flatten().map(|b| b.len().min(u32::MAX as usize) as u32).unwrap_or(0); }
    blob_reg().lock().unwrap().get(&blob_handle).map(|b| b.len().min(u32::MAX as usize) as u32).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn cunning_blob_copy(blob_handle: u64, out_ptr: *mut u8, out_cap: u32) -> u32 {
    if out_ptr.is_null() { return 0; }
    let b = if let Some(store) = session_store() { match store.get(blob_handle).ok().flatten() { Some(v) => v, None => return 0 } }
    else { match blob_reg().lock().unwrap().get(&blob_handle).cloned() { Some(v) => v, None => return 0 } };
    let n = b.len().min(out_cap as usize);
    unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), out_ptr, n); }
    n.min(u32::MAX as usize) as u32
}

#[no_mangle]
pub extern "C" fn cunning_blob_release(blob_handle: u64) {
    if session_store().is_some() { return; }
    blob_reg().lock().unwrap().remove(&blob_handle);
}

#[no_mangle]
pub extern "C" fn cunning_spline_snapshot_json_to_fbs_zstd(json_ptr: *const u8, json_len: u32, zstd_level: i32) -> u64 {
    if json_ptr.is_null() { return 0; }
    let json = unsafe { std::slice::from_raw_parts(json_ptr, json_len as usize) };
    let snap: cunning_kernel::algorithms::algorithms_runtime::unity_spline::editor::harness::SplineContainerSnapshot = match serde_json::from_slice(json) { Ok(v) => v, Err(_) => return 0 };
    let bytes = cunning_kernel::spline_snapshot_fbs::encode_snapshot_fbs_zstd(&snap, zstd_level);
    if let Some(store) = session_store() { store.insert_alloc(&bytes).ok().unwrap_or(0) } else { let id = NEXT_BLOB_ID.fetch_add(1, Ordering::Relaxed); blob_reg().lock().unwrap().insert(id, bytes); id }
}
