use std::{collections::HashMap, sync::Arc};
use bytemuck::{Pod, Zeroable};
use etagere::AtlasAllocator;
use wgpu::util::DeviceExt;

use cosmic_text::{Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, PlatformFallback, Shaping, SwashCache};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::atomic::AtomicU32;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, Default)]
pub struct GpuTextBatchStats {
    pub frame_id: u64,
    pub texts: u64,
    pub clip_regions: u64,
    pub drawcalls: u64,
    pub verts: u64,
}

static GPU_TEXT_LAST_FRAME: AtomicU64 = AtomicU64::new(0);
static GPU_TEXT_LAST_TEXTS: AtomicU64 = AtomicU64::new(0);
static GPU_TEXT_LAST_CLIP_REGIONS: AtomicU64 = AtomicU64::new(0);
static GPU_TEXT_LAST_DRAWCALLS: AtomicU64 = AtomicU64::new(0);
static GPU_TEXT_LAST_VERTS: AtomicU64 = AtomicU64::new(0);
static GPU_TEXT_VERBOSE_DETAILS: AtomicBool = AtomicBool::new(false);
static GPU_TEXT_LAST_DETAILS_FRAME: AtomicU64 = AtomicU64::new(0);
static GPU_TEXT_LAST_DETAILS: OnceLock<Mutex<Vec<GpuTextClipBatchStat>>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default)]
pub struct GpuTextClipBatchStat {
    pub scissor: [u32; 4],
    pub callbacks: u32,
    pub glyphs: u32,
    pub verts: u32,
}

pub fn gpu_text_last_stats() -> GpuTextBatchStats {
    GpuTextBatchStats {
        frame_id: GPU_TEXT_LAST_FRAME.load(Ordering::Relaxed),
        texts: GPU_TEXT_LAST_TEXTS.load(Ordering::Relaxed),
        clip_regions: GPU_TEXT_LAST_CLIP_REGIONS.load(Ordering::Relaxed),
        drawcalls: GPU_TEXT_LAST_DRAWCALLS.load(Ordering::Relaxed),
        verts: GPU_TEXT_LAST_VERTS.load(Ordering::Relaxed),
    }
}

pub fn gpu_text_set_verbose_details_enabled(enabled: bool) { GPU_TEXT_VERBOSE_DETAILS.store(enabled, Ordering::Relaxed); }

pub fn gpu_text_last_batch_details() -> (u64, Vec<GpuTextClipBatchStat>) {
    let fid = GPU_TEXT_LAST_DETAILS_FRAME.load(Ordering::Relaxed);
    let v = GPU_TEXT_LAST_DETAILS.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap().clone();
    (fid, v)
}

fn gpu_text_debug_level() -> u32 {
    std::env::var("CUNNING_GPU_TEXT_DEBUG").unwrap_or("0".to_string()).parse().unwrap_or(0)
}

fn gpu_text_skip_raster() -> bool { std::env::var("CUNNING_GPU_TEXT_SKIP_RASTER").map(|v| v != "0").unwrap_or(false) }
fn gpu_text_skip_upload() -> bool { std::env::var("CUNNING_GPU_TEXT_SKIP_UPLOAD").map(|v| v != "0").unwrap_or(false) }
fn gpu_text_dummy_upload() -> bool { std::env::var("CUNNING_GPU_TEXT_DUMMY_UPLOAD").map(|v| v != "0").unwrap_or(false) }

#[derive(Clone, Copy, Debug)]
pub struct GpuTextTuning {
    pub scale: f32,
    pub line_height_factor: f32,
    pub y_offset_points: f32,
    pub gamma: f32,
    pub grayscale_enhanced_contrast: f32,
    pub content_weight: f32,
    pub sans_family: u32, // 0=Auto(SansSerif), 1=Segoe UI, 2=Microsoft YaHei, 3=SimHei, 4=SimSun
    pub upload_budget: u32, // max new glyph uploads per frame (0 disables uploads)
    pub scissor_pad_y: u32,
    pub row_bounds_clamp: bool,
    pub load_system_fonts: bool, // note: only affects next FontSystem creation
}

static GT_INIT: OnceLock<()> = OnceLock::new();
const F32_NAN_BITS: u32 = 0x7FC0_0000;
static GT_SCALE: AtomicU32 = AtomicU32::new(F32_NAN_BITS);
static GT_LH: AtomicU32 = AtomicU32::new(F32_NAN_BITS);
static GT_YOFF: AtomicU32 = AtomicU32::new(F32_NAN_BITS);
static GT_GAMMA: AtomicU32 = AtomicU32::new(F32_NAN_BITS);
static GT_CONTRAST: AtomicU32 = AtomicU32::new(F32_NAN_BITS);
static GT_PADY: AtomicU32 = AtomicU32::new(u32::MAX);
static GT_ROW_CLAMP: AtomicU32 = AtomicU32::new(u32::MAX);
static GT_LOAD_SYS: AtomicU32 = AtomicU32::new(u32::MAX);
// Default weight 1.15 to "thin out" the glyphs slightly (solve "too bold" issue)
static GT_WEIGHT: AtomicU32 = AtomicU32::new(0x3F933333); // 1.15
static GT_SANS_FAMILY: AtomicU32 = AtomicU32::new(0);
static GT_UPLOAD_BUDGET: AtomicU32 = AtomicU32::new(u32::MAX);

#[inline]
fn env_f32(k: &str, d: f32) -> f32 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
#[inline]
fn env_u32(k: &str, d: u32) -> u32 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
#[inline]
fn env_bool(k: &str, d: bool) -> bool { std::env::var(k).map(|v| v != "0").unwrap_or(d) }

