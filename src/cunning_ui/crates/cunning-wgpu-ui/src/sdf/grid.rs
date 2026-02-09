use std::{collections::HashMap, sync::Arc};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SdfGridUniform {
    // Packed as vec4s to avoid std140 padding surprises:
    // rect_min_size: xy = rect_min, zw = rect_size
    pub rect_min_size: [f32; 4],
    // pan_grid: xy = pan, z = grid_size, w = line_width
    pub pan_grid: [f32; 4],
    pub color: [f32; 4],
    // time_hover: x=time, y=hover_state, z=major_alpha_mul, w=ripple_phase (0=off, >0=active)
    pub time_hover: [f32; 4],
    // screen_params: xy=screen_size, zw=ripple_center (graph coords)
    pub screen_params: [f32; 4],
    // ripple_params: x=ripple_intensity (0-1), y=ripple_speed, z=ripple_wavelength, w=ripple_decay
    pub ripple_params: [f32; 4],
}

const SDF_GRID_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) @interpolate(flat) inst: u32,
};

struct Uniforms {
    rect_min_size: vec4<f32>,
    pan_grid: vec4<f32>,
    color: vec4<f32>,
    time_hover: vec4<f32>,
    screen_params: vec4<f32>,
    ripple_params: vec4<f32>,
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
    out.local_pos = uv * u.screen_params.xy;
    out.inst = iid;
    return out;
}

fn grid_pattern(p: vec2<f32>, grid: f32, w: f32, hover_state: f32) -> f32 {
    let g = max(1e-4, grid);
    let lw = max(0.5, w);
    let fx = fract(p.x / g);
    let fy = fract(p.y / g);
    let dx = min(fx, 1.0 - fx) * g;
    let dy = min(fy, 1.0 - fy) * g;
    let d_line = min(dx, dy);
    let d_dot = length(vec2<f32>(dx, dy)); 
    let dot_radius = w * 2.0;
    let mask_dot = 1.0 - smoothstep(dot_radius - 0.5, dot_radius + 0.5, d_dot);
    let mask_line = 1.0 - smoothstep(lw * 0.5 - 0.5, lw * 0.5 + 0.5, d_line);
    let lines_visibility = smoothstep(0.0, 1.0, hover_state);
    return max(mask_dot, mask_line * lines_visibility);
}

// Ripple wave function: returns brightness boost (0-1) based on distance from center
fn ripple_wave(p: vec2<f32>, center: vec2<f32>, phase: f32, speed: f32, wavelength: f32, decay: f32) -> f32 {
    let d = length(p - center);
    let wave_front = phase * speed;
    let dist_from_front = d - wave_front;
    // Multiple concentric rings
    let ring = sin(dist_from_front / wavelength * 6.283185) * 0.5 + 0.5;
    // Fade out behind wave front and with distance
    let behind = smoothstep(0.0, wavelength * 2.0, -dist_from_front);
    let fade = exp(-d * decay * 0.001);
    return ring * behind * fade;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let u = inst_buf[in.inst];
    let rect_min = u.rect_min_size.xy;
    let rect_size = u.rect_min_size.zw;
    let local = in.local_pos - rect_min;
    if (local.x < 0.0 || local.y < 0.0 || local.x > rect_size.x || local.y > rect_size.y) { discard; }

    let pan = u.pan_grid.xy;
    let grid_size = u.pan_grid.z;
    let line_width = u.pan_grid.w;
    let time = u.time_hover.x;
    let hover_state = u.time_hover.y;
    let major_alpha_mul = max(0.0, u.time_hover.z);
    let ripple_phase = u.time_hover.w;
    let ripple_center = u.screen_params.zw;
    let ripple_intensity = u.ripple_params.x;
    let ripple_speed = u.ripple_params.y;
    let ripple_wavelength = u.ripple_params.z;
    let ripple_decay = u.ripple_params.w;
    let major_ratio = 5.0;
    let major_width_mul = 1.5;
    let p = local + pan;
    
    // Base Grid
    var alpha = grid_pattern(p, grid_size, line_width, hover_state);
    let alpha_major = grid_pattern(p, grid_size * major_ratio, line_width * major_width_mul, hover_state) * major_alpha_mul;
    alpha = max(alpha, alpha_major);

    // Ripple effect (deep thinking mode)
    var ripple_boost = 0.0;
    var color_shift = vec3<f32>(0.0, 0.0, 0.0);
    if (ripple_intensity > 0.001) {
        // Convert local screen pos to graph coords for ripple center comparison
        let graph_p = local;
        let r = ripple_wave(graph_p, ripple_center, ripple_phase, ripple_speed, ripple_wavelength, ripple_decay);
        ripple_boost = r * ripple_intensity * 0.6;
        // Color shift: blue -> purple hue in ripple
        color_shift = vec3<f32>(0.2, 0.1, 0.4) * r * ripple_intensity;
    }

    let final_alpha = (alpha + ripple_boost) * u.color.a;
    if (final_alpha <= 0.0) { discard; }
    let final_color = u.color.rgb + color_shift;
    return vec4<f32>(final_color, min(final_alpha, 1.0));
}
"#;

