//! NanoVoxelPointScatter node: reference top view -> AI colored dots -> paint/add voxels.

use base64::Engine;
use bevy::prelude::*;
use crossbeam_channel::{unbounded, Receiver};
use image::{GenericImageView, ImageBuffer, Rgba};
use reqwest::blocking::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::mesh::{Attribute, Geometry};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::nodes::structs::{NodeGraphResource, NodeId, NodeType};
use crate::register_node;

use crate::nodes::voxel::voxel_edit::{discrete_to_surface_mesh, read_discrete_payload, write_discrete_payload, ATTR_VOXEL_SIZE_DETAIL, ATTR_VOXEL_SRC_PRIM};
use cunning_kernel::algorithms::algorithms_editor::voxel as vox;

pub const NODE_NANO_VOXEL_POINT_SCATTER: &str = "Nano Voxel Point Scatter";

const PARAM_PROMPT: &str = "prompt";
const PARAM_REF_IMAGE: &str = "reference_image";
const PARAM_REF_WHITE_THR: &str = "ref_white_threshold";
const PARAM_MIN_VAL: &str = "min_value";
const PARAM_MIN_SAT: &str = "min_saturation";
const PARAM_PI_R: &str = "palette_r";
const PARAM_PI_G: &str = "palette_g";
const PARAM_PI_B: &str = "palette_b";
const PARAM_PAINT_SURFACE: &str = "paint_surface";
const PARAM_Y_OFFSET: &str = "y_offset";
const PARAM_MAX_POINTS: &str = "max_points";
const PARAM_SEED: &str = "seed";
const PARAM_TIMEOUT_S: &str = "timeout_s";
const PARAM_GENERATE: &str = "generate";
const PARAM_SCATTER_IMAGE: &str = "scatter_image";
const PARAM_STATUS: &str = "status";
const PARAM_ERROR: &str = "error";
const PARAM_BUSY: &str = "busy";

const SYS_SCATTER: &str = "You edit a TOP-DOWN reference image and add sparse colored points.\nRules:\n- Output MUST be a single image and nothing else.\n- Keep resolution and framing EXACTLY.\n- Do NOT repaint the base; only add small colored dots.\n- Only place dots on bright/white regions; do not place on black background.\n- Use distinct saturated colors (prefer pure Red/Green/Blue) so downstream can map colors to voxel palette.\n- No text, watermark, or annotations.\n";

#[derive(Default)]
pub struct NanoVoxelPointScatterNode;

impl NodeParameters for NanoVoxelPointScatterNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(crate::nodes::ai_texture::PARAM_AI_SYSTEM_PROMPT, "System Prompt", "AI", ParameterValue::String(SYS_SCATTER.to_string()), ParameterUIType::Code),
            Parameter::new(PARAM_PROMPT, "Prompt", "AI", ParameterValue::String("Scatter a few clusters of colorful flowers, rocks, and props.".to_string()), ParameterUIType::String),
            Parameter::new(PARAM_REF_IMAGE, "Reference Image", "Input", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into(), "jpg".into(), "jpeg".into(), "webp".into()] }),
            Parameter::new(PARAM_REF_WHITE_THR, "Ref White Threshold", "Scatter", ParameterValue::Float(0.30), ParameterUIType::FloatSlider { min: 0.0, max: 1.0 }),
            Parameter::new(PARAM_MIN_VAL, "Min Value", "Scatter", ParameterValue::Float(0.35), ParameterUIType::FloatSlider { min: 0.0, max: 1.0 }),
            Parameter::new(PARAM_MIN_SAT, "Min Saturation", "Scatter", ParameterValue::Float(0.20), ParameterUIType::FloatSlider { min: 0.0, max: 1.0 }),
            Parameter::new(PARAM_PI_R, "Palette R", "Voxel", ParameterValue::Int(2), ParameterUIType::IntSlider { min: 1, max: 255 }),
            Parameter::new(PARAM_PI_G, "Palette G", "Voxel", ParameterValue::Int(3), ParameterUIType::IntSlider { min: 1, max: 255 }),
            Parameter::new(PARAM_PI_B, "Palette B", "Voxel", ParameterValue::Int(4), ParameterUIType::IntSlider { min: 1, max: 255 }),
            Parameter::new(PARAM_PAINT_SURFACE, "Paint Surface", "Voxel", ParameterValue::Bool(true), ParameterUIType::Toggle),
            Parameter::new(PARAM_Y_OFFSET, "Y Offset", "Voxel", ParameterValue::Int(1), ParameterUIType::IntSlider { min: -8, max: 8 }),
            Parameter::new(PARAM_MAX_POINTS, "Max Points", "Scatter", ParameterValue::Int(20000), ParameterUIType::IntSlider { min: 0, max: 200000 }),
            Parameter::new(PARAM_SEED, "Seed", "AI", ParameterValue::Int(0), ParameterUIType::IntSlider { min: 0, max: 2_147_483_647 }),
            Parameter::new(PARAM_TIMEOUT_S, "Timeout (s)", "AI", ParameterValue::Int(0), ParameterUIType::IntSlider { min: 0, max: 1800 }),
            Parameter::new(PARAM_GENERATE, "Generate", "AI", ParameterValue::Int(0), ParameterUIType::BusyButton { busy_param: PARAM_BUSY.to_string(), busy_label: "Generating...".to_string(), busy_label_param: Some(PARAM_STATUS.to_string()) }),
            Parameter::new(PARAM_SCATTER_IMAGE, "Scatter Image (Internal)", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
            Parameter::new(PARAM_STATUS, "Status", "Internal", ParameterValue::String(String::new()), ParameterUIType::String),
            Parameter::new(PARAM_ERROR, "Error", "Internal", ParameterValue::String(String::new()), ParameterUIType::String),
            Parameter::new(PARAM_BUSY, "Busy (Internal)", "Debug", ParameterValue::Bool(false), ParameterUIType::Toggle),
        ]
    }
}

