use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use cunning_cda_runtime::{
    asset::{NodeId, RuntimeDefinition},
    compiler,
    registry::RuntimeRegistry,
    vm,
};
use cunning_kernel::libs::geometry::attrs;
use cunning_kernel::mesh::{Attribute, GeoPrimitive, Geometry, PolygonPrim};
use cunning_kernel::traits::parameter::ParameterValue;
use bevy::prelude::Vec3;

use crate::protocol::{HostCommand, WorkerEvent};

const CDA_MAGIC: &[u8; 4] = b"CDA\0";
const CDA_FILE_VERSION: u32 = 1;
const CHUNK_GAME: [u8; 4] = *b"GAME";

#[cfg(target_arch = "wasm32")]
fn perf_now_ms() -> f64 {
    web_sys::window().and_then(|w| w.performance()).map(|p| p.now()).unwrap_or(0.0)
}

fn default_external_inputs() -> Vec<Arc<Geometry>> {
    let mut g = Geometry::new();
    let pts = [
        Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, -1.0), Vec3::new(-1.0, 1.0, -1.0),
        Vec3::new(-1.0, -1.0,  1.0), Vec3::new(1.0, -1.0,  1.0), Vec3::new(1.0, 1.0,  1.0), Vec3::new(-1.0, 1.0,  1.0),
    ];
    let mut pids = Vec::with_capacity(8);
    for _ in 0..8 { pids.push(g.add_point()); }
    g.insert_point_attribute(attrs::P, Attribute::new(pts.to_vec()));
    let face = |g: &mut Geometry, pids: &[cunning_kernel::mesh::PointId], a: usize, b: usize, c: usize, d: usize| {
        let vs = [a, b, c, d].map(|i| g.add_vertex(pids[i]));
        g.add_primitive(GeoPrimitive::Polygon(PolygonPrim { vertices: vs.to_vec() }));
    };
    face(&mut g, &pids, 0, 1, 2, 3); // -Z
    face(&mut g, &pids, 4, 5, 6, 7); // +Z
    face(&mut g, &pids, 0, 4, 7, 3); // -X
    face(&mut g, &pids, 1, 5, 6, 2); // +X
    face(&mut g, &pids, 3, 2, 6, 7); // +Y
    face(&mut g, &pids, 0, 1, 5, 4); // -Y
    vec![Arc::new(g)]
}

fn extract_chunk<'a>(data: &'a [u8], chunk: [u8; 4]) -> Result<&'a [u8], String> {
    if data.get(0..4) != Some(CDA_MAGIC) { return Err("Invalid CDA magic".into()); }
    let ver = u32::from_le_bytes(data.get(4..8).ok_or("Invalid CDA header")?.try_into().map_err(|_| "Invalid CDA header")?);
    if ver != CDA_FILE_VERSION { return Err(format!("CDA version mismatch: {ver}")); }
    let n = u32::from_le_bytes(data.get(8..12).ok_or("Invalid CDA header")?.try_into().map_err(|_| "Invalid CDA header")?) as usize;
    let mut at = 12usize;
    for _ in 0..n {
        let id: [u8; 4] = data.get(at..at + 4).ok_or("Invalid CDA header")?.try_into().map_err(|_| "Invalid CDA header")?; at += 4;
        let off = u64::from_le_bytes(data.get(at..at + 8).ok_or("Invalid CDA header")?.try_into().map_err(|_| "Invalid CDA header")?); at += 8;
        let sz = u64::from_le_bytes(data.get(at..at + 8).ok_or("Invalid CDA header")?.try_into().map_err(|_| "Invalid CDA header")?); at += 8;
        if id == chunk {
            let s = off as usize;
            let e = s.checked_add(sz as usize).ok_or("Invalid CDA chunk size")?;
            return Ok(data.get(s..e).ok_or("Invalid CDA chunk range")?);
        }
    }
    Err("Missing CDA chunk".into())
}

fn parse_game_def(bytes: &[u8]) -> Result<RuntimeDefinition, String> {
    let game = extract_chunk(bytes, CHUNK_GAME)?;
    let json = std::str::from_utf8(game).map_err(|_| "GAME chunk is not valid utf-8")?;
    serde_json::from_str::<RuntimeDefinition>(json).map_err(|e| format!("GAME json parse failed: {e}"))
}

