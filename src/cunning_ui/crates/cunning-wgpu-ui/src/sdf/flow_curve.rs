use std::{collections::HashMap, sync::Arc};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SdfFlowCurveUniform {
    pub p01: [f32; 4],           // p0.xy, p1.xy
    pub p23: [f32; 4],           // p2.xy, p3.xy
    pub color: [f32; 4],         // rgba
    pub params0: [f32; 4],       // thickness, softness, pulse_width, pulse_spacing
    pub params1: [f32; 4],       // speed, phase, flow_intensity, blink_hz
    pub screen_params: [f32; 4], // screen_size.xy, reserved, reserved
}

const SDF_FLOW_CURVE_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) @interpolate(flat) inst: u32,
};

struct TimeU {
    time: vec4<f32>,
};

struct Uniforms {
    p01: vec4<f32>,
    p23: vec4<f32>,
    color: vec4<f32>,
    params0: vec4<f32>,       // x=thickness, y=softness, z=pulse_width, w=pulse_spacing
    params1: vec4<f32>,       // x=speed, y=phase, z=flow_intensity, w=blink_hz
    screen_params: vec4<f32>, // xy=screen_size
};

@group(0) @binding(0) var<storage, read> inst_buf: array<Uniforms>;
@group(0) @binding(1) var<uniform> tbuf: TimeU;

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32, @builtin(instance_index) iid: u32) -> VertexOutput {
    let u = inst_buf[iid];
    let x = f32((in_vertex_index << 1u) & 2u);
    let y = f32(in_vertex_index & 2u);
    let pos = vec2<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0);
    var out: VertexOutput;
    out.clip_position = vec4<f32>(pos, 0.0, 1.0);
    let uv = vec2<f32>(x, y);
    out.local_pos = uv * u.screen_params.xy;
    out.inst = iid;
    return out;
}

fn bezier_point(t: f32, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>, p3: vec2<f32>) -> vec2<f32> {
    let u1 = 1.0 - t;
    let tt = t * t;
    let uu = u1 * u1;
    let uuu = uu * u1;
    let ttt = tt * t;
    return uuu * p0 + 3.0 * uu * t * p1 + 3.0 * u1 * tt * p2 + ttt * p3;
}

// Returns: x = distance to segment, y = distance along segment from a (0..len)
fn sd_segment_with_s(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let pa = p - a;
    let ba = b - a;
    let l2 = dot(ba, ba);
    if (l2 <= 1e-6) { return vec2<f32>(length(pa), 0.0); }
    let h = clamp(dot(pa, ba) / l2, 0.0, 1.0);
    let closest = a + ba * h;
    return vec2<f32>(length(p - closest), length(ba) * h);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let u = inst_buf[in.inst];
    let p0 = u.p01.xy;
    let p1 = u.p01.zw;
    let p2 = u.p23.xy;
    let p3 = u.p23.zw;
    let p = in.local_pos;
    let segments = 30;
    var min_dist = 1e9;
    var best_s = 0.0;
    var accum = 0.0;
    var prev = p0;
    for (var i = 1; i <= segments; i++) {
        let tt = f32(i) / f32(segments);
        let curr = bezier_point(tt, p0, p1, p2, p3);
        let seg = sd_segment_with_s(p, prev, curr);
        if (seg.x < min_dist) {
            min_dist = seg.x;
            best_s = accum + seg.y;
        }
        accum = accum + length(curr - prev);
        prev = curr;
    }

    let half_thick = u.params0.x * 0.5;
    let sdf = min_dist - half_thick;
    let aa = max(1.0, u.params0.y);
    var alpha = 1.0 - smoothstep(-aa * 0.5, aa * 0.5, sdf);
    if (alpha <= 0.0) { discard; }

    // Flow pulse along arc-length (best_s). Use normalized s for stable frequency.
    let s = best_s / max(1e-3, accum);
    let spacing = max(0.02, u.params0.w);
    let pw = clamp(u.params0.z, 0.001, 0.5);
    let speed = u.params1.x;
    let phase = u.params1.y;
    let flow_k = max(0.0, u.params1.z);
    // NOTE: direction: negative time moves from input->output in our editor layout.
    let x = fract((s / spacing) - (tbuf.time.x * speed) + phase);
    // Visible asymmetric pulse: exp tail from head (x=0 -> 1, x->1 -> ~0).
    let pulse = exp(-x / max(1e-3, pw));

    let blink_hz = max(0.0, u.params1.w);
    if (blink_hz > 0.0) {
        let b = 0.55 + 0.45 * sin(tbuf.time.x * 6.2831853 * blink_hz);
        alpha = alpha * b;
    }

    // Failed state: use high blink_hz to indicate broken flow.
    if (blink_hz >= 2.9 && flow_k <= 0.0) {
        let m = step(fract(s * 17.0 + phase), 0.72);
        alpha = alpha * m;
        if (alpha <= 0.0) { discard; }
    }

    let base = u.color;
    let boost = 1.0 + (pulse * flow_k) * 2.2;
    let rgb = base.rgb * boost;
    return vec4<f32>(rgb, base.a * alpha);
}
"#;

