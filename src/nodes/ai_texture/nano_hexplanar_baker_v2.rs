//! Nano HexPlanar Baker V2: BaseColor UV + AI Height UV + deterministic Normal UV.

use base64::Engine;
use bevy::prelude::*;
use crossbeam_channel::{unbounded, Receiver, Sender};
use image::{ImageBuffer, Rgba};
use once_cell::sync::OnceCell;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::cunning_core::traits::node_interface::{NodeInteraction, NodeOp, NodeParameters, ServiceProvider};
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::Geometry;
use crate::nodes::gpu::runtime::GpuRuntime;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::nodes::structs::{NodeGraphResource, NodeId, NodeType, PortId};
use crate::nodes::port_key;
use crate::register_node;

pub const NODE_NANO_HEXPLANAR_BAKER_V2: &str = "Nano HexPlanar Baker V2";

pub const PARAM_PROMPT_BASE: &str = "prompt_basecolor";
pub const PARAM_PROMPT_HEIGHT: &str = "prompt_height";
pub const PARAM_TILE_RES: &str = "tile_res";
pub const PARAM_UV_RES: &str = "uv_res";
pub const PARAM_SEAM_PX: &str = "seam_px";
pub const PARAM_SEAM_FIX: &str = "seam_fix";
pub const PARAM_HEIGHT_STRENGTH: &str = "height_strength";
pub const PARAM_TIMEOUT_S: &str = "timeout_s";
pub const PARAM_GENERATE: &str = "generate";
pub const PARAM_STATUS: &str = "status";
pub const PARAM_STAGE_LABEL: &str = "stage_label";
pub const PARAM_THINKING: &str = "thinking";
pub const PARAM_ERROR: &str = "error";
pub const PARAM_BUSY: &str = "busy";

pub const PARAM_GUIDE_ATLAS: &str = "guide_atlas";
pub const PARAM_BASE_ATLAS: &str = "basecolor_atlas";
pub const PARAM_BASE_UV: &str = "basecolor_uv";
pub const PARAM_BASE_UV_FIXED: &str = "basecolor_uv_fixed";
pub const PARAM_SEAM_MASK: &str = "seam_mask";
pub const PARAM_HEIGHT_UV: &str = "height_uv";
pub const PARAM_NORMAL_UV: &str = "normal_uv";

const DEFAULT_PROMPT_BASE: &str = "A stylized but physically plausible material. Output pure albedo (no shading, no AO, no highlights).";
const DEFAULT_PROMPT_HEIGHT: &str = "Generate a UV-space displacement/height map for the input basecolor UV. Output MUST be a single grayscale image (R=G=B). Black=low, White=high. No shadows, no lighting, no AO, no text/watermark. Preserve UV alignment exactly.";

const SYS_BASE_ATLAS: &str = "You generate a 3x2 atlas texture for a 3D asset based on a provided 3x2 guide atlas.\nRules:\n- Output MUST be a single image and nothing else.\n- Preserve the 3x2 layout EXACTLY.\n- Each tile corresponds to an orthographic view: row0=[+X,-X,+Y], row1=[-Y,+Z,-Z].\n- No text, watermark, or annotations.\n- Output pure BaseColor/Albedo only: no shadows, AO, highlights, or baked lighting.\n";
const SYS_INPAINT: &str = "You repair seams using a mask.\nRules:\n- Input includes an image and a mask image.\n- Treat mask white as regions to modify, black as keep.\n- Output MUST be a single repaired image and nothing else.\n- Do not change style.\n";
const SYS_HEIGHT_UV: &str = "You generate a UV-space height/displacement map.\nRules:\n- Output MUST be a single image and nothing else.\n- Output MUST be grayscale height only (R=G=B).\n- Preserve UV alignment EXACTLY; no shift/scale.\n- No lighting, no AO, no shadows, no highlights.\n- No text, watermark, or annotations.\n";

#[derive(Default)]
pub struct NanoHexPlanarBakerV2Node;

