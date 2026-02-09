use super::definitions::{
    Tool, ToolContext, ToolDefinition, ToolError, ToolLog, ToolLogLevel, ToolOutput,
};
use super::diff::compute_file_diff;
use crate::cunning_core::plugin_system::{rust_build, PluginSystem};
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::nodes::parameter::ParameterValue;
use crate::nodes::PortId;
use crate::nodes::{Connection, Node, NodeGraph, NodeType};
use bevy_egui::egui::Pos2;
use serde::Deserialize;
use serde_json::{json, Value};
use serde_path_to_error;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

fn trunc_mid(s: &str, max_chars: usize) -> String {
    let n = s.chars().count();
    if n <= max_chars {
        return s.to_string();
    }
    let head = max_chars.saturating_sub(1200).max(600);
    let tail = 900usize.min(max_chars.saturating_sub(head + 50));
    let h: String = s.chars().take(head).collect();
    let t: String = s
        .chars()
        .rev()
        .take(tail)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!(
        "{}\n...<truncated {} chars>...\n{}",
        h,
        n.saturating_sub(head + tail),
        t
    )
}

fn parse_cargo_errors(build_log: &str, max: usize) -> Vec<Value> {
    let mut out = Vec::new();
    let mut last_code: Option<String> = None;
    let mut last_msg: Option<String> = None;

    for line in build_log.lines() {
        let l = line.trim();
        if l.starts_with("error[") || l.starts_with("error[E") || l.starts_with("error:") {
            // error[E0308]: mismatched types
            let code = if let Some(a) = l
                .find('[')
                .and_then(|i| l[i + 1..].find(']').map(|j| (i + 1, i + 1 + j)))
            {
                Some(l[a.0..a.1].to_string())
            } else {
                None
            };
            let msg = l.splitn(2, ": ").nth(1).unwrap_or(l).to_string();
            last_code = code;
            last_msg = Some(msg);
        }

        if l.starts_with("-->") || l.starts_with("-- >") || l.contains(" --> ") {
            let loc = l.split(" --> ").nth(1).unwrap_or("").trim();
            // path:line:col
            let mut file = String::new();
            let mut line_no: Option<u32> = None;
            let mut col_no: Option<u32> = None;
            let mut parts = loc.rsplitn(3, ':').collect::<Vec<_>>();
            if parts.len() == 3 {
                col_no = parts[0].trim().parse::<u32>().ok();
                line_no = parts[1].trim().parse::<u32>().ok();
                file = parts[2].trim().to_string();
            } else {
                file = loc.to_string();
            }

            if let Some(msg) = last_msg.take() {
                out.push(json!({
                    "file": file,
                    "line": line_no,
                    "col": col_no,
                    "code": last_code.clone().unwrap_or_default(),
                    "message": msg,
                }));
                if out.len() >= max {
                    break;
                }
            }
        }
    }

    // Fallback: if we saw error lines but no locations, emit them anyway.
    if out.is_empty() {
        for line in build_log.lines().map(|l| l.trim()).filter(|l| {
            l.starts_with("error[") || l.starts_with("error[E") || l.starts_with("error:")
        }) {
            let code = if let Some(a) = line
                .find('[')
                .and_then(|i| line[i + 1..].find(']').map(|j| (i + 1, i + 1 + j)))
            {
                line[a.0..a.1].to_string()
            } else {
                String::new()
            };
            let msg = line.splitn(2, ": ").nth(1).unwrap_or(line).to_string();
            out.push(json!({"file":"","line":null,"col":null,"code":code,"message":msg}));
            if out.len() >= max {
                break;
            }
        }
    }
    out
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn extract_region_with_lines(s: &str, begin: &str, end: &str) -> Option<(usize, usize, String)> {
    let i0 = s.find(begin)? + begin.len();
    let i1 = s.find(end)?;
    if i1 <= i0 {
        return None;
    }
    let begin_line = s[..i0].lines().count();
    let end_line = s[..i1].lines().count().max(begin_line + 1);
    Some((begin_line, end_line, s[i0..i1].to_string()))
}

fn parse_hex_u64(s: &str) -> Option<u64> {
    let t = s.trim().trim_start_matches("0x");
    u64::from_str_radix(t, 16).ok()
}

fn find_region_bounds<'a>(
    s: &'a str,
    begin: &str,
    end: &str,
) -> Result<(usize, usize, &'a str), ToolError> {
    let i0 = s
        .find(begin)
        .ok_or_else(|| ToolError(format!("Region markers not found: {} .. {}", begin, end)))?
        + begin.len();
    let rel = s[i0..]
        .find(end)
        .ok_or_else(|| ToolError(format!("Region markers not found: {} .. {}", begin, end)))?;
    let i1 = i0 + rel;
    if i1 <= i0 {
        return Err(ToolError("Invalid region markers order".into()));
    }
    Ok((i0, i1, &s[i0..i1]))
}

#[derive(Deserialize)]
struct ExtractUserCodeArgs {
    plugin_name: String,
    #[serde(default)]
    region: Option<String>,
}

pub struct ExtractUserCodeTool;

