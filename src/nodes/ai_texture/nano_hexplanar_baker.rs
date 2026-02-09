//! Nano HexPlanar Baker: 6-view atlas generation + UV bake + optional seam fix.

use base64::Engine;
use bevy::prelude::*;
use bytemuck::{Pod, Zeroable};
use crossbeam_channel::{unbounded, Receiver, Sender};
use image::{ImageBuffer, Rgba};
use once_cell::sync::OnceCell;
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;

static TOKIO_RT: OnceCell<tokio::runtime::Runtime> = OnceCell::new();
#[inline]
fn tokio_rt() -> &'static tokio::runtime::Runtime {
    TOKIO_RT.get_or_init(|| tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime"))
}

use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::cunning_core::traits::node_interface::{NodeInteraction, ServiceProvider};
use crate::libs::geometry::attrs;
use crate::mesh::{Attribute as GeoAttr, GeoPrimitive, Geometry, PolygonPrim};
use crate::nodes::gpu::runtime::GpuRuntime;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::nodes::structs::{NodeGraphResource, NodeId, NodeType, PortId};
use crate::nodes::port_key;
use crate::register_node;

pub const NODE_NANO_HEXPLANAR_BAKER: &str = "Nano HexPlanar Baker";

pub const PARAM_PROMPT: &str = "prompt";
pub const PARAM_TILE_RES: &str = "tile_res";
pub const PARAM_UV_RES: &str = "uv_res";
pub const PARAM_SEAM_PX: &str = "seam_px";
pub const PARAM_GEN_BASE: &str = "gen_basecolor";
pub const PARAM_GEN_NORMAL: &str = "gen_normal";
pub const PARAM_GEN_ORM: &str = "gen_orm";
pub const PARAM_GEN_EMISSIVE: &str = "gen_emissive";
pub const PARAM_SEAM_FIX: &str = "seam_fix";
pub const PARAM_GENERATE: &str = "generate";
pub const PARAM_TIMEOUT_S: &str = "timeout_s";
pub const PARAM_STATUS: &str = "status";
pub const PARAM_STAGE_LABEL: &str = "stage_label";
pub const PARAM_THINKING: &str = "thinking";
pub const PARAM_ERROR: &str = "error";
pub const PARAM_BUSY: &str = "busy";

pub const PARAM_GUIDE_ATLAS: &str = "guide_atlas";
pub const PARAM_BASE_ATLAS: &str = "basecolor_atlas";
pub const PARAM_NORMAL_ATLAS: &str = "normal_atlas";
pub const PARAM_ORM_ATLAS: &str = "orm_atlas";
pub const PARAM_EMISSIVE_ATLAS: &str = "emissive_atlas";

pub const PARAM_BASE_UV: &str = "basecolor_uv";
pub const PARAM_NORMAL_UV: &str = "normal_uv";
pub const PARAM_ORM_UV: &str = "orm_uv";
pub const PARAM_EMISSIVE_UV: &str = "emissive_uv";
pub const PARAM_SEAM_MASK: &str = "seam_mask";
pub const PARAM_BASE_UV_FIXED: &str = "basecolor_uv_fixed";

const DEFAULT_PROMPT: &str = "A stylized but physically plausible material.";
const SYS_BASE_ATLAS: &str = "You generate a 3x2 atlas texture for a 3D asset based on a provided 3x2 guide atlas.\nRules:\n- Output MUST be a single image and nothing else.\n- Preserve the 3x2 layout EXACTLY.\n- Each tile corresponds to an orthographic view: row0=[+X,-X,+Y], row1=[-Y,+Z,-Z].\n- No text, watermark, or annotations.\n- Keep style consistent across all 6 tiles.\n";
const SYS_DERIVED_ATLAS: &str = "You generate a derived 3x2 atlas from a given BaseColor 3x2 atlas.\nRules:\n- Output MUST be a single image and nothing else.\n- Preserve the 3x2 layout EXACTLY (no shift/scale).\n- No text, watermark, or annotations.\n";
const SYS_INPAINT: &str = "You repair seams using a mask.\nRules:\n- Input includes an image and a mask image.\n- Treat mask white as regions to modify, black as keep.\n- Output MUST be a single repaired image and nothing else.\n- Do not change style.\n";

#[derive(Default)]
pub struct NanoHexPlanarBakerNode;

impl NodeParameters for NanoHexPlanarBakerNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                crate::nodes::ai_texture::PARAM_AI_SYSTEM_PROMPT,
                "System Prompt (BaseColor Atlas)",
                "AI",
                ParameterValue::String(SYS_BASE_ATLAS.to_string()),
                ParameterUIType::Code,
            ),
            Parameter::new(
                PARAM_PROMPT,
                "Prompt",
                "AI",
                ParameterValue::String(DEFAULT_PROMPT.to_string()),
                ParameterUIType::String,
            ),
            Parameter::new(
                PARAM_TILE_RES,
                "Tile Resolution",
                "AI",
                ParameterValue::Int(512),
                ParameterUIType::IntSlider { min: 128, max: 2048 },
            ),
            Parameter::new(
                PARAM_UV_RES,
                "UV Resolution",
                "AI",
                ParameterValue::Int(1024),
                ParameterUIType::IntSlider { min: 128, max: 4096 },
            ),
            Parameter::new(PARAM_TIMEOUT_S, "Timeout (s)", "AI", ParameterValue::Int(0), ParameterUIType::IntSlider { min: 0, max: 3600 }),
            Parameter::new(
                PARAM_SEAM_PX,
                "Seam Width (px)",
                "AI",
                ParameterValue::Int(8),
                ParameterUIType::IntSlider { min: 1, max: 64 },
            ),
            Parameter::new(PARAM_GEN_BASE, "BaseColor", "Outputs", ParameterValue::Bool(true), ParameterUIType::Toggle),
            Parameter::new(PARAM_GEN_NORMAL, "Normal", "Outputs", ParameterValue::Bool(false), ParameterUIType::Toggle),
            Parameter::new(PARAM_GEN_ORM, "ORM", "Outputs", ParameterValue::Bool(false), ParameterUIType::Toggle),
            Parameter::new(PARAM_GEN_EMISSIVE, "Emissive", "Outputs", ParameterValue::Bool(false), ParameterUIType::Toggle),
            Parameter::new(PARAM_SEAM_FIX, "Seam Fix (BaseColor UV)", "Outputs", ParameterValue::Bool(true), ParameterUIType::Toggle),
            Parameter::new(
                PARAM_GENERATE,
                "Generate",
                "AI",
                ParameterValue::Int(0),
                ParameterUIType::BusyButton {
                    busy_param: PARAM_BUSY.to_string(),
                    busy_label: "Generating...".to_string(),
                    busy_label_param: Some(PARAM_STAGE_LABEL.to_string()),
                },
            ),
            Parameter::new(PARAM_STATUS, "Status", "Internal", ParameterValue::String(String::new()), ParameterUIType::String),
            Parameter::new(PARAM_STAGE_LABEL, "Stage Label", "Internal", ParameterValue::String(String::new()), ParameterUIType::String),
            Parameter::new(PARAM_THINKING, "Thinking", "Internal", ParameterValue::String(String::new()), ParameterUIType::Code),
            Parameter::new(PARAM_ERROR, "Error", "Internal", ParameterValue::String(String::new()), ParameterUIType::String),
            Parameter::new(PARAM_BUSY, "Busy (Internal)", "Debug", ParameterValue::Bool(false), ParameterUIType::Toggle),
            Parameter::new(PARAM_GUIDE_ATLAS, "Guide Atlas", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
            Parameter::new(PARAM_BASE_ATLAS, "BaseColor Atlas", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into(), "jpg".into(), "jpeg".into(), "webp".into()] }),
            Parameter::new(PARAM_NORMAL_ATLAS, "Normal Atlas", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into(), "jpg".into(), "jpeg".into(), "webp".into()] }),
            Parameter::new(PARAM_ORM_ATLAS, "ORM Atlas", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into(), "jpg".into(), "jpeg".into(), "webp".into()] }),
            Parameter::new(PARAM_EMISSIVE_ATLAS, "Emissive Atlas", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into(), "jpg".into(), "jpeg".into(), "webp".into()] }),
            Parameter::new(PARAM_BASE_UV, "BaseColor UV", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
            Parameter::new(PARAM_NORMAL_UV, "Normal UV", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
            Parameter::new(PARAM_ORM_UV, "ORM UV", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
            Parameter::new(PARAM_EMISSIVE_UV, "Emissive UV", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
            Parameter::new(PARAM_SEAM_MASK, "Seam Mask", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
            Parameter::new(PARAM_BASE_UV_FIXED, "BaseColor UV (Fixed)", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
        ]
    }
}

impl NodeOp for NanoHexPlanarBakerNode {
    fn compute(&self, params: &[Parameter], inputs: &[std::sync::Arc<dyn crate::libs::geometry::geo_ref::GeometryRef>]) -> std::sync::Arc<Geometry> {
        let src = inputs.get(0).map(|g| g.materialize()).unwrap_or_else(Geometry::new);
        let mut out = src;
        let base = p_str(params, PARAM_BASE_UV_FIXED, "").trim().to_string().if_empty_then(|| p_str(params, PARAM_BASE_UV, ""));
        let normal = p_str(params, PARAM_NORMAL_UV, "");
        let orm = p_str(params, PARAM_ORM_UV, "");
        let emissive = p_str(params, PARAM_EMISSIVE_UV, "");
        if !base.trim().is_empty() {
            out.set_detail_attribute(attrs::MAT_KIND, vec!["standard".to_string()]);
            out.set_detail_attribute(attrs::MAT_BASECOLOR_TEX, vec![base.trim().to_string()]);
            if !normal.trim().is_empty() { out.set_detail_attribute(attrs::MAT_NORMAL_TEX, vec![normal.trim().to_string()]); }
            if !orm.trim().is_empty() { out.set_detail_attribute(attrs::MAT_ORM_TEX, vec![orm.trim().to_string()]); }
            if !emissive.trim().is_empty() { out.set_detail_attribute(attrs::MAT_EMISSIVE_TEX, vec![emissive.trim().to_string()]); }
        }
        std::sync::Arc::new(out)
    }
}

impl NodeInteraction for NanoHexPlanarBakerNode {
    fn has_hud(&self) -> bool { true }
    fn draw_hud(&self, ui: &mut bevy_egui::egui::Ui, services: &dyn ServiceProvider, node_id: uuid::Uuid) {
        egui_extras::install_image_loaders(ui.ctx());
        ui.label(bevy_egui::egui::RichText::new("Nano HexPlanar Baker").small().weak());
        let g = services.get::<NodeGraphResource>().map(|r| &r.0);
        let Some(node) = g.and_then(|g| g.nodes.get(&node_id)) else { ui.label("(No node)"); return; };
        let get_s = |k: &str| node.parameters.iter().find(|p| p.name == k).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.trim().to_string()) } else { None }).filter(|s| !s.is_empty()).unwrap_or_default();
        let base_atlas = get_s(PARAM_BASE_ATLAS);
        let guide_atlas = get_s(PARAM_GUIDE_ATLAS);
        let status = get_s(PARAM_STATUS);
        if !status.is_empty() { ui.label(bevy_egui::egui::RichText::new(status).small().weak()); }
        let rel = if !base_atlas.is_empty() { base_atlas } else { guide_atlas };
        if let Some(uri) = assets_file_uri(rel.as_str()) {
            let w = 360.0f32.min(ui.available_width()).max(180.0);
            let h = w * (2.0 / 3.0);
            let resp = ui.add(bevy_egui::egui::Image::new(uri).fit_to_exact_size(bevy_egui::egui::vec2(w, h)));
            if resp.hovered() { ui.ctx().set_cursor_icon(bevy_egui::egui::CursorIcon::ZoomIn); }
        } else {
            ui.label("(No atlas yet)");
        }
    }
}

