use core::ffi::{c_char, c_void};

pub type GeoHandle = u64;

pub const CUNNING_PLUGIN_ABI_VERSION: u32 = 6;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CUuid { pub lo: u64, pub hi: u64 }

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CStringView { pub ptr: *const c_char, pub len: u32 }

// FFI view type: raw pointer + length. Thread-safety is the caller's responsibility.
unsafe impl Send for CStringView {}
unsafe impl Sync for CStringView {}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CPluginDetails {
    pub abi_version: u32,
    pub name: CStringView,
    pub version: CStringView,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CExecutionCtx { pub time: f64, pub frame: f32 }

#[repr(C)]
#[derive(Copy, Clone)]
pub enum CParamTag { Int = 0, Float = 1, Bool = 2, Vec2 = 3, Vec3 = 4, Vec4 = 5, String = 6, Color3 = 7, Color4 = 8, Curve = 9 }

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CParamValue {
    pub tag: CParamTag,
    pub _pad0: [u32; 3],
    pub a: u64,
    pub b: u64,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub enum CParamUiTag { None = 0, FloatSlider = 1, IntSlider = 2, Vec2Drag = 3, Vec3Drag = 4, Vec4Drag = 5, String = 6, Toggle = 7, Dropdown = 8, Color = 9, Code = 10, CurvePoints = 11 }

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CParamUi {
    pub tag: CParamUiTag,
    pub _pad0: [u32; 3],
    pub a: u64,
    pub b: u64,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CParamDesc {
    pub name: CStringView,
    pub label: CStringView,
    pub group: CStringView,
    pub default_value: CParamValue,
    pub ui: CParamUi,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub enum CCurveType { Polygon = 0, Bezier = 1, Nurbs = 2 }

#[repr(C)]
#[derive(Copy, Clone)]
pub enum CPointMode { Corner = 0, Bezier = 1 }

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CCurveControlPoint {
    pub id: CUuid,
    pub position: [f32; 3],
    pub mode: CPointMode,
    pub _pad0: [u32; 2],
    pub handle_in: [f32; 3],
    pub _pad1: u32,
    pub handle_out: [f32; 3],
    pub _pad2: u32,
    pub weight: f32,
    pub _pad3: [u32; 3],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CCurveData { pub pts: *const CCurveControlPoint, pub len: u32, pub closed: u32, pub curve_type: u32 }

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CPortList { pub ptr: *const CStringView, pub len: u32 }

#[repr(C)]
#[derive(Copy, Clone)]
pub enum CInputStyle { Single = 0, Multi = 1, NamedPorts = 2 }

#[repr(C)]
#[derive(Copy, Clone)]
pub enum CNodeStyle { Normal = 0, Large = 1 }

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CNodeDesc {
    pub name: CStringView,
    pub category: CStringView,
    pub inputs: CPortList,
    pub outputs: CPortList,
    pub input_style: CInputStyle,
    pub node_style: CNodeStyle,
    pub params: *const CParamDesc,
    pub params_len: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CGeoSlice { pub ptr: *mut c_void, pub len: u32, pub stride: u32 }

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum CAttrDomain { Detail = 0, Point = 1, Vertex = 2, Primitive = 3, Edge = 4 }

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum CAttrType { I32 = 0, F32 = 1, Vec2 = 2, Vec3 = 3, Vec4 = 4, Bool = 5, String = 6 }

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CHostApi {
    pub userdata: *mut c_void,
    pub geo_create: extern "C" fn(userdata: *mut c_void) -> GeoHandle,
    pub geo_clone: extern "C" fn(userdata: *mut c_void, src: GeoHandle) -> GeoHandle,
    pub geo_drop: extern "C" fn(userdata: *mut c_void, h: GeoHandle),
    pub geo_point_count: extern "C" fn(userdata: *mut c_void, h: GeoHandle) -> u32,
    pub geo_vertex_count: extern "C" fn(userdata: *mut c_void, h: GeoHandle) -> u32,
    pub geo_prim_count: extern "C" fn(userdata: *mut c_void, h: GeoHandle) -> u32,
    pub geo_edge_count: extern "C" fn(userdata: *mut c_void, h: GeoHandle) -> u32,
    pub geo_add_point: extern "C" fn(userdata: *mut c_void, h: GeoHandle) -> u32,
    pub geo_add_vertex: extern "C" fn(userdata: *mut c_void, h: GeoHandle, point_dense: u32) -> u32,
    pub geo_remove_point: extern "C" fn(userdata: *mut c_void, h: GeoHandle, point_dense: u32),
    pub geo_remove_vertex: extern "C" fn(userdata: *mut c_void, h: GeoHandle, vtx_dense: u32),
    pub geo_add_edge: extern "C" fn(userdata: *mut c_void, h: GeoHandle, p0_dense: u32, p1_dense: u32) -> u32,
    pub geo_remove_edge: extern "C" fn(userdata: *mut c_void, h: GeoHandle, edge_dense: u32),
    pub geo_add_polygon: extern "C" fn(userdata: *mut c_void, h: GeoHandle, point_dense: *const u32, point_len: u32) -> u32,
    pub geo_add_polyline: extern "C" fn(userdata: *mut c_void, h: GeoHandle, point_dense: *const u32, point_len: u32, closed: u32) -> u32,
    pub geo_set_prim_vertices: extern "C" fn(userdata: *mut c_void, h: GeoHandle, prim_dense: u32, vtx_dense: *const u32, vtx_len: u32),
    pub geo_remove_prim: extern "C" fn(userdata: *mut c_void, h: GeoHandle, prim_dense: u32),
    pub geo_vertex_point: extern "C" fn(userdata: *mut c_void, h: GeoHandle, vtx_dense: u32) -> u32,
    pub geo_edge_points: extern "C" fn(userdata: *mut c_void, h: GeoHandle, edge_dense: u32, out_p0p1: *mut u32) -> u32,
    pub geo_prim_point_count: extern "C" fn(userdata: *mut c_void, h: GeoHandle, prim_dense: u32) -> u32,
    pub geo_prim_points: extern "C" fn(userdata: *mut c_void, h: GeoHandle, prim_dense: u32, out_points: *mut u32, out_cap: u32) -> u32,
    pub geo_get_point_position: extern "C" fn(userdata: *mut c_void, h: GeoHandle, point_dense: u32, out_xyz: *mut f32) -> u32,
    pub geo_set_point_position: extern "C" fn(userdata: *mut c_void, h: GeoHandle, point_dense: u32, xyz: *const f32) -> u32,
    pub attr_ensure: extern "C" fn(userdata: *mut c_void, h: GeoHandle, domain: CAttrDomain, ty: CAttrType, name: CStringView, len: u32) -> CGeoSlice,
    pub attr_view: extern "C" fn(userdata: *mut c_void, h: GeoHandle, domain: CAttrDomain, ty: CAttrType, name: CStringView) -> CGeoSlice,
    pub attr_bool_get: extern "C" fn(userdata: *mut c_void, h: GeoHandle, domain: CAttrDomain, name: CStringView, index: u32) -> u32,
    pub attr_bool_set: extern "C" fn(userdata: *mut c_void, h: GeoHandle, domain: CAttrDomain, name: CStringView, index: u32, value: u32) -> u32,
    pub attr_string_get: extern "C" fn(userdata: *mut c_void, h: GeoHandle, domain: CAttrDomain, name: CStringView, index: u32, out_ptr: *mut u8, out_cap: u32) -> u32,
    pub attr_string_set: extern "C" fn(userdata: *mut c_void, h: GeoHandle, domain: CAttrDomain, name: CStringView, index: u32, value: CStringView) -> u32,

    // --- Node data I/O (for interaction) ---
    pub node_state_get: extern "C" fn(userdata: *mut c_void, node: CUuid, key: CStringView, out_ptr: *mut u8, out_cap: u32) -> u32,
    pub node_state_set: extern "C" fn(userdata: *mut c_void, node: CUuid, key: CStringView, bytes: *const u8, len: u32) -> u32,
    pub node_curve_get: extern "C" fn(userdata: *mut c_void, node: CUuid, param: CStringView, out_pts: *mut CCurveControlPoint, out_cap: u32, out_len: *mut u32, out_closed: *mut u32, out_ty: *mut u32) -> u32,
    pub node_curve_set: extern "C" fn(userdata: *mut c_void, node: CUuid, param: CStringView, pts: *const CCurveControlPoint, len: u32, closed: u32, ty: u32) -> u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CNodeVTable {
    pub create: extern "C" fn() -> *mut c_void,
    pub compute: extern "C" fn(instance: *mut c_void, host: *const CHostApi, ctx: *const CExecutionCtx, inputs: *const GeoHandle, inputs_len: u32, params: *const CParamValue, params_len: u32, out: *mut GeoHandle) -> i32,
    pub destroy: extern "C" fn(instance: *mut c_void),
}

#[repr(C)]
#[derive(Copy, Clone)]
pub enum CHudCmdTag { Label = 0, Button = 1, Toggle = 2, Separator = 3 }

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CHudCmd {
    pub tag: CHudCmdTag,
    pub id: u32,
    pub value: u32,
    pub _pad0: u32,
    pub text: CStringView,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CHudEvent { pub id: u32, pub value: u32 }

#[repr(C)]
#[derive(Copy, Clone)]
pub enum CGizmoCmdTag { Mesh = 0, Line = 1 }

#[repr(C)]
#[derive(Copy, Clone)]
pub enum CGizmoPrimitive { Sphere = 0, Cube = 1, Cylinder = 2, Cone = 3, Plane = 4 }

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CTransform {
    pub translation: [f32; 3],
    pub _pad0: u32,
    pub rotation_xyzw: [f32; 4],
    pub scale: [f32; 3],
    pub _pad1: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CGizmoCmd {
    pub tag: CGizmoCmdTag,
    pub pick_id: u32,
    pub primitive: CGizmoPrimitive,
    pub _pad0: u32,
    pub transform: CTransform,
    pub color_rgba: [f32; 4],
    pub p0: [f32; 3],
    pub _pad1: u32,
    pub p1: [f32; 3],
    pub _pad2: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub enum CGizmoEventTag { Click = 0, Drag = 1, Release = 2 }

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CGizmoEvent {
    pub tag: CGizmoEventTag,
    pub pick_id: u32,
    pub world_pos: [f32; 3],
    pub _pad0: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub enum CInputEventTag { KeyDown = 0, KeyUp = 1 }

#[repr(C)]
#[derive(Copy, Clone)]
pub enum CKeyCode { F = 0, G = 1, H = 2, K = 3, Ctrl = 4, Shift = 5, Backspace = 6 }

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CInputEvent { pub tag: CInputEventTag, pub key: CKeyCode, pub _pad0: [u32; 2] }

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CNodeInteractionVTable {
    pub hud_build: extern "C" fn(instance: *mut c_void, host: *const CHostApi, node: CUuid, out_cmds: *mut CHudCmd, out_cap: u32) -> u32,
    pub hud_event: extern "C" fn(instance: *mut c_void, host: *const CHostApi, node: CUuid, e: *const CHudEvent) -> i32,
    pub gizmo_build: extern "C" fn(instance: *mut c_void, host: *const CHostApi, node: CUuid, out_cmds: *mut CGizmoCmd, out_cap: u32) -> u32,
    pub gizmo_event: extern "C" fn(instance: *mut c_void, host: *const CHostApi, node: CUuid, e: *const CGizmoEvent) -> i32,
    pub input_event: extern "C" fn(instance: *mut c_void, host: *const CHostApi, node: CUuid, e: *const CInputEvent) -> i32,
}

pub type PluginInfoFn = unsafe extern "C" fn() -> CPluginDetails;
pub type PluginNodeCountFn = unsafe extern "C" fn() -> u32;
pub type PluginGetNodeDescFn = unsafe extern "C" fn(i: u32, out: *mut CNodeDesc) -> i32;
pub type PluginGetNodeVTableFn = unsafe extern "C" fn(i: u32) -> CNodeVTable;
pub type PluginGetNodeInteractionVTableFn = unsafe extern "C" fn(i: u32, out: *mut CNodeInteractionVTable) -> i32;