impl NodeOp for NanoVoxelPointScatterNode {
    fn compute(&self, params: &[Parameter], inputs: &[std::sync::Arc<dyn crate::libs::geometry::geo_ref::GeometryRef>]) -> std::sync::Arc<Geometry> {
        puffin::profile_scope!("NanoVoxelPointScatter::compute");
        let input = inputs.first().map(|g| g.materialize()).unwrap_or_else(Geometry::new);
        let grid = read_grid_from_input(&input).unwrap_or_else(|| vox::DiscreteSdfGrid::new(read_voxel_size(&input)));
        let mut out_grid = grid.clone();

        let scatter_rel = p_str(params, PARAM_SCATTER_IMAGE, "").trim().to_string();
        let ref_path = p_str(params, PARAM_REF_IMAGE, "").trim().to_string();
        if !scatter_rel.is_empty() && !ref_path.is_empty() {
            if let (Ok(scatter), Ok(reference)) = (load_image_from_assets(&scatter_rel), load_image_any(&ref_path)) {
                apply_scatter(
                    &grid,
                    &mut out_grid,
                    &scatter,
                    &reference,
                    p_f32(params, PARAM_REF_WHITE_THR, 0.30).clamp(0.0, 1.0),
                    p_f32(params, PARAM_MIN_VAL, 0.35).clamp(0.0, 1.0),
                    p_f32(params, PARAM_MIN_SAT, 0.20).clamp(0.0, 1.0),
                    p_i32(params, PARAM_PI_R, 2),
                    p_i32(params, PARAM_PI_G, 3),
                    p_i32(params, PARAM_PI_B, 4),
                    p_bool(params, PARAM_PAINT_SURFACE, true),
                    p_i32(params, PARAM_Y_OFFSET, 1),
                    p_i32(params, PARAM_MAX_POINTS, 20000).max(0) as usize,
                );
            }
        }

        let mut out = discrete_to_surface_mesh(&out_grid);
        let prim_n = out.primitives().len();
        if prim_n > 0 { out.insert_primitive_attribute(ATTR_VOXEL_SRC_PRIM, Attribute::new(vec![true; prim_n])); }
        out.set_detail_attribute(ATTR_VOXEL_SIZE_DETAIL, vec![out_grid.voxel_size.max(0.001)]);
        write_discrete_payload(&mut out, &out_grid);
        std::sync::Arc::new(out)
    }
}

register_node!(
    NODE_NANO_VOXEL_POINT_SCATTER,
    "AI Texture",
    NanoVoxelPointScatterNode;
    inputs: &["Geometry"],
    outputs: &["Voxel"],
    style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts
);