#[inline]
fn assets_file_uri(rel: &str) -> Option<String> {
    let rel = rel.trim();
    if rel.is_empty() { return None; }
    if rel.starts_with("file://") { return Some(rel.to_string()); }
    let cwd = std::env::current_dir().ok()?;
    let p0 = std::path::PathBuf::from(rel);
    let abs = if p0.is_absolute() { p0 } else { cwd.join("assets").join(rel) };
    let abs = abs.canonicalize().unwrap_or(abs);
    let mut p = abs.to_string_lossy().to_string();
    if let Some(s) = p.strip_prefix(r"\\?\") { p = s.to_string(); }
    p = p.replace('\\', "/");
    if let Some(s) = p.strip_prefix("//?/") { p = s.to_string(); }
    Some(format!("file:///{}", p))
}

register_node!(
    NODE_NANO_HEXPLANAR_BAKER,
    "AI Texture",
    NanoHexPlanarBakerNode,
    NanoHexPlanarBakerNode;
    inputs: &["Geometry"],
    outputs: &["Output"],
    style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts
);

trait IfEmptyThen {
    fn if_empty_then(self, f: impl FnOnce() -> String) -> String;
}
impl IfEmptyThen for String {
    fn if_empty_then(self, f: impl FnOnce() -> String) -> String {
        if self.trim().is_empty() { f() } else { self }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    CapturingGuides,
    GeneratingBaseColor,
    GeneratingNormal,
    GeneratingORM,
    GeneratingEmissive,
    Baking,
    SeamMask,
    InpaintingBaseColor,
    RegenNormal,
    Done,
}

impl Stage {
    fn label(self) -> &'static str {
        match self {
            Stage::CapturingGuides => "Generating... (Guides)",
            Stage::GeneratingBaseColor => "Generating... (BaseColor)",
            Stage::GeneratingNormal => "Generating... (Normal)",
            Stage::GeneratingORM => "Generating... (ORM)",
            Stage::GeneratingEmissive => "Generating... (Emissive)",
            Stage::Baking => "Generating... (Baking)",
            Stage::SeamMask => "Generating... (SeamMask)",
            Stage::InpaintingBaseColor => "Generating... (SeamFix)",
            Stage::RegenNormal => "Generating... (NormalFix)",
            Stage::Done => "Done",
        }
    }
}

#[derive(Clone, Debug)]
struct BakerSpec {
    node_id: NodeId,
    gen: i32,
    prompt: String,
    tile_res: u32,
    uv_res: u32,
    seam_px: u32,
    gen_base: bool,
    gen_normal: bool,
    gen_orm: bool,
    gen_emissive: bool,
    seam_fix: bool,
    timeout_s: i32,
    in_geo: Geometry,
}

#[derive(Clone, Debug)]
enum BakerMsg {
    Progress(Stage, String),
    Thinking(String),
    Done(BakerOut),
    Fail(String),
}

#[derive(Clone, Debug, Default)]
struct BakerOut {
    guide_atlas: String,
    base_atlas: String,
    normal_atlas: String,
    orm_atlas: String,
    emissive_atlas: String,
    base_uv: String,
    normal_uv: String,
    orm_uv: String,
    emissive_uv: String,
    seam_mask: String,
    base_uv_fixed: String,
}

#[derive(Resource, Default)]
pub(crate) struct NanoHexPlanarBakerJobs {
    map: HashMap<NodeId, JobState>,
}

#[derive(Default)]
struct JobState {
    last_gen: i32,
    inflight: bool,
    rx: Option<Receiver<BakerMsg>>,
    last_poll: Option<Instant>,
    thinking: String,
}

#[inline]
fn p_i32(params: &[Parameter], name: &str, d: i32) -> i32 {
    params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::Int(v) = &p.value { Some(*v) } else { None }).unwrap_or(d)
}
#[inline]
fn p_u32(params: &[Parameter], name: &str, d: u32) -> u32 { p_i32(params, name, d as i32).max(0) as u32 }
#[inline]
fn p_bool(params: &[Parameter], name: &str, d: bool) -> bool {
    params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::Bool(v) = &p.value { Some(*v) } else { None }).unwrap_or(d)
}
#[inline]
fn p_str(params: &[Parameter], name: &str, d: &str) -> String {
    params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None }).unwrap_or_else(|| d.to_string())
}

#[inline]
fn set_str(params: &mut [Parameter], name: &str, v: String) -> bool {
    if let Some(p) = params.iter_mut().find(|p| p.name == name) {
        if let ParameterValue::String(cur) = &mut p.value {
            if *cur != v { *cur = v; return true; }
        } else { p.value = ParameterValue::String(v); return true; }
    }
    false
}
#[inline]
fn set_bool(params: &mut [Parameter], name: &str, v: bool) -> bool {
    if let Some(p) = params.iter_mut().find(|p| p.name == name) {
        if let ParameterValue::Bool(cur) = &mut p.value {
            if *cur != v { *cur = v; return true; }
        } else { p.value = ParameterValue::Bool(v); return true; }
    }
    false
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

fn has_uv(geo: &Geometry) -> bool {
    geo.get_vertex_attribute(attrs::UV).is_some() || geo.get_point_attribute(attrs::UV).is_some()
}

fn file_rel_in_assets(rel: &str) -> Option<String> {
    let rel = rel.replace('\\', "/");
    if rel.starts_with("assets/") { Some(rel.trim_start_matches("assets/").to_string()) } else { Some(rel) }
}

fn ensure_dir(p: &std::path::Path) -> std::io::Result<()> { std::fs::create_dir_all(p) }

fn load_gemini_key() -> String {
    if let Ok(k) = std::env::var("GEMINI_API_KEY") { if !k.trim().is_empty() { return k; } }
    let path = std::env::current_dir().ok().map(|p| p.join("settings/ai/providers.json"));
    let raw = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    v.get("gemini").and_then(|g| g.get("api_key")).and_then(|x| x.as_str()).unwrap_or("").trim().to_string()
}

fn load_gemini_model_image() -> String {
    if let Ok(m) = std::env::var("CUNNING_GEMINI_MODEL_IMAGE") { if !m.trim().is_empty() { return m; } }
    if let Ok(m) = std::env::var("CUNNING_GEMINI_IMAGE_MODEL") { if !m.trim().is_empty() { return m; } }
    let path = std::env::current_dir().ok().map(|p| p.join("settings/ai/providers.json"));
    let raw = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    v.get("gemini").and_then(|g| g.get("model_image").or_else(|| g.get("image_model"))).and_then(|x| x.as_str()).unwrap_or("gemini-3-pro-image-preview").trim().to_string()
}

#[inline]
fn image_size_for_px(px: u32) -> &'static str {
    if px <= 1024 { "1K" } else if px <= 2048 { "2K" } else { "4K" }
}

