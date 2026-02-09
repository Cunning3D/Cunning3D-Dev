use std::{collections::HashMap, sync::Arc};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

// ----------------------------------------------------------------------------
// Uniform Data
// ----------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SdfCurveUniform {
    pub p0: [f32; 2],       // 0
    pub p1: [f32; 2],       // 8
    pub p2: [f32; 2],       // 16
    pub p3: [f32; 2],       // 24
    pub color: [f32; 4],    // 32
    pub thickness: f32,     // 48
    pub softness: f32,      // 52
    pub screen_size: [f32; 2], // 56
    pub _pad: [f32; 2],     // 64 (Align to 16 bytes)
}

// ----------------------------------------------------------------------------
// Shader
// ----------------------------------------------------------------------------

const SDF_CURVE_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) @interpolate(flat) inst: u32,
};

struct Uniforms {
    p0: vec2<f32>,
    p1: vec2<f32>,
    p2: vec2<f32>,
    p3: vec2<f32>,
    color: vec4<f32>,
    thickness: f32,
    softness: f32,
    screen_size: vec2<f32>,
};

@group(0) @binding(0) var<storage, read> inst_buf: array<Uniforms>;

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32, @builtin(instance_index) iid: u32) -> VertexOutput {
    let u = inst_buf[iid];
    let x = f32((in_vertex_index << 1u) & 2u);
    let y = f32(in_vertex_index & 2u);
    let pos = vec2<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0);
    
    var out: VertexOutput;
    out.clip_position = vec4<f32>(pos, 0.0, 1.0);
    let uv = vec2<f32>(x, y);
    // Keep local_pos in the same screen-space units as egui (points).
    // The visible region (NDC -1..1) interpolates uv to 0..1, so we must NOT scale by 0.5.
    out.local_pos = uv * u.screen_size;
    out.inst = iid;
    
    return out;
}

fn sdSegment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

fn bezier_point(t: f32, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>, p3: vec2<f32>) -> vec2<f32> {
    let u = 1.0 - t;
    let tt = t * t;
    let uu = u * u;
    let uuu = uu * u;
    let ttt = tt * t;
    return uuu * p0 + 3.0 * uu * t * p1 + 3.0 * u * tt * p2 + ttt * p3;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let u = inst_buf[in.inst];
    let p = in.local_pos;
    
    // Flattening algorithm
    let SEGMENTS = 30;
    var min_dist = 1e9;
    
    var prev = u.p0;
    for (var i = 1; i <= SEGMENTS; i++) {
        let t = f32(i) / f32(SEGMENTS);
        let curr = bezier_point(t, u.p0, u.p1, u.p2, u.p3);
        let d = sdSegment(p, prev, curr);
        min_dist = min(min_dist, d);
        prev = curr;
    }
    
    let half_thick = u.thickness * 0.5;
    let sdf = min_dist - half_thick;
    
    // Anti-aliasing
    let aa_width = max(1.0, u.softness); 
    let alpha = 1.0 - smoothstep(-aa_width*0.5, aa_width*0.5, sdf);
    
    if (alpha <= 0.0) {
        discard;
    }
    
    return vec4<f32>(u.color.rgb, u.color.a * alpha);
}
"#;

#[derive(Clone, Copy, Debug, Default)]
pub struct SdfCurveBatchStats { pub frame_id: u64, pub instances: u64, pub clip_regions: u64, pub drawcalls: u64 }

#[derive(Clone, Copy, Debug, Default)]
pub struct SdfCurveClipBatchStat { pub scissor: [u32; 4], pub instances: u32 }

static SDF_CURVE_LAST_FRAME: AtomicU64 = AtomicU64::new(0);
static SDF_CURVE_LAST_INST: AtomicU64 = AtomicU64::new(0);
static SDF_CURVE_LAST_REGIONS: AtomicU64 = AtomicU64::new(0);
static SDF_CURVE_LAST_DRAWCALLS: AtomicU64 = AtomicU64::new(0);
static SDF_CURVE_VERBOSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SDF_CURVE_LAST_DETAILS_FRAME: AtomicU64 = AtomicU64::new(0);
static SDF_CURVE_LAST_DETAILS: OnceLock<Mutex<Vec<SdfCurveClipBatchStat>>> = OnceLock::new();

pub fn sdf_curve_last_stats() -> SdfCurveBatchStats {
    SdfCurveBatchStats {
        frame_id: SDF_CURVE_LAST_FRAME.load(Ordering::Relaxed),
        instances: SDF_CURVE_LAST_INST.load(Ordering::Relaxed),
        clip_regions: SDF_CURVE_LAST_REGIONS.load(Ordering::Relaxed),
        drawcalls: SDF_CURVE_LAST_DRAWCALLS.load(Ordering::Relaxed),
    }
}

