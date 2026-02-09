//! NanoHeightmap node: Gemini image generation (heightmap-only).

use bevy::prelude::*;
use base64::Engine;
use crossbeam_channel::{unbounded, Receiver};
use reqwest::blocking::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::nodes::structs::NodeId;
use crate::register_node;

pub const NODE_NANO_HEIGHTMAP: &str = "Nano Heightmap";
pub const PARAM_PROMPT: &str = "prompt";
pub const PARAM_W: &str = "width";
pub const PARAM_H: &str = "height";
pub const PARAM_SEED: &str = "seed";
pub const PARAM_TIMEOUT_S: &str = "timeout_s";
pub const PARAM_GENERATE: &str = "generate";
pub const PARAM_IMAGE_PATH: &str = "image_path";
pub const PARAM_STATUS: &str = "status";
pub const PARAM_ERROR: &str = "error";
pub const PARAM_BUSY: &str = "busy";

pub const ATTR_HEIGHTMAP_PATH: &str = "__ai_heightmap_path";

const DEFAULT_SYSTEM_PROMPT: &str = "You are generating a heightmap texture for a DCC node.\n- Output MUST be a single image and nothing else.\n- Camera MUST be orthographic TOP-DOWN view (no perspective).\n- The image MUST represent height only: grayscale (R=G=B), no color.\n- No text, no watermark, no annotations.\n- Black=low, White=high.\n- Avoid hard edges unless asked.\n- Keep edges tile-friendly unless asked otherwise.";

#[derive(Default)]
pub struct NanoHeightmapNode;

impl NodeParameters for NanoHeightmapNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                crate::nodes::ai_texture::PARAM_AI_SYSTEM_PROMPT,
                "System Prompt",
                "AI",
                ParameterValue::String(DEFAULT_SYSTEM_PROMPT.to_string()),
                ParameterUIType::Code,
            ),
            Parameter::new(
                PARAM_PROMPT,
                "Prompt",
                "AI",
                ParameterValue::String("A smooth mountainous terrain with ridges.".to_string()),
                ParameterUIType::String,
            ),
            Parameter::new(
                PARAM_W,
                "Width",
                "AI",
                ParameterValue::Int(512),
                ParameterUIType::IntSlider { min: 64, max: 2048 },
            ),
            Parameter::new(
                PARAM_H,
                "Height",
                "AI",
                ParameterValue::Int(512),
                ParameterUIType::IntSlider { min: 64, max: 2048 },
            ),
            Parameter::new(
                PARAM_SEED,
                "Seed",
                "AI",
                ParameterValue::Int(0),
                ParameterUIType::IntSlider { min: 0, max: 2_147_483_647 },
            ),
            Parameter::new(
                PARAM_TIMEOUT_S,
                "Timeout (s)",
                "AI",
                ParameterValue::Int(0),
                ParameterUIType::IntSlider { min: 0, max: 600 },
            ),
            Parameter::new(
                PARAM_GENERATE,
                "Generate",
                "AI",
                ParameterValue::Int(0),
                ParameterUIType::BusyButton {
                    busy_param: PARAM_BUSY.to_string(),
                    busy_label: "Generating...".to_string(),
                    busy_label_param: None,
                },
            ),
            Parameter::new(
                PARAM_IMAGE_PATH,
                "Image Path (Internal)",
                "Internal",
                ParameterValue::String(String::new()),
                ParameterUIType::FilePath {
                    filters: vec!["png".to_string(), "jpg".to_string(), "jpeg".to_string(), "webp".to_string()],
                },
            ),
            Parameter::new(
                PARAM_STATUS,
                "Status",
                "Internal",
                ParameterValue::String(String::new()),
                ParameterUIType::String,
            ),
            Parameter::new(
                PARAM_ERROR,
                "Error",
                "Internal",
                ParameterValue::String(String::new()),
                ParameterUIType::String,
            ),
            Parameter::new(
                PARAM_BUSY,
                "Busy (Internal)",
                "Debug",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

impl NodeOp for NanoHeightmapNode {
    fn compute(&self, params: &[Parameter], _inputs: &[std::sync::Arc<dyn crate::libs::geometry::geo_ref::GeometryRef>]) -> std::sync::Arc<Geometry> {
        let mut out = Geometry::new();
        let image_path = params
            .iter()
            .find(|p| p.name == PARAM_IMAGE_PATH)
            .and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.as_str()) } else { None })
            .unwrap_or("")
            .trim()
            .to_string();
        if !image_path.is_empty() {
            out.set_detail_attribute(ATTR_HEIGHTMAP_PATH, vec![image_path]);
        }
        std::sync::Arc::new(out)
    }
}

