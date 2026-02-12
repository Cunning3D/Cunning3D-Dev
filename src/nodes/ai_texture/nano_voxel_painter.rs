//! NanoVoxelPainter node: voxel mask -> 3x2 atlas -> quantized voxel palette.

use base64::Engine;
use bevy::prelude::*;
use bytemuck::{Pod, Zeroable};
use crossbeam_channel::{unbounded, Receiver};
use image::{GenericImageView, ImageBuffer, Rgba};
use once_cell::sync::OnceCell;
use reqwest::blocking::Client;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;

use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::mesh::{Attribute, Geometry};
use crate::nodes::gpu::runtime::GpuRuntime;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::nodes::structs::{NodeGraphResource, NodeId, NodeType, PortId};
use crate::nodes::port_key;
use crate::register_node;

use crate::nodes::voxel::voxel_edit::{
    discrete_to_surface_mesh, read_discrete_payload, write_discrete_payload,
    ATTR_VOXEL_MASK_CELLS_I32, ATTR_VOXEL_SIZE_DETAIL, ATTR_VOXEL_SRC_PRIM,
};
use cunning_kernel::algorithms::algorithms_editor::voxel as vox;

pub const NODE_NANO_VOXEL_PAINTER: &str = "Nano Voxel Painter";

#[inline]
fn hash64(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

#[inline]
fn stable_u128(a: &str, b: &str) -> u128 { ((hash64(a) as u128) << 64) | (hash64(b) as u128) }

#[inline]
fn voxel_node_for_result(input: &Geometry, base_atlas: &str, guide_atlas: &str, pal_max: usize, depth_eps: f32) -> uuid::Uuid {
    let in_id = input
        .get_detail_attribute("__voxel_node")
        .and_then(|a| a.as_slice::<String>())
        .and_then(|v| v.first())
        .cloned()
        .unwrap_or_default();
    let baked = input
        .get_detail_attribute("__voxel_baked_cursor")
        .and_then(|a| a.as_slice::<i32>())
        .and_then(|v| v.first().copied())
        .unwrap_or(0)
        .max(0);
    let k0 = format!("nano_voxel_painter:{in_id}:{baked}:{pal_max}");
    let k1 = format!("atlas:{base_atlas}|guide:{guide_atlas}|eps:{:.4}", depth_eps.max(0.0));
    uuid::Uuid::from_u128(stable_u128(&k0, &k1))
}

const PARAM_PROMPT: &str = "prompt";
const PARAM_TILE_RES: &str = "tile_res";
const PARAM_PAL_MAX: &str = "palette_max";
const PARAM_MASK_ONLY: &str = "mask_only";
const PARAM_REF_IMAGE: &str = "reference_image";
const PARAM_DEPTH_EPS: &str = "depth_eps";
const PARAM_MAX_SURFACE_VOXELS: &str = "max_surface_voxels";
const PARAM_TIMEOUT_S: &str = "timeout_s";
const PARAM_GENERATE: &str = "generate";
const PARAM_GUIDE_ATLAS: &str = "guide_atlas";
const PARAM_BASE_ATLAS: &str = "basecolor_atlas";
const PARAM_STATUS: &str = "status";
const PARAM_ERROR: &str = "error";
const PARAM_BUSY: &str = "busy";

pub(crate) const SYS_ATLAS: &str = "You generate a 3x2 atlas texture for a voxel asset based on a provided 3x2 guide atlas.\nRules:\n- Output MUST be a single image and nothing else.\n- Preserve the 3x2 layout EXACTLY.\n- Each tile corresponds to an orthographic view: row0=[+X,-X,+Y], row1=[-Y,+Z,-Z].\n- No text, watermark, or annotations.\n- Keep style consistent across all 6 tiles.\n";

#[derive(Default)]
pub struct NanoVoxelPainterNode;

impl NodeParameters for NanoVoxelPainterNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                crate::nodes::ai_texture::PARAM_AI_SYSTEM_PROMPT,
                "System Prompt",
                "AI",
                ParameterValue::String(SYS_ATLAS.to_string()),
                ParameterUIType::Code,
            ),
            Parameter::new(PARAM_PROMPT, "Prompt", "AI", ParameterValue::String("A stylized but physically plausible material.".to_string()), ParameterUIType::String),
            Parameter::new(PARAM_TILE_RES, "Tile Resolution", "AI", ParameterValue::Int(384), ParameterUIType::IntSlider { min: 128, max: 1024 }),
            Parameter::new(PARAM_PAL_MAX, "Palette Max", "Voxel", ParameterValue::Int(64), ParameterUIType::IntSlider { min: 4, max: 200 }),
            Parameter::new(PARAM_MASK_ONLY, "Mask Only", "Voxel", ParameterValue::Bool(true), ParameterUIType::Toggle),
            Parameter::new(PARAM_DEPTH_EPS, "Depth Epsilon", "Voxel", ParameterValue::Float(0.02), ParameterUIType::FloatSlider { min: 0.0, max: 0.2 }),
            Parameter::new(PARAM_MAX_SURFACE_VOXELS, "Max Surface Voxels", "Safety", ParameterValue::Int(250_000), ParameterUIType::IntSlider { min: 10_000, max: 5_000_000 }),
            Parameter::new(
                PARAM_REF_IMAGE,
                "Reference Image",
                "AI",
                ParameterValue::String(String::new()),
                ParameterUIType::FilePath { filters: vec!["png".into(), "jpg".into(), "jpeg".into(), "webp".into()] },
            ),
            Parameter::new(PARAM_TIMEOUT_S, "Timeout (s)", "AI", ParameterValue::Int(0), ParameterUIType::IntSlider { min: 0, max: 1800 }),
            Parameter::new(
                PARAM_GENERATE,
                "Generate",
                "AI",
                ParameterValue::Int(0),
                ParameterUIType::BusyButton { busy_param: PARAM_BUSY.to_string(), busy_label: "Generating...".to_string(), busy_label_param: Some(PARAM_STATUS.to_string()) },
            ),
            Parameter::new(PARAM_GUIDE_ATLAS, "Guide Atlas (Internal)", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
            Parameter::new(PARAM_BASE_ATLAS, "BaseColor Atlas (Internal)", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into(), "jpg".into(), "jpeg".into(), "webp".into()] }),
            Parameter::new(PARAM_STATUS, "Status", "Internal", ParameterValue::String(String::new()), ParameterUIType::String),
            Parameter::new(PARAM_ERROR, "Error", "Internal", ParameterValue::String(String::new()), ParameterUIType::String),
            Parameter::new(PARAM_BUSY, "Busy (Internal)", "Debug", ParameterValue::Bool(false), ParameterUIType::Toggle),
        ]
    }
}

