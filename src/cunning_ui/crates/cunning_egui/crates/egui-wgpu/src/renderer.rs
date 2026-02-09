#![allow(unsafe_code)]

use std::{borrow::Cow, num::NonZeroU64, ops::Range, sync::OnceLock};

use epaint::{ahash::HashMap, emath::NumExt, PaintCallbackInfo, Primitive, Vertex};
use cunning_wgpu_ui::sdf::circle::{SdfCircleRenderer, SdfCircleUniform};
use cunning_wgpu_ui::sdf::curve::{SdfCurveRenderer, SdfCurveUniform};
use cunning_wgpu_ui::sdf::ellipse::{SdfEllipseRenderer, SdfEllipseUniform};
use cunning_wgpu_ui::sdf::rect::{SdfRectInstance, SdfRectRenderer};

use wgpu::util::DeviceExt as _;

// ----------------------------------------------------------------------------
// Mesh -> SDF rewrite (library-level "no pain" path)
// ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutoSdfKind {
    Rect,
    Circle,
    Curve,
}

#[derive(Clone, Copy, Debug)]
enum AutoSdf {
    Rect(SdfRectInstance),
    Circle(SdfCircleUniform),
    Curve(SdfCurveUniform),
}

#[inline]
fn rgba32(c: epaint::Color32) -> [f32; 4] {
    epaint::Rgba::from(c).to_array()
}

#[inline]
fn mesh_uniform_color(mesh: &epaint::Mesh) -> Option<epaint::Color32> {
    let c0 = mesh.vertices.first()?.color;
    if mesh.vertices.iter().all(|v| v.color == c0) { Some(c0) } else { None }
}

#[inline]
fn mesh_uv_constant(mesh: &epaint::Mesh) -> bool {
    let uv0 = match mesh.vertices.first() { Some(v) => v.uv, None => return false };
    let eps = 1e-4;
    mesh.vertices.iter().all(|v| (v.uv.x - uv0.x).abs() <= eps && (v.uv.y - uv0.y).abs() <= eps)
}

fn try_mesh_as_rect(mesh: &epaint::Mesh) -> Option<epaint::Rect> {
    if mesh.vertices.len() != 4 || mesh.indices.len() != 6 { return None; }
    if !mesh_uv_constant(mesh) { return None; }
    let xs: Vec<f32> = mesh.vertices.iter().map(|v| v.pos.x).collect();
    let ys: Vec<f32> = mesh.vertices.iter().map(|v| v.pos.y).collect();
    let min_x = xs.iter().copied().fold(f32::INFINITY, f32::min);
    let max_x = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let min_y = ys.iter().copied().fold(f32::INFINITY, f32::min);
    let max_y = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if !(max_x > min_x && max_y > min_y) { return None; }
    let eps = 0.01;
    // verify corners (axis-aligned)
    for v in &mesh.vertices {
        let ok_x = (v.pos.x - min_x).abs() <= eps || (v.pos.x - max_x).abs() <= eps;
        let ok_y = (v.pos.y - min_y).abs() <= eps || (v.pos.y - max_y).abs() <= eps;
        if !(ok_x && ok_y) { return None; }
    }
    Some(epaint::Rect::from_min_max(epaint::pos2(min_x, min_y), epaint::pos2(max_x, max_y)))
}

fn try_mesh_as_circle(mesh: &epaint::Mesh) -> Option<(epaint::Pos2, f32)> {
    let n = mesh.vertices.len();
    if n < 12 || n > 256 { return None; }
    if !mesh_uv_constant(mesh) { return None; }
    // Approx center by average
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    for v in &mesh.vertices { cx += v.pos.x; cy += v.pos.y; }
    cx /= n as f32;
    cy /= n as f32;
    let c = epaint::pos2(cx, cy);
    let mut rs: Vec<f32> = Vec::with_capacity(n);
    for v in &mesh.vertices { rs.push(v.pos.distance(c)); }
    let r_mean = rs.iter().copied().sum::<f32>() / n as f32;
    if r_mean <= 0.5 { return None; }
    let var = rs.iter().map(|r| (r - r_mean) * (r - r_mean)).sum::<f32>() / n as f32;
    let std = var.sqrt();
    if std / r_mean > 0.05 { return None; }
    Some((c, r_mean))
}

fn try_mesh_as_line_curve(mesh: &epaint::Mesh) -> Option<(epaint::Pos2, epaint::Pos2, f32)> {
    if mesh.vertices.len() != 4 || mesh.indices.len() != 6 { return None; }
    if !mesh_uv_constant(mesh) { return None; }
    // PCA for direction
    let pts: Vec<epaint::Pos2> = mesh.vertices.iter().map(|v| v.pos).collect();
    let mut mx = 0.0f32;
    let mut my = 0.0f32;
    for p in &pts { mx += p.x; my += p.y; }
    mx /= 4.0; my /= 4.0;
    let mut cxx = 0.0f32; let mut cyy = 0.0f32; let mut cxy = 0.0f32;
    for p in &pts {
        let dx = p.x - mx; let dy = p.y - my;
        cxx += dx * dx; cyy += dy * dy; cxy += dx * dy;
    }
    let ang = 0.5 * (2.0 * cxy).atan2(cxx - cyy);
    let ux = ang.cos(); let uy = ang.sin();
    let v = epaint::vec2(-uy, ux);
    let mut proj_u: Vec<(f32, epaint::Pos2)> = pts.iter().map(|p| ((p.x - mx) * ux + (p.y - my) * uy, *p)).collect();
    proj_u.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let min_u = proj_u.first()?.0;
    let max_u = proj_u.last()?.0;
    if (max_u - min_u).abs() < 1.0 { return None; }
    let eps = 0.05 * (max_u - min_u).abs();
    let mut a_pts: Vec<epaint::Pos2> = Vec::new();
    let mut b_pts: Vec<epaint::Pos2> = Vec::new();
    for (t, p) in proj_u {
        if (t - min_u).abs() <= eps { a_pts.push(p); }
        if (t - max_u).abs() <= eps { b_pts.push(p); }
    }
    if a_pts.is_empty() || b_pts.is_empty() { return None; }
    let mid = |ps: &[epaint::Pos2]| {
        let mut x = 0.0; let mut y = 0.0;
        for p in ps { x += p.x; y += p.y; }
        epaint::pos2(x / ps.len() as f32, y / ps.len() as f32)
    };
    let p0 = mid(&a_pts);
    let p1 = mid(&b_pts);
    // thickness from projection onto normal
    let mut min_v = f32::INFINITY;
    let mut max_v = f32::NEG_INFINITY;
    for p in &pts {
        let dv = (p.x - mx) * v.x + (p.y - my) * v.y;
        min_v = min_v.min(dv);
        max_v = max_v.max(dv);
    }
    let thickness = (max_v - min_v).abs().max(1.0);
    Some((p0, p1, thickness))
}

