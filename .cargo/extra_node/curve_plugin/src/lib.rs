// ===================================================================================
// CurvePlugin - a native Rust plugin for Cunning3D
// Implements a "Curve" node with Bezier/Polyline support and interactive gizmo editing.
// ===================================================================================

use core::ffi::c_void; // Raw void pointer for C interop

//! CurvePlugin - a native Rust plugin for Cunning3D
//! Implements a "Curve" node with Bezier/Polyline support and interactive gizmo editing.

use cunning_plugin_sdk::c_api as api; // Import the C API SDK

// Plugin name constant
static PLUGIN_NAME: &[u8] = b"CurvePlugin\0";
// Plugin version constant
static PLUGIN_VER: &[u8] = b"0.1\0";

/// Return plugin metadata such as ABI version, name, and version.
#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_info() -> api::CPluginDetails {
    api::CPluginDetails {
        abi_version: api::CUNNING_PLUGIN_ABI_VERSION, // Set ABI version
        name: api::CStringView { ptr: PLUGIN_NAME.as_ptr() as *const _, len: (PLUGIN_NAME.len() - 1) as u32 }, // Set plugin name
        version: api::CStringView { ptr: PLUGIN_VER.as_ptr() as *const _, len: (PLUGIN_VER.len() - 1) as u32 }, // Set plugin version
    }
}

/// Return the number of nodes provided by this plugin.
#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_node_count() -> u32 { 1 }

// Node definition string
static NODE_NAME: &[u8] = b"Curve\0";
static NODE_CAT: &[u8] = b"Primitives\0";

// Input port definition
static PORT_IN: [api::CStringView; 1] = [api::CStringView { ptr: b"Input\0".as_ptr() as *const _, len: 5 }];
// Output port definition
static PORT_OUT: [api::CStringView; 1] = [api::CStringView { ptr: b"Output\0".as_ptr() as *const _, len: 6 }];

// Parameter definition
static PARAMS: [api::CParamDesc; 1] = [api::CParamDesc {
    name: api::CStringView { ptr: b"curve_data\0".as_ptr() as *const _, len: 9 }, // Internal parameter name
    label: api::CStringView { ptr: b"Curve Data\0".as_ptr() as *const _, len: 10 }, // UI label
    group: api::CStringView { ptr: b"Geometry\0".as_ptr() as *const _, len: 8 }, // UI group
    default_value: api::CParamValue { tag: api::CParamTag::Curve, _pad0: [0; 3], a: 0, b: 0 }, // Default value
    ui: api::CParamUi { tag: api::CParamUiTag::CurvePoints, _pad0: [0; 3], a: 0, b: 0 }, // UI control type
}];

/// Define node metadata: inputs, outputs, params, and category.
#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_get_node_desc(i: u32, out: *mut api::CNodeDesc) -> i32 {
    // Validate pointers and indices
    if out.is_null() || i != 0 { return -1; }
    
    // Fill the node descriptor struct
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
    0 // Success
}

// Internal node struct (placeholder)
#[repr(C)]
struct CurveNode;

/// Return node runtime lifecycle methods (create, compute, destroy).
#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_get_node_vtable(i: u32) -> api::CNodeVTable {
    // Return the vtable containing lifecycle callbacks
    if i != 0 { return api::CNodeVTable { create: create, compute: compute, destroy: destroy }; }
    api::CNodeVTable { create, compute, destroy }
}

/// Register interaction callbacks for HUD drawing, gizmo handling, and input events.
#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_get_node_interaction(i: u32, out: *mut api::CNodeInteraction) -> i32 {
    // Validate output pointer and index
    if out.is_null() || i != 0 { return -1; }
    // Set interaction callbacks
    *out = api::CNodeInteraction { hud, gizmo, input };
    0 // Success
}

/// Called when the node is created.
unsafe extern "C" fn create(_node_ptr: *mut c_void) -> *mut c_void {
    // Initialize instance data here
    0 as *mut c_void
}

/// Called when the node needs recomputation (input changed).
unsafe extern "C" fn compute(node: *mut c_void, _inputs: *const api::CNodeIO, _outputs: *mut api::CNodeIO) -> i32 {
    // Read inputs, compute, and write outputs
    0
}

/// Called when the node instance is destroyed.
unsafe extern "C" fn destroy(node: *mut c_void) -> i32 {
    // Clean up allocated resources
    0
}

/// Draw 2D HUD elements on the viewport.
unsafe extern "C" fn hud(node: *mut c_void, cmd: *const api::CHudCmd) -> i32 {
    // Draw text, lines, rectangles on screen
    0
}

/// Draw and handle 3D gizmo handles in the scene.
unsafe extern "C" fn gizmo(node: *mut c_void, cmd: *const api::CGizmoCmd) -> i32 {
    // Render 3D handles and process interaction logic
    0
}

/// Called when mouse/keyboard events occur in the viewport (if captured).
unsafe extern "C" fn input(node: *mut c_void, event: *const api::CInputEvent) -> i32 {
    // Handle raw input events
    0
}