impl NodeOp for NanoVoxelPainterNode {
    fn compute(&self, params: &[Parameter], inputs: &[std::sync::Arc<dyn crate::libs::geometry::geo_ref::GeometryRef>]) -> std::sync::Arc<Geometry> {
        puffin::profile_scope!("NanoVoxelPainter::compute");
        let input = inputs.first().map(|g| g.materialize()).unwrap_or_else(Geometry::new);
        let mask_cells = read_mask_cells(&input);
        let mask_only = p_bool(params, PARAM_MASK_ONLY, true) && !mask_cells.is_empty();
        let base_atlas = p_str(params, PARAM_BASE_ATLAS, "").trim().to_string();
        let guide_atlas = p_str(params, PARAM_GUIDE_ATLAS, "").trim().to_string();
        let depth_eps = p_f32(params, PARAM_DEPTH_EPS, 0.02).max(0.0);
        let max_surface_voxels = p_i32(params, PARAM_MAX_SURFACE_VOXELS, 250_000).max(10_000) as usize;
        let grid = read_grid_from_input(&input).unwrap_or_else(|| vox::DiscreteVoxelGrid::new(read_voxel_size(&input)));
        let mut out_grid = grid.clone();
        if !base_atlas.is_empty() {
            let pal_max = p_i32(params, PARAM_PAL_MAX, 64).clamp(4, 200) as usize;
            if let Ok(base_img) = load_image_from_assets(&base_atlas) {
                let guide_img = (!guide_atlas.is_empty()).then(|| load_image_from_assets(&guide_atlas)).transpose().ok().flatten();
                out_grid = recolor_grid_from_atlas(
                    &grid,
                    &base_img,
                    guide_img.as_ref(),
                    pal_max,
                    mask_only.then_some(&mask_cells),
                    depth_eps,
                    max_surface_voxels,
                );
            }
        }
        // Register as voxel for viewport (GPU chunk renderer) + carry voxel metadata downstream.
        let pal_max = p_i32(params, PARAM_PAL_MAX, 64).clamp(4, 200) as usize;
        let voxel_node = voxel_node_for_result(&input, &base_atlas, &guide_atlas, pal_max, depth_eps);
        cunning_kernel::nodes::voxel::voxel_edit::voxel_render_register_grid(voxel_node, out_grid.voxel_size.max(0.001), out_grid.clone());
        let mut out = discrete_to_surface_mesh(&out_grid);
        let prim_n = out.primitives().len();
        if prim_n > 0 { out.insert_primitive_attribute(ATTR_VOXEL_SRC_PRIM, Attribute::new(vec![true; prim_n])); }
        out.set_detail_attribute(ATTR_VOXEL_SIZE_DETAIL, vec![out_grid.voxel_size.max(0.001)]);
        out.set_detail_attribute("__voxel_pure", vec![1.0f32]);
        out.set_detail_attribute("__voxel_node", vec![voxel_node.to_string()]);
        // Quick inspect: what colors did AI actually use?
        let mut used = [false; 256];
        for (_c, v) in out_grid.voxels.iter() { used[v.palette_index as usize] = true; }
        let mut used_hex: Vec<String> = Vec::new();
        let mut used_n = 0usize;
        for i in 1..256usize {
            if !used[i] { continue; }
            used_n += 1;
            if used_hex.len() < 64 {
                let c = out_grid.palette.get(i).map(|e| e.color).unwrap_or([255, 255, 255, 255]);
                used_hex.push(format!("#{:02X}{:02X}{:02X}{:02X}", c[0], c[1], c[2], c[3]));
            }
        }
        out.set_detail_attribute("__voxel_palette_used_n", vec![used_n as f32]);
        if !used_hex.is_empty() { out.set_detail_attribute("__voxel_palette_used_hex", used_hex); }
        if !mask_cells.is_empty() {
            let mut flat: Vec<i32> = Vec::with_capacity(mask_cells.len() * 3);
            for c in mask_cells.iter() { flat.extend_from_slice(&[c.x, c.y, c.z]); }
            out.set_detail_attribute(ATTR_VOXEL_MASK_CELLS_I32, flat);
        }
        write_discrete_payload(&mut out, &out_grid);
        std::sync::Arc::new(out)
    }
}

register_node!(
    NODE_NANO_VOXEL_PAINTER,
    "AI Texture",
    NanoVoxelPainterNode;
    inputs: &["Geometry"],
    outputs: &["Voxel"],
    style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts
);

#[derive(Clone, Debug)]
pub(crate) struct ImageBlob { pub mime: String, pub bytes: Vec<u8> }

#[derive(Clone, Debug)]
struct NanoVoxelPainterSpec {
    node_id: NodeId,
    system_prompt: String,
    prompt: String,
    tile_res: u32,
    ref_image: String,
    timeout_s: i32,
    in_geo: Geometry,
}

#[derive(Clone, Debug)]
struct JobResult {
    node_id: NodeId,
    guide_rel: Option<String>,
    base_rel: Option<String>,
    err: Option<String>,
    elapsed_ms: u128,
}

#[derive(Resource, Default)]
pub(crate) struct NanoVoxelPainterJobs { map: HashMap<NodeId, JobState> }

#[derive(Default)]
struct JobState { last_gen: i32, inflight: bool, rx: Option<Receiver<JobResult>>, last_poll: Option<Instant> }

#[inline]
fn p_i32(params: &[Parameter], name: &str, d: i32) -> i32 {
    params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::Int(v) = &p.value { Some(*v) } else { None }).unwrap_or(d)
}
#[inline]
fn p_f32(params: &[Parameter], name: &str, d: f32) -> f32 {
    params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::Float(v) = &p.value { Some(*v) } else { None }).unwrap_or(d)
}
#[inline]
fn p_bool(params: &[Parameter], name: &str, d: bool) -> bool {
    params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::Bool(v) = &p.value { Some(*v) } else { None }).unwrap_or(d)
}
#[inline]
fn p_str(params: &[Parameter], name: &str, d: &str) -> String {
    params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None }).unwrap_or_else(|| d.to_string())
}

fn set_str(params: &mut [Parameter], name: &str, v: String) -> bool {
    if let Some(p) = params.iter_mut().find(|p| p.name == name) {
        if let ParameterValue::String(cur) = &mut p.value { if *cur != v { *cur = v; return true; } }
        else { p.value = ParameterValue::String(v); return true; }
    }
    false
}
fn set_bool(params: &mut [Parameter], name: &str, v: bool) -> bool {
    if let Some(p) = params.iter_mut().find(|p| p.name == name) {
        if let ParameterValue::Bool(cur) = &mut p.value { if *cur != v { *cur = v; return true; } }
        else { p.value = ParameterValue::Bool(v); return true; }
    }
    false
}

fn file_rel_in_assets(subdir: &str, name: &str) -> (std::path::PathBuf, String) {
    let assets = std::env::current_dir().ok().unwrap_or_default().join("assets");
    let rel = format!("{}/{}", subdir.trim_matches('/'), name);
    (assets.join(&rel), rel.replace('\\', "/"))
}