#[derive(Clone, Debug)]
struct ImageBlob { mime: String, bytes: Vec<u8> }

#[derive(Clone, Debug)]
struct Spec { node_id: NodeId, system_prompt: String, prompt: String, ref_image: String, seed: i32, timeout_s: i32 }

#[derive(Clone, Debug)]
struct JobResult { node_id: NodeId, scatter_rel: Option<String>, err: Option<String>, elapsed_ms: u128 }

#[derive(Resource, Default)]
pub(crate) struct NanoVoxelPointScatterJobs { map: HashMap<NodeId, JobState> }

#[derive(Default)]
struct JobState { last_gen: i32, inflight: bool, rx: Option<Receiver<JobResult>>, last_poll: Option<Instant> }

#[inline]
fn p_i32(params: &[Parameter], name: &str, d: i32) -> i32 { params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::Int(v) = &p.value { Some(*v) } else { None }).unwrap_or(d) }
#[inline]
fn p_f32(params: &[Parameter], name: &str, d: f32) -> f32 { params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::Float(v) = &p.value { Some(*v) } else { None }).unwrap_or(d) }
#[inline]
fn p_bool(params: &[Parameter], name: &str, d: bool) -> bool { params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::Bool(v) = &p.value { Some(*v) } else { None }).unwrap_or(d) }
#[inline]
fn p_str(params: &[Parameter], name: &str, d: &str) -> String { params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None }).unwrap_or_else(|| d.to_string()) }

#[inline]
fn set_str(params: &mut [Parameter], name: &str, v: String) -> bool {
    if let Some(p) = params.iter_mut().find(|p| p.name == name) {
        if let ParameterValue::String(cur) = &mut p.value { if *cur != v { *cur = v; return true; } }
        else { p.value = ParameterValue::String(v); return true; }
    }
    false
}
#[inline]
fn set_bool(params: &mut [Parameter], name: &str, v: bool) -> bool {
    if let Some(p) = params.iter_mut().find(|p| p.name == name) {
        if let ParameterValue::Bool(cur) = &mut p.value { if *cur != v { *cur = v; return true; } }
        else { p.value = ParameterValue::Bool(v); return true; }
    }
    false
}

