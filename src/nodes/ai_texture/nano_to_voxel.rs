//! NanoToVoxel node: Gemini depth atlas -> point cloud -> voxel chunks (GPU preview).

use bevy::prelude::*;
use crossbeam_channel::{unbounded, Receiver};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::Geometry;
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::nodes::structs::{NodeGraphResource, NodeId, NodeType, PortId};
use crate::nodes::port_key;
use crate::register_node;

use crate::nodes::voxel::voxel_edit::ATTR_VOXEL_SIZE_DETAIL;
use cunning_kernel::algorithms::algorithms_editor::voxel as vox;

use super::nano_to_3d_common::*;

pub const NODE_NANO_TO_VOXEL: &str = "Nano To Voxel";

const PARAM_SYS: &str = crate::nodes::ai_texture::PARAM_AI_SYSTEM_PROMPT;
const PARAM_PROMPT: &str = "prompt";
const PARAM_REF_IMAGE: &str = "reference_image";
const PARAM_TILE_RES: &str = "tile_res";
const PARAM_WORLD_SIZE: &str = "world_size";
const PARAM_SAMPLE_STEP: &str = "sample_step";
const PARAM_DEPTH_MIN: &str = "depth_min";
const PARAM_VOXEL_SIZE: &str = "voxel_size";
const PARAM_PI: &str = "palette_index";
const PARAM_TIMEOUT_S: &str = "timeout_s";
const PARAM_GENERATE: &str = "generate";
const PARAM_DEPTH_ATLAS: &str = "depth_atlas";
const PARAM_VOXEL_NODE: &str = "voxel_node";
const PARAM_STATUS: &str = "status";
const PARAM_ERROR: &str = "error";
const PARAM_BUSY: &str = "busy";

const SYS_DEPTH_ATLAS: &str =
    "You are a 3D Consistency Refiner.\n\
Input: A rough Normal/Depth Atlas (Front, Back, Left, Right, Top, Bottom) derived from a coarse 3D model.\n\
Task: Generate a high-quality, consistent 3x2 Depth Atlas for all 6 views.\n\
Rules:\n\
- Respect the coarse shape in the input.\n\
- Add details and refine surface.\n\
- Ensure strict 3D consistency.\n\
- Output 3x2 Depth Atlas.\n\
- Row 1: Front, Back, Left.\n\
- Row 2: Right, Top, Bottom.\n\
- Depth: White=Near, Black=Far.\n\
- Background Black.\n";

#[derive(Default)]
pub struct NanoToVoxelNode;