pub(crate) fn load_gemini_key() -> String {
    let k = crate::cunning_core::ai_service::gemini::api_key::read_gemini_api_key_env();
    if !k.trim().is_empty() { return k; }
    let raw = std::fs::read_to_string(crate::runtime_paths::ai_providers_path()).unwrap_or_default();
    let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    v.get("gemini").and_then(|g| g.get("api_key")).and_then(|x| x.as_str()).unwrap_or("").trim().to_string()
}
pub(crate) fn load_gemini_model_image() -> String {
    if let Ok(m) = std::env::var("CUNNING_GEMINI_MODEL_IMAGE") { if !m.trim().is_empty() { return m; } }
    if let Ok(m) = std::env::var("CUNNING_GEMINI_IMAGE_MODEL") { if !m.trim().is_empty() { return m; } }
    let raw = std::fs::read_to_string(crate::runtime_paths::ai_providers_path()).unwrap_or_default();
    let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    v.get("gemini").and_then(|g| g.get("model_image").or_else(|| g.get("image_model"))).and_then(|x| x.as_str()).unwrap_or("gemini-3-pro-image-preview").trim().to_string()
}

#[inline]
fn image_size_for_px(px: u32) -> &'static str { if px <= 1024 { "1K" } else if px <= 2048 { "2K" } else { "4K" } }
#[inline]
fn aspect_ratio_for_wh(w: u32, h: u32) -> &'static str {
    if w == 0 || h == 0 { return "1:1"; }
    let a = w as f32 / h as f32;
    [
        ("1:1", 1.0f32), ("2:3", 2.0/3.0), ("3:2", 3.0/2.0), ("3:4", 3.0/4.0), ("4:3", 4.0/3.0),
        ("4:5", 4.0/5.0), ("5:4", 5.0/4.0), ("9:16", 9.0/16.0), ("16:9", 16.0/9.0), ("21:9", 21.0/9.0),
    ]
    .into_iter()
    .map(|(k, v)| (k, (a - v).abs()))
    .min_by(|a, b| a.1.total_cmp(&b.1))
    .map(|(k, _)| k)
    .unwrap_or("1:1")
}

pub(crate) fn gemini_generate_image(timeout_s: i32, api_key: &str, model: &str, system_prompt: &str, user_prompt: &str, refs: &[ImageBlob], out_w: u32, out_h: u32) -> Result<ImageBlob, String> {
    let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent");
    let mut parts: Vec<Value> = vec![serde_json::json!({ "text": format!("{system_prompt}\n\n{user_prompt}") })];
    for r in refs {
        parts.push(serde_json::json!({ "inlineData": { "mimeType": r.mime, "data": base64::engine::general_purpose::STANDARD.encode(&r.bytes) } }));
    }
    let body = serde_json::json!({
        "contents": [{ "role": "user", "parts": parts }],
        "generationConfig": { "responseModalities": ["TEXT", "IMAGE"], "imageConfig": { "aspectRatio": aspect_ratio_for_wh(out_w.max(1), out_h.max(1)), "imageSize": image_size_for_px(out_w.max(out_h).max(1)) } }
    });
    let mut b = Client::builder().connect_timeout(Duration::from_secs(10));
    if timeout_s > 0 { b = b.timeout(Duration::from_secs(timeout_s as u64)); }
    let client = b.build().map_err(|e| e.to_string())?;
    let resp = client.post(url).header("x-goog-api-key", api_key).json(&body).send().map_err(|e| e.to_string())?;
    let status = resp.status();
    let txt = resp.text().unwrap_or_default();
    if !status.is_success() { return Err(format!("HTTP {}: {}", status.as_u16(), txt)); }
    let v: Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
    let parts = v.get("candidates").and_then(|c| c.get(0)).and_then(|c| c.get("content")).and_then(|c| c.get("parts")).and_then(|p| p.as_array()).cloned().unwrap_or_default();
    for p in parts {
        let id = p.get("inlineData").or_else(|| p.get("inline_data"));
        if let Some(x) = id {
            let mime = x.get("mimeType").or_else(|| x.get("mime_type")).and_then(|m| m.as_str()).unwrap_or("").to_string();
            let data = x.get("data").and_then(|d| d.as_str()).unwrap_or("");
            if mime.starts_with("image/") && !data.is_empty() {
                let bytes = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).map_err(|e| e.to_string())?;
                return Ok(ImageBlob { mime, bytes });
            }
        }
    }
    Err("Gemini: no image inlineData returned".to_string())
}

fn load_image_from_assets(rel: &str) -> Result<image::DynamicImage, String> {
    let assets = std::env::current_dir().map_err(|e| e.to_string())?.join("assets");
    let p = assets.join(rel);
    image::open(&p).map_err(|e| format!("image::open({}): {}", p.display(), e))
}

fn read_voxel_size(g: &Geometry) -> f32 {
    g.get_detail_attribute(ATTR_VOXEL_SIZE_DETAIL).and_then(|a| a.as_slice::<f32>()).and_then(|v| v.first().copied()).unwrap_or(0.1).max(0.001)
}

fn read_mask_cells(g: &Geometry) -> HashSet<IVec3> {
    let Some(v) = g.get_detail_attribute(ATTR_VOXEL_MASK_CELLS_I32).and_then(|a| a.as_slice::<i32>()) else { return HashSet::new(); };
    if v.len() % 3 != 0 { return HashSet::new(); }
    let mut out: HashSet<IVec3> = HashSet::with_capacity(v.len() / 3);
    for i in 0..(v.len() / 3) { out.insert(IVec3::new(v[i * 3], v[i * 3 + 1], v[i * 3 + 2])); }
    out
}

fn read_grid_from_input(g: &Geometry) -> Option<vox::DiscreteVoxelGrid> {
    if let Some(grid) = read_discrete_payload(g, read_voxel_size(g)) { return Some(grid); }
    let nid = g.get_detail_attribute("__voxel_node").and_then(|a| a.as_slice::<String>()).and_then(|v| v.first()).and_then(|s| uuid::Uuid::parse_str(s.trim()).ok())?;
    cunning_kernel::nodes::voxel::voxel_edit::voxel_render_get_grid(nid)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct VtxP3N3 { pos: [f32; 3], n: [f32; 3] }

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Cam6 {
    view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    pos: [f32; 4],
    dir: [f32; 4],
    near_far: [f32; 4],
}

struct GuidePipelines { pl: wgpu::RenderPipeline, bgl_cam_dyn: wgpu::BindGroupLayout }
static GUIDE_PPL: OnceCell<GuidePipelines> = OnceCell::new();

fn guide_pipelines(rt: &GpuRuntime) -> &'static GuidePipelines {
    GUIDE_PPL.get_or_init(|| {
        let dev = rt.device();
        let bgl_cam_dyn = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_vox_guides_cam_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: None },
                count: None,
            }],
        });
        let vs = r#"
