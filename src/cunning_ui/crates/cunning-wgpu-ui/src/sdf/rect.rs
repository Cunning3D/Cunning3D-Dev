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
pub struct SdfRectUniform {
    pub center: [f32; 2],       // 0
    pub half_size: [f32; 2],    // 8
    pub corner_radii: [f32; 4], // 16
    pub fill_color: [f32; 4],   // 32
    pub shadow_color: [f32; 4], // 48
    pub shadow_blur: f32,       // 64
    pub _pad1: f32,             // 68
    pub shadow_offset: [f32; 2],// 72
    pub border_width: f32,      // 80
    pub _pad2: [f32; 3],        // 84
    pub border_color: [f32; 4], // 96
    pub screen_size: [f32; 2],  // 112
    pub _pad3: [f32; 2],        // 120
}

// ----------------------------------------------------------------------------
// Shader
// ----------------------------------------------------------------------------

const SDF_RECT_SHADER: &str = r#"
struct Globals {
    screen_size: vec2<f32>,
    _pad: vec2<f32>,
};

struct Instance {
    center: vec2<f32>,
    half_size: vec2<f32>,
    corner_radii: vec4<f32>, // nw, ne, se, sw
    fill_color: vec4<f32>,
    shadow_color: vec4<f32>,
    // x=shadow_blur, y=shadow_spread, z=shadow_offset.x, w=shadow_offset.y
    shadow_params: vec4<f32>,
    // x=border_width, yzw unused
    border_params: vec4<f32>,
    border_color: vec4<f32>,
    clip_min: vec2<f32>,
    clip_max: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) @interpolate(flat) inst: u32,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> inst_buf: array<Instance>;

fn position_from_screen(screen_pos: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(
        2.0 * screen_pos.x / g.screen_size.x - 1.0,
        1.0 - 2.0 * screen_pos.y / g.screen_size.y,
        0.0,
        1.0,
    );
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32, @builtin(instance_index) iid: u32) -> VertexOutput {
    let u = inst_buf[iid];
    // Expand bounds to cover border + shadow.
    let blur = max(u.shadow_params.x, 1.0);
    let spread = u.shadow_params.y;
    let off = abs(u.shadow_params.zw);
    let bw = max(u.border_params.x, 0.0);
    let ext = u.half_size + vec2<f32>(spread + blur + bw + 2.0) + off;
    let mn = u.center - ext;
    let mx = u.center + ext;
    let c = select(vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), (vid == 1u || vid == 2u || vid == 4u));
    let c2 = select(c, vec2<f32>(1.0, 1.0), (vid == 2u || vid == 4u));
    let corner = select(c2, vec2<f32>(0.0, 1.0), (vid == 5u));
    let sp = mix(mn, mx, corner);

    var out: VertexOutput;
    out.clip_position = position_from_screen(sp);
    out.local_pos = sp;
    out.inst = iid;
    return out;
}

fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r: vec4<f32>) -> f32 {
    var select_r = r.x; // nw
    if (p.x > 0.0) {
        if (p.y > 0.0) { select_r = r.z; } // se
        else { select_r = r.y; } // ne
    } else {
        if (p.y > 0.0) { select_r = r.w; } // sw
        else { select_r = r.x; } // nw
    }

    let q = abs(p) - b + select_r;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - select_r;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let u = inst_buf[in.inst];
    if (in.local_pos.x < u.clip_min.x || in.local_pos.y < u.clip_min.y || in.local_pos.x > u.clip_max.x || in.local_pos.y > u.clip_max.y) { discard; }
    let p = in.local_pos - u.center;
    let d = sd_rounded_box(p, u.half_size, u.corner_radii);
    let aa = 1.0;

    // Shadow: u.shadow_params = [blur, spread, offset.x, offset.y]
    var shadow_a = 0.0;
    if (u.shadow_color.a > 0.0 && (u.shadow_params.x > 0.0 || u.shadow_params.y > 0.0)) {
        let sp = p - u.shadow_params.zw;
        let hb = u.half_size + vec2<f32>(u.shadow_params.y);
        let rr = u.corner_radii + vec4<f32>(u.shadow_params.y);
        let sd = sd_rounded_box(sp, hb, rr);
        let blur = max(u.shadow_params.x, 1.0);
        // Softer/less "hard edge" shadow: extend AA slightly into negative range.
        shadow_a = (1.0 - smoothstep(-aa, blur, sd)) * u.shadow_color.a;
    }

    let fill_cov = 1.0 - smoothstep(-aa, aa, d);
    let fill_a = fill_cov * u.fill_color.a;

    var border_a = 0.0;
    if (u.border_params.x > 0.0) {
        let border_cov = 1.0 - smoothstep(-aa, aa, abs(d) - u.border_params.x * 0.5);
        border_a = border_cov * u.border_color.a;
    }

    // Premultiplied over: shadow -> fill -> border
    let shadow = u.shadow_color * shadow_a;
    let fill = u.fill_color * fill_a;
    let border = u.border_color * border_a;
    var out = shadow + fill * (1.0 - shadow.a);
    out = border + out * (1.0 - border.a);
    if (out.a <= 0.0) { discard; }
    return out;
}
"#;

