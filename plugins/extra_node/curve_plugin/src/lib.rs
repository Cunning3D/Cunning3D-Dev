use core::ffi::c_void;
use cunning_plugin_sdk::c_api as api;

static PLUGIN_NAME: &[u8] = b"CurvePlugin\0";
static PLUGIN_VER: &[u8] = b"0.1\0";

#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_info() -> api::CPluginDetails {
    api::CPluginDetails {
        abi_version: api::CUNNING_PLUGIN_ABI_VERSION,
        name: api::CStringView { ptr: PLUGIN_NAME.as_ptr() as *const _, len: (PLUGIN_NAME.len() - 1) as u32 },
        version: api::CStringView { ptr: PLUGIN_VER.as_ptr() as *const _, len: (PLUGIN_VER.len() - 1) as u32 },
    }
}

#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_node_count() -> u32 { 1 }

static NODE_NAME: &[u8] = b"Curve\0";
static NODE_CAT: &[u8] = b"Primitives\0";
static PORT_IN: [api::CStringView; 1] = [api::CStringView { ptr: b"Input\0".as_ptr() as *const _, len: 5 }];
static PORT_OUT: [api::CStringView; 1] = [api::CStringView { ptr: b"Output\0".as_ptr() as *const _, len: 6 }];
static PARAMS: [api::CParamDesc; 1] = [api::CParamDesc {
    name: api::CStringView { ptr: b"curve_data\0".as_ptr() as *const _, len: 9 },
    label: api::CStringView { ptr: b"Curve Data\0".as_ptr() as *const _, len: 10 },
    group: api::CStringView { ptr: b"Geometry\0".as_ptr() as *const _, len: 8 },
    default_value: api::CParamValue { tag: api::CParamTag::Curve, _pad0: [0; 3], a: 0, b: 0 },
    ui: api::CParamUi { tag: api::CParamUiTag::CurvePoints, _pad0: [0; 3], a: 0, b: 0 },
}];

#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_get_node_desc(i: u32, out: *mut api::CNodeDesc) -> i32 {
    if out.is_null() || i != 0 { return -1; }
    *out = api::CNodeDesc {
        name: api::CStringView { ptr: NODE_NAME.as_ptr() as *const _, len: (NODE_NAME.len() - 1) as u32 },
        category: api::CStringView { ptr: NODE_CAT.as_ptr() as *const _, len: (NODE_CAT.len() - 1) as u32 },
        inputs: api::CPortList { ptr: PORT_IN.as_ptr(), len: 1 },
        outputs: api::CPortList { ptr: PORT_OUT.as_ptr(), len: 1 },
        input_style: api::CInputStyle::Single,
        node_style: api::CNodeStyle::Normal,
        params: PARAMS.as_ptr(),
        params_len: PARAMS.len() as u32,
    };
    0
}

#[repr(C)]
struct CurveNode;

#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_get_node_vtable(i: u32) -> api::CNodeVTable {
    if i != 0 { return api::CNodeVTable { create: create, compute: compute, destroy: destroy }; }
    api::CNodeVTable { create, compute, destroy }
}

#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_get_node_interaction_vtable(i: u32, out: *mut api::CNodeInteractionVTable) -> i32 {
    if out.is_null() || i != 0 { return -1; }
    *out = api::CNodeInteractionVTable { hud_build, hud_event, gizmo_build, gizmo_event, input_event };
    0
}

extern "C" fn create() -> *mut c_void { Box::into_raw(Box::new(CurveNode)) as *mut c_void }
extern "C" fn destroy(p: *mut c_void) { if !p.is_null() { unsafe { drop(Box::from_raw(p as *mut CurveNode)); } } }