#[inline]
fn aspect_ratio_for_wh(w: u32, h: u32) -> &'static str {
    if w == 0 || h == 0 { return "1:1"; }
    let a = w as f32 / h as f32;
    let (best, _d) = [
        ("1:1", 1.0f32),
        ("2:3", 2.0 / 3.0),
        ("3:2", 3.0 / 2.0),
        ("3:4", 3.0 / 4.0),
        ("4:3", 4.0 / 3.0),
        ("4:5", 4.0 / 5.0),
        ("5:4", 5.0 / 4.0),
        ("9:16", 9.0 / 16.0),
        ("16:9", 16.0 / 9.0),
        ("21:9", 21.0 / 9.0),
    ]
    .into_iter()
    .map(|(k, v)| (k, (a - v).abs()))
    .min_by(|a, b| a.1.total_cmp(&b.1))
    .unwrap_or(("1:1", 0.0));
    best
}

#[derive(Clone, Debug)]
struct ImageBlob { mime: String, bytes: Vec<u8> }

fn gemini_generate_image(
    timeout_s: i32,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    refs: &[ImageBlob],
    out_w: u32,
    out_h: u32,
) -> Result<ImageBlob, String> {
    gemini_generate_image_streaming(timeout_s, api_key, model, system_prompt, user_prompt, refs, out_w, out_h, None)
}

fn gemini_generate_image_streaming(
    timeout_s: i32,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    refs: &[ImageBlob],
    out_w: u32,
    out_h: u32,
    tx: Option<&Sender<BakerMsg>>,
) -> Result<ImageBlob, String> {
    let mut parts: Vec<Value> = vec![serde_json::json!({ "text": format!("{system_prompt}\n\n{user_prompt}") })];
    for r in refs {
        parts.push(serde_json::json!({ "inlineData": { "mimeType": r.mime, "data": base64::engine::general_purpose::STANDARD.encode(&r.bytes) } }));
    }
    let body = serde_json::json!({
        "contents": [{ "role": "user", "parts": parts }],
        "generationConfig": {
            "responseModalities": ["TEXT", "IMAGE"],
            "imageConfig": {
                "aspectRatio": aspect_ratio_for_wh(out_w.max(1), out_h.max(1)),
                "imageSize": image_size_for_px(out_w.max(out_h).max(1))
            }
        }
    });

    tokio_rt().block_on(async {
        use futures_lite::StreamExt;
        let connect_s = if timeout_s > 0 { 10 } else { 60 };
        let mut b = reqwest::Client::builder().connect_timeout(Duration::from_secs(connect_s));
        if timeout_s > 0 { b = b.timeout(Duration::from_secs(timeout_s as u64)); }
        let client = b.build().map_err(|e| e.to_string())?;

        let stream_url = format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:streamGenerateContent?alt=sse&key={api_key}");
        let non_stream_url = format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={api_key}");

        let resp = client.post(&stream_url).json(&body).send().await.map_err(|e| e.to_string())?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return gemini_non_stream(&client, &non_stream_url, &body, tx).await;
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("HTTP {}: {}", status.as_u16(), text));
        }

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        let mut out_img: Option<ImageBlob> = None;
        let idle = if timeout_s > 0 { Some(Duration::from_secs(45)) } else { None };
        let mut last_io = tokio::time::Instant::now();

        loop {
            let item: Option<Result<_, reqwest::Error>> = if let Some(idle) = idle {
                tokio::select! {
                    _ = tokio::time::sleep_until(last_io + idle) => { return Err("idle timeout".to_string()); }
                    item = stream.next() => item
                }
            } else {
                stream.next().await
            };
            let Some(item) = item else { break; };
            let bytes = item.map_err(|e| e.to_string())?;
            last_io = tokio::time::Instant::now();
            buffer.push_str(&String::from_utf8_lossy(&bytes));
            while let Some(nl) = buffer.find('\n') {
                let mut line = buffer[..nl].to_string();
                buffer.drain(..nl + 1);
                line = line.trim().to_string();
                if line.is_empty() { continue; }
                let Some(data) = line.strip_prefix("data: ") else { continue; };
                let data = data.trim();
                if data == "[DONE]" { break; }
                let json: Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
                let parts = json.get("candidates").and_then(|c| c.get(0)).and_then(|c| c.get("content")).and_then(|c| c.get("parts")).and_then(|p| p.as_array()).cloned().unwrap_or_default();
                for p in parts {
                    if let Some(text) = p.get("text").and_then(|v| v.as_str()) {
                        if !text.is_empty() { if let Some(tx) = tx { let _ = tx.send(BakerMsg::Thinking(text.to_string())); } }
                    }
                    let id = p.get("inlineData").or_else(|| p.get("inline_data"));
                    if let Some(x) = id {
                        let mime = x.get("mimeType").or_else(|| x.get("mime_type")).and_then(|m| m.as_str()).unwrap_or("").to_string();
                        let data = x.get("data").and_then(|d| d.as_str()).unwrap_or("");
                        if mime.starts_with("image/") && !data.is_empty() && out_img.is_none() {
                            let bytes = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).map_err(|e| e.to_string())?;
                            out_img = Some(ImageBlob { mime, bytes });
                        }
                    }
                }
            }
        }
        out_img.ok_or_else(|| "Gemini: no image inlineData returned".to_string())
    })
}