register_node!(
    NODE_NANO_HEIGHTMAP,
    "AI Texture",
    NanoHeightmapNode;
    inputs: &[],
    outputs: &["Heightmap"],
    style: crate::cunning_core::registries::node_registry::InputStyle::Single
);

#[derive(Clone, Debug)]
struct NanoHeightmapSpec {
    system_prompt: String,
    prompt: String,
    w: i32,
    h: i32,
    seed: i32,
    timeout_s: i32,
}

#[derive(Clone, Debug)]
struct ImageBlob {
    mime: String,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
struct JobResult {
    node_id: NodeId,
    rel_path: Option<String>,
    err: Option<String>,
    elapsed_ms: u128,
}

#[derive(Resource, Default)]
pub(crate) struct NanoHeightmapJobs {
    map: HashMap<NodeId, JobState>,
}

#[derive(Default)]
struct JobState {
    last_gen: i32,
    inflight: bool,
    rx: Option<Receiver<JobResult>>,
    last_poll: Option<Instant>,
}

fn p_i32(params: &[Parameter], name: &str, d: i32) -> i32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| if let ParameterValue::Int(v) = &p.value { Some(*v) } else { None })
        .unwrap_or(d)
}

fn p_str(params: &[Parameter], name: &str, d: &str) -> String {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None })
        .unwrap_or_else(|| d.to_string())
}

fn set_str(params: &mut [Parameter], name: &str, v: String) -> bool {
    if let Some(p) = params.iter_mut().find(|p| p.name == name) {
        if let ParameterValue::String(cur) = &mut p.value {
            if *cur != v {
                *cur = v;
                return true;
            }
        }
    }
    false
}

fn set_bool(params: &mut [Parameter], name: &str, v: bool) -> bool {
    if let Some(p) = params.iter_mut().find(|p| p.name == name) {
        if let ParameterValue::Bool(cur) = &mut p.value {
            if *cur != v {
                *cur = v;
                return true;
            }
        } else {
            p.value = ParameterValue::Bool(v);
            return true;
        }
    }
    false
}