fn load_gemini_key() -> String {
    let k = crate::cunning_core::ai_service::gemini::api_key::read_gemini_api_key_env();
    if !k.trim().is_empty() { return k; }
    let raw = std::fs::read_to_string(crate::runtime_paths::ai_providers_path()).unwrap_or_default();
    let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    v.get("gemini").and_then(|g| g.get("api_key")).and_then(|x| x.as_str()).unwrap_or("").trim().to_string()
}
fn load_gemini_model_image() -> String {
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

fn gemini_generate_image(timeout_s: i32, api_key: &str, model: &str, system_prompt: &str, user_prompt: &str, refs: &[ImageBlob], out_w: u32, out_h: u32) -> Result<ImageBlob, String> {
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

fn file_rel_in_assets(subdir: &str, name: &str) -> (std::path::PathBuf, String) {
    let assets = std::env::current_dir().ok().unwrap_or_default().join("assets");
    let rel = format!("{}/{}", subdir.trim_matches('/'), name);
    (assets.join(&rel), rel.replace('\\', "/"))
}

fn load_image_from_assets(rel: &str) -> Result<image::DynamicImage, String> {
    let assets = std::env::current_dir().map_err(|e| e.to_string())?.join("assets");
    let p = assets.join(rel);
    image::open(&p).map_err(|e| format!("image::open({}): {}", p.display(), e))
}

fn load_image_any(path: &str) -> Result<image::DynamicImage, String> {
    let p = path.trim();
    if p.is_empty() { return Err("Empty image path".to_string()); }
    let abs = std::path::PathBuf::from(p);
    if abs.exists() { return image::open(&abs).map_err(|e| e.to_string()); }
    load_image_from_assets(p)
}

fn read_ref_blob(path: &str) -> Option<ImageBlob> {
    let p = path.trim();
    if p.is_empty() { return None; }
    let abs = std::path::PathBuf::from(p);
    let bytes = std::fs::read(&abs).ok().or_else(|| {
        let assets = std::env::current_dir().ok()?.join("assets");
        std::fs::read(assets.join(p)).ok()
    })?;
    let pl = p.to_ascii_lowercase();
    let mime = if pl.ends_with(".png") { "image/png" } else if pl.ends_with(".webp") { "image/webp" } else { "image/jpeg" }.to_string();
    Some(ImageBlob { mime, bytes })
}

fn spawn_job(gen: i32, spec: Spec) -> Receiver<JobResult> {
    let (tx, rx) = unbounded();
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let out = (|| -> Result<String, String> {
            let ref_blob = read_ref_blob(&spec.ref_image).ok_or("Missing reference image.".to_string())?;
            let ref_img = image::load_from_memory(&ref_blob.bytes).map_err(|e| e.to_string())?;
            let (w, h) = ref_img.dimensions();
            if w == 0 || h == 0 { return Err("Reference image has zero size.".to_string()); }
            let api_key = load_gemini_key();
            if api_key.trim().is_empty() { return Err("Missing Gemini API key (GEMINI_API_KEY or settings/ai/providers.json)".to_string()); }
            let model = load_gemini_model_image();
            let sys = spec.system_prompt.trim();
            let sys = if sys.is_empty() { SYS_SCATTER } else { sys };
            let prompt = format!("{}\nSeed: {}\n", spec.prompt.trim(), spec.seed);
            let img = gemini_generate_image(spec.timeout_s, &api_key, &model, sys, prompt.as_str(), &[ref_blob], w, h)?;
            let rgba = image::load_from_memory(&img.bytes).map(|i| i.to_rgba8()).map_err(|e| e.to_string())?;
            let rgba = image::imageops::resize(&rgba, w, h, image::imageops::FilterType::Lanczos3).into_raw();
            let name = format!("scatter_{}_{}.png", spec.node_id, gen.max(0));
            let (abs, rel) = file_rel_in_assets(format!("textures/ai_voxel_scatter/{}", spec.node_id).as_str(), &name);
            if let Some(p) = abs.parent() { let _ = std::fs::create_dir_all(p); }
            let img: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(w, h, rgba).ok_or("ImageBuffer".to_string())?;
            img.save(&abs).map_err(|e| e.to_string())?;
            Ok(rel)
        })();
        let (scatter_rel, err) = match out { Ok(p) => (Some(p), None), Err(e) => (None, Some(e)) };
        let _ = tx.send(JobResult { node_id: spec.node_id, scatter_rel, err, elapsed_ms: t0.elapsed().as_millis() });
    });
    rx
}

pub(crate) fn nano_voxel_point_scatter_jobs_system(
    mut jobs: ResMut<NanoVoxelPointScatterJobs>,
    mut graph_res: ResMut<NodeGraphResource>,
    mut graph_changed: MessageWriter<'_, crate::GraphChanged>,
) {
    let g = &mut graph_res.0;
    let now = Instant::now();

    let mut started: Vec<NodeId> = Vec::new();
    for (id, n) in g.nodes.iter() {
        let is_target = matches!(&n.node_type, NodeType::Generic(s) if s == NODE_NANO_VOXEL_POINT_SCATTER);
        if !is_target { continue; }
        let gen = p_i32(&n.parameters, PARAM_GENERATE, 0);
        let st = jobs.map.entry(*id).or_default();
        if gen <= st.last_gen || st.inflight { continue; }
        let spec = Spec {
            node_id: *id,
            system_prompt: p_str(&n.parameters, crate::nodes::ai_texture::PARAM_AI_SYSTEM_PROMPT, SYS_SCATTER),
            prompt: p_str(&n.parameters, PARAM_PROMPT, ""),
            ref_image: p_str(&n.parameters, PARAM_REF_IMAGE, ""),
            seed: p_i32(&n.parameters, PARAM_SEED, 0),
            timeout_s: p_i32(&n.parameters, PARAM_TIMEOUT_S, 0),
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
                if let Some(p) = r.scatter_rel { changed |= set_str(&mut n.parameters, PARAM_SCATTER_IMAGE, p); }
            }
            if changed {
                g.bump_param_revision();
                g.mark_dirty(r.node_id);
                graph_changed.write_default();
            }
        }
    }
}

fn read_voxel_size(g: &Geometry) -> f32 {
    g.get_detail_attribute(ATTR_VOXEL_SIZE_DETAIL).and_then(|a| a.as_slice::<f32>()).and_then(|v| v.first().copied()).unwrap_or(0.1).max(0.001)
}