struct VIn { @location(0) pos: vec3<f32>, @location(1) n: vec3<f32> };
struct Cam { view_proj: mat4x4<f32>, view: mat4x4<f32>, pos: vec4<f32>, dir: vec4<f32>, near_far: vec4<f32> };
@group(0) @binding(0) var<uniform> cam: Cam;
struct VOut { @builtin(position) clip: vec4<f32>, @location(0) n_view: vec3<f32>, @location(1) v_z: f32 };
@vertex fn vs_main(v: VIn) -> VOut {
  let wp = vec4<f32>(v.pos, 1.0);
  let vp = cam.view * wp;
  var o: VOut;
  o.clip = cam.view_proj * wp;
  o.n_view = normalize((cam.view * vec4<f32>(v.n, 0.0)).xyz);
  o.v_z = -vp.z;
  return o;
}
"#;
        let fs = r#"
@group(0) @binding(0) var<uniform> cam: Cam;
@fragment fn fs_main(@location(0) n_view: vec3<f32>, @location(1) v_z: f32) -> @location(0) vec4<f32> {
  let n = n_view * 0.5 + vec3<f32>(0.5);
  let near = cam.near_far.x;
  let far = cam.near_far.y;
  let d = clamp((v_z - near) / max(0.0001, (far - near)), 0.0, 1.0);
  return vec4<f32>(n, d);
}
"#;
        let sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("c3d_vox_guides_sm"), source: wgpu::ShaderSource::Wgsl(format!("{vs}\n{fs}").into()) });
        let pl_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("c3d_vox_guides_pl_layout"), bind_group_layouts: &[&bgl_cam_dyn], immediate_size: 0 });
        let pl = dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("c3d_vox_guides_pl"),
            layout: Some(&pl_layout),
            vertex: wgpu::VertexState {
                module: &sm,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<VtxP3N3>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &sm,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: true, depth_compare: wgpu::CompareFunction::LessEqual, stencil: Default::default(), bias: Default::default() }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        GuidePipelines { pl, bgl_cam_dyn }
    })
}

struct MeshGpu { vb: wgpu::Buffer, ib: wgpu::Buffer, n_idx: u32 }

fn geom_to_tri_p3n3(geo: &Geometry) -> Result<(Vec<VtxP3N3>, Vec<u32>), String> {
    let p = geo.get_point_attribute(crate::cunning_core::core::geometry::attrs::P).and_then(|a| a.as_slice::<Vec3>()).ok_or("Missing @P")?;
    let n_v = geo.get_vertex_attribute(crate::cunning_core::core::geometry::attrs::N).and_then(|a| a.as_slice::<Vec3>());
    let n_p = geo.get_point_attribute(crate::cunning_core::core::geometry::attrs::N).and_then(|a| a.as_slice::<Vec3>());
    let n_prim = geo.get_primitive_attribute(crate::cunning_core::core::geometry::attrs::N).and_then(|a| a.as_slice::<Vec3>());
    let mut verts: Vec<VtxP3N3> = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    for (pi, prim) in geo.primitives().values().iter().enumerate() {
        let vs = prim.vertices();
        if vs.len() < 3 { continue; }
        let prim_n = n_prim.and_then(|v| v.get(pi).copied()).unwrap_or(Vec3::Y);
        let mut prim_vidx: Vec<u32> = Vec::with_capacity(vs.len());
        for &vid in vs {
            let v = geo.vertices().get(vid.into()).ok_or("Vertex id")?;
            let pdi = geo.points().get_dense_index(v.point_id.into()).ok_or("Point dense")?;
            let pos = *p.get(pdi).ok_or("P")?;
            let n = n_v
                .and_then(|nv| geo.vertices().get_dense_index(vid.into()).and_then(|di| nv.get(di).copied()))
                .or_else(|| n_p.and_then(|np| np.get(pdi).copied()))
                .unwrap_or(prim_n);
            let di = verts.len() as u32;
            verts.push(VtxP3N3 { pos: [pos.x, pos.y, pos.z], n: [n.x, n.y, n.z] });
            prim_vidx.push(di);
        }
        for i in 1..(prim_vidx.len() - 1) {
            idx.extend_from_slice(&[prim_vidx[0], prim_vidx[i], prim_vidx[i + 1]]);
        }
    }
    if idx.is_empty() { return Err("No triangles".to_string()); }
    Ok((verts, idx))
}

fn upload_mesh(rt: &GpuRuntime, geo: &Geometry) -> Result<MeshGpu, String> {
    let (verts, idx) = geom_to_tri_p3n3(geo)?;
    let dev = rt.device();
    Ok(MeshGpu {
        vb: dev.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("c3d_vox_vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX }),
        ib: dev.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("c3d_vox_ib"), contents: bytemuck::cast_slice(&idx), usage: wgpu::BufferUsages::INDEX }),
        n_idx: idx.len() as u32,
    })
}

fn readback_rgba8(dev: &wgpu::Device, q: &wgpu::Queue, tex: &wgpu::Texture, w: u32, h: u32) -> Result<Vec<u8>, String> {
    let bpp: usize = 4;
    let bytes_per_row: usize = (((w as usize) * bpp + 255) / 256) * 256;
    let size = (bytes_per_row as u64) * (h as u64);
    let buf = dev.create_buffer(&wgpu::BufferDescriptor { label: Some("c3d_vox_rb"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("c3d_vox_rb_enc") });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes_per_row as u32), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    q.submit([enc.finish()]);
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r.is_ok()); });
    let t0 = Instant::now();
    let timeout = Duration::from_secs(120);
    let mut ok = None;
    while t0.elapsed() < timeout {
        let _ = dev.poll(wgpu::PollType::Poll);
        if let Ok(v) = rx.try_recv() { ok = Some(v); break; }
        std::thread::sleep(Duration::from_millis(2));
    }
    if ok != Some(true) { return Err("GPU readback timeout or failed map_async.".to_string()); }
    let mapped = slice.get_mapped_range();
    let mut out = vec![0u8; (w as usize) * (h as usize) * bpp];
    for y in 0..h as usize {
        let src = &mapped[y * bytes_per_row..y * bytes_per_row + (w as usize * bpp)];
        out[y * w as usize * bpp..(y + 1) * w as usize * bpp].copy_from_slice(src);
    }
    drop(mapped);
    buf.unmap();
    Ok(out)
}

fn bbox_center_extent_voxel(grid: &vox::DiscreteVoxelGrid, voxel_size: f32) -> Option<(Vec3, Vec3)> {
    let (mn, mx) = grid.bounds()?;
    let mnw = Vec3::new(mn.x as f32, mn.y as f32, mn.z as f32) * voxel_size;
    let mxw = Vec3::new((mx.x + 1) as f32, (mx.y + 1) as f32, (mx.z + 1) as f32) * voxel_size;
    Some(((mnw + mxw) * 0.5, (mxw - mnw) * 0.5))
}

