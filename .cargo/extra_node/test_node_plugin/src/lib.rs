//! Generated Rust plugin node (NodeSpec driven).

use cunning_plugin_sdk::c_api::*;
use core::ffi::c_void;

static PLUGIN_NAME: &[u8] = b"test_node_plugin";
static PLUGIN_VERSION: &[u8] = b"0.1.0";
static NODE_NAME: &[u8] = b"Test Node";
static NODE_CAT: &[u8] = b"Utility";

static IN_0: &[u8] = b"Input";
static OUT_0: &[u8] = b"Output";

static INPUTS: [CStringView; 1] = [CStringView { ptr: IN_0.as_ptr() as *const i8, len: IN_0.len() as u32 },];
static OUTPUTS: [CStringView; 1] = [CStringView { ptr: OUT_0.as_ptr() as *const i8, len: OUT_0.len() as u32 },];

#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_info() -> CPluginDetails {
    CPluginDetails { abi_version: CUNNING_PLUGIN_ABI_VERSION, name: CStringView { ptr: PLUGIN_NAME.as_ptr() as *const i8, len: PLUGIN_NAME.len() as u32 }, version: CStringView { ptr: PLUGIN_VERSION.as_ptr() as *const i8, len: PLUGIN_VERSION.len() as u32 } }
}

#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_node_count() -> u32 { 1 }

static PARAMS: [CParamDesc; 0] = [
];

#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_get_node_desc(_i: u32, out: *mut CNodeDesc) -> i32 {
    if out.is_null() { return 1; }
    *out = CNodeDesc { name: CStringView { ptr: NODE_NAME.as_ptr() as *const i8, len: NODE_NAME.len() as u32 }, category: CStringView { ptr: NODE_CAT.as_ptr() as *const i8, len: NODE_CAT.len() as u32 }, inputs: CPortList { ptr: INPUTS.as_ptr(), len: INPUTS.len() as u32 }, outputs: CPortList { ptr: OUTPUTS.as_ptr(), len: OUTPUTS.len() as u32 }, input_style: CInputStyle::Single, node_style: CNodeStyle::Normal, params: PARAMS.as_ptr(), params_len: PARAMS.len() as u32 };
    0
}

extern "C" fn create() -> *mut c_void { core::ptr::null_mut() }
extern "C" fn destroy(_p: *mut c_void) {}

fn decode_f32(p: &CParamValue) -> f32 { f32::from_bits(p.a as u32) }
fn decode_i32(p: &CParamValue) -> i32 { p.a as i64 as i32 }
fn decode_bool(p: &CParamValue) -> bool { p.a != 0 }
fn decode_vec3(p: &CParamValue) -> (f32,f32,f32) { (f32::from_bits(p.a as u32), f32::from_bits((p.a>>32) as u32), f32::from_bits(p.b as u32)) }

// === USER_CODE_BEGIN ===

// Implement your algorithm logic here.
// You can use HostApi for geometry or attribute operations.

// === USER_CODE_END ===

extern "C" fn compute(_instance: *mut c_void, host: *const CHostApi, _ctx: *const CExecutionCtx, inputs: *const GeoHandle, inputs_len: u32, params: *const CParamValue, params_len: u32, out: *mut GeoHandle) -> i32 {
    unsafe {
        if host.is_null() || out.is_null() { return 1; }
        let host = &*host;
        let in0 = if inputs.is_null() || inputs_len == 0 { 0 } else { *inputs };
        let _params = if params.is_null() || params_len == 0 { &[][..] } else { core::slice::from_raw_parts(params, params_len as usize) };
        // Default behavior: passthrough/clone input.
        let g = if in0 != 0 { (host.geo_clone)(host.userdata, in0) } else { (host.geo_create)(host.userdata) };
        *out = g;
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_get_node_vtable(_i: u32) -> CNodeVTable { CNodeVTable { create, compute, destroy } }