pub fn sdf_curve_set_verbose_details_enabled(enabled: bool) { SDF_CURVE_VERBOSE.store(enabled, Ordering::Relaxed); }

pub fn sdf_curve_last_batch_details() -> (u64, Vec<SdfCurveClipBatchStat>) {
    let fid = SDF_CURVE_LAST_DETAILS_FRAME.load(Ordering::Relaxed);
    let v = SDF_CURVE_LAST_DETAILS.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap().clone();
    (fid, v)
}

// ----------------------------------------------------------------------------
// Renderer
// ----------------------------------------------------------------------------

pub struct SdfCurveRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl SdfCurveRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("SDF Curve Shader"),
            source: wgpu::ShaderSource::Wgsl(SDF_CURVE_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SDF Curve Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SDF Curve Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SDF Curve Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
        }
    }

    pub fn prepare_bind_group(&self, device: &wgpu::Device, instances: &[SdfCurveUniform]) -> wgpu::BindGroup {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("SDF Curve Instances"),
            contents: bytemuck::cast_slice(instances),
            usage: wgpu::BufferUsages::STORAGE,
        });

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SDF Curve Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        })
    }
}

// ----------------------------------------------------------------------------
// Callback
// ----------------------------------------------------------------------------

struct Batch {
    scissor: [u32; 4],
    instances: Vec<SdfCurveUniform>,
    bind_group: Option<wgpu::BindGroup>,
    leader: bool,
}

struct SdfCurveQueue {
    pub renderer: Arc<SdfCurveRenderer>,
    batches: Vec<Batch>,
    batch_map: HashMap<u64, usize>,
    pub target_px: [u32; 2],
    pub last_frame_id: u64,
    pub last_upload_frame_id: u64,
    last_seq_id: u64,
}

pub struct SdfCurveCallback {
    pub uniform: SdfCurveUniform,
    pub frame_id: u64,
    pub _clip: egui::Rect,
    pub key: Arc<AtomicU64>,
}

#[inline]
fn pack_key(batch_idx: usize, is_leader: bool) -> u64 { (1u64 << 63) | ((batch_idx as u64) << 1) | (is_leader as u64) }

#[inline]
fn unpack_key(v: u64) -> Option<(usize, bool)> { if (v >> 63) == 0 { None } else { Some((((v & ((1u64 << 63) - 1)) >> 1) as usize, (v & 1) != 0)) } }

impl crate::CallbackTrait for SdfCurveCallback {
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
        let queue = resources.entry::<SdfCurveQueue>().or_insert_with(|| {
            SdfCurveQueue {
                renderer: Arc::new(SdfCurveRenderer::new(device, fmt)),
                batches: Vec::with_capacity(128),
                batch_map: HashMap::with_capacity(128),
                target_px: [1, 1],
                last_frame_id: 0,
                last_upload_frame_id: 0,
                last_seq_id: 0,
            }
        });
        queue.target_px = screen_descriptor.size_in_pixels;
        let ppp = screen_descriptor.pixels_per_point.max(1e-6);

        if fid != queue.last_frame_id {
            queue.batches.clear();
            queue.batch_map.clear();
            queue.last_frame_id = fid;
            queue.last_seq_id = 0;
        }

        // Order-preserving batching: only across adjacent primitives.
        if seq != queue.last_seq_id.saturating_add(1) { queue.batch_map.clear(); }
        queue.last_seq_id = seq;
        let (min_x, min_y, w, h) = {
            let tw = queue.target_px[0];
            let th = queue.target_px[1];
            if tw == 0 || th == 0 { return Vec::new(); }
            let rect = self._clip;
            let min_x = (rect.min.x * ppp).floor().max(0.0) as u32;
            let min_y = (rect.min.y * ppp).floor().max(0.0) as u32;
            let mut max_x = (rect.max.x * ppp).ceil().max(0.0) as u32;
            let mut max_y = (rect.max.y * ppp).ceil().max(0.0) as u32;
            if min_x >= tw || min_y >= th { return Vec::new(); }
            if max_x > tw { max_x = tw; }
            if max_y > th { max_y = th; }
            if max_x <= min_x || max_y <= min_y { return Vec::new(); }
            (min_x, min_y, max_x - min_x, max_y - min_y)
        };
        let k = ((min_x.min(0xFFFF) as u64) << 48)
            | ((min_y.min(0xFFFF) as u64) << 32)
            | ((w.min(0xFFFF) as u64) << 16)
            | (h.min(0xFFFF) as u64);
        let bi = *queue.batch_map.entry(k).or_insert_with(|| {
            let idx = queue.batches.len();
            queue.batches.push(Batch { scissor: [min_x, min_y, w, h], instances: Vec::with_capacity(256), bind_group: None, leader: true });
            idx
        });
        let b = &mut queue.batches[bi];
        let is_leader = b.leader;
        b.leader = false;
        b.instances.push(self.uniform);
        self.key.store(pack_key(bi, is_leader), Ordering::Relaxed);
        