impl NodeParameters for NanoToVoxelNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(PARAM_SYS, "System Prompt (Depth Atlas)", "AI", ParameterValue::String(SYS_DEPTH_ATLAS.to_string()), ParameterUIType::Code),
            Parameter::new(PARAM_PROMPT, "Prompt", "AI", ParameterValue::String("A stylized banana figurine, clean silhouette.".to_string()), ParameterUIType::String),
            Parameter::new(PARAM_REF_IMAGE, "Reference Image", "AI", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into(), "jpg".into(), "jpeg".into(), "webp".into()] }),
            Parameter::new(PARAM_TILE_RES, "Tile Resolution", "AI", ParameterValue::Int(256), ParameterUIType::IntSlider { min: 64, max: 1024 }),
            Parameter::new(PARAM_WORLD_SIZE, "World Size", "Voxel", ParameterValue::Float(1.0), ParameterUIType::FloatSlider { min: 0.01, max: 100.0 }),
            Parameter::new(PARAM_SAMPLE_STEP, "Sample Step", "Voxel", ParameterValue::Int(4), ParameterUIType::IntSlider { min: 1, max: 16 }),
            Parameter::new(PARAM_DEPTH_MIN, "Depth Min", "Voxel", ParameterValue::Float(0.05), ParameterUIType::FloatSlider { min: 0.0, max: 0.5 }),
            Parameter::new(PARAM_VOXEL_SIZE, "Voxel Size", "Voxel", ParameterValue::Float(0.05), ParameterUIType::FloatSlider { min: 0.001, max: 10.0 }),
            Parameter::new(PARAM_PI, "Palette Index", "Voxel", ParameterValue::Int(1), ParameterUIType::IntSlider { min: 1, max: 255 }),
            Parameter::new(PARAM_TIMEOUT_S, "Timeout (s)", "AI", ParameterValue::Int(0), ParameterUIType::IntSlider { min: 0, max: 1800 }),
            Parameter::new(PARAM_GENERATE, "Generate", "AI", ParameterValue::Int(0), ParameterUIType::BusyButton { busy_param: PARAM_BUSY.to_string(), busy_label: "Generating...".to_string(), busy_label_param: Some(PARAM_STATUS.to_string()) }),
            Parameter::new(PARAM_DEPTH_ATLAS, "Depth Atlas (Internal)", "Internal", ParameterValue::String(String::new()), ParameterUIType::FilePath { filters: vec!["png".into(), "jpg".into(), "jpeg".into(), "webp".into()] }),
            Parameter::new(PARAM_VOXEL_NODE, "Voxel Node (Internal)", "Internal", ParameterValue::String(String::new()), ParameterUIType::String),
            Parameter::new(PARAM_STATUS, "Status", "Internal", ParameterValue::String(String::new()), ParameterUIType::String),
            Parameter::new(PARAM_ERROR, "Error", "Internal", ParameterValue::String(String::new()), ParameterUIType::String),
            Parameter::new(PARAM_BUSY, "Busy (Internal)", "Debug", ParameterValue::Bool(false), ParameterUIType::Toggle),
        ]
    }
}

impl NodeOp for NanoToVoxelNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mut out = Geometry::new();
        let vs = p_f32(params, PARAM_VOXEL_SIZE, 0.05).max(0.001);
        let nid = resolve_voxel_node_id(params).or_else(|| resolve_voxel_node_id_from_input(inputs));
        if let Some(nid) = nid {
            out.set_detail_attribute(ATTR_VOXEL_SIZE_DETAIL, vec![vs]);
            out.set_detail_attribute("__voxel_pure", vec![1.0f32]);
            out.set_detail_attribute("__voxel_node", vec![nid]);
        }
        Arc::new(out)
    }
}

register_node!(
    NODE_NANO_TO_VOXEL,
    "AI Generation",
    NanoToVoxelNode;
    inputs: &["Geometry"],
    outputs: &["Voxel"],
    style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts
);

#[derive(Clone, Debug)]
struct Spec {
    node_id: NodeId,
    gen: i32,
    sys: String,
    prompt: String,
    ref_image: String,
    tile_res: u32,
    world_size: f32,
    sample_step: u32,
    depth_min: f32,
    voxel_size: f32,
    pi: u8,
    timeout_s: i32,
}

#[derive(Clone, Debug)]
struct JobResult {
    node_id: NodeId,
    depth_atlas_rel: Option<String>,
    voxel_node: Option<String>,
    err: Option<String>,
    elapsed_ms: u128,
}

#[derive(Resource, Default)]
pub(crate) struct NanoToVoxelJobs { map: HashMap<NodeId, JobState> }

#[derive(Default)]
struct JobState { last_gen: i32, inflight: bool, rx: Option<Receiver<JobResult>>, last_poll: Option<Instant> }

#[inline]
fn p_i32(params: &[Parameter], name: &str, d: i32) -> i32 { params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::Int(v) = &p.value { Some(*v) } else { None }).unwrap_or(d) }
#[inline]
fn p_f32(params: &[Parameter], name: &str, d: f32) -> f32 { params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::Float(v) = &p.value { Some(*v) } else { None }).unwrap_or(d) }
#[inline]
fn p_str(params: &[Parameter], name: &str, d: &str) -> String { params.iter().find(|p| p.name == name).and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None }).unwrap_or_else(|| d.to_string()) }