async fn gemini_non_stream(
    client: &reqwest::Client,
    url: &str,
    body: &Value,
    tx: Option<&Sender<BakerMsg>>,
) -> Result<ImageBlob, String> {
    let resp = client.post(url).json(body).send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let txt = resp.text().await.unwrap_or_default();
    if !status.is_success() { return Err(format!("HTTP {}: {}", status.as_u16(), txt)); }
    let v: Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
    let parts = v.get("candidates").and_then(|c| c.get(0)).and_then(|c| c.get("content")).and_then(|c| c.get("parts")).and_then(|p| p.as_array()).cloned().unwrap_or_default();
    for p in &parts {
        if let Some(text) = p.get("text").and_then(|v| v.as_str()) {
            if !text.is_empty() { if let Some(tx) = tx { let _ = tx.send(BakerMsg::Thinking(text.to_string())); } }
        }
    }
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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vtx { pos: [f32; 3], n: [f32; 3], uv: [f32; 2] }

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Cam6 {
    view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    pos: [f32; 4],
    dir: [f32; 4],
    near_far: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BakeUniforms {
    cams: [Cam6; 6],
    atlas_wh: [f32; 2],
    tile_wh: [f32; 2],
    depth_eps: f32,
    _pad0: f32,
    // Uniform buffers require struct size to be a multiple of 16 bytes (WGSL layout rules).
    // Without this tail padding, Rust size=1080 but WGSL expects 1088.
    _pad1: [f32; 2],
}

pub(crate) struct MeshGpu { pub(crate) vb: wgpu::Buffer, pub(crate) ib: wgpu::Buffer, pub(crate) n_idx: u32, pub(crate) lb: wgpu::Buffer, pub(crate) n_line_idx: u32 }

fn geom_to_tri_buffers(geo: &Geometry) -> Result<(Vec<Vtx>, Vec<u32>, Vec<u32>), String> {
    let p = geo.get_point_attribute(attrs::P).and_then(|a: &GeoAttr| a.as_slice::<Vec3>()).ok_or("Missing @P")?;
    let uv_v = geo.get_vertex_attribute(attrs::UV).and_then(|a: &GeoAttr| a.as_slice::<Vec2>());
    let uv_p = geo.get_point_attribute(attrs::UV).and_then(|a: &GeoAttr| a.as_slice::<Vec2>());
    let n_v = geo.get_vertex_attribute(attrs::N).and_then(|a: &GeoAttr| a.as_slice::<Vec3>());
    let mut verts: Vec<Vtx> = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    let mut line_idx: Vec<u32> = Vec::new();
    for prim in geo.primitives().values() {
        let GeoPrimitive::Polygon(PolygonPrim { vertices }) = prim else { continue };
        if vertices.len() < 3 { continue; }
        let mut prim_vidx: Vec<u32> = Vec::with_capacity(vertices.len());
        for &vid in vertices {
            let v = geo.vertices().get(vid.into()).ok_or("Vertex id")?;
            let pdi = geo.points().get_dense_index(v.point_id.into()).ok_or("Point dense")?;
            let pos = *p.get(pdi).ok_or("P")?;
            let uv = if let Some(u) = uv_v.and_then(|u| geo.vertices().get_dense_index(vid.into()).and_then(|di| u.get(di).copied())) {
                u
            } else if let Some(u) = uv_p.and_then(|u| u.get(pdi).copied()) {
                u
            } else {
                return Err("Missing @uv".to_string());
            };
            let n = n_v.and_then(|n| geo.vertices().get_dense_index(vid.into()).and_then(|di| n.get(di).copied())).unwrap_or(Vec3::Y);
            let di = verts.len() as u32;
            verts.push(Vtx { pos: [pos.x, pos.y, pos.z], n: [n.x, n.y, n.z], uv: [uv.x, uv.y] });
            prim_vidx.push(di);
        }
        for i in 1..(prim_vidx.len() - 1) {
            idx.extend_from_slice(&[prim_vidx[0], prim_vidx[i], prim_vidx[i + 1]]);
        }
        for i in 0..prim_vidx.len() {
            let a = prim_vidx[i];
            let b = prim_vidx[(i + 1) % prim_vidx.len()];
            line_idx.extend_from_slice(&[a, b]);
        }
    }
    if idx.is_empty() { return Err("No triangles".to_string()); }
    Ok((verts, idx, line_idx))
}

pub(crate) fn bbox_center_extent(geo: &Geometry) -> Option<(Vec3, Vec3)> {
    let fp = geo.compute_fingerprint();
    let (min, max) = (fp.bbox_min?, fp.bbox_max?);
    let mn = Vec3::new(min[0], min[1], min[2]);
    let mx = Vec3::new(max[0], max[1], max[2]);
    Some(((mn + mx) * 0.5, (mx - mn) * 0.5))
}

pub(crate) fn build_cams(center: Vec3, ext: Vec3) -> [Cam6; 6] {
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
    [
        mk(Vec3::X, Vec3::Y),  // +X
        mk(-Vec3::X, Vec3::Y), // -X
        mk(Vec3::Y, Vec3::Z),  // +Y
        mk(-Vec3::Y, Vec3::Z), // -Y
        mk(Vec3::Z, Vec3::Y),  // +Z
        mk(-Vec3::Z, Vec3::Y), // -Z
    ]
}

struct AtlasPipelines { guide_pl: wgpu::RenderPipeline, bake_pl: wgpu::RenderPipeline, line_pl: wgpu::RenderPipeline, bgl_cam_dyn: wgpu::BindGroupLayout, bgl_bake: wgpu::BindGroupLayout }
static ATLAS_PPL: OnceCell<AtlasPipelines> = OnceCell::new();

fn pipelines(rt: &GpuRuntime) -> &'static AtlasPipelines {
    ATLAS_PPL.get_or_init(|| {
        let dev = rt.device();
        let bgl_cam_dyn = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_hex_cam_dyn_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bgl_bake = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_hex_bake_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });

        let guide_wgsl = r#"
struct Cam { view_proj: mat4x4<f32>, view: mat4x4<f32>, pos: vec4<f32>, dir: vec4<f32>, near_far: vec4<f32> };
@group(0) @binding(0) var<uniform> cam: Cam;

struct VIn { @location(0) pos: vec3<f32>, @location(1) n: vec3<f32>, @location(2) uv: vec2<f32> };
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

@fragment fn fs_main(@location(0) n_view: vec3<f32>, @location(1) v_z: f32) -> @location(0) vec4<f32> {
  let n = n_view * 0.5 + vec3<f32>(0.5);
  let near = cam.near_far.x;
  let far = cam.near_far.y;
  let d = clamp((v_z - near) / max(0.0001, (far - near)), 0.0, 1.0);
  return vec4<f32>(n, d);
}
"#;
        let guide_sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("c3d_hex_guide"), source: wgpu::ShaderSource::Wgsl(guide_wgsl.into()) });
        let guide_pl = {
            let pl = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("c3d_hex_guide_pl"), bind_group_layouts: &[&bgl_cam_dyn], immediate_size: 0 });
            dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("c3d_hex_guide_ppl"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &guide_sm,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vtx>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &guide_sm,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: true, depth_compare: wgpu::CompareFunction::LessEqual, stencil: Default::default(), bias: Default::default() }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let vs_bake = r#"
struct VIn { @location(0) pos: vec3<f32>, @location(1) n: vec3<f32>, @location(2) uv: vec2<f32> };
struct VOut { @builtin(position) clip: vec4<f32>, @location(0) wpos: vec3<f32>, @location(1) wn: vec3<f32> };
@vertex fn vs_main(v: VIn) -> VOut {
  var o: VOut;
  let uv = v.uv;
  o.clip = vec4<f32>(uv.x * 2.0 - 1.0, (1.0 - uv.y) * 2.0 - 1.0, 0.0, 1.0);
  o.wpos = v.pos;
  o.wn = normalize(v.n);
  return o;
}
"#;
        let fs_bake = r#"
struct Cam { view_proj: mat4x4<f32>, view: mat4x4<f32>, pos: vec4<f32>, dir: vec4<f32>, near_far: vec4<f32> };
struct U { cams: array<Cam, 6>, atlas_wh: vec2<f32>, tile_wh: vec2<f32>, depth_eps: f32, _pad0: f32 };
@group(0) @binding(0) var atlas: texture_2d<f32>;
@group(0) @binding(1) var guides: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> u: U;

fn tile_uv(idx: u32, suv: vec2<f32>) -> vec2<f32> {
  let tx = f32(i32(idx) % 3);
  let ty = f32(i32(idx) / 3);
  let px = (tx * u.tile_wh.x + suv.x * u.tile_wh.x) / u.atlas_wh.x;
  let py = (ty * u.tile_wh.y + suv.y * u.tile_wh.y) / u.atlas_wh.y;
  return vec2<f32>(px, py);
}

fn proj_uv(cam: Cam, wpos: vec3<f32>) -> vec3<f32> {
  let clip = cam.view_proj * vec4<f32>(wpos, 1.0);
  let ndc = clip.xyz / max(1e-6, clip.w);
  let uv = ndc.xy * 0.5 + vec2<f32>(0.5);
  return vec3<f32>(uv, ndc.z);
}

fn depth_lin(cam: Cam, wpos: vec3<f32>) -> f32 {
  let vp = cam.view * vec4<f32>(wpos, 1.0);
  let z = -vp.z;
  let near = cam.near_far.x;
  let far = cam.near_far.y;
  return clamp((z - near) / max(0.0001, (far - near)), 0.0, 1.0);
}

@fragment fn fs_main(@location(0) wpos: vec3<f32>, @location(1) wn: vec3<f32>) -> @location(0) vec4<f32> {
  var best_i: u32 = 0u;
  var best_d: f32 = -1e9;
  for (var i: u32 = 0u; i < 6u; i = i + 1u) {
    let d = dot(wn, normalize(u.cams[i].dir.xyz));
    if (d > best_d) { best_d = d; best_i = i; }
  }
  // try best, then fallback to others if occluded
  for (var k: u32 = 0u; k < 6u; k = k + 1u) {
    let i = (best_i + k) % 6u;
    let puv = proj_uv(u.cams[i], wpos);
    if (puv.x < 0.0 || puv.x > 1.0 || puv.y < 0.0 || puv.y > 1.0) { continue; }
    let auv = tile_uv(i, vec2<f32>(puv.x, 1.0 - puv.y));
    let gd = textureSample(guides, samp, auv).a;
    let d0 = depth_lin(u.cams[i], wpos);
    if (abs(gd - d0) > u.depth_eps) { continue; }
    let c = textureSample(atlas, samp, auv);
    return vec4<f32>(c.rgb, 1.0);
  }
  return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}
"#;
        let bake_sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("c3d_hex_bake"), source: wgpu::ShaderSource::Wgsl(format!("{vs_bake}\n{fs_bake}").into()) });
        let bake_pl = {
            let pl = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("c3d_hex_bake_pl"), bind_group_layouts: &[&bgl_bake], immediate_size: 0 });
            dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("c3d_hex_bake_ppl"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &bake_sm,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vtx>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &bake_sm,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8UnormSrgb, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let vs_line = r#"
struct VIn { @location(0) pos: vec3<f32>, @location(1) n: vec3<f32>, @location(2) uv: vec2<f32> };
struct VOut { @builtin(position) clip: vec4<f32> };
@vertex fn vs_main(v: VIn) -> VOut {
  var o: VOut;
  let uv = v.uv;
  o.clip = vec4<f32>(uv.x * 2.0 - 1.0, (1.0 - uv.y) * 2.0 - 1.0, 0.0, 1.0);
  return o;
}
"#;
        let fs_line = r#"
@fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 1.0, 1.0, 1.0); }
"#;
        let line_sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("c3d_hex_line"), source: wgpu::ShaderSource::Wgsl(format!("{vs_line}\n{fs_line}").into()) });
        let line_pl = {
            let pl = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("c3d_hex_line_pl"), bind_group_layouts: &[], immediate_size: 0 });
            dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("c3d_hex_line_ppl"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &line_sm,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vtx>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &line_sm,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        AtlasPipelines { guide_pl, bake_pl, line_pl, bgl_cam_dyn, bgl_bake }
    })
}

