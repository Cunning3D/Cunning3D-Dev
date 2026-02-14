//! Cunning3D internal runtime module ABI (hot-loadable DLLs).

#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_char, c_void};

pub const C3D_RUNTIME_ABI_VERSION: u32 = 1;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CStringView {
    pub ptr: *const c_char,
    pub len: u32,
}

unsafe impl Send for CStringView {}
unsafe impl Sync for CStringView {}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct HostApi {
    pub userdata: *mut c_void,
    pub log_info: extern "C" fn(userdata: *mut c_void, msg: CStringView),
    pub log_warn: extern "C" fn(userdata: *mut c_void, msg: CStringView),
    pub log_error: extern "C" fn(userdata: *mut c_void, msg: CStringView),
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CommandDesc {
    pub name: CStringView,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct RuntimeApi {
    pub abi_version: u32,
    pub runtime_build_id: u64,
    pub command_count: extern "C" fn() -> u32,
    pub command_desc: extern "C" fn(i: u32, out: *mut CommandDesc) -> u32,
    pub command_run: extern "C" fn(i: u32, host: *const HostApi) -> i32,
}

pub type GetRuntimeApiFn = unsafe extern "C" fn() -> *const RuntimeApi;

