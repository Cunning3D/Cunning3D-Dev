//! VoxelFromHeightmap node: image heightmap -> discrete voxel payload + surface mesh.

use bevy::prelude::*;
use image::GenericImageView;
use std::sync::Arc;
use std::path::PathBuf;
use std::collections::HashMap;
use bytemuck::{Pod, Zeroable};
use once_cell::sync::OnceCell;
use wgpu::util::DeviceExt;

use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::{Attribute, Geometry};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::nodes::gpu::runtime::GpuRuntime;
use crate::nodes::voxel::voxel_edit::{
    voxel_render_register_chunks, ATTR_VOXEL_SIZE_DETAIL,
};
use crate::register_node;
use crate::volume::CHUNK_SIZE;

const PARAM_HEIGHTMAP_PATH: &str = "heightmap_path";
const PARAM_VOXEL_SIZE: &str = "voxel_size";
const PARAM_HEIGHT_SCALE: &str = "height_scale";
const PARAM_HEIGHT_OFFSET: &str = "height_offset";
const PARAM_SOLID_PI: &str = "palette_index";
const PARAM_PI_MIN: &str = "palette_min";
const PARAM_PI_MAX: &str = "palette_max";
const PARAM_GRAD_NOISE: &str = "gradient_noise";
const PARAM_INVERT: &str = "invert";
const PARAM_SURFACE_ONLY: &str = "surface_only";

const ATTR_HEIGHTMAP_PATH: &str = "__ai_heightmap_path";
const ATTR_HEIGHTMAP_ERROR: &str = "__ai_heightmap_error";
const ATTR_HEIGHTMAP_STATS: &str = "__ai_heightmap_stats";
const ATTR_VOXEL_PURE: &str = "__voxel_pure";
const ATTR_VOXEL_NODE: &str = "__voxel_node";

#[derive(Default)]
pub struct VoxelFromHeightmapNode;

impl NodeParameters for VoxelFromHeightmapNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                PARAM_HEIGHTMAP_PATH,
                "Heightmap",
                "Input",
                ParameterValue::String(String::new()),
                ParameterUIType::FilePath {
                    filters: vec!["png".to_string(), "jpg".to_string(), "jpeg".to_string(), "webp".to_string()],
                },
            ),
            Parameter::new(
                PARAM_VOXEL_SIZE,
                "Voxel Size",
                "Voxel",
                ParameterValue::Float(0.1),
                ParameterUIType::FloatSlider { min: 0.001, max: 10.0 },
            ),
            Parameter::new(
                PARAM_HEIGHT_SCALE,
                "Height Scale",
                "Voxel",
                ParameterValue::Float(10.0),
                ParameterUIType::FloatSlider { min: 0.0, max: 2000.0 },
            ),
            Parameter::new(
                PARAM_HEIGHT_OFFSET,
                "Height Offset",
                "Voxel",
                ParameterValue::Float(0.0),
                ParameterUIType::FloatSlider { min: -2000.0, max: 2000.0 },
            ),
            Parameter::new(
                PARAM_SOLID_PI,
                "Palette Index",
                "Voxel",
                ParameterValue::Int(1),
                ParameterUIType::IntSlider { min: 1, max: 255 },
            ),
            Parameter::new(
                PARAM_PI_MIN,
                "Palette Min",
                "Voxel",
                ParameterValue::Int(1),
                ParameterUIType::IntSlider { min: 1, max: 255 },
            ),
            Parameter::new(
                PARAM_PI_MAX,
                "Palette Max",
                "Voxel",
                ParameterValue::Int(1),
                ParameterUIType::IntSlider { min: 1, max: 255 },
            ),
            Parameter::new(
                PARAM_GRAD_NOISE,
                "Gradient Noise",
                "Voxel",
                ParameterValue::Float(0.0),
                ParameterUIType::FloatSlider { min: 0.0, max: 1.0 },
            ),
            Parameter::new(
                PARAM_INVERT,
                "Invert",
                "Voxel",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                PARAM_SURFACE_ONLY,
                "Surface Only",
                "Voxel",
                ParameterValue::Bool(true),
                ParameterUIType::Toggle,
            ),
        ]
    }
}

fn p_f32(params: &[Parameter], name: &str, d: f32) -> f32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| if let ParameterValue::Float(v) = &p.value { Some(*v) } else { None })
        .unwrap_or(d)
}

fn p_i32(params: &[Parameter], name: &str, d: i32) -> i32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| if let ParameterValue::Int(v) = &p.value { Some(*v) } else { None })
        .unwrap_or(d)
}