fn load_gemini_key() -> String {
    if let Ok(k) = std::env::var("GEMINI_API_KEY") {
        if !k.trim().is_empty() {
            return k;
        }
    }
    let cwd = std::env::current_dir().ok();
    let path = cwd
        .as_ref()
        .map(|p| p.join("settings/ai/providers.json"));
    let raw = path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    v.get("gemini")
        .and_then(|g| g.get("api_key"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn load_gemini_model_image() -> String {
    if let Ok(m) = std::env::var("CUNNING_GEMINI_MODEL_IMAGE") {
        if !m.trim().is_empty() {
            return m;
        }
    }
    if let Ok(m) = std::env::var("CUNNING_GEMINI_IMAGE_MODEL") {
        if !m.trim().is_empty() {
            return m;
        }
    }
    let cwd = std::env::current_dir().ok();
    let path = cwd.as_ref().map(|p| p.join("settings/ai/providers.json"));
    let raw = path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let s = v
        .get("gemini")
        .and_then(|g| g.get("model_image"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if !s.is_empty() { s } else { "gemini-3-pro-image-preview".to_string() }
}

fn build_prompt(spec: &NanoHeightmapSpec) -> String {
    let sys = spec.system_prompt.trim();
    format!(
        "System:\n{sys}\n\nUser:\n{desc}\n\nDesired size: {w}x{h}\nSeed: {seed}\n",
        sys = if sys.is_empty() { DEFAULT_SYSTEM_PROMPT } else { sys },
        w = spec.w.max(64),
        h = spec.h.max(64),
        desc = spec.prompt.trim(),
        seed = spec.seed
    )
}

fn default_image_model() -> String {
    load_gemini_model_image()
}

fn image_size_for_px(px: i32) -> &'static str {
    if px <= 1024 { "1K" } else if px <= 2048 { "2K" } else { "4K" }
}

fn aspect_ratio_for_wh(w: i32, h: i32) -> &'static str {
    if w <= 0 || h <= 0 { return "1:1"; }
    let a = w as f32 / h as f32;
    let (best, _d) = [
        ("1:1", 1.0f32),
        ("2:3", 2.0/3.0),
        ("3:2", 3.0/2.0),
        ("3:4", 3.0/4.0),
        ("4:3", 4.0/3.0),
        ("4:5", 4.0/5.0),
        ("5:4", 5.0/4.0),
        ("9:16", 9.0/16.0),
        ("16:9", 16.0/9.0),
        ("21:9", 21.0/9.0),
    ]
    .into_iter()
    .map(|(k, v)| (k, (a - v).abs()))
    .min_by(|a, b| a.1.total_cmp(&b.1))
    .unwrap_or(("1:1", 0.0));
    best
}

fn ext_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/jpg" => "jpg",
        "image/webp" => "webp",
        _ => "png",
    }
}

fn file_rel_in_assets(subdir: &str, name: &str) -> (std::path::PathBuf, String) {
    let assets = std::env::current_dir().ok().unwrap_or_default().join("assets");
    let rel = format!("{}/{}", subdir.trim_matches('/'), name);
    (assets.join(&rel), rel.replace('\\', "/"))
}

fn request_nano_heightmap_image(spec: &NanoHeightmapSpec) -> Result<ImageBlob, String> {
    let api_key = load_gemini_key();
    if api_key.trim().is_empty() {
        return Err("Missing Gemini API key (set GEMINI_API_KEY or settings/ai/providers.json gemini.api_key).".to_string());
    }
    let model = default_image_model();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
        model
    );
    let ar = aspect_ratio_for_wh(spec.w.max(1), spec.h.max(1));
    let sz = image_size_for_px(spec.w.max(spec.h).max(1));
    let body = serde_json::json!({
        "contents": [{
            "role": "user",
            "parts": [{"text": build_prompt(spec)}]
        }],
        "generationConfig": {
            "responseModalities": ["TEXT", "IMAGE"],
            "imageConfig": {
                "aspectRatio": ar,
                "imageSize": sz
            }
        }
    });
    let mut b = Client::builder().connect_timeout(Duration::from_secs(10));
    if spec.timeout_s > 0 { b = b.timeout(Duration::from_secs(spec.timeout_s as u64)); }
    let client = b.build().map_err(|e| e.to_string())?;
    let resp = client
        .post(&url)
        .header("x-goog-api-key", api_key)
        .json(&body)
        .send()
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let txt = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(format!("HTTP {} (model={}): {}", status.as_u16(), model, txt));
    }
    let v: Value = serde_json::from_str(&txt).map_err(|e| format!("JSON: {}", e))?;
    let parts = v
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    for p in parts {
        let inline = p
            .get("inlineData")
            .or_else(|| p.get("inline_data"))
            .and_then(|x| x.as_object());
        if let Some(inline) = inline {
            let mime = inline
                .get("mimeType")
                .or_else(|| inline.get("mime_type"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if !mime.starts_with("image/") {
                continue;
            }
            let data = inline.get("data").and_then(|x| x.as_str()).unwrap_or("");
            if data.trim().is_empty() {
                continue;
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| format!("base64: {}", e));
            return bytes.map(|bytes| ImageBlob { mime: mime.to_string(), bytes });
        }
    }
    Err("No image inlineData returned by Gemini.".to_string())
}

fn spawn_job(node_id: NodeId, gen: i32, spec: NanoHeightmapSpec) -> Receiver<JobResult> {
    let (tx, rx) = unbounded::<JobResult>();
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let out = request_nano_heightmap_image(&spec)
            .and_then(|img| {
                let ext = ext_for_mime(img.mime.trim());
                let file_name = format!("nano_heightmap_{}_{}.{}", node_id, gen.max(0), ext);
                let (abs, rel) = file_rel_in_assets("textures/ai_heightmaps", &file_name);
                if let Some(p) = abs.parent() {
                    let _ = std::fs::create_dir_all(p);
                }
                std::fs::write(&abs, &img.bytes).map_err(|e| e.to_string())?;
                Ok(rel)
            });
        let (rel_path, err) = match out {
            Ok(p) => (Some(p), None),
            Err(e) => (None, Some(e)),
        };
        let _ = tx.send(JobResult {
            node_id,
            rel_path,
            err,
            elapsed_ms: t0.elapsed().as_millis(),
        });
    });
    rx
}