#[derive(Clone, Copy, Debug, Default)]
pub struct SdfRectBatchStats {
    pub frame_id: u64,
    pub instances: u64,
    pub clip_regions: u64,
    pub drawcalls: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SdfRectClipBatchStat {
    pub scissor: [u32; 4],
    pub instances: u32,
}

static SDF_RECT_LAST_FRAME: AtomicU64 = AtomicU64::new(0);
static SDF_RECT_LAST_INST: AtomicU64 = AtomicU64::new(0);
static SDF_RECT_LAST_REGIONS: AtomicU64 = AtomicU64::new(0);
static SDF_RECT_LAST_DRAWCALLS: AtomicU64 = AtomicU64::new(0);
static SDF_RECT_VERBOSE_DETAILS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SDF_RECT_LAST_DETAILS_FRAME: AtomicU64 = AtomicU64::new(0);
static SDF_RECT_LAST_DETAILS: OnceLock<Mutex<Vec<SdfRectClipBatchStat>>> = OnceLock::new();

pub fn sdf_rect_last_stats() -> SdfRectBatchStats {
    SdfRectBatchStats {
        frame_id: SDF_RECT_LAST_FRAME.load(Ordering::Relaxed),
        instances: SDF_RECT_LAST_INST.load(Ordering::Relaxed),
        clip_regions: SDF_RECT_LAST_REGIONS.load(Ordering::Relaxed),
        drawcalls: SDF_RECT_LAST_DRAWCALLS.load(Ordering::Relaxed),
    }
}

pub fn sdf_rect_set_verbose_details_enabled(enabled: bool) { SDF_RECT_VERBOSE_DETAILS.store(enabled, Ordering::Relaxed); }

pub fn sdf_rect_last_batch_details() -> (u64, Vec<SdfRectClipBatchStat>) {
    let fid = SDF_RECT_LAST_DETAILS_FRAME.load(Ordering::Relaxed);
    let v = SDF_RECT_LAST_DETAILS.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap().clone();
    (fid, v)
}


// ----------------------------------------------------------------------------
// Renderer
// ----------------------------------------------------------------------------

pub struct SdfRectRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl SdfRectRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("SDF Rect Shader"),
            source: wgpu::ShaderSource::Wgsl(SDF_RECT_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SDF Rect Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SDF Rect Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SDF Rect Pipeline"),
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
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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

    pub fn prepare_bind_group(
        &self,
        device: &wgpu::Device,
        globals: [f32; 2],
        instances: &[SdfRectInstance],
    ) -> wgpu::BindGroup {
        #[repr(C)]
        #[derive(Clone, Copy, Pod, Zeroable)]
        struct Globals { screen_size: [f32; 2], _pad: [f32; 2] }
        let gbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("SDF Rect Globals"),
            contents: bytemuck::bytes_of(&Globals { screen_size: globals, _pad: [0.0; 2] }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("SDF Rect Instances"),
            contents: bytemuck::cast_slice(instances),
            usage: wgpu::BufferUsages::STORAGE,
        });

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SDF Rect Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: gbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: ibuf.as_entire_binding() },
            ],
        })
    }
}