#[inline]
fn sv(s: &'static [u8]) -> api::CStringView { api::CStringView { ptr: s.as_ptr() as *const _, len: (s.len().saturating_sub(1)) as u32 } }

#[inline]
unsafe fn read_u32(host: &api::CHostApi, node: api::CUuid, key: api::CStringView) -> u32 {
    let mut buf = [0u8; 4];
    let n = (host.node_state_get)(host.userdata, node, key, buf.as_mut_ptr(), 4);
    if n < 4 { 0 } else { u32::from_le_bytes(buf) }
}

#[inline]
unsafe fn write_u32(host: &api::CHostApi, node: api::CUuid, key: api::CStringView, v: u32) {
    let b = v.to_le_bytes();
    let _ = (host.node_state_set)(host.userdata, node, key, b.as_ptr(), 4);
}

fn cubic_bezier(p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], p3: [f32; 3], t: f32) -> [f32; 3] {
    let (t2, t3) = (t * t, t * t * t);
    let omt = 1.0 - t;
    let (omt2, omt3) = (omt * omt, omt * omt * omt);
    [
        p0[0] * omt3 + p1[0] * (3.0 * omt2 * t) + p2[0] * (3.0 * omt * t2) + p3[0] * t3,
        p0[1] * omt3 + p1[1] * (3.0 * omt2 * t) + p2[1] * (3.0 * omt * t2) + p3[1] * t3,
        p0[2] * omt3 + p1[2] * (3.0 * omt2 * t) + p2[2] * (3.0 * omt * t2) + p3[2] * t3,
    ]
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn mul3(a: [f32; 3], s: f32) -> [f32; 3] { [a[0] * s, a[1] * s, a[2] * s] }
fn len3(a: [f32; 3]) -> f32 { (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt() }
fn norm3(a: [f32; 3]) -> [f32; 3] { let l = len3(a); if l <= 1e-6 { [0.0, 0.0, 0.0] } else { [a[0] / l, a[1] / l, a[2] / l] } }

// Auto tangents: Catmull-Rom style converted to cubic Bezier handles.
fn auto_handles(pts: &mut [api::CCurveControlPoint], closed: u32) {
    let n = pts.len();
    if n < 2 { return; }
    for i in 0..n {
        let p = pts[i].position;
        let prev = if i == 0 { if closed != 0 { pts[n - 1].position } else { p } } else { pts[i - 1].position };
        let next = if i + 1 == n { if closed != 0 { pts[0].position } else { p } } else { pts[i + 1].position };
        let t = if closed == 0 && i == 0 { sub3(next, p) } else if closed == 0 && i + 1 == n { sub3(p, prev) } else { mul3(sub3(next, prev), 0.5) };
        let h = mul3(t, 1.0 / 3.0);
        pts[i].mode = api::CPointMode::Bezier;
        pts[i].handle_out = h;
        pts[i].handle_in = mul3(h, -1.0);
    }
}

fn sample_positions(data: &api::CCurveData) -> Vec<[f32; 3]> {
    let pts = unsafe { core::slice::from_raw_parts(data.pts, data.len as usize) };
    if pts.is_empty() { return Vec::new(); }
    match data.curve_type {
        x if x == api::CCurveType::Polygon as u32 => pts.iter().map(|p| p.position).collect(),
        x if x == api::CCurveType::Bezier as u32 => {
            if pts.len() < 2 { return pts.iter().map(|p| p.position).collect(); }
            let segs = if data.closed != 0 { pts.len() } else { pts.len() - 1 };
            let mut out = Vec::new();
            let res = 20;
            for i in 0..segs {
                let p0 = &pts[i];
                let p1 = &pts[(i + 1) % pts.len()];
                let s = p0.position;
                let c1 = add3(p0.position, p0.handle_out);
                let c2 = add3(p1.position, p1.handle_in);
                let e = p1.position;
                for step in 0..res { out.push(cubic_bezier(s, c1, c2, e, step as f32 / res as f32)); }
            }
            if data.closed == 0 { out.push(pts.last().unwrap().position); }
            out
        }
        _ => pts.iter().map(|p| p.position).collect(), // NURBS: start with polyline fallback (will be upgraded next)
    }
}

unsafe fn decode_curve_param(params: *const api::CParamValue, params_len: u32) -> Option<api::CCurveData> {
    if params.is_null() || params_len == 0 { return None; }
    let ps = core::slice::from_raw_parts(params, params_len as usize);
    for p in ps {
        if matches!(p.tag, api::CParamTag::Curve) && p.a != 0 {
            let ptr = p.a as usize as *const api::CCurveData;
            if !ptr.is_null() { return Some(*ptr); }
        }
    }
    None
}

// --- Compute: CurveData -> polyline geometry ---
extern "C" fn compute(_instance: *mut c_void, host: *const api::CHostApi, _ctx: *const api::CExecutionCtx, _inputs: *const api::GeoHandle, _inputs_len: u32, _params: *const api::CParamValue, _params_len: u32, out: *mut api::GeoHandle) -> i32 {
    if host.is_null() || out.is_null() { return -1; }
    let host = unsafe { &*host };
    let Some(curve) = (unsafe { decode_curve_param(_params, _params_len) }) else { unsafe { *out = (host.geo_create)(host.userdata) }; return 0; };
    let positions = sample_positions(&curve);
    let h = (host.geo_create)(host.userdata);
    let mut pd: Vec<u32> = Vec::with_capacity(positions.len());
    for _ in 0..positions.len() { pd.push((host.geo_add_point)(host.userdata, h)); }

    // @P
    let s = (host.attr_ensure)(host.userdata, h, api::CAttrDomain::Point, api::CAttrType::Vec3, sv(b"@P\0"), pd.len() as u32);
    if !s.ptr.is_null() && s.stride >= 12 {
        let stride = (s.stride / 4) as usize;
        let fp = s.ptr as *mut f32;
        for (i, p) in positions.iter().enumerate() {
            let o = i * stride;
            unsafe { *fp.add(o) = p[0]; *fp.add(o + 1) = p[1]; *fp.add(o + 2) = p[2]; }
        }
    }
    // @N (up)
    let s = (host.attr_ensure)(host.userdata, h, api::CAttrDomain::Point, api::CAttrType::Vec3, sv(b"@N\0"), pd.len() as u32);
    if !s.ptr.is_null() && s.stride >= 12 {
        let stride = (s.stride / 4) as usize;
        let fp = s.ptr as *mut f32;
        for i in 0..positions.len() {
            let o = i * stride;
            unsafe { *fp.add(o) = 0.0; *fp.add(o + 1) = 1.0; *fp.add(o + 2) = 0.0; }
        }
    }
    if positions.len() >= 2 { let _ = (host.geo_add_polyline)(host.userdata, h, pd.as_ptr(), pd.len() as u32, curve.closed); }
    unsafe { *out = h; }
    0
}

// --- Interaction: Curve editing lives via host node_curve_get/set ---
extern "C" fn hud_build(_instance: *mut c_void, host: *const api::CHostApi, node: api::CUuid, out_cmds: *mut api::CHudCmd, out_cap: u32) -> u32 {
    if host.is_null() || out_cmds.is_null() || out_cap < 6 { return 0; }
    let host = unsafe { &*host };
    let mode = unsafe { read_u32(host, node, sv(b"mode\0")) };
    let indep = unsafe { read_u32(host, node, sv(b"indep_handles\0")) };
    let cmds = unsafe { core::slice::from_raw_parts_mut(out_cmds, out_cap as usize) };
    cmds[0] = api::CHudCmd { tag: api::CHudCmdTag::Label, id: 0, value: 0, _pad0: 0, text: sv(b"Curve Tool\0") };
    cmds[1] = api::CHudCmd { tag: api::CHudCmdTag::Button, id: 1, value: if mode == 0 { 1 } else { 0 }, _pad0: 0, text: sv(b"F: Edit\0") };
    cmds[2] = api::CHudCmd { tag: api::CHudCmdTag::Button, id: 2, value: if mode == 1 { 1 } else { 0 }, _pad0: 0, text: sv(b"G: Draw\0") };
    cmds[3] = api::CHudCmd { tag: api::CHudCmdTag::Button, id: 3, value: if mode == 2 { 1 } else { 0 }, _pad0: 0, text: sv(b"H: Auto\0") };
    cmds[4] = api::CHudCmd { tag: api::CHudCmdTag::Toggle, id: 4, value: if indep != 0 { 1 } else { 0 }, _pad0: 0, text: sv(b"K: Independent Handles\0") };
    cmds[5] = api::CHudCmd { tag: api::CHudCmdTag::Separator, id: 0, value: 0, _pad0: 0, text: api::CStringView { ptr: core::ptr::null(), len: 0 } };
    6
}

extern "C" fn hud_event(_instance: *mut c_void, host: *const api::CHostApi, node: api::CUuid, e: *const api::CHudEvent) -> i32 {
    if host.is_null() || e.is_null() { return -1; }
    let host = unsafe { &*host };
    let e = unsafe { &*e };
    match e.id {
        1 => unsafe { write_u32(host, node, sv(b"mode\0"), 0) },
        2 => unsafe { write_u32(host, node, sv(b"mode\0"), 1) },
        3 => unsafe { write_u32(host, node, sv(b"mode\0"), 2) },
        4 => unsafe { write_u32(host, node, sv(b"indep_handles\0"), if e.value != 0 { 1 } else { 0 }) },
        _ => {}
    }
    0
}

unsafe fn host_curve_get(host: &api::CHostApi, node: api::CUuid) -> Option<(Vec<api::CCurveControlPoint>, u32, u32)> {
    let mut pts: Vec<api::CCurveControlPoint> = vec![core::mem::zeroed(); 512];
    let mut out_len = 0u32;
    let mut out_closed = 0u32;
    let mut out_ty = api::CCurveType::Polygon as u32;
    let ok = (host.node_curve_get)(host.userdata, node, sv(b"curve_data\0"), pts.as_mut_ptr(), pts.len() as u32, &mut out_len as *mut _, &mut out_closed as *mut _, &mut out_ty as *mut _);
    if ok == 0 { return None; }
    pts.truncate(out_len.min(pts.len() as u32) as usize);
    Some((pts, out_closed, out_ty))
}

unsafe fn host_curve_set(host: &api::CHostApi, node: api::CUuid, pts: &[api::CCurveControlPoint], closed: u32, ty: u32) -> bool {
    (host.node_curve_set)(host.userdata, node, sv(b"curve_data\0"), pts.as_ptr(), pts.len() as u32, closed, ty) != 0
}

extern "C" fn gizmo_build(_instance: *mut c_void, host: *const api::CHostApi, node: api::CUuid, out_cmds: *mut api::CGizmoCmd, out_cap: u32) -> u32 {
    if host.is_null() || out_cmds.is_null() || out_cap == 0 { return 0; }
    let host = unsafe { &*host };
    let mode = unsafe { read_u32(host, node, sv(b"mode\0")) };
    let Some((pts, closed, ty)) = (unsafe { host_curve_get(host, node) }) else { return 0; };
    let data = api::CCurveData { pts: pts.as_ptr(), len: pts.len() as u32, closed, curve_type: ty };
    let sampled = sample_positions(&data);
    let cmds = unsafe { core::slice::from_raw_parts_mut(out_cmds, out_cap as usize) };
    let mut n = 0usize;

    // Curve preview lines
    for i in 0..sampled.len().saturating_sub(1) {
        if n >= cmds.len() { break; }
        cmds[n] = api::CGizmoCmd {
            tag: api::CGizmoCmdTag::Line,
            pick_id: 0,
            primitive: api::CGizmoPrimitive::Sphere,
            _pad0: 0,
            transform: api::CTransform { translation: [0.0; 3], _pad0: 0, rotation_xyzw: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3], _pad1: 0 },
            color_rgba: [1.0, 0.85, 0.2, 0.7],
            p0: sampled[i],
            _pad1: 0,
            p1: sampled[i + 1],
            _pad2: 0,
        };
        n += 1;
    }

    // Control points + handles
    let is_bezier = ty == api::CCurveType::Bezier as u32;
    let show_handles = is_bezier && mode != 2;
    for (i, p) in pts.iter().enumerate() {
        if n >= cmds.len() { break; }
        cmds[n] = api::CGizmoCmd {
            tag: api::CGizmoCmdTag::Mesh,
            pick_id: (i as u32) + 1,
            primitive: api::CGizmoPrimitive::Sphere,
            _pad0: 0,
            transform: api::CTransform { translation: p.position, _pad0: 0, rotation_xyzw: [0.0, 0.0, 0.0, 1.0], scale: [0.05, 0.05, 0.05], _pad1: 0 },
            color_rgba: [1.0, 1.0, 1.0, 1.0],
            p0: [0.0; 3],
            _pad1: 0,
            p1: [0.0; 3],
            _pad2: 0,
        };
        n += 1;
        if show_handles {
            let hin = add3(p.position, p.handle_in);
            let hout = add3(p.position, p.handle_out);

            if n < cmds.len() {
                cmds[n] = api::CGizmoCmd { tag: api::CGizmoCmdTag::Line, pick_id: 0, primitive: api::CGizmoPrimitive::Sphere, _pad0: 0, transform: api::CTransform { translation: [0.0; 3], _pad0: 0, rotation_xyzw: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3], _pad1: 0 }, color_rgba: [1.0, 1.0, 0.0, 0.35], p0: p.position, _pad1: 0, p1: hin, _pad2: 0 };
                n += 1;
            }
            if n < cmds.len() {
                cmds[n] = api::CGizmoCmd { tag: api::CGizmoCmdTag::Line, pick_id: 0, primitive: api::CGizmoPrimitive::Sphere, _pad0: 0, transform: api::CTransform { translation: [0.0; 3], _pad0: 0, rotation_xyzw: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3], _pad1: 0 }, color_rgba: [1.0, 1.0, 0.0, 0.35], p0: p.position, _pad1: 0, p1: hout, _pad2: 0 };
                n += 1;
            }
            if n < cmds.len() {
                cmds[n] = api::CGizmoCmd { tag: api::CGizmoCmdTag::Mesh, pick_id: 1000 + (i as u32), primitive: api::CGizmoPrimitive::Cube, _pad0: 0, transform: api::CTransform { translation: hin, _pad0: 0, rotation_xyzw: [0.0, 0.0, 0.0, 1.0], scale: [0.03, 0.03, 0.03], _pad1: 0 }, color_rgba: [1.0, 1.0, 0.0, 1.0], p0: [0.0; 3], _pad1: 0, p1: [0.0; 3], _pad2: 0 };
                n += 1;
            }
            if n < cmds.len() {
                cmds[n] = api::CGizmoCmd { tag: api::CGizmoCmdTag::Mesh, pick_id: 2000 + (i as u32), primitive: api::CGizmoPrimitive::Cube, _pad0: 0, transform: api::CTransform { translation: hout, _pad0: 0, rotation_xyzw: [0.0, 0.0, 0.0, 1.0], scale: [0.03, 0.03, 0.03], _pad1: 0 }, color_rgba: [1.0, 1.0, 0.0, 1.0], p0: [0.0; 3], _pad1: 0, p1: [0.0; 3], _pad2: 0 };
                n += 1;
            }
        }
    }
    n as u32
}