#[derive(Clone, Copy, Debug, Default)]
pub struct SdfFlowCurveBatchStats {
    pub frame_id: u64,
    pub instances: u64,
    pub clip_regions: u64,
    pub drawcalls: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SdfFlowCurveClipBatchStat {
    pub scissor: [u32; 4],
    pub instances: u32,
}

static SDF_FLOW_LAST_FRAME: AtomicU64 = AtomicU64::new(0);
static SDF_FLOW_LAST_INST: AtomicU64 = AtomicU64::new(0);
static SDF_FLOW_LAST_REGIONS: AtomicU64 = AtomicU64::new(0);
static SDF_FLOW_LAST_DRAWCALLS: AtomicU64 = AtomicU64::new(0);
static SDF_FLOW_VERBOSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SDF_FLOW_LAST_DETAILS_FRAME: AtomicU64 = AtomicU64::new(0);
static SDF_FLOW_LAST_DETAILS: OnceLock<Mutex<Vec<SdfFlowCurveClipBatchStat>>> = OnceLock::new();

pub fn sdf_flow_curve_last_stats() -> SdfFlowCurveBatchStats {
    SdfFlowCurveBatchStats {
        frame_id: SDF_FLOW_LAST_FRAME.load(Ordering::Relaxed),
        instances: SDF_FLOW_LAST_INST.load(Ordering::Relaxed),
        clip_regions: SDF_FLOW_LAST_REGIONS.load(Ordering::Relaxed),
        drawcalls: SDF_FLOW_LAST_DRAWCALLS.load(Ordering::Relaxed),
    }
}

pub fn sdf_flow_curve_set_verbose_details_enabled(enabled: bool) {
    SDF_FLOW_VERBOSE.store(enabled, Ordering::Relaxed);
}

pub fn sdf_flow_curve_last_batch_details() -> (u64, Vec<SdfFlowCurveClipBatchStat>) {
    let fid = SDF_FLOW_LAST_DETAILS_FRAME.load(Ordering::Relaxed);
    let v = SDF_FLOW_LAST_DETAILS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .clone();
    (fid, v)
}

pub struct SdfFlowCurveRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl SdfFlowCurveRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("SDF Flow Curve Shader"),
            source: wgpu::ShaderSource::Wgsl(SDF_FLOW_CURVE_SHADER.into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SDF Flow Curve Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SDF Flow Curve Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SDF Flow Curve Pipeline"),
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
}

struct Batch {
    scissor: [u32; 4],
    instances: Vec<SdfFlowCurveUniform>,
    bind_group: Option<wgpu::BindGroup>,
    leader: bool,
}

struct SdfFlowCurveQueue {
    pub renderer: Arc<SdfFlowCurveRenderer>,
    batches: Vec<Batch>,
    batch_map: HashMap<u64, usize>,
    pub target_px: [u32; 2],
    pub last_frame_id: u64,
    pub last_upload_frame_id: u64,
    pub last_seq_id: u64,
    pub time_buf: wgpu::Buffer,
}

pub struct SdfFlowCurveCallback {
    pub uniform: SdfFlowCurveUniform,
    pub frame_id: u64,
    pub _clip: egui::Rect,
    pub key: Arc<AtomicU64>,
}

#[inline]
fn pack_key(batch_idx: usize, is_leader: bool) -> u64 {
    (1u64 << 63) | ((batch_idx as u64) << 1) | (is_leader as u64)
}

#[inline]
fn unpack_key(v: u64) -> Option<(usize, bool)> {
    if (v >> 63) == 0 {
        None
    } else {
        Some((
            (((v & ((1u64 << 63) - 1)) >> 1) as usize),
            (v & 1) != 0,
        ))
    }
}

impl crate::CallbackTrait for SdfFlowCurveCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        screen_descriptor: &crate::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut crate::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let fid = resources
            .get::<super::SdfFrameId>()
            .map(|v| v.0)
            .unwrap_or(self.frame_id);
        let seq = resources
            .get::<super::SdfRenderSeq>()
            .map(|v| v.0)
            .unwrap_or(0);
        let fmt = super::target_format(resources);
        let queue = resources.entry::<SdfFlowCurveQueue>().or_insert_with(|| {
            let time_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("SDF Flow Curve Time"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            SdfFlowCurveQueue {
                renderer: Arc::new(SdfFlowCurveRenderer::new(device, fmt)),
                batches: Vec::with_capacity(128),
                batch_map: HashMap::with_capacity(128),
                target_px: [1, 1],
                last_frame_id: 0,
                last_upload_frame_id: 0,
                last_seq_id: 0,
                time_buf,
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

        if seq != queue.last_seq_id.saturating_add(1) {
            queue.batch_map.clear();
        }
        queue.last_seq_id = seq;

        let (min_x, min_y, w, h) = {
            let tw = queue.target_px[0];
            let th = queue.target_px[1];
            if tw == 0 || th == 0 {
                return Vec::new();
            }
            let rect = self._clip;
            let min_x = (rect.min.x * ppp).floor().max(0.0) as u32;
            let min_y = (rect.min.y * ppp).floor().max(0.0) as u32;
            let mut max_x = (rect.max.x * ppp).ceil().max(0.0) as u32;
            let mut max_y = (rect.max.y * ppp).ceil().max(0.0) as u32;
            if min_x >= tw || min_y >= th {
                return Vec::new();
            }
            if max_x > tw {
                max_x = tw;
            }
            if max_y > th {
                max_y = th;
            }
            if max_x <= min_x || max_y <= min_y {
                return Vec::new();
            }
            (min_x, min_y, max_x - min_x, max_y - min_y)
        };
        let k = ((min_x.min(0xFFFF) as u64) << 48)
            | ((min_y.min(0xFFFF) as u64) << 32)
            | ((w.min(0xFFFF) as u64) << 16)
            | (h.min(0xFFFF) as u64);
        let bi = *queue.batch_map.entry(k).or_insert_with(|| {
            let idx = queue.batches.len();
            queue.batches.push(Batch {
                scissor: [min_x, min_y, w, h],
                instances: Vec::with_capacity(256),
                bind_group: None,
                leader: true,
            });
            idx
        });
        let b = &mut queue.batches[bi];
        let is_leader = b.leader;
        b.leader = false;
        b.instances.push(self.uniform);
        self.key.store(pack_key(bi, is_leader), Ordering::Relaxed);
        Vec::new()
    }

    fn finish_prepare(
        &self,
        device: &wgpu::Device,
        wgpu_queue: &wgpu::Queue,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut crate::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let fid = resources
            .get::<super::SdfFrameId>()
            .map(|v| v.0)
            .unwrap_or(self.frame_id);
        let t = resources.get::<super::SdfTime>().map(|v| v.0).unwrap_or(0.0);
        let Some(q) = resources.get_mut::<SdfFlowCurveQueue>() else {
            return Vec::new();
        };
        if fid == q.last_upload_frame_id {
            return Vec::new();
        }
        q.last_upload_frame_id = fid;

        let tb: [f32; 4] = [t, 0.0, 0.0, 0.0];
        wgpu_queue.write_buffer(&q.time_buf, 0, bytemuck::cast_slice(&tb));

        for b in &mut q.batches {
            if b.instances.is_empty() {
                b.bind_group = None;
                continue;
            }
            let inst_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("SDF Flow Curve Instances"),
                contents: bytemuck::cast_slice(&b.instances),
                usage: wgpu::BufferUsages::STORAGE,
            });
            b.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("SDF Flow Curve Bind Group"),
                layout: &q.renderer.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: inst_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: q.time_buf.as_entire_binding(),
                    },
                ],
            }));
        }

        let inst_total: u64 = q.batches.iter().map(|b| b.instances.len() as u64).sum();
        let regions: u64 = q
            .batches
            .iter()
            .filter(|b| !b.instances.is_empty())
            .count() as u64;
        SDF_FLOW_LAST_FRAME.store(fid, Ordering::Relaxed);
        SDF_FLOW_LAST_INST.store(inst_total, Ordering::Relaxed);
        SDF_FLOW_LAST_REGIONS.store(regions, Ordering::Relaxed);
        SDF_FLOW_LAST_DRAWCALLS.store(regions, Ordering::Relaxed);
        if SDF_FLOW_VERBOSE.load(Ordering::Relaxed) {
            let mut out: Vec<SdfFlowCurveClipBatchStat> = Vec::with_capacity(q.batches.len());
            for b in q.batches.iter().filter(|b| !b.instances.is_empty()) {
                out.push(SdfFlowCurveClipBatchStat {
                    scissor: b.scissor,
                    instances: b.instances.len() as u32,
                });
            }
            out.sort_by_key(|b| (b.scissor[0], b.scissor[1], b.scissor[2], b.scissor[3]));
            *SDF_FLOW_LAST_DETAILS
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .unwrap() = out;
            SDF_FLOW_LAST_DETAILS_FRAME.store(fid, Ordering::Relaxed);
        }
        Vec::new()
    }

    fn paint<'a>(
        &'a self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'a>,
        resources: &'a crate::CallbackResources,
    ) {
        if let Some(q) = resources.get::<SdfFlowCurveQueue>() {
            let Some((bi, is_leader)) = unpack_key(self.key.load(Ordering::Relaxed)) else {
                return;
            };
            if !is_leader {
                return;
            }
            let Some(b) = q.batches.get(bi) else {
                return;
            };
            let Some(bg) = b.bind_group.as_ref() else {
                return;
            };
            let tw = q.target_px[0];
            let th = q.target_px[1];
            if tw == 0 || th == 0 {
                return;
            }
            render_pass.set_viewport(0.0, 0.0, tw as f32, th as f32, 0.0, 1.0);
            let Some((cx, cy, cw, ch)) = super::clamp_scissor(&info, q.target_px) else {
                return;
            };
            let [bx, by, bw, bh] = b.scissor;
            let x0 = cx.max(bx);
            let y0 = cy.max(by);
            let x1 = cx.saturating_add(cw).min(bx.saturating_add(bw));
            let y1 = cy.saturating_add(ch).min(by.saturating_add(bh));
            let w = x1.saturating_sub(x0);
            let h = y1.saturating_sub(y0);
            if w == 0 || h == 0 {
                return;
            }
            render_pass.set_scissor_rect(x0, y0, w, h);
            render_pass.set_pipeline(&q.renderer.pipeline);
            render_pass.set_bind_group(0, bg, &[]);
            render_pass.draw(0..3, 0..b.instances.len() as u32);
        }
    }
}

pub fn create_sdf_flow_curve_callback(
    rect: egui::Rect,
    uniform: SdfFlowCurveUniform,
    frame_id: u64,
) -> egui::PaintCallback {
    crate::Callback::new_paint_callback(
        rect,
        SdfFlowCurveCallback {
            uniform,
            frame_id,
            _clip: rect,
            key: Arc::new(AtomicU64::new(0)),
        },
    )
}