// ----------------------------------------------------------------------------
// Callback
// ----------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SdfRectInstance {
    pub center: [f32; 2],
    pub half_size: [f32; 2],
    pub corner_radii: [f32; 4],
    pub fill_color: [f32; 4],
    pub shadow_color: [f32; 4],
    pub shadow_params: [f32; 4], // blur, spread, offx, offy
    pub border_params: [f32; 4], // border_width, 0,0,0
    pub border_color: [f32; 4],
    pub clip_min: [f32; 2],
    pub clip_max: [f32; 2],
}

pub struct SdfQueue {
    pub renderer: Arc<SdfRectRenderer>,
    pub target_px: [u32; 2],
    pub ppp: f32,
    pub last_frame_id: u64,
    batches: Vec<Batch>,
    batch_map: HashMap<u64, usize>,
    pub last_upload_frame_id: u64,
    last_seq_id: u64,
}

struct Batch {
    scissor: [u32; 4],
    instances: Vec<SdfRectInstance>,
    bind_group: Option<wgpu::BindGroup>,
    leader: bool,
}

pub struct SdfRectCallback {
    pub uniform: SdfRectUniform,
    pub frame_id: u64,
    pub _clip: egui::Rect,
    pub key: Arc<AtomicU64>, // packed: [valid:1][batch_idx:62][is_leader:1]
}

#[inline]
fn pack_key(batch_idx: usize, is_leader: bool) -> u64 { (1u64 << 63) | ((batch_idx as u64) << 1) | (is_leader as u64) }

#[inline]
fn unpack_key(v: u64) -> Option<(usize, bool)> { if (v >> 63) == 0 { None } else { Some(((((v & ((1u64 << 63) - 1)) >> 1) as usize), (v & 1) != 0)) } }

impl crate::CallbackTrait for SdfRectCallback {
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
        let queue = resources.entry::<SdfQueue>().or_insert_with(|| {
            SdfQueue {
                renderer: Arc::new(SdfRectRenderer::new(device, fmt)),
                target_px: [1, 1],
                ppp: 1.0,
                last_frame_id: 0,
                batches: Vec::with_capacity(64),
                batch_map: HashMap::with_capacity(64),
                last_upload_frame_id: 0,
                last_seq_id: 0,
            }
        });
        queue.target_px = screen_descriptor.size_in_pixels;
        queue.ppp = screen_descriptor.pixels_per_point.max(1e-6);

        if fid != queue.last_frame_id {
            queue.batches.clear();
            queue.batch_map.clear();
            queue.last_frame_id = fid;
            queue.last_seq_id = 0;
        }

        // Order-preserving batching: only across adjacent primitives.
        if seq != queue.last_seq_id.saturating_add(1) {
            queue.batch_map.clear();
        }
        queue.last_seq_id = seq;

        let (min_x, min_y, w, h) = {
            let tw = queue.target_px[0];
            let th = queue.target_px[1];
            if tw == 0 || th == 0 { return Vec::new(); }
            let rect = self._clip;
            let ppp = queue.ppp.max(1e-6);
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
        
        let sc = [min_x, min_y, w, h];
        let k = ((sc[0] as u64) << 48) ^ ((sc[1] as u64) << 32) ^ ((sc[2] as u64) << 16) ^ (sc[3] as u64);
        let bi = *queue.batch_map.entry(k).or_insert_with(|| {
            let bi = queue.batches.len();
            queue.batches.push(Batch { scissor: sc, instances: Vec::with_capacity(32), bind_group: None, leader: true });
            bi
        });
        let b = &mut queue.batches[bi];
        let is_leader = b.leader;
        b.leader = false;
        let u = self.uniform;
        b.instances.push(SdfRectInstance {
            center: u.center,
            half_size: u.half_size,
            corner_radii: u.corner_radii,
            fill_color: u.fill_color,
            shadow_color: u.shadow_color,
            shadow_params: [u.shadow_blur, u._pad1, u.shadow_offset[0], u.shadow_offset[1]],
            border_params: [u.border_width, 0.0, 0.0, 0.0],
            border_color: u.border_color,
            clip_min: [self._clip.min.x, self._clip.min.y],
            clip_max: [self._clip.max.x, self._clip.max.y],
        });
        
        self.key.store(pack_key(bi, is_leader), Ordering::Relaxed);
        
        Vec::new()
    }