fn build_cams(center: Vec3, ext: Vec3) -> [Cam6; 6] {
    let r = ext.length().max(0.001);
    let dist = r * 2.5;
    let half = ext.max_element().max(0.001) * 1.1;
    let near = 0.0;
    let far = dist * 4.0 + r * 2.0;
    let proj = Mat4::orthographic_rh(-half, half, -half, half, near, far);
    let mk = |dir: Vec3, up: Vec3| {
        let pos = center + dir * dist;
        let view = Mat4::look_at_rh(pos, center, up);
        let vp = proj * view;
        Cam6 { view_proj: vp.to_cols_array_2d(), view: view.to_cols_array_2d(), pos: [pos.x, pos.y, pos.z, 0.0], dir: [dir.x, dir.y, dir.z, 0.0], near_far: [near, far, 0.0, 0.0] }
    };
    [mk(Vec3::X, Vec3::Y), mk(-Vec3::X, Vec3::Y), mk(Vec3::Y, Vec3::Z), mk(-Vec3::Y, Vec3::Z), mk(Vec3::Z, Vec3::Y), mk(-Vec3::Z, Vec3::Y)]
}

fn render_guides(rt: &GpuRuntime, mesh: &MeshGpu, cams: &[Cam6; 6], tile: u32) -> Result<Vec<u8>, String> {
    let ppl = guide_pipelines(rt);
    let dev = rt.device();
    let q = rt.queue();
    let w = tile * 3;
    let h = tile * 2;
    let out = dev.create_texture(&wgpu::TextureDescriptor {
        label: Some("c3d_vox_guides_out"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth = dev.create_texture(&wgpu::TextureDescriptor {
        label: Some("c3d_vox_guides_depth"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let stride = 256u64;
    let mut cam_bytes = vec![0u8; (stride * 6) as usize];
    for i in 0..6 {
        let b = bytemuck::bytes_of(&cams[i]);
        cam_bytes[i * stride as usize..i * stride as usize + b.len()].copy_from_slice(b);
    }
    let cam_buf = dev.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("c3d_vox_cam6"), contents: &cam_bytes, usage: wgpu::BufferUsages::UNIFORM });
    let bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("c3d_vox_cam_bg"),
        layout: &ppl.bgl_cam_dyn,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &cam_buf, offset: 0, size: Some(std::num::NonZeroU64::new(std::mem::size_of::<Cam6>() as u64).unwrap()) }) }],
    });
    let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("c3d_vox_guides_enc") });
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("c3d_vox_guides_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &out.create_view(&Default::default()), depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &depth.create_view(&Default::default()), depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&ppl.pl);
        pass.set_vertex_buffer(0, mesh.vb.slice(..));
        pass.set_index_buffer(mesh.ib.slice(..), wgpu::IndexFormat::Uint32);
        for i in 0..6u32 {
            let tx = i % 3;
            let ty = i / 3;
            pass.set_viewport((tx * tile) as f32, (ty * tile) as f32, tile as f32, tile as f32, 0.0, 1.0);
            pass.set_scissor_rect(tx * tile, ty * tile, tile, tile);
            pass.set_bind_group(0, &bg, &[(i * stride as u32) as u32]);
            pass.draw_indexed(0..mesh.n_idx, 0, 0..1);
        }
    }
    q.submit([enc.finish()]);
    readback_rgba8(dev, q, &out, w, h)
}

pub(crate) fn guide_atlas_rgba_gpu(grid: &vox::DiscreteVoxelGrid, voxel_size: f32, tile: u32, max_surface_voxels: usize) -> Result<(Vec<u8>, [Cam6; 6]), String> {
    if grid.voxels.is_empty() { return Err("Empty voxel grid.".to_string()); }
    // Avoid meshing the full volume (can be millions of voxels). Build a capped proxy surface set.
    let mut proxy = vox::DiscreteVoxelGrid::new(voxel_size.max(0.001));
    proxy.palette = grid.palette.clone();
    let mut surface: Vec<(IVec3, u8)> = Vec::new();
    surface.reserve(grid.voxel_count().min(max_surface_voxels.saturating_mul(2)));
    let ns = [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z];
    for (vox::discrete::VoxelCoord(c), v) in grid.voxels.iter() {
        let mut is_surf = false;
        for d in ns { if !grid.is_solid(c.x + d.x, c.y + d.y, c.z + d.z) { is_surf = true; break; } }
        if is_surf { surface.push((*c, v.palette_index.max(1))); }
    }
    if surface.is_empty() { return Err("No surface voxels.".to_string()); }
    let keep_n = max_surface_voxels.max(1);
    if surface.len() <= keep_n {
        for (c, pi) in surface.into_iter() { proxy.set(c.x, c.y, c.z, vox::DiscreteVoxel { palette_index: pi, color_override: None }); }
    } else {
        #[inline] fn hash_u32(mut x: u32) -> u32 { x ^= x >> 16; x = x.wrapping_mul(0x7feb_352d); x ^= x >> 15; x = x.wrapping_mul(0x846c_a68b); x ^= x >> 16; x }
        let p = (keep_n as f32 / surface.len().max(1) as f32).clamp(0.000001, 1.0);
        let thr = (p * (u32::MAX as f32)) as u32;
        let mut kept = 0usize;
        for (c, pi) in surface.into_iter() {
            if kept >= keep_n { break; }
            let h = hash_u32((c.x as u32).wrapping_mul(73856093) ^ (c.y as u32).wrapping_mul(19349663) ^ (c.z as u32).wrapping_mul(83492791) ^ 0xC3D0_5EED);
            if h <= thr { proxy.set(c.x, c.y, c.z, vox::DiscreteVoxel { palette_index: pi, color_override: None }); kept += 1; }
        }
        if proxy.voxels.is_empty() {
            // `surface` was consumed by into_iter above; pick a stable fallback directly from the original grid.
            let (c, pi) = grid.voxels.iter().next().map(|(vox::discrete::VoxelCoord(c), v)| (*c, v.palette_index.max(1))).unwrap();
            proxy.set(c.x, c.y, c.z, vox::DiscreteVoxel { palette_index: pi, color_override: None });
        }
    }
    let surf = discrete_to_surface_mesh(&proxy);
    let rt = GpuRuntime::get_blocking();
    let mesh = upload_mesh(rt, &surf)?;
    let (center, ext) = bbox_center_extent_voxel(grid, voxel_size).ok_or("Missing bounds".to_string())?;
    let cams = build_cams(center, ext);
    let rgba = render_guides(rt, &mesh, &cams, tile)?;
    Ok((rgba, cams))
}

#[derive(Clone)]
struct AtlasSampleCtx {
    tile: u32,
    atlas_w: u32,
    atlas_h: u32,
    tile_w: u32,
    tile_h: u32,
    cams: [Cam6; 6],
    depth_eps: f32,
}