#[inline]
fn set_str(params: &mut [Parameter], name: &str, v: String) -> bool {
    if let Some(p) = params.iter_mut().find(|p| p.name == name) {
        if let ParameterValue::String(cur) = &mut p.value { if *cur == v { return false; } *cur = v; return true; }
        p.value = ParameterValue::String(v);
        return true;
    }
    false
}
#[inline]
fn set_bool(params: &mut [Parameter], name: &str, v: bool) -> bool {
    if let Some(p) = params.iter_mut().find(|p| p.name == name) {
        if let ParameterValue::Bool(cur) = &mut p.value { if *cur == v { return false; } *cur = v; return true; }
        p.value = ParameterValue::Bool(v);
        return true;
    }
    false
}

#[inline]
fn resolve_ref_image_from_input(inputs: &[Arc<dyn GeometryRef>]) -> Option<String> {
    let g = inputs.first().map(|g| g.materialize())?;
    g.get_detail_attribute(ATTR_REF_IMAGE_PATH)
        .and_then(|a| a.as_slice::<String>())
        .and_then(|v| v.first())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[inline]
fn resolve_voxel_node_id(params: &[Parameter]) -> Option<String> {
    let s = p_str(params, PARAM_VOXEL_NODE, "").trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

#[inline]
fn resolve_voxel_node_id_from_input(inputs: &[Arc<dyn GeometryRef>]) -> Option<String> {
    let g = inputs.first().map(|g| g.materialize())?;
    g.get_detail_attribute("__voxel_node")
        .and_then(|a| a.as_slice::<String>())
        .and_then(|v| v.first())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn spawn_job(spec: Spec) -> Receiver<JobResult> {
    let (tx, rx) = unbounded();
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let r = (|| -> Result<(String, String), String> {
            let atlas_w = spec.tile_res * 3;
            let atlas_h = spec.tile_res * 2;
            
            // --- Stage 1: Coarse Geometry (3 Views) ---
            let sys_stage1 = "You are a 3D generator. Generate a 3x2 Depth Atlas.\n\
Rules:\n\
- Output 3 views: Front, Back, Left.\n\
- Row 1: Front, Back, Left.\n\
- Row 2: Leave Black.\n\
- Output grayscale depth (White=Near, Black=Far).\n\
- Background Black.\n\
- RIGID OBJECT. NO DEFORMATION.\n\
- Camera rotates around center.";

            let mut imgs_stage1: Vec<ImgIn> = Vec::new();
            if !spec.ref_image.trim().is_empty() { 
                imgs_stage1.push(load_image_path(spec.ref_image.as_str())?); 
            }
            
            // Call Gemini Stage 1
            let img1 = gemini_generate_image(spec.timeout_s, sys_stage1, spec.prompt.as_str(), &imgs_stage1, atlas_w, atlas_h)?;
            
            // Process Stage 1 -> Coarse SDF
            let rgba1 = decode_rgba_resized(&img1.bytes, atlas_w, atlas_h)?;
            let points1 = points_from_depth_atlas(&rgba1, spec.tile_res, spec.world_size, spec.sample_step, spec.depth_min);
            if points1.is_empty() { return Err("Stage 1 produced no points.".to_string()); }
            
            // Use splat_radius=3 for coarse model
            let sdf1 = splat_points_to_sdf(&points1, spec.voxel_size, 3);
            
            // Render Coarse SDF -> Normal Atlas (6 views)
            let atlas_bytes = render_sdf_to_atlas(&sdf1, spec.tile_res, spec.world_size, false)?;
            
            // --- Stage 2: Refinement (6 Views) ---
            let mut imgs_stage2: Vec<ImgIn> = Vec::new();
            if !spec.ref_image.trim().is_empty() { 
                imgs_stage2.push(load_image_path(spec.ref_image.as_str())?); 
            }
            imgs_stage2.push(ImgIn { mime: "image/png".to_string(), bytes: atlas_bytes });
            
            // Call Gemini Stage 2
            let img2 = gemini_generate_image(spec.timeout_s, spec.sys.as_str(), spec.prompt.as_str(), &imgs_stage2, atlas_w, atlas_h)?;
            
            // --- Stage 3: High Fidelity (24 Views) ---
            // Process Stage 2 -> Refined SDF
            let rgba2 = decode_rgba_resized(&img2.bytes, atlas_w, atlas_h)?;
            let points2 = points_from_depth_atlas(&rgba2, spec.tile_res, spec.world_size, spec.sample_step, spec.depth_min);
            if points2.is_empty() { return Err("Stage 2 produced no points.".to_string()); }
            let sdf2 = splat_points_to_sdf(&points2, spec.voxel_size, 2); // Tighter radius for stage 2

            // Render Refined SDF -> Normal Atlas (24 views)
            let atlas_24_bytes = render_sdf_to_atlas(&sdf2, spec.tile_res, spec.world_size, true)?;
            
            let sys_stage3 = "You are a 3D Consistency Refiner.
Input: A 24-view Normal Atlas (8x3 grid) derived from a refined 3D model.
Task: Generate a high-quality 24-view Depth Atlas.
Rules:
- Output 24 views in 8 columns x 3 rows.
- Row 1: Equator (0, 45, 90, 135, 180, 225, 270, 315 deg).
- Row 2: Elevation +45 deg.
- Row 3: Elevation -45 deg.
- Depth: White=Near, Black=Far.
- Background Black.
- STRICT GEOMETRY CONSISTENCY.";

            let mut imgs_stage3: Vec<ImgIn> = Vec::new();
            if !spec.ref_image.trim().is_empty() { 
                imgs_stage3.push(load_image_path(spec.ref_image.as_str())?); 
            }
            imgs_stage3.push(ImgIn { mime: "image/png".to_string(), bytes: atlas_24_bytes });

            let atlas_w_24 = spec.tile_res * 8;
            let atlas_h_24 = spec.tile_res * 3;
            let img3 = gemini_generate_image(spec.timeout_s, sys_stage3, spec.prompt.as_str(), &imgs_stage3, atlas_w_24, atlas_h_24)?;

            let rel = save_img_under_assets("textures/ai_nano_to_3d", &format!("nano_to_voxel_depth_{}_{}", spec.node_id, spec.gen.max(0)), &img3)?;
            
            // Final Processing (Stage 3 -> Points)
            let rgba3 = decode_rgba_resized(&img3.bytes, atlas_w_24, atlas_h_24)?;
            let points3 = points_from_depth_atlas(&rgba3, spec.tile_res, spec.world_size, spec.sample_step, spec.depth_min);
            if points3.is_empty() { return Err("Stage 3 produced no points.".to_string()); }
            
            let (chunks, solid) = raster_points_to_voxel_chunks(&points3, spec.voxel_size, spec.pi);
            let node_uuid = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, format!("c3d:nano_to_voxel:{}:{}", spec.node_id, spec.gen.max(0)).as_bytes());
            let palette = vox::DiscreteSdfGrid::new(spec.voxel_size.max(0.001)).palette;
            cunning_kernel::nodes::voxel::voxel_edit::voxel_render_register_chunks(node_uuid, spec.voxel_size.max(0.001), palette, chunks, solid);
            Ok((rel, node_uuid.to_string()))
        })();
        let (depth_atlas_rel, voxel_node, err) = match r {
            Ok((a, b)) => (Some(a), Some(b), None),
            Err(e) => (None, None, Some(e)),
        };
        let _ = tx.send(JobResult { node_id: spec.node_id, depth_atlas_rel, voxel_node, err, elapsed_ms: t0.elapsed().as_millis() });
    });
    rx
}