pub(crate) fn upload_mesh(rt: &GpuRuntime, geo: &Geometry) -> Result<MeshGpu, String> {
    let (verts, idx, line_idx) = geom_to_tri_buffers(geo)?;
    let dev = rt.device();
    Ok(MeshGpu {
        vb: dev.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("c3d_hex_vb"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX }),
        ib: dev.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("c3d_hex_ib"), contents: bytemuck::cast_slice(&idx), usage: wgpu::BufferUsages::INDEX }),
        n_idx: idx.len() as u32,
        lb: dev.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("c3d_hex_lb"), contents: bytemuck::cast_slice(&line_idx), usage: wgpu::BufferUsages::INDEX }),
        n_line_idx: line_idx.len() as u32,
    })
}

fn readback_rgba8(
    dev: &wgpu::Device,
    q: &wgpu::Queue,
    tex: &wgpu::Texture,
    w: u32,
    h: u32,
    fmt: wgpu::TextureFormat,
) -> Result<Vec<u8>, String> {
    let bpp: usize = match fmt { wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => 4, _ => 4 };
    let bytes_per_row: usize = (((w as usize) * bpp + 255) / 256) * 256;
    let size = (bytes_per_row as u64) * (h as u64);
    let buf = dev.create_buffer(&wgpu::BufferDescriptor { label: Some("c3d_hex_rb"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("c3d_hex_rb_enc") });
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
    // First-time pipeline compilation can be slow; allow a generous readback timeout.
    let timeout = std::time::Duration::from_secs(120);
    let mut ok = None;
    while t0.elapsed() < timeout {
        let _ = dev.poll(wgpu::PollType::Poll);
        if let Ok(v) = rx.try_recv() { ok = Some(v); break; }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    if ok != Some(true) {
        return Err("GPU readback timeout or failed map_async.".to_string());
    }
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

fn tex_from_rgba8(rt: &GpuRuntime, rgba: &[u8], w: u32, h: u32, srgb: bool) -> wgpu::Texture {
    let dev = rt.device();
    let q = rt.queue();
    let fmt = if srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
    let tex = dev.create_texture(&wgpu::TextureDescriptor {
        label: Some("c3d_hex_src_tex"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: fmt,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    q.write_texture(
        tex.as_image_copy(),
        rgba,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    tex
}

pub(crate) fn render_guides(rt: &GpuRuntime, mesh: &MeshGpu, cams: &[Cam6; 6], tile: u32) -> Result<Vec<u8>, String> {
    let ppl = pipelines(rt);
    let dev = rt.device();
    let q = rt.queue();
    let w = tile * 3;
    let h = tile * 2;
    let out = dev.create_texture(&wgpu::TextureDescriptor {
        label: Some("c3d_hex_guides_out"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth = dev.create_texture(&wgpu::TextureDescriptor {
        label: Some("c3d_hex_guides_depth"),
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
    let cam_buf = dev.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("c3d_hex_cam6"), contents: &cam_bytes, usage: wgpu::BufferUsages::UNIFORM });
    let bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("c3d_hex_cam_bg"),
        layout: &ppl.bgl_cam_dyn,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &cam_buf, offset: 0, size: Some(std::num::NonZeroU64::new(std::mem::size_of::<Cam6>() as u64).unwrap()) }) }],
    });

    let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("c3d_hex_guides_enc") });
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("c3d_hex_guides_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &out.create_view(&Default::default()),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &depth.create_view(&Default::default()), depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&ppl.guide_pl);
        pass.set_vertex_buffer(0, mesh.vb.slice(..));
        pass.set_index_buffer(mesh.ib.slice(..), wgpu::IndexFormat::Uint32);
        for i in 0..6u32 {
            let tx = i % 3;
            let ty = i / 3;
            pass.set_viewport((tx * tile) as f32, (ty * tile) as f32, tile as f32, tile as f32, 0.0, 1.0);
            pass.set_scissor_rect(tx * tile, ty * tile, tile, tile);
            pass.set_bind_group(0, &bg, &[(i as u32 * stride as u32) as u32]);
            pass.draw_indexed(0..mesh.n_idx, 0, 0..1);
        }
    }
    q.submit([enc.finish()]);
    readback_rgba8(dev, q, &out, w, h, wgpu::TextureFormat::Rgba8Unorm)
}

pub(crate) fn bake_uv(rt: &GpuRuntime, mesh: &MeshGpu, cams: &[Cam6; 6], atlas_rgba: &[u8], atlas_srgb: bool, guide_rgba: &[u8], tile: u32, uv_res: u32) -> Result<Vec<u8>, String> {
    let ppl = pipelines(rt);
    let dev = rt.device();
    let q = rt.queue();
    let aw = tile * 3;
    let ah = tile * 2;
    let atlas_tex = tex_from_rgba8(rt, atlas_rgba, aw, ah, atlas_srgb);
    let guide_tex = tex_from_rgba8(rt, guide_rgba, aw, ah, false);
    let sampler = dev.create_sampler(&wgpu::SamplerDescriptor { label: Some("c3d_hex_samp"), address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
    let u = BakeUniforms { cams: *cams, atlas_wh: [aw as f32, ah as f32], tile_wh: [tile as f32, tile as f32], depth_eps: 0.02, _pad0: 0.0, _pad1: [0.0; 2] };
    let u_buf = dev.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("c3d_hex_bake_u"), contents: bytemuck::bytes_of(&u), usage: wgpu::BufferUsages::UNIFORM });
    let bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("c3d_hex_bake_bg"),
        layout: &ppl.bgl_bake,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&atlas_tex.create_view(&Default::default())) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&guide_tex.create_view(&Default::default())) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: u_buf.as_entire_binding() },
        ],
    });
    let out = dev.create_texture(&wgpu::TextureDescriptor {
        label: Some("c3d_hex_bake_out"),
        size: wgpu::Extent3d { width: uv_res, height: uv_res, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("c3d_hex_bake_enc") });
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("c3d_hex_bake_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &out.create_view(&Default::default()),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&ppl.bake_pl);
        pass.set_bind_group(0, &bg, &[]);
        pass.set_vertex_buffer(0, mesh.vb.slice(..));
        pass.set_index_buffer(mesh.ib.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..mesh.n_idx, 0, 0..1);
    }
    q.submit([enc.finish()]);
    readback_rgba8(dev, q, &out, uv_res, uv_res, wgpu::TextureFormat::Rgba8UnormSrgb)
}

pub(crate) fn edge_mask_uv(rt: &GpuRuntime, mesh: &MeshGpu, uv_res: u32) -> Result<Vec<u8>, String> {
    let ppl = pipelines(rt);
    let dev = rt.device();
    let q = rt.queue();
    let out = dev.create_texture(&wgpu::TextureDescriptor {
        label: Some("c3d_hex_edge_out"),
        size: wgpu::Extent3d { width: uv_res, height: uv_res, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("c3d_hex_edge_enc") });
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("c3d_hex_edge_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &out.create_view(&Default::default()),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&ppl.line_pl);
        pass.set_vertex_buffer(0, mesh.vb.slice(..));
        pass.set_index_buffer(mesh.lb.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..mesh.n_line_idx, 0, 0..1);
    }
    q.submit([enc.finish()]);
    readback_rgba8(dev, q, &out, uv_res, uv_res, wgpu::TextureFormat::Rgba8Unorm)
}

pub(crate) fn dilate_mask(mut m: Vec<u8>, w: u32, h: u32, px: u32) -> Vec<u8> {
    let w = w as i32;
    let h = h as i32;
    let src = m.clone();
    let r = px as i32;
    for y in 0..h {
        for x in 0..w {
            let mut on = false;
            'o: for dy in -r..=r {
                for dx in -r..=r {
                    let xx = x + dx;
                    let yy = y + dy;
                    if xx < 0 || yy < 0 || xx >= w || yy >= h { continue; }
                    let i = (yy as usize * w as usize + xx as usize) * 4;
                    if src[i] > 0 { on = true; break 'o; }
                }
            }
            let i = (y as usize * w as usize + x as usize) * 4;
            let v = if on { 255 } else { 0 };
            m[i] = v; m[i + 1] = v; m[i + 2] = v; m[i + 3] = 255;
        }
    }
    m
}

fn save_png_abs(abs_path: &std::path::Path, rgba: &[u8], w: u32, h: u32) -> Result<(), String> {
    let img: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(w, h, rgba.to_vec()).ok_or("ImageBuffer")?;
    img.save(abs_path).map_err(|e| e.to_string())
}

fn load_image_rgba(abs_path: &std::path::Path) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::open(abs_path).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = img.dimensions();
    Ok((img.into_raw(), w, h))
}

fn ext_for_mime(m: &str) -> &'static str {
    if m.contains("png") { "png" } else if m.contains("jpeg") || m.contains("jpg") { "jpg" } else if m.contains("webp") { "webp" } else { "png" }
}

