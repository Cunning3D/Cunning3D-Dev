//! Phase 3: GPU SDF brush (clay sculpt) - in-place updates on chunked SDF storage.
//!
//! This module is designed for the "hot path":
//! - Dispatch one workgroup per chunk (256 threads), each thread iterates voxels.
//! - Per-chunk sign-crossing is reduced in workgroup memory, then appended once.
//! - The modified SDF stays on GPU; CPU readback is optional (debug/export).

use bevy::prelude::Vec3;
use bytemuck::{Pod, Zeroable};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::mpsc;

use crate::nodes::gpu::runtime::GpuRuntime;
use crate::sdf_engine::gpu_pool::SdfGpuPool;

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct SdfBrushParams {
    /// Brush endpoint A (world). For a sphere: this is the center.
    pub p0_world: [f32; 3],
    pub radius_world: f32,
    /// Brush endpoint B (world). For a sphere: same as `p0_world`.
    pub p1_world: [f32; 3],
    pub voxel_size: f32,
    pub smooth_k: f32,
    /// 0 = union/add, 1 = subtract, 2 = intersect
    pub mode: u32,
    /// 0 = sphere, 1 = capsule
    pub shape: u32,
    /// number of work-items in `pool.work_items`
    pub work_items_count: u32,
    /// max dirty slots to write
    pub max_dirty: u32,
    pub _pad0: [u32; 3],
}

#[derive(Clone)]
pub struct BrushDirtyResult {
    pub seq: u64,
    pub dirty_slots: Vec<u32>,
}

pub struct GpuSdfBrush {
    seq: u64,
    tx: Sender<BrushDirtyResult>,
    rx: Receiver<BrushDirtyResult>,

    ppl: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,

    params: wgpu::Buffer,
    dirty_slots: wgpu::Buffer,
    dirty_count: wgpu::Buffer,
    staging: wgpu::Buffer,

    cap_dirty: u32,
}

impl GpuSdfBrush {
    pub fn new(rt: &GpuRuntime) -> Self {
        let (tx, rx) = mpsc::channel();
        let dev = rt.device();
        let sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c3d_sdf_brush_wgsl"),
            source: wgpu::ShaderSource::Wgsl(SDF_BRUSH_WGSL.into()),
        });

        let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_sdf_brush_bgl"),
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
                // chunk_pool (read_write f32[])
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
                // dirty_slots out
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
                // dirty_count out
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
            ],
        });

        let pl = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("c3d_sdf_brush_pl"),
            bind_group_layouts: &[&bgl],
            immediate_size: 0,
        });
        let ppl = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_sdf_brush_ppl"),
            layout: Some(&pl),
            module: &sm,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let params = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_brush_params"),
            size: std::mem::size_of::<SdfBrushParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let cap_dirty = 1024u32;
        let dirty_slots = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_brush_dirty_slots"),
            size: (cap_dirty as u64) * 4u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let dirty_count = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_brush_dirty_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_brush_staging"),
            size: 16 + (cap_dirty as u64) * 4u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            seq: 0,
            tx,
            rx,
            ppl,
            bgl,
            params,
            dirty_slots,
            dirty_count,
            staging,
            cap_dirty,
        }
    }

    #[inline]
    pub fn poll(&mut self) -> Option<BrushDirtyResult> {
        let mut last = None;
        while let Ok(v) = self.rx.try_recv() { last = Some(v); }
        last
    }

    fn ensure_dirty_cap(&mut self, rt: &GpuRuntime, need: u32) {
        if self.cap_dirty >= need { return; }
        let dev = rt.device();
        let cap = need.next_power_of_two().max(1024);
        self.cap_dirty = cap;
        self.dirty_slots = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_brush_dirty_slots"),
            size: (cap as u64) * 4u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        self.dirty_count = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_brush_dirty_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.staging = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_brush_staging"),
            size: 16 + (cap as u64) * 4u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }

    /// Dispatch the brush over `pool.work_items[0..work_items_count]`.
    ///
    /// - `pool.work_items` must already be populated for the current chunk set.
    /// - Returns `seq` and delivers dirty slots via `poll()`.
    pub fn sculpt_sphere_async(
        &mut self,
        rt: &GpuRuntime,
        pool: &SdfGpuPool,
        center_world: Vec3,
        radius_world: f32,
        voxel_size: f32,
        smooth_k: f32,
        mode: u32,
        work_items_count: u32,
        max_dirty: u32,
    ) -> u64 {
        self.sculpt_capsule_async(
            rt,
            pool,
            center_world,
            center_world,
            radius_world,
            voxel_size,
            smooth_k,
            mode,
            /*shape=*/ 0,
            work_items_count,
            max_dirty,
        )
    }

    pub fn sculpt_capsule_async(
        &mut self,
        rt: &GpuRuntime,
        pool: &SdfGpuPool,
        p0_world: Vec3,
        p1_world: Vec3,
        radius_world: f32,
        voxel_size: f32,
        smooth_k: f32,
        mode: u32,
        shape: u32,
        work_items_count: u32,
        max_dirty: u32,
    ) -> u64 {
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;

        let wi = work_items_count.min(u32::MAX);
        let want_dirty = max_dirty.max(1).min(self.cap_dirty.max(1));
        self.ensure_dirty_cap(rt, want_dirty);

        let p = SdfBrushParams {
            p0_world: p0_world.to_array(),
            radius_world: radius_world.max(0.0),
            p1_world: p1_world.to_array(),
            voxel_size: voxel_size.max(0.00001),
            smooth_k: smooth_k.max(0.0),
            mode,
            shape,
            work_items_count: wi,
            max_dirty: want_dirty,
            _pad0: [0; 3],
        };

        let dev = rt.device();
        let q = rt.queue();
        q.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
        q.write_buffer(&self.dirty_count, 0, bytemuck::bytes_of(&0u32));

        let bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3d_sdf_brush_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: pool.chunk_pool.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: pool.work_items.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.dirty_slots.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.dirty_count.as_entire_binding() },
            ],
        });

        let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("c3d_sdf_brush_enc"),
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("c3d_sdf_brush_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.ppl);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(wi, 1, 1);
        }
        enc.copy_buffer_to_buffer(&self.dirty_count, 0, &self.staging, 0, 4);
        enc.copy_buffer_to_buffer(&self.dirty_slots, 0, &self.staging, 16, (want_dirty as u64) * 4u64);
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
                let n = (cnt as usize).min(want_dirty as usize);
                let bytes = &mapped[16..16 + n * 4];
                let dirty_slots: Vec<u32> = bytemuck::cast_slice(bytes).to_vec();
                drop(mapped);
                staging_cb.unmap();
                let _ = tx.send(BrushDirtyResult { seq, dirty_slots });
            } else {
                staging_cb.unmap();
            }
        });

        seq
    }
}