impl NodeParameters for NanoHexPlanarBakerV2Node {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(crate::nodes::ai_texture::PARAM_AI_SYSTEM_PROMPT, "System Prompt (BaseColor Atlas)", "AI", ParameterValue::String(SYS_BASE_ATLAS.to_string()), ParameterUIType::Code),
            Parameter::new(PARAM_PROMPT_BASE, "Prompt (BaseColor)", "AI", ParameterValue::String(DEFAULT_PROMPT_BASE.to_string()), ParameterUIType::String),
            Parameter::new(PARAM_PROMPT_HEIGHT, "Prompt (Height UV)", "AI", ParameterValue::String(DEFAULT_PROMPT_HEIGHT.to_string()), ParameterUIType::String),
            Parameter::new(PARAM_TILE_RES, "Tile Resolution", "AI", ParameterValue::Int(512), ParameterUIType::IntSlider { min: 128, max: 2048 }),
            Parameter::new(PARAM_UV_RES, "UV Resolution", "AI", ParameterValue::Int(1024), ParameterUIType::IntSlider { min: 128, max: 4096 }),
            Parameter::new(PARAM_TIMEOUT_S, "Timeout (s)", "AI", ParameterValue::Int(0), ParameterUIType::IntSlider { min: 0, max: 3600 }),
            Parameter::new(PARAM_SEAM_PX, "Seam Width (px)", "AI", ParameterValue::Int(8), ParameterUIType::IntSlider { min: 1, max: 64 }),
            Parameter::new(PARAM_SEAM_FIX, "Seam Fix (BaseColor UV)", "AI", ParameterValue::Bool(true), ParameterUIType::Toggle),
            Parameter::new(PARAM_HEIGHT_STRENGTH, "Height Strength", "AI", ParameterValue::Float(2.0), ParameterUIType::FloatSlider { min: 0.1, max: 20.0 }),
            Parameter::new(PARAM_GENERATE, "Generate", "AI", ParameterValue::Int(0), ParameterUIType::BusyButton { busy_param: PARAM_BUSY.to_string(), busy_label: "Generating...".to_string(), busy_label_param: Some(PARAM_STAGE_LABEL.to_string()) }),
            Parameter::new(PARAM_STATUS, "Status", "Internal", ParameterValue::String(String::new()), ParameterUIType::String),
            Parameter::new(PARAM_STAGE_LABEL, "Stage Label", "Internal", ParameterValue::String(String::new()), ParameterUIType::String),
            Parameter::new(PARAM_THINKING, "Thinking", "Internal", ParameterValue::String(String::new()), ParameterUIType::Code),
            Parameter::new(PARAM_ERROR, "Error", "Internal", ParameterValue::String(String::new()), ParameterUIType::String),
            Parameter::new(PARAM_BUSY, "Busy (Internal)", "Debug", ParameterValue::Bool(false), ParameterUIType::Toggle),
            Parameter::new(PARAM_GUIDE_ATLAS, "Guide Atlas", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
            Parameter::new(PARAM_BASE_ATLAS, "BaseColor Atlas", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into(), "jpg".into(), "jpeg".into(), "webp".into()] }),
            Parameter::new(PARAM_BASE_UV, "BaseColor UV", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
            Parameter::new(PARAM_BASE_UV_FIXED, "BaseColor UV (Fixed)", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
            Parameter::new(PARAM_SEAM_MASK, "Seam Mask", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
            Parameter::new(PARAM_HEIGHT_UV, "Height UV", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
            Parameter::new(PARAM_NORMAL_UV, "Normal UV", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into()] }),
        ]
    }
}

impl NodeOp for NanoHexPlanarBakerV2Node {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let src = inputs.get(0).map(|g| g.materialize()).unwrap_or_else(Geometry::new);
        let mut out = src;
        let base = p_str(params, PARAM_BASE_UV_FIXED, "").trim().to_string().if_empty_then(|| p_str(params, PARAM_BASE_UV, ""));
        let height = p_str(params, PARAM_HEIGHT_UV, "");
        let normal = p_str(params, PARAM_NORMAL_UV, "");
        if !base.trim().is_empty() {
            out.set_detail_attribute(attrs::MAT_KIND, vec!["standard".to_string()]);
            out.set_detail_attribute(attrs::MAT_BASECOLOR_TEX, vec![base.trim().to_string()]);
            if !normal.trim().is_empty() { out.set_detail_attribute(attrs::MAT_NORMAL_TEX, vec![normal.trim().to_string()]); }
        }
        if !height.trim().is_empty() {
            out.set_detail_attribute(crate::nodes::ai_texture::ATTR_HEIGHTMAP_PATH, vec![height.trim().to_string()]);
        }
        Arc::new(out)
    }
}