fn atlas_ctx_from_guide(cams: [Cam6; 6], guide_w: u32, guide_h: u32, depth_eps: f32) -> AtlasSampleCtx {
    let tile_w = (guide_w / 3).max(1);
    let tile_h = (guide_h / 2).max(1);
    AtlasSampleCtx { tile: tile_w, atlas_w: guide_w, atlas_h: guide_h, tile_w, tile_h, cams, depth_eps }
}

fn tile_uv(idx: u32, suv: Vec2, ctx: &AtlasSampleCtx) -> Vec2 {
    let tx = (idx % 3) as f32;
    let ty = (idx / 3) as f32;
    Vec2::new((tx * ctx.tile_w as f32 + suv.x * ctx.tile_w as f32) / ctx.atlas_w as f32, (ty * ctx.tile_h as f32 + suv.y * ctx.tile_h as f32) / ctx.atlas_h as f32)
}

fn proj_uv(cam: &Cam6, wpos: Vec3) -> Option<(Vec2, f32)> {
    let vp = Mat4::from_cols_array_2d(&cam.view_proj);
    let clip = vp * wpos.extend(1.0);
    if clip.w.abs() < 1.0e-6 { return None; }
    let ndc = clip.xyz() / clip.w;
    Some((ndc.xy() * 0.5 + Vec2::splat(0.5), ndc.z))
}

fn depth_lin(cam: &Cam6, wpos: Vec3) -> f32 {
    let v = Mat4::from_cols_array_2d(&cam.view);
    let vp = v * wpos.extend(1.0);
    let z = -vp.z;
    let near = cam.near_far[0];
    let far = cam.near_far[1];
    ((z - near) / (far - near).max(1.0e-6)).clamp(0.0, 1.0)
}