fn p_bool(params: &[Parameter], name: &str, d: bool) -> bool {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| if let ParameterValue::Bool(v) = &p.value { Some(*v) } else { None })
        .unwrap_or(d)
}

fn p_str(params: &[Parameter], name: &str, d: &str) -> String {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| if let ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None })
        .unwrap_or_else(|| d.to_string())
}

fn resolve_heightmap_path(params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Option<String> {
    // Prefer upstream geometry detail attribute.
    if let Some(g) = inputs.first().map(|g| g.materialize()) {
        if let Some(p) = g
            .get_detail_attribute(ATTR_HEIGHTMAP_PATH)
            .and_then(|a| a.as_slice::<String>())
            .and_then(|v| v.first())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            return Some(p);
        }
    }
    let p = p_str(params, PARAM_HEIGHTMAP_PATH, "").trim().to_string();
    if p.is_empty() { None } else { Some(p) }
}

fn load_image_path(p: &str) -> Result<image::DynamicImage, String> {
    let s = p.trim();
    if s.is_empty() { return Err("Empty heightmap path.".to_string()); }
    let raw = PathBuf::from(s);
    if raw.is_absolute() {
        return image::open(&raw).map_err(|e| format!("image::open({}): {}", raw.display(), e));
    }
    let s2 = s.replace('\\', "/");
    let rel = s2.trim_start_matches("assets/").trim_start_matches("./").to_string();
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let assets = cwd.join("assets");
    let p_assets = assets.join(&rel);
    if p_assets.exists() {
        return image::open(&p_assets).map_err(|e| format!("image::open({}): {}", p_assets.display(), e));
    }
    let p_cwd = cwd.join(&rel);
    if p_cwd.exists() {
        return image::open(&p_cwd).map_err(|e| format!("image::open({}): {}", p_cwd.display(), e));
    }
    Err(format!("Heightmap not found. Tried: {}, {}", p_assets.display(), p_cwd.display()))
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuParams {
    w: u32,
    h: u32,
    max_h: u32,
    invert: u32,
    voxel_size: f32,
    height_scale: f32,
    height_offset: f32,
    grad_noise: f32,
    pi: u32,
    pi_min: u32,
    pi_max: u32,
    seed: u32,
    _pad0: [u32; 4],
}

struct Pipelines {
    bgl: wgpu::BindGroupLayout,
    ppl_find: wgpu::ComputePipeline,
    ppl_cols: wgpu::ComputePipeline,
}
static PPL: OnceCell<Pipelines> = OnceCell::new();

fn pipelines(rt: &GpuRuntime) -> &'static Pipelines {
    PPL.get_or_init(|| {
        let dev = rt.device();
        let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_heightmap_voxel_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let code = r#"
struct MinMax { mn: atomic<u32>, mx: atomic<u32> };
struct Params {
  w: u32, h: u32, max_h: u32, invert: u32,
  voxel_size: f32, height_scale: f32, height_offset: f32, grad_noise: f32,
  pi: u32, pi_min: u32, pi_max: u32, seed: u32,
  _pad0: vec4<u32>,
};
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> mm: MinMax;
@group(0) @binding(2) var<storage, read_write> out_cols: array<u32>;
@group(0) @binding(3) var<uniform> u: Params;

fn hash_u32(x: u32) -> u32 {
  var v = x;
  v = v ^ (v >> 16u);
  v = v * 0x7feb352du;
  v = v ^ (v >> 15u);
  v = v * 0x846ca68bu;
  v = v ^ (v >> 16u);
  return v;
}
fn rand01(x: u32, y: u32, seed: u32) -> f32 {
  let h = hash_u32(x * 73856093u ^ y * 19349663u ^ seed);
  return f32(h) * (1.0 / 4294967295.0);
}

@compute @workgroup_size(16,16,1)
fn find_minmax(@builtin(global_invocation_id) gid: vec3<u32>) {
  let x = gid.x;
  let y = gid.y;
  if (x >= u.w || y >= u.h) { return; }
  let p = textureLoad(tex, vec2<i32>(i32(x), i32(y)), 0).x;
  let g = u32(clamp(p * 255.0 + 0.5, 0.0, 255.0));
  atomicMin(&mm.mn, g);
  atomicMax(&mm.mx, g);
}

@compute @workgroup_size(16,16,1)
fn make_columns(@builtin(global_invocation_id) gid: vec3<u32>) {
  let x = gid.x;
  let z = gid.y;
  if (x >= u.w || z >= u.h) { return; }
  let yy = u.h - 1u - z;
  let p = textureLoad(tex, vec2<i32>(i32(x), i32(yy)), 0).x;
  var v = clamp(p, 0.0, 1.0);
  if (u.invert != 0u) { v = 1.0 - v; }
  let hw = v * u.height_scale + u.height_offset;
  let hv0 = i32(round(hw / max(1e-6, u.voxel_size)));
  let hv = u32(clamp(hv0, 0, i32(u.max_h)));

  let p0 = min(u.pi_min, u.pi_max);
  let p1 = max(u.pi_min, u.pi_max);
  var pi = u.pi;
  if (p0 != p1) {
    let mn = f32(atomicLoad(&mm.mn));
    let mx = f32(atomicLoad(&mm.mx));
    let range = max(1.0, mx - mn);
    let n = (rand01(x, z, u.seed) - 0.5) * 2.0 * (u.grad_noise * 8.0);
    let t = clamp((v * 255.0 + n - mn) / range, 0.0, 1.0);
    pi = u32(clamp(round(f32(p0) + t * f32(p1 - p0)), 1.0, 255.0));
  }
  let idx = z * u.w + x;
  out_cols[idx] = (pi << 16u) | (hv & 0xffffu);
}
"#;
        let sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c3d_heightmap_voxel_wgsl"),
            source: wgpu::ShaderSource::Wgsl(code.into()),
        });
        let pl = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("c3d_heightmap_voxel_pl"),
            bind_group_layouts: &[&bgl],
            immediate_size: 0,
        });
        let ppl_find = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_heightmap_find_minmax"),
            layout: Some(&pl),
            module: &sm,
            entry_point: Some("find_minmax"),
            compilation_options: Default::default(),
            cache: None,
        });
        let ppl_cols = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_heightmap_make_columns"),
            layout: Some(&pl),
            module: &sm,
            entry_point: Some("make_columns"),
            compilation_options: Default::default(),
            cache: None,
        });
        Pipelines { bgl, ppl_find, ppl_cols }
    })
}