impl NodeInteraction for NanoHexPlanarBakerV2Node {
    fn has_hud(&self) -> bool { true }
    fn draw_hud(&self, ui: &mut bevy_egui::egui::Ui, services: &dyn ServiceProvider, node_id: uuid::Uuid) {
        egui_extras::install_image_loaders(ui.ctx());
        ui.label(bevy_egui::egui::RichText::new("Nano HexPlanar Baker V2").small().weak());
        let g = services.get::<NodeGraphResource>().map(|r| &r.0);
        let Some(node) = g.and_then(|g| g.nodes.get(&node_id)) else { ui.label("(No node)"); return; };
        let get_s = |k: &str| node.parameters.iter().find(|p| p.name == k).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.trim().to_string()) } else { None }).filter(|s| !s.is_empty()).unwrap_or_default();
        let status = get_s(PARAM_STATUS);
        if !status.is_empty() { ui.label(bevy_egui::egui::RichText::new(status).small().weak()); }
        let imgs = [get_s(PARAM_BASE_UV_FIXED), get_s(PARAM_HEIGHT_UV), get_s(PARAM_NORMAL_UV)];
        for rel in imgs.into_iter().filter(|s| !s.is_empty()) {
            if let Some(uri) = assets_file_uri(rel.as_str()) {
                let w = 320.0f32.min(ui.available_width()).max(180.0);
                let resp = ui.add(bevy_egui::egui::Image::new(uri).fit_to_exact_size(bevy_egui::egui::vec2(w, w)));
                if resp.hovered() { ui.ctx().set_cursor_icon(bevy_egui::egui::CursorIcon::ZoomIn); }
            }
        }
    }
}

register_node!(NODE_NANO_HEXPLANAR_BAKER_V2, "AI Texture", NanoHexPlanarBakerV2Node, NanoHexPlanarBakerV2Node; inputs: &["Geometry"], outputs: &["Output"], style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts);

// ---------------- Jobs ----------------

#[derive(Default, Resource)]
pub struct NanoHexPlanarBakerV2Jobs { map: HashMap<NodeId, JobState> }

#[derive(Default)]
struct JobState { last_gen: i32, inflight: bool, rx: Option<Receiver<Msg>>, last_poll: Option<Instant>, thinking: String }

#[derive(Clone, Debug)]
struct Spec {
    node_id: NodeId,
    gen: i32,
    sys_base: String,
    prompt_base: String,
    prompt_height: String,
    tile_res: u32,
    uv_res: u32,
    seam_px: u32,
    seam_fix: bool,
    height_strength: f32,
    timeout_s: i32,
    in_geo: Geometry,
}

#[derive(Default, Clone)]
struct Out { guide_atlas: String, base_atlas: String, base_uv: String, base_uv_fixed: String, seam_mask: String, height_uv: String, normal_uv: String }

#[derive(Clone, Debug)]
enum Stage { CapturingGuides, GeneratingBaseColor, Baking, SeamMask, SeamFix, HeightUv, NormalUv, Done }
impl Stage { fn label(self) -> &'static str { match self { Self::CapturingGuides => "Generating... (Guides)", Self::GeneratingBaseColor => "Generating... (BaseColor)", Self::Baking => "Generating... (Baking)", Self::SeamMask => "Generating... (SeamMask)", Self::SeamFix => "Generating... (SeamFix)", Self::HeightUv => "Generating... (Height)", Self::NormalUv => "Generating... (Normal)", Self::Done => "Done" } } }

enum Msg { Progress(Stage, String), Thinking(String), Done(Out), Fail(String) }

fn spawn_job(spec: Spec) -> Receiver<Msg> {
    let (tx, rx) = unbounded();
    std::thread::spawn(move || {
        let tx2 = tx.clone();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(spec, tx2)));
        if let Err(p) = r {
            let msg = if let Some(s) = p.downcast_ref::<&str>() { (*s).to_string() } else if let Some(s) = p.downcast_ref::<String>() { s.clone() } else { "Unknown panic payload".to_string() };
            let _ = tx.send(Msg::Fail(format!("Job panicked: {msg} (set RUST_BACKTRACE=1 for stack trace).")));
        }
    });
    rx
}

