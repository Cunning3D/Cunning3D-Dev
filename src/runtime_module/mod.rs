//! Internal runtime module loader (hot-loadable DLLs, not node plugins).

use bevy::prelude::*;
use cunning_runtime_api::{CStringView, GetRuntimeApiFn, HostApi, RuntimeApi};
use libloading::Library;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub mod build_jobs;

#[derive(Resource, Default, Clone)]
pub struct RuntimeModuleLog(pub Arc<Mutex<Vec<String>>>);

impl RuntimeModuleLog {
    #[inline]
    pub fn push(&self, s: impl Into<String>) {
        if let Ok(mut v) = self.0.lock() {
            v.push(s.into());
            let len = v.len();
            if len > 512 {
                v.drain(0..len - 512);
            }
        }
    }
}

#[derive(Clone)]
pub struct LoadedRuntimeModule {
    _lib: Arc<Library>,
    api: *const RuntimeApi,
}

unsafe impl Send for LoadedRuntimeModule {}
unsafe impl Sync for LoadedRuntimeModule {}

impl LoadedRuntimeModule {
    #[inline]
    pub fn api(&self) -> &RuntimeApi {
        unsafe { &*self.api }
    }
}

#[derive(Resource, Default)]
pub struct RuntimeModuleState {
    pub module: Option<LoadedRuntimeModule>,
    pub last_loaded_path: Option<PathBuf>,
}

pub fn runtime_modules_dir() -> PathBuf {
    PathBuf::from("runtime_modules")
}

pub fn load_runtime_module(path: &Path) -> Result<LoadedRuntimeModule, String> {
    unsafe {
        let lib = Library::new(path).map_err(|e| format!("load dll failed: {e}"))?;
        let get_api = lib
            .get::<GetRuntimeApiFn>(b"c3d_runtime_get_api")
            .map_err(|e| format!("missing symbol c3d_runtime_get_api: {e}"))?;
        let api = get_api();
        if api.is_null() {
            return Err("c3d_runtime_get_api returned null".into());
        }
        Ok(LoadedRuntimeModule { _lib: Arc::new(lib), api })
    }
}

pub fn runtime_tick_system(
    state: Res<RuntimeModuleState>,
    log: Res<RuntimeModuleLog>,
) {
    let Some(m) = state.module.as_ref() else { return; };
    extern "C" fn log_info(u: *mut core::ffi::c_void, msg: CStringView) {
        unsafe {
            let log = &*(u as *const RuntimeModuleLog);
            if msg.ptr.is_null() || msg.len == 0 {
                log.push("<null>");
                return;
            }
            let b = core::slice::from_raw_parts(msg.ptr as *const u8, msg.len as usize);
            let s = core::str::from_utf8(b).unwrap_or("<utf8>");
            log.push(s.to_string());
        }
    }
    extern "C" fn log_warn(u: *mut core::ffi::c_void, msg: CStringView) { log_info(u, msg) }
    extern "C" fn log_error(u: *mut core::ffi::c_void, msg: CStringView) { log_info(u, msg) }
    let host = HostApi {
        userdata: (&*log as *const RuntimeModuleLog) as *mut core::ffi::c_void,
        log_info,
        log_warn,
        log_error,
    };
    let api = m.api();
    // Reserved for future per-frame runtime hooks.
    let _ = (api, host);
}

#[inline]
fn sv_to_string(s: CStringView) -> String {
    if s.ptr.is_null() || s.len == 0 {
        return String::new();
    }
    let b = unsafe { core::slice::from_raw_parts(s.ptr as *const u8, s.len as usize) };
    core::str::from_utf8(b).unwrap_or("<utf8>").to_string()
}

pub fn list_commands(state: &RuntimeModuleState) -> Vec<String> {
    let Some(m) = state.module.as_ref() else { return Vec::new(); };
    let api = m.api();
    let n = (api.command_count)() as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut d = cunning_runtime_api::CommandDesc { name: CStringView { ptr: core::ptr::null(), len: 0 } };
        if (api.command_desc)(i as u32, &mut d as *mut _) == 0 { continue; }
        out.push(sv_to_string(d.name));
    }
    out
}

pub fn run_command(state: &RuntimeModuleState, idx: u32, log: &RuntimeModuleLog) -> Result<(), String> {
    let Some(m) = state.module.as_ref() else { return Err("runtime module not loaded".into()); };
    extern "C" fn log_info(u: *mut core::ffi::c_void, msg: CStringView) {
        unsafe { (&*(u as *const RuntimeModuleLog)).push(sv_to_string(msg)); }
    }
    extern "C" fn log_warn(u: *mut core::ffi::c_void, msg: CStringView) { log_info(u, msg) }
    extern "C" fn log_error(u: *mut core::ffi::c_void, msg: CStringView) { log_info(u, msg) }
    let host = HostApi {
        userdata: (log as *const RuntimeModuleLog) as *mut core::ffi::c_void,
        log_info,
        log_warn,
        log_error,
    };
    let api = m.api();
    let rc = (api.command_run)(idx, &host as *const _);
    if rc == 0 { Ok(()) } else { Err(format!("command failed: {rc}")) }
}