fn read_grid_from_input(g: &Geometry) -> Option<vox::DiscreteSdfGrid> {
    if let Some(grid) = read_discrete_payload(g, read_voxel_size(g)) { return Some(grid); }
    let nid = g.get_detail_attribute("__voxel_node").and_then(|a| a.as_slice::<String>()).and_then(|v| v.first()).and_then(|s| uuid::Uuid::parse_str(s.trim()).ok())?;
    cunning_kernel::nodes::voxel::voxel_edit::voxel_render_get_grid(nid)
}

#[inline]
fn clamp_pi(v: i32) -> u8 { (v.clamp(1, 255)) as u8 }

fn top_y_map(grid: &vox::DiscreteSdfGrid) -> HashMap<(i32, i32), i32> {
    let mut out: HashMap<(i32, i32), i32> = HashMap::new();
    for (vox::discrete::VoxelCoord(c), _v) in grid.voxels.iter() {
        let k = (c.x, c.z);
        match out.get(&k).copied() {
            Some(y) if y >= c.y => {}
            _ => { out.insert(k, c.y); }
        }
    }
    out
}

#[inline]
fn rgb_sat_val01(p: [u8; 4]) -> (f32, f32, f32) {
    let r = p[0] as f32 * (1.0 / 255.0);
    let g = p[1] as f32 * (1.0 / 255.0);
    let b = p[2] as f32 * (1.0 / 255.0);
    let mx = r.max(g.max(b));
    let mn = r.min(g.min(b));
    let sat = if mx <= 1e-6 { 0.0 } else { (mx - mn) / mx };
    (r, sat, mx)
}

fn apply_scatter(
    grid_in: &vox::DiscreteSdfGrid,
    grid_out: &mut vox::DiscreteSdfGrid,
    scatter: &image::DynamicImage,
    reference: &image::DynamicImage,
    ref_white_thr: f32,
    min_val: f32,
    min_sat: f32,
    pi_r: i32,
    pi_g: i32,
    pi_b: i32,
    paint_surface: bool,
    y_offset: i32,
    max_points: usize,
) {
    let Some((mn, mx)) = grid_in.bounds() else { return; };
    let (w, h) = scatter.dimensions();
    if w == 0 || h == 0 { return; }
    let (rw, rh) = reference.dimensions();
    if rw == 0 || rh == 0 { return; }
    let sx = (w - 1).max(1) as f32;
    let sy = (h - 1).max(1) as f32;
    let rx = (rw - 1).max(1) as f32;
    let ry = (rh - 1).max(1) as f32;
    let dx = (mx.x - mn.x).max(1) as f32;
    let dz = (mx.z - mn.z).max(1) as f32;
    let top = top_y_map(grid_in);
    let scatter = scatter.to_rgba8();
    let reference = reference.to_luma8();
    let (pi_r, pi_g, pi_b) = (clamp_pi(pi_r), clamp_pi(pi_g), clamp_pi(pi_b));

    let mut painted = 0usize;
    'outer: for py in 0..h {
        for px in 0..w {
            if max_points != 0 && painted >= max_points { break 'outer; }
            let sp = scatter.get_pixel(px, py).0;
            if sp[3] == 0 { continue; }
            let (_, sat, val) = rgb_sat_val01(sp);
            if val < min_val || sat < min_sat { continue; }

            let rpx = ((px as f32 / sx) * rx).round() as u32;
            let rpy = ((py as f32 / sy) * ry).round() as u32;
            let rv = reference.get_pixel(rpx.min(rw - 1), rpy.min(rh - 1))[0] as f32 * (1.0 / 255.0);
            if rv < ref_white_thr { continue; }

            let u = px as f32 / sx;
            let v = py as f32 / sy;
            let x = mn.x + (u * dx).round() as i32;
            let z = mn.z + ((1.0 - v) * dz).round() as i32;
            let Some(&y) = top.get(&(x, z)) else { continue; };

            let (r, g, b) = (sp[0], sp[1], sp[2]);
            let pi = if r >= g && r >= b { pi_r } else if g >= r && g >= b { pi_g } else { pi_b };
            let ty = if paint_surface { y } else { y + y_offset };
            grid_out.set(x, ty, z, vox::DiscreteVoxel { palette_index: pi, color_override: None });
            painted += 1;
        }
    }
}