fn run(spec: Spec, tx: Sender<Msg>) {
    let rt = GpuRuntime::get_blocking();
    let max_dim = rt.device().limits().max_texture_dimension_2d.max(1);
    let max_tile = (max_dim / 3).min(max_dim / 2).max(1);
    let tile_res = spec.tile_res.min(max_tile);
    let uv_res = spec.uv_res.min(max_dim);
    let _ = tx.send(Msg::Progress(Stage::CapturingGuides, if tile_res != spec.tile_res || uv_res != spec.uv_res { format!("{} (tile {}→{}, uv {}→{}, gpu limit {})", Stage::CapturingGuides.label(), spec.tile_res, tile_res, spec.uv_res, uv_res, max_dim) } else { Stage::CapturingGuides.label().to_string() }));

    // Reuse V1 GPU capture/bake helpers.
    let mesh = match super::nano_hexplanar_baker::upload_mesh(rt, &spec.in_geo) { Ok(m) => m, Err(e) => { let _ = tx.send(Msg::Fail(e)); return; } };
    let Some((center, ext)) = super::nano_hexplanar_baker::bbox_center_extent(&spec.in_geo) else { let _ = tx.send(Msg::Fail("Missing bbox".to_string())); return; };
    let cams = super::nano_hexplanar_baker::build_cams(center, ext);
    let guide_rgba = match super::nano_hexplanar_baker::render_guides(rt, &mesh, &cams, tile_res) { Ok(v) => v, Err(e) => { let _ = tx.send(Msg::Fail(format!("Guides capture failed: {e}"))); return; } };

    let root = std::env::current_dir().ok().unwrap_or_else(|| std::path::PathBuf::from("."));
    let out_dir = root.join("assets").join("textures").join("ai_bakes_v2").join(spec.node_id.to_string());
    if ensure_dir(&out_dir).is_err() { let _ = tx.send(Msg::Fail("Failed to create output dir".to_string())); return; }
    let gen = spec.gen.max(0);

    let guide_abs = out_dir.join(format!("guide_atlas_{gen}.png"));
    if save_png_abs(&guide_abs, &guide_rgba, tile_res * 3, tile_res * 2).is_err() { let _ = tx.send(Msg::Fail("Failed to save guide atlas".to_string())); return; }

    let api_key = load_gemini_key();
    if api_key.trim().is_empty() { let _ = tx.send(Msg::Fail("Gemini API key missing (GEMINI_API_KEY or settings/ai/providers.json)".to_string())); return; }
    let model = load_gemini_model_image();

    let guide_blob = Img { mime: "image/png".to_string(), bytes: std::fs::read(&guide_abs).unwrap_or_default() };
    let (atlas_w, atlas_h) = (tile_res * 3, tile_res * 2);

    let _ = tx.send(Msg::Progress(Stage::GeneratingBaseColor, Stage::GeneratingBaseColor.label().to_string()));
    let base_img = match gemini_image_stream(spec.timeout_s, &api_key, &model, spec.sys_base.as_str(), spec.prompt_base.as_str(), &[guide_blob.clone()], atlas_w, atlas_h, Some(&tx)) { Ok(b) => b, Err(e) => { let _ = tx.send(Msg::Fail(e)); return; } };
    let base_rgba = match decode_resize_rgba(&base_img.bytes, atlas_w, atlas_h) { Ok(v) => v, Err(e) => { let _ = tx.send(Msg::Fail(format!("BaseColor decode/resize: {e}"))); return; } };
    let base_abs = out_dir.join(format!("basecolor_atlas_{gen}.png"));
    if save_png_abs(&base_abs, &base_rgba, atlas_w, atlas_h).is_err() { let _ = tx.send(Msg::Fail("Failed to save base atlas".to_string())); return; }

    let mut out = Out::default();
    out.guide_atlas = file_rel_in_assets(guide_abs.strip_prefix(root.join("assets")).unwrap_or(&guide_abs).to_string_lossy().as_ref()).unwrap_or_default();
    out.base_atlas = file_rel_in_assets(base_abs.strip_prefix(root.join("assets")).unwrap_or(&base_abs).to_string_lossy().as_ref()).unwrap_or_default();

    let _ = tx.send(Msg::Progress(Stage::Baking, Stage::Baking.label().to_string()));
    let base_uv_rgba = match super::nano_hexplanar_baker::bake_uv(rt, &mesh, &cams, &base_rgba, true, &guide_rgba, tile_res, uv_res) { Ok(v) => v, Err(e) => { let _ = tx.send(Msg::Fail(format!("UV bake failed: {e}"))); return; } };
    let base_uv_abs = out_dir.join(format!("basecolor_uv_{gen}.png"));
    if save_png_abs(&base_uv_abs, &base_uv_rgba, uv_res, uv_res).is_err() { let _ = tx.send(Msg::Fail("Failed to save base uv".to_string())); return; }
    out.base_uv = file_rel_in_assets(base_uv_abs.strip_prefix(root.join("assets")).unwrap_or(&base_uv_abs).to_string_lossy().as_ref()).unwrap_or_default();

    let mut final_base_uv_abs = base_uv_abs.clone();
    if spec.seam_fix {
        let _ = tx.send(Msg::Progress(Stage::SeamMask, Stage::SeamMask.label().to_string()));
        let edge = match super::nano_hexplanar_baker::edge_mask_uv(rt, &mesh, uv_res) { Ok(v) => v, Err(e) => { let _ = tx.send(Msg::Fail(format!("SeamMask render failed: {e}"))); return; } };
        let mask = super::nano_hexplanar_baker::dilate_mask(edge, uv_res, uv_res, spec.seam_px);
        let mask_abs = out_dir.join(format!("seam_mask_{gen}.png"));
        if save_png_abs(&mask_abs, &mask, uv_res, uv_res).is_err() { let _ = tx.send(Msg::Fail("Failed to save seam mask".to_string())); return; }
        out.seam_mask = file_rel_in_assets(mask_abs.strip_prefix(root.join("assets")).unwrap_or(&mask_abs).to_string_lossy().as_ref()).unwrap_or_default();

        let _ = tx.send(Msg::Progress(Stage::SeamFix, Stage::SeamFix.label().to_string()));
        let base_uv_blob = Img { mime: "image/png".to_string(), bytes: std::fs::read(&final_base_uv_abs).unwrap_or_default() };
        let mask_blob = Img { mime: "image/png".to_string(), bytes: std::fs::read(&mask_abs).unwrap_or_default() };
        let img = match gemini_image_stream(spec.timeout_s, &api_key, &model, SYS_INPAINT, "Repair seams in the first image using the second image as mask (white=fix). Output the repaired image.", &[base_uv_blob, mask_blob], uv_res, uv_res, Some(&tx)) { Ok(b) => b, Err(e) => { let _ = tx.send(Msg::Fail(e)); return; } };
        let fixed_rgba = match decode_resize_rgba(&img.bytes, uv_res, uv_res) { Ok(v) => v, Err(e) => { let _ = tx.send(Msg::Fail(format!("SeamFix decode/resize: {e}"))); return; } };
        let fixed_abs = out_dir.join(format!("basecolor_uv_fixed_{gen}.png"));
        if save_png_abs(&fixed_abs, &fixed_rgba, uv_res, uv_res).is_err() { let _ = tx.send(Msg::Fail("Failed to save seam-fixed base uv".to_string())); return; }
        out.base_uv_fixed = file_rel_in_assets(fixed_abs.strip_prefix(root.join("assets")).unwrap_or(&fixed_abs).to_string_lossy().as_ref()).unwrap_or_default();
        final_base_uv_abs = fixed_abs;
    }

    let _ = tx.send(Msg::Progress(Stage::HeightUv, Stage::HeightUv.label().to_string()));
    let ref_blob = Img { mime: "image/png".to_string(), bytes: std::fs::read(&final_base_uv_abs).unwrap_or_default() };
    let h_img = match gemini_image_stream(spec.timeout_s, &api_key, &model, SYS_HEIGHT_UV, spec.prompt_height.as_str(), &[ref_blob], uv_res, uv_res, Some(&tx)) { Ok(b) => b, Err(e) => { let _ = tx.send(Msg::Fail(e)); return; } };
    let h_rgba = match decode_resize_rgba(&h_img.bytes, uv_res, uv_res) { Ok(v) => v, Err(e) => { let _ = tx.send(Msg::Fail(format!("Height decode/resize: {e}"))); return; } };
    let height_abs = out_dir.join(format!("height_uv_{gen}.png"));
    if save_png_abs(&height_abs, &h_rgba, uv_res, uv_res).is_err() { let _ = tx.send(Msg::Fail("Failed to save height uv".to_string())); return; }
    out.height_uv = file_rel_in_assets(height_abs.strip_prefix(root.join("assets")).unwrap_or(&height_abs).to_string_lossy().as_ref()).unwrap_or_default();

    let _ = tx.send(Msg::Progress(Stage::NormalUv, Stage::NormalUv.label().to_string()));
    let n_rgba = height_to_normal(&h_rgba, uv_res, uv_res, spec.height_strength);
    let normal_abs = out_dir.join(format!("normal_uv_{gen}.png"));
    if save_png_abs(&normal_abs, &n_rgba, uv_res, uv_res).is_err() { let _ = tx.send(Msg::Fail("Failed to save normal uv".to_string())); return; }
    out.normal_uv = file_rel_in_assets(normal_abs.strip_prefix(root.join("assets")).unwrap_or(&normal_abs).to_string_lossy().as_ref()).unwrap_or_default();

    let _ = tx.send(Msg::Progress(Stage::Done, Stage::Done.label().to_string()));
    let _ = tx.send(Msg::Done(out));
}