#[derive(Clone, Copy, Debug, Default)]
pub struct SdfGridBatchStats { pub frame_id: u64, pub instances: u64, pub clip_regions: u64, pub drawcalls: u64 }

#[derive(Clone, Copy, Debug, Default)]
pub struct SdfGridClipBatchStat { pub scissor: [u32; 4], pub instances: u32 }

static SDF_GRID_LAST_FRAME: AtomicU64 = AtomicU64::new(0);
static SDF_GRID_LAST_INST: AtomicU64 = AtomicU64::new(0);
static SDF_GRID_LAST_REGIONS: AtomicU64 = AtomicU64::new(0);
static SDF_GRID_LAST_DRAWCALLS: AtomicU64 = AtomicU64::new(0);
static SDF_GRID_VERBOSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SDF_GRID_LAST_DETAILS_FRAME: AtomicU64 = AtomicU64::new(0);
static SDF_GRID_LAST_DETAILS: OnceLock<Mutex<Vec<SdfGridClipBatchStat>>> = OnceLock::new();

pub fn sdf_grid_last_stats() -> SdfGridBatchStats {
    SdfGridBatchStats {
        frame_id: SDF_GRID_LAST_FRAME.load(Ordering::Relaxed),
        instances: SDF_GRID_LAST_INST.load(Ordering::Relaxed),
        clip_regions: SDF_GRID_LAST_REGIONS.load(Ordering::Relaxed),
        drawcalls: SDF_GRID_LAST_DRAWCALLS.load(Ordering::Relaxed),
    }
}

pub fn sdf_grid_set_verbose_details_enabled(enabled: bool) { SDF_GRID_VERBOSE.store(enabled, Ordering::Relaxed); }

pub fn sdf_grid_last_batch_details() -> (u64, Vec<SdfGridClipBatchStat>) {
    let fid = SDF_GRID_LAST_DETAILS_FRAME.load(Ordering::Relaxed);
    let v = SDF_GRID_LAST_DETAILS.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap().clone();
    (fid, v)
}

pub struct SdfGridRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl SdfGridRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("SDF Grid Shader"),
            source: wgpu::ShaderSource::Wgsl(SDF_GRID_SHADER.into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SDF Grid Bind Group Layout"),
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
            label: Some("SDF Grid Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SDF Grid Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
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
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, bind_group_layout }
    }

    pub fn prepare_bind_group(&self, device: &wgpu::Device, instances: &[SdfGridUniform]) -> wgpu::BindGroup {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("SDF Grid Instances"),
            contents: bytemuck::cast_slice(instances),
            usage: wgpu::BufferUsages::STORAGE,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SDF Grid Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        })
    }
}

struct Batch {
    scissor: [u32; 4],
    instances: Vec<SdfGridUniform>,
    bind_group: Option<wgpu::BindGroup>,
    leader: bool,
}