fn spawn_job(spec: BakerSpec) -> Receiver<BakerMsg> {
    let (tx, rx) = unbounded();
    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_baker(spec, tx2)));
        if let Err(p) = r {
            let msg = if let Some(s) = p.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = p.downcast_ref::<String>() {
                s.clone()
            } else {
                "Unknown panic payload".to_string()
            };
            let _ = tx.send(BakerMsg::Fail(format!("Job panicked: {msg} (set RUST_BACKTRACE=1 for stack trace).")));
        }
    });
    rx
}

fn run_baker(spec: BakerSpec, tx: Sender<BakerMsg>) {
    let rt = GpuRuntime::get_blocking();
    let max_dim = rt.device().limits().max_texture_dimension_2d.max(1);
    let max_tile = (max_dim / 3).min(max_dim / 2).max(1);
    let tile_res = spec.tile_res.min(max_tile);
    let uv_res = spec.uv_res.min(max_dim);
    let guide_label = if tile_res != spec.tile_res || uv_res != spec.uv_res {
        format!("{} (tile {}→{}, uv {}→{}, gpu limit {})", Stage::CapturingGuides.label(), spec.tile_res, tile_res, spec.uv_res, uv_res, max_dim)
    } else {
        Stage::CapturingGuides.label().to_string()
    };
    let _ = tx.send(BakerMsg::Progress(Stage::CapturingGuides, guide_label));
    let mesh = match upload_mesh(rt, &spec.in_geo) { Ok(m) => m, Err(e) => { let _ = tx.send(BakerMsg::Fail(e)); return; } };
    let Some((center, ext)) = bbox_center_extent(&spec.in_geo) else { let _ = tx.send(BakerMsg::Fail("Missing bbox".to_string())); return; };
    let cams = build_cams(center, ext);
    let guide_rgba = match render_guides(rt, &mesh, &cams, tile_res) {
        Ok(v) => v,
        Err(e) => { let _ = tx.send(BakerMsg::Fail(format!("Guides capture failed: {e}"))); return; }
    };

    let root = std::env::current_dir().ok().unwrap_or_else(|| std::path::PathBuf::from("."));
    let out_dir = root.join("assets").join("textures").join("ai_bakes").join(spec.node_id.to_string());
    if ensure_dir(&out_dir).is_err() { let _ = tx.send(BakerMsg::Fail("Failed to create output dir".to_string())); return; }

    let gen = spec.gen.max(0);
    let guide_abs = out_dir.join(format!("guide_atlas_{gen}.png"));
    if save_png_abs(&guide_abs, &guide_rgba, tile_res * 3, tile_res * 2).is_err() { let _ = tx.send(BakerMsg::Fail("Failed to save guide atlas".to_string())); return; }

    let api_key = load_gemini_key();
    if api_key.trim().is_empty() { let _ = tx.send(BakerMsg::Fail("Gemini API key missing (GEMINI_API_KEY or settings/ai/providers.json)".to_string())); return; }
    let model = load_gemini_model_image();

    let guide_blob = ImageBlob { mime: "image/png".to_string(), bytes: std::fs::read(&guide_abs).unwrap_or_default() };
    let (atlas_w, atlas_h) = (tile_res * 3, tile_res * 2);

    if !spec.gen_base && !(spec.gen_normal || spec.gen_orm || spec.gen_emissive) {
        let _ = tx.send(BakerMsg::Fail("No outputs selected.".to_string()));
        return;
    }

    let _ = tx.send(BakerMsg::Progress(Stage::GeneratingBaseColor, Stage::GeneratingBaseColor.label().to_string()));
    let base_img = match gemini_generate_image_streaming(spec.timeout_s, &api_key, &model, SYS_BASE_ATLAS, spec.prompt.as_str(), &[guide_blob.clone()], atlas_w, atlas_h, Some(&tx)) {
        Ok(b) => b, Err(e) => { let _ = tx.send(BakerMsg::Fail(e)); return; }
    };
    let base_rgba = match image::load_from_memory(&base_img.bytes).map(|i| i.to_rgba8()).map_err(|e| e.to_string()).and_then(|img| Ok(image::imageops::resize(&img, atlas_w, atlas_h, image::imageops::FilterType::Lanczos3).into_raw())) {
        Ok(v) => v,
        Err(e) => { let _ = tx.send(BakerMsg::Fail(format!("BaseColor decode/resize: {e}"))); return; }
    };
    let base_abs = out_dir.join(format!("basecolor_atlas_{gen}.png"));
    if save_png_abs(&base_abs, &base_rgba, atlas_w, atlas_h).is_err() { let _ = tx.send(BakerMsg::Fail("Failed to save base atlas".to_string())); return; }

    let mut out = BakerOut::default();
    out.guide_atlas = file_rel_in_assets(guide_abs.strip_prefix(root.join("assets")).unwrap_or(&guide_abs).to_string_lossy().as_ref()).unwrap_or_default();
    out.base_atlas = file_rel_in_assets(base_abs.strip_prefix(root.join("assets")).unwrap_or(&base_abs).to_string_lossy().as_ref()).unwrap_or_default();

    let mut normal_abs = None;
    let mut orm_abs = None;
    let mut emissive_abs = None;

    let base_blob = ImageBlob { mime: "image/png".to_string(), bytes: std::fs::read(&base_abs).unwrap_or_default() };

    if spec.gen_normal {
        let _ = tx.send(BakerMsg::Progress(Stage::GeneratingNormal, Stage::GeneratingNormal.label().to_string()));
        let img = match gemini_generate_image_streaming(spec.timeout_s, &api_key, &model, SYS_DERIVED_ATLAS, "Generate a tangent-space Normal 3x2 atlas (RGB encodes XYZ; +Z is blue).", &[base_blob.clone(), guide_blob.clone()], atlas_w, atlas_h, Some(&tx)) {
            Ok(b) => b, Err(e) => { let _ = tx.send(BakerMsg::Fail(e)); return; }
        };
        let rgba = match image::load_from_memory(&img.bytes).map(|i| i.to_rgba8()).map_err(|e| e.to_string()).and_then(|img| Ok(image::imageops::resize(&img, atlas_w, atlas_h, image::imageops::FilterType::Lanczos3).into_raw())) {
            Ok(v) => v,
            Err(e) => { let _ = tx.send(BakerMsg::Fail(format!("Normal decode/resize: {e}"))); return; }
        };
        let p = out_dir.join(format!("normal_atlas_{gen}.png"));
        if save_png_abs(&p, &rgba, atlas_w, atlas_h).is_err() { let _ = tx.send(BakerMsg::Fail("Failed to save normal atlas".to_string())); return; }
        out.normal_atlas = file_rel_in_assets(p.strip_prefix(root.join("assets")).unwrap_or(&p).to_string_lossy().as_ref()).unwrap_or_default();
        normal_abs = Some(p);
    }

    if spec.gen_orm {
        let _ = tx.send(BakerMsg::Progress(Stage::GeneratingORM, Stage::GeneratingORM.label().to_string()));
        let img = match gemini_generate_image_streaming(spec.timeout_s, &api_key, &model, SYS_DERIVED_ATLAS, "Generate an ORM 3x2 atlas. Channel packing: R=AO, G=Roughness, B=Metallic. Keep ranges physically plausible.", &[base_blob.clone(), guide_blob.clone()], atlas_w, atlas_h, Some(&tx)) {
            Ok(b) => b, Err(e) => { let _ = tx.send(BakerMsg::Fail(e)); return; }
        };
        let rgba = match image::load_from_memory(&img.bytes).map(|i| i.to_rgba8()).map_err(|e| e.to_string()).and_then(|img| Ok(image::imageops::resize(&img, atlas_w, atlas_h, image::imageops::FilterType::Lanczos3).into_raw())) {
            Ok(v) => v,
            Err(e) => { let _ = tx.send(BakerMsg::Fail(format!("ORM decode/resize: {e}"))); return; }
        };
        let p = out_dir.join(format!("orm_atlas_{gen}.png"));
        if save_png_abs(&p, &rgba, atlas_w, atlas_h).is_err() { let _ = tx.send(BakerMsg::Fail("Failed to save orm atlas".to_string())); return; }
        out.orm_atlas = file_rel_in_assets(p.strip_prefix(root.join("assets")).unwrap_or(&p).to_string_lossy().as_ref()).unwrap_or_default();
        orm_abs = Some(p);
    }

    if spec.gen_emissive {
        let _ = tx.send(BakerMsg::Progress(Stage::GeneratingEmissive, Stage::GeneratingEmissive.label().to_string()));
        let img = match gemini_generate_image_streaming(spec.timeout_s, &api_key, &model, SYS_DERIVED_ATLAS, "Generate an Emissive 3x2 atlas. Black means no emission; avoid affecting non-emissive regions.", &[base_blob.clone(), guide_blob.clone()], atlas_w, atlas_h, Some(&tx)) {
            Ok(b) => b, Err(e) => { let _ = tx.send(BakerMsg::Fail(e)); return; }
        };
        let rgba = match image::load_from_memory(&img.bytes).map(|i| i.to_rgba8()).map_err(|e| e.to_string()).and_then(|img| Ok(image::imageops::resize(&img, atlas_w, atlas_h, image::imageops::FilterType::Lanczos3).into_raw())) {
            Ok(v) => v,
            Err(e) => { let _ = tx.send(BakerMsg::Fail(format!("Emissive decode/resize: {e}"))); return; }
        };
        let p = out_dir.join(format!("emissive_atlas_{gen}.png"));
        if save_png_abs(&p, &rgba, atlas_w, atlas_h).is_err() { let _ = tx.send(BakerMsg::Fail("Failed to save emissive atlas".to_string())); return; }
        out.emissive_atlas = file_rel_in_assets(p.strip_prefix(root.join("assets")).unwrap_or(&p).to_string_lossy().as_ref()).unwrap_or_default();
        emissive_abs = Some(p);
    }

    let _ = tx.send(BakerMsg::Progress(Stage::Baking, Stage::Baking.label().to_string()));
    let base_uv_rgba = match bake_uv(rt, &mesh, &cams, &base_rgba, true, &guide_rgba, tile_res, uv_res) {
        Ok(v) => v,
        Err(e) => { let _ = tx.send(BakerMsg::Fail(format!("UV bake failed: {e}"))); return; }
    };
    let base_uv_abs = out_dir.join(format!("basecolor_uv_{gen}.png"));
    if save_png_abs(&base_uv_abs, &base_uv_rgba, uv_res, uv_res).is_err() { let _ = tx.send(BakerMsg::Fail("Failed to save base uv".to_string())); return; }
    out.base_uv = file_rel_in_assets(base_uv_abs.strip_prefix(root.join("assets")).unwrap_or(&base_uv_abs).to_string_lossy().as_ref()).unwrap_or_default();

    if let Some(p) = normal_abs.as_ref() {
        let (rgba, _w, _h) = match load_image_rgba(p) { Ok(v) => v, Err(e) => { let _ = tx.send(BakerMsg::Fail(e)); return; } };
        let uv = match bake_uv(rt, &mesh, &cams, &rgba, false, &guide_rgba, tile_res, uv_res) {
            Ok(v) => v,
            Err(e) => { let _ = tx.send(BakerMsg::Fail(format!("Normal UV bake failed: {e}"))); return; }
        };
        let abs = out_dir.join(format!("normal_uv_{gen}.png"));
        if save_png_abs(&abs, &uv, uv_res, uv_res).is_err() { let _ = tx.send(BakerMsg::Fail("Failed to save normal uv".to_string())); return; }
        out.normal_uv = file_rel_in_assets(abs.strip_prefix(root.join("assets")).unwrap_or(&abs).to_string_lossy().as_ref()).unwrap_or_default();
    }
    if let Some(p) = orm_abs.as_ref() {
        let (rgba, _, _) = match load_image_rgba(p) { Ok(v) => v, Err(e) => { let _ = tx.send(BakerMsg::Fail(e)); return; } };
        let uv = match bake_uv(rt, &mesh, &cams, &rgba, false, &guide_rgba, tile_res, uv_res) {
            Ok(v) => v,
            Err(e) => { let _ = tx.send(BakerMsg::Fail(format!("ORM UV bake failed: {e}"))); return; }
        };
        let abs = out_dir.join(format!("orm_uv_{gen}.png"));
        if save_png_abs(&abs, &uv, uv_res, uv_res).is_err() { let _ = tx.send(BakerMsg::Fail("Failed to save orm uv".to_string())); return; }
        out.orm_uv = file_rel_in_assets(abs.strip_prefix(root.join("assets")).unwrap_or(&abs).to_string_lossy().as_ref()).unwrap_or_default();
    }
    if let Some(p) = emissive_abs.as_ref() {
        let (rgba, _, _) = match load_image_rgba(p) { Ok(v) => v, Err(e) => { let _ = tx.send(BakerMsg::Fail(e)); return; } };
        let uv = match bake_uv(rt, &mesh, &cams, &rgba, true, &guide_rgba, tile_res, uv_res) {
            Ok(v) => v,
            Err(e) => { let _ = tx.send(BakerMsg::Fail(format!("Emissive UV bake failed: {e}"))); return; }
        };
        let abs = out_dir.join(format!("emissive_uv_{gen}.png"));
        if save_png_abs(&abs, &uv, uv_res, uv_res).is_err() { let _ = tx.send(BakerMsg::Fail("Failed to save emissive uv".to_string())); return; }
        out.emissive_uv = file_rel_in_assets(abs.strip_prefix(root.join("assets")).unwrap_or(&abs).to_string_lossy().as_ref()).unwrap_or_default();
    }

    let _ = tx.send(BakerMsg::Progress(Stage::SeamMask, Stage::SeamMask.label().to_string()));
    let edge = match edge_mask_uv(rt, &mesh, uv_res) {
        Ok(v) => v,
        Err(e) => { let _ = tx.send(BakerMsg::Fail(format!("SeamMask render failed: {e}"))); return; }
    };
    let mask = dilate_mask(edge, uv_res, uv_res, spec.seam_px);
    let mask_abs = out_dir.join(format!("seam_mask_{gen}.png"));
    if save_png_abs(&mask_abs, &mask, uv_res, uv_res).is_err() { let _ = tx.send(BakerMsg::Fail("Failed to save seam mask".to_string())); return; }
    out.seam_mask = file_rel_in_assets(mask_abs.strip_prefix(root.join("assets")).unwrap_or(&mask_abs).to_string_lossy().as_ref()).unwrap_or_default();

    if spec.seam_fix {
        let _ = tx.send(BakerMsg::Progress(Stage::InpaintingBaseColor, Stage::InpaintingBaseColor.label().to_string()));
        let base_uv_blob = ImageBlob { mime: "image/png".to_string(), bytes: std::fs::read(&base_uv_abs).unwrap_or_default() };
        let mask_blob = ImageBlob { mime: "image/png".to_string(), bytes: std::fs::read(&mask_abs).unwrap_or_default() };
        let img = match gemini_generate_image_streaming(spec.timeout_s, &api_key, &model, SYS_INPAINT, "Repair seams in the first image using the second image as mask (white=fix). Output the repaired image.", &[base_uv_blob.clone(), mask_blob.clone()], uv_res, uv_res, Some(&tx)) {
            Ok(b) => b, Err(e) => { let _ = tx.send(BakerMsg::Fail(e)); return; }
        };
        let fixed_rgba = match image::load_from_memory(&img.bytes).map(|i| i.to_rgba8()).map_err(|e| e.to_string()).and_then(|img| Ok(image::imageops::resize(&img, uv_res, uv_res, image::imageops::FilterType::Lanczos3).into_raw())) {
            Ok(v) => v,
            Err(e) => { let _ = tx.send(BakerMsg::Fail(format!("SeamFix decode/resize: {e}"))); return; }
        };
        let fixed_abs = out_dir.join(format!("basecolor_uv_fixed_{gen}.png"));
        if save_png_abs(&fixed_abs, &fixed_rgba, uv_res, uv_res).is_err() { let _ = tx.send(BakerMsg::Fail("Failed to save seam-fixed base uv".to_string())); return; }
        out.base_uv_fixed = file_rel_in_assets(fixed_abs.strip_prefix(root.join("assets")).unwrap_or(&fixed_abs).to_string_lossy().as_ref()).unwrap_or_default();

        if spec.gen_normal {
            let _ = tx.send(BakerMsg::Progress(Stage::RegenNormal, Stage::RegenNormal.label().to_string()));
            let ref_blob = ImageBlob { mime: "image/png".to_string(), bytes: std::fs::read(&fixed_abs).unwrap_or_default() };
            let nimg = match gemini_generate_image_streaming(spec.timeout_s, &api_key, &model, "You generate a tangent-space normal map for a UV texture.\n- Output MUST be a single image.\n- Same resolution.\n- No shift/scale.\n", "Generate a tangent-space Normal map for this UV BaseColor. Preserve UV alignment exactly.", &[ref_blob], uv_res, uv_res, Some(&tx)) {
                Ok(b) => b, Err(e) => { let _ = tx.send(BakerMsg::Fail(e)); return; }
            };
            let n_rgba = match image::load_from_memory(&nimg.bytes).map(|i| i.to_rgba8()).map_err(|e| e.to_string()).and_then(|img| Ok(image::imageops::resize(&img, uv_res, uv_res, image::imageops::FilterType::Lanczos3).into_raw())) {
                Ok(v) => v,
                Err(e) => { let _ = tx.send(BakerMsg::Fail(format!("NormalFix decode/resize: {e}"))); return; }
            };
            let n_abs = out_dir.join(format!("normal_uv_fixed_{gen}.png"));
            if save_png_abs(&n_abs, &n_rgba, uv_res, uv_res).is_err() { let _ = tx.send(BakerMsg::Fail("Failed to save normal uv fixed".to_string())); return; }
            out.normal_uv = file_rel_in_assets(n_abs.strip_prefix(root.join("assets")).unwrap_or(&n_abs).to_string_lossy().as_ref()).unwrap_or_default();
        }
    }

    let _ = tx.send(BakerMsg::Progress(Stage::Done, Stage::Done.label().to_string()));
    let _ = tx.send(BakerMsg::Done(out));
}