fn sample_rgba_nearest(rgba: &[u8], w: u32, h: u32, uv: Vec2) -> [u8; 4] {
    let x = (uv.x.clamp(0.0, 1.0) * (w.saturating_sub(1) as f32)).round() as u32;
    let y = (uv.y.clamp(0.0, 1.0) * (h.saturating_sub(1) as f32)).round() as u32;
    let i = ((y * w + x) * 4) as usize;
    if i + 3 >= rgba.len() { return [0, 0, 0, 0]; }
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

fn try_sample_view(idx: u32, wpos: Vec3, guide_rgba: Option<(&[u8], u32, u32)>, base_rgba: (&[u8], u32, u32), ctx: &AtlasSampleCtx) -> Option<[u8; 4]> {
    let (uv, _ndc_z) = proj_uv(&ctx.cams[idx as usize], wpos)?;
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 { return None; }
    let suv = Vec2::new(uv.x, 1.0 - uv.y);
    let auv = tile_uv(idx, suv, ctx);
    if let Some((g, gw, gh)) = guide_rgba {
        let gd = sample_rgba_nearest(g, gw, gh, auv)[3] as f32 / 255.0;
        let d0 = depth_lin(&ctx.cams[idx as usize], wpos);
        if (gd - d0).abs() > ctx.depth_eps { return None; }
    }
    let (b, bw, bh) = base_rgba;
    let p = sample_rgba_nearest(b, bw, bh, auv);
    (p[3] > 0).then_some(p)
}

fn face_views_sorted(n: Vec3) -> [u32; 6] {
    let dirs: [(u32, Vec3); 6] = [(0, Vec3::X), (1, -Vec3::X), (2, Vec3::Y), (3, -Vec3::Y), (4, Vec3::Z), (5, -Vec3::Z)];
    let mut v = dirs.map(|(i, d)| (i, n.dot(d)));
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    [v[0].0, v[1].0, v[2].0, v[3].0, v[4].0, v[5].0]
}

fn ok_srgb_u8_to_linear_f32(c: u8) -> f32 {
    let x = c as f32 / 255.0;
    if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
}
fn ok_linear_f32_to_srgb_u8(x: f32) -> u8 {
    let x = x.clamp(0.0, 1.0);
    let y = if x <= 0.0031308 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
    (y * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8
}

fn oklab_from_srgb_u8(rgb: [u8; 3]) -> Vec3 {
    // sRGB -> linear -> LMS -> OKLab (Björn Ottosson).
    let r = ok_srgb_u8_to_linear_f32(rgb[0]);
    let g = ok_srgb_u8_to_linear_f32(rgb[1]);
    let b = ok_srgb_u8_to_linear_f32(rgb[2]);
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();
    Vec3::new(
        0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
    )
}

fn srgb_u8_from_oklab(lab: Vec3) -> [u8; 3] {
    let l_ = lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z;
    let m_ = lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z;
    let s_ = lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    let r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
    let g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
    let b = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;
    [ok_linear_f32_to_srgb_u8(r), ok_linear_f32_to_srgb_u8(g), ok_linear_f32_to_srgb_u8(b)]
}

fn kmeans_pp_oklab(colors: &[Vec3], k: usize, iters: usize) -> Vec<Vec3> {
    if colors.is_empty() { return vec![Vec3::new(1.0, 0.0, 0.0)]; }
    let k = k.max(1).min(colors.len());
    let mut centers: Vec<Vec3> = Vec::with_capacity(k);
    centers.push(colors[0]);
    let mut dist2: Vec<f32> = vec![0.0; colors.len()];
    for _ in 1..k {
        let mut sum = 0.0f64;
        for (i, &c) in colors.iter().enumerate() {
            let mut d = f32::MAX;
            for &m in centers.iter() { d = d.min((c - m).length_squared()); }
            dist2[i] = d;
            sum += d as f64;
        }
        if sum <= 1.0e-12 { break; }
        let mut target = (sum * 0.61803398875) as f64; // deterministic "random"
        let mut pick = 0usize;
        for (i, &d) in dist2.iter().enumerate() {
            target -= d as f64;
            if target <= 0.0 { pick = i; break; }
        }
        centers.push(colors[pick]);
    }
    let k = centers.len().max(1);
    let mut assign: Vec<usize> = vec![0; colors.len()];
    for _ in 0..iters.max(1) {
        for (i, &c) in colors.iter().enumerate() {
            let mut best = 0usize;
            let mut best_d = f32::MAX;
            for (j, &m) in centers.iter().enumerate() {
                let d = (c - m).length_squared();
                if d < best_d { best_d = d; best = j; }
            }
            assign[i] = best;
        }
        let mut acc = vec![Vec3::ZERO; k];
        let mut cnt = vec![0u32; k];
        for (i, &a) in assign.iter().enumerate() { acc[a] += colors[i]; cnt[a] += 1; }
        for j in 0..k {
            if cnt[j] > 0 { centers[j] = acc[j] / (cnt[j] as f32); }
        }
    }
    centers
}

pub(crate) fn recolor_grid_from_atlas(
    grid: &vox::DiscreteVoxelGrid,
    base_atlas: &image::DynamicImage,
    guide_atlas: Option<&image::DynamicImage>,
    pal_max: usize,
    mask: Option<&HashSet<IVec3>>,
    depth_eps: f32,
    max_surface_voxels: usize,
) -> vox::DiscreteVoxelGrid {
    if grid.bounds().is_none() { return grid.clone(); }
    if grid.voxel_count() > max_surface_voxels.saturating_mul(32) { return grid.clone(); }
    let (bw, bh) = base_atlas.dimensions();
    if bw == 0 || bh == 0 { return grid.clone(); }
    let base_rgba8 = base_atlas.to_rgba8().into_raw();
    let guide_rgba8 = guide_atlas.map(|g| { let (w, h) = g.dimensions(); (g.to_rgba8().into_raw(), w, h) });
    let (gw, gh) = guide_rgba8.as_ref().map(|(_, w, h)| (*w, *h)).unwrap_or((bw, bh));
    let voxel_size = grid.voxel_size.max(0.001);
    let cams = bbox_center_extent_voxel(grid, voxel_size).map(|(c, e)| build_cams(c, e)).unwrap_or_else(|| build_cams(Vec3::ZERO, Vec3::ONE));
    let ctx = atlas_ctx_from_guide(cams, gw, gh, depth_eps);
    let base_ref = (base_rgba8.as_slice(), bw, bh);

    let mut paint_map: HashMap<IVec3, [u8; 4]> = HashMap::new();
    let mut keep_pi: HashSet<u8> = HashSet::new();
    for (vox::discrete::VoxelCoord(c), v) in grid.voxels.iter() {
        if let Some(m) = mask { if !m.contains(c) { keep_pi.insert(v.palette_index); continue; } }
        let mut cols: Vec<Vec3> = Vec::new();
        for (d, nrm) in [(IVec3::X, Vec3::X), (IVec3::NEG_X, -Vec3::X), (IVec3::Y, Vec3::Y), (IVec3::NEG_Y, -Vec3::Y), (IVec3::Z, Vec3::Z), (IVec3::NEG_Z, -Vec3::Z)] {
            let nb = *c + d;
            if grid.is_solid(nb.x, nb.y, nb.z) { continue; }
            let wpos = (Vec3::new(c.x as f32 + 0.5, c.y as f32 + 0.5, c.z as f32 + 0.5) + nrm * 0.5) * voxel_size;
            let order = face_views_sorted(nrm);
            let guide_ref = guide_rgba8.as_ref().map(|(g, w, h)| (g.as_slice(), *w, *h));
            let mut picked: Option<[u8; 4]> = None;
            for &vidx in order.iter().take(4) {
                if let Some(p) = try_sample_view(vidx, wpos, guide_ref, base_ref, &ctx) { picked = Some(p); break; }
            }
            if let Some(p) = picked {
                cols.push(Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32));
            }
        }
        if cols.is_empty() { keep_pi.insert(v.palette_index); continue; }
        let mut acc = Vec3::ZERO;
        for c0 in cols.iter() { acc += *c0; }
        let c0 = (acc / cols.len() as f32).clamp(Vec3::ZERO, Vec3::splat(255.0));
        paint_map.insert(*c, [c0.x as u8, c0.y as u8, c0.z as u8, 255]);
    }
    if paint_map.is_empty() { return grid.clone(); }

    let mut labs: Vec<Vec3> = Vec::with_capacity(paint_map.len());
    for c in paint_map.values() { labs.push(oklab_from_srgb_u8([c[0], c[1], c[2]])); }
    let centers = kmeans_pp_oklab(&labs, pal_max, 8);
    let mut gen_pal: Vec<vox::discrete::PaletteEntry> = Vec::with_capacity(centers.len() + 1);
    gen_pal.push(vox::discrete::PaletteEntry { color: [0, 0, 0, 0], ..default() });
    for lab in centers {
        let rgb = srgb_u8_from_oklab(lab);
        gen_pal.push(vox::discrete::PaletteEntry { color: [rgb[0], rgb[1], rgb[2], 255], ..default() });
    }

    let mut out = grid.clone();
    let mut new_palette: Vec<vox::discrete::PaletteEntry> = Vec::with_capacity(256);
    new_palette.extend_from_slice(&gen_pal);
    let mut map_old: HashMap<u8, u8> = HashMap::new();
    for pi in keep_pi.into_iter() {
        if new_palette.len() >= 256 { break; }
        let idx = new_palette.len() as u8;
        let e = out.palette.get(pi as usize).cloned().unwrap_or_default();
        new_palette.push(e);
        map_old.insert(pi, idx);
    }
    while new_palette.len() < 256 { new_palette.push(vox::discrete::PaletteEntry::default()); }
    out.palette = new_palette;

    for (vox::discrete::VoxelCoord(c), v) in out.voxels.iter_mut() {
        if let Some(m) = mask { if !m.contains(c) { v.palette_index = *map_old.get(&v.palette_index).unwrap_or(&v.palette_index); continue; } }
        let Some(col) = paint_map.get(c) else { continue; };
        let mut best = 1u8;
        let mut best_d = i32::MAX;
        for i in 1..gen_pal.len().min(256) {
            let pc = gen_pal[i].color;
            let dl = oklab_from_srgb_u8([pc[0], pc[1], pc[2]]) - oklab_from_srgb_u8([col[0], col[1], col[2]]);
            let d = (dl.length_squared() * 1_000_000.0) as i32;
            if d < best_d { best_d = d; best = i as u8; }
        }
        v.palette_index = best.max(1);
    }
    out
}

fn read_ref_image_blob(path: &str) -> Option<ImageBlob> {
    let p = path.trim();
    if p.is_empty() { return None; }
    let abs = std::path::PathBuf::from(p);
    let bytes = std::fs::read(&abs).ok().or_else(|| {
        let assets = std::env::current_dir().ok()?.join("assets");
        std::fs::read(assets.join(p)).ok()
    })?;
    let mime = if p.to_ascii_lowercase().ends_with(".png") { "image/png" } else if p.to_ascii_lowercase().ends_with(".webp") { "image/webp" } else { "image/jpeg" }.to_string();
    Some(ImageBlob { mime, bytes })
}

fn spawn_job(gen: i32, spec: NanoVoxelPainterSpec) -> Receiver<JobResult> {
    let (tx, rx) = unbounded();
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let out = (|| -> Result<(String, String), String> {
            let grid = read_grid_from_input(&spec.in_geo).ok_or("Missing voxel input (payload/cache).")?;
            let tile = spec.tile_res.max(128).min(1024);
            let w = tile * 3;
            let h = tile * 2;
            let (guide_rgba, _cams) = guide_atlas_rgba_gpu(&grid, grid.voxel_size.max(0.001), tile, 250_000)?;
            let guide_name = format!("guide_voxel_{}_{}.png", spec.node_id, gen.max(0));
            let (guide_abs, guide_rel) = file_rel_in_assets(format!("textures/ai_bakes/{}", spec.node_id).as_str(), &guide_name);
            if let Some(p) = guide_abs.parent() { let _ = std::fs::create_dir_all(p); }
            let img: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(w, h, guide_rgba).ok_or("Guide ImageBuffer")?;
            img.save(&guide_abs).map_err(|e| e.to_string())?;

            let api_key = load_gemini_key();
            if api_key.trim().is_empty() { return Err("Missing Gemini API key (GEMINI_API_KEY or settings/ai/providers.json)".to_string()); }
            let model = load_gemini_model_image();
            let guide_blob = ImageBlob { mime: "image/png".to_string(), bytes: std::fs::read(&guide_abs).unwrap_or_default() };
            let mut refs = vec![guide_blob];
            if let Some(r) = read_ref_image_blob(&spec.ref_image) { refs.push(r); }
            let base = gemini_generate_image(spec.timeout_s, &api_key, &model, spec.system_prompt.as_str(), spec.prompt.as_str(), &refs, w, h)?;
            let rgba = image::load_from_memory(&base.bytes).map(|i| i.to_rgba8()).map_err(|e| e.to_string())?;
            let rgba = image::imageops::resize(&rgba, w, h, image::imageops::FilterType::Lanczos3).into_raw();
            let base_name = format!("basecolor_atlas_{}_{}.png", spec.node_id, gen.max(0));
            let (base_abs, base_rel) = file_rel_in_assets(format!("textures/ai_bakes/{}", spec.node_id).as_str(), &base_name);
            let img: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(w, h, rgba).ok_or("Base ImageBuffer")?;
            img.save(&base_abs).map_err(|e| e.to_string())?;
            Ok((guide_rel, base_rel))
        })();
        let (guide_rel, base_rel, err) = match out {
            Ok((g, b)) => (Some(g), Some(b), None),
            Err(e) => (None, None, Some(e)),
        };
        let _ = tx.send(JobResult { node_id: spec.node_id, guide_rel, base_rel, err, elapsed_ms: t0.elapsed().as_millis() });
    });
    rx
}