impl Tool for ExtractUserCodeTool {
    fn name(&self) -> &str {
        "extract_user_code"
    }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Extract USER_CODE (or USER_INTERACTION_CODE) region from a Rust plugin lib.rs, returning line range + stable hash. Use this before PATCH mode edits.".to_string(),
            parameters: json!({"type":"object","properties":{"plugin_name":{"type":"string"},"region":{"type":"string","description":"user (default) | interaction"}},"required":["plugin_name"]}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let a: ExtractUserCodeArgs =
            serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid args: {e}")))?;
        if a.plugin_name.contains("..")
            || a.plugin_name.contains('/')
            || a.plugin_name.contains('\\')
        {
            return Err(ToolError("Invalid plugin_name".to_string()));
        }
        let region = a
            .region
            .unwrap_or_else(|| "user".to_string())
            .to_ascii_lowercase();
        let lib_path = std::path::Path::new("plugins/extra_node")
            .join(&a.plugin_name)
            .join("src/lib.rs");
        let s = std::fs::read_to_string(&lib_path)
            .map_err(|e| ToolError(format!("Read lib.rs failed: {e}")))?;
        let (begin, end) = if region == "interaction" {
            (
                "// === USER_INTERACTION_CODE_BEGIN ===",
                "// === USER_INTERACTION_CODE_END ===",
            )
        } else {
            ("// === USER_CODE_BEGIN ===", "// === USER_CODE_END ===")
        };
        let (l0, l1, code) = extract_region_with_lines(&s, begin, end)
            .ok_or_else(|| ToolError(format!("Region markers not found: {} .. {}", begin, end)))?;
        let h = fnv1a64(code.as_bytes());
        let excerpt = trunc_mid(&code, 14000);
        let diag = json!({"plugin_name": a.plugin_name, "region": region, "begin_line": l0, "end_line": l1, "hash_fnv1a64": format!("{:016x}", h), "char_len": code.chars().count()});
        Ok(ToolOutput::new(
            format!(
                "EXTRACTED USER REGION.\n\nAI_DIAG_JSON:\n{}\n\nCODE:\n{}",
                serde_json::to_string_pretty(&diag).unwrap_or_else(|_| "{}".to_string()),
                excerpt
            ),
            vec![ToolLog {
                message: format!("Extracted {} region from {}", region, lib_path.display()),
                level: ToolLogLevel::Success,
            }],
        ))
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
enum ParamType {
    Int,
    Float,
    Bool,
    String,
    Vec2,
    Vec3,
    Vec4,
    Color3,
    Color4,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
enum UiKind {
    None,
    FloatSlider,
    IntSlider,
    Vec2Drag,
    Vec3Drag,
    Vec4Drag,
    String,
    Toggle,
    Dropdown,
    Color,
    Code,
}

#[derive(Deserialize, Clone)]
struct UiSpec {
    kind: UiKind,
    #[serde(default)]
    min: Option<f32>,
    #[serde(default)]
    max: Option<f32>,
    #[serde(default)]
    show_alpha: Option<bool>,
}

#[derive(Deserialize, Clone)]
struct ParamSpec {
    name: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(rename = "type")]
    ty: ParamType,
    #[serde(default)]
    default: Value,
    #[serde(default)]
    ui: Option<UiSpec>,
}

#[derive(Deserialize, Clone)]
struct NodeSpec {
    name: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    inputs: Vec<String>,
    #[serde(default)]
    outputs: Vec<String>,
    #[serde(default)]
    params: Vec<ParamSpec>,
}

// === Interaction Spec (HUD/Gizmo/Input) ===
#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
enum HudCmdTag {
    #[default]
    Label,
    Button,
    Toggle,
    Separator,
}

#[derive(Deserialize, Clone)]
struct HudCmdSpec {
    tag: HudCmdTag,
    #[serde(default)]
    id: u32,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
enum GizmoPrimitive {
    #[default]
    Sphere,
    Cube,
    Cylinder,
    Cone,
    Plane,
}

#[derive(Deserialize, Clone)]
struct GizmoPrimSpec {
    pick_id: u32,
    primitive: GizmoPrimitive,
    #[serde(default)]
    position: [f32; 3],
    #[serde(default = "default_scale")]
    scale: [f32; 3],
    #[serde(default = "default_color")]
    color: [f32; 4],
}
fn default_scale() -> [f32; 3] {
    [0.05, 0.05, 0.05]
}
fn default_color() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}

#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
enum InputKeyCode {
    #[default]
    F,
    G,
    H,
    K,
    Space,
    Enter,
    Escape,
    Delete,
    Backspace,
}

#[derive(Deserialize, Clone)]
struct InputKeySpec {
    key: InputKeyCode,
    #[serde(default)]
    action: String,
}

#[derive(Deserialize, Clone, Default)]
struct InteractionSpec {
    #[serde(default)]
    hud_commands: Vec<HudCmdSpec>,
    #[serde(default)]
    gizmo_primitives: Vec<GizmoPrimSpec>,
    #[serde(default)]
    input_keys: Vec<InputKeySpec>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
enum SmokeInputKind {
    CreateCube,
    CreateSphere,
    Empty,
}

#[derive(Deserialize, Clone)]
struct SmokeInputSpec {
    kind: SmokeInputKind,
    #[serde(default)]
    params: HashMap<String, Value>,
}

#[derive(Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AssertSpec {
    NonEmpty,
    PointCountEqInput,
    PrimCountEqInput,
    BboxScalesBy {
        factor: f32,
        #[serde(default)]
        tolerance: Option<f32>,
    },
}

#[derive(Deserialize, Clone)]
struct SmokeTestSpec {
    input: SmokeInputSpec,
    #[serde(default)]
    plugin_params: HashMap<String, Value>,
    #[serde(default)]
    asserts: Vec<AssertSpec>,
}

// Patch mode for efficient updates (only modify USER_CODE region)
#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
enum NodeSpecMode {
    #[default]
    Create,
    Patch,
}

#[derive(Deserialize, Clone)]
struct UserCodePatch {
    old: String,
    new: String,
}

#[derive(Deserialize, Clone)]
struct RustNodeSpec {
    plugin_name: String,
    #[serde(default)]
    mode: NodeSpecMode,
    #[serde(default)]
    node: Option<NodeSpec>,
    #[serde(default)]
    user_code: Option<String>, // Direct replacement of USER_CODE region
    #[serde(default)]
    user_code_patch: Option<UserCodePatch>, // Patch mode: old->new
    #[serde(default)]
    expected_user_code_hash_fnv1a64: Option<String>, // Safety guard for PATCH mode.
    #[serde(default)]
    interaction: Option<InteractionSpec>,
    #[serde(default)]
    smoke_test: Option<SmokeTestSpec>,
    #[serde(default)]
    release: Option<bool>,
    #[serde(default)]
    offline: Option<bool>,
    #[serde(default)]
    locked: Option<bool>,
}

fn bstr(name: &str) -> String {
    name.as_bytes()
        .iter()
        .flat_map(|b| std::ascii::escape_default(*b))
        .map(|c| c as char)
        .collect()
}

fn gen_cstring_view(name: &str, sym: &str) -> String {
    format!("static {sym}: &[u8] = b\"{}\";\nstatic {sym}_SV: CStringView = CStringView {{ ptr: {sym}.as_ptr() as *const i8, len: {sym}.len() as u32 }};\n", bstr(name))
}

fn gen_param_value(p: &ParamSpec) -> (String, String) {
    let dv = &p.default;
    match p.ty {
        ParamType::Int => {
            let v = dv.as_i64().unwrap_or(0);
            ("CParamTag::Int".to_string(), format!("CParamValue {{ tag: {0}, _pad0: [0;3], a: {1}i64 as u64, b: 0 }}", "CParamTag::Int", v))
        }
        ParamType::Float => {
            let v = dv.as_f64().unwrap_or(0.0) as f32;
            ("CParamTag::Float".to_string(), format!("CParamValue {{ tag: {0}, _pad0: [0;3], a: ({1}f32).to_bits() as u64, b: 0 }}", "CParamTag::Float", v))
        }
        ParamType::Bool => {
            let v = dv.as_bool().unwrap_or(false);
            ("CParamTag::Bool".to_string(), format!("CParamValue {{ tag: {0}, _pad0: [0;3], a: {1}u64, b: 0 }}", "CParamTag::Bool", if v { 1 } else { 0 }))
        }
        ParamType::Vec2 => {
            let a = dv.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let b = dv.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            ("CParamTag::Vec2".to_string(), format!("CParamValue {{ tag: {0}, _pad0: [0;3], a: ({1}u64) | (({2}u64)<<32), b: 0 }}", "CParamTag::Vec2", a.to_bits(), b.to_bits()))
        }
        ParamType::Vec3 | ParamType::Color3 => {
            let a0 = dv.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let a1 = dv.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let a2 = dv.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let tag = if matches!(p.ty, ParamType::Color3) { "CParamTag::Color3" } else { "CParamTag::Vec3" };
            (tag.to_string(), format!("CParamValue {{ tag: {0}, _pad0: [0;3], a: ({1}u64) | (({2}u64)<<32), b: {3}u64 }}", tag, a0.to_bits(), a1.to_bits(), a2.to_bits()))
        }
        ParamType::Vec4 | ParamType::Color4 => {
            let a0 = dv.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let a1 = dv.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let a2 = dv.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let a3 = dv.get(3).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let tag = if matches!(p.ty, ParamType::Color4) { "CParamTag::Color4" } else { "CParamTag::Vec4" };
            (tag.to_string(), format!("CParamValue {{ tag: {0}, _pad0: [0;3], a: ({1}u64) | (({2}u64)<<32), b: ({3}u64) | (({4}u64)<<32) }}", tag, a0.to_bits(), a1.to_bits(), a2.to_bits(), a3.to_bits()))
        }
        ParamType::String => {
            ("CParamTag::String".to_string(), format!("CParamValue {{ tag: {0}, _pad0: [0;3], a: {1}.as_ptr() as usize as u64, b: {1}.len() as u64 }}", "CParamTag::String", format!("P_{}_DEF", p.name.to_uppercase().replace(|c: char| !c.is_ascii_alphanumeric(), "_"))))
        }
    }
}

fn gen_param_ui(p: &ParamSpec) -> String {
    let Some(ui) = &p.ui else {
        return "CParamUi { tag: CParamUiTag::None, _pad0: [0;3], a: 0, b: 0 }".to_string();
    };
    match ui.kind {
        UiKind::FloatSlider => {
            let (mn, mx) = (ui.min.unwrap_or(0.0), ui.max.unwrap_or(1.0));
            format!("CParamUi {{ tag: CParamUiTag::FloatSlider, _pad0: [0;3], a: ({mn}f32).to_bits() as u64, b: ({mx}f32).to_bits() as u64 }}")
        }
        UiKind::IntSlider => {
            let (mn, mx) = (ui.min.unwrap_or(0.0) as i64, ui.max.unwrap_or(10.0) as i64);
            format!("CParamUi {{ tag: CParamUiTag::IntSlider, _pad0: [0;3], a: {mn}i64 as u64, b: {mx}i64 as u64 }}")
        }
        UiKind::Vec2Drag => {
            "CParamUi { tag: CParamUiTag::Vec2Drag, _pad0: [0;3], a: 0, b: 0 }".to_string()
        }
        UiKind::Vec3Drag => {
            "CParamUi { tag: CParamUiTag::Vec3Drag, _pad0: [0;3], a: 0, b: 0 }".to_string()
        }
        UiKind::Vec4Drag => {
            "CParamUi { tag: CParamUiTag::Vec4Drag, _pad0: [0;3], a: 0, b: 0 }".to_string()
        }
        UiKind::String => {
            "CParamUi { tag: CParamUiTag::String, _pad0: [0;3], a: 0, b: 0 }".to_string()
        }
        UiKind::Toggle => {
            "CParamUi { tag: CParamUiTag::Toggle, _pad0: [0;3], a: 0, b: 0 }".to_string()
        }
        UiKind::Dropdown => {
            "CParamUi { tag: CParamUiTag::Dropdown, _pad0: [0;3], a: 0, b: 0 }".to_string()
        }
        UiKind::Code => "CParamUi { tag: CParamUiTag::Code, _pad0: [0;3], a: 0, b: 0 }".to_string(),
        UiKind::Color => format!(
            "CParamUi {{ tag: CParamUiTag::Color, _pad0: [0;3], a: {0}u64, b: 0 }}",
            if ui.show_alpha.unwrap_or(false) { 1 } else { 0 }
        ),
        UiKind::None => "CParamUi { tag: CParamUiTag::None, _pad0: [0;3], a: 0, b: 0 }".to_string(),
    }
}

fn extract_user_region(s: &str) -> Option<String> {
    let a = "// === USER_CODE_BEGIN ===";
    let b = "// === USER_CODE_END ===";
    let i = s.find(a)? + a.len();
    let j = s.find(b)?;
    if j <= i {
        return None;
    }
    Some(s[i..j].to_string())
}

fn extract_user_interaction_region(s: &str) -> Option<String> {
    let a = "// === USER_INTERACTION_CODE_BEGIN ===";
    let b = "// === USER_INTERACTION_CODE_END ===";
    let i = s.find(a)? + a.len();
    let j = s.find(b)?;
    if j <= i {
        return None;
    }
    Some(s[i..j].to_string())
}

fn gen_interaction_code(spec: &InteractionSpec, existing: Option<&str>) -> String {
    let mut out = String::new();
    // HUD text statics
    for (i, h) in spec.hud_commands.iter().enumerate() {
        out.push_str(&format!(
            "static HUD_TEXT_{i}: &[u8] = b\"{}\";\n",
            bstr(&h.text)
        ));
    }
    if !spec.hud_commands.is_empty() {
        out.push('\n');
    }
    // HUD build
    out.push_str(&format!("extern \"C\" fn hud_build(_instance: *mut c_void, _host: *const CHostApi, _node: CUuid, out_cmds: *mut CHudCmd, out_cap: u32) -> u32 {{\n    if out_cmds.is_null() || out_cap < {} {{ return 0; }}\n    let cmds = unsafe {{ core::slice::from_raw_parts_mut(out_cmds, out_cap as usize) }};\n", spec.hud_commands.len()));
    for (i, h) in spec.hud_commands.iter().enumerate() {
        let tag = match h.tag {
            HudCmdTag::Label => "CHudCmdTag::Label",
            HudCmdTag::Button => "CHudCmdTag::Button",
            HudCmdTag::Toggle => "CHudCmdTag::Toggle",
            HudCmdTag::Separator => "CHudCmdTag::Separator",
        };
        out.push_str(&format!("    cmds[{i}] = CHudCmd {{ tag: {tag}, id: {}, value: 0, _pad0: 0, text: CStringView {{ ptr: HUD_TEXT_{i}.as_ptr() as *const i8, len: HUD_TEXT_{i}.len() as u32 }} }};\n", h.id));
    }
    out.push_str(&format!("    {}\n}}\n\n", spec.hud_commands.len()));
    // HUD event
    out.push_str("extern \"C\" fn hud_event(_instance: *mut c_void, _host: *const CHostApi, _node: CUuid, _e: *const CHudEvent) -> i32 { 0 }\n\n");
    // Gizmo build
    out.push_str(&format!("extern \"C\" fn gizmo_build(_instance: *mut c_void, _host: *const CHostApi, _node: CUuid, out_cmds: *mut CGizmoCmd, out_cap: u32) -> u32 {{\n    if out_cmds.is_null() || out_cap < {} {{ return 0; }}\n    let cmds = unsafe {{ core::slice::from_raw_parts_mut(out_cmds, out_cap as usize) }};\n", spec.gizmo_primitives.len()));
    for (i, g) in spec.gizmo_primitives.iter().enumerate() {
        let prim = match g.primitive {
            GizmoPrimitive::Sphere => "CGizmoPrimitive::Sphere",
            GizmoPrimitive::Cube => "CGizmoPrimitive::Cube",
            GizmoPrimitive::Cylinder => "CGizmoPrimitive::Cylinder",
            GizmoPrimitive::Cone => "CGizmoPrimitive::Cone",
            GizmoPrimitive::Plane => "CGizmoPrimitive::Plane",
        };
        out.push_str(&format!("    cmds[{i}] = CGizmoCmd {{ tag: CGizmoCmdTag::Mesh, pick_id: {}, primitive: {prim}, _pad0: 0, transform: CTransform {{ translation: [{:.6}, {:.6}, {:.6}], _pad0: 0, rotation_xyzw: [0.0, 0.0, 0.0, 1.0], scale: [{:.6}, {:.6}, {:.6}], _pad1: 0 }}, color_rgba: [{:.6}, {:.6}, {:.6}, {:.6}], p0: [0.0; 3], _pad1: 0, p1: [0.0; 3], _pad2: 0 }};\n", g.pick_id, g.position[0], g.position[1], g.position[2], g.scale[0], g.scale[1], g.scale[2], g.color[0], g.color[1], g.color[2], g.color[3]));
    }
    out.push_str(&format!("    {}\n}}\n\n", spec.gizmo_primitives.len()));
    // Gizmo event with user region
    out.push_str("extern \"C\" fn gizmo_event(_instance: *mut c_void, host: *const CHostApi, node: CUuid, e: *const CGizmoEvent) -> i32 {\n    if host.is_null() || e.is_null() { return -1; }\n    let _host = unsafe { &*host };\n    let _e = unsafe { &*e };\n");
    out.push_str("    // === USER_INTERACTION_CODE_BEGIN ===\n");
    let user_code = existing.and_then(extract_user_interaction_region).unwrap_or_else(|| "\n    // Handle gizmo events here: _e.tag (Click/Drag/Release), _e.pick_id, _e.world_pos\n    // Use HostApi to read/write node state or curve data.\n\n".to_string());
    out.push_str(&user_code);
    if !user_code.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("    // === USER_INTERACTION_CODE_END ===\n    0\n}\n\n");
    // Input event
    out.push_str("extern \"C\" fn input_event(_instance: *mut c_void, _host: *const CHostApi, _node: CUuid, e: *const CInputEvent) -> i32 {\n    if e.is_null() { return -1; }\n    let _e = unsafe { &*e };\n");
    if !spec.input_keys.is_empty() {
        out.push_str(
            "    if matches!(_e.tag, CInputEventTag::KeyDown) {\n        match _e.key {\n",
        );
        for k in &spec.input_keys {
            let kc = match k.key {
                InputKeyCode::F => "CKeyCode::F",
                InputKeyCode::G => "CKeyCode::G",
                InputKeyCode::H => "CKeyCode::H",
                InputKeyCode::K => "CKeyCode::K",
                InputKeyCode::Space => "CKeyCode::Space",
                InputKeyCode::Enter => "CKeyCode::Enter",
                InputKeyCode::Escape => "CKeyCode::Escape",
                InputKeyCode::Delete => "CKeyCode::Delete",
                InputKeyCode::Backspace => "CKeyCode::Backspace",
            };
            out.push_str(&format!("            {kc} => {{ /* {} */ }}\n", k.action));
        }
        out.push_str("            _ => {}\n        }\n    }\n");
    }
    out.push_str("    0\n}\n\n");
    // VTable export
    out.push_str("#[no_mangle]\npub unsafe extern \"C\" fn cunning_plugin_get_node_interaction_vtable(_i: u32, out: *mut CNodeInteractionVTable) -> i32 {\n    if out.is_null() { return -1; }\n    *out = CNodeInteractionVTable { hud_build, hud_event, gizmo_build, gizmo_event, input_event };\n    0\n}\n");
    out
}

fn generate_lib_rs(spec: &RustNodeSpec, existing: Option<&str>) -> String {
    let plugin = &spec.plugin_name;
    let node = spec
        .node
        .as_ref()
        .expect("node is required for generate_lib_rs");
    let cat = node
        .category
        .clone()
        .unwrap_or_else(|| "External/Rust".to_string());
    let inputs = if node.inputs.is_empty() {
        vec!["Input".to_string()]
    } else {
        node.inputs.clone()
    };
    let outputs = if node.outputs.is_empty() {
        vec!["Output".to_string()]
    } else {
        node.outputs.clone()
    };

    let mut out = String::new();
    out.push_str("//! Generated Rust plugin node (NodeSpec driven).\n\n");
    out.push_str("use cunning_plugin_sdk::c_api::*;\nuse core::ffi::c_void;\n\n");
    out.push_str(&format!(
        "static PLUGIN_NAME: &[u8] = b\"{}\";\nstatic PLUGIN_VERSION: &[u8] = b\"0.1.0\";\n",
        bstr(plugin)
    ));
    out.push_str(&format!(
        "static NODE_NAME: &[u8] = b\"{}\";\n",
        bstr(&node.name)
    ));
    out.push_str(&format!(
        "static NODE_CAT: &[u8] = b\"{}\";\n\n",
        bstr(&cat)
    ));

    for (i, s) in inputs.iter().enumerate() {
        out.push_str(&format!("static IN_{i}: &[u8] = b\"{}\";\n", bstr(s)));
    }
    for (i, s) in outputs.iter().enumerate() {
        out.push_str(&format!("static OUT_{i}: &[u8] = b\"{}\";\n", bstr(s)));
    }
    out.push('\n');

    out.push_str("static INPUTS: [CStringView; ");
    out.push_str(&inputs.len().to_string());
    out.push_str("] = [");
    for (i, _) in inputs.iter().enumerate() {
        out.push_str(&format!(
            "CStringView {{ ptr: IN_{i}.as_ptr() as *const i8, len: IN_{i}.len() as u32 }},",
        ));
    }
    out.push_str("];\n");

    out.push_str("static OUTPUTS: [CStringView; ");
    out.push_str(&outputs.len().to_string());
    out.push_str("] = [");
    for (i, _) in outputs.iter().enumerate() {
        out.push_str(&format!(
            "CStringView {{ ptr: OUT_{i}.as_ptr() as *const i8, len: OUT_{i}.len() as u32 }},",
        ));
    }
    out.push_str("];\n\n");

    for p in &node.params {
        if matches!(p.ty, ParamType::String) {
            let sym = format!(
                "P_{}_DEF",
                p.name
                    .to_uppercase()
                    .replace(|c: char| !c.is_ascii_alphanumeric(), "_")
            );
            let s = p.default.as_str().unwrap_or("");
            out.push_str(&format!("static {sym}: &[u8] = b\"{}\";\n", bstr(s)));
        }
    }
    if !node.params.is_empty() {
        out.push('\n');
    }

    for p in &node.params {
        let n = p
            .name
            .to_uppercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
        out.push_str(&gen_cstring_view(&p.name, &format!("P_{n}_NAME")));
        out.push_str(&gen_cstring_view(
            p.label.as_deref().unwrap_or(&p.name),
            &format!("P_{n}_LABEL"),
        ));
        out.push_str(&gen_cstring_view(
            p.group.as_deref().unwrap_or("General"),
            &format!("P_{n}_GROUP"),
        ));
        out.push('\n');
    }

    out.push_str(&format!("#[no_mangle]\npub unsafe extern \"C\" fn cunning_plugin_info() -> CPluginDetails {{\n    CPluginDetails {{ abi_version: CUNNING_PLUGIN_ABI_VERSION, name: CStringView {{ ptr: PLUGIN_NAME.as_ptr() as *const i8, len: PLUGIN_NAME.len() as u32 }}, version: CStringView {{ ptr: PLUGIN_VERSION.as_ptr() as *const i8, len: PLUGIN_VERSION.len() as u32 }} }}\n}}\n\n"));
    out.push_str(
        "#[no_mangle]\npub unsafe extern \"C\" fn cunning_plugin_node_count() -> u32 { 1 }\n\n",
    );

    out.push_str("static PARAMS: [CParamDesc; ");
    out.push_str(&node.params.len().to_string());
    out.push_str("] = [\n");
    for p in &node.params {
        let n = p
            .name
            .to_uppercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
        let (_tag, dv) = gen_param_value(p);
        let ui = gen_param_ui(p);
        out.push_str(&format!("    CParamDesc {{ name: P_{n}_NAME_SV, label: P_{n}_LABEL_SV, group: P_{n}_GROUP_SV, default_value: {dv}, ui: {ui} }},\n"));
    }
    out.push_str("];\n\n");

    out.push_str("#[no_mangle]\npub unsafe extern \"C\" fn cunning_plugin_get_node_desc(_i: u32, out: *mut CNodeDesc) -> i32 {\n    if out.is_null() { return 1; }\n    *out = CNodeDesc { name: CStringView { ptr: NODE_NAME.as_ptr() as *const i8, len: NODE_NAME.len() as u32 }, category: CStringView { ptr: NODE_CAT.as_ptr() as *const i8, len: NODE_CAT.len() as u32 }, inputs: CPortList { ptr: INPUTS.as_ptr(), len: INPUTS.len() as u32 }, outputs: CPortList { ptr: OUTPUTS.as_ptr(), len: OUTPUTS.len() as u32 }, input_style: CInputStyle::Single, node_style: CNodeStyle::Normal, params: PARAMS.as_ptr(), params_len: PARAMS.len() as u32 };\n    0\n}\n\n");

    out.push_str("extern \"C\" fn create() -> *mut c_void { core::ptr::null_mut() }\nextern \"C\" fn destroy(_p: *mut c_void) {}\n\n");

    out.push_str("fn decode_f32(p: &CParamValue) -> f32 { f32::from_bits(p.a as u32) }\nfn decode_i32(p: &CParamValue) -> i32 { p.a as i64 as i32 }\nfn decode_bool(p: &CParamValue) -> bool { p.a != 0 }\nfn decode_vec3(p: &CParamValue) -> (f32,f32,f32) { (f32::from_bits(p.a as u32), f32::from_bits((p.a>>32) as u32), f32::from_bits(p.b as u32)) }\n\n");

    out.push_str("// === USER_CODE_BEGIN ===\n");
    let user = existing.and_then(extract_user_region).unwrap_or_else(|| "\n// Write your algorithm here.\n// You can use HostApi for geometry/attribute operations.\n\n".to_string());
    out.push_str(&user);
    if !user.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("// === USER_CODE_END ===\n\n");

    out.push_str("extern \"C\" fn compute(_instance: *mut c_void, host: *const CHostApi, _ctx: *const CExecutionCtx, inputs: *const GeoHandle, inputs_len: u32, params: *const CParamValue, params_len: u32, out: *mut GeoHandle) -> i32 {\n    unsafe {\n        if host.is_null() || out.is_null() { return 1; }\n        let host = &*host;\n        let in0 = if inputs.is_null() || inputs_len == 0 { 0 } else { *inputs };\n        let _params = if params.is_null() || params_len == 0 { &[][..] } else { core::slice::from_raw_parts(params, params_len as usize) };\n        // Default behavior: passthrough/clone input.\n        let g = if in0 != 0 { (host.geo_clone)(host.userdata, in0) } else { (host.geo_create)(host.userdata) };\n        *out = g;\n        0\n    }\n}\n\n");

    out.push_str("#[no_mangle]\npub unsafe extern \"C\" fn cunning_plugin_get_node_vtable(_i: u32) -> CNodeVTable { CNodeVTable { create, compute, destroy } }\n\n");
    // Generate interaction VTable if spec.interaction is present
    if let Some(interaction) = &spec.interaction {
        out.push_str(&gen_interaction_code(interaction, existing));
    }
    out
}

fn set_params(node: &mut Node, map: &HashMap<String, Value>) {
    for p in &mut node.parameters {
        let Some(v) = map.get(&p.name) else {
            continue;
        };
        p.value = match &p.value {
            ParameterValue::Int(_) => ParameterValue::Int(v.as_i64().unwrap_or(0) as i32),
            ParameterValue::Float(_) => ParameterValue::Float(v.as_f64().unwrap_or(0.0) as f32),
            ParameterValue::Bool(_) => ParameterValue::Bool(v.as_bool().unwrap_or(false)),
            ParameterValue::String(_) => {
                ParameterValue::String(v.as_str().unwrap_or("").to_string())
            }
            ParameterValue::Vec2(_) => {
                let a = v.get(0).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                let b = v.get(1).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                ParameterValue::Vec2(bevy::prelude::Vec2::new(a, b))
            }
            ParameterValue::Vec3(_) => {
                let a = v.get(0).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                let b = v.get(1).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                let c = v.get(2).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                ParameterValue::Vec3(bevy::prelude::Vec3::new(a, b, c))
            }
            ParameterValue::Vec4(_) => {
                let a = v.get(0).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                let b = v.get(1).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                let c = v.get(2).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                let d = v.get(3).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                ParameterValue::Vec4(bevy::prelude::Vec4::new(a, b, c, d))
            }
            ParameterValue::Color(_) => {
                let a = v.get(0).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                let b = v.get(1).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                let c = v.get(2).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                ParameterValue::Color(bevy::prelude::Vec3::new(a, b, c))
            }
            ParameterValue::Color4(_) => {
                let a = v.get(0).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                let b = v.get(1).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                let c = v.get(2).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                let d = v.get(3).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                ParameterValue::Color4(bevy::prelude::Vec4::new(a, b, c, d))
            }
            _ => p.value.clone(),
        };
    }
}

fn build_capability_report(
    spec: &RustNodeSpec,
    node: &NodeSpec,
    fp: &crate::mesh::GeometryFingerprint,
) -> Value {
    let mut caps: Vec<String> = Vec::new();
    // Input/Output capabilities
    caps.push(format!("inputs: {}", node.inputs.len()));
    caps.push(format!("outputs: {}", node.outputs.len()));
    // Parameter capabilities
    if !node.params.is_empty() {
        let types: Vec<_> = node.params.iter().map(|p| format!("{:?}", p.ty)).collect();
        caps.push(format!(
            "params: {} ({:?})",
            node.params.len(),
            types.join(", ")
        ));
    }
    // Interaction capabilities
    if let Some(int) = &spec.interaction {
        if !int.hud_commands.is_empty() {
            caps.push(format!("hud: {} commands", int.hud_commands.len()));
        }
        if !int.gizmo_primitives.is_empty() {
            caps.push(format!("gizmo: {} primitives", int.gizmo_primitives.len()));
        }
        if !int.input_keys.is_empty() {
            caps.push(format!("input: {} keybinds", int.input_keys.len()));
        }
    }
    // Geometry output capabilities (from fingerprint)
    let mut geo_caps: Vec<String> = Vec::new();
    if fp.point_count > 0 {
        geo_caps.push(format!("points: {}", fp.point_count));
    }
    if fp.primitive_count > 0 {
        geo_caps.push(format!("prims: {}", fp.primitive_count));
    }
    if let (Some(mi), Some(ma)) = (fp.bbox_min, fp.bbox_max) {
        let size = [
            (ma[0] - mi[0]).abs(),
            (ma[1] - mi[1]).abs(),
            (ma[2] - mi[2]).abs(),
        ];
        geo_caps.push(format!(
            "bbox: [{:.2}, {:.2}, {:.2}]",
            size[0], size[1], size[2]
        ));
    }
    // Limitations (heuristics based on spec)
    let mut limits: Vec<String> = Vec::new();
    if spec.interaction.is_none() {
        limits.push("no interaction".into());
    }
    if node.params.is_empty() {
        limits.push("no parameters".into());
    }
    // Assemble report
    json!({
        "node_name": node.name,
        "category": node.category.clone().unwrap_or_else(|| "External/Rust".to_string()),
        "capabilities": caps,
        "geometry_output": geo_caps,
        "limitations": limits,
        "deterministic": spec.smoke_test.as_ref().map(|t| t.asserts.iter().any(|a| matches!(a, AssertSpec::PointCountEqInput | AssertSpec::PrimCountEqInput))).unwrap_or(false),
    })
}

fn eval_asserts(
    test: &SmokeTestSpec,
    fp_in: &crate::mesh::GeometryFingerprint,
    fp_out: &crate::mesh::GeometryFingerprint,
) -> Vec<String> {
    let mut fails = Vec::new();
    for a in &test.asserts {
        match *a {
            AssertSpec::NonEmpty => {
                if fp_out.point_count == 0 && fp_out.primitive_count == 0 {
                    fails.push("non_empty failed".to_string());
                }
            }
            AssertSpec::PointCountEqInput => {
                if fp_out.point_count != fp_in.point_count {
                    fails.push(format!(
                        "point_count_eq_input failed (in={}, out={})",
                        fp_in.point_count, fp_out.point_count
                    ));
                }
            }
            AssertSpec::PrimCountEqInput => {
                if fp_out.primitive_count != fp_in.primitive_count {
                    fails.push(format!(
                        "prim_count_eq_input failed (in={}, out={})",
                        fp_in.primitive_count, fp_out.primitive_count
                    ));
                }
            }
            AssertSpec::BboxScalesBy { factor, tolerance } => {
                let tol = tolerance.unwrap_or(1e-3);
                let (Some(mi0), Some(ma0), Some(mi1), Some(ma1)) = (
                    fp_in.bbox_min,
                    fp_in.bbox_max,
                    fp_out.bbox_min,
                    fp_out.bbox_max,
                ) else {
                    fails.push("bbox_scales_by failed (missing bbox)".to_string());
                    continue;
                };
                let s0 = [ma0[0] - mi0[0], ma0[1] - mi0[1], ma0[2] - mi0[2]];
                let s1 = [ma1[0] - mi1[0], ma1[1] - mi1[1], ma1[2] - mi1[2]];
                for i in 0..3 {
                    let t = s0[i] * factor;
                    if (s1[i] - t).abs() > tol {
                        fails.push(format!(
                            "bbox_scales_by failed axis{} (want~{}, got {})",
                            i, t, s1[i]
                        ));
                        break;
                    }
                }
            }
        }
    }
    fails
}

fn to_snake(s: &str) -> String {
    if s.contains('_') { return s.to_ascii_lowercase(); }
    let mut out = String::with_capacity(s.len() + 8);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i != 0 { out.push('_'); }
            out.push(ch.to_ascii_lowercase());
        } else { out.push(ch); }
    }
    out.to_ascii_lowercase()
}

