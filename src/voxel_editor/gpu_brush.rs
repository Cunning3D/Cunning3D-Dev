//! Brush cell generation for voxel tools (GPU-first async readback, no frame stall).
use bevy::prelude::{IVec3, Vec3};
use crate::nodes::gpu::runtime::GpuRuntime;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::mpsc;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct OutCell {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    _pad: i32,
}

#[derive(Clone)]
pub struct BrushAsyncResult {
    pub seq: u64,
    pub center_cell: IVec3,
    pub cells: Vec<OutCell>,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct BrushParams {
    center: [f32; 3],
    radius: f32,
    voxel_size: f32,
    max_out: u32,
    _pad0: u32,
    _pad1: u32,
}

pub struct GpuBrush {
    seq: u64,
    tx: Sender<BrushAsyncResult>,
    rx: Receiver<BrushAsyncResult>,
    ppl_sphere: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    bg: wgpu::BindGroup,
    params: wgpu::Buffer,
    out_cells: wgpu::Buffer,
    out_count: wgpu::Buffer,
    staging: wgpu::Buffer,
    cap: u32,
}

impl GpuBrush {
    #[inline]
    pub fn new(rt: &GpuRuntime) -> Self {
        let (tx, rx) = mpsc::channel();
        let dev = rt.device();
        let shader = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c3d_voxel_brush_wgsl"),
            source: wgpu::ShaderSource::Wgsl(VOXEL_BRUSH_WGSL.into()),
        });
        let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_voxel_brush_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
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
            ],
        });
        let ppl_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("c3d_voxel_brush_ppl_layout"),
            bind_group_layouts: &[&bgl],
            immediate_size: 0,
        });
        let ppl_sphere = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_voxel_brush_sphere"),
            layout: Some(&ppl_layout),
            module: &shader,
            entry_point: Some("brush_sphere"),
            compilation_options: Default::default(),
            cache: None,
        });

        let cap = 1024u32;
        let params = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_voxel_brush_params"),
            size: std::mem::size_of::<BrushParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let out_cells = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_voxel_brush_out_cells"),
            size: (cap as u64) * (std::mem::size_of::<OutCell>() as u64),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let out_count = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_voxel_brush_out_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_voxel_brush_staging"),
            size: 16 + (cap as u64) * (std::mem::size_of::<OutCell>() as u64),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3d_voxel_brush_bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: out_cells.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: out_count.as_entire_binding() },
            ],
        });

        Self { seq: 0, tx, rx, ppl_sphere, bgl, bg, params, out_cells, out_count, staging, cap }
    }

    #[inline]
    pub fn poll(&mut self) -> Option<BrushAsyncResult> {
        let mut last = None;
        while let Ok(v) = self.rx.try_recv() { last = Some(v); }
        last
    }

    fn ensure_cap(&mut self, rt: &GpuRuntime, need: u32) {
        if self.cap >= need { return; }
        let dev = rt.device();
        let cap = need.next_power_of_two().max(1024);
        self.cap = cap;
        self.out_cells = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_voxel_brush_out_cells"),
            size: (cap as u64) * (std::mem::size_of::<OutCell>() as u64),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        self.out_count = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_voxel_brush_out_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.staging = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_voxel_brush_staging"),
            size: 16 + (cap as u64) * (std::mem::size_of::<OutCell>() as u64),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3d_voxel_brush_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.out_cells.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.out_count.as_entire_binding() },
            ],
        });
    }

    pub fn gen_sphere_cells_async(
        &mut self,
        rt: &GpuRuntime,
        center: [f32; 3],
        radius: f32,
        voxel_size: f32,
        max_out: u32,
    ) -> u64 {
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;
        let vs = voxel_size.max(0.001);
        let r = radius.max(0.0);
        let center_cell = (Vec3::from_array(center) / vs).floor().as_ivec3();
        let ri = ((r / vs).ceil().max(0.0) as i32).max(0);
        let dim = (2 * ri + 1).max(0) as u32;
        let total = dim.saturating_mul(dim).saturating_mul(dim);
        let want_out = max_out.min(total).max(1);
        self.ensure_cap(rt, want_out);

        let params = BrushParams { center, radius: r, voxel_size: vs, max_out: want_out, _pad0: 0, _pad1: 0 };
        let dev = rt.device();
        let q = rt.queue();
        q.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        q.write_buffer(&self.out_count, 0, bytemuck::bytes_of(&0u32));

        let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("c3d_voxel_brush_enc"),
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("c3d_voxel_brush_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.ppl_sphere);
            pass.set_bind_group(0, &self.bg, &[]);
            let wg = (total + 255) / 256;
            pass.dispatch_workgroups(wg, 1, 1);
        }
        enc.copy_buffer_to_buffer(&self.out_count, 0, &self.staging, 0, 4);
        let cell_bytes = (want_out as u64) * (std::mem::size_of::<OutCell>() as u64);
        enc.copy_buffer_to_buffer(&self.out_cells, 0, &self.staging, 16, cell_bytes);
        q.submit([enc.finish()]);
        let _ = dev.poll(wgpu::PollType::Poll);

        let staging = self.staging.clone();
        let staging_cb = staging.clone();
        let tx = self.tx.clone();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |res| {
            if res.is_ok() {
                let slice = staging_cb.slice(..);
                let mapped = slice.get_mapped_range();
                let cnt = u32::from_le_bytes([mapped[0], mapped[1], mapped[2], mapped[3]]);
                let n = (cnt as usize).min(want_out as usize);
                let cells_bytes = &mapped[16..16 + n * std::mem::size_of::<OutCell>()];
                let cells: Vec<OutCell> = bytemuck::cast_slice(cells_bytes).to_vec();
                drop(mapped);
                staging_cb.unmap();
                let _ = tx.send(BrushAsyncResult { seq, center_cell, cells });
            } else {
                staging_cb.unmap();
            }
        });
        seq
    }

    pub fn gen_sphere_cells(
        &self,
        _rt: &GpuRuntime,
        center: [f32; 3],
        radius: f32,
        voxel_size: f32,
        max_out: u32,
    ) -> Vec<OutCell> {
        gen_sphere_cells_cpu(center, radius, voxel_size, max_out)
    }

    pub fn gen_box_cells(
        &self,
        _rt: &GpuRuntime,
        center: [f32; 3],
        half: [f32; 3],
        voxel_size: f32,
        max_out: u32,
    ) -> Vec<OutCell> {
        gen_box_cells_cpu(center, half, voxel_size, max_out)
    }
}