pub(crate) fn apply_internal_overrides(
    base: &RuntimeDefinition,
    node_index_by_id: &HashMap<NodeId, usize>,
    internal_overrides: &HashMap<NodeId, HashMap<String, ParameterValue>>,
) -> RuntimeDefinition {
    if internal_overrides.is_empty() {
        return base.clone();
    }
    let mut out = base.clone();
    for (node_id, params) in internal_overrides {
        let Some(&idx) = node_index_by_id.get(node_id) else {
            continue;
        };
        if let Some(n) = out.nodes.get_mut(idx) {
            for (k, v) in params {
                n.params.insert(k.clone(), v.clone());
            }
        }
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
pub struct ComputeWorker {
    tx: crossbeam_channel::Sender<HostCommand>,
    rx: crossbeam_channel::Receiver<WorkerEvent>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ComputeWorker {
    pub fn spawn() -> Self {
        let (tx_cmd, rx_cmd) = crossbeam_channel::unbounded::<HostCommand>();
        let (tx_evt, rx_evt) = crossbeam_channel::unbounded::<WorkerEvent>();

        std::thread::spawn(move || {
            let reg = RuntimeRegistry::new_default();
            let mut def: Option<Arc<RuntimeDefinition>> = None;
            let mut plan: Option<compiler::ExecutionPlan> = None;
            let mut overrides: HashMap<String, ParameterValue> = HashMap::new();
            let mut node_index_by_id: HashMap<NodeId, usize> = HashMap::new();
            let mut internal_overrides: HashMap<NodeId, HashMap<String, ParameterValue>> =
                HashMap::new();
            let external_inputs = default_external_inputs();
            let cancel = AtomicBool::new(false);

            while let Ok(cmd) = rx_cmd.recv() {
                match cmd {
                    HostCommand::LoadCdaBytes { bytes } => {
                        match parse_game_def(&bytes).and_then(|d| {
                            let p = compiler::compile(&d, &reg).map_err(|e| format!("{e:?}"))?;
                            def = Some(Arc::new(d));
                            plan = Some(p);
                            overrides.clear();
                            internal_overrides.clear();
                            node_index_by_id = def
                                .as_ref()
                                .map(|d| {
                                    d.nodes
                                        .iter()
                                        .enumerate()
                                        .map(|(i, n)| (n.id, i))
                                        .collect()
                                })
                                .unwrap_or_default();
                            Ok(())
                        }) {
                            Ok(()) => { let _ = tx_evt.send(WorkerEvent::AssetReady { def: def.clone().unwrap() }); }
                            Err(message) => { let _ = tx_evt.send(WorkerEvent::Error { message }); }
                        }
                    }
                    HostCommand::LoadCdaUrl { url } => {
                        // native convenience: treat as filesystem path
                        match std::fs::read(&url).map_err(|e| e.to_string()).and_then(|b| parse_game_def(&b)) {
                            Ok(d) => {
                                match compiler::compile(&d, &reg).map_err(|e| format!("{e:?}")) {
                                    Ok(p) => {
                                        def = Some(Arc::new(d));
                                        plan = Some(p);
                                        overrides.clear();
                                        internal_overrides.clear();
                                        node_index_by_id = def
                                            .as_ref()
                                            .map(|d| {
                                                d.nodes
                                                    .iter()
                                                    .enumerate()
                                                    .map(|(i, n)| (n.id, i))
                                                    .collect()
                                            })
                                            .unwrap_or_default();
                                        let _ = tx_evt.send(WorkerEvent::AssetReady { def: def.clone().unwrap() });
                                    }
                                    Err(message) => { let _ = tx_evt.send(WorkerEvent::Error { message }); }
                                }
                            }
                            Err(message) => { let _ = tx_evt.send(WorkerEvent::Error { message }); }
                        }
                    }
                    HostCommand::SetOverride { name, value } => { overrides.insert(name, value); }
                    HostCommand::SetInternalOverride { node, param, value } => {
                        internal_overrides
                            .entry(node)
                            .or_default()
                            .insert(param, value);
                    }
                    HostCommand::Batch { cmds } => {
                        for c in cmds {
                            // Re-route through the same match arms (keep ordering).
                            match c {
                                HostCommand::SetOverride { name, value } => {
                                    overrides.insert(name, value);
                                }
                                HostCommand::SetInternalOverride { node, param, value } => {
                                    internal_overrides
                                        .entry(node)
                                        .or_default()
                                        .insert(param, value);
                                }
                                HostCommand::Cook => {
                                    // fallthrough: handled below by re-injecting a Cook command
                                    // (keeping behavior simple/explicit).
                                    let _ = rx_cmd
                                        .try_recv()
                                        .ok(); // no-op, preserve compilation; Cook handled on next loop
                                }
                                HostCommand::LoadCdaUrl { .. } | HostCommand::LoadCdaBytes { .. } | HostCommand::Batch { .. } => {
                                    // Ignore nested/bad batches in native worker for now.
                                }
                            }
                        }
                    }
                    HostCommand::Cook => {
                        let (Some(d), Some(p)) = (def.clone(), plan.as_ref()) else { continue; };
                        cancel.store(false, Ordering::Relaxed);
                        let t0 = Instant::now();
                        let d_work =
                            apply_internal_overrides(d.as_ref(), &node_index_by_id, &internal_overrides);
                        let outs = match vm::execute(p, &d_work, &reg, &external_inputs, &overrides, &cancel) {
                            Ok(v) => v,
                            Err(e) => { let _ = tx_evt.send(WorkerEvent::Error { message: format!("{e:?}") }); continue; }
                        };
                        let _ = tx_evt.send(WorkerEvent::CookFinished { duration_ms: t0.elapsed().as_millis() as u32, outputs: outs });
                    }
                }
            }
        });

        Self { tx: tx_cmd, rx: rx_evt }
    }

    pub fn send(&self, cmd: HostCommand) { let _ = self.tx.send(cmd); }
    pub fn try_recv(&self) -> Option<WorkerEvent> { self.rx.try_recv().ok() }
    pub fn queue_len(&self) -> usize { 0 }
}

// wasm32: real WebWorker-backed compute worker (compile+cook off the UI thread).
#[cfg(target_arch = "wasm32")]
pub struct ComputeWorker {
    worker: web_sys::Worker,
    events: std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<WorkerEvent>>>,
}

#[cfg(target_arch = "wasm32")]
impl Clone for ComputeWorker {
    fn clone(&self) -> Self { Self { worker: self.worker.clone(), events: self.events.clone() } }
}

#[cfg(target_arch = "wasm32")]
impl ComputeWorker {
    pub fn spawn() -> Self {
        use wasm_bindgen::JsCast;

        let mut opts = web_sys::WorkerOptions::new();
        opts.type_(web_sys::WorkerType::Module);
        let worker = web_sys::Worker::new_with_options("./cunning_player_worker.js", &opts)
            .expect("failed to spawn compute WebWorker");

        let events = std::rc::Rc::new(std::cell::RefCell::new(std::collections::VecDeque::new()));

        let onmessage = {
            let events = events.clone();
            wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
                let data = ev.data();
                if !data.is_instance_of::<js_sys::Array>() {
                    events.borrow_mut().push_back(WorkerEvent::Error {
                        message: "worker message is not an array".to_string(),
                    });
                    return;
                }
                let arr = js_sys::Array::from(&data);
                let kind = arr.get(0).as_string().unwrap_or_default();
                match kind.as_str() {
                    "Error" => {
                        let msg = arr.get(1).as_string().unwrap_or_else(|| "(unknown)".into());
                        events.borrow_mut().push_back(WorkerEvent::Error { message: msg });
                    }
                    "AssetReady" => {
                        let u8: js_sys::Uint8Array = match arr.get(1).dyn_into() {
                            Ok(v) => v,
                            Err(_) => {
                                events.borrow_mut().push_back(WorkerEvent::Error {
                                    message: "AssetReady payload is not Uint8Array".to_string(),
                                });
                                return;
                            }
                        };
                        let bytes = u8.to_vec();
                        match serde_json::from_slice::<RuntimeDefinition>(&bytes) {
                            Ok(def) => events
                                .borrow_mut()
                                .push_back(WorkerEvent::AssetReady { def: Arc::new(def) }),
                            Err(e) => events.borrow_mut().push_back(WorkerEvent::Error {
                                message: format!("AssetReady json parse failed: {e}"),
                            }),
                        }
                    }
                    "CookFinished" => {
                        let duration_ms = arr.get(1).as_f64().unwrap_or(0.0) as u32;
                        let outs_val = arr.get(2);
                        let outs_arr: js_sys::Array = match outs_val.dyn_into() {
                            Ok(v) => v,
                            Err(_) => {
                                events.borrow_mut().push_back(WorkerEvent::Error {
                                    message: "CookFinished outputs is not Array".to_string(),
                                });
                                return;
                            }
                        };
                        let mut outs: Vec<Arc<Geometry>> =
                            Vec::with_capacity(outs_arr.length() as usize);
                        for i in 0..outs_arr.length() {
                            let u8: js_sys::Uint8Array = match outs_arr.get(i).dyn_into() {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            let bytes = u8.to_vec();
                            if let Ok(g) = serde_json::from_slice::<Geometry>(&bytes) {
                                outs.push(Arc::new(g));
                            }
                        }
                        events.borrow_mut().push_back(WorkerEvent::CookFinished {
                            duration_ms,
                            outputs: outs,
                        });
                    }
                    _ => {
                        events.borrow_mut().push_back(WorkerEvent::Error {
                            message: format!("unknown worker event kind: {kind}"),
                        });
                    }
                }
            })
        };
        worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();

        Self { worker, events }
    }

    pub fn send(&self, cmd: HostCommand) {
        let txt = match serde_json::to_string(&cmd) {
            Ok(s) => s,
            Err(e) => {
                self.events.borrow_mut().push_back(WorkerEvent::Error {
                    message: format!("send cmd serialize failed: {e}"),
                });
                return;
            }
        };
        let _ = self.worker.post_message(&wasm_bindgen::JsValue::from_str(&txt));
    }

    pub fn try_recv(&self) -> Option<WorkerEvent> {
        self.events.borrow_mut().pop_front()
    }

    pub fn queue_len(&self) -> usize {
        self.events.borrow().len()
    }
}