fn slugify_plugin_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 8);
    let mut last_us = false;
    for ch in name.chars() {
        let ok = ch.is_ascii_alphanumeric();
        if ok {
            out.push(ch.to_ascii_lowercase());
            last_us = false;
        } else if !last_us {
            out.push('_');
            last_us = true;
        }
    }
    let base = out.trim_matches('_');
    let base = if base.is_empty() { "my_plugin" } else { base };
    let base = base.strip_suffix("_plugin").unwrap_or(base);
    format!("{base}_plugin")
}

fn normalize_nodespec_value(v: Value) -> Result<Value, ToolError> {
    let mut v = match v {
        Value::String(s) => serde_json::from_str::<Value>(&s).map_err(|e| ToolError(format!("Invalid nodespec JSON string: {e}")))?,
        other => other,
    };
    let Value::Object(mut root) = v else { return Err(ToolError("Invalid nodespec: expected object".into())); };

    // Accept NodeSpec directly (Zed-like/legacy): {name, category, inputs, outputs, params, ...}
    if root.get("node").is_none()
        && root.get("name").and_then(|v| v.as_str()).is_some()
        && (root.get("inputs").is_some() || root.get("outputs").is_some() || root.get("params").is_some() || root.get("parameters").is_some() || root.get("category").is_some())
    {
        let mut node = serde_json::Map::new();
        for k in ["name", "category", "inputs", "outputs", "params", "parameters"] {
            if let Some(v) = root.remove(k) { node.insert(k.to_string(), v); }
        }
        let pn = root
            .get("plugin_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| node.get("plugin_name").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .or_else(|| node.get("name").and_then(|v| v.as_str()).map(slugify_plugin_name))
            .unwrap_or_else(|| "my_plugin".to_string());
        root.entry("plugin_name".to_string()).or_insert(Value::String(pn));
        root.insert("node".to_string(), Value::Object(node));
    }

    // Promote node.plugin_name to top-level plugin_name.
    let node_plugin_name = root
        .get("node")
        .and_then(|n| n.as_object())
        .and_then(|n| n.get("plugin_name"))
        .cloned();
    if root.get("plugin_name").is_none() {
        if let Some(pn) = node_plugin_name { root.insert("plugin_name".to_string(), pn); }
    }

    if let Some(node) = root.get_mut("node").and_then(|n| n.as_object_mut()) {
        if let Some(inputs) = node.get_mut("inputs").and_then(|x| x.as_array_mut()) {
            if inputs.iter().any(|it| it.is_object()) {
                let names = inputs.iter().filter_map(|it| it.get("name").and_then(|n| n.as_str()).map(|s| Value::String(s.to_string()))).collect::<Vec<_>>();
                *inputs = names;
            }
        }
        if let Some(outputs) = node.get_mut("outputs").and_then(|x| x.as_array_mut()) {
            if outputs.iter().any(|it| it.is_object()) {
                let names = outputs.iter().filter_map(|it| it.get("name").and_then(|n| n.as_str()).map(|s| Value::String(s.to_string()))).collect::<Vec<_>>();
                *outputs = names;
            }
        }
        if node.get("params").is_none() {
            if let Some(p) = node.remove("parameters") { node.insert("params".to_string(), p); }
        }
        if let Some(params) = node.get_mut("params").and_then(|x| x.as_array_mut()) {
            for p in params.iter_mut().filter_map(|x| x.as_object_mut()) {
                if p.get("type").is_none() {
                    if let Some(t) = p.remove("param_type").and_then(|v| v.as_str().map(|s| Value::String(to_snake(s))).or(Some(v))) {
                        p.insert("type".to_string(), t);
                    }
                } else if let Some(t) = p.get_mut("type").and_then(|v| v.as_str()).map(|s| s.to_string()) {
                    p.insert("type".to_string(), Value::String(to_snake(&t)));
                }
                if let Some(ui_v) = p.get_mut("ui") {
                    if let Some(s) = ui_v.as_str().map(|s| s.to_string()) {
                        *ui_v = json!({"kind": to_snake(&s)});
                    } else if let Some(ui) = ui_v.as_object_mut() {
                        if let Some(k) = ui.get("kind").and_then(|v| v.as_str()).map(|s| s.to_string()) {
                            ui.insert("kind".to_string(), Value::String(to_snake(&k)));
                        }
                    }
                }
            }
        }
    }
    Ok(Value::Object(root))
}

pub struct ApplyRustNodeSpecTool {
    registry: Arc<NodeRegistry>,
}

impl ApplyRustNodeSpecTool {
    pub fn new(registry: Arc<NodeRegistry>) -> Self {
        Self { registry }
    }
}

impl Tool for ApplyRustNodeSpecTool {
    fn name(&self) -> &str {
        "apply_rust_nodespec"
    }
    fn is_long_running(&self) -> bool {
        true
    } // Cargo build takes time
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: r#"Apply a Rust NodeSpec. Modes:
- CREATE (default): Generate full plugin from node spec. Requires: plugin_name, node.
- PATCH: Only modify USER_CODE region. Requires: plugin_name, user_code_patch {old, new} or user_code (full replacement).
Patch mode saves tokens by not regenerating framework code."#.to_string(),
            parameters: json!({"type":"object","properties":{"nodespec":{"type":["object","string"],"description":"NodeSpec object (or JSON string)"},"build":{"type":"boolean","description":"If true, run cargo build + copy DLL + reload plugins (+ optional smoke_test). Default: false."}},"required":["nodespec"]}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        self.execute_inner(args, None)
    }

    fn execute_with_context(
        &self,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        self.execute_inner(args, Some(ctx))
    }
}

#[cfg(all(test, feature = "ai_ws_rust_patch_tests"))]
mod rust_patch_tests {
    use super::*;

    #[test]
    fn parse_hex_u64_accepts_plain_and_0x() {
        assert_eq!(parse_hex_u64("0"), Some(0));
        assert_eq!(parse_hex_u64("0x10"), Some(16));
        assert_eq!(parse_hex_u64("0010"), Some(16));
        assert_eq!(parse_hex_u64("zz"), None);
    }

    #[test]
    fn find_region_bounds_finds_inner_slice() {
        let s = "aa// === USER_CODE_BEGIN ===\nX\n// === USER_CODE_END ===bb";
        let (i, j, inner) =
            find_region_bounds(s, "// === USER_CODE_BEGIN ===", "// === USER_CODE_END ===")
                .unwrap();
        assert!(j > i);
        assert_eq!(inner.trim(), "X");
    }
}

impl ApplyRustNodeSpecTool {
    fn execute_inner(
        &self,
        args: Value,
        ctx: Option<&ToolContext>,
    ) -> Result<ToolOutput, ToolError> {
        let mut progress = |msg: &str, level: ToolLogLevel| {
            if let Some(c) = ctx {
                c.report_progress(ToolLog {
                    message: msg.to_string(),
                    level,
                });
            }
        };
        let mut check_cancel = || -> Result<(), ToolError> {
            if ctx.is_some_and(|c| c.is_cancelled()) {
                Err(ToolError("Cancelled".into()))
            } else {
                Ok(())
            }
        };
        check_cancel()?;
        progress("Parsing NodeSpec...", ToolLogLevel::Info);
        let build_requested = args.get("build").and_then(|v| v.as_bool()).unwrap_or(false);
        let mut nodespec_v = args
            .get("nodespec")
            .or_else(|| args.get("nodespec_json"))
            .cloned()
            .unwrap_or(args.clone());
        // If caller provided plugin_name alongside nodespec(NodeSpec-direct), merge it in early.
        if let (Some(pn), Value::Object(ref mut m)) = (args.get("plugin_name").and_then(|v| v.as_str()), &mut nodespec_v) {
            if !m.contains_key("plugin_name") { m.insert("plugin_name".to_string(), Value::String(pn.to_string())); }
        }
        let nodespec_v = normalize_nodespec_value(nodespec_v)?;
        let nodespec_str = serde_json::to_string(&nodespec_v).unwrap_or_default();
        // Use serde_path_to_error for precise JSON path in error messages
        let mut spec: RustNodeSpec = {
            let mut de = serde_json::Deserializer::from_str(&nodespec_str);
            serde_path_to_error::deserialize(&mut de).map_err(|e| {
                let path = e.path().to_string();
                let inner = e.inner().to_string();
                let hint = "Hint: add plugin_name; wrap NodeSpec as {plugin_name, node:{...}}; node.inputs/outputs are string arrays; node.params items are {name,type,default} with type=int|float|bool|string|vec2|vec3|vec4|color3|color4.";
                let truncated = trunc_mid(&serde_json::to_string_pretty(&nodespec_v).unwrap_or_default(), 2000);
                ToolError(format!("Invalid nodespec at `{}`: {}. {}\n\nNormalized JSON (truncated):\n{}", path, inner, hint, truncated))
            })?
        };
        if spec.plugin_name.contains("..")
            || spec.plugin_name.contains('/')
            || spec.plugin_name.contains('\\')
        {
            return Err(ToolError("Invalid plugin_name".to_string()));
        }

        let crate_dir = Path::new("plugins/extra_node").join(&spec.plugin_name);
        let tpl = Path::new("crates/cunning_plugin_sdk/template_rust_node");
        let mut logs: Vec<ToolLog> = Vec::new();
        let mut diffs = Vec::new();
        let lib_path = crate_dir.join("src/lib.rs");
        check_cancel()?;
        progress("Applying files...", ToolLogLevel::Info);

        // Default to PATCH when plugin exists and request only modifies user regions.
        if matches!(spec.mode, NodeSpecMode::Create)
            && crate_dir.exists()
            && spec.node.is_none()
            && (spec.user_code.is_some() || spec.user_code_patch.is_some())
        {
            spec.mode = NodeSpecMode::Patch;
            logs.push(ToolLog {
                message:
                    "Policy: existing plugin + user_code change => using PATCH mode by default."
                        .to_string(),
                level: ToolLogLevel::Info,
            });
        }

        // PATCH MODE: Only modify USER_CODE region
        if matches!(spec.mode, NodeSpecMode::Patch) {
            let existing = std::fs::read_to_string(&lib_path)
                .map_err(|e| ToolError(format!("Cannot patch: file not found: {e}")))?;
            let begin = "// === USER_CODE_BEGIN ===";
            let end = "// === USER_CODE_END ===";
            let (i, j, user_region) = find_region_bounds(&existing, begin, end)?;
            let hash_before = fnv1a64(user_region.as_bytes());
            if let Some(exp) = spec
                .expected_user_code_hash_fnv1a64
                .as_deref()
                .and_then(parse_hex_u64)
            {
                if exp != hash_before {
                    return Err(ToolError(format!("Patch refused: USER_CODE hash mismatch (expected={:016x}, current={:016x}).", exp, hash_before)));
                }
            }

            let new_content = if let Some(patch) = &spec.user_code_patch {
                // Patch mode: old -> new replacement within USER_CODE region
                let hits = user_region.match_indices(&patch.old).count();
                if hits == 0 {
                    return Err(ToolError(format!(
                        "Patch failed: 'old' text not found in USER_CODE region (hash={:016x}).",
                        hash_before
                    )));
                }
                if hits > 1 {
                    return Err(ToolError(format!("Patch refused: 'old' text matches {} times in USER_CODE region (hash={:016x}).", hits, hash_before)));
                }
                let patched_region = user_region.replacen(&patch.old, &patch.new, 1);
                format!("{}{}{}", &existing[..i], patched_region, &existing[j..])
            } else if let Some(user_code) = &spec.user_code {
                // Direct replacement of USER_CODE region
                let uc = user_code.trim_matches('\n');
                format!("{}\n{}\n{}", &existing[..i], uc, &existing[j..])
            } else {
                return Err(ToolError(
                    "Patch mode requires 'user_code' or 'user_code_patch'".into(),
                ));
            };

            if let Some(d) = compute_file_diff(
                lib_path.to_string_lossy().to_string(),
                &existing,
                &new_content,
            ) {
                diffs.push(d);
            }
            std::fs::write(&lib_path, &new_content)
                .map_err(|e| ToolError(format!("write lib.rs failed: {e}")))?;
            let (_, _, user_after) = find_region_bounds(&new_content, begin, end)?;
            let hash_after = fnv1a64(user_after.as_bytes());
            logs.push(ToolLog {
                message: format!(
                    "Patched USER_CODE region (hash {:016x}->{:016x}).",
                    hash_before, hash_after
                ),
                level: ToolLogLevel::Success,
            });
        } else {
            // CREATE MODE: Full generation
            let node = spec
                .node
                .as_ref()
                .ok_or_else(|| ToolError("CREATE mode requires 'node' field".into()))?;
            if node.name.trim().is_empty() {
                return Err(ToolError("nodespec.node.name is required".to_string()));
            }

            if !crate_dir.exists() {
                let _ = std::fs::create_dir_all(crate_dir.join("src"))
                    .map_err(|e| ToolError(format!("mkdir failed: {e}")))?;
                let cargo = std::fs::read_to_string(tpl.join("Cargo.toml"))
                    .map_err(|e| ToolError(format!("read template Cargo.toml failed: {e}")))?;
                let cargo_path = crate_dir.join("Cargo.toml");
                let cargo_new = cargo.replace("__PLUGIN_NAME__", &spec.plugin_name);
                if let Some(d) = compute_file_diff(
                    cargo_path.to_string_lossy().to_string(),
                    "",
                    &cargo_new,
                ) {
                    diffs.push(d);
                }
                std::fs::write(&cargo_path, cargo_new)
                .map_err(|e| ToolError(format!("write Cargo.toml failed: {e}")))?;
                logs.push(ToolLog {
                    message: format!("Created crate {}", crate_dir.display()),
                    level: ToolLogLevel::Success,
                });
            }

            let ns_path = crate_dir.join("nodespec.json");
            let _ = std::fs::write(
                &ns_path,
                serde_json::to_string_pretty(&nodespec_v).unwrap_or_else(|_| "{}".to_string()),
            );

            let existing = std::fs::read_to_string(&lib_path).ok();
            let old_lib = existing.clone().unwrap_or_default();
            let lib = generate_lib_rs(&spec, existing.as_deref());
            if let Some(d) = compute_file_diff(
                lib_path.to_string_lossy().to_string(),
                &old_lib,
                &lib,
            ) {
                diffs.push(d);
            }
            std::fs::write(&lib_path, lib)
                .map_err(|e| ToolError(format!("write lib.rs failed: {e}")))?;
            logs.push(ToolLog {
                message: "Updated src/lib.rs from NodeSpec (preserved USER_CODE if present)."
                    .to_string(),
                level: ToolLogLevel::Success,
            });
        }

        if !build_requested {
            logs.push(ToolLog { message: "Build skipped (set build=true to compile/reload).".to_string(), level: ToolLogLevel::Info });
            let diag = json!({
                "build_status": "skipped",
                "smoke_status": "skipped",
                "plugin_name": spec.plugin_name,
                "node_type": spec.node.as_ref().map(|n| n.name.clone()).unwrap_or_default(),
                "dll_path": null,
                "copied_dll_path": null,
                "cargo_errors": [],
                "smoke_fails": [],
                "build_log_excerpt": "",
            });
            let mut out = ToolOutput::new(format!(
                "Applied NodeSpec (build skipped).\n\nAI_DIAG_JSON:\n{}",
                serde_json::to_string_pretty(&diag).unwrap_or_else(|_| "{}".to_string()),
            ), logs);
            out.ui_diffs.extend(diffs);
            return Ok(out);
        }

        let release = spec.release.unwrap_or(true);
        let offline = spec.offline.unwrap_or(false);
        let locked = spec.locked.unwrap_or(false);
        check_cancel()?;
        progress("Queueing background cargo build (AppJobs)...", ToolLogLevel::Info);
        let mut req =
            crate::cunning_core::plugin_system::CompileRustPluginRequest::for_extra_node(
                spec.plugin_name.clone(),
            );
        req.release = release;
        req.offline = offline;
        req.locked = locked;
        req.hot_reload = true;
        crate::cunning_core::plugin_system::request_compile_rust_plugin(req).map_err(ToolError)?;

        // No-hitch: smoke test is deferred until build finishes (handled by app UI/workflow).
        if spec.smoke_test.is_some() {
            logs.push(ToolLog {
                message: "Smoke test deferred: build runs in background and hot-reload happens on completion."
                    .to_string(),
                level: ToolLogLevel::Warning,
            });
        }

        let diag = json!({
            "build_status": "queued",
            "smoke_status": if spec.smoke_test.is_some() { "deferred" } else { "none" },
            "plugin_name": spec.plugin_name,
            "node_type": spec.node.as_ref().map(|n| n.name.clone()).unwrap_or_default(),
            "dll_path": null,
            "copied_dll_path": null,
            "cargo_errors": [],
            "smoke_fails": [],
            "build_log_excerpt": "",
        });
        let mut out = ToolOutput::new(format!(
            "Applied NodeSpec and queued background build.\n\nAI_DIAG_JSON:\n{}",
            serde_json::to_string_pretty(&diag).unwrap_or_else(|_| "{}".to_string()),
        ), logs);
        out.ui_diffs.extend(diffs);
        return Ok(out);
        #[cfg(any())]
        {
        check_cancel()?;
        progress("Running smoke test graph...", ToolLogLevel::Info);
        let mut graph = self
            .graph
            .lock()
            .map_err(|_| ToolError("Failed to lock NodeGraph".to_string()))?;
        let prev_display = graph.display_node;
        let base_y = 0.0;

        let input_id = Uuid::new_v4();
        let mut input_node = match test.input.kind {
            SmokeInputKind::CreateCube => Node::new(
                input_id,
                "AI_Smoke_Input".to_string(),
                NodeType::CreateCube,
                Pos2::new(0.0, base_y),
            ),
            SmokeInputKind::CreateSphere => Node::new(
                input_id,
                "AI_Smoke_Input".to_string(),
                NodeType::CreateSphere,
                Pos2::new(0.0, base_y),
            ),
            SmokeInputKind::Empty => Node::new(
                input_id,
                "AI_Smoke_Input".to_string(),
                NodeType::Generic("Merge".to_string()),
                Pos2::new(0.0, base_y),
            ),
        };
        set_params(&mut input_node, &test.input.params);
        graph.nodes.insert(input_id, input_node);

        let plug_id = Uuid::new_v4();
        let mut plug = Node::new(
            plug_id,
            "AI_Smoke_Plugin".to_string(),
            NodeType::Generic(node.name.clone()),
            Pos2::new(240.0, base_y),
        );
        if let NodeType::Generic(type_name) = &plug.node_type {
            if let Some(desc) = self.registry.nodes.read().unwrap().get(type_name) {
                plug.parameters = (desc.parameters_factory)();
                plug.inputs = desc
                    .inputs
                    .iter()
                    .map(|s| (PortId::from(s.as_str()), ()))
                    .collect();
                plug.outputs = desc
                    .outputs
                    .iter()
                    .map(|s| (PortId::from(s.as_str()), ()))
                    .collect();
                plug.rebuild_ports();
            }
        }
        set_params(&mut plug, &test.plugin_params);
        graph.nodes.insert(plug_id, plug);

        let to_port = node.inputs.get(0).map(|s| s.as_str()).unwrap_or("Input");
        let c = Connection {
            id: Uuid::new_v4(),
            from_node: input_id,
            from_port: PortId::from("Output"),
            to_node: plug_id,
            to_port: PortId::from(to_port),
            order: 0,
            waypoints: Vec::new(),
        };
        graph.connections.insert(c.id, c);
        graph.display_node = Some(plug_id);
        graph.mark_dirty(input_id);
        graph.mark_dirty(plug_id);
        graph.ensure_display_node_default();

        let targets: HashSet<Uuid> = HashSet::new();
        graph.compute(&targets, &*self.registry, None);
        check_cancel()?;

        let fin = graph
            .geometry_cache
            .get(&input_id)
            .cloned()
            .ok_or_else(|| ToolError("Smoke test failed: input not cooked".to_string()))?;
        let fout = graph
            .geometry_cache
            .get(&plug_id)
            .cloned()
            .ok_or_else(|| ToolError("Smoke test failed: output not cooked".to_string()))?;
        let fp_in = fin.compute_fingerprint();
        let fp_out = fout.compute_fingerprint();
        let fails = eval_asserts(&test, &fp_in, &fp_out);

        graph.nodes.remove(&input_id);
        graph.nodes.remove(&plug_id);
        graph.connections.retain(|_, c| {
            c.from_node != input_id
                && c.to_node != input_id
                && c.from_node != plug_id
                && c.to_node != plug_id
        });
        graph.geometry_cache.remove(&input_id);
        graph.geometry_cache.remove(&plug_id);
        graph.display_node = prev_display;

        if fails.is_empty() {
            logs.push(ToolLog {
                message: "Smoke test PASSED.".to_string(),
                level: ToolLogLevel::Success,
            });
            // Generate capability report for successful builds
            let cap_report = build_capability_report(&spec, &node, &fp_out);
            let diag = json!({
                "build_status": "ok",
                "smoke_status": "passed",
                "plugin_name": spec.plugin_name,
                "node_type": node.name,
                "dll_path": dll.display().to_string(),
                "copied_dll_path": copied.display().to_string(),
                "cargo_errors": [],
                "smoke_fails": [],
                "build_log_excerpt": trunc_mid(&build_log, 6000),
                "fingerprint": { "point_count": fp_out.point_count, "primitive_count": fp_out.primitive_count, "bbox_min": fp_out.bbox_min, "bbox_max": fp_out.bbox_max },
                "capability_report": cap_report,
            });
            Ok(ToolOutput::new(format!(
                "Applied NodeSpec + Smoke test PASSED.\n\nAI_DIAG_JSON:\n{}\n\nCapability Report:\n{}\n\nBuild log (excerpt):\n{}",
                serde_json::to_string_pretty(&diag).unwrap_or_else(|_| "{}".to_string()),
                serde_json::to_string_pretty(&cap_report).unwrap_or_else(|_| "{}".to_string()),
                trunc_mid(&build_log, 6000),
            ), logs))
        } else {
            logs.push(ToolLog {
                message: format!("Smoke test FAILED ({} asserts).", fails.len()),
                level: ToolLogLevel::Error,
            });
            for f in &fails {
                logs.push(ToolLog {
                    message: f.clone(),
                    level: ToolLogLevel::Error,
                });
            }
            let diag = json!({
                "build_status": "ok",
                "smoke_status": "failed",
                "plugin_name": spec.plugin_name,
                "node_type": node.name,
                "dll_path": dll.display().to_string(),
                "copied_dll_path": copied.display().to_string(),
                "cargo_errors": [],
                "smoke_fails": fails,
                "build_log_excerpt": trunc_mid(&build_log, 6000),
                "fingerprint": { "point_count": fp_out.point_count, "primitive_count": fp_out.primitive_count, "bbox_min": fp_out.bbox_min, "bbox_max": fp_out.bbox_max },
            });
            Ok(ToolOutput::new(format!(
                "Applied NodeSpec but Smoke test FAILED.\n\nAI_DIAG_JSON:\n{}\n\nBuild log (excerpt):\n{}",
                serde_json::to_string_pretty(&diag).unwrap_or_else(|_| "{}".to_string()),
                trunc_mid(&build_log, 6000),
            ), logs))
        }
        }
    }
}