extern "C" fn gizmo_event(_instance: *mut c_void, host: *const api::CHostApi, node: api::CUuid, e: *const api::CGizmoEvent) -> i32 {
    if host.is_null() || e.is_null() { return -1; }
    let host = unsafe { &*host };
    let e = unsafe { &*e };
    let Some((mut pts, mut closed, ty)) = (unsafe { host_curve_get(host, node) }) else { return -1; };
    let indep = unsafe { read_u32(host, node, sv(b"indep_handles\0")) } != 0;
    let mode = unsafe { read_u32(host, node, sv(b"mode\0")) };
    let mut ty = ty;
    let is_bezier = ty == api::CCurveType::Bezier as u32;
    let wp = e.world_pos;

    match e.tag {
        api::CGizmoEventTag::Click => {
            if e.pick_id == 0 {
                // Click canvas: add a point
                pts.push(api::CCurveControlPoint {
                    id: api::CUuid { lo: 0, hi: 0 },
                    position: wp,
                    mode: if mode == 2 { api::CPointMode::Bezier } else { api::CPointMode::Corner },
                    _pad0: [0; 2],
                    handle_in: [-0.1, 0.0, 0.0],
                    _pad1: 0,
                    handle_out: [0.1, 0.0, 0.0],
                    _pad2: 0,
                    weight: 1.0,
                    _pad3: [0; 3],
                });
                if mode == 2 {
                    ty = api::CCurveType::Bezier as u32;
                    auto_handles(&mut pts, closed);
                }
                let _ = unsafe { host_curve_set(host, node, &pts, closed, ty) };
                return 0;
            }
            // Click first point to close in Draw/Auto modes
            if (mode == 1 || mode == 2) && e.pick_id == 1 && closed == 0 && pts.len() > 2 {
                closed = 1;
                if mode == 2 { ty = api::CCurveType::Bezier as u32; auto_handles(&mut pts, closed); }
                let _ = unsafe { host_curve_set(host, node, &pts, closed, ty) };
                return 0;
            }
        }
        api::CGizmoEventTag::Drag => {
            let pid = e.pick_id;
            if pid >= 1 && (pid as usize) <= pts.len() {
                pts[(pid - 1) as usize].position = wp;
                if mode == 2 { ty = api::CCurveType::Bezier as u32; auto_handles(&mut pts, closed); }
                let _ = unsafe { host_curve_set(host, node, &pts, closed, ty) };
                return 0;
            }
            if mode != 2 && is_bezier && pid >= 1000 && pid < 2000 {
                let i = (pid - 1000) as usize;
                if i < pts.len() {
                    let v = sub3(wp, pts[i].position);
                    pts[i].handle_in = v;
                    if !indep { pts[i].handle_out = mul3(v, -1.0); }
                    let _ = unsafe { host_curve_set(host, node, &pts, closed, ty) };
                    return 0;
                }
            }
            if mode != 2 && is_bezier && pid >= 2000 {
                let i = (pid - 2000) as usize;
                if i < pts.len() {
                    let v = sub3(wp, pts[i].position);
                    pts[i].handle_out = v;
                    if !indep { pts[i].handle_in = mul3(v, -1.0); }
                    let _ = unsafe { host_curve_set(host, node, &pts, closed, ty) };
                    return 0;
                }
            }
        }
        api::CGizmoEventTag::Release => {}
    }
    -1
}