fn auto_sdf_from_mesh(mesh: &epaint::Mesh, screen_size_in_points: [f32; 2]) -> Option<(AutoSdfKind, AutoSdf)> {
    // Only rewrite "solid-color UI shapes" that sample the default egui texture (white pixel).
    // Never rewrite user textures (icons/images), otherwise logos can disappear.
    match mesh.texture_id {
        epaint::TextureId::Managed(0) => {}
        _ => return None,
    }
    let color = mesh_uniform_color(mesh)?;
    // only rewrite solid-color UI shapes
    if !mesh_uv_constant(mesh) { return None; }
    if let Some(r) = try_mesh_as_rect(mesh) {
        let c = r.center();
        let s = r.size();
        return Some((AutoSdfKind::Rect, AutoSdf::Rect(SdfRectInstance {
            center: [c.x, c.y],
            half_size: [s.x * 0.5, s.y * 0.5],
            corner_radii: [0.0; 4],
            fill_color: rgba32(color),
            shadow_color: [0.0; 4],
            shadow_params: [0.0; 4],
            border_params: [0.0; 4],
            border_color: [0.0; 4],
            clip_min: [0.0, 0.0],
            clip_max: screen_size_in_points,
        })));
    }
    if let Some((center, radius)) = try_mesh_as_circle(mesh) {
        return Some((AutoSdfKind::Circle, AutoSdf::Circle(SdfCircleUniform {
            center: [center.x, center.y],
            radius,
            border_width: 0.0,
            fill_color: rgba32(color),
            border_color: rgba32(color),
            softness: 1.0,
            _pad0: 0.0,
            screen_size: screen_size_in_points,
            _pad1: [0.0; 2],
            _pad2: [0.0; 2],
        })));
    }
    if let Some((p0, p3, thickness)) = try_mesh_as_line_curve(mesh) {
        let d = p3 - p0;
        let p1 = p0 + d / 3.0;
        let p2 = p0 + d * 2.0 / 3.0;
        return Some((AutoSdfKind::Curve, AutoSdf::Curve(SdfCurveUniform {
            p0: [p0.x, p0.y],
            p1: [p1.x, p1.y],
            p2: [p2.x, p2.y],
            p3: [p3.x, p3.y],
            color: rgba32(color),
            thickness,
            softness: 1.0,
            screen_size: screen_size_in_points,
            _pad: [0.0; 2],
        })));
    }
    None
}

pub use cunning_wgpu_ui::{Callback, CallbackResources, CallbackTrait, ScreenDescriptor, TargetFormat};

fn screen_size_in_points(sd: &ScreenDescriptor) -> [f32; 2] {
    [sd.size_in_pixels[0] as f32 / sd.pixels_per_point, sd.size_in_pixels[1] as f32 / sd.pixels_per_point]
}

/// Uniform buffer used when rendering.
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct UniformBuffer {
    screen_size_in_points: [f32; 2],
    dithering: u32,
    predictable_texture_filtering: u32,
}

#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct TexMeta {
    kind: u32, // 0: regular, 1: SDF (reserved)
    _pad0: [u32; 3], // pad to 16 bytes
    _pad1: [u32; 4], // matches WGSL vec3<u32> rounded up to 16 bytes (total struct = 32 bytes)
}

static EGUI_FONT_SDF_SPREAD: OnceLock<f32> = OnceLock::new();
fn egui_font_sdf_spread_px() -> f32 {
    *EGUI_FONT_SDF_SPREAD.get_or_init(|| std::env::var("CUNNING_EGUI_FONT_SDF_SPREAD").ok().and_then(|v| v.parse().ok()).unwrap_or(8.0f32).max(1.0f32))
}

#[derive(Clone)]
struct FontAtlasCache {
    w: u32,
    h: u32,
    alpha: Vec<f32>,
}

impl PartialEq for UniformBuffer {
    fn eq(&self, other: &Self) -> bool {
        self.screen_size_in_points == other.screen_size_in_points
            && self.dithering == other.dithering
            && self.predictable_texture_filtering == other.predictable_texture_filtering
    }
}

pub struct GpuTexture {
    pub texture: Option<wgpu::Texture>,
    pub bind_group: wgpu::BindGroup,
    pub(crate) _meta_buffer: wgpu::Buffer, // keep alive for bind group
    pub width: u32,
    pub height: u32,
}

struct SlicedBuffer {
    buffer: wgpu::Buffer,
    slices: Vec<Range<usize>>,
    capacity: wgpu::BufferAddress,
}

#[derive(Clone, Copy, Debug)]
pub struct RendererOptions {
    pub msaa_samples: u32,
    pub depth_stencil_format: Option<wgpu::TextureFormat>,
    pub dithering: bool,
    pub predictable_texture_filtering: bool,
}

impl RendererOptions {
    pub const PREDICTABLE: Self = Self {
        msaa_samples: 1,
        depth_stencil_format: None,
        dithering: false,
        predictable_texture_filtering: true,
    };
}

impl Default for RendererOptions {
    fn default() -> Self {
        Self {
            msaa_samples: 0,
            depth_stencil_format: None,
            dithering: true,
            predictable_texture_filtering: false,
        }
    }
}

/// Renderer for a egui based GUI.
pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    sdf_rect: SdfRectRenderer,
    sdf_rect_bind_groups: Vec<wgpu::BindGroup>,
    sdf_rect_counts: Vec<u32>,
    sdf_rect_indices: Vec<Option<usize>>,
    sdf_circle: SdfCircleRenderer,
    sdf_circle_bind_groups: Vec<wgpu::BindGroup>,
    sdf_circle_counts: Vec<u32>,
    sdf_circle_indices: Vec<Option<usize>>,
    sdf_ellipse: SdfEllipseRenderer,
    sdf_ellipse_bind_groups: Vec<wgpu::BindGroup>,
    sdf_ellipse_counts: Vec<u32>,
    sdf_ellipse_indices: Vec<Option<usize>>,
    sdf_curve: SdfCurveRenderer,
    sdf_curve_bind_groups: Vec<wgpu::BindGroup>,
    sdf_curve_counts: Vec<u32>,
    sdf_curve_indices: Vec<Option<usize>>,
    // When true for a given paint job index, we skip drawing the Mesh and instead draw a SDF batch (leader-only).
    auto_sdf_skip_mesh: Vec<bool>,

    index_buffer: SlicedBuffer,
    vertex_buffer: SlicedBuffer,

    uniform_buffer: wgpu::Buffer,
    previous_uniform_buffer_content: UniformBuffer,
    uniform_bind_group: wgpu::BindGroup,
    texture_bind_group_layout: wgpu::BindGroupLayout,

    /// Map of egui texture IDs to textures and their associated bindgroups (texture view +
    /// sampler). The texture may be None if the TextureId is just a handle to a user-provided
    /// sampler.
    textures: HashMap<epaint::TextureId, GpuTexture>,
    next_user_texture_id: u64,
    samplers: HashMap<epaint::textures::TextureOptions, wgpu::Sampler>,
    options: RendererOptions,
    staging_keepalive: Vec<wgpu::Buffer>,
    font_atlas_cache: Option<FontAtlasCache>,
    font_sdf_dirty: bool,

    /// Storage for resources shared with all invocations of [`CallbackTrait`]'s methods.
    ///
    /// See also [`CallbackTrait`].
    pub callback_resources: CallbackResources,
    sdf_ui_frame: u64,
}