pub(crate) fn nano_hexplanar_baker_v2_jobs_system(
    mut node_graph_res: ResMut<NodeGraphResource>,
    mut jobs: ResMut<NanoHexPlanarBakerV2Jobs>,
    mut graph_changed: MessageWriter<'_, crate::GraphChanged>,
) {
    let g = &mut node_graph_res.0;
    let now = Instant::now();
    let nodes: Vec<(NodeId, Vec<Parameter>)> = g.nodes.iter().filter_map(|(id, n)| matches!(&n.node_type, NodeType::Generic(s) if s == NODE_NANO_HEXPLANAR_BAKER_V2).then_some((*id, n.parameters.clone()))).collect();
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

            let in_port = PortId::from(port_key::in0().as_str());
            let Some((src_n, src_p)) = first_src(g, node_id, &in_port) else {
                changed |= set_str(&mut params, PARAM_ERROR, "No input geometry connected.".to_string());
                changed |= set_str(&mut params, PARAM_STATUS, "Failed: No input geometry.".to_string());
                changed |= set_str(&mut params, PARAM_STAGE_LABEL, "Generate".to_string());
                changed |= set_bool(&mut params, PARAM_BUSY, false);
                st.inflight = false;
                if changed { if let Some(n) = g.nodes.get_mut(&node_id) { n.parameters = params.clone(); g.bump_param_revision(); graph_changed.write(crate::GraphChanged); } }
                continue;
            };
            let Some(in_geo) = cached_output_geo(g, src_n, &src_p).map(|x| (*x).clone()) else {
                changed |= set_str(&mut params, PARAM_ERROR, "Input geometry not cooked yet.".to_string());
                changed |= set_str(&mut params, PARAM_STATUS, "Waiting for cook...".to_string());
                changed |= set_str(&mut params, PARAM_STAGE_LABEL, "Waiting...".to_string());
                changed |= set_bool(&mut params, PARAM_BUSY, false);
                st.inflight = false;
                if changed { if let Some(n) = g.nodes.get_mut(&node_id) { n.parameters = params.clone(); g.bump_param_revision(); graph_changed.write(crate::GraphChanged); } }
                continue;
            };
            if !has_uv(&in_geo) {
                changed |= set_str(&mut params, PARAM_ERROR, "Missing @uv (vertex or point attribute).".to_string());
                changed |= set_str(&mut params, PARAM_STATUS, "Failed: Missing UV.".to_string());
                changed |= set_str(&mut params, PARAM_STAGE_LABEL, "Generate".to_string());
                changed |= set_bool(&mut params, PARAM_BUSY, false);
                st.inflight = false;
                if changed { if let Some(n) = g.nodes.get_mut(&node_id) { n.parameters = params.clone(); g.bump_param_revision(); graph_changed.write(crate::GraphChanged); } }
                continue;
            }

            let spec = Spec {
                node_id,
                gen,
                sys_base: p_str(&params, crate::nodes::ai_texture::PARAM_AI_SYSTEM_PROMPT, SYS_BASE_ATLAS),
                prompt_base: p_str(&params, PARAM_PROMPT_BASE, DEFAULT_PROMPT_BASE),
                prompt_height: p_str(&params, PARAM_PROMPT_HEIGHT, DEFAULT_PROMPT_HEIGHT),
                tile_res: p_u32(&params, PARAM_TILE_RES, 512).max(128).min(2048),
                uv_res: p_u32(&params, PARAM_UV_RES, 1024).max(128).min(4096),
                seam_px: p_u32(&params, PARAM_SEAM_PX, 8).max(1).min(64),
                seam_fix: p_bool(&params, PARAM_SEAM_FIX, true),
                height_strength: p_f32(&params, PARAM_HEIGHT_STRENGTH, 2.0).max(0.01),
                timeout_s: p_i32(&params, PARAM_TIMEOUT_S, 0),
                in_geo,
            };
            st.rx = Some(spawn_job(spec));
            if changed { if let Some(n) = g.nodes.get_mut(&node_id) { n.parameters = params.clone(); g.bump_param_revision(); graph_changed.write(crate::GraphChanged); } }
        }

        if st.inflight {
            let mut done: Option<Out> = None;
            let mut fail: Option<String> = None;
            let mut prog: Option<(Stage, String)> = None;
            if let Some(rx) = &st.rx {
                while let Ok(m) = rx.try_recv() {
                    match m {
                        Msg::Progress(s, t) => prog = Some((s, t)),
                        Msg::Thinking(t) => { st.thinking.push_str(&t); st.thinking.push('\n'); },
                        Msg::Done(o) => done = Some(o),
                        Msg::Fail(e) => fail = Some(e),
                    }
                }
            }
            let mut changed = false;
            if let Some((_s, t)) = prog {
                changed |= set_str(&mut params, PARAM_STAGE_LABEL, t.clone());
                changed |= set_str(&mut params, PARAM_STATUS, t);
                st.last_poll = Some(now);
            }
            if !st.thinking.is_empty() { changed |= set_str(&mut params, PARAM_THINKING, st.thinking.clone()); }
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
                changed |= set_str(&mut params, PARAM_BASE_UV, o.base_uv);
                changed |= set_str(&mut params, PARAM_BASE_UV_FIXED, o.base_uv_fixed);
                changed |= set_str(&mut params, PARAM_SEAM_MASK, o.seam_mask);
                changed |= set_str(&mut params, PARAM_HEIGHT_UV, o.height_uv);
                changed |= set_str(&mut params, PARAM_NORMAL_UV, o.normal_uv);
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

// ---------------- Helpers ----------------

trait IfEmptyThen { fn if_empty_then(self, f: impl FnOnce() -> String) -> String; }
impl IfEmptyThen for String { fn if_empty_then(self, f: impl FnOnce() -> String) -> String { if self.trim().is_empty() { f() } else { self } } }

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

#[inline]
fn ensure_dir(p: &std::path::Path) -> std::io::Result<()> { std::fs::create_dir_all(p) }

#[inline]
fn save_png_abs(abs_path: &std::path::Path, rgba: &[u8], w: u32, h: u32) -> Result<(), String> {
    let img: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(w, h, rgba.to_vec()).ok_or("ImageBuffer")?;
    img.save(abs_path).map_err(|e| e.to_string())
}

#[inline]
fn decode_resize_rgba(bytes: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    image::load_from_memory(bytes).map(|i| i.to_rgba8()).map_err(|e| e.to_string()).and_then(|img| Ok(image::imageops::resize(&img, w, h, image::imageops::FilterType::Lanczos3).into_raw()))
}

fn height_to_normal(h_rgba: &[u8], w: u32, h: u32, strength: f32) -> Vec<u8> {
    let w = w as i32;
    let h = h as i32;
    let mut out = vec![0u8; (w * h * 4) as usize];
    let idx = |x: i32, y: i32| -> usize { ((y as usize * w as usize + x as usize) * 4) };
    let get_h = |x: i32, y: i32| -> f32 {
        let x = x.clamp(0, w - 1);
        let y = y.clamp(0, h - 1);
        let i = idx(x, y);
        (h_rgba[i] as f32) / 255.0
    };
    let k = strength.max(1e-3);
    for y in 0..h {
        for x in 0..w {
            let dx = get_h(x + 1, y) - get_h(x - 1, y);
            let dy = get_h(x, y + 1) - get_h(x, y - 1);
            let mut n = Vec3::new(-dx * k, -dy * k, 1.0);
            let l = n.length().max(1e-6);
            n /= l;
            let i = idx(x, y);
            out[i] = ((n.x * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            out[i + 1] = ((n.y * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            out[i + 2] = ((n.z * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            out[i + 3] = 255;
        }
    }
    out
}

// ---- Gemini (image) ----

static TOKIO_RT: OnceCell<tokio::runtime::Runtime> = OnceCell::new();
#[inline] fn tokio_rt() -> &'static tokio::runtime::Runtime { TOKIO_RT.get_or_init(|| tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime")) }

#[derive(Clone, Debug)]
struct Img { mime: String, bytes: Vec<u8> }

#[derive(Clone, Debug)]
struct ImgOut { mime: String, bytes: Vec<u8> }

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

fn image_size_for_px(px: u32) -> &'static str { if px <= 1024 { "1K" } else if px <= 2048 { "2K" } else { "4K" } }
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

fn gemini_image_stream(timeout_s: i32, api_key: &str, model: &str, system: &str, prompt: &str, images: &[Img], w: u32, h: u32, tx: Option<&Sender<Msg>>) -> Result<ImgOut, String> {
    let mut parts: Vec<Value> = vec![json!({ "text": format!("{system}\n\n{prompt}") })];
    for img in images {
        parts.push(json!({ "inlineData": { "mimeType": img.mime, "data": base64::engine::general_purpose::STANDARD.encode(&img.bytes) } }));
    }
    let body = json!({
        "contents": [{ "role": "user", "parts": parts }],
        "generationConfig": { "responseModalities": ["TEXT", "IMAGE"], "imageConfig": { "aspectRatio": aspect_ratio_for_wh(w.max(1), h.max(1)), "imageSize": image_size_for_px(w.max(h).max(1)) } }
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
        if resp.status() == reqwest::StatusCode::NOT_FOUND { return gemini_non_stream(&client, &non_stream_url, &body, tx).await; }
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("HTTP {}: {}", status.as_u16(), text));
        }

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        let mut out_img: Option<ImgOut> = None;
        let idle = if timeout_s > 0 { Some(Duration::from_secs(45)) } else { None };
        let mut last_io = tokio::time::Instant::now();

        loop {
            let item: Option<Result<_, reqwest::Error>> = if let Some(idle) = idle {
                tokio::select! { _ = tokio::time::sleep_until(last_io + idle) => { return Err("idle timeout".to_string()); } item = stream.next() => item }
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
                        if !text.is_empty() { if let Some(tx) = tx { let _ = tx.send(Msg::Thinking(text.to_string())); } }
                    }
                    let id = p.get("inlineData").or_else(|| p.get("inline_data"));
                    if let Some(x) = id {
                        let mime = x.get("mimeType").or_else(|| x.get("mime_type")).and_then(|m| m.as_str()).unwrap_or("").to_string();
                        let data = x.get("data").and_then(|d| d.as_str()).unwrap_or("");
                        if mime.starts_with("image/") && !data.is_empty() && out_img.is_none() {
                            let bytes = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).map_err(|e| e.to_string())?;
                            out_img = Some(ImgOut { mime, bytes });
                        }
                    }
                }
            }
        }
        out_img.ok_or_else(|| "Gemini: no image inlineData returned".to_string())
    })
}

async fn gemini_non_stream(client: &reqwest::Client, url: &str, body: &Value, tx: Option<&Sender<Msg>>) -> Result<ImgOut, String> {
    let resp = client.post(url).json(body).send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let txt = resp.text().await.unwrap_or_default();
    if !status.is_success() { return Err(format!("HTTP {}: {}", status.as_u16(), txt)); }
    let v: Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
    let parts = v.get("candidates").and_then(|c| c.get(0)).and_then(|c| c.get("content")).and_then(|c| c.get("parts")).and_then(|p| p.as_array()).cloned().unwrap_or_default();
    for p in &parts {
        if let Some(text) = p.get("text").and_then(|v| v.as_str()) {
            if !text.is_empty() { if let Some(tx) = tx { let _ = tx.send(Msg::Thinking(text.to_string())); } }
        }
    }
    for p in parts {
        let id = p.get("inlineData").or_else(|| p.get("inline_data"));
        if let Some(x) = id {
            let mime = x.get("mimeType").or_else(|| x.get("mime_type")).and_then(|m| m.as_str()).unwrap_or("").to_string();
            let data = x.get("data").and_then(|d| d.as_str()).unwrap_or("");
            if mime.starts_with("image/") && !data.is_empty() {
                let bytes = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).map_err(|e| e.to_string())?;
                return Ok(ImgOut { mime, bytes });
            }
        }
    }
    Err("Gemini: no image inlineData returned".to_string())
}

// ---- graph helpers (copied minimal from V1 patterns) ----

fn p_str(params: &[Parameter], name: &str, d: &str) -> String { params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None }).unwrap_or_else(|| d.to_string()) }
fn p_i32(params: &[Parameter], name: &str, d: i32) -> i32 { params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::Int(v) = p.value { Some(v) } else { None }).unwrap_or(d) }
fn p_u32(params: &[Parameter], name: &str, d: u32) -> u32 { p_i32(params, name, d as i32).max(0) as u32 }
fn p_f32(params: &[Parameter], name: &str, d: f32) -> f32 { params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::Float(v) = p.value { Some(v) } else { None }).unwrap_or(d) }
fn p_bool(params: &[Parameter], name: &str, d: bool) -> bool { params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::Bool(v) = p.value { Some(v) } else { None }).unwrap_or(d) }