extern "C" fn input_event(_instance: *mut c_void, host: *const api::CHostApi, node: api::CUuid, e: *const api::CInputEvent) -> i32 {
    if host.is_null() || e.is_null() { return -1; }
    let host = unsafe { &*host };
    let e = unsafe { &*e };
    if !matches!(e.tag, api::CInputEventTag::KeyDown) { return 0; }
    match e.key {
        api::CKeyCode::F => unsafe { write_u32(host, node, sv(b"mode\0"), 0) },
        api::CKeyCode::G => unsafe { write_u32(host, node, sv(b"mode\0"), 1) },
        api::CKeyCode::H => unsafe {
            write_u32(host, node, sv(b"mode\0"), 2);
            if let Some((mut pts, closed, _ty)) = host_curve_get(host, node) {
                auto_handles(&mut pts, closed);
                let _ = host_curve_set(host, node, &pts, closed, api::CCurveType::Bezier as u32);
            }
        },
        api::CKeyCode::K => {
            let v = unsafe { read_u32(host, node, sv(b"indep_handles\0")) };
            unsafe { write_u32(host, node, sv(b"indep_handles\0"), if v == 0 { 1 } else { 0 }) };
        }
        _ => {}
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Host {
        points: Vec<[f32; 3]>,
        prim_polyline: Option<(Vec<u32>, u32)>,
        attr_p: Vec<f32>,
        attr_n: Vec<f32>,
        state: HashMap<(u64, u64, &'static str), Vec<u8>>,
        curve: Vec<api::CCurveControlPoint>,
        curve_closed: u32,
        curve_ty: u32,
    }

    fn key_of(node: api::CUuid, k: api::CStringView) -> (u64, u64, &'static str) {
        let s = unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(k.ptr as *const u8, k.len as usize)) };
        // keys in this test are static literals
        (node.lo, node.hi, Box::leak(s.to_string().into_boxed_str()))
    }

    extern "C" fn geo_create(u: *mut c_void) -> api::GeoHandle { let _ = u; 1 }
    extern "C" fn geo_clone(_u: *mut c_void, _src: api::GeoHandle) -> api::GeoHandle { 1 }
    extern "C" fn geo_drop(_u: *mut c_void, _h: api::GeoHandle) {}
    extern "C" fn geo_point_count(u: *mut c_void, _h: api::GeoHandle) -> u32 { unsafe { (&*(u as *mut Host)).points.len() as u32 } }
    extern "C" fn geo_vertex_count(_u: *mut c_void, _h: api::GeoHandle) -> u32 { 0 }
    extern "C" fn geo_prim_count(u: *mut c_void, _h: api::GeoHandle) -> u32 { unsafe { if (&*(u as *mut Host)).prim_polyline.is_some() { 1 } else { 0 } } }
    extern "C" fn geo_edge_count(_u: *mut c_void, _h: api::GeoHandle) -> u32 { 0 }
    extern "C" fn geo_add_point(u: *mut c_void, _h: api::GeoHandle) -> u32 {
        unsafe {
            let h = &mut *(u as *mut Host);
            let id = h.points.len() as u32;
            h.points.push([0.0, 0.0, 0.0]);
            id
        }
    }
    extern "C" fn geo_add_vertex(_u: *mut c_void, _h: api::GeoHandle, _pd: u32) -> u32 { 0 }
    extern "C" fn geo_remove_point(_u: *mut c_void, _h: api::GeoHandle, _pd: u32) {}
    extern "C" fn geo_remove_vertex(_u: *mut c_void, _h: api::GeoHandle, _vd: u32) {}
    extern "C" fn geo_add_edge(_u: *mut c_void, _h: api::GeoHandle, _p0: u32, _p1: u32) -> u32 { 0 }
    extern "C" fn geo_remove_edge(_u: *mut c_void, _h: api::GeoHandle, _ed: u32) {}
    extern "C" fn geo_add_polygon(_u: *mut c_void, _h: api::GeoHandle, _pd: *const u32, _pl: u32) -> u32 { 0 }
    extern "C" fn geo_add_polyline(u: *mut c_void, _h: api::GeoHandle, pd: *const u32, pl: u32, closed: u32) -> u32 {
        unsafe {
            let h = &mut *(u as *mut Host);
            let pts = if pd.is_null() || pl == 0 { Vec::new() } else { core::slice::from_raw_parts(pd, pl as usize).to_vec() };
            h.prim_polyline = Some((pts, closed));
            1
        }
    }
    extern "C" fn geo_set_prim_vertices(_u: *mut c_void, _h: api::GeoHandle, _prim: u32, _vtx: *const u32, _vl: u32) {}
    extern "C" fn geo_remove_prim(_u: *mut c_void, _h: api::GeoHandle, _prim: u32) {}
    extern "C" fn geo_vertex_point(_u: *mut c_void, _h: api::GeoHandle, _vd: u32) -> u32 { 0 }
    extern "C" fn geo_edge_points(_u: *mut c_void, _h: api::GeoHandle, _ed: u32, _out: *mut u32) -> u32 { 0 }
    extern "C" fn geo_prim_point_count(_u: *mut c_void, _h: api::GeoHandle, _prim: u32) -> u32 { 0 }
    extern "C" fn geo_prim_points(_u: *mut c_void, _h: api::GeoHandle, _prim: u32, _out: *mut u32, _cap: u32) -> u32 { 0 }
    extern "C" fn geo_get_point_position(_u: *mut c_void, _h: api::GeoHandle, _pd: u32, _out: *mut f32) -> u32 { 0 }
    extern "C" fn geo_set_point_position(_u: *mut c_void, _h: api::GeoHandle, _pd: u32, _xyz: *const f32) -> u32 { 0 }

    extern "C" fn attr_ensure(u: *mut c_void, _h: api::GeoHandle, domain: api::CAttrDomain, ty: api::CAttrType, name: api::CStringView, len: u32) -> api::CGeoSlice {
        unsafe {
            let h = &mut *(u as *mut Host);
            let nm = core::str::from_utf8_unchecked(core::slice::from_raw_parts(name.ptr as *const u8, name.len as usize));
            if domain != api::CAttrDomain::Point || ty != api::CAttrType::Vec3 { return api::CGeoSlice { ptr: core::ptr::null_mut(), len: 0, stride: 0 }; }
            let want = (len as usize) * 3;
            if nm == "@P" {
                if h.attr_p.len() < want { h.attr_p.resize(want, 0.0); }
                return api::CGeoSlice { ptr: h.attr_p.as_mut_ptr() as *mut c_void, len, stride: 12 };
            }
            if nm == "@N" {
                if h.attr_n.len() < want { h.attr_n.resize(want, 0.0); }
                return api::CGeoSlice { ptr: h.attr_n.as_mut_ptr() as *mut c_void, len, stride: 12 };
            }
            api::CGeoSlice { ptr: core::ptr::null_mut(), len: 0, stride: 0 }
        }
    }
    extern "C" fn attr_view(_u: *mut c_void, _h: api::GeoHandle, _d: api::CAttrDomain, _t: api::CAttrType, _n: api::CStringView) -> api::CGeoSlice { api::CGeoSlice { ptr: core::ptr::null_mut(), len: 0, stride: 0 } }
    extern "C" fn attr_bool_get(_u: *mut c_void, _h: api::GeoHandle, _d: api::CAttrDomain, _n: api::CStringView, _i: u32) -> u32 { 0 }
    extern "C" fn attr_bool_set(_u: *mut c_void, _h: api::GeoHandle, _d: api::CAttrDomain, _n: api::CStringView, _i: u32, _v: u32) -> u32 { 0 }
    extern "C" fn attr_string_get(_u: *mut c_void, _h: api::GeoHandle, _d: api::CAttrDomain, _n: api::CStringView, _i: u32, _out: *mut u8, _cap: u32) -> u32 { 0 }
    extern "C" fn attr_string_set(_u: *mut c_void, _h: api::GeoHandle, _d: api::CAttrDomain, _n: api::CStringView, _i: u32, _v: api::CStringView) -> u32 { 0 }

    extern "C" fn node_state_get(u: *mut c_void, node: api::CUuid, key: api::CStringView, out_ptr: *mut u8, out_cap: u32) -> u32 {
        unsafe {
            let h = &mut *(u as *mut Host);
            let k = key_of(node, key);
            let v = h.state.get(&k).cloned().unwrap_or_default();
            if !out_ptr.is_null() && out_cap > 0 {
                let n = (v.len() as u32).min(out_cap) as usize;
                core::ptr::copy_nonoverlapping(v.as_ptr(), out_ptr, n);
            }
            v.len() as u32
        }
    }
    extern "C" fn node_state_set(u: *mut c_void, node: api::CUuid, key: api::CStringView, bytes: *const u8, len: u32) -> u32 {
        unsafe {
            let h = &mut *(u as *mut Host);
            let k = key_of(node, key);
            let b = if bytes.is_null() || len == 0 { Vec::new() } else { core::slice::from_raw_parts(bytes, len as usize).to_vec() };
            h.state.insert(k, b);
            1
        }
    }
    extern "C" fn node_curve_get(u: *mut c_void, _node: api::CUuid, _param: api::CStringView, out_pts: *mut api::CCurveControlPoint, out_cap: u32, out_len: *mut u32, out_closed: *mut u32, out_ty: *mut u32) -> u32 {
        unsafe {
            let h = &mut *(u as *mut Host);
            if !out_len.is_null() { *out_len = h.curve.len() as u32; }
            if !out_closed.is_null() { *out_closed = h.curve_closed; }
            if !out_ty.is_null() { *out_ty = h.curve_ty; }
            if !out_pts.is_null() && out_cap > 0 {
                let n = (h.curve.len() as u32).min(out_cap) as usize;
                core::ptr::copy_nonoverlapping(h.curve.as_ptr(), out_pts, n);
            }
            1
        }
    }
    extern "C" fn node_curve_set(u: *mut c_void, _node: api::CUuid, _param: api::CStringView, pts: *const api::CCurveControlPoint, len: u32, closed: u32, ty: u32) -> u32 {
        unsafe {
            let h = &mut *(u as *mut Host);
            let s = if pts.is_null() || len == 0 { &[] } else { core::slice::from_raw_parts(pts, len as usize) };
            h.curve = s.to_vec();
            h.curve_closed = closed;
            h.curve_ty = ty;
            1
        }
    }

    fn host_api(h: &mut Host) -> api::CHostApi {
        api::CHostApi {
            userdata: h as *mut _ as *mut c_void,
            geo_create,
            geo_clone,
            geo_drop,
            geo_point_count,
            geo_vertex_count,
            geo_prim_count,
            geo_edge_count,
            geo_add_point,
            geo_add_vertex,
            geo_remove_point,
            geo_remove_vertex,
            geo_add_edge,
            geo_remove_edge,
            geo_add_polygon,
            geo_add_polyline,
            geo_set_prim_vertices,
            geo_remove_prim,
            geo_vertex_point,
            geo_edge_points,
            geo_prim_point_count,
            geo_prim_points,
            geo_get_point_position,
            geo_set_point_position,
            attr_ensure,
            attr_view,
            attr_bool_get,
            attr_bool_set,
            attr_string_get,
            attr_string_set,
            node_state_get,
            node_state_set,
            node_curve_get,
            node_curve_set,
        }
    }

    #[test]
    fn smoke_compute_polygon() {
        let mut h = Host::default();
        let host = host_api(&mut h);
        let curve_pts = vec![
            api::CCurveControlPoint { id: api::CUuid { lo: 1, hi: 2 }, position: [0.0, 0.0, 0.0], mode: api::CPointMode::Corner, _pad0: [0; 2], handle_in: [0.0; 3], _pad1: 0, handle_out: [0.0; 3], _pad2: 0, weight: 1.0, _pad3: [0; 3] },
            api::CCurveControlPoint { id: api::CUuid { lo: 3, hi: 4 }, position: [1.0, 0.0, 0.0], mode: api::CPointMode::Corner, _pad0: [0; 2], handle_in: [0.0; 3], _pad1: 0, handle_out: [0.0; 3], _pad2: 0, weight: 1.0, _pad3: [0; 3] },
        ];
        let data = api::CCurveData { pts: curve_pts.as_ptr(), len: curve_pts.len() as u32, closed: 0, curve_type: api::CCurveType::Polygon as u32 };
        let pv = api::CParamValue { tag: api::CParamTag::Curve, _pad0: [0; 3], a: (&data as *const api::CCurveData as usize as u64), b: 0 };
        let mut out = 0u64;
        assert_eq!(compute(core::ptr::null_mut(), &host, core::ptr::null(), core::ptr::null(), 0, &pv as *const _, 1, &mut out as *mut _), 0);
        assert_eq!(h.points.len(), 2);
        assert!(h.prim_polyline.is_some());
    }

    #[test]
    fn smoke_input_and_click_add_point() {
        let mut h = Host::default();
        h.curve_ty = api::CCurveType::Bezier as u32;
        h.curve = vec![api::CCurveControlPoint { id: api::CUuid { lo: 1, hi: 1 }, position: [0.0, 0.0, 0.0], mode: api::CPointMode::Corner, _pad0: [0; 2], handle_in: [0.0; 3], _pad1: 0, handle_out: [0.0; 3], _pad2: 0, weight: 1.0, _pad3: [0; 3] }];
        let host = host_api(&mut h);
        let node = api::CUuid { lo: 9, hi: 9 };

        let e = api::CInputEvent { tag: api::CInputEventTag::KeyDown, key: api::CKeyCode::G, _pad0: [0; 2] };
        assert_eq!(input_event(core::ptr::null_mut(), &host, node, &e as *const _), 0);
        // Click on empty (pick_id=0) should add a point
        let ge = api::CGizmoEvent { tag: api::CGizmoEventTag::Click, pick_id: 0, world_pos: [1.0, 0.0, 0.0], _pad0: 0 };
        assert_eq!(gizmo_event(core::ptr::null_mut(), &host, node, &ge as *const _), 0);
        assert_eq!(h.curve.len(), 2);
    }
}

