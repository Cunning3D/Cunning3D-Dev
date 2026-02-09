//! WebWorker runtime for wasm32: runs CDA compile+cook off the main UI thread.
//!
//! This module is activated when the same `cunning_player` wasm is instantiated in a WebWorker.
//! The crate's `#[wasm_bindgen(start)]` detects worker context and calls `install_worker_runtime()`.

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::sync::Arc;

    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    use cunning_cda_runtime::{
        asset::{NodeId, RuntimeDefinition},
        compiler,
        registry::RuntimeRegistry,
        vm,
    };
    use cunning_kernel::mesh::Geometry;
    use cunning_kernel::traits::parameter::ParameterValue;

    use crate::protocol::{HostCommand, WorkerWireEvent};
    use crate::worker::apply_internal_overrides;

    fn post_wire_event(scope: &web_sys::DedicatedWorkerGlobalScope, ev: WorkerWireEvent) {
        // We send a structured-clone friendly array:
        // - ["Error", message]
        // - ["AssetReady", Uint8Array(def_json)]
        // - ["CookFinished", duration_ms, [Uint8Array(out0), Uint8Array(out1), ...]]
        let msg = js_sys::Array::new();
        let mut transfer = js_sys::Array::new();

        match ev {
            WorkerWireEvent::Error { message } => {
                msg.push(&JsValue::from_str("Error"));
                msg.push(&JsValue::from_str(&message));
                let _ = scope.post_message(&msg);
            }
            WorkerWireEvent::AssetReady { def_json } => {
                msg.push(&JsValue::from_str("AssetReady"));
                let u8 = js_sys::Uint8Array::from(def_json.as_slice());
                transfer.push(&u8.buffer());
                msg.push(&u8);
                let _ = scope.post_message_with_transfer(&msg, &transfer);
            }
            WorkerWireEvent::CookFinished {
                duration_ms,
                outputs_json,
            } => {
                msg.push(&JsValue::from_str("CookFinished"));
                msg.push(&JsValue::from_f64(duration_ms as f64));
                let outs = js_sys::Array::new();
                for b in outputs_json {
                    let u8 = js_sys::Uint8Array::from(b.as_slice());
                    transfer.push(&u8.buffer());
                    outs.push(&u8);
                }
                msg.push(&outs);
                let _ = scope.post_message_with_transfer(&msg, &transfer);
            }
        }
    }

    fn default_external_inputs() -> Vec<Arc<Geometry>> {
        // keep the exact same default as the main-thread worker (simple cube)
        use bevy::prelude::Vec3;
        use cunning_kernel::libs::geometry::attrs;
        use cunning_kernel::mesh::{Attribute, GeoPrimitive, PolygonPrim};

        let mut g = Geometry::new();
        let pts = [
            Vec3::new(-1.0, -1.0, -1.0),
            Vec3::new(1.0, -1.0, -1.0),
            Vec3::new(1.0, 1.0, -1.0),
            Vec3::new(-1.0, 1.0, -1.0),
            Vec3::new(-1.0, -1.0, 1.0),
            Vec3::new(1.0, -1.0, 1.0),
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(-1.0, 1.0, 1.0),
        ];
        let mut pids = Vec::with_capacity(8);
        for _ in 0..8 {
            pids.push(g.add_point());
        }
        g.insert_point_attribute(attrs::P, Attribute::new(pts.to_vec()));
        let face = |g: &mut Geometry,
                    pids: &[cunning_kernel::mesh::PointId],
                    a: usize,
                    b: usize,
                    c: usize,
                    d: usize| {
            let vs = [a, b, c, d].map(|i| g.add_vertex(pids[i]));
            g.add_primitive(GeoPrimitive::Polygon(PolygonPrim {
                vertices: vs.to_vec(),
            }));
        };
        face(&mut g, &pids, 0, 1, 2, 3); // -Z
        face(&mut g, &pids, 4, 5, 6, 7); // +Z
        face(&mut g, &pids, 0, 4, 7, 3); // -X
        face(&mut g, &pids, 1, 5, 6, 2); // +X
        face(&mut g, &pids, 3, 2, 6, 7); // +Y
        face(&mut g, &pids, 0, 1, 5, 4); // -Y
        vec![Arc::new(g)]
    }

    async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
        use wasm_bindgen_futures::JsFuture;
        let scope = js_sys::global()
            .dyn_into::<web_sys::DedicatedWorkerGlobalScope>()
            .map_err(|_| "not in worker")?;
        let resp = JsFuture::from(scope.fetch_with_str(url))
            .await
            .map_err(|e| format!("{e:?}"))?;
        let resp: web_sys::Response = resp.dyn_into().map_err(|_| "fetch response cast failed")?;
        if !resp.ok() {
            return Err(format!("http {} {}", resp.status(), resp.status_text()));
        }
        let ab = JsFuture::from(
            resp.array_buffer()
                .map_err(|_| "array_buffer failed")?,
        )
        .await
        .map_err(|e| format!("{e:?}"))?;
        let u8 = js_sys::Uint8Array::new(&ab);
        Ok(u8.to_vec())
    }

    const CDA_MAGIC: &[u8; 4] = b"CDA\0";
    const CDA_FILE_VERSION: u32 = 1;
    const CHUNK_GAME: [u8; 4] = *b"GAME";

    fn extract_chunk<'a>(data: &'a [u8], chunk: [u8; 4]) -> Result<&'a [u8], String> {
        if data.get(0..4) != Some(CDA_MAGIC) {
            return Err("Invalid CDA magic".into());
        }
        let ver = u32::from_le_bytes(
            data.get(4..8)
                .ok_or("Invalid CDA header")?
                .try_into()
                .map_err(|_| "Invalid CDA header")?,
        );
        if ver != CDA_FILE_VERSION {
            return Err(format!("CDA version mismatch: {ver}"));
        }
        let n = u32::from_le_bytes(
            data.get(8..12)
                .ok_or("Invalid CDA header")?
                .try_into()
                .map_err(|_| "Invalid CDA header")?,
        ) as usize;
        let mut at = 12usize;
        for _ in 0..n {
            let id: [u8; 4] = data
                .get(at..at + 4)
                .ok_or("Invalid CDA header")?
                .try_into()
                .map_err(|_| "Invalid CDA header")?;
            at += 4;
            let off = u64::from_le_bytes(
                data.get(at..at + 8)
                    .ok_or("Invalid CDA header")?
                    .try_into()
                    .map_err(|_| "Invalid CDA header")?,
            );
            at += 8;
            let sz = u64::from_le_bytes(
                data.get(at..at + 8)
                    .ok_or("Invalid CDA header")?
                    .try_into()
                    .map_err(|_| "Invalid CDA header")?,
            );
            at += 8;
            if id == chunk {
                let s = off as usize;
                let e = s
                    .checked_add(sz as usize)
                    .ok_or("Invalid CDA chunk size")?;
                return Ok(data
                    .get(s..e)
                    .ok_or("Invalid CDA chunk range")?);
            }
        }
        Err("Missing CDA chunk".into())
    }

    fn parse_game_def(bytes: &[u8]) -> Result<RuntimeDefinition, String> {
        let game = extract_chunk(bytes, CHUNK_GAME)?;
        let json = std::str::from_utf8(game).map_err(|_| "GAME chunk is not valid utf-8")?;
        serde_json::from_str::<RuntimeDefinition>(json)
            .map_err(|e| format!("GAME json parse failed: {e}"))
    }

    struct State {
        reg: RuntimeRegistry,
        def: Option<RuntimeDefinition>,
        plan: Option<compiler::ExecutionPlan>,
        overrides: HashMap<String, ParameterValue>,
        node_index_by_id: HashMap<NodeId, usize>,
        internal_overrides: HashMap<NodeId, HashMap<String, ParameterValue>>,
        external_inputs: Vec<Arc<Geometry>>,
        load_epoch: u64,
    }

    impl State {
        fn new() -> Self {
            Self {
                reg: RuntimeRegistry::new_default(),
                def: None,
                plan: None,
                overrides: HashMap::new(),
                node_index_by_id: HashMap::new(),
                internal_overrides: HashMap::new(),
                external_inputs: default_external_inputs(),
                load_epoch: 0,
            }
        }
    }

    pub fn install_worker_runtime() {
        let scope: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();

        let st = Rc::new(RefCell::new(State::new()));

        let onmessage = {
            let scope = scope.clone();
            let st = st.clone();
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
                let Some(txt) = ev.data().as_string() else {
                    post_wire_event(
                        &scope,
                        WorkerWireEvent::Error {
                            message: "worker expected string message".to_string(),
                        },
                    );
                    return;
                };
                let cmd = match serde_json::from_str::<HostCommand>(&txt) {
                    Ok(c) => c,
                    Err(e) => {
                        post_wire_event(
                            &scope,
                            WorkerWireEvent::Error {
                                message: format!("worker cmd parse failed: {e}"),
                            },
                        );
                        return;
                    }
                };

                // Handle commands synchronously inside the worker thread.
                // NOTE: Heavy work (Cook) is intentionally off the UI thread.
                match cmd {
                    HostCommand::LoadCdaUrl { url } => {
                        let scope2 = scope.clone();
                        let st2 = st.clone();
                        let epoch = {
                            let mut st = st.borrow_mut();
                            st.load_epoch = st.load_epoch.wrapping_add(1);
                            st.load_epoch
                        };
                        wasm_bindgen_futures::spawn_local(async move {
                            let bytes = match fetch_bytes(&url).await {
                                Ok(b) => b,
                                Err(message) => {
                                    if st2.borrow().load_epoch == epoch {
                                        post_wire_event(&scope2, WorkerWireEvent::Error { message });
                                    }
                                    return;
                                }
                            };
                            if st2.borrow().load_epoch != epoch {
                                return;
                            }
                            let mut st = st2.borrow_mut();
                            let d = match parse_game_def(&bytes)
                                .and_then(|d| {
                                    let p = compiler::compile(&d, &st.reg)
                                        .map_err(|e| format!("{e:?}"))?;
                                    st.plan = Some(p);
                                    Ok(d)
                                }) {
                                Ok(d) => d,
                                Err(message) => {
                                    if st.load_epoch == epoch {
                                        post_wire_event(&scope2, WorkerWireEvent::Error { message });
                                    }
                                    return;
                                }
                            };
                            st.node_index_by_id = d
                                .nodes
                                .iter()
                                .enumerate()
                                .map(|(i, n)| (n.id, i))
                                .collect();
                            st.overrides.clear();
                            st.internal_overrides.clear();
                            st.def = Some(d);

                            let def_json = serde_json::to_vec(st.def.as_ref().unwrap())
                                .unwrap_or_default();
                            if st.load_epoch == epoch {
                                post_wire_event(&scope2, WorkerWireEvent::AssetReady { def_json });
                            }
                        });
                    }
                    HostCommand::LoadCdaBytes { bytes } => {
                        let mut st = st.borrow_mut();
                        let d = match parse_game_def(&bytes).and_then(|d| {
                            let p = compiler::compile(&d, &st.reg).map_err(|e| format!("{e:?}"))?;
                            st.plan = Some(p);
                            Ok(d)
                        }) {
                            Ok(d) => d,
                            Err(message) => {
                                post_wire_event(&scope, WorkerWireEvent::Error { message });
                                return;
                            }
                        };
                        st.node_index_by_id = d
                            .nodes
                            .iter()
                            .enumerate()
                            .map(|(i, n)| (n.id, i))
                            .collect();
                        st.overrides.clear();
                        st.internal_overrides.clear();
                        st.def = Some(d);
                        let def_json = serde_json::to_vec(st.def.as_ref().unwrap()).unwrap_or_default();
                        post_wire_event(&scope, WorkerWireEvent::AssetReady { def_json });
                    }
                    HostCommand::SetOverride { name, value } => {
                        st.borrow_mut().overrides.insert(name, value);
                    }
                    HostCommand::SetInternalOverride { node, param, value } => {
                        st.borrow_mut()
                            .internal_overrides
                            .entry(node)
                            .or_default()
                            .insert(param, value);
                    }
                    HostCommand::Batch { cmds } => {
                        for c in cmds {
                            // Re-dispatch locally
                            let s = serde_json::to_string(&c).unwrap_or_default();
                            // Call handler by emulating a message event.
                            // (Keep behavior simple; batches are used mainly on main thread.)
                            let _ = s;
                            match c {
                                HostCommand::SetOverride { name, value } => {
                                    st.borrow_mut().overrides.insert(name, value);
                                }
                                HostCommand::SetInternalOverride { node, param, value } => {
                                    st.borrow_mut()
                                        .internal_overrides
                                        .entry(node)
                                        .or_default()
                                        .insert(param, value);
                                }
                                HostCommand::Cook => {
                                    // fallthrough; handled after batch (caller can send explicit Cook too)
                                }
                                _ => {}
                            }
                        }
                    }
                    HostCommand::Cook => {
                        let t0 = js_sys::Date::now();
                        let mut st = st.borrow_mut();
                        let (Some(def), Some(plan)) = (st.def.as_ref(), st.plan.as_ref()) else {
                            post_wire_event(
                                &scope,
                                WorkerWireEvent::Error {
                                    message: "Cook requested before asset loaded".to_string(),
                                },
                            );
                            return;
                        };

                        let d_work = apply_internal_overrides(
                            def,
                            &st.node_index_by_id,
                            &st.internal_overrides,
                        );
                        let cancel = std::sync::atomic::AtomicBool::new(false);
                        let outs = match vm::execute(
                            plan,
                            &d_work,
                            &st.reg,
                            &st.external_inputs,
                            &st.overrides,
                            &cancel,
                        ) {
                            Ok(v) => v,
                            Err(e) => {
                                post_wire_event(
                                    &scope,
                                    WorkerWireEvent::Error {
                                        message: format!("{e:?}"),
                                    },
                                );
                                return;
                            }
                        };

                        let mut outputs_json: Vec<Vec<u8>> = Vec::with_capacity(outs.len());
                        for g in outs {
                            // Geometry derives Serialize/Deserialize; JSON transport is simple and robust.
                            let b = serde_json::to_vec(g.as_ref()).unwrap_or_default();
                            outputs_json.push(b);
                        }
                        let dt = (js_sys::Date::now() - t0).max(0.0) as u32;
                        post_wire_event(
                            &scope,
                            WorkerWireEvent::CookFinished {
                                duration_ms: dt,
                                outputs_json,
                            },
                        );
                    }
                }
            })
        };

        scope.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();
    }
}

/// Install the worker runtime when running in WebWorker.
pub fn install_worker_runtime() {
    #[cfg(target_arch = "wasm32")]
    wasm::install_worker_runtime();
}