pub(crate) fn nano_heightmap_jobs_system(
    mut jobs: ResMut<NanoHeightmapJobs>,
    mut graph_res: ResMut<crate::nodes::NodeGraphResource>,
    mut graph_changed: MessageWriter<'_, crate::GraphChanged>,
) {
    let g = &mut graph_res.0;
    let now = Instant::now();

    // Start jobs.
    let mut started: Vec<(NodeId, i32)> = Vec::new();
    for (id, n) in g.nodes.iter() {
        let is_target = matches!(&n.node_type, crate::nodes::NodeType::Generic(s) if s == NODE_NANO_HEIGHTMAP);
        if !is_target {
            continue;
        }
        let gen = p_i32(&n.parameters, PARAM_GENERATE, 0);
        let st = jobs.map.entry(*id).or_default();
        if gen <= st.last_gen || st.inflight {
            continue;
        }
        let spec = NanoHeightmapSpec {
            system_prompt: p_str(&n.parameters, crate::nodes::ai_texture::PARAM_AI_SYSTEM_PROMPT, DEFAULT_SYSTEM_PROMPT),
            prompt: p_str(&n.parameters, PARAM_PROMPT, ""),
            w: p_i32(&n.parameters, PARAM_W, 512),
            h: p_i32(&n.parameters, PARAM_H, 512),
            seed: p_i32(&n.parameters, PARAM_SEED, 0),
            timeout_s: p_i32(&n.parameters, PARAM_TIMEOUT_S, 0),
        };
        st.last_gen = gen;
        st.inflight = true;
        st.last_poll = Some(now);
        st.rx = Some(spawn_job(*id, gen, spec));
        started.push((*id, gen));
    }
    for (id, _gen) in started {
        if let Some(n) = g.nodes.get_mut(&id) {
            let mut changed = false;
            changed |= set_bool(&mut n.parameters, PARAM_BUSY, true);
            let gen = p_i32(&n.parameters, PARAM_GENERATE, 0);
            changed |= set_str(&mut n.parameters, PARAM_STATUS, format!("Generating... ({})", gen));
            changed |= set_str(&mut n.parameters, PARAM_ERROR, String::new());
            if changed {
                g.bump_param_revision();
                graph_changed.write_default();
            }
        }
    }

    // Poll completions.
    let mut done: Vec<JobResult> = Vec::new();
    for st in jobs.map.values_mut() {
        if !st.inflight {
            continue;
        }
        let Some(rx) = st.rx.as_ref() else { continue; };
        while let Ok(r) = rx.try_recv() {
            done.push(r);
        }
    }

    for r in done {
        let Some(st) = jobs.map.get_mut(&r.node_id) else { continue; };
        st.inflight = false;
        st.rx = None;
        st.last_poll = None;
        if let Some(n) = g.nodes.get_mut(&r.node_id) {
            let mut changed = false;
            changed |= set_bool(&mut n.parameters, PARAM_BUSY, false);
            if let Some(p) = r.rel_path.clone() {
                changed |= set_str(&mut n.parameters, PARAM_IMAGE_PATH, p);
                changed |= set_str(&mut n.parameters, PARAM_STATUS, format!("OK ({}ms)", r.elapsed_ms));
                changed |= set_str(&mut n.parameters, PARAM_ERROR, String::new());
            } else if let Some(e) = r.err.clone() {
                changed |= set_str(&mut n.parameters, PARAM_STATUS, format!("Failed ({}ms)", r.elapsed_ms));
                changed |= set_str(&mut n.parameters, PARAM_ERROR, e);
            }
            if changed {
                g.bump_param_revision();
                g.mark_dirty(r.node_id);
                graph_changed.write_default();
            }
        }
    }
}

