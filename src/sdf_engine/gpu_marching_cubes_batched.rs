//! Batched GPU Marching Cubes over a chunk pool + work-item list.
//!
//! This is the intended "real" hot-path extractor:
//! - One workgroup per chunk (`workgroup_id.x` selects a work item).
//! - Two-pass per workgroup: count vertices then emit, using a workgroup scan.
//! - Only one global atomic allocation per chunk (not per triangle).
//!
//! Output is an append-style `out_verts` with `out_count`. A later render stage
//! can draw this directly via indirect args (GPU-only), or you can read back for debug/export.

use bevy::prelude::IVec3;
use bytemuck::{Pod, Zeroable};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::mpsc;
use wgpu::util::DeviceExt;

use crate::nodes::gpu::runtime::GpuRuntime;
use crate::sdf_engine::gpu_pool::{SdfGpuPool, SdfWorkItem};

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct McBatchParams {
    pub voxel_size: f32,
    pub iso_value: f32,
    pub background_value: f32,
    pub invert: u32,
    pub work_items_count: u32,
    pub max_vertices: u32,
    pub _pad0: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct McVertex {
    pub position: [f32; 4],
    pub normal: [f32; 4],
}

#[derive(Clone)]
pub struct McBatchAsyncResult {
    pub seq: u64,
    pub vertices: Vec<McVertex>,
}

pub struct GpuMarchingCubesBatched {
    seq: u64,
    tx: Sender<McBatchAsyncResult>,
    rx: Receiver<McBatchAsyncResult>,

    ppl: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,

    args_ppl: wgpu::ComputePipeline,
    args_bgl: wgpu::BindGroupLayout,

    edge_table: wgpu::Buffer,
    tri_table: wgpu::Buffer,

    params: wgpu::Buffer,
    out_verts: wgpu::Buffer,
    out_count: wgpu::Buffer,
    indirect_args: wgpu::Buffer,
    staging: wgpu::Buffer,
    cap_verts: u32,
}

impl GpuMarchingCubesBatched {
    pub fn new(rt: &GpuRuntime) -> Self {
        let (tx, rx) = mpsc::channel();
        let dev = rt.device();

        let edge_table_u32: Vec<u32> = marching_cubes::tables::EDGE_TABLE
            .iter()
            .map(|&v| v as u16 as u32)
            .collect();
        let tri_table_i32: Vec<i32> = marching_cubes::tables::TRI_TABLE
            .iter()
            .flat_map(|row| row.iter().copied())
            .map(|v| v as i32)
            .collect();

        let edge_table = dev.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("c3d_sdf_mc_batch_edge_table"),
            contents: bytemuck::cast_slice(&edge_table_u32),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let tri_table = dev.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("c3d_sdf_mc_batch_tri_table"),
            contents: bytemuck::cast_slice(&tri_table_i32),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c3d_sdf_mc_batch_wgsl"),
            source: wgpu::ShaderSource::Wgsl(SDF_MC_BATCH_WGSL.into()),
        });

        let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_sdf_mc_batch_bgl"),
            entries: &[
                // params
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
                // chunk_pool (read)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // work_items (read)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // out_verts (write)
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // out_count (atomic)
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // edge_table
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // tri_table
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pl = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("c3d_sdf_mc_batch_pl"),
            bind_group_layouts: &[&bgl],
            immediate_size: 0,
        });
        let ppl = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_sdf_mc_batch_ppl"),
            layout: Some(&pl),
            module: &sm,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let args_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_sdf_mc_batch_args_bgl"),
            entries: &[
                // out_count
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // out_args
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
            ],
        });
        let args_sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c3d_sdf_mc_batch_args_wgsl"),
            source: wgpu::ShaderSource::Wgsl(SDF_MC_BATCH_ARGS_WGSL.into()),
        });
        let args_pl = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("c3d_sdf_mc_batch_args_pl"),
            bind_group_layouts: &[&args_bgl],
            immediate_size: 0,
        });
        let args_ppl = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_sdf_mc_batch_args_ppl"),
            layout: Some(&args_pl),
            module: &args_sm,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let params = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_batch_params"),
            size: std::mem::size_of::<McBatchParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let cap_verts = 1_000_000u32;
        let out_verts = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_batch_out_verts"),
            size: (cap_verts as u64) * (std::mem::size_of::<McVertex>() as u64),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let out_count = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_batch_out_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let indirect_args = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_batch_indirect_args"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_batch_staging"),
            size: 16 + (cap_verts as u64) * (std::mem::size_of::<McVertex>() as u64),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            seq: 0,
            tx,
            rx,
            ppl,
            bgl,
            args_ppl,
            args_bgl,
            edge_table,
            tri_table,
            params,
            out_verts,
            out_count,
            indirect_args,
            staging,
            cap_verts,
        }
    }

    #[inline]
    pub fn poll(&mut self) -> Option<McBatchAsyncResult> {
        let mut last = None;
        while let Ok(v) = self.rx.try_recv() { last = Some(v); }
        last
    }

    fn ensure_cap(&mut self, rt: &GpuRuntime, need_verts: u32) {
        if self.cap_verts >= need_verts { return; }
        let dev = rt.device();
        let cap = need_verts.next_power_of_two().max(1_000_000);
        self.cap_verts = cap;
        self.out_verts = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_batch_out_verts"),
            size: (cap as u64) * (std::mem::size_of::<McVertex>() as u64),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        self.out_count = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_batch_out_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.staging = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_batch_staging"),
            size: 16 + (cap as u64) * (std::mem::size_of::<McVertex>() as u64),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }

    #[inline]
    pub fn vertex_buffer(&self) -> &wgpu::Buffer { &self.out_verts }

    #[inline]
    pub fn indirect_args_buffer(&self) -> &wgpu::Buffer { &self.indirect_args }

    /// Dispatch batched meshing for the current `pool.work_items[0..work_items_count]`.
    ///
    /// If `readback` is true, vertices are copied to a staging buffer and returned asynchronously via `poll()`.
    pub fn extract_async(
        &mut self,
        rt: &GpuRuntime,
        pool: &SdfGpuPool,
        voxel_size: f32,
        iso_value: f32,
        background_value: f32,
        invert: bool,
        work_items_count: u32,
        max_vertices: u32,
        readback: bool,
    ) -> u64 {
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;

        let wi = work_items_count.max(1);
        let max_vertices = max_vertices.max(1);
        self.ensure_cap(rt, max_vertices);
        let max_vertices = max_vertices.min(self.cap_verts);

        let params = McBatchParams {
            voxel_size: voxel_size.max(0.00001),
            iso_value,
            background_value,
            invert: if invert { 1 } else { 0 },
            work_items_count: wi,
            max_vertices,
            _pad0: [0, 0],
        };

        let dev = rt.device();
        let q = rt.queue();
        q.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        q.write_buffer(&self.out_count, 0, bytemuck::bytes_of(&0u32));

        let bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3d_sdf_mc_batch_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: pool.chunk_pool.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: pool.work_items.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.out_verts.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.out_count.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: self.edge_table.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: self.tri_table.as_entire_binding() },
            ],
        });

        let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("c3d_sdf_mc_batch_enc"),
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("c3d_sdf_mc_batch_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.ppl);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(wi, 1, 1);
        }

        // Prepare indirect args for GPU-only draw (vertex_count, instance_count, first_vertex, first_instance).
        {
            let bg_args = dev.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("c3d_sdf_mc_batch_args_bg"),
                layout: &self.args_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.out_count.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: self.indirect_args.as_entire_binding() },
                ],
            });
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("c3d_sdf_mc_batch_args_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.args_ppl);
            pass.set_bind_group(0, &bg_args, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }

        if readback {
            enc.copy_buffer_to_buffer(&self.out_count, 0, &self.staging, 0, 4);
            let vb = (max_vertices as u64) * (std::mem::size_of::<McVertex>() as u64);
            enc.copy_buffer_to_buffer(&self.out_verts, 0, &self.staging, 16, vb);
        }

        q.submit([enc.finish()]);
        let _ = dev.poll(wgpu::PollType::Poll);

        if readback {
            let staging = self.staging.clone();
            let staging_cb = staging.clone();
            let tx = self.tx.clone();
            staging.slice(..).map_async(wgpu::MapMode::Read, move |res| {
                if res.is_ok() {
                    let slice = staging_cb.slice(..);
                    let mapped = slice.get_mapped_range();
                    let cnt = u32::from_le_bytes([mapped[0], mapped[1], mapped[2], mapped[3]]);
                    let n = (cnt as usize).min(max_vertices as usize);
                    let bytes = &mapped[16..16 + n * std::mem::size_of::<McVertex>()];
                    let vertices: Vec<McVertex> = bytemuck::cast_slice(bytes).to_vec();
                    drop(mapped);
                    staging_cb.unmap();
                    let _ = tx.send(McBatchAsyncResult { seq, vertices });
                } else {
                    staging_cb.unmap();
                }
            });
        }

        seq
    }
}