fn set_str(params: &mut [Parameter], name: &str, v: String) -> bool { for p in params { if p.name == name { if let ParameterValue::String(cur) = &mut p.value { if *cur == v { return false; } *cur = v; return true; } p.value = ParameterValue::String(v); return true; } } false }
fn set_bool(params: &mut [Parameter], name: &str, v: bool) -> bool { for p in params { if p.name == name { if let ParameterValue::Bool(cur) = &mut p.value { if *cur == v { return false; } *cur = v; return true; } p.value = ParameterValue::Bool(v); return true; } } false }

fn has_uv(geo: &Geometry) -> bool {
    geo.get_vertex_attribute(attrs::UV).is_some() || geo.get_point_attribute(attrs::UV).is_some()
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

fn cached_output_geo(graph: &crate::nodes::structs::NodeGraph, nid: NodeId, port: &PortId) -> Option<Arc<Geometry>> {
    let is_cda = graph.nodes.get(&nid).map(|n| matches!(n.node_type, NodeType::CDA(_))).unwrap_or(false);
    if is_cda { graph.port_geometry_cache.get(&(nid, port.clone())).cloned() } else { graph.geometry_cache.get(&nid).cloned() }
}

fn file_rel_in_assets(rel: &str) -> Option<String> {
    let rel = rel.replace('\\', "/");
    if rel.starts_with("assets/") { Some(rel.trim_start_matches("assets/").to_string()) } else { Some(rel) }
}