fn gt_init() {
    GT_INIT.get_or_init(|| {
        GT_SCALE.store(env_f32("CUNNING_GPU_TEXT_SCALE", 1.0).clamp(0.5, 2.0).to_bits(), Ordering::Relaxed);
        GT_LH.store(env_f32("CUNNING_GPU_TEXT_LINE_HEIGHT_FACTOR", 1.25).clamp(1.0, 2.0).to_bits(), Ordering::Relaxed);
        GT_YOFF.store(env_f32("CUNNING_GPU_TEXT_Y_OFFSET", 0.0).clamp(-8.0, 8.0).to_bits(), Ordering::Relaxed);
        GT_GAMMA.store(env_f32("CUNNING_FONTS_GAMMA", 1.8).clamp(1.0, 2.2).to_bits(), Ordering::Relaxed);
        // Default boosted slightly to match GPUI/Zed-style perceived sharpness.
        // Match GPUI/Zed default; users can raise it live in the tuning panel when they want extra sharpness.
        GT_CONTRAST.store(env_f32("CUNNING_FONTS_GRAYSCALE_ENHANCED_CONTRAST", 1.0).max(0.0).to_bits(), Ordering::Relaxed);
        GT_PADY.store(env_u32("CUNNING_GPU_TEXT_SCISSOR_PAD_Y", 4).min(32), Ordering::Relaxed);
        GT_ROW_CLAMP.store(env_bool("CUNNING_GPU_TEXT_ROW_CLAMP", true) as u32, Ordering::Relaxed);
        GT_LOAD_SYS.store(env_bool("CUNNING_GPU_TEXT_LOAD_SYSTEM_FONTS", false) as u32, Ordering::Relaxed);
        GT_WEIGHT.store(env_f32("CUNNING_FONTS_CONTENT_WEIGHT", 1.15).max(0.01).to_bits(), Ordering::Relaxed);
        GT_SANS_FAMILY.store(env_u32("CUNNING_GPU_TEXT_SANS_FAMILY", 0), Ordering::Relaxed);
        GT_UPLOAD_BUDGET.store(env_u32("CUNNING_GPU_TEXT_UPLOAD_BUDGET", 96).min(4096), Ordering::Relaxed);
    });
}

#[inline]
fn snap_to_pixel(v_points: f32, ppp: f32) -> f32 {
    if !ppp.is_finite() || ppp <= 0.0 { return v_points; }
    (v_points * ppp).round() / ppp
}

#[inline]
pub fn gpu_text_tuning_get() -> GpuTextTuning {
    gt_init();
    GpuTextTuning {
        scale: f32::from_bits(GT_SCALE.load(Ordering::Relaxed)),
        line_height_factor: f32::from_bits(GT_LH.load(Ordering::Relaxed)),
        y_offset_points: f32::from_bits(GT_YOFF.load(Ordering::Relaxed)),
        gamma: f32::from_bits(GT_GAMMA.load(Ordering::Relaxed)),
        grayscale_enhanced_contrast: f32::from_bits(GT_CONTRAST.load(Ordering::Relaxed)),
        content_weight: f32::from_bits(GT_WEIGHT.load(Ordering::Relaxed)).max(0.01),
        sans_family: GT_SANS_FAMILY.load(Ordering::Relaxed),
        upload_budget: GT_UPLOAD_BUDGET.load(Ordering::Relaxed).min(4096),
        scissor_pad_y: GT_PADY.load(Ordering::Relaxed),
        row_bounds_clamp: GT_ROW_CLAMP.load(Ordering::Relaxed) != 0,
        load_system_fonts: GT_LOAD_SYS.load(Ordering::Relaxed) != 0,
    }
}

#[inline]
pub fn gpu_text_tuning_set(t: GpuTextTuning) {
    gt_init();
    GT_SCALE.store(t.scale.clamp(0.5, 2.0).to_bits(), Ordering::Relaxed);
    GT_LH.store(t.line_height_factor.clamp(1.0, 2.0).to_bits(), Ordering::Relaxed);
    GT_YOFF.store(t.y_offset_points.clamp(-8.0, 8.0).to_bits(), Ordering::Relaxed);
    GT_GAMMA.store(t.gamma.clamp(1.0, 2.2).to_bits(), Ordering::Relaxed);
    GT_CONTRAST.store(t.grayscale_enhanced_contrast.max(0.0).to_bits(), Ordering::Relaxed);
    GT_WEIGHT.store(t.content_weight.max(0.01).to_bits(), Ordering::Relaxed);
    GT_SANS_FAMILY.store(t.sans_family, Ordering::Relaxed);
    GT_UPLOAD_BUDGET.store(t.upload_budget.min(4096), Ordering::Relaxed);
    GT_PADY.store(t.scissor_pad_y.min(32), Ordering::Relaxed);
    GT_ROW_CLAMP.store(t.row_bounds_clamp as u32, Ordering::Relaxed);
    GT_LOAD_SYS.store(t.load_system_fonts as u32, Ordering::Relaxed);
}

#[inline]
fn gpu_text_scale() -> f32 { gt_init(); f32::from_bits(GT_SCALE.load(Ordering::Relaxed)).clamp(0.5, 2.0) }
#[inline]
fn gpu_text_gamma() -> f32 { gt_init(); f32::from_bits(GT_GAMMA.load(Ordering::Relaxed)).clamp(1.0, 2.2) }
#[inline]
fn gpu_text_grayscale_enhanced_contrast() -> f32 { gt_init(); f32::from_bits(GT_CONTRAST.load(Ordering::Relaxed)).max(0.0) }
#[inline]
fn gpu_text_y_offset_points() -> f32 { gt_init(); f32::from_bits(GT_YOFF.load(Ordering::Relaxed)) }
#[inline]
fn gpu_text_line_height_factor() -> f32 { gt_init(); f32::from_bits(GT_LH.load(Ordering::Relaxed)).clamp(1.0, 2.0) }
#[inline]
fn gpu_text_scissor_pad_y() -> u32 { gt_init(); GT_PADY.load(Ordering::Relaxed).min(32) }
#[inline]
fn gpu_text_row_bounds_clamp() -> bool { gt_init(); GT_ROW_CLAMP.load(Ordering::Relaxed) != 0 }
#[inline]
fn gpu_text_load_system_fonts() -> bool { gt_init(); GT_LOAD_SYS.load(Ordering::Relaxed) != 0 }
#[inline]
fn gpu_text_content_weight() -> f32 { gt_init(); f32::from_bits(GT_WEIGHT.load(Ordering::Relaxed)).max(0.01) }
#[inline]
fn gpu_text_sans_family_id() -> u32 { gt_init(); GT_SANS_FAMILY.load(Ordering::Relaxed) }
#[inline]
fn gpu_text_upload_budget() -> u32 { gt_init(); GT_UPLOAD_BUDGET.load(Ordering::Relaxed).min(4096) }