impl Renderer {
    /// Creates a renderer for a egui UI.
    ///
    /// `output_color_format` should preferably be [`wgpu::TextureFormat::Rgba8Unorm`] or
    /// [`wgpu::TextureFormat::Bgra8Unorm`], i.e. in gamma-space.
    pub fn new(
        device: &wgpu::Device,
        output_color_format: wgpu::TextureFormat,
        options: RendererOptions,
    ) -> Self {
        crate::profile_function!();

        let shader = wgpu::ShaderModuleDescriptor {
            label: Some("egui"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("egui.wgsl"))),
        };
        let module = {
            crate::profile_scope!("create_shader_module");
            device.create_shader_module(shader)
        };

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("egui_uniform_buffer"),
            contents: bytemuck::cast_slice(&[UniformBuffer {
                screen_size_in_points: [0.0, 0.0],
                dithering: u32::from(options.dithering),
                predictable_texture_filtering: u32::from(options.predictable_texture_filtering),
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_bind_group_layout = {
            crate::profile_scope!("create_bind_group_layout");
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("egui_uniform_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(std::mem::size_of::<UniformBuffer>() as _),
                        ty: wgpu::BufferBindingType::Uniform,
                    },
                    count: None,
                }],
            })
        };

        let uniform_bind_group = {
            crate::profile_scope!("create_bind_group");
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("egui_uniform_bind_group"),
                layout: &uniform_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniform_buffer,
                        offset: 0,
                        size: None,
                    }),
                }],
            })
        };

        let texture_bind_group_layout = {
            crate::profile_scope!("create_bind_group_layout");
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("egui_texture_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(std::mem::size_of::<TexMeta>() as _),
                            ty: wgpu::BufferBindingType::Uniform,
                        },
                        count: None,
                    },
                ],
            })
        };

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("egui_pipeline_layout"),
            bind_group_layouts: &[&uniform_bind_group_layout, &texture_bind_group_layout],
            immediate_size: 0,
        });

        let depth_stencil = options.depth_stencil_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::Always,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });

        let pipeline = {
            crate::profile_scope!("create_render_pipeline");
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("egui_pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    entry_point: Some("vs_main"),
                    module: &module,
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: 5 * 4,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        // 0: vec2 position
                        // 1: vec2 texture coordinates
                        // 2: uint color
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32],
                    }],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    unclipped_depth: false,
                    conservative: false,
                    cull_mode: None,
                    front_face: wgpu::FrontFace::default(),
                    polygon_mode: wgpu::PolygonMode::default(),
                    strip_index_format: None,
                },
                depth_stencil,
                multisample: wgpu::MultisampleState {
                    alpha_to_coverage_enabled: false,
                    count: options.msaa_samples.max(1),
                    mask: !0,
                },

                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: if output_color_format.is_srgb() {
                        log::warn!("Detected a linear (sRGBA aware) framebuffer {:?}. egui prefers Rgba8Unorm or Bgra8Unorm", output_color_format);
                        Some("fs_main_linear_framebuffer")
                    } else {
                        Some("fs_main_gamma_framebuffer") // this is what we prefer
                    },
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: output_color_format,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            }
        )
        };

        const VERTEX_BUFFER_START_CAPACITY: wgpu::BufferAddress =
            (std::mem::size_of::<Vertex>() * 1024) as _;
        const INDEX_BUFFER_START_CAPACITY: wgpu::BufferAddress =
            (std::mem::size_of::<u32>() * 1024 * 3) as _;

        let mut callback_resources = CallbackResources::default();
        callback_resources.entry::<TargetFormat>().or_insert_with(|| TargetFormat(output_color_format));

        Self {
            pipeline,
            sdf_rect: SdfRectRenderer::new(device, output_color_format),
            sdf_rect_bind_groups: Vec::with_capacity(256),
            sdf_rect_counts: Vec::with_capacity(256),
            sdf_rect_indices: Vec::with_capacity(2048),
            sdf_circle: SdfCircleRenderer::new(device, output_color_format),
            sdf_circle_bind_groups: Vec::with_capacity(256),
            sdf_circle_counts: Vec::with_capacity(256),
            sdf_circle_indices: Vec::with_capacity(2048),
            sdf_ellipse: SdfEllipseRenderer::new(device, output_color_format),
            sdf_ellipse_bind_groups: Vec::with_capacity(256),
            sdf_ellipse_counts: Vec::with_capacity(256),
            sdf_ellipse_indices: Vec::with_capacity(2048),
            sdf_curve: SdfCurveRenderer::new(device, output_color_format),
            sdf_curve_bind_groups: Vec::with_capacity(256),
            sdf_curve_counts: Vec::with_capacity(256),
            sdf_curve_indices: Vec::with_capacity(2048),
            auto_sdf_skip_mesh: Vec::with_capacity(2048),
            vertex_buffer: SlicedBuffer {
                buffer: create_vertex_buffer(device, VERTEX_BUFFER_START_CAPACITY),
                slices: Vec::with_capacity(64),
                capacity: VERTEX_BUFFER_START_CAPACITY,
            },
            index_buffer: SlicedBuffer {
                buffer: create_index_buffer(device, INDEX_BUFFER_START_CAPACITY),
                slices: Vec::with_capacity(64),
                capacity: INDEX_BUFFER_START_CAPACITY,
            },
            uniform_buffer,
            // Buffers on wgpu are zero initialized, so this is indeed its current state!
            previous_uniform_buffer_content: UniformBuffer {
                screen_size_in_points: [0.0, 0.0],
                dithering: u32::from(options.dithering),
                predictable_texture_filtering: u32::from(options.predictable_texture_filtering),
            },
            uniform_bind_group,
            texture_bind_group_layout,
            textures: HashMap::default(),
            next_user_texture_id: 0,
            samplers: HashMap::default(),
            options,
            staging_keepalive: Vec::new(),
            font_atlas_cache: None,
            font_sdf_dirty: false,
            callback_resources,
            sdf_ui_frame: 0,
        }
    }

    pub fn begin_frame(&mut self) {
        self.staging_keepalive.clear();
    }

    #[inline]
    fn mark_font_sdf_dirty(&mut self) { self.font_sdf_dirty = true; }

    /// Rebuild and upload the SDF font atlas at most once per frame.
    /// This avoids doing full-atlas CPU SDF generation for each incremental glyph update,
    /// which can otherwise cause startup-time stalls/crashes (esp. when loading CJK fonts).
    pub fn flush_font_sdf(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder) {
        if !self.font_sdf_dirty { return; }
        self.font_sdf_dirty = false;
        let id = epaint::TextureId::default();
        let Some(cache) = self.font_atlas_cache.as_ref() else { return; };
        let Some(t) = self.textures.get(&id) else { return; };
        let Some(tex) = t.texture.as_ref() else { return; };
        if t.width == 0 || t.height == 0 { return; }
        log::info!("egui-wgpu: flush_font_sdf atlas={}x{} alpha_len={}", t.width, t.height, cache.alpha.len());
        crate::profile_scope!("font -> sdf (flush)");
        let rgba = Self::font_sdf_rgba(&cache.alpha, t.width, t.height, egui_font_sdf_spread_px());
        let staging = Self::copy_rgba_texture(device, encoder, tex, wgpu::Origin3d::ZERO, t.width, t.height, &rgba);
        queue.write_buffer(&t._meta_buffer, 0, bytemuck::cast_slice(&[TexMeta { kind: 1, _pad0: [0; 3], _pad1: [0; 4] }]));
        self.staging_keepalive.push(staging);
    }

    fn edt_1d(f: &[f32], n: usize, d: &mut [f32]) {
        let mut v = vec![0usize; n];
        let mut z = vec![0f32; n + 1];
        let mut k: isize = 0;
        v[0] = 0;
        z[0] = f32::NEG_INFINITY;
        z[1] = f32::INFINITY;
        for q in 1..n {
            let fq = f[q] + (q * q) as f32;
            let mut s = (fq - (f[v[k as usize]] + (v[k as usize] * v[k as usize]) as f32)) / (2.0 * (q as f32 - v[k as usize] as f32));
            while s <= z[k as usize] {
                k -= 1;
                s = (fq - (f[v[k as usize]] + (v[k as usize] * v[k as usize]) as f32)) / (2.0 * (q as f32 - v[k as usize] as f32));
            }
            k += 1;
            v[k as usize] = q;
            z[k as usize] = s;
            z[k as usize + 1] = f32::INFINITY;
        }
        let mut k2: usize = 0;
        for q in 0..n {
            while z[k2 + 1] < q as f32 { k2 += 1; }
            let dx = q as f32 - v[k2] as f32;
            d[q] = dx * dx + f[v[k2]];
        }
    }

    fn edt_2d(feature: &[u8], w: usize, h: usize) -> Vec<f32> {
        let inf = 1.0e20;
        let mut tmp = vec![0f32; w * h];
        let mut out = vec![0f32; w * h];
        let mut f = vec![0f32; w.max(h)];
        let mut d = vec![0f32; w.max(h)];
        for y in 0..h {
            for x in 0..w { f[x] = if feature[y * w + x] != 0 { 0.0 } else { inf }; }
            Self::edt_1d(&f[..w], w, &mut d[..w]);
            for x in 0..w { tmp[y * w + x] = d[x]; }
        }
        for x in 0..w {
            for y in 0..h { f[y] = tmp[y * w + x]; }
            Self::edt_1d(&f[..h], h, &mut d[..h]);
            for y in 0..h { out[y * w + x] = d[y]; }
        }
        out
    }

    fn font_sdf_rgba(alpha: &[f32], w: u32, h: u32, spread: f32) -> Vec<u8> {
        let (wz, hz) = (w as usize, h as usize);
        let mut inside = vec![0u8; wz * hz];
        let mut outside = vec![0u8; wz * hz];
        for i in 0..inside.len() {
            let m = (alpha[i] > 0.5) as u8;
            inside[i] = m;
            outside[i] = 1 - m;
        }
        let dt_inside = Self::edt_2d(&inside, wz, hz);
        let dt_outside = Self::edt_2d(&outside, wz, hz);
        let inv = 1.0 / (2.0 * spread);
        let mut rgba = vec![0u8; wz * hz * 4];
        for i in 0..inside.len() {
            let sd = if inside[i] != 0 { dt_outside[i].sqrt() } else { -dt_inside[i].sqrt() };
            let v = (0.5 + sd * inv).clamp(0.0, 1.0);
            let a = (v * 255.0).round() as u8;
            let o = i * 4;
            rgba[o] = 255;
            rgba[o + 1] = 255;
            rgba[o + 2] = 255;
            rgba[o + 3] = a;
        }
        rgba
    }

    fn copy_rgba_texture(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        origin: wgpu::Origin3d,
        w: u32,
        h: u32,
        rgba: &[u8],
    ) -> wgpu::Buffer {
        let unpadded_bpr = 4 * w;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bpr = ((unpadded_bpr + align - 1) / align) * align;
        let mut padded = vec![0u8; (padded_bpr * h) as usize];
        for row in 0..h as usize {
            let src0 = row * unpadded_bpr as usize;
            let dst0 = row * padded_bpr as usize;
            padded[dst0..dst0 + unpadded_bpr as usize].copy_from_slice(&rgba[src0..src0 + unpadded_bpr as usize]);
        }
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("egui_tex_staging"),
            size: padded.len() as u64,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::MAP_WRITE,
            mapped_at_creation: true,
        });
        staging.slice(..).get_mapped_range_mut().copy_from_slice(&padded);
        staging.unmap();
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bpr),
                    rows_per_image: Some(h),
                },
            },
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        staging
    }

    /// Executes the egui renderer onto an existing wgpu renderpass.
    pub fn render<'rp>(
        &'rp self,
        render_pass: &mut wgpu::RenderPass<'rp>,
        paint_jobs: &'rp [epaint::ClippedPrimitive],
        screen_descriptor: &ScreenDescriptor,
    ) {
        crate::profile_function!();

        let pixels_per_point = screen_descriptor.pixels_per_point;
        let size_in_pixels = screen_descriptor.size_in_pixels;

        // Whether or not we need to reset the render pass because a paint callback has just
        // run.
        let mut needs_reset = true;

        let mut index_buffer_slices = self.index_buffer.slices.iter();
        let mut vertex_buffer_slices = self.vertex_buffer.slices.iter();

        for (job_i, epaint::ClippedPrimitive { clip_rect, primitive }) in paint_jobs.iter().enumerate() {
            if needs_reset {
                render_pass.set_viewport(
                    0.0,
                    0.0,
                    size_in_pixels[0] as f32,
                    size_in_pixels[1] as f32,
                    0.0,
                    1.0,
                );
                render_pass.set_pipeline(&self.pipeline);
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                needs_reset = false;
            }

            {
                let rect = ScissorRect::new(clip_rect, pixels_per_point, size_in_pixels);

                if rect.width == 0 || rect.height == 0 {
                    // Skip rendering zero-sized clip areas.
                    if let Primitive::Mesh(_) = primitive {
                        // If this is a mesh, we need to advance the index and vertex buffer iterators:
                        index_buffer_slices.next().unwrap();
                        vertex_buffer_slices.next().unwrap();
                    }
                    continue;
                }

                render_pass.set_scissor_rect(rect.x, rect.y, rect.width, rect.height);
            }

            match primitive {
                Primitive::Mesh(mesh) => {
                    let index_buffer_slice = index_buffer_slices.next().unwrap();
                    let vertex_buffer_slice = vertex_buffer_slices.next().unwrap();

                    // If this mesh was auto-rewritten to SDF, skip drawing the mesh.
                    if self.auto_sdf_skip_mesh.get(job_i).copied().unwrap_or(false) {
                        if let Some(bg_i) = self.sdf_rect_indices.get(job_i).copied().flatten() {
                            if let Some(bg) = self.sdf_rect_bind_groups.get(bg_i) {
                                let inst_n = *self.sdf_rect_counts.get(bg_i).unwrap_or(&1);
                                render_pass.set_pipeline(&self.sdf_rect.pipeline);
                                render_pass.set_bind_group(0, bg, &[]);
                                render_pass.draw(0..6, 0..inst_n);
                                needs_reset = true;
                            }
                        } else if let Some(bg_i) = self.sdf_circle_indices.get(job_i).copied().flatten() {
                            if let Some(bg) = self.sdf_circle_bind_groups.get(bg_i) {
                                let inst_n = *self.sdf_circle_counts.get(bg_i).unwrap_or(&1);
                                render_pass.set_pipeline(&self.sdf_circle.pipeline);
                                render_pass.set_bind_group(0, bg, &[]);
                                render_pass.draw(0..3, 0..inst_n);
                                needs_reset = true;
                            }
                        } else if let Some(bg_i) = self.sdf_curve_indices.get(job_i).copied().flatten() {
                            if let Some(bg) = self.sdf_curve_bind_groups.get(bg_i) {
                                let inst_n = *self.sdf_curve_counts.get(bg_i).unwrap_or(&1);
                                render_pass.set_pipeline(&self.sdf_curve.pipeline);
                                render_pass.set_bind_group(0, bg, &[]);
                                render_pass.draw(0..3, 0..inst_n);
                                needs_reset = true;
                            }
                        }
                        continue;
                    }

                    if let Some(t) = self.textures.get(&mesh.texture_id) {
                        render_pass.set_bind_group(1, &t.bind_group, &[]);
                        render_pass.set_index_buffer(
                            self.index_buffer.buffer.slice(
                                index_buffer_slice.start as u64..index_buffer_slice.end as u64,
                            ),
                            wgpu::IndexFormat::Uint32,
                        );
                        render_pass.set_vertex_buffer(
                            0,
                            self.vertex_buffer.buffer.slice(
                                vertex_buffer_slice.start as u64..vertex_buffer_slice.end as u64,
                            ),
                        );
                        render_pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
                    } else {
                        log::warn!("Missing texture: {:?}", mesh.texture_id);
                    }
                }
                Primitive::Callback(callback) => {
                    let Some(cbfn) = callback.callback.downcast_ref::<Callback>() else {
                        // We already warned in the `prepare` callback
                        continue;
                    };

                    let info = PaintCallbackInfo {
                        viewport: callback.rect,
                        clip_rect: *clip_rect,
                        pixels_per_point,
                        screen_size_px: size_in_pixels,
                    };

                    let viewport_px = info.viewport_in_pixels();
                    if viewport_px.width_px > 0 && viewport_px.height_px > 0 {
                        crate::profile_scope!("callback");

                        needs_reset = true;

                        // We're setting a default viewport for the render pass as a
                        // courtesy for the user, so that they don't have to think about
                        // it in the simple case where they just want to fill the whole
                        // paint area.
                        //
                        // The user still has the possibility of setting their own custom
                        // viewport during the paint callback, effectively overriding this
                        // one.
                        render_pass.set_viewport(
                            viewport_px.left_px as f32,
                            viewport_px.top_px as f32,
                            viewport_px.width_px as f32,
                            viewport_px.height_px as f32,
                            0.0,
                            1.0,
                        );

                        cbfn.0.paint(info, render_pass, &self.callback_resources);
                    }
                }
            }
        }

        render_pass.set_scissor_rect(0, 0, size_in_pixels[0], size_in_pixels[1]);
    }

    /// Should be called before `render()`.
    pub fn update_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        id: epaint::TextureId,
        image_delta: &epaint::ImageDelta,
    ) {
        crate::profile_function!();

        let width = image_delta.image.width() as u32;
        let height = image_delta.image.height() as u32;
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let data_bytes: &[u8] = match &image_delta.image {
            epaint::ImageData::Color(image) => {
                assert_eq!(width as usize * height as usize, image.pixels.len(), "Mismatch between texture size and texel count");
                bytemuck::cast_slice(image.pixels.as_slice())
            }
        };

        if let Some(pos) = image_delta.pos {
            // update the existing texture
            let t = self
                .textures
                .get(&id)
                .expect("Tried to update a texture that has not been allocated yet.");
            let origin = wgpu::Origin3d { x: pos[0] as u32, y: pos[1] as u32, z: 0 };
            crate::profile_scope!("copy_buffer_to_texture");
            let tex = t.texture.as_ref().expect("Tried to update user texture.");
            let staging = Self::copy_rgba_texture(device, encoder, tex, origin, width, height, data_bytes);
            self.staging_keepalive.push(staging);
        } else {
            // allocate a new texture
            // Use same label for all resources associated with this texture id (no point in retyping the type)
            let label_str = format!("egui_texid_{id:?}");
            let label = Some(label_str.as_str());
            let texture = {
                crate::profile_scope!("create_texture");
                device.create_texture(&wgpu::TextureDescriptor {
                    label,
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb, // Minspec for wgpu WebGL emulation is WebGL2, so this should always be supported.
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
                })
            };
            let sampler = self
                .samplers
                .entry(image_delta.options)
                .or_insert_with(|| create_sampler(image_delta.options, device));
            let kind = 0;
            let meta = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("egui_tex_meta"),
                contents: bytemuck::cast_slice(&[TexMeta { kind, _pad0: [0; 3], _pad1: [0; 4] }]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label,
                layout: &self.texture_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &texture.create_view(&wgpu::TextureViewDescriptor::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &meta,
                            offset: 0,
                            size: None,
                        }),
                    },
                ],
            });
            let origin = wgpu::Origin3d::ZERO;
            crate::profile_scope!("copy_buffer_to_texture");
            let staging = Self::copy_rgba_texture(device, encoder, &texture, origin, width, height, data_bytes);
            self.staging_keepalive.push(staging);
            self.textures.insert(id, GpuTexture { texture: Some(texture), bind_group, _meta_buffer: meta, width, height });
        };
    }

    pub fn free_texture(&mut self, id: &epaint::TextureId) {
        self.textures.remove(id);
        if *id == epaint::TextureId::default() { self.font_atlas_cache = None; }
    }

    /// Get the WGPU texture and bind group associated to a texture that has been allocated by egui.
    ///
    /// This could be used by custom paint hooks to render images that have been added through
    /// [`epaint::Context::load_texture`](https://docs.rs/egui/latest/egui/struct.Context.html#method.load_texture).
    pub fn texture(
        &self,
        id: &epaint::TextureId,
    ) -> Option<&GpuTexture> {
        self.textures.get(id)
    }

    /// Registers a `wgpu::Texture` with a `epaint::TextureId`.
    ///
    /// This enables the application to reference the texture inside an image ui element.
    /// This effectively enables off-screen rendering inside the egui UI. Texture must have
    /// the texture format `TextureFormat::Rgba8UnormSrgb` and
    /// Texture usage `TextureUsage::SAMPLED`.
    pub fn register_native_texture(
        &mut self,
        device: &wgpu::Device,
        texture: &wgpu::TextureView,
        texture_filter: wgpu::FilterMode,
    ) -> epaint::TextureId {
        self.register_native_texture_with_sampler_options(
            device,
            texture,
            wgpu::SamplerDescriptor {
                label: Some(format!("egui_user_image_{}", self.next_user_texture_id).as_str()),
                mag_filter: texture_filter,
                min_filter: texture_filter,
                ..Default::default()
            },
        )
    }

    /// Registers a `wgpu::Texture` with an existing `epaint::TextureId`.
    ///
    /// This enables applications to reuse `TextureId`s.
    pub fn update_egui_texture_from_wgpu_texture(
        &mut self,
        device: &wgpu::Device,
        texture: &wgpu::TextureView,
        texture_filter: wgpu::FilterMode,
        id: epaint::TextureId,
    ) {
        self.update_egui_texture_from_wgpu_texture_with_sampler_options(
            device,
            texture,
            wgpu::SamplerDescriptor {
                label: Some(format!("egui_user_image_{}", self.next_user_texture_id).as_str()),
                mag_filter: texture_filter,
                min_filter: texture_filter,
                ..Default::default()
            },
            id,
        );
    }

    /// Registers a `wgpu::Texture` with a `epaint::TextureId` while also accepting custom
    /// `wgpu::SamplerDescriptor` options.
    ///
    /// This allows applications to specify individual minification/magnification filters as well as
    /// custom mipmap and tiling options.
    ///
    /// The `Texture` must have the format `TextureFormat::Rgba8UnormSrgb` and usage
    /// `TextureUsage::SAMPLED`. Any compare function supplied in the `SamplerDescriptor` will be
    /// ignored.
    #[allow(clippy::needless_pass_by_value)] // false positive
    pub fn register_native_texture_with_sampler_options(
        &mut self,
        device: &wgpu::Device,
        texture: &wgpu::TextureView,
        sampler_descriptor: wgpu::SamplerDescriptor<'_>,
    ) -> epaint::TextureId {
        crate::profile_function!();

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            compare: None,
            ..sampler_descriptor
        });
        let meta = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("egui_tex_meta_user"),
            contents: bytemuck::cast_slice(&[TexMeta { kind: 0, _pad0: [0; 3], _pad1: [0; 4] }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(format!("egui_user_image_{}", self.next_user_texture_id).as_str()),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &meta,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        });

        let id = epaint::TextureId::User(self.next_user_texture_id);
        self.textures.insert(id, GpuTexture { texture: None, bind_group, _meta_buffer: meta, width: 0, height: 0 });
        self.next_user_texture_id += 1;

        id
    }

    /// Registers a `wgpu::Texture` with an existing `epaint::TextureId` while also accepting custom
    /// `wgpu::SamplerDescriptor` options.
    ///
    /// This allows applications to reuse `TextureId`s created with custom sampler options.
    #[allow(clippy::needless_pass_by_value)] // false positive
    pub fn update_egui_texture_from_wgpu_texture_with_sampler_options(
        &mut self,
        device: &wgpu::Device,
        texture: &wgpu::TextureView,
        sampler_descriptor: wgpu::SamplerDescriptor<'_>,
        id: epaint::TextureId,
    ) {
        crate::profile_function!();

        let t = self
            .textures
            .get_mut(&id)
            .expect("Tried to update a texture that has not been allocated yet.");

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            compare: None,
            ..sampler_descriptor
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(format!("egui_user_image_{}", self.next_user_texture_id).as_str()),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &t._meta_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        });

        t.bind_group = bind_group;
    }

    /// Uploads the uniform, vertex and index data used by the renderer.
    /// Should be called before `render()`.
    ///
    /// Returns all user-defined command buffers gathered from [`CallbackTrait::prepare`] & [`CallbackTrait::finish_prepare`] callbacks.
    pub fn update_buffers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        paint_jobs: &[epaint::ClippedPrimitive],
        screen_descriptor: &ScreenDescriptor,
    ) -> Vec<wgpu::CommandBuffer> {
        crate::profile_function!();

        let screen_size_in_points = screen_size_in_points(screen_descriptor);

        let uniform_buffer_content = UniformBuffer {
            screen_size_in_points,
            dithering: u32::from(self.options.dithering),
            predictable_texture_filtering: u32::from(self.options.predictable_texture_filtering),
        };
        if uniform_buffer_content != self.previous_uniform_buffer_content {
            crate::profile_scope!("update uniforms");
            queue.write_buffer(
                &self.uniform_buffer,
                0,
                bytemuck::cast_slice(&[uniform_buffer_content]),
            );
            self.previous_uniform_buffer_content = uniform_buffer_content;
        }

        self.sdf_rect_bind_groups.clear();
        self.sdf_rect_counts.clear();
        self.sdf_rect_indices.clear();
        self.sdf_circle_bind_groups.clear();
        self.sdf_circle_counts.clear();
        self.sdf_circle_indices.clear();
        self.sdf_ellipse_bind_groups.clear();
        self.sdf_ellipse_counts.clear();
        self.sdf_ellipse_indices.clear();
        self.sdf_curve_bind_groups.clear();
        self.sdf_curve_counts.clear();
        self.sdf_curve_indices.clear();
        self.auto_sdf_skip_mesh.clear();

        self.sdf_ui_frame = self.sdf_ui_frame.saturating_add(1);
        let mut sdf_ui_rects = 0u64;
        let mut sdf_ui_circles = 0u64;
        let mut sdf_ui_ellipses = 0u64;
        let mut sdf_ui_curves = 0u64;
        let mut sdf_ui_clip_runs = 0u64;
        let mut sdf_ui_drawcalls = 0u64;

        // Determine how many vertices & indices need to be rendered, gather prepare callbacks, and prepare SDF rect BGs.
        // We also record each callback's primitive index so SDF/GPU-text callbacks can do safe, order-preserving batching
        // only across adjacent primitives (never across intervening meshes).
        let mut callbacks: Vec<(u64, &dyn crate::CallbackTrait)> = Vec::new();
        let (mut vertex_count, mut index_count) = (0usize, 0usize);
        {
            crate::profile_scope!("count_vertices_indices");
            let mut i = 0usize;
            while i < paint_jobs.len() {
                let clipped_primitive = &paint_jobs[i];
                match &clipped_primitive.primitive {
                    Primitive::Mesh(mesh) => {
                        // Library-level "no pain" rewrite: detect common solid-color UI meshes and draw them via SDF.
                        if let Some((kind0, _)) = auto_sdf_from_mesh(mesh, screen_size_in_points) {
                            let clip_rect = clipped_primitive.clip_rect;
                            let mut j = i;
                            match kind0 {
                                AutoSdfKind::Rect => {
                                    let batch_idx = self.sdf_rect_bind_groups.len();
                                    let mut instances: Vec<SdfRectInstance> = Vec::with_capacity(32);
                                    while j < paint_jobs.len() {
                                        let cj = &paint_jobs[j];
                                        if cj.clip_rect != clip_rect { break; }
                                        let Primitive::Mesh(mj) = &cj.primitive else { break; };
                                        let Some((k, a)) = auto_sdf_from_mesh(mj, screen_size_in_points) else { break; };
                                        if k != AutoSdfKind::Rect { break; }
                                        let AutoSdf::Rect(inst) = a else { break; };
                                        instances.push(inst);
                                        vertex_count += mj.vertices.len();
                                        index_count += mj.indices.len();
                                        j += 1;
                                    }
                                    self.sdf_rect_bind_groups.push(self.sdf_rect.prepare_bind_group(device, screen_size_in_points, &instances));
                                    self.sdf_rect_counts.push(instances.len() as u32);
                                    sdf_ui_rects = sdf_ui_rects.saturating_add(instances.len() as u64);
                                    sdf_ui_clip_runs = sdf_ui_clip_runs.saturating_add(1);
                                    sdf_ui_drawcalls = sdf_ui_drawcalls.saturating_add(1);
                                    for k in i..j {
                                        self.sdf_rect_indices.push(if k == i { Some(batch_idx) } else { None });
                                        self.sdf_circle_indices.push(None);
                                        self.sdf_ellipse_indices.push(None);
                                        self.sdf_curve_indices.push(None);
                                        self.auto_sdf_skip_mesh.push(true);
                                    }
                                    i = j;
                                }
                                AutoSdfKind::Circle => {
                                    let batch_idx = self.sdf_circle_bind_groups.len();
                                    let mut instances: Vec<SdfCircleUniform> = Vec::with_capacity(64);
                                    while j < paint_jobs.len() {
                                        let cj = &paint_jobs[j];
                                        if cj.clip_rect != clip_rect { break; }
                                        let Primitive::Mesh(mj) = &cj.primitive else { break; };
                                        let Some((k, a)) = auto_sdf_from_mesh(mj, screen_size_in_points) else { break; };
                                        if k != AutoSdfKind::Circle { break; }
                                        let AutoSdf::Circle(inst) = a else { break; };
                                        instances.push(inst);
                                        vertex_count += mj.vertices.len();
                                        index_count += mj.indices.len();
                                        j += 1;
                                    }
                                    self.sdf_circle_bind_groups.push(self.sdf_circle.prepare_bind_group(device, &instances));
                                    self.sdf_circle_counts.push(instances.len() as u32);
                                    sdf_ui_circles = sdf_ui_circles.saturating_add(instances.len() as u64);
                                    sdf_ui_clip_runs = sdf_ui_clip_runs.saturating_add(1);
                                    sdf_ui_drawcalls = sdf_ui_drawcalls.saturating_add(1);
                                    for k in i..j {
                                        self.sdf_rect_indices.push(None);
                                        self.sdf_circle_indices.push(if k == i { Some(batch_idx) } else { None });
                                        self.sdf_ellipse_indices.push(None);
                                        self.sdf_curve_indices.push(None);
                                        self.auto_sdf_skip_mesh.push(true);
                                    }
                                    i = j;
                                }
                                AutoSdfKind::Curve => {
                                    let batch_idx = self.sdf_curve_bind_groups.len();
                                    let mut instances: Vec<SdfCurveUniform> = Vec::with_capacity(64);
                                    while j < paint_jobs.len() {
                                        let cj = &paint_jobs[j];
                                        if cj.clip_rect != clip_rect { break; }
                                        let Primitive::Mesh(mj) = &cj.primitive else { break; };
                                        let Some((k, a)) = auto_sdf_from_mesh(mj, screen_size_in_points) else { break; };
                                        if k != AutoSdfKind::Curve { break; }
                                        let AutoSdf::Curve(inst) = a else { break; };
                                        instances.push(inst);
                                        vertex_count += mj.vertices.len();
                                        index_count += mj.indices.len();
                                        j += 1;
                                    }
                                    self.sdf_curve_bind_groups.push(self.sdf_curve.prepare_bind_group(device, &instances));
                                    self.sdf_curve_counts.push(instances.len() as u32);
                                    sdf_ui_curves = sdf_ui_curves.saturating_add(instances.len() as u64);
                                    sdf_ui_clip_runs = sdf_ui_clip_runs.saturating_add(1);
                                    sdf_ui_drawcalls = sdf_ui_drawcalls.saturating_add(1);
                                    for k in i..j {
                                        self.sdf_rect_indices.push(None);
                                        self.sdf_circle_indices.push(None);
                                        self.sdf_ellipse_indices.push(None);
                                        self.sdf_curve_indices.push(if k == i { Some(batch_idx) } else { None });
                                        self.auto_sdf_skip_mesh.push(true);
                                    }
                                    i = j;
                                }
                            }
                        } else {
                            self.sdf_rect_indices.push(None);
                            self.sdf_circle_indices.push(None);
                            self.sdf_ellipse_indices.push(None);
                            self.sdf_curve_indices.push(None);
                            self.auto_sdf_skip_mesh.push(false);
                            vertex_count += mesh.vertices.len();
                            index_count += mesh.indices.len();
                            i += 1;
                        }
                    }
                    Primitive::Callback(callback) => {
                        self.sdf_rect_indices.push(None);
                        self.sdf_circle_indices.push(None);
                        self.sdf_ellipse_indices.push(None);
                        self.sdf_curve_indices.push(None);
                        self.auto_sdf_skip_mesh.push(false);
                        if let Some(c) = callback.callback.downcast_ref::<Callback>() { callbacks.push((i as u64, c.0.as_ref())); }
                        else { log::warn!("Unknown paint callback: expected `egui_wgpu::Callback`"); }
                        i += 1;
                    }
                }
            }
        }

        cunning_wgpu_ui::sdf::ui_stats::sdf_ui_set_stats(cunning_wgpu_ui::sdf::ui_stats::SdfUiBatchStats {
            frame_id: self.sdf_ui_frame,
            rects: sdf_ui_rects,
            circles: sdf_ui_circles,
            ellipses: sdf_ui_ellipses,
            curves: sdf_ui_curves,
            clip_runs: sdf_ui_clip_runs,
            drawcalls: sdf_ui_drawcalls,
        });

        if index_count > 0 {
            crate::profile_scope!("indices", index_count.to_string());

            self.index_buffer.slices.clear();

            let required_index_buffer_size = (std::mem::size_of::<u32>() * index_count) as u64;
            if self.index_buffer.capacity < required_index_buffer_size {
                // Resize index buffer if needed.
                self.index_buffer.capacity =
                    (self.index_buffer.capacity * 2).at_least(required_index_buffer_size);
                self.index_buffer.buffer = create_index_buffer(device, self.index_buffer.capacity);
            }

            let index_buffer_staging = queue.write_buffer_with(
                &self.index_buffer.buffer,
                0,
                NonZeroU64::new(required_index_buffer_size).unwrap(),
            );

            let Some(mut index_buffer_staging) = index_buffer_staging else {
                panic!("Failed to create staging buffer for index data. Index count: {index_count}. Required index buffer size: {required_index_buffer_size}. Actual size {} and capacity: {} (bytes)", self.index_buffer.buffer.size(), self.index_buffer.capacity);
            };

            let mut index_offset = 0;
            for epaint::ClippedPrimitive { primitive, .. } in paint_jobs {
                match primitive {
                    Primitive::Mesh(mesh) => {
                        let size = mesh.indices.len() * std::mem::size_of::<u32>();
                        let slice = index_offset..(size + index_offset);
                        index_buffer_staging[slice.clone()]
                            .copy_from_slice(bytemuck::cast_slice(&mesh.indices));
                        self.index_buffer.slices.push(slice);
                        index_offset += size;
                    }
                    Primitive::Callback(_) => {}
                }
            }
        }
        if vertex_count > 0 {
            crate::profile_scope!("vertices", vertex_count.to_string());

            self.vertex_buffer.slices.clear();

            let required_vertex_buffer_size = (std::mem::size_of::<Vertex>() * vertex_count) as u64;
            if self.vertex_buffer.capacity < required_vertex_buffer_size {
                // Resize vertex buffer if needed.
                self.vertex_buffer.capacity =
                    (self.vertex_buffer.capacity * 2).at_least(required_vertex_buffer_size);
                self.vertex_buffer.buffer =
                    create_vertex_buffer(device, self.vertex_buffer.capacity);
            }

            let vertex_buffer_staging = queue.write_buffer_with(
                &self.vertex_buffer.buffer,
                0,
                NonZeroU64::new(required_vertex_buffer_size).unwrap(),
            );

            let Some(mut vertex_buffer_staging) = vertex_buffer_staging else {
                panic!("Failed to create staging buffer for vertex data. Vertex count: {vertex_count}. Required vertex buffer size: {required_vertex_buffer_size}. Actual size {} and capacity: {} (bytes)", self.vertex_buffer.buffer.size(), self.vertex_buffer.capacity);
            };

            let mut vertex_offset = 0;
            for epaint::ClippedPrimitive { primitive, .. } in paint_jobs {
                match primitive {
                    Primitive::Mesh(mesh) => {
                        let size = mesh.vertices.len() * std::mem::size_of::<Vertex>();
                        let slice = vertex_offset..(size + vertex_offset);
                        vertex_buffer_staging[slice.clone()]
                            .copy_from_slice(bytemuck::cast_slice(&mesh.vertices));
                        self.vertex_buffer.slices.push(slice);
                        vertex_offset += size;
                    }
                    Primitive::Callback(_) => {}
                }
            }
        }

        let mut user_cmd_bufs = Vec::new();
        {
            crate::profile_scope!("prepare callbacks");
            for (seq, callback) in &callbacks {
                let s = self
                    .callback_resources
                    .entry::<cunning_wgpu_ui::sdf::SdfRenderSeq>()
                    .or_insert(cunning_wgpu_ui::sdf::SdfRenderSeq(0));
                s.0 = *seq;
                user_cmd_bufs.extend(callback.prepare(device, queue, screen_descriptor, encoder, &mut self.callback_resources));
            }
        }
        {
            crate::profile_scope!("finish prepare callbacks");
            for (seq, callback) in &callbacks {
                let s = self
                    .callback_resources
                    .entry::<cunning_wgpu_ui::sdf::SdfRenderSeq>()
                    .or_insert(cunning_wgpu_ui::sdf::SdfRenderSeq(0));
                s.0 = *seq;
                user_cmd_bufs.extend(callback.finish_prepare(device, queue, encoder, &mut self.callback_resources));
            }
        }

        user_cmd_bufs
    }
}

