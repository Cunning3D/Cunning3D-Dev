use std::{collections::HashMap, sync::Arc};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use std::sync::atomic::{AtomicU64, Ordering};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SdfEllipseUniform {
    pub center: [f32; 2],
    pub radius: [f32; 2],
    pub border_width: f32,
    pub softness: f32,
    pub _pad_pre_color: [f32; 2], // align vec4 (WGSL) after two f32
    pub fill_color: [f32; 4],
    pub border_color: [f32; 4],
    pub screen_size: [f32; 2],
    pub _pad0: [f32; 2],
    pub _pad1: [f32; 2],
    pub _pad2: [f32; 2], // WGSL struct size rounds up to 96 bytes
}

const SDF_ELLIPSE_SHADER: &str = r#"
struct VertexOutput { @builtin(position) clip_position: vec4<f32>, @location(0) local_pos: vec2<f32>, @location(1) @interpolate(flat) inst: u32, };
// Must match Rust `SdfEllipseUniform` (std430 rules for storage buffers).
struct Uniforms {
    center: vec2<f32>,
    radius: vec2<f32>,
    border_width: f32,
    softness: f32,
    _pad_pre_color: vec2<f32>,
    fill_color: vec4<f32>,
    border_color: vec4<f32>,
    screen_size: vec2<f32>,
    _pad0: vec2<f32>,
    _pad1: vec2<f32>,
    _pad2: vec2<f32>,
};
@group(0) @binding(0) var<storage, read> inst_buf: array<Uniforms>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32, @builtin(instance_index) iid: u32) -> VertexOutput {
    let u = inst_buf[iid];
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    let pos = vec2<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0);
    var out: VertexOutput;
    out.clip_position = vec4<f32>(pos, 0.0, 1.0);
    out.local_pos = vec2<f32>(x, y) * u.screen_size;
    out.inst = iid;
    return out;
}

fn ellipse_sd(p: vec2<f32>, r: vec2<f32>) -> f32 {
    let rr = max(r, vec2<f32>(1e-6, 1e-6));
    let q = p / rr;
    let k = length(q) - 1.0;
    return k * min(rr.x, rr.y);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let u = inst_buf[in.inst];
    let aa = max(0.5, u.softness);
    let r0 = max(u.radius, vec2<f32>(0.0, 0.0));
    let bw = clamp(u.border_width, 0.0, min(r0.x, r0.y));
    let r1 = max(r0 - vec2<f32>(bw, bw), vec2<f32>(0.0, 0.0));

    let p = in.local_pos - u.center;
    let d0 = ellipse_sd(p, r0);
    let outer = 1.0 - smoothstep(-aa, aa, d0);
    if (outer <= 0.0) { discard; }

    let d1 = ellipse_sd(p, r1);
    let inner = 1.0 - smoothstep(-aa, aa, d1);

    let fill_a = inner;
    let border_a = (outer - inner);
    
    // Inputs are Linear Premultiplied. Modulate by coverage.
    let fill = u.fill_color * fill_a;
    let border = u.border_color * border_a;
    
    // Composite border over fill
    let out = border + fill * (1.0 - border.a);
    if (out.a <= 0.0) { discard; }
    return out;
}
"#;

pub struct SdfEllipseRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl SdfEllipseRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("SDF Ellipse Shader"), source: wgpu::ShaderSource::Wgsl(SDF_ELLIPSE_SHADER.into()) });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SDF Ellipse Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("SDF Ellipse Pipeline Layout"), bind_group_layouts: &[&bind_group_layout], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SDF Ellipse Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, bind_group_layout }
    }

    pub fn prepare_bind_group(&self, device: &wgpu::Device, instances: &[SdfEllipseUniform]) -> wgpu::BindGroup {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("SDF Ellipse Instances"),
            contents: bytemuck::cast_slice(instances),
            usage: wgpu::BufferUsages::STORAGE,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SDF Ellipse Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        })
    }
}

struct Batch { scissor: [u32; 4], instances: Vec<SdfEllipseUniform>, bind_group: Option<wgpu::BindGroup>, leader: bool }
struct SdfEllipseQueue { renderer: Arc<SdfEllipseRenderer>, batches: Vec<Batch>, batch_map: HashMap<u64, usize>, target_px: [u32; 2], last_frame_id: u64, last_upload_frame_id: u64 }

pub struct SdfEllipseCallback { pub uniform: SdfEllipseUniform, pub frame_id: u64, pub _clip: egui::Rect, pub key: Arc<AtomicU64> }