const SDF_BRUSH_WGSL: &str = r#"
const CHUNK_SIZE: u32 = 16u;
const CHUNK_VOXELS: u32 = 4096u;
const MISSING_SLOT: u32 = 0xffffffffu;

struct Params {
  p0_world: vec3<f32>,
  radius_world: f32,
  p1_world: vec3<f32>,
  voxel_size: f32,
  smooth_k: f32,
  mode: u32,
  shape: u32,
  work_items_count: u32,
  max_dirty: u32,
  _pad0: vec3<u32>,
};

struct WorkItem {
  chunk_key: vec4<i32>,
  slot: u32,
  _pad0: array<u32, 3>,
  neighbor_slots: array<u32, 27>,
  _pad1: u32,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> chunk_pool: array<f32>;
@group(0) @binding(2) var<storage, read> work_items: array<WorkItem>;
@group(0) @binding(3) var<storage, read_write> dirty_slots: array<u32>;
@group(0) @binding(4) var<storage, read_write> dirty_count: atomic<u32>;

fn smin_poly(a: f32, b: f32, k: f32) -> f32 {
  if (k <= 0.0) { return min(a, b); }
  let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
  return mix(b, a, h) - k * h * (1.0 - h);
}

fn smax_poly(a: f32, b: f32, k: f32) -> f32 {
  return -smin_poly(-a, -b, k);
}

fn sd_sphere(pw: vec3<f32>, r: f32) -> f32 {
  return length(pw) - r;
}

fn sd_capsule(pw: vec3<f32>, a: vec3<f32>, b: vec3<f32>, r: f32) -> f32 {
  let pa = pw - a;
  let ba = b - a;
  let baba = dot(ba, ba);
  if (baba < 1e-12) {
    return length(pa) - r;
  }
  let h = clamp(dot(pa, ba) / baba, 0.0, 1.0);
  return length(pa - ba * h) - r;
}

@compute @workgroup_size(256)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
  let wi = wg.x;
  if (wi >= p.work_items_count) { return; }

  let item = work_items[wi];
  let slot = item.slot;
  if (slot == MISSING_SLOT) { return; }

  var<workgroup> crossed: atomic<u32>;
  if (lid.x == 0u) { atomicStore(&crossed, 0u); }
  workgroupBarrier();

  let ck = vec3<i32>(item.chunk_key.x, item.chunk_key.y, item.chunk_key.z);
  let voxel_origin = vec3<i32>(ck.x * i32(CHUNK_SIZE), ck.y * i32(CHUNK_SIZE), ck.z * i32(CHUNK_SIZE));
  let base = u32(slot) * CHUNK_VOXELS;

  var i = lid.x;
  loop {
    if (i >= CHUNK_VOXELS) { break; }

    let x = i % CHUNK_SIZE;
    let y = (i / CHUNK_SIZE) % CHUNK_SIZE;
    let z = i / (CHUNK_SIZE * CHUNK_SIZE);
    let gp = vec3<i32>(voxel_origin.x + i32(x), voxel_origin.y + i32(y), voxel_origin.z + i32(z));
    let world = (vec3<f32>(f32(gp.x) + 0.5, f32(gp.y) + 0.5, f32(gp.z) + 0.5)) * p.voxel_size;

    let old_v = chunk_pool[base + i];
    var b = 0.0;
    if (p.shape == 0u) {
      b = sd_sphere(world - p.p0_world, p.radius_world);
    } else {
      b = sd_capsule(world, p.p0_world, p.p1_world, p.radius_world);
    }

    var new_v = old_v;
    if (p.mode == 0u) {
      // union/add
      new_v = smin_poly(old_v, b, p.smooth_k);
    } else if (p.mode == 1u) {
      // subtract / difference: a \ b  => max(a, -b)
      new_v = smax_poly(old_v, -b, p.smooth_k);
    } else if (p.mode == 2u) {
      // intersect
      new_v = smax_poly(old_v, b, p.smooth_k);
    }

    chunk_pool[base + i] = new_v;

    let s0 = old_v < 0.0;
    let s1 = new_v < 0.0;
    if (s0 != s1) {
      atomicOr(&crossed, 1u);
    }

    i = i + 256u;
  }

  workgroupBarrier();
  if (lid.x == 0u) {
    if (atomicLoad(&crossed) != 0u) {
      let out_i = atomicAdd(&dirty_count, 1u);
      if (out_i < p.max_dirty) {
        dirty_slots[out_i] = slot;
      }
    }
  }
}
"#;
