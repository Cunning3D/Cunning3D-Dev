//! Example Rust plugin node (C ABI).

use cunning_plugin_sdk::c_api::*;
use core::ffi::c_void;

static NAME: &[u8] = b"draggable_translator_plugin";
static VERSION: &[u8] = b"0.1.0";
static NODE_NAME: &[u8] = b"Draggable Translator";
static CAT: &[u8] = b"External/Rust";
static IN0: &[u8] = b"Input";
static OUT0: &[u8] = b"Output";

static INPUTS: [CStringView; 1] = [CStringView { ptr: IN0.as_ptr() as *const i8, len: 5 }];
static OUTPUTS: [CStringView; 1] = [CStringView { ptr: OUT0.as_ptr() as *const i8, len: 6 }];
static NO_PARAMS: [CParamDesc; 0] = [];

#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_info() -> CPluginDetails {
    CPluginDetails {
        abi_version: CUNNING_PLUGIN_ABI_VERSION,
        name: CStringView { ptr: NAME.as_ptr() as *const i8, len: NAME.len() as u32 },
        version: CStringView { ptr: VERSION.as_ptr() as *const i8, len: VERSION.len() as u32 },
    }
}

#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_node_count() -> u32 { 1 }

#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_get_node_desc(_i: u32, out: *mut CNodeDesc) -> i32 {
    if out.is_null() { return 1; }
    *out = CNodeDesc {
        name: CStringView { ptr: NODE_NAME.as_ptr() as *const i8, len: NODE_NAME.len() as u32 },
        category: CStringView { ptr: CAT.as_ptr() as *const i8, len: CAT.len() as u32 },
        inputs: CPortList { ptr: INPUTS.as_ptr(), len: INPUTS.len() as u32 },
        outputs: CPortList { ptr: OUTPUTS.as_ptr(), len: OUTPUTS.len() as u32 },
        input_style: CInputStyle::Single,
        node_style: CNodeStyle::Normal,
        params: NO_PARAMS.as_ptr(),
        params_len: 0,
    };
    0
}

extern "C" fn create() -> *mut c_void { core::ptr::null_mut() }
extern "C" fn destroy(_p: *mut c_void) {}

extern "C" fn compute(_instance: *mut c_void, host: *const CHostApi, _ctx: *const CExecutionCtx, inputs: *const GeoHandle, inputs_len: u32, _params: *const CParamValue, _params_len: u32, out: *mut GeoHandle) -> i32 {
    unsafe {
        if host.is_null() || out.is_null() { return 1; }
        let host = &*host;
        let in0 = if inputs.is_null() || inputs_len == 0 { 0 } else { *inputs };
        let g = if in0 != 0 { (host.geo_clone)(host.userdata, in0) } else { (host.geo_create)(host.userdata) };
        let p = (host.geo_add_point)(host.userdata, g);
        let xyz: [f32; 3] = [0.0, 0.0, 0.0];
        let _ = (host.geo_set_point_position)(host.userdata, g, p, xyz.as_ptr());
        *out = g;
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_get_node_vtable(_i: u32) -> CNodeVTable {
    CNodeVTable { create, compute, destroy }
}