// From Zed/GPUI: crates/gpui/src/platform.rs:get_gamma_correction_ratios
fn gamma_ratios_from_gamma(gamma: f32) -> [f32; 4] {
    const RATIOS: [[f32; 4]; 13] = [
        [0.0000 / 4.0, 0.0000 / 4.0, 0.0000 / 4.0, 0.0000 / 4.0],
        [0.0166 / 4.0, -0.0807 / 4.0, 0.2227 / 4.0, -0.0751 / 4.0],
        [0.0350 / 4.0, -0.1760 / 4.0, 0.4325 / 4.0, -0.1370 / 4.0],
        [0.0543 / 4.0, -0.2821 / 4.0, 0.6302 / 4.0, -0.1876 / 4.0],
        [0.0739 / 4.0, -0.3963 / 4.0, 0.8167 / 4.0, -0.2287 / 4.0],
        [0.0933 / 4.0, -0.5161 / 4.0, 0.9926 / 4.0, -0.2616 / 4.0],
        [0.1121 / 4.0, -0.6395 / 4.0, 1.1588 / 4.0, -0.2877 / 4.0],
        [0.1300 / 4.0, -0.7649 / 4.0, 1.3159 / 4.0, -0.3080 / 4.0],
        [0.1469 / 4.0, -0.8911 / 4.0, 1.4644 / 4.0, -0.3234 / 4.0],
        [0.1627 / 4.0, -1.0170 / 4.0, 1.6051 / 4.0, -0.3347 / 4.0],
        [0.1773 / 4.0, -1.1420 / 4.0, 1.7385 / 4.0, -0.3426 / 4.0],
        [0.1908 / 4.0, -1.2652 / 4.0, 1.8650 / 4.0, -0.3476 / 4.0],
        [0.2031 / 4.0, -1.3864 / 4.0, 1.9851 / 4.0, -0.3501 / 4.0],
    ];
    const NORM13: f32 = ((0x10000 as f64) / (255.0 * 255.0) * 4.0) as f32;
    const NORM24: f32 = ((0x100 as f64) / (255.0) * 4.0) as f32;
    let idx = ((gamma * 10.0).round() as isize).clamp(10, 22) as usize - 10;
    let r = RATIOS[idx];
    [r[0] * NORM13, r[1] * NORM24, r[2] * NORM13, r[3] * NORM24]
}

fn log_debug(msg: &str) {
    if gpu_text_debug_level() > 0 {
        eprintln!("[GPU_TEXT] {}", msg);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TextUniform {
    screen_size: [f32; 2],
    _pad0: [f32; 2],
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    // WGSL `vec3` has 16-byte alignment in uniform buffers, so we must pad from offset 36 -> 48.
    _pad_align: [f32; 3],
    _pad1: [f32; 3],
    // New field for controlling font weight/thickness (thinning)
    content_weight: f32, // 4 bytes
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TextVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    clip: [f32; 4], // min_x,min_y,max_x,max_y in pixels
}

const GPU_TEXT_SHADER: &str = r#"
struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) clip: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) clip: vec4<f32>,
};

struct Uniforms {
    screen_size: vec2<f32>,
    _pad0: vec2<f32>,
    gamma_ratios: vec4<f32>,
    grayscale_enhanced_contrast: f32,
    _pad1: vec3<f32>,
    // Offset 60 bytes -> 64 bytes total struct size
    content_weight: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(1) @binding(0) var glyph_tex: texture_2d<f32>;
@group(1) @binding(1) var glyph_samp: sampler;

fn sdf_coverage(dist: f32) -> f32 {
    let w = clamp(fwidth(dist), 1.0 / 255.0, 0.25);
    // Aggressively bolden: center at 0.4 (dilation) to match native weight and fix visibility at small sizes.
    // Also clamp w to prevent complete blowout on extreme angles.
    return smoothstep(0.4 - w, 0.4 + w, dist);
}

// Contrast and gamma correction adapted from Zed/GPUI (Microsoft Terminal DWrite alpha correction).
fn color_brightness(color: vec3<f32>) -> f32 { return dot(color, vec3<f32>(0.30, 0.59, 0.11)); }
fn light_on_dark_contrast(enhanced: f32, color: vec3<f32>) -> f32 { let b = color_brightness(color); return enhanced * saturate(4.0 * (0.75 - b)); }
fn enhance_contrast(alpha: f32, k: f32) -> f32 { return alpha * (k + 1.0) / (alpha * k + 1.0); }
fn apply_alpha_correction(a: f32, b: f32, g: vec4<f32>) -> f32 { let ba = g.x * b + g.y; let c = ba * a + (g.z * b + g.w); return a + a * (1.0 - a) * c; }
fn apply_contrast_and_gamma_correction(sample: f32, color: vec3<f32>, enhanced: f32, g: vec4<f32>) -> f32 {
    let k = light_on_dark_contrast(enhanced, color);
    let b = color_brightness(color);
    return apply_alpha_correction(enhance_contrast(sample, k), b, g);
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let p = in.pos / u.screen_size;
    let ndc = vec2<f32>(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0);
    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    out.clip = in.clip;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var sample = textureSample(glyph_tex, glyph_samp, in.uv).r;
    
    // Thinning/Weight adjustment:
    // If content_weight > 1.0, low values drop faster -> thinner.
    // If content_weight < 1.0, low values boosted -> bolder.
    // Default 1.0 = no change.
    sample = pow(sample, u.content_weight);

    let a = apply_contrast_and_gamma_correction(sample, in.color.rgb, u.grayscale_enhanced_contrast, u.gamma_ratios);
    let out_a = in.color.a * a;
    if (out_a <= 0.0) { discard; }
    // `in.color` is linear premultiplied; modulate coverage by atlas alpha.
    return in.color * a;
}
"#;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct GlyphKey(CacheKey);

#[derive(Clone, Copy, Debug)]
struct AtlasEntry {
    uv_min: [f32; 2],
    uv_max: [f32; 2],
    // placement in pixels relative to baseline
    left: i32,
    top: i32,
    w: u32,
    h: u32,
}

struct TextAtlas {
    allocator: AtlasAllocator,
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    size: (u32, u32),
    glyphs: HashMap<GlyphKey, AtlasEntry>,
}

impl TextAtlas {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let size = (2048u32, 2048u32);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Cunning Text Atlas"),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Mask-only atlas: R8 (sample `.r` in shader). We upload via copy_buffer_to_texture (not write_texture).
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Cunning Text Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            // Linear filtering is essential for SDF to reconstruct smooth distance field between texels.
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Cunning Text Atlas BindGroup"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        Self {
            allocator: AtlasAllocator::new(etagere::Size::new(size.0 as i32, size.1 as i32)),
            texture,
            bind_group,
            size,
            glyphs: HashMap::new(),
        }
    }
}