const VOXEL_BRUSH_WGSL: &str = r#"
struct Params { center: vec3<f32>, radius: f32, voxel_size: f32, max_out: u32, _pad0: u32, _pad1: u32 };
struct OutCell { x: i32, y: i32, z: i32, _pad: i32 };
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> out_cells: array<OutCell>;
@group(0) @binding(2) var<storage, read_write> out_count: atomic<u32>;

@compute @workgroup_size(256)
fn brush_sphere(@builtin(global_invocation_id) gid: vec3<u32>) {
  let vs = max(p.voxel_size, 0.001);
  let c = p.center / vs;
  let r = max(p.radius, 0.0) / vs;
  let r2 = r * r;
  let cc = floor(c);
  let ri = i32(ceil(r));
  let d = u32(max(2 * ri + 1, 0));
  let total = d * d * d;
  let idx = gid.x;
  if (idx >= total) { return; }
  let x = i32(idx % d);
  let y = i32((idx / d) % d);
  let z = i32(idx / (d * d));
  let mn = vec3<i32>(i32(cc.x) - ri, i32(cc.y) - ri, i32(cc.z) - ri);
  let cell = mn + vec3<i32>(x, y, z);
  let dp = (vec3<f32>(f32(cell.x) + 0.5, f32(cell.y) + 0.5, f32(cell.z) + 0.5) - c);
  if (dot(dp, dp) > r2) { return; }
  let out_i = atomicAdd(&out_count, 1u);
  if (out_i >= p.max_out) { return; }
  out_cells[out_i] = OutCell(cell.x, cell.y, cell.z, 0);
}
"#;

#[inline]
fn gen_box_cells_cpu(center: [f32; 3], half: [f32; 3], voxel_size: f32, max_out: u32) -> Vec<OutCell> {
    if max_out == 0 { return Vec::new(); }
    let vs = voxel_size.max(0.001);
    let c = Vec3::from_array(center);
    let mn = ((c - Vec3::from_array(half)) / vs).floor().as_ivec3();
    let mx = ((c + Vec3::from_array(half)) / vs).floor().as_ivec3();
    let sx = (mx.x - mn.x + 1).max(0) as u32;
    let sy = (mx.y - mn.y + 1).max(0) as u32;
    let sz = (mx.z - mn.z + 1).max(0) as u32;
    let n = sx.saturating_mul(sy).saturating_mul(sz).min(max_out) as usize;
    if n == 0 { return Vec::new(); }
    let mut out = Vec::with_capacity(n);
    for z in mn.z..=mx.z { for y in mn.y..=mx.y { for x in mn.x..=mx.x {
        out.push(OutCell { x, y, z, _pad: 0 });
        if out.len() >= max_out as usize { return out; }
    }}}
    out
}

#[inline]
fn gen_sphere_cells_cpu(center: [f32; 3], radius: f32, voxel_size: f32, max_out: u32) -> Vec<OutCell> {
    if max_out == 0 { return Vec::new(); }
    let vs = voxel_size.max(0.001);
    let inv_vs = 1.0 / vs;
    let c = Vec3::from_array(center) * inv_vs;
    let r = (radius.max(0.0) * inv_vs).max(0.0);
    let r2 = r * r;
    let cc = c.floor().as_ivec3();
    let ri = r.ceil().max(0.0) as i32;
    let mn = cc - IVec3::splat(ri);
    let mx = cc + IVec3::splat(ri);
    let sx = (mx.x - mn.x + 1).max(0) as u32;
    let sy = (mx.y - mn.y + 1).max(0) as u32;
    let sz = (mx.z - mn.z + 1).max(0) as u32;
    let n_est = sx.saturating_mul(sy).saturating_mul(sz).min(max_out) as usize;
    if n_est == 0 { return Vec::new(); }
    let mut out = Vec::with_capacity(n_est);
    for z in mn.z..=mx.z { for y in mn.y..=mx.y { for x in mn.x..=mx.x {
        let d = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5) - c;
        if d.length_squared() > r2 { continue; }
        out.push(OutCell { x, y, z, _pad: 0 });
        if out.len() >= max_out as usize { return out; }
    }}}
    out
}
