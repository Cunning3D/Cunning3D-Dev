use std::sync::{Arc, Mutex};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SdfTriUniform {
    pub p01: [f32; 4],            // p0.xy, p1.xy
    pub p2_: [f32; 4],            // p2.xy, pad
    pub fill_color: [f32; 4],
    pub border_color: [f32; 4],
    pub params: [f32; 4],         // border_width, softness, _, _
    pub screen_params: [f32; 4],  // screen_size.xy, _, _
}

const SDF_TRI_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
};

struct Uniforms {
    p01: vec4<f32>,
    p2_: vec4<f32>,
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

fn sd_triangle(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> f32 {
    // Assumes CCW order.
    let ab = b - a;
    let bc = c - b;
    let ca = a - c;
    let s0 = cross2(ab, p - a);
    let s1 = cross2(bc, p - b);
    let s2 = cross2(ca, p - c);
    let inside = (s0 >= 0.0) && (s1 >= 0.0) && (s2 >= 0.0);
    let dist = min(sd_segment(p, a, b), min(sd_segment(p, b, c), sd_segment(p, c, a)));
    return select(dist, -dist, inside);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let a = u.p01.xy;
    let b = u.p01.zw;
    let c = u.p2_.xy;
    let sd = sd_triangle(in.local_pos, a, b, c);
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

pub struct SdfTriRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

impl SdfTriRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("SDF Tri Shader"),
            source: wgpu::ShaderSource::Wgsl(SDF_TRI_SHADER.into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SDF Tri Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("SDF Tri Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SDF Tri Pipeline"),
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

    pub fn prepare_bind_group(&self, device: &wgpu::Device, uniform: SdfTriUniform) -> wgpu::BindGroup {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("SDF Tri Uniform Buffer"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SDF Tri Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        })
    }
}

pub struct SdfTriQueue {
    pub renderer: Arc<SdfTriRenderer>,
    pub bind_groups: Vec<wgpu::BindGroup>,
    pub target_px: [u32; 2],
    pub last_frame_id: u64,
}

pub struct SdfTriCallback {
    pub uniform: SdfTriUniform,
    pub frame_id: u64,
    pub index: Arc<Mutex<Option<usize>>>,
}

impl crate::CallbackTrait for SdfTriCallback {
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
        let queue = resources.entry::<SdfTriQueue>().or_insert_with(|| SdfTriQueue {
            renderer: Arc::new(SdfTriRenderer::new(device, fmt)),
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
        if let Some(queue) = resources.get::<SdfTriQueue>() {
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

pub fn create_sdf_tri_callback(rect: egui::Rect, uniform: SdfTriUniform, frame_id: u64) -> egui::PaintCallback {
    crate::Callback::new_paint_callback(rect, SdfTriCallback { uniform, frame_id, index: Arc::new(Mutex::new(None)) })
}