struct GpuTextRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform_layout: wgpu::BindGroupLayout,
    atlas_layout: wgpu::BindGroupLayout,
}

impl GpuTextRenderer {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Cunning GPU Text Shader"),
            source: wgpu::ShaderSource::Wgsl(GPU_TEXT_SHADER.into()),
        });

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cunning GPU Text Uniform BGL"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cunning GPU Text Atlas BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cunning GPU Text Pipeline Layout"),
            bind_group_layouts: &[&uniform_layout, &atlas_layout],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TextVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Cunning GPU Text Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[vertex_layout] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self { pipeline, uniform_layout, atlas_layout }
    }
}

pub struct GpuTextQueue {
    renderer: Arc<GpuTextRenderer>,
    atlas: TextAtlas,
    font_system: FontSystem,
    swash_cache: SwashCache,
    buffer: Buffer,
    scratch_glyphs: Vec<(CacheKey, i32, i32, i32)>,
    scratch_verts: Vec<TextVertex>,
    scratch_upload: Vec<u8>, // aligned staging bytes (COPY_BYTES_PER_ROW_ALIGNMENT)
    pending_bytes: Vec<u8>,
    pending_uploads: Vec<PendingR8Upload>,
    new_glyphs_this_frame: u32,
    clip_batches: Vec<TextClipBatch>,
    clip_map: HashMap<u64, usize>,
    all_verts: Vec<TextVertex>,
    // Per-frame transient buffers
    vb: Option<wgpu::Buffer>,
    uniform_bg: Option<wgpu::BindGroup>,
    last_upload_frame_id: u64,
    staging_buffers: Vec<wgpu::Buffer>,
    target_px: [u32; 2],
    ppp: f32,
    last_frame_id: u64,
    frame_texts: u64,
    last_seq_id: u64,
}

#[derive(Clone, Copy, Debug)]
struct PendingR8Upload {
    offset: u64,
    bytes_per_row: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

struct TextClipBatch {
    scissor: [u32; 4],
    verts: Vec<TextVertex>,
    start: u32,
    count: u32,
    leader: bool,
    callbacks: u32,
}

#[derive(Clone)]
pub struct GpuTextUniform {
    pub text: String,
    pub pos: egui::Pos2,
    pub color: egui::Color32,
    pub font_px: f32, // in egui points
    pub bounds: egui::Vec2, // layout bounds in egui points
    pub family: u8, // 0: SansSerif, 1: Monospace
}

pub struct GpuTextCallback {
    pub rect: egui::Rect,
    pub uniform: GpuTextUniform,
    pub frame_id: u64,
    pub key: Arc<AtomicU64>, // packed: [valid:1][batch_idx:62][is_leader:1]
}

#[inline]
fn pack_key(batch_idx: usize, is_leader: bool) -> u64 {
    (1u64 << 63) | ((batch_idx as u64) << 1) | (is_leader as u64)
}

#[inline]
fn unpack_key(v: u64) -> Option<(usize, bool)> {
    if (v >> 63) == 0 { return None; }
    let is_leader = (v & 1) != 0;
    let batch_idx = ((v & ((1u64 << 63) - 1)) >> 1) as usize;
    Some((batch_idx, is_leader))
}

#[inline]
fn align_up_u64(v: u64, align: u64) -> u64 { if align == 0 { v } else { ((v + align - 1) / align) * align } }

#[allow(dead_code)]
fn write_rgba8_texture(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, staging_keepalive: &mut Vec<wgpu::Buffer>, texture: &wgpu::Texture, x: u32, y: u32, w: u32, h: u32, rgba: &[u8]) {
    if w == 0 || h == 0 { return; }
    let expected = (w.saturating_mul(h).saturating_mul(4)) as usize;
    if rgba.len() < expected { return; }
    let bytes_per_row = w.saturating_mul(4);
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u32;
    let padded_bpr = ((bytes_per_row + align - 1) / align) * align;
    let padded_len = (padded_bpr.saturating_mul(h)) as usize;
    let mut padded = vec![0u8; padded_len];
    for row in 0..h {
        let src = (row * bytes_per_row) as usize;
        let dst = (row * padded_bpr) as usize;
        padded[dst..dst + bytes_per_row as usize].copy_from_slice(&rgba[src..src + bytes_per_row as usize]);
    }
    let staging = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Cunning Text Atlas Staging (RGBA8)"), contents: &padded, usage: wgpu::BufferUsages::COPY_SRC });
    encoder.copy_buffer_to_texture(
        wgpu::TexelCopyBufferInfo { buffer: &staging, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded_bpr), rows_per_image: Some(h) } },
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    staging_keepalive.push(staging);
}

// Queue glyph mask upload (R8) with a 1px transparent border (avoid linear sampling bleeding).
fn queue_r8_texture_padded(
    pending_bytes: &mut Vec<u8>,
    pending_uploads: &mut Vec<PendingR8Upload>,
    scratch_upload: &mut Vec<u8>,
    x0: u32,
    y0: u32,
    w: u32,
    h: u32,
    data: &[u8],
) {
    let pw = w.saturating_add(2);
    let ph = h.saturating_add(2);
    if pw == 0 || ph == 0 { return; }
    if data.len() < (w.saturating_mul(h)) as usize { return; }
    let bytes_per_row = pw;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u32;
    let padded_bpr = ((bytes_per_row + align - 1) / align) * align;
    let need = (padded_bpr.saturating_mul(ph)) as usize;
    if scratch_upload.len() < need { scratch_upload.resize(need, 0); }
    scratch_upload[..need].fill(if gpu_text_dummy_upload() { 255 } else { 0 });
    if !gpu_text_dummy_upload() {
        for yy in 0..h {
            let src = (yy * w) as usize;
            let dst = ((yy + 1) * padded_bpr + 1) as usize;
            scratch_upload[dst..dst + w as usize].copy_from_slice(&data[src..src + w as usize]);
        }
    }
    let off = align_up_u64(pending_bytes.len() as u64, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64);
    if pending_bytes.len() as u64 != off { pending_bytes.resize(off as usize, 0); }
    pending_uploads.push(PendingR8Upload { offset: off, bytes_per_row: padded_bpr, x: x0, y: y0, w: pw, h: ph });
    pending_bytes.extend_from_slice(&scratch_upload[..need]);
}