fn readback_u32(dev: &wgpu::Device, q: &wgpu::Queue, src: &wgpu::Buffer, len_u32: usize) -> Result<Vec<u32>, String> {
    let size = (len_u32 * 4) as u64;
    let rb = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_heightmap_rb"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("c3d_heightmap_rb_enc") });
    enc.copy_buffer_to_buffer(src, 0, &rb, 0, size);
    q.submit([enc.finish()]);
    let slice = rb.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r.is_ok()); });
    let t0 = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(30);
    let mut ok = None;
    while t0.elapsed() < timeout {
        let _ = dev.poll(wgpu::PollType::Poll);
        if let Ok(v) = rx.try_recv() { ok = Some(v); break; }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    if ok != Some(true) { return Err("GPU readback timeout or failed map_async.".to_string()); }
    let mapped = slice.get_mapped_range();
    let mut out = vec![0u32; len_u32];
    out.copy_from_slice(bytemuck::cast_slice::<u8, u32>(&mapped));
    drop(mapped);
    rb.unmap();
    Ok(out)
}

fn gpu_columns(params: &GpuParams, gray: &image::ImageBuffer<image::Luma<u8>, Vec<u8>>) -> Result<Vec<u32>, String> {
    let rt = GpuRuntime::get_blocking();
    let dev = rt.device();
    let q = rt.queue();
    let ppl = pipelines(rt);
    let (w, h) = (params.w.max(1), params.h.max(1));
    let tex = dev.create_texture(&wgpu::TextureDescriptor {
        label: Some("c3d_heightmap_tex"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    // wgpu requires bytes_per_row to be 256-byte aligned for texture uploads on most backends.
    // Pad each row so arbitrary resolutions work (e.g. 384x384).
    let src = gray.as_raw();
    let stride = ((w as usize + 255) / 256) * 256;
    let mut padded = vec![0u8; stride * (h as usize)];
    for y in 0..(h as usize) {
        let s0 = y * (w as usize);
        let d0 = y * stride;
        padded[d0..d0 + (w as usize)].copy_from_slice(&src[s0..s0 + (w as usize)]);
    }
    q.write_texture(
        tex.as_image_copy(),
        &padded,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride as u32), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    let view = tex.create_view(&Default::default());
    let mm_init = [u32::MAX, 0u32];
    let mm = dev.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("c3d_heightmap_mm"),
        contents: bytemuck::cast_slice(&mm_init),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let out = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_heightmap_cols"),
        size: (w as u64) * (h as u64) * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    // Uniform buffers must satisfy WGSL layout size/alignment (often 16-byte multiples).
    // Create a padded buffer to avoid validation errors if struct size differs across compilers.
    let u_bytes = bytemuck::bytes_of(params);
    let u_size = ((u_bytes.len() as u64) + 15) & !15;
    let u = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_heightmap_u"),
        size: u_size.max(64),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    q.write_buffer(&u, 0, u_bytes);
    let bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("c3d_heightmap_bg"),
        layout: &ppl.bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: mm.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: out.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: u.as_entire_binding() },
        ],
    });
    let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("c3d_heightmap_enc") });
    {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("c3d_heightmap_find"), timestamp_writes: None });
        pass.set_pipeline(&ppl.ppl_find);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups((w + 15) / 16, (h + 15) / 16, 1);
    }
    {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("c3d_heightmap_cols"), timestamp_writes: None });
        pass.set_pipeline(&ppl.ppl_cols);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups((w + 15) / 16, (h + 15) / 16, 1);
    }
    q.submit([enc.finish()]);
    readback_u32(dev, q, &out, (w as usize) * (h as usize))
}

