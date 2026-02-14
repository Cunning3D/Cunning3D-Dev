// ===================================================================================
// CurvePlugin - Cunning3D 的原生 Rust 插件
// 实现了支持 Bezier/Polyline 和交互式 Gizmo 编辑的 "Curve" 节点。
// ===================================================================================

use core::ffi::c_void; // 导入用于 C 互操作的原始 void 指针

//! CurvePlugin - Cunning3D 的原生 Rust 插件
//! 实现了支持 Bezier/Polyline 和交互式 Gizmo 编辑的 "Curve" 节点。

use cunning_plugin_sdk::c_api as api; // 导入 C API SDK

// 插件名称常量
static PLUGIN_NAME: &[u8] = b"CurvePlugin\0";
// 插件版本常量
static PLUGIN_VER: &[u8] = b"0.1\0";

/// 返回插件详细信息，如 ABI 版本、名称和版本号。
#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_info() -> api::CPluginDetails {
    api::CPluginDetails {
        abi_version: api::CUNNING_PLUGIN_ABI_VERSION, // 设置 ABI 版本
        name: api::CStringView { ptr: PLUGIN_NAME.as_ptr() as *const _, len: (PLUGIN_NAME.len() - 1) as u32 }, // 设置插件名称
        version: api::CStringView { ptr: PLUGIN_VER.as_ptr() as *const _, len: (PLUGIN_VER.len() - 1) as u32 }, // 设置插件版本
    }
}

/// 返回此插件提供的节点数量。
#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_node_count() -> u32 { 1 }

// 节点定义字符串
static NODE_NAME: &[u8] = b"Curve\0";
static NODE_CAT: &[u8] = b"Primitives\0";

// 输入端口定义
static PORT_IN: [api::CStringView; 1] = [api::CStringView { ptr: b"Input\0".as_ptr() as *const _, len: 5 }];
// 输出端口定义
static PORT_OUT: [api::CStringView; 1] = [api::CStringView { ptr: b"Output\0".as_ptr() as *const _, len: 6 }];

// 参数定义
static PARAMS: [api::CParamDesc; 1] = [api::CParamDesc {
    name: api::CStringView { ptr: b"curve_data\0".as_ptr() as *const _, len: 9 }, // 参数内部名称
    label: api::CStringView { ptr: b"Curve Data\0".as_ptr() as *const _, len: 10 }, // UI 标签
    group: api::CStringView { ptr: b"Geometry\0".as_ptr() as *const _, len: 8 }, // UI 分组
    default_value: api::CParamValue { tag: api::CParamTag::Curve, _pad0: [0; 3], a: 0, b: 0 }, // 默认值
    ui: api::CParamUi { tag: api::CParamUiTag::CurvePoints, _pad0: [0; 3], a: 0, b: 0 }, // UI 控件类型
}];

/// 定义节点元数据：输入、输出、参数和类别。
#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_get_node_desc(i: u32, out: *mut api::CNodeDesc) -> i32 {
    // 检查指针和索引的有效性
    if out.is_null() || i != 0 { return -1; }
    
    // 填充节点描述结构体
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
    0 // 返回成功
}

// 内部节点结构体（占位符）
#[repr(C)]
struct CurveNode;

/// 返回节点运行时生命周期方法（create, compute, destroy）。
#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_get_node_vtable(i: u32) -> api::CNodeVTable {
    // 返回包含生命周期回调的虚函数表
    if i != 0 { return api::CNodeVTable { create: create, compute: compute, destroy: destroy }; }
    api::CNodeVTable { create, compute, destroy }
}

/// 注册用于 HUD 绘制、Gizmo 处理和输入事件的交互回调。
#[no_mangle]
pub unsafe extern "C" fn cunning_plugin_get_node_interaction(i: u32, out: *mut api::CNodeInteraction) -> i32 {
    // 检查输出指针和索引
    if out.is_null() || i != 0 { return -1; }
    // 设置交互回调
    *out = api::CNodeInteraction { hud, gizmo, input };
    0 // 返回成功
}

/// 当节点被创建时调用。
unsafe extern "C" fn create(_node_ptr: *mut c_void) -> *mut c_void {
    // 在此初始化实例数据
    0 as *mut c_void
}

/// 当节点需要重新计算（输入改变）时调用。
unsafe extern "C" fn compute(node: *mut c_void, _inputs: *const api::CNodeIO, _outputs: *mut api::CNodeIO) -> i32 {
    // 读取输入，执行计算，写入输出
    0
}

/// 当节点实例被销毁时调用。
unsafe extern "C" fn destroy(node: *mut c_void) -> i32 {
    // 清理已分配的资源
    0
}

/// 用于在视口上绘制 2D HUD 元素。
unsafe extern "C" fn hud(node: *mut c_void, cmd: *const api::CHudCmd) -> i32 {
    // 在屏幕上绘制文本、线条、矩形
    0
}

/// 用于在场景中绘制和处理 3D Gizmo（��柄）。
unsafe extern "C" fn gizmo(node: *mut c_void, cmd: *const api::CGizmoCmd) -> i32 {
    // 绘制 3D 手柄并处理交互逻辑
    0
}

/// 当视口中发生鼠标或键盘事件（如果被捕获）时调用。
unsafe extern "C" fn input(node: *mut c_void, event: *const api::CInputEvent) -> i32 {
    // 处理原始输入事件
    0
}