        Vec::new()
    }

    fn finish_prepare(&self, device: &wgpu::Device, _queue: &wgpu::Queue, _egui_encoder: &mut wgpu::CommandEncoder, resources: &mut crate::CallbackResources) -> Vec<wgpu::CommandBuffer> {
        let fid = resources.get::<super::SdfFrameId>().map(|v| v.0).unwrap_or(self.frame_id);
        let Some(q) = resources.get_mut::<SdfCurveQueue>() else { return Vec::new(); };
        if fid == q.last_upload_frame_id { return Vec::new(); }
        q.last_upload_frame_id = fid;
        for b in &mut q.batches {
            if b.instances.is_empty() { b.bind_group = None; continue; }
            b.bind_group = Some(q.renderer.prepare_bind_group(device, &b.instances));
        }
        let inst_total: u64 = q.batches.iter().map(|b| b.instances.len() as u64).sum();
        let regions: u64 = q.batches.iter().filter(|b| !b.instances.is_empty()).count() as u64;
        SDF_CURVE_LAST_FRAME.store(fid, Ordering::Relaxed);
        SDF_CURVE_LAST_INST.store(inst_total, Ordering::Relaxed);
        SDF_CURVE_LAST_REGIONS.store(regions, Ordering::Relaxed);
        SDF_CURVE_LAST_DRAWCALLS.store(regions, Ordering::Relaxed);
        if SDF_CURVE_VERBOSE.load(Ordering::Relaxed) {
            let mut out: Vec<SdfCurveClipBatchStat> = Vec::with_capacity(q.batches.len());
            for b in q.batches.iter().filter(|b| !b.instances.is_empty()) {
                out.push(SdfCurveClipBatchStat { scissor: b.scissor, instances: b.instances.len() as u32 });
            }
            out.sort_by_key(|b| (b.scissor[0], b.scissor[1], b.scissor[2], b.scissor[3]));
            *SDF_CURVE_LAST_DETAILS.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap() = out;
            SDF_CURVE_LAST_DETAILS_FRAME.store(fid, Ordering::Relaxed);
        }
        Vec::new()
    }

    fn paint<'a>(&'a self, info: egui::PaintCallbackInfo, render_pass: &mut wgpu::RenderPass<'a>, resources: &'a crate::CallbackResources) {
        if let Some(q) = resources.get::<SdfCurveQueue>() {
            let Some((bi, is_leader)) = unpack_key(self.key.load(Ordering::Relaxed)) else { return; };
            if !is_leader { return; }
            let Some(b) = q.batches.get(bi) else { return; };
            let Some(bg) = b.bind_group.as_ref() else { return; };
            let tw = q.target_px[0];
            let th = q.target_px[1];
            if tw == 0 || th == 0 { return; }
            // egui-wgpu sets viewport to callback.rect; our shaders use full-screen coordinates.
            render_pass.set_viewport(0.0, 0.0, tw as f32, th as f32, 0.0, 1.0);
            let Some((cx, cy, cw, ch)) = super::clamp_scissor(&info, q.target_px) else { return; };
            let [bx, by, bw, bh] = b.scissor;
            let x0 = cx.max(bx);
            let y0 = cy.max(by);
            let x1 = cx.saturating_add(cw).min(bx.saturating_add(bw));
            let y1 = cy.saturating_add(ch).min(by.saturating_add(bh));
            let w = x1.saturating_sub(x0);
            let h = y1.saturating_sub(y0);
            if w == 0 || h == 0 { return; }
            render_pass.set_scissor_rect(x0, y0, w, h);
            render_pass.set_pipeline(&q.renderer.pipeline);
            render_pass.set_bind_group(0, bg, &[]);
            render_pass.draw(0..3, 0..b.instances.len() as u32);
        }
    }
}

pub fn create_sdf_curve_callback(
    rect: egui::Rect,
    uniform: SdfCurveUniform,
    frame_id: u64,
) -> egui::PaintCallback {
    crate::Callback::new_paint_callback(
        rect,
        SdfCurveCallback {
            uniform,
            frame_id,
            _clip: rect,
            key: Arc::new(AtomicU64::new(0)),
        },
    )
}