impl NodeOp for VoxelFromHeightmapNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        puffin::profile_scope!("VoxelFromHeightmap::compute");
        let voxel_size = p_f32(params, PARAM_VOXEL_SIZE, 0.1).max(0.001);
        let height_scale = p_f32(params, PARAM_HEIGHT_SCALE, 10.0).max(0.0);
        let height_offset = p_f32(params, PARAM_HEIGHT_OFFSET, 0.0);
        let pi = p_i32(params, PARAM_SOLID_PI, 1).clamp(1, 255) as u8;
        let pi_min = p_i32(params, PARAM_PI_MIN, pi as i32).clamp(1, 255) as u8;
        let pi_max = p_i32(params, PARAM_PI_MAX, pi as i32).clamp(1, 255) as u8;
        let grad_noise = p_f32(params, PARAM_GRAD_NOISE, 0.0).clamp(0.0, 1.0);
        let invert = p_bool(params, PARAM_INVERT, false);
        let surface_only = p_bool(params, PARAM_SURFACE_ONLY, true);

        let Some(rel) = resolve_heightmap_path(params, inputs) else {
            let mut out = Geometry::new();
            out.set_detail_attribute(ATTR_HEIGHTMAP_ERROR, vec!["Missing heightmap path (connect Nano Heightmap or set Heightmap parameter).".to_string()]);
            return Arc::new(out);
        };
        let img = match load_image_path(&rel) {
            Ok(i) => i,
            Err(e) => {
                warn!("Voxel From Heightmap: failed to load '{}': {}", rel, e);
                let mut out = Geometry::new();
                out.set_detail_attribute(ATTR_HEIGHTMAP_ERROR, vec![format!("Failed to load '{}': {}", rel, e)]);
                return Arc::new(out);
            }
        };
        let (w, h) = img.dimensions();
        if w == 0 || h == 0 {
            return Arc::new(Geometry::new());
        }
        let gray = img.to_luma8();
        let max_h = ((height_scale.abs() + height_offset.abs()).max(1.0) / voxel_size).ceil().max(1.0) as u32;
        let (p0, p1) = (pi_min.min(pi_max), pi_min.max(pi_max));
        let seed = (p0 as u32) ^ ((p1 as u32) << 8) ^ 0xC3D0_5EEDu32;
        let gpu_p = GpuParams {
            w,
            h,
            max_h,
            invert: if invert { 1 } else { 0 },
            voxel_size,
            height_scale,
            height_offset,
            grad_noise,
            pi: pi as u32,
            pi_min: pi_min as u32,
            pi_max: pi_max as u32,
            seed,
            _pad0: [0; 4],
        };
        let cols = match gpu_columns(&gpu_p, &gray) {
            Ok(v) => v,
            Err(e) => {
                warn!("Voxel From Heightmap (GPU): {}", e);
                let mut out = Geometry::new();
                out.set_detail_attribute(ATTR_HEIGHTMAP_ERROR, vec![format!("GPU voxelization failed: {}", e)]);
                return Arc::new(out);
            }
        };

        // GPU columns pack: [pi<<16 | hv] per (x,z) where z matches output z (top-down flip applied in shader).
        let mut total = 0usize;
        let mut max_hv = 0u32;
        for &v in cols.iter() {
            let hv = v & 0xffff;
            max_hv = max_hv.max(hv);
            total += hv as usize;
        }
        if total == 0 {
            let mut out = Geometry::new();
            out.set_detail_attribute(
                ATTR_HEIGHTMAP_ERROR,
                vec![format!(
                    "No voxels generated (all heights are 0). Try increasing Height Scale / Height Offset, or check the heightmap is not black. max_hv={}",
                    max_hv
                )],
            );
            out.set_detail_attribute(ATTR_HEIGHTMAP_STATS, vec![format!("w={} h={} total_voxels=0 max_hv={}", w, h, max_hv)]);
            out.set_detail_attribute(ATTR_VOXEL_SIZE_DETAIL, vec![voxel_size]);
            return Arc::new(out);
        }
        let cs = CHUNK_SIZE.max(1);
        let csu = cs as usize;
        let cs2u = csu * csu;
        let cs3u = cs2u * csu;
        let idx3 = |lx: i32, ly: i32, lz: i32| -> usize {
            (lz as usize) * cs2u + (ly as usize) * csu + (lx as usize)
        };
        let mut chunks: HashMap<IVec3, Vec<u8>> = HashMap::new();
        let mut solid: HashMap<IVec3, u32> = HashMap::new();
        let mut set_vox = |x: i32, y: i32, z: i32, pi2: u8| {
            if pi2 == 0 { return; }
            let ck = IVec3::new(x / cs, y / cs, z / cs);
            let lx = x - ck.x * cs;
            let ly = y - ck.y * cs;
            let lz = z - ck.z * cs;
            let buf = chunks.entry(ck).or_insert_with(|| vec![0u8; cs3u]);
            let i = idx3(lx, ly, lz);
            if buf[i] == 0 {
                buf[i] = pi2;
                *solid.entry(ck).or_insert(0) += 1;
            }
        };
        let get_hv = |x: i32, z: i32| -> i32 {
            if x < 0 || z < 0 || x >= w as i32 || z >= h as i32 { return 0; }
            let v = cols[(z as usize) * (w as usize) + (x as usize)];
            (v & 0xffff) as i32
        };
        for z in 0..h as i32 {
            for x in 0..w as i32 {
                let v = cols[(z as usize) * (w as usize) + (x as usize)];
                let hv = (v & 0xffff) as i32;
                if hv <= 0 { continue; }
                let pi2 = ((v >> 16) & 0xff) as u8;
                if surface_only {
                    let y_top = hv - 1;
                    set_vox(x, y_top, z, pi2.max(1));
                    let mn = get_hv(x - 1, z)
                        .min(get_hv(x + 1, z))
                        .min(get_hv(x, z - 1))
                        .min(get_hv(x, z + 1))
                        .max(0);
                    if mn < hv - 1 {
                        for y in mn..(hv - 1) {
                            set_vox(x, y, z, pi2.max(1));
                        }
                    }
                } else {
                    for y in 0..hv {
                        set_vox(x, y, z, pi2.max(1));
                    }
                }
            }
        }

        // GPU-first viewport path: register the grid into the voxel render cache and output a pure-voxel marker.
        let nid = uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            format!(
                "c3d:voxel_from_heightmap:{}:{}:{}:{}:{}:{}:{}:{}:{}",
                rel,
                voxel_size,
                height_scale,
                height_offset,
                invert as u32,
                pi,
                pi_min,
                pi_max,
                grad_noise
            )
            .as_bytes(),
        );
        let palette = cunning_kernel::algorithms::algorithms_editor::voxel::DiscreteVoxelGrid::new(voxel_size).palette;
        voxel_render_register_chunks(nid, voxel_size, palette, chunks, solid);
        let mut out = Geometry::new();
        out.set_detail_attribute(ATTR_VOXEL_SIZE_DETAIL, vec![voxel_size]);
        out.set_detail_attribute(ATTR_VOXEL_PURE, vec![1.0f32]);
        out.set_detail_attribute(ATTR_VOXEL_NODE, vec![nid.to_string()]);
        out.set_detail_attribute(ATTR_HEIGHTMAP_STATS, vec![format!("w={} h={} total_voxels={} max_hv={}", w, h, total, max_hv)]);
        Arc::new(out)
    }
}

register_node!("Voxel From Heightmap", "Voxel", VoxelFromHeightmapNode);