pub(crate) fn nano_to_voxel_jobs_system(
    mut graph_res: ResMut<NodeGraphResource>,
    mut jobs: ResMut<NanoToVoxelJobs>,
    mut graph_changed: MessageWriter<'_, crate::GraphChanged>,
) {
    let g = &mut graph_res.0;
    let now = Instant::now();
    let nodes: Vec<(NodeId, Vec<Parameter>)> = g.nodes.iter().filter_map(|(id, n)| matches!(&n.node_type, NodeType::Generic(s) if s == NODE_NANO_TO_VOXEL).then_some((*id, n.parameters.clone()))).collect();
    for (node_id, mut params) in nodes {
        let st = jobs.map.entry(node_id).or_default();
        let gen = p_i32(&params, PARAM_GENERATE, 0);
        if gen != st.last_gen && !st.inflight {
            st.last_gen = gen;
            st.inflight = true;
            st.last_poll = Some(now);
            let mut changed = false;
            changed |= set_bool(&mut params, PARAM_BUSY, true);
            changed |= set_str(&mut params, PARAM_STATUS, "Starting...".to_string());
            changed |= set_str(&mut params, PARAM_ERROR, String::new());
            let in_port = PortId::from(port_key::in0().as_str());
            let in_geo = first_src(g, node_id, &in_port)
                .and_then(|(src_n, src_p)| cached_output_geo(g, src_n, &src_p).map(|x| (*x).clone()))
                .unwrap_or_else(Geometry::new);
            let spec = Spec {
                node_id,
                gen,
                sys: p_str(&params, PARAM_SYS, SYS_DEPTH_ATLAS),
                prompt: p_str(&params, PARAM_PROMPT, ""),
                ref_image: in_geo
                    .get_detail_attribute(ATTR_REF_IMAGE_PATH)
                    .and_then(|a| a.as_slice::<String>())
                    .and_then(|v| v.first())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| p_str(&params, PARAM_REF_IMAGE, "")),
                tile_res: p_i32(&params, PARAM_TILE_RES, 256).max(64) as u32,
                world_size: p_f32(&params, PARAM_WORLD_SIZE, 1.0).max(0.001),
                sample_step: p_i32(&params, PARAM_SAMPLE_STEP, 4).max(1) as u32,
                depth_min: p_f32(&params, PARAM_DEPTH_MIN, 0.05).clamp(0.0, 1.0),
                voxel_size: p_f32(&params, PARAM_VOXEL_SIZE, 0.05).max(0.001),
                pi: p_i32(&params, PARAM_PI, 1).clamp(1, 255) as u8,
                timeout_s: p_i32(&params, PARAM_TIMEOUT_S, 0),
            };
            st.rx = Some(spawn_job(spec));
            if changed {
                if let Some(n) = g.nodes.get_mut(&node_id) {
                    n.parameters = params.clone();
                    g.bump_param_revision();
                    graph_changed.write(crate::GraphChanged);
                }
            }
        }

        if st.inflight {
            let mut done: Option<JobResult> = None;
            if let Some(rx) = &st.rx {
                while let Ok(r) = rx.try_recv() { done = Some(r); }
            }
            if let Some(r) = done {
                st.inflight = false;
                st.rx = None;
                st.last_poll = None;
                if let Some(n) = g.nodes.get_mut(&r.node_id) {
                    let mut p = n.parameters.clone();
                    let mut changed = false;
                    changed |= set_bool(&mut p, PARAM_BUSY, false);
                    if let Some(e) = r.err.clone() {
                        changed |= set_str(&mut p, PARAM_STATUS, format!("Failed ({}ms)", r.elapsed_ms));
                        changed |= set_str(&mut p, PARAM_ERROR, e);
                    } else {
                        if let Some(a) = r.depth_atlas_rel { changed |= set_str(&mut p, PARAM_DEPTH_ATLAS, a); }
                        if let Some(v) = r.voxel_node { changed |= set_str(&mut p, PARAM_VOXEL_NODE, v); }
                        changed |= set_str(&mut p, PARAM_STATUS, format!("OK ({}ms)", r.elapsed_ms));
                        changed |= set_str(&mut p, PARAM_ERROR, String::new());
                    }
                    if changed {
                        n.parameters = p;
                        g.bump_param_revision();
                        g.mark_dirty(r.node_id);
                        graph_changed.write(crate::GraphChanged);
                    }
                }
            }
        }
    }
}

// ---- graph helpers (minimal) ----

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
