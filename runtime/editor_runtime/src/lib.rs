//! Internal editor runtime module (hot-loadable).

use cunning_runtime_api::{CStringView, CommandDesc, HostApi, RuntimeApi, C3D_RUNTIME_ABI_VERSION};
use core::ffi::c_char;

include!(concat!(env!("OUT_DIR"), "/build_id.rs"));

static CMD0: &[u8] = b"Ping";
static CMD1: &[u8] = b"BuildId";

const fn sv(b: &'static [u8]) -> CStringView {
    CStringView { ptr: b.as_ptr() as *const c_char, len: b.len() as u32 }
}

extern "C" fn command_count() -> u32 {
    2
}

extern "C" fn command_desc(i: u32, out: *mut CommandDesc) -> u32 {
    if out.is_null() {
        return 0;
    }
    let name = match i {
        0 => sv(CMD0),
        1 => sv(CMD1),
        _ => return 0,
    };
    unsafe { *out = CommandDesc { name } };
    1
}

extern "C" fn command_run(i: u32, host: *const HostApi) -> i32 {
    if host.is_null() {
        return -1;
    }
    let h = unsafe { &*host };
    match i {
        0 => {
            (h.log_info)(h.userdata, sv(b"editor_runtime: ping"));
            0
        }
        1 => {
            let msg = format!("editor_runtime: build_id={}", BUILD_ID);
            let b = msg.as_bytes();
            (h.log_info)(h.userdata, CStringView { ptr: b.as_ptr() as *const c_char, len: b.len() as u32 });
            0
        }
        _ => -1,
    }
}

static API: RuntimeApi = RuntimeApi {
    abi_version: C3D_RUNTIME_ABI_VERSION,
    runtime_build_id: BUILD_ID,
    command_count,
    command_desc,
    command_run,
};

#[no_mangle]
pub unsafe extern "C" fn c3d_runtime_get_api() -> *const RuntimeApi {
    &API as *const RuntimeApi
}