// Keep the WorkItem layout in sync with `gpu_pool::SdfWorkItem`.
#[allow(dead_code)]
fn _assert_work_item_layout(_: SdfWorkItem) {}

pub const SDF_MC_BATCH_WGSL: &str = r#"
const CHUNK_SIZE: i32 = 16;
const CELLS: u32 = 3375u; // 15^3
const CHUNK_VOXELS: u32 = 4096u;
const MISSING_SLOT: u32 = 0xffffffffu;

struct Params {
  voxel_size: f32,
  iso_value: f32,
  background_value: f32,
  invert: u32,
  work_items_count: u32,
  max_vertices: u32,
  _pad0: vec2<u32>,
};

struct WorkItem {
  chunk_key: vec4<i32>,
  slot: u32,
  _pad0: array<u32, 3>,
  neighbor_slots: array<u32, 27>,
  _pad1: u32,
};

struct Vertex {
  pos: vec4<f32>,
  normal: vec4<f32>,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> chunk_pool: array<f32>;
@group(0) @binding(2) var<storage, read> work_items: array<WorkItem>;
@group(0) @binding(3) var<storage, read_write> out_verts: array<Vertex>;
@group(0) @binding(4) var<storage, read_write> out_count: atomic<u32>;
@group(0) @binding(5) var<storage, read> edge_table: array<u32>;
@group(0) @binding(6) var<storage, read> tri_table: array<i32>;

fn nidx(dx: i32, dy: i32, dz: i32) -> u32 {
  return u32((dz + 1) * 9 + (dy + 1) * 3 + (dx + 1));
}

fn chunk_index(x: i32, y: i32, z: i32) -> u32 {
  return u32(x + y * CHUNK_SIZE + z * CHUNK_SIZE * CHUNK_SIZE);
}

fn load_rel(item: WorkItem, rel: vec3<i32>) -> f32 {
  var ox: i32 = 0;
  var oy: i32 = 0;
  var oz: i32 = 0;
  var lx = rel.x;
  var ly = rel.y;
  var lz = rel.z;
  if (lx < 0) { ox = -1; lx = lx + CHUNK_SIZE; }
  if (lx >= CHUNK_SIZE) { ox = 1; lx = lx - CHUNK_SIZE; }
  if (ly < 0) { oy = -1; ly = ly + CHUNK_SIZE; }
  if (ly >= CHUNK_SIZE) { oy = 1; ly = ly - CHUNK_SIZE; }
  if (lz < 0) { oz = -1; lz = lz + CHUNK_SIZE; }
  if (lz >= CHUNK_SIZE) { oz = 1; lz = lz - CHUNK_SIZE; }
  let ns = item.neighbor_slots[nidx(ox, oy, oz)];
  if (ns == MISSING_SLOT) {
    return p.background_value;
  }
  let base = ns * CHUNK_VOXELS;
  let v = chunk_pool[base + chunk_index(lx, ly, lz)];
  if (p.invert == 1u) { return -v; }
  return v;
}

fn sample_trilinear(item: WorkItem, pos_vox: vec3<f32>, base_vox: vec3<i32>) -> f32 {
  let i0 = vec3<i32>(i32(floor(pos_vox.x)), i32(floor(pos_vox.y)), i32(floor(pos_vox.z)));
  let f = pos_vox - vec3<f32>(f32(i0.x), f32(i0.y), f32(i0.z));
  let r000 = load_rel(item, (i0 + vec3<i32>(0, 0, 0)) - base_vox);
  let r100 = load_rel(item, (i0 + vec3<i32>(1, 0, 0)) - base_vox);
  let r010 = load_rel(item, (i0 + vec3<i32>(0, 1, 0)) - base_vox);
  let r110 = load_rel(item, (i0 + vec3<i32>(1, 1, 0)) - base_vox);
  let r001 = load_rel(item, (i0 + vec3<i32>(0, 0, 1)) - base_vox);
  let r101 = load_rel(item, (i0 + vec3<i32>(1, 0, 1)) - base_vox);
  let r011 = load_rel(item, (i0 + vec3<i32>(0, 1, 1)) - base_vox);
  let r111 = load_rel(item, (i0 + vec3<i32>(1, 1, 1)) - base_vox);

  let v00 = mix(r000, r100, f.x);
  let v10 = mix(r010, r110, f.x);
  let v01 = mix(r001, r101, f.x);
  let v11 = mix(r011, r111, f.x);
  let v0 = mix(v00, v10, f.y);
  let v1 = mix(v01, v11, f.y);
  return mix(v0, v1, f.z);
}

fn normal_at(item: WorkItem, pos_vox: vec3<f32>, base_vox: vec3<i32>) -> vec3<f32> {
  let e = 1.0;
  let dx = sample_trilinear(item, pos_vox + vec3<f32>(e, 0.0, 0.0), base_vox) - sample_trilinear(item, pos_vox - vec3<f32>(e, 0.0, 0.0), base_vox);
  let dy = sample_trilinear(item, pos_vox + vec3<f32>(0.0, e, 0.0), base_vox) - sample_trilinear(item, pos_vox - vec3<f32>(0.0, e, 0.0), base_vox);
  let dz = sample_trilinear(item, pos_vox + vec3<f32>(0.0, 0.0, e), base_vox) - sample_trilinear(item, pos_vox - vec3<f32>(0.0, 0.0, e), base_vox);
  let n = vec3<f32>(dx, dy, dz);
  let l = length(n);
  if (l < 1e-6) { return vec3<f32>(0.0, 1.0, 0.0); }
  return n / l;
}

fn vertex_interp(iso: f32, p1: vec3<f32>, p2: vec3<f32>, v1: f32, v2: f32) -> vec3<f32> {
  if (abs(iso - v1) < 0.00001) { return p1; }
  if (abs(iso - v2) < 0.00001) { return p2; }
  if (abs(v1 - v2) < 0.00001) { return p1; }
  let t = (iso - v1) / (v2 - v1);
  return p1 + t * (p2 - p1);
}

fn tri_entries(cube_index: u32) -> u32 {
  let base = i32(cube_index) * 16;
  var n: u32 = 0u;
  loop {
    if (n >= 16u) { break; }
    if (tri_table[base + i32(n)] == -1) { break; }
    n = n + 1u;
  }
  return n;
}

var<workgroup> scan: array<u32, 256>;
var<workgroup> wg_base: u32;
var<workgroup> wg_total: u32;
var<workgroup> wg_alloc: u32;

fn scan_exclusive_256(x: u32, lid: u32) -> u32 {
  scan[lid] = x;
  workgroupBarrier();

  // upsweep
  var offset: u32 = 1u;
  for (var d: u32 = 128u; d > 0u; d = d >> 1u) {
    workgroupBarrier();
    if (lid < d) {
      let ai = offset * (2u * lid + 1u) - 1u;
      let bi = offset * (2u * lid + 2u) - 1u;
      scan[bi] = scan[bi] + scan[ai];
    }
    offset = offset << 1u;
  }

  workgroupBarrier();
  // save total, set last to 0
  if (lid == 0u) {
    wg_total = scan[255u];
    scan[255u] = 0u;
  }

  // downsweep
  for (var d2: u32 = 1u; d2 <= 128u; d2 = d2 << 1u) {
    offset = offset >> 1u;
    workgroupBarrier();
    if (lid < d2) {
      let ai = offset * (2u * lid + 1u) - 1u;
      let bi = offset * (2u * lid + 2u) - 1u;
      let t = scan[ai];
      scan[ai] = scan[bi];
      scan[bi] = scan[bi] + t;
    }
  }
  workgroupBarrier();
  return scan[lid];
}

@compute @workgroup_size(256)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid3: vec3<u32>) {
  let wi = wg.x;
  let lid = lid3.x;
  if (wi >= p.work_items_count) { return; }
  let item = work_items[wi];
  if (item.slot == MISSING_SLOT) { return; }

  let ck = vec3<i32>(item.chunk_key.x, item.chunk_key.y, item.chunk_key.z);
  let base_vox = vec3<i32>(ck.x * CHUNK_SIZE, ck.y * CHUNK_SIZE, ck.z * CHUNK_SIZE);

  // pass 1: count vertices this thread will emit
  var local_count: u32 = 0u;
  var ci = lid;
  loop {
    if (ci >= CELLS) { break; }
    let x = i32(ci % 15u);
    let y = i32((ci / 15u) % 15u);
    let z = i32(ci / (15u * 15u));

    let v0 = load_rel(item, vec3<i32>(x + 0, y + 0, z + 0));
    let v1 = load_rel(item, vec3<i32>(x + 1, y + 0, z + 0));
    let v2 = load_rel(item, vec3<i32>(x + 1, y + 1, z + 0));
    let v3 = load_rel(item, vec3<i32>(x + 0, y + 1, z + 0));
    let v4 = load_rel(item, vec3<i32>(x + 0, y + 0, z + 1));
    let v5 = load_rel(item, vec3<i32>(x + 1, y + 0, z + 1));
    let v6 = load_rel(item, vec3<i32>(x + 1, y + 1, z + 1));
    let v7 = load_rel(item, vec3<i32>(x + 0, y + 1, z + 1));

    var cube_index: u32 = 0u;
    if (v0 < p.iso_value) { cube_index = cube_index | 1u; }
    if (v1 < p.iso_value) { cube_index = cube_index | 2u; }
    if (v2 < p.iso_value) { cube_index = cube_index | 4u; }
    if (v3 < p.iso_value) { cube_index = cube_index | 8u; }
    if (v4 < p.iso_value) { cube_index = cube_index | 16u; }
    if (v5 < p.iso_value) { cube_index = cube_index | 32u; }
    if (v6 < p.iso_value) { cube_index = cube_index | 64u; }
    if (v7 < p.iso_value) { cube_index = cube_index | 128u; }

    if (cube_index != 0u && cube_index != 255u) {
      local_count = local_count + tri_entries(cube_index);
    }
    ci = ci + 256u;
  }

  let thread_base = scan_exclusive_256(local_count, lid);
  if (lid == 0u) {
    // Reserve space in the global append buffer without exceeding `max_vertices`.
    // Keep allocations a multiple of 3 so we never expose partial triangles.
    var old = atomicLoad(&out_count);
    loop {
      if (old >= p.max_vertices) {
        wg_base = p.max_vertices;
        wg_alloc = 0u;
        break;
      }
      let avail = p.max_vertices - old;
      let avail3 = avail - (avail % 3u);
      let want = wg_total;
      let alloc = min(want, avail3);
      if (alloc == 0u) {
        wg_base = p.max_vertices;
        wg_alloc = 0u;
        break;
      }
      let r = atomicCompareExchangeWeak(&out_count, old, old + alloc);
      if (r.exchanged) {
        wg_base = old;
        wg_alloc = alloc;
        break;
      }
      old = r.old_value;
    }
  }
  workgroupBarrier();

  // pass 2: emit vertices
  let out_end = wg_base + wg_alloc;
  var out_i = wg_base + thread_base;
  ci = lid;
  loop {
    if (ci >= CELLS) { break; }
    let x = i32(ci % 15u);
    let y = i32((ci / 15u) % 15u);
    let z = i32(ci / (15u * 15u));

    let s0 = load_rel(item, vec3<i32>(x + 0, y + 0, z + 0));
    let s1 = load_rel(item, vec3<i32>(x + 1, y + 0, z + 0));
    let s2 = load_rel(item, vec3<i32>(x + 1, y + 1, z + 0));
    let s3 = load_rel(item, vec3<i32>(x + 0, y + 1, z + 0));
    let s4 = load_rel(item, vec3<i32>(x + 0, y + 0, z + 1));
    let s5 = load_rel(item, vec3<i32>(x + 1, y + 0, z + 1));
    let s6 = load_rel(item, vec3<i32>(x + 1, y + 1, z + 1));
    let s7 = load_rel(item, vec3<i32>(x + 0, y + 1, z + 1));

    var cube_index: u32 = 0u;
    if (s0 < p.iso_value) { cube_index = cube_index | 1u; }
    if (s1 < p.iso_value) { cube_index = cube_index | 2u; }
    if (s2 < p.iso_value) { cube_index = cube_index | 4u; }
    if (s3 < p.iso_value) { cube_index = cube_index | 8u; }
    if (s4 < p.iso_value) { cube_index = cube_index | 16u; }
    if (s5 < p.iso_value) { cube_index = cube_index | 32u; }
    if (s6 < p.iso_value) { cube_index = cube_index | 64u; }
    if (s7 < p.iso_value) { cube_index = cube_index | 128u; }
    if (cube_index == 0u || cube_index == 255u) { ci = ci + 256u; continue; }

    let edges = edge_table[cube_index];
    if (edges == 0u) { ci = ci + 256u; continue; }

    let base = vec3<f32>(f32(base_vox.x + x), f32(base_vox.y + y), f32(base_vox.z + z));
    let p0 = base + vec3<f32>(0.0, 0.0, 0.0);
    let p1 = base + vec3<f32>(1.0, 0.0, 0.0);
    let p2 = base + vec3<f32>(1.0, 1.0, 0.0);
    let p3 = base + vec3<f32>(0.0, 1.0, 0.0);
    let p4 = base + vec3<f32>(0.0, 0.0, 1.0);
    let p5 = base + vec3<f32>(1.0, 0.0, 1.0);
    let p6 = base + vec3<f32>(1.0, 1.0, 1.0);
    let p7 = base + vec3<f32>(0.0, 1.0, 1.0);

    var vertlist: array<vec3<f32>, 12>;
    if ((edges & 1u) != 0u) { vertlist[0] = vertex_interp(p.iso_value, p0, p1, s0, s1); }
    if ((edges & 2u) != 0u) { vertlist[1] = vertex_interp(p.iso_value, p1, p2, s1, s2); }
    if ((edges & 4u) != 0u) { vertlist[2] = vertex_interp(p.iso_value, p2, p3, s2, s3); }
    if ((edges & 8u) != 0u) { vertlist[3] = vertex_interp(p.iso_value, p3, p0, s3, s0); }
    if ((edges & 16u) != 0u) { vertlist[4] = vertex_interp(p.iso_value, p4, p5, s4, s5); }
    if ((edges & 32u) != 0u) { vertlist[5] = vertex_interp(p.iso_value, p5, p6, s5, s6); }
    if ((edges & 64u) != 0u) { vertlist[6] = vertex_interp(p.iso_value, p6, p7, s6, s7); }
    if ((edges & 128u) != 0u) { vertlist[7] = vertex_interp(p.iso_value, p7, p4, s7, s4); }
    if ((edges & 256u) != 0u) { vertlist[8] = vertex_interp(p.iso_value, p0, p4, s0, s4); }
    if ((edges & 512u) != 0u) { vertlist[9] = vertex_interp(p.iso_value, p1, p5, s1, s5); }
    if ((edges & 1024u) != 0u) { vertlist[10] = vertex_interp(p.iso_value, p2, p6, s2, s6); }
    if ((edges & 2048u) != 0u) { vertlist[11] = vertex_interp(p.iso_value, p3, p7, s3, s7); }

    let tri_base = i32(cube_index) * 16;
    var ti: i32 = 0;
    loop {
      if (ti >= 16) { break; }
      let e0 = tri_table[tri_base + ti + 0];
      if (e0 == -1) { break; }
      let e1 = tri_table[tri_base + ti + 1];
      let e2 = tri_table[tri_base + ti + 2];

      let v0 = vertlist[u32(e0)];
      let v1 = vertlist[u32(e1)];
      let v2 = vertlist[u32(e2)];

      let n0 = normal_at(item, v0, base_vox);
      let n1 = normal_at(item, v1, base_vox);
      let n2 = normal_at(item, v2, base_vox);

      let w0 = v0 * p.voxel_size;
      let w1 = v1 * p.voxel_size;
      let w2 = v2 * p.voxel_size;

      if (out_i + 2u < out_end) {
        out_verts[out_i + 0u] = Vertex(vec4<f32>(w0, 1.0), vec4<f32>(n0, 0.0));
        out_verts[out_i + 1u] = Vertex(vec4<f32>(w1, 1.0), vec4<f32>(n1, 0.0));
        out_verts[out_i + 2u] = Vertex(vec4<f32>(w2, 1.0), vec4<f32>(n2, 0.0));
      }
      out_i = out_i + 3u;
      ti = ti + 3;
    }

    ci = ci + 256u;
  }
}
"#;

pub const SDF_MC_BATCH_ARGS_WGSL: &str = r#"
struct DrawIndirect { vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32 };
struct OutCount { count: atomic<u32> };
@group(0) @binding(0) var<storage, read> out_count: OutCount;
@group(0) @binding(1) var<storage, read_write> out_args: DrawIndirect;

@compute @workgroup_size(1)
fn main() {
  let vc = atomicLoad(&out_count.count);
  out_args.vertex_count = vc;
  out_args.instance_count = 1u;
  out_args.first_vertex = 0u;
  out_args.first_instance = 0u;
}
"#;