fn first_src(graph: &crate::nodes::structs::NodeGraph, to: NodeId, to_port: &PortId) -> Option<(NodeId, PortId)> {
    let mut srcs: Vec<(crate::nodes::ConnectionId, NodeId, PortId)> = graph
        .connections
        .values()
        .filter(|c| c.to_node == to && c.to_port.as_str() == to_port.as_str())
        .map(|c| (c.id, c.from_node, c.from_port.clone()))
        .collect();
    srcs.sort_by(|a, b| a.0.cmp(&b.0));
    srcs.into_iter().next().map(|(_, n, p)| (n, p))
}

fn cached_output_geo(graph: &crate::nodes::structs::NodeGraph, nid: NodeId, port: &PortId) -> Option<std::sync::Arc<Geometry>> {
    let is_cda = graph.nodes.get(&nid).map(|n| matches!(n.node_type, NodeType::CDA(_))).unwrap_or(false);
    if is_cda { graph.port_geometry_cache.get(&(nid, port.clone())).cloned() } else { graph.geometry_cache.get(&nid).cloned() }
}

pub(crate) fn nano_voxel_painter_jobs_system(
    mut jobs: ResMut<NanoVoxelPainterJobs>,
    mut graph_res: ResMut<NodeGraphResource>,
    mut graph_changed: MessageWriter<'_, crate::GraphChanged>,
) {
    let g = &mut graph_res.0;
    let now = Instant::now();

    // Start jobs.
    let mut started: Vec<NodeId> = Vec::new();
    for (id, n) in g.nodes.iter() {
        let is_target = matches!(&n.node_type, NodeType::Generic(s) if s == NODE_NANO_VOXEL_PAINTER);
        if !is_target { continue; }
        let gen = p_i32(&n.parameters, PARAM_GENERATE, 0);
        let st = jobs.map.entry(*id).or_default();
        if gen <= st.last_gen || st.inflight { continue; }
        let in_port = port_key::in0();
        let Some((src_n, src_p)) = first_src(g, *id, &in_port) else { continue; };
        let Some(in_geo) = cached_output_geo(g, src_n, &src_p).map(|x| (*x).clone()) else { continue; };
        let spec = NanoVoxelPainterSpec {
            node_id: *id,
            system_prompt: p_str(&n.parameters, crate::nodes::ai_texture::PARAM_AI_SYSTEM_PROMPT, SYS_ATLAS),
            prompt: p_str(&n.parameters, PARAM_PROMPT, ""),
            tile_res: p_i32(&n.parameters, PARAM_TILE_RES, 384).max(128) as u32,
            ref_image: p_str(&n.parameters, PARAM_REF_IMAGE, ""),
            timeout_s: p_i32(&n.parameters, PARAM_TIMEOUT_S, 0),
            in_geo,
        };
        st.last_gen = gen;
        st.inflight = true;
        st.last_poll = Some(now);
        st.rx = Some(spawn_job(gen, spec));
        started.push(*id);
    }
    for id in started {
        if let Some(n) = g.nodes.get_mut(&id) {
            let mut changed = false;
            changed |= set_bool(&mut n.parameters, PARAM_BUSY, true);
            changed |= set_str(&mut n.parameters, PARAM_STATUS, "Generating...".to_string());
            changed |= set_str(&mut n.parameters, PARAM_ERROR, String::new());
            if changed { g.bump_param_revision(); graph_changed.write_default(); }
        }
    }

    // Poll completions.
    let mut done: Vec<JobResult> = Vec::new();
    for st in jobs.map.values_mut() {
        if !st.inflight { continue; }
        let Some(rx) = st.rx.as_ref() else { continue; };
        while let Ok(r) = rx.try_recv() { done.push(r); }
    }

    for r in done {
        let Some(st) = jobs.map.get_mut(&r.node_id) else { continue; };
        st.inflight = false;
        st.rx = None;
        st.last_poll = None;
        if let Some(n) = g.nodes.get_mut(&r.node_id) {
            let mut changed = false;
            changed |= set_bool(&mut n.parameters, PARAM_BUSY, false);
            if let Some(e) = r.err {
                changed |= set_str(&mut n.parameters, PARAM_STATUS, format!("Failed ({}ms)", r.elapsed_ms));
                changed |= set_str(&mut n.parameters, PARAM_ERROR, e);
            } else {
                changed |= set_str(&mut n.parameters, PARAM_STATUS, format!("OK ({}ms)", r.elapsed_ms));
                changed |= set_str(&mut n.parameters, PARAM_ERROR, String::new());
                if let Some(p) = r.guide_rel { changed |= set_str(&mut n.parameters, PARAM_GUIDE_ATLAS, p); }
                if let Some(p) = r.base_rel { changed |= set_str(&mut n.parameters, PARAM_BASE_ATLAS, p); }
            }
            if changed {
                g.bump_param_revision();
                g.mark_dirty(r.node_id);
                graph_changed.write_default();
            }
        }
    }
}