    fn finish_prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut crate::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let fid = resources.get::<super::SdfFrameId>().map(|v| v.0).unwrap_or(self.frame_id);
        let Some(queue) = resources.get_mut::<SdfQueue>() else { return Vec::new(); };
        if fid == queue.last_upload_frame_id { return Vec::new(); }
        queue.last_upload_frame_id = fid;
        let globals = [queue.target_px[0] as f32 / queue.ppp, queue.target_px[1] as f32 / queue.ppp];
        for b in &mut queue.batches {
            if b.instances.is_empty() { b.bind_group = None; continue; }
            b.bind_group = Some(queue.renderer.prepare_bind_group(device, globals, &b.instances));
        }
        let inst_total: u64 = queue.batches.iter().map(|b| b.instances.len() as u64).sum();
        let regions: u64 = queue.batches.iter().filter(|b| !b.instances.is_empty()).count() as u64;
        SDF_RECT_LAST_FRAME.store(fid, Ordering::Relaxed);
        SDF_RECT_LAST_INST.store(inst_total, Ordering::Relaxed);
        SDF_RECT_LAST_REGIONS.store(regions, Ordering::Relaxed);
        SDF_RECT_LAST_DRAWCALLS.store(regions, Ordering::Relaxed);
        if SDF_RECT_VERBOSE_DETAILS.load(Ordering::Relaxed) {
            let mut out: Vec<SdfRectClipBatchStat> = Vec::with_capacity(queue.batches.len());
            for b in queue.batches.iter().filter(|b| !b.instances.is_empty()) {
                out.push(SdfRectClipBatchStat { scissor: b.scissor, instances: b.instances.len() as u32 });
            }
            out.sort_by_key(|b| (b.scissor[0], b.scissor[1], b.scissor[2], b.scissor[3]));
            *SDF_RECT_LAST_DETAILS.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap() = out;
            SDF_RECT_LAST_DETAILS_FRAME.store(fid, Ordering::Relaxed);
        }
        Vec::new()
    }

    fn paint<'a>(&'a self, info: egui::PaintCallbackInfo, render_pass: &mut wgpu::RenderPass<'a>, resources: &'a crate::CallbackResources) {
        if let Some(queue) = resources.get::<SdfQueue>() {
            let Some((bi, is_leader)) = unpack_key(self.key.load(Ordering::Relaxed)) else { return; };
            if !is_leader { return; }
            let Some(b) = queue.batches.get(bi) else { return; };
            let Some(bg) = b.bind_group.as_ref() else { return; };

            let tw = queue.target_px[0];
            let th = queue.target_px[1];
            if tw == 0 || th == 0 { return; }
            // egui-wgpu sets viewport to callback.rect; our shaders use full-screen coordinates.
            render_pass.set_viewport(0.0, 0.0, tw as f32, th as f32, 0.0, 1.0);
            let Some((min_x, min_y, w, h)) = super::clamp_scissor(&info, queue.target_px) else { return; };

            render_pass.set_scissor_rect(min_x, min_y, w, h);
            render_pass.set_pipeline(&queue.renderer.pipeline);
            render_pass.set_bind_group(0, bg, &[]);
            render_pass.draw(0..6, 0..b.instances.len() as u32);
        }
    }
}

pub fn create_sdf_rect_callback(
    rect: egui::Rect,
    uniform: SdfRectUniform,
    frame_id: u64,
) -> egui::PaintCallback {
    crate::Callback::new_paint_callback(
        rect,
        SdfRectCallback {
            uniform,
            frame_id,
            _clip: rect,
            key: Arc::new(AtomicU64::new(0)),
        },
    )
}