fn create_sampler(
    options: epaint::textures::TextureOptions,
    device: &wgpu::Device,
) -> wgpu::Sampler {
    let mag_filter = match options.magnification {
        epaint::textures::TextureFilter::Nearest => wgpu::FilterMode::Nearest,
        epaint::textures::TextureFilter::Linear => wgpu::FilterMode::Linear,
    };
    let min_filter = match options.minification {
        epaint::textures::TextureFilter::Nearest => wgpu::FilterMode::Nearest,
        epaint::textures::TextureFilter::Linear => wgpu::FilterMode::Linear,
    };
    let address_mode = match options.wrap_mode {
        epaint::textures::TextureWrapMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        epaint::textures::TextureWrapMode::Repeat => wgpu::AddressMode::Repeat,
        epaint::textures::TextureWrapMode::MirroredRepeat => wgpu::AddressMode::MirrorRepeat,
    };
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(&format!(
            "egui sampler (mag: {mag_filter:?}, min {min_filter:?})"
        )),
        mag_filter,
        min_filter,
        address_mode_u: address_mode,
        address_mode_v: address_mode,
        ..Default::default()
    })
}

fn create_vertex_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    crate::profile_function!();
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("egui_vertex_buffer"),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        size,
        mapped_at_creation: false,
    })
}