fn detect_locale() -> String {
    std::env::var("LC_ALL").or_else(|_| std::env::var("LC_CTYPE")).or_else(|_| std::env::var("LANG")).unwrap_or_else(|_| "en-US".to_owned())
}

fn create_font_system() -> FontSystem {
    let mut db = cosmic_text::fontdb::Database::new();
    #[cfg(target_arch = "wasm32")]
    {
        let data: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cunning_wasm_sans_font.bin"));
        db.load_font_data(data.to_vec());
        // Prefer the same family names we use on native Windows, but fall back if the font doesn't provide them.
        fn has_family(db: &cosmic_text::fontdb::Database, name: &str) -> bool { db.faces().any(|f| f.families.iter().any(|(fam, _)| fam.as_str() == name)) }
        fn pick<'a>(db: &cosmic_text::fontdb::Database, names: &'a [&'a str]) -> Option<&'a str> { names.iter().copied().find(|n| has_family(db, n)) }
        let sans = pick(&db, &["Microsoft YaHei", "Microsoft YaHei UI", "SimHei", "SimSun"])
            .or_else(|| db.faces().next().and_then(|f| f.families.first().map(|(fam, _)| fam.as_str())))
            .unwrap_or("sans-serif")
            .to_string();
        db.set_sans_serif_family(&sans);
        db.set_monospace_family(&sans);
        db.set_serif_family(&sans);
    }
    #[cfg(target_os = "windows")]
    {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_owned());
        let dirs = [std::path::PathBuf::from(format!("{windir}\\Fonts")), std::path::PathBuf::from("C:\\Windows\\Fonts")];
        // CJK support: prefer common system CJK fonts (YaHei/SimSun/HeiTi), then fall back to the system font database.
        let candidates = [
            // UI/Latin
            "segoeui.ttf", "seguisym.ttf", "seguiemj.ttf",
            // Mono
            "consola.ttf", "cascadiamono.ttf", "cascadiacode.ttf",
            // CJK
            "simhei.ttf", "msyh.ttf", "msyh.ttc", "simsun.ttc",
        ];
        for dir in dirs { if !dir.exists() { continue; } for f in candidates { if let Ok(data) = std::fs::read(dir.join(f)) { db.load_font_data(data); } } }
        // Stability first: the Windows font directory may contain corrupted fonts (e.g. mstmc.ttf); full scans can occasionally crash native code.
        // For broader coverage, set env var: CUNNING_GPU_TEXT_LOAD_SYSTEM_FONTS=1
        if gpu_text_load_system_fonts() {
            db.load_system_fonts();
        }
        // Keep defaults consistent, but only if the family actually exists (avoid tofu due to missing family name).
        fn has_family(db: &cosmic_text::fontdb::Database, name: &str) -> bool {
            db.faces().any(|f| f.families.iter().any(|(fam, _)| fam.as_str() == name))
        }
        fn pick<'a>(db: &cosmic_text::fontdb::Database, names: &'a [&'a str]) -> Option<&'a str> {
            names.iter().copied().find(|n| has_family(db, n))
        }
        let sans = pick(&db, &["Microsoft YaHei", "Microsoft YaHei UI", "SimHei", "SimSun", "Segoe UI"]).unwrap_or("Segoe UI");
        let mono = pick(&db, &["Consolas", "Cascadia Mono", "Cascadia Code", "Segoe UI Mono"]).unwrap_or("Consolas");
        let serif = pick(&db, &["Segoe UI", "Times New Roman"]).unwrap_or("Segoe UI");
        db.set_sans_serif_family(sans);
        db.set_monospace_family(mono);
        db.set_serif_family(serif);
    }
    FontSystem::new_with_locale_and_db_and_fallback(detect_locale(), db, PlatformFallback)
}

impl crate::CallbackTrait for GpuTextCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        screen_descriptor: &crate::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut crate::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let fid = resources.get::<super::SdfFrameId>().map(|v| v.0).unwrap_or(self.frame_id);
        let seq = resources.get::<super::SdfRenderSeq>().map(|v| v.0).unwrap_or(0);
        let fmt = super::target_format(resources);
        let q = resources.entry::<GpuTextQueue>().or_insert_with(|| {
            let renderer = Arc::new(GpuTextRenderer::new(device, fmt));
            let atlas = TextAtlas::new(device, &renderer.atlas_layout);
            let mut font_system = create_font_system();
            let buffer = Buffer::new(&mut font_system, Metrics::new(14.0, 18.0));
            GpuTextQueue {
                renderer,
                atlas,
                font_system,
                swash_cache: SwashCache::new(),
                buffer,
                scratch_glyphs: Vec::with_capacity(256),
                scratch_verts: Vec::with_capacity(1024),
                scratch_upload: Vec::with_capacity(4096),
                pending_bytes: Vec::with_capacity(64 * 1024),
                pending_uploads: Vec::with_capacity(256),
                new_glyphs_this_frame: 0,
                clip_batches: Vec::with_capacity(64),
                clip_map: HashMap::with_capacity(64),
                all_verts: Vec::with_capacity(4096),
                vb: None,
                uniform_bg: None,
                last_upload_frame_id: 0,
                staging_buffers: Vec::with_capacity(256),
                target_px: [1, 1],
                ppp: 1.0,
                last_frame_id: 0,
                frame_texts: 0,
                last_seq_id: 0,
            }
        });
        q.target_px = screen_descriptor.size_in_pixels;
        q.ppp = screen_descriptor.pixels_per_point.max(1e-6);

        if fid != q.last_frame_id {
            q.vb = None;
            q.uniform_bg = None;
            q.last_upload_frame_id = 0;
            q.clip_batches.clear();
            q.clip_map.clear();
            q.all_verts.clear();
            q.staging_buffers.clear();
            q.pending_bytes.clear();
            q.pending_uploads.clear();
            q.last_frame_id = fid;
            q.frame_texts = 0;
            q.last_seq_id = 0;
            q.new_glyphs_this_frame = 0;
        }

        // Order-preserving batching:
        // Only batch across *adjacent* primitives in the paint list (seq must be consecutive).
        // If there's anything between (native mesh / other callback), we must break batching.
        if seq != q.last_seq_id.saturating_add(1) {
            q.clip_map.clear();
        }
        q.last_seq_id = seq;
        
        let ppp = screen_descriptor.pixels_per_point.max(1e-6);
        let desired_font_px = (self.uniform.font_px * ppp * gpu_text_scale()) as f32;
        let desired_font_px = if desired_font_px.is_finite() { desired_font_px.max(0.1) } else { 13.0 };
        // Prevent tiny fonts from producing 0-sized glyph bitmaps (which makes whole labels disappear at far zoom).
        // Rasterize at a safe minimum, then scale quads down to the desired size.
        const MIN_RASTER_PX: f32 = 6.0;
        let raster_font_px = desired_font_px.max(MIN_RASTER_PX);
        let draw_scale = (desired_font_px / raster_font_px).clamp(0.01, 1.0);
        let text = self.uniform.text.as_str();
        // log_debug(&format!("Preparing text: '{}'", text)); // reduce spam
        let sans = match gpu_text_sans_family_id() {
            1 => Family::Name("Segoe UI"),
            2 => Family::Name("Microsoft YaHei"),
            3 => Family::Name("SimHei"),
            4 => Family::Name("SimSun"),
            _ => Family::SansSerif,
        };
        let attrs = Attrs::new().family(if self.uniform.family == 1 { Family::Monospace } else { sans });
        // scissor in final pixel space (used for batch key + shader clip discard)
        let (min_x, min_y, w, h) = {
            let tw = q.target_px[0];
            let th = q.target_px[1];
            if tw == 0 || th == 0 { return Vec::new(); }
            let rect = self.rect;
            let min_x = (rect.min.x * ppp).round().max(0.0) as u32;
            let min_y = (rect.min.y * ppp).round().max(0.0) as u32;
            let mut max_x = (rect.max.x * ppp).round().max(0.0) as u32;
            let mut max_y = (rect.max.y * ppp).round().max(0.0) as u32;
            if min_x >= tw || min_y >= th { return Vec::new(); }
            if max_x > tw { max_x = tw; }
            if max_y > th { max_y = th; }
            if max_x <= min_x || max_y <= min_y { return Vec::new(); }
            (min_x, min_y, max_x - min_x, max_y - min_y)
        };
        let clip = [min_x as f32, min_y as f32, (min_x + w) as f32, (min_y + h) as f32];
        
        let glyphs = &mut q.scratch_glyphs;
        glyphs.clear();
        glyphs.reserve(text.len().max(1));
        {
            let mut buf = q.buffer.borrow_with(&mut q.font_system);
            let desired_lh_px = (self.uniform.bounds.y * ppp).max(desired_font_px * gpu_text_line_height_factor());
            let lh = (desired_lh_px / draw_scale).max(raster_font_px * gpu_text_line_height_factor());
            buf.set_metrics(Metrics::new(raster_font_px, lh));
            // Match egui's wrapping/ellipsize behavior by constraining layout width to the allocated bounds.
            // Add slack to avoid edge-case disappearance due to tiny layout diffs at small sizes.
            let bw = (self.uniform.bounds.x * ppp / draw_scale).max(1.0);
            let bw = if bw.is_finite() && bw > 1.0 { Some((bw + 32.0).min(q.target_px[0] as f32)) } else { None };
            buf.set_size(bw, None);
            // PERF: Advanced shaping is expensive. Most UI labels are ASCII; use Basic shaping there.
            // Keep Advanced for CJK/complex scripts.
            buf.set_text(text, &attrs, if text.is_ascii() { Shaping::Basic } else { Shaping::Advanced });
            buf.shape_until_scroll(false);
            for run in buf.layout_runs() {
                let line_y = run.line_y as i32;
                for g in run.glyphs {
                    // CRITICAL PERF FIX:
                    // cosmic-text CacheKey includes subpixel positioning bins.
                    // Collapsing/expanding panels changes text layout positions -> new subpixel bins -> tons of "new glyphs"
                    // -> massive rasterize+upload spikes (visible as UI hitch even on collapse).
                    // Force per-glyph origin such that (x,y) lands on integer pixels (subpixel bins -> 0).
                    let ox = if g.x.is_finite() { g.x.round() - g.x } else { 0.0 };
                    let oy = if g.y.is_finite() { g.y.round() - g.y } else { 0.0 };
                    let p = g.physical((ox, oy), 1.0);
                    glyphs.push((p.cache_key, p.x, p.y, line_y));
                }
            }
        }
        if gpu_text_skip_raster() { log_debug("CUNNING_GPU_TEXT_SKIP_RASTER=1 -> skip raster/upload"); return Vec::new(); }

        // Build vertices
        let verts = &mut q.scratch_verts;
        verts.clear();
        verts.reserve(text.len().max(1) * 6);
        let color = egui::Rgba::from(self.uniform.color).to_array();
        for &(cache_key, gx, gy, line_y) in glyphs.iter() {
            // IMPORTANT: Do NOT include glyph position in the atlas key.
            // Including gx/gy makes every layout shift (e.g. collapsing/expanding settings panels)
            // look like "new glyphs", causing massive re-rasterization + uploads and UI hitches.
            let key = GlyphKey(cache_key);
                let entry = if let Some(e) = q.atlas.glyphs.get(&key) {
                    *e
                } else {
                    // Budget uploads to avoid huge stalls when a lot of new UI appears at once (collapsing/expanding panels).
                    // Remaining glyphs will be uploaded over subsequent frames.
                    let budget = gpu_text_upload_budget();
                    if budget == 0 || q.new_glyphs_this_frame >= budget {
                        continue;
                    }
                    let image = match q.swash_cache.get_image(&mut q.font_system, cache_key) {
                        Some(i) => i.clone(),
                        None => {
                            log_debug(&format!("Failed to rasterize glyph {:?}", cache_key));
                            continue;
                        },
                    };
                    if gpu_text_skip_upload() { log_debug(&format!("CUNNING_GPU_TEXT_SKIP_UPLOAD=1 -> skip upload for {:?}", cache_key)); continue; }

                    // Allocate with 1px padding and upload full padded tile (border alpha=0) to avoid linear sampling bleeding.
                    let w = image.placement.width as u32;
                    let h = image.placement.height as u32;
                    if w == 0 || h == 0 || image.data.len() != (w * h) as usize { continue; }
                    let alloc = match q.atlas.allocator.allocate(etagere::Size::new((w + 2) as i32, (h + 2) as i32)) { Some(a) => a, None => continue };
                    let x0 = alloc.rectangle.min.x as u32;
                    let y0 = alloc.rectangle.min.y as u32;
                    let x = x0 + 1;
                    let y = y0 + 1;
                    if x0.saturating_add(w + 2) > q.atlas.size.0 || y0.saturating_add(h + 2) > q.atlas.size.1 {
                        log_debug(&format!("Atlas OOB: x={} y={} w={} h={} atlas={}x{}", x, y, w, h, q.atlas.size.0, q.atlas.size.1));
                        continue;
                    }
                    queue_r8_texture_padded(&mut q.pending_bytes, &mut q.pending_uploads, &mut q.scratch_upload, x0, y0, w, h, &image.data);
                    q.new_glyphs_this_frame = q.new_glyphs_this_frame.saturating_add(1);
                    // We already allocated/uploaded with a 1px transparent padding border around the glyph.
                    // So we can sample the full glyph rect without half-texel insetting (which can look like
                    // consistent top/bottom "shaving" across sizes).
                    let inv_w = 1.0 / q.atlas.size.0 as f32;
                    let inv_h = 1.0 / q.atlas.size.1 as f32;
                    let uv_min = [x as f32 * inv_w, y as f32 * inv_h];
                    let uv_max = [(x + w) as f32 * inv_w, (y + h) as f32 * inv_h];
                    let entry = AtlasEntry { uv_min, uv_max, left: image.placement.left, top: image.placement.top, w, h };
                    q.atlas.glyphs.insert(key, entry);
                    entry
                };

                let x0_px = (gx + entry.left) as f32 * draw_scale;
                let y0_px = (line_y + gy - entry.top) as f32 * draw_scale;
                // Pixel-snap glyph quads to reduce "soft" sampling blur compared to egui native.
                // (GPUI/Zed does similar pixel alignment.)
                let mut x0 = self.uniform.pos.x + (x0_px / ppp);
                let mut y0 = self.uniform.pos.y + (y0_px / ppp) + gpu_text_y_offset_points();
                let mut x1 = x0 + ((entry.w as f32 * draw_scale) / ppp);
                let mut y1 = y0 + ((entry.h as f32 * draw_scale) / ppp);
                x0 = snap_to_pixel(x0, ppp);
                y0 = snap_to_pixel(y0, ppp);
                x1 = snap_to_pixel(x1, ppp);
                y1 = snap_to_pixel(y1, ppp);

                let u0 = entry.uv_min[0];
                let v0 = entry.uv_min[1];
                let u1 = entry.uv_max[0];
                let v1 = entry.uv_max[1];

                verts.push(TextVertex { pos: [x0, y0], uv: [u0, v0], color, clip });
                verts.push(TextVertex { pos: [x1, y0], uv: [u1, v0], color, clip });
                verts.push(TextVertex { pos: [x1, y1], uv: [u1, v1], color, clip });
                verts.push(TextVertex { pos: [x0, y0], uv: [u0, v0], color, clip });
                verts.push(TextVertex { pos: [x1, y1], uv: [u1, v1], color, clip });
                verts.push(TextVertex { pos: [x0, y1], uv: [u0, v1], color, clip });
        }

        // Keep glyph quads inside the row bounds to avoid "next row covers previous row" artifacts (looks like bottom is being masked).
        // This happens when our baseline/metrics differ slightly from egui's row rect.
        if gpu_text_row_bounds_clamp() && !verts.is_empty() && self.uniform.bounds.y.is_finite() && self.uniform.bounds.y > 0.0 {
            let desired_min = self.uniform.pos.y;
            let desired_max = self.uniform.pos.y + self.uniform.bounds.y;
            let mut min_y = f32::INFINITY;
            let mut max_y = f32::NEG_INFINITY;
            for v in verts.iter() { min_y = min_y.min(v.pos[1]); max_y = max_y.max(v.pos[1]); }
            let up = desired_min - min_y;
            let down = desired_max - max_y;
            let shift = if up > 0.0 && down < 0.0 { 0.5 * (up + down) } else if down < 0.0 { down } else if up > 0.0 { up } else { 0.0 };
            if shift != 0.0 {
                for v in verts.iter_mut() { v.pos[1] += shift; }
            }
        }

        if verts.is_empty() { return Vec::new(); }
        q.frame_texts = q.frame_texts.saturating_add(1);
        
        let sc = [min_x, min_y, w, h];
        let k = ((sc[0] as u64) << 48) ^ ((sc[1] as u64) << 32) ^ ((sc[2] as u64) << 16) ^ (sc[3] as u64);
        let bi = *q.clip_map.entry(k).or_insert_with(|| {
            let bi = q.clip_batches.len();
            q.clip_batches.push(TextClipBatch {
                scissor: sc,
                verts: Vec::with_capacity(512),
                start: 0,
                count: 0,
                leader: true,
                callbacks: 0,
            });
            bi
        });
        let b = &mut q.clip_batches[bi];
        let is_leader = b.leader;
        b.leader = false;
        b.callbacks = b.callbacks.saturating_add(1);
        b.verts.extend_from_slice(verts);
        self.key.store(pack_key(bi, is_leader), Ordering::Relaxed);
        Vec::new()
    }

    fn finish_prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut crate::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let fid = resources.get::<super::SdfFrameId>().map(|v| v.0).unwrap_or(self.frame_id);
        let Some(q) = resources.get_mut::<GpuTextQueue>() else { return Vec::new(); };
        if fid == q.last_upload_frame_id { return Vec::new(); }
        q.last_upload_frame_id = fid;

        // GPUI-style batching: merge all new glyph uploads into a single staging buffer per frame.
        if !q.pending_uploads.is_empty() && !q.pending_bytes.is_empty() {
            let staging = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Cunning GPU Text Atlas Staging (batched)"),
                contents: &q.pending_bytes,
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            for u in q.pending_uploads.drain(..) {
                egui_encoder.copy_buffer_to_texture(
                    wgpu::TexelCopyBufferInfo {
                        buffer: &staging,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: u.offset,
                            bytes_per_row: Some(u.bytes_per_row),
                            rows_per_image: Some(u.h),
                        },
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &q.atlas.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x: u.x, y: u.y, z: 0 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d { width: u.w, height: u.h, depth_or_array_layers: 1 },
                );
            }
            q.staging_buffers.push(staging);
            q.pending_bytes.clear();
        } else {
            q.pending_uploads.clear();
            q.pending_bytes.clear();
        }

        let total: usize = q.clip_batches.iter().map(|b| b.verts.len()).sum();
        if total == 0 { q.vb = None; q.uniform_bg = None; GPU_TEXT_LAST_FRAME.store(fid, Ordering::Relaxed); GPU_TEXT_LAST_TEXTS.store(q.frame_texts, Ordering::Relaxed); GPU_TEXT_LAST_CLIP_REGIONS.store(0, Ordering::Relaxed); GPU_TEXT_LAST_DRAWCALLS.store(0, Ordering::Relaxed); GPU_TEXT_LAST_VERTS.store(0, Ordering::Relaxed); return Vec::new(); }
        let uniform = TextUniform {
            screen_size: [q.target_px[0] as f32 / q.ppp, q.target_px[1] as f32 / q.ppp],
            _pad0: [0.0; 2],
            gamma_ratios: gamma_ratios_from_gamma(gpu_text_gamma()),
            grayscale_enhanced_contrast: gpu_text_grayscale_enhanced_contrast(),
            _pad_align: [0.0; 3],
            _pad1: [0.0; 3],
            content_weight: gpu_text_content_weight(),
        };
        let ub = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Cunning GPU Text UB (batched)"), contents: bytemuck::bytes_of(&uniform), usage: wgpu::BufferUsages::UNIFORM });
        q.uniform_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Cunning GPU Text Uniform BG (batched)"),
            layout: &q.renderer.uniform_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() }],
        }));
        q.all_verts.clear();
        q.all_verts.reserve(total);
        let mut cursor = 0u32;
        for b in &mut q.clip_batches {
            b.start = cursor;
            b.count = b.verts.len() as u32;
            cursor = cursor.saturating_add(b.count);
            q.all_verts.extend_from_slice(&b.verts);
        }
        q.vb = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Cunning GPU Text VB (batched)"), contents: bytemuck::cast_slice(&q.all_verts), usage: wgpu::BufferUsages::VERTEX }));
        GPU_TEXT_LAST_FRAME.store(fid, Ordering::Relaxed);
        GPU_TEXT_LAST_TEXTS.store(q.frame_texts, Ordering::Relaxed);
        let batches = q.clip_batches.iter().filter(|b| b.count > 0).count() as u64;
        GPU_TEXT_LAST_CLIP_REGIONS.store(batches, Ordering::Relaxed);
        GPU_TEXT_LAST_DRAWCALLS.store(batches, Ordering::Relaxed);
        GPU_TEXT_LAST_VERTS.store(total as u64, Ordering::Relaxed);
        if GPU_TEXT_VERBOSE_DETAILS.load(Ordering::Relaxed) {
            let mut out: Vec<GpuTextClipBatchStat> = Vec::with_capacity(q.clip_batches.len());
            for b in q.clip_batches.iter().filter(|b| b.count > 0) {
                let verts = b.count;
                out.push(GpuTextClipBatchStat { scissor: b.scissor, callbacks: b.callbacks, glyphs: verts / 6, verts });
            }
            out.sort_by_key(|b| (b.scissor[0], b.scissor[1], b.scissor[2], b.scissor[3]));
            *GPU_TEXT_LAST_DETAILS.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap() = out;
            GPU_TEXT_LAST_DETAILS_FRAME.store(fid, Ordering::Relaxed);
        }
        Vec::new()
    }

    fn paint<'a>(&'a self, info: egui::PaintCallbackInfo, render_pass: &mut wgpu::RenderPass<'a>, resources: &'a crate::CallbackResources) {
        if let Some(q) = resources.get::<GpuTextQueue>() {
            if let Some((bi, is_leader)) = unpack_key(self.key.load(Ordering::Relaxed)) {
                if !is_leader { return; }
                let Some(b) = q.clip_batches.get(bi) else { return; };
                if b.count == 0 { return; }
                let Some(bg0) = q.uniform_bg.as_ref() else { return; };
                let Some(vb) = q.vb.as_ref() else { return; };
                
                let tw = q.target_px[0];
                let th = q.target_px[1];
                if tw == 0 || th == 0 { return; }
                // egui-wgpu sets viewport to callback.rect as a courtesy; our shaders use full-screen coordinates.
                // Force full-screen viewport so dragging/scrolling/pan never desyncs text position.
                render_pass.set_viewport(0.0, 0.0, tw as f32, th as f32, 0.0, 1.0);
                let Some((min_x, min_y, w0, h0)) = super::clamp_scissor(&info, q.target_px) else { return; };
                // Prevent top/bottom shaving due to tight clip_rect vs glyph raster bounds (TextEdit/code blocks are the worst).
                // Expand vertically by a couple pixels while staying within framebuffer.
                let pad_y: u32 = gpu_text_scissor_pad_y();
                let max_x = (min_x.saturating_add(w0)).min(tw);
                let max_y0 = min_y.saturating_add(h0);
                let min_y = min_y.saturating_sub(pad_y);
                let max_y = (max_y0.saturating_add(pad_y)).min(th);
                let w = max_x.saturating_sub(min_x);
                let h = max_y.saturating_sub(min_y);
                if w == 0 || h == 0 { return; }

                render_pass.set_scissor_rect(min_x, min_y, w, h);
                render_pass.set_pipeline(&q.renderer.pipeline);
                render_pass.set_bind_group(0, bg0, &[]);
                render_pass.set_bind_group(1, &q.atlas.bind_group, &[]);
                let stride = std::mem::size_of::<TextVertex>() as u64;
                let off0 = b.start as u64 * stride;
                let off1 = (b.start as u64 + b.count as u64) * stride;
                render_pass.set_vertex_buffer(0, vb.slice(off0..off1));
                render_pass.draw(0..b.count, 0..1);
            }
        }
    }
}

pub fn create_gpu_text_callback(rect: egui::Rect, uniform: GpuTextUniform, frame_id: u64) -> egui::PaintCallback {
    crate::Callback::new_paint_callback(rect, GpuTextCallback { rect, uniform, frame_id, key: Arc::new(AtomicU64::new(0)) })
}