struct SdfGridQueue {
    pub renderer: Arc<SdfGridRenderer>,
    batches: Vec<Batch>,
    batch_map: HashMap<u64, usize>,
    pub target_px: [u32; 2],
    pub last_frame_id: u64,
    pub last_upload_frame_id: u64,
    last_seq_id: u64,
}

pub struct SdfGridCallback {
    pub uniform: SdfGridUniform,
    pub frame_id: u64,
    pub _clip: egui::Rect,
    pub key: Arc<AtomicU64>, // packed: [valid:1][batch_idx:62][is_leader:1]
}

#[inline]
fn pack_key(batch_idx: usize, is_leader: bool) -> u64 { (1u64 << 63) | ((batch_idx as u64) << 1) | (is_leader as u64) }

#[inline]
fn unpack_key(v: u64) -> Option<(usize, bool)> { if (v >> 63) == 0 { None } else { Some((((v & ((1u64 << 63) - 1)) >> 1) as usize, (v & 1) != 0)) } }

impl crate::CallbackTrait for SdfGridCallback {
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
        let queue = resources.entry::<SdfGridQueue>().or_insert_with(|| SdfGridQueue {
            renderer: Arc::new(SdfGridRenderer::new(device, fmt)),
            batches: Vec::with_capacity(64),
            batch_map: HashMap::with_capacity(64),
            target_px: [1, 1],
            last_frame_id: 0,
            last_upload_frame_id: 0,
            last_seq_id: 0,
        });
        queue.target_px = screen_descriptor.size_in_pixels;
        let ppp = screen_descriptor.pixels_per_point.max(1e-6);
        if fid != queue.last_frame_id { queue.batches.clear(); queue.batch_map.clear(); queue.last_frame_id = fid; queue.last_seq_id = 0; }

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
            queue.batches.push(Batch { scissor: [min_x, min_y, w, h], instances: Vec::with_capacity(8), bind_group: None, leader: true });
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
        let Some(q) = resources.get_mut::<SdfGridQueue>() else { return Vec::new(); };
        if fid == q.last_upload_frame_id { return Vec::new(); }
        q.last_upload_frame_id = fid;
        for b in &mut q.batches {
            if b.instances.is_empty() { b.bind_group = None; continue; }
            b.bind_group = Some(q.renderer.prepare_bind_group(device, &b.instances));
        }
        let inst_total: u64 = q.batches.iter().map(|b| b.instances.len() as u64).sum();
        let regions: u64 = q.batches.iter().filter(|b| !b.instances.is_empty()).count() as u64;
        SDF_GRID_LAST_FRAME.store(fid, Ordering::Relaxed);
        SDF_GRID_LAST_INST.store(inst_total, Ordering::Relaxed);
        SDF_GRID_LAST_REGIONS.store(regions, Ordering::Relaxed);
        SDF_GRID_LAST_DRAWCALLS.store(regions, Ordering::Relaxed);
        if SDF_GRID_VERBOSE.load(Ordering::Relaxed) {
            let mut out: Vec<SdfGridClipBatchStat> = Vec::with_capacity(q.batches.len());
            for b in q.batches.iter().filter(|b| !b.instances.is_empty()) {
                out.push(SdfGridClipBatchStat { scissor: b.scissor, instances: b.instances.len() as u32 });
            }
            out.sort_by_key(|b| (b.scissor[0], b.scissor[1], b.scissor[2], b.scissor[3]));
            *SDF_GRID_LAST_DETAILS.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap() = out;
            SDF_GRID_LAST_DETAILS_FRAME.store(fid, Ordering::Relaxed);
        }
        Vec::new()
    }

    fn paint<'a>(&'a self, info: egui::PaintCallbackInfo, render_pass: &mut wgpu::RenderPass<'a>, resources: &'a crate::CallbackResources) {
        if let Some(q) = resources.get::<SdfGridQueue>() {
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

pub fn create_sdf_grid_callback(rect: egui::Rect, uniform: SdfGridUniform, frame_id: u64) -> egui::PaintCallback {
    crate::Callback::new_paint_callback(rect, SdfGridCallback { uniform, frame_id, _clip: rect, key: Arc::new(AtomicU64::new(0)) })
}