fn create_index_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    crate::profile_function!();
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("egui_index_buffer"),
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        size,
        mapped_at_creation: false,
    })
}

/// A Rect in physical pixel space, used for setting clipping rectangles.
struct ScissorRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl ScissorRect {
    fn new(clip_rect: &epaint::Rect, pixels_per_point: f32, target_size: [u32; 2]) -> Self {
        // Transform clip rect to physical pixels:
        let clip_min_x = pixels_per_point * clip_rect.min.x;
        let clip_min_y = pixels_per_point * clip_rect.min.y;
        let clip_max_x = pixels_per_point * clip_rect.max.x;
        let clip_max_y = pixels_per_point * clip_rect.max.y;

        // IMPORTANT: Use floor/ceil to avoid shaving off 1px (icons/logos near edges can look "covered").
        let clip_min_x = clip_min_x.floor().max(0.0) as u32;
        let clip_min_y = clip_min_y.floor().max(0.0) as u32;
        let clip_max_x = clip_max_x.ceil().max(0.0) as u32;
        let clip_max_y = clip_max_y.ceil().max(0.0) as u32;

        // Clamp:
        let clip_min_x = clip_min_x.clamp(0, target_size[0]);
        let clip_min_y = clip_min_y.clamp(0, target_size[1]);
        let clip_max_x = clip_max_x.clamp(clip_min_x, target_size[0]);
        let clip_max_y = clip_max_y.clamp(clip_min_y, target_size[1]);

        Self {
            x: clip_min_x,
            y: clip_min_y,
            width: clip_max_x - clip_min_x,
            height: clip_max_y - clip_min_y,
        }
    }
}

#[test]
fn renderer_impl_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Renderer>();
}