#[inline]
fn pack_key(batch_idx: usize, is_leader: bool) -> u64 { (1u64 << 63) | ((batch_idx as u64) << 1) | (is_leader as u64) }

#[inline]
fn unpack_key(v: u64) -> Option<(usize, bool)> { if (v >> 63) == 0 { None } else { Some((((v & ((1u64 << 63) - 1)) >> 1) as usize, (v & 1) != 0)) } }

impl crate::CallbackTrait for SdfEllipseCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        screen_descriptor: &crate::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut crate::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let fid = resources.get::<super::SdfFrameId>().map(|v| v.0).unwrap_or(self.frame_id);
        let fmt = super::target_format(resources);
        let q = resources.entry::<SdfEllipseQueue>().or_insert_with(|| SdfEllipseQueue { renderer: Arc::new(SdfEllipseRenderer::new(device, fmt)), batches: Vec::with_capacity(128), batch_map: HashMap::with_capacity(128), target_px: [1, 1], last_frame_id: 0, last_upload_frame_id: 0 });
        q.target_px = screen_descriptor.size_in_pixels;
        let ppp = screen_descriptor.pixels_per_point.max(1e-6);
        if fid != q.last_frame_id { q.batches.clear(); q.batch_map.clear(); q.last_frame_id = fid; q.last_upload_frame_id = 0; }
        let (min_x, min_y, w, h) = {
            let tw = q.target_px[0];
            let th = q.target_px[1];
            if tw == 0 || th == 0 { return Vec::new(); }
            let rect = self._clip;
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
        let k = ((min_x.min(0xFFFF) as u64) << 48)
            | ((min_y.min(0xFFFF) as u64) << 32)
            | ((w.min(0xFFFF) as u64) << 16)
            | (h.min(0xFFFF) as u64);
        let bi = *q.batch_map.entry(k).or_insert_with(|| {
            let idx = q.batches.len();
            q.batches.push(Batch { scissor: [min_x, min_y, w, h], instances: Vec::with_capacity(128), bind_group: None, leader: true });
            idx
        });
        let b = &mut q.batches[bi];
        let is_leader = b.leader;
        b.leader = false;
        b.instances.push(self.uniform);
        self.key.store(pack_key(bi, is_leader), Ordering::Relaxed);
        Vec::new()
    }

    fn finish_prepare(&self, device: &wgpu::Device, _queue: &wgpu::Queue, _egui_encoder: &mut wgpu::CommandEncoder, resources: &mut crate::CallbackResources) -> Vec<wgpu::CommandBuffer> {
        let fid = resources.get::<super::SdfFrameId>().map(|v| v.0).unwrap_or(self.frame_id);
        let Some(q) = resources.get_mut::<SdfEllipseQueue>() else { return Vec::new(); };
        if fid == q.last_upload_frame_id { return Vec::new(); }
        q.last_upload_frame_id = fid;
        for b in &mut q.batches {
            if b.instances.is_empty() { b.bind_group = None; continue; }
            b.bind_group = Some(q.renderer.prepare_bind_group(device, &b.instances));
        }
        Vec::new()
    }

    fn paint<'a>(&'a self, info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'a>, res: &'a crate::CallbackResources) {
        let Some(q) = res.get::<SdfEllipseQueue>() else { return; };
        let Some((bi, is_leader)) = unpack_key(self.key.load(Ordering::Relaxed)) else { return; };
        if !is_leader { return; }
        let Some(b) = q.batches.get(bi) else { return; };
        let Some(bg) = b.bind_group.as_ref() else { return; };
        let [x, y, mut w, mut h] = b.scissor;
        let tw = q.target_px[0];
        let th = q.target_px[1];
        if x >= tw || y >= th { return; }
        if w == 0 || h == 0 { return; }
        if x.saturating_add(w) > tw { w = tw - x; }
        if y.saturating_add(h) > th { h = th - y; }
        if w == 0 || h == 0 { return; }
        let _ = info;
        pass.set_scissor_rect(x, y, w, h);
        pass.set_pipeline(&q.renderer.pipeline);
        pass.set_bind_group(0, bg, &[]);
        pass.draw(0..3, 0..b.instances.len() as u32);
    }
}

pub fn create_sdf_ellipse_callback(rect: egui::Rect, uniform: SdfEllipseUniform, frame_id: u64) -> egui::PaintCallback {
    crate::Callback::new_paint_callback(rect, SdfEllipseCallback { uniform, frame_id, _clip: rect, key: Arc::new(AtomicU64::new(0)) })
}


