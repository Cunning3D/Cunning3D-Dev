use std::sync::{Arc, Mutex};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SdfQuadUniform {
    pub p01: [f32; 4],
    pub p23: [f32; 4],
    pub fill_color: [f32; 4],
    pub border_color: [f32; 4],
    pub params: [f32; 4],        // border_width, softness, _, _
    pub screen_params: [f32; 4], // screen_size.xy, _, _
}

const SDF_QUAD_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
};

struct Uniforms {
    p01: vec4<f32>,
    p23: vec4<f32>,
    fill_color: vec4<f32>,
    border_color: vec4<f32>,
    params: vec4<f32>,        // x=border_width, y=softness
    screen_params: vec4<f32>, // xy=screen_size
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    let x = f32((in_vertex_index << 1u) & 2u);
    let y = f32(in_vertex_index & 2u);
    let pos = vec2<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0);
    var out: VertexOutput;
    out.clip_position = vec4<f32>(pos, 0.0, 1.0);
    let uv = vec2<f32>(x, y);
    out.local_pos = uv * u.screen_params.xy;
    return out;
}

fn cross2(a: vec2<f32>, b: vec2<f32>) -> f32 { return a.x * b.y - a.y * b.x; }

fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(1e-6, dot(ba, ba)), 0.0, 1.0);
    return length(pa - ba * h);
}

fn sd_convex_quad(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, d: vec2<f32>) -> f32 {
    // Assumes CCW order.
    let ab = b - a; let bc = c - b; let cd = d - c; let da = a - d;
    let s0 = cross2(ab, p - a);
    let s1 = cross2(bc, p - b);
    let s2 = cross2(cd, p - c);
    let s3 = cross2(da, p - d);
    let inside = (s0 >= 0.0) && (s1 >= 0.0) && (s2 >= 0.0) && (s3 >= 0.0);
    let dist = min(min(sd_segment(p, a, b), sd_segment(p, b, c)), min(sd_segment(p, c, d), sd_segment(p, d, a)));
    return select(dist, -dist, inside);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let a = u.p01.xy;
    let b = u.p01.zw;
    let c = u.p23.xy;
    let d = u.p23.zw;
    let sd = sd_convex_quad(in.local_pos, a, b, c, d);
    let aa = max(0.5, u.params.y);
    let fill_cov = 1.0 - smoothstep(-aa, aa, sd);
    let fill_a = fill_cov * u.fill_color.a;
    var border_a = 0.0;
    if (u.params.x > 0.0) {
        let bw = u.params.x;
        let border_cov = 1.0 - smoothstep(-aa, aa, abs(sd) - bw * 0.5);
        border_a = border_cov * u.border_color.a;
    }
    let out_a = border_a + fill_a * (1.0 - border_a);
    if (out_a <= 0.0) { discard; }
    let out_rgb = (u.border_color.rgb * border_a + u.fill_color.rgb * fill_a * (1.0 - border_a)) / out_a;
    return vec4<f32>(out_rgb, out_a);
}
"#;

pub struct SdfQuadRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl SdfQuadRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("SDF Quad Shader"),
            source: wgpu::ShaderSource::Wgsl(SDF_QUAD_SHADER.into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SDF Quad Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SDF Quad Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SDF Quad Pipeline"),
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

    pub fn prepare_bind_group(&self, device: &wgpu::Device, uniform: SdfQuadUniform) -> wgpu::BindGroup {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("SDF Quad Uniform Buffer"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SDF Quad Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        })
    }
}

pub struct SdfQuadQueue {
    pub renderer: Arc<SdfQuadRenderer>,
    pub bind_groups: Vec<wgpu::BindGroup>,
    pub target_px: [u32; 2],
    pub last_frame_id: u64,
}

pub struct SdfQuadCallback {
    pub uniform: SdfQuadUniform,
    pub frame_id: u64,
    pub index: Arc<Mutex<Option<usize>>>,
}

impl crate::CallbackTrait for SdfQuadCallback {
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
        let queue = resources.entry::<SdfQuadQueue>().or_insert_with(|| SdfQuadQueue {
            renderer: Arc::new(SdfQuadRenderer::new(device, fmt)),
            bind_groups: Vec::with_capacity(256),
            target_px: [1, 1],
            last_frame_id: 0,
        });
        queue.target_px = screen_descriptor.size_in_pixels;
        if fid != queue.last_frame_id { queue.bind_groups.clear(); queue.last_frame_id = fid; }
        let bg = queue.renderer.prepare_bind_group(device, self.uniform);
        let idx = queue.bind_groups.len();
        queue.bind_groups.push(bg);
        *self.index.lock().unwrap() = Some(idx);
        Vec::new()
    }

    fn paint<'a>(&'a self, info: egui::PaintCallbackInfo, render_pass: &mut wgpu::RenderPass<'a>, resources: &'a crate::CallbackResources) {
        if let Some(queue) = resources.get::<SdfQuadQueue>() {
            if let Some(idx) = *self.index.lock().unwrap() {
                if let Some(bg) = queue.bind_groups.get(idx) {
                    if let Some((x, y, w, h)) = super::clamp_scissor(&info, queue.target_px) { render_pass.set_scissor_rect(x, y, w, h); }
                    render_pass.set_pipeline(&queue.renderer.pipeline);
                    render_pass.set_bind_group(0, bg, &[]);
                    render_pass.draw(0..3, 0..1);
                }
            }
        }
    }
}

pub fn create_sdf_quad_callback(rect: egui::Rect, uniform: SdfQuadUniform, frame_id: u64) -> egui::PaintCallback {
    crate::Callback::new_paint_callback(rect, SdfQuadCallback { uniform, frame_id, index: Arc::new(Mutex::new(None)) })
}