pub(crate) fn nano_hexplanar_baker_jobs_system(
    mut node_graph_res: ResMut<NodeGraphResource>,
    mut jobs: ResMut<NanoHexPlanarBakerJobs>,
    mut graph_changed: MessageWriter<'_, crate::GraphChanged>,
) {
    let g = &mut node_graph_res.0;
    let now = Instant::now();
    let nodes: Vec<(NodeId, Vec<Parameter>)> = g.nodes.iter().filter_map(|(id, n)| matches!(&n.node_type, NodeType::Generic(s) if s == NODE_NANO_HEXPLANAR_BAKER).then_some((*id, n.parameters.clone()))).collect();
    for (node_id, mut params) in nodes {
        let st = jobs.map.entry(node_id).or_default();
        let gen = p_i32(&params, PARAM_GENERATE, 0);
        if gen != st.last_gen && !st.inflight {
            st.last_gen = gen;
            st.inflight = true;
            st.last_poll = Some(now);
            st.thinking.clear();
            let mut changed = false;
            changed |= set_bool(&mut params, PARAM_BUSY, true);
            changed |= set_str(&mut params, PARAM_ERROR, String::new());
            changed |= set_str(&mut params, PARAM_STATUS, "Starting...".to_string());
            changed |= set_str(&mut params, PARAM_STAGE_LABEL, Stage::CapturingGuides.label().to_string());
            changed |= set_str(&mut params, PARAM_THINKING, String::new());

            let in_port = port_key::in0();
            let Some((src_n, src_p)) = first_src(g, node_id, &in_port) else {
                changed |= set_str(&mut params, PARAM_ERROR, "No input geometry connected.".to_string());
                changed |= set_str(&mut params, PARAM_STATUS, "Failed: No input geometry.".to_string());
                changed |= set_str(&mut params, PARAM_STAGE_LABEL, "Generate".to_string());
                changed |= set_bool(&mut params, PARAM_BUSY, false);
                st.inflight = false;
                if changed {
                    if let Some(n) = g.nodes.get_mut(&node_id) { n.parameters = params.clone(); g.bump_param_revision(); graph_changed.write(crate::GraphChanged); }
                }
                continue;
            };
            let Some(in_geo) = cached_output_geo(g, src_n, &src_p).map(|x| (*x).clone()) else {
                changed |= set_str(&mut params, PARAM_ERROR, "Input geometry not cooked yet.".to_string());
                changed |= set_str(&mut params, PARAM_STATUS, "Waiting for cook...".to_string());
                changed |= set_str(&mut params, PARAM_STAGE_LABEL, "Waiting...".to_string());
                changed |= set_bool(&mut params, PARAM_BUSY, false);
                st.inflight = false;
                if changed {
                    if let Some(n) = g.nodes.get_mut(&node_id) { n.parameters = params.clone(); g.bump_param_revision(); graph_changed.write(crate::GraphChanged); }
                }
                continue;
            };
            if !has_uv(&in_geo) {
                changed |= set_str(&mut params, PARAM_ERROR, "Missing @uv (vertex or point attribute).".to_string());
                changed |= set_str(&mut params, PARAM_STATUS, "Failed: Missing UV.".to_string());
                changed |= set_str(&mut params, PARAM_STAGE_LABEL, "Generate".to_string());
                changed |= set_bool(&mut params, PARAM_BUSY, false);
                st.inflight = false;
                if changed {
                    if let Some(n) = g.nodes.get_mut(&node_id) { n.parameters = params.clone(); g.bump_param_revision(); graph_changed.write(crate::GraphChanged); }
                }
                continue;
            }
            let spec = BakerSpec {
                node_id,
                gen,
                prompt: p_str(&params, PARAM_PROMPT, DEFAULT_PROMPT),
                tile_res: p_u32(&params, PARAM_TILE_RES, 512).max(128).min(2048),
                uv_res: p_u32(&params, PARAM_UV_RES, 1024).max(128).min(4096),
                seam_px: p_u32(&params, PARAM_SEAM_PX, 8).max(1).min(64),
                gen_base: {
                    let b = p_bool(&params, PARAM_GEN_BASE, true);
                    let n = p_bool(&params, PARAM_GEN_NORMAL, false);
                    let o = p_bool(&params, PARAM_GEN_ORM, false);
                    let e = p_bool(&params, PARAM_GEN_EMISSIVE, false);
                    b || n || o || e
                },
                gen_normal: p_bool(&params, PARAM_GEN_NORMAL, false),
                gen_orm: p_bool(&params, PARAM_GEN_ORM, false),
                gen_emissive: p_bool(&params, PARAM_GEN_EMISSIVE, false),
                seam_fix: p_bool(&params, PARAM_SEAM_FIX, true),
                timeout_s: p_i32(&params, PARAM_TIMEOUT_S, 0),
                in_geo,
            };
            st.rx = Some(spawn_job(spec));
            if changed {
                if let Some(n) = g.nodes.get_mut(&node_id) { n.parameters = params.clone(); g.bump_param_revision(); graph_changed.write(crate::GraphChanged); }
            }
        }

        if st.inflight {
            let mut done: Option<BakerOut> = None;
            let mut fail: Option<String> = None;
            let mut prog: Option<(Stage, String)> = None;
            if let Some(rx) = &st.rx {
                while let Ok(m) = rx.try_recv() {
                    match m {
                        BakerMsg::Progress(s, t) => prog = Some((s, t)),
                        BakerMsg::Thinking(t) => { st.thinking.push_str(&t); st.thinking.push('\n'); },
                        BakerMsg::Done(o) => done = Some(o),
                        BakerMsg::Fail(e) => fail = Some(e),
                    }
                }
            }
            let mut changed = false;
            if let Some((_s, t)) = prog {
                changed |= set_str(&mut params, PARAM_STAGE_LABEL, t.clone());
                changed |= set_str(&mut params, PARAM_STATUS, t);
                st.last_poll = Some(now);
            }
            if !st.thinking.is_empty() {
                changed |= set_str(&mut params, PARAM_THINKING, st.thinking.clone());
            }
            if let Some(err) = fail {
                changed |= set_str(&mut params, PARAM_ERROR, err.clone());
                changed |= set_str(&mut params, PARAM_STATUS, format!("Failed: {err}"));
                changed |= set_str(&mut params, PARAM_STAGE_LABEL, "Generate".to_string());
                changed |= set_bool(&mut params, PARAM_BUSY, false);
                st.inflight = false;
                st.rx = None;
            }
            if let Some(o) = done {
                changed |= set_str(&mut params, PARAM_GUIDE_ATLAS, o.guide_atlas);
                changed |= set_str(&mut params, PARAM_BASE_ATLAS, o.base_atlas);
                changed |= set_str(&mut params, PARAM_NORMAL_ATLAS, o.normal_atlas);
                changed |= set_str(&mut params, PARAM_ORM_ATLAS, o.orm_atlas);
                changed |= set_str(&mut params, PARAM_EMISSIVE_ATLAS, o.emissive_atlas);
                changed |= set_str(&mut params, PARAM_BASE_UV, o.base_uv);
                changed |= set_str(&mut params, PARAM_NORMAL_UV, o.normal_uv);
                changed |= set_str(&mut params, PARAM_ORM_UV, o.orm_uv);
                changed |= set_str(&mut params, PARAM_EMISSIVE_UV, o.emissive_uv);
                changed |= set_str(&mut params, PARAM_SEAM_MASK, o.seam_mask);
                changed |= set_str(&mut params, PARAM_BASE_UV_FIXED, o.base_uv_fixed);
                changed |= set_str(&mut params, PARAM_ERROR, String::new());
                changed |= set_str(&mut params, PARAM_STATUS, "Done".to_string());
                changed |= set_str(&mut params, PARAM_STAGE_LABEL, "Done".to_string());
                changed |= set_bool(&mut params, PARAM_BUSY, false);
                st.inflight = false;
                st.rx = None;
            }
            let timeout_s = p_i32(&params, PARAM_TIMEOUT_S, 0).max(0) as u64;
            if timeout_s != 0 && st.last_poll.is_some_and(|t| now.duration_since(t) > Duration::from_secs(timeout_s)) {
                changed |= set_str(&mut params, PARAM_ERROR, "Job timed out.".to_string());
                changed |= set_str(&mut params, PARAM_STATUS, "Failed: timeout".to_string());
                changed |= set_str(&mut params, PARAM_STAGE_LABEL, "Generate".to_string());
                changed |= set_bool(&mut params, PARAM_BUSY, false);
                st.inflight = false;
                st.rx = None;
            }
            if changed {
                if let Some(n) = g.nodes.get_mut(&node_id) {
                    n.parameters = params;
                    g.bump_param_revision();
                    graph_changed.write(crate::GraphChanged);
                }
            }
        }
    }
}

