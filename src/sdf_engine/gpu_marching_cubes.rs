//! Phase 1: GPU-accelerated Marching Cubes for SDF (Signed Distance Fields).
//! High-speed mesh extraction directly from chunked f32 volumetric data.
use bevy::prelude::IVec3;
use crate::cunning_core::core::geometry::sdf::{SdfChunk, SdfGrid, CHUNK_SIZE};
use crate::nodes::gpu::runtime::GpuRuntime;
use bytemuck::{Pod, Zeroable};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::mpsc;
use wgpu::util::DeviceExt;

/// Defines the input parameter structure for the WGSL compute pass.
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct MarchingCubesParams {
    /// Global voxel-space (index-space) origin of the chunk: `chunk_key * CHUNK_SIZE`.
    pub chunk_origin: [i32; 4],
    /// The surface threshold (usually 0.0).
    pub iso_value: f32,
    /// Scale mapping voxel index -> world space.
    pub voxel_size: f32,
    /// 1 if inside/outside flip is requested, else 0.
    pub invert: u32,
    /// Safety limit to prevent buffer overflow.
    pub max_vertices: u32,
}

/// The extracted vertex structure compatible with standard PBR geometry.
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct SdfVertex {
    /// xyz = position, w = 1.0 (padding / alignment for WGSL).
    pub position: [f32; 4],
    /// xyz = normal, w = 0.0 (padding / alignment for WGSL).
    pub normal: [f32; 4],
}

/// Represents the asynchronous or synchronous result of a chunk extraction.
pub struct McAsyncResult {
    pub seq: u64,
    pub chunk_key: IVec3,
    pub vertices: Vec<SdfVertex>,
}

pub struct GpuMarchingCubes {
    seq: u64,
    tx: Sender<McAsyncResult>,
    rx: Receiver<McAsyncResult>,
    ppl: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    bg: wgpu::BindGroup,
    params: wgpu::Buffer,
    chunk_data: wgpu::Buffer,
    edge_table: wgpu::Buffer,
    tri_table: wgpu::Buffer,
    out_verts: wgpu::Buffer,
    out_count: wgpu::Buffer,
    staging: wgpu::Buffer,
    cap: u32,
}

impl GpuMarchingCubes {
    pub fn new(rt: &GpuRuntime) -> Self {
        let (tx, rx) = mpsc::channel();
        let dev = rt.device();
        let shader = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c3d_sdf_marching_cubes_wgsl"),
            source: wgpu::ShaderSource::Wgsl(SDF_MARCHING_CUBES_WGSL.into()),
        });

        // Binding 0: Parameters (Uniform)
        // Binding 1: Voxel Data (Storage, Read) -> Since we are dealing with 1 chunk, size is 16*16*16 f32s.
        // Binding 2: Out Vertices (Storage, ReadWrite)
        // Binding 3: Out Atomic Count (Storage, ReadWrite)
        // Binding 4: Edge Table (Storage, ReadOnly)
        // Binding 5: Tri Table (Storage, ReadOnly)
        let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_sdf_mc_bgl"),
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
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
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
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
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
            ],
        });

        let ppl_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("c3d_sdf_mc_layout"),
            bind_group_layouts: &[&bgl],
            immediate_size: 0,
        });

        let ppl = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_sdf_mc_pipeline"),
            layout: Some(&ppl_layout),
            module: &shader,
            entry_point: Some("extract_surface"),
            compilation_options: Default::default(),
            cache: None,
        });

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
            label: Some("c3d_sdf_mc_edge_table"),
            contents: bytemuck::cast_slice(&edge_table_u32),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let tri_table = dev.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("c3d_sdf_mc_tri_table"),
            contents: bytemuck::cast_slice(&tri_table_i32),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let cap = 65536u32;
        let params = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_params"),
            size: std::mem::size_of::<MarchingCubesParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let chunk_data = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_chunk_data"),
            size: (CHUNK_SIZE as u64) * (CHUNK_SIZE as u64) * (CHUNK_SIZE as u64) * 4u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let out_verts = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_out_verts"),
            size: (cap as u64) * (std::mem::size_of::<SdfVertex>() as u64),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let out_count = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_out_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_staging"),
            size: 16 + (cap as u64) * (std::mem::size_of::<SdfVertex>() as u64),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3d_sdf_mc_bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: chunk_data.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: out_verts.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: out_count.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: edge_table.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: tri_table.as_entire_binding() },
            ],
        });

        Self {
            seq: 0,
            tx,
            rx,
            ppl,
            bgl,
            bg,
            params,
            chunk_data,
            edge_table,
            tri_table,
            out_verts,
            out_count,
            staging,
            cap,
        }
    }

    #[inline]
    pub fn poll(&mut self) -> Option<McAsyncResult> {
        let mut last = None;
        while let Ok(v) = self.rx.try_recv() { last = Some(v); }
        last
    }

    fn ensure_cap(&mut self, rt: &GpuRuntime, need: u32) {
        if self.cap >= need { return; }
        let dev = rt.device();
        let cap = need.next_power_of_two().max(65536);
        self.cap = cap;
        self.out_verts = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_out_verts"),
            size: (cap as u64) * (std::mem::size_of::<SdfVertex>() as u64),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        self.out_count = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_out_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.staging = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_sdf_mc_staging"),
            size: 16 + (cap as u64) * (std::mem::size_of::<SdfVertex>() as u64),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3d_sdf_mc_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.chunk_data.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.out_verts.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.out_count.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.edge_table.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: self.tri_table.as_entire_binding() },
            ],
        });
    }

    /// Extract triangles for a single SDF chunk (16^3 samples -> 15^3 cells).
    ///
    /// Notes:
    /// - This currently only sees values inside the chunk; cells spanning chunk borders are not extracted yet.
    /// - Results are returned asynchronously via `poll()`.
    pub fn extract_chunk_async(
        &mut self,
        rt: &GpuRuntime,
        chunk_key: IVec3,
        chunk: &SdfChunk,
        voxel_size: f32,
        iso_value: f32,
        invert: bool,
        max_vertices: u32,
    ) -> u64 {
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;

        let want_out = max_vertices.max(1);
        self.ensure_cap(rt, want_out);
        let want_out = want_out.min(self.cap);

        let origin = chunk_key * CHUNK_SIZE;
        let params = MarchingCubesParams {
            chunk_origin: [origin.x, origin.y, origin.z, 0],
            iso_value,
            voxel_size: voxel_size.max(0.00001),
            invert: if invert { 1 } else { 0 },
            max_vertices: want_out,
        };

        let dev = rt.device();
        let q = rt.queue();
        q.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        q.write_buffer(&self.chunk_data, 0, bytemuck::cast_slice(&chunk.data));
        q.write_buffer(&self.out_count, 0, bytemuck::bytes_of(&0u32));

        let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("c3d_sdf_mc_enc"),
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("c3d_sdf_mc_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.ppl);
            pass.set_bind_group(0, &self.bg, &[]);
            // We dispatch the full 16^3 and early-out in shader for x/y/z >= 15.
            pass.dispatch_workgroups(4, 4, 4);
        }
        enc.copy_buffer_to_buffer(&self.out_count, 0, &self.staging, 0, 4);
        let vert_bytes = (want_out as u64) * (std::mem::size_of::<SdfVertex>() as u64);
        enc.copy_buffer_to_buffer(&self.out_verts, 0, &self.staging, 16, vert_bytes);
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
                let bytes = &mapped[16..16 + n * std::mem::size_of::<SdfVertex>()];
                let vertices: Vec<SdfVertex> = bytemuck::cast_slice(bytes).to_vec();
                drop(mapped);
                staging_cb.unmap();
                let _ = tx.send(McAsyncResult { seq, chunk_key, vertices });
            } else {
                staging_cb.unmap();
            }
        });
        seq
    }

    #[inline]
    pub fn extract_grid_chunk_async(
        &mut self,
        rt: &GpuRuntime,
        grid: &SdfGrid,
        chunk_key: IVec3,
        iso_value: f32,
        invert: bool,
        max_vertices: u32,
    ) -> Option<u64> {
        let c = grid.chunks.get(&chunk_key)?;
        Some(self.extract_chunk_async(rt, chunk_key, c, grid.voxel_size, iso_value, invert, max_vertices))
    }
}

// ------------------------------------------------------------------------------------------
// WGSL Shader Payload
// ------------------------------------------------------------------------------------------
// Note: Lookup tables are provided via immutable storage buffers (bindings 4/5)
// using `marching-cubes` crate constants on the Rust side.
const SDF_MARCHING_CUBES_WGSL: &str = r#"
struct Params {
    chunk_origin: vec4<i32>,
    iso_value: f32,
    voxel_size: f32,
    invert: u32,
    max_vertices: u32,
};

struct Vertex {
    pos: vec4<f32>,
    normal: vec4<f32>,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> chunk_data: array<f32>;
@group(0) @binding(2) var<storage, read_write> out_verts: array<Vertex>;
@group(0) @binding(3) var<storage, read_write> out_count: atomic<u32>;
@group(0) @binding(4) var<storage, read> edge_table: array<u32>;
@group(0) @binding(5) var<storage, read> tri_table: array<i32>;

fn reserve_vertices(n: u32) -> u32 {
    // CAS loop to avoid over-reserving (and thus reading/drawing unwritten verts).
    var old = atomicLoad(&out_count);
    loop {
        if (old + n > p.max_vertices) {
            return 0xffffffffu;
        }
        let r = atomicCompareExchangeWeak(&out_count, old, old + n);
        if (r.exchanged) {
            return old;
        }
        old = r.old_value;
    }
}

// Helper to access 1D chunk array as 3D (assuming CHUNK_SIZE = 16)
fn load_raw(x: i32, y: i32, z: i32) -> f32 {
    let ix = clamp(x, 0, 15);
    let iy = clamp(y, 0, 15);
    let iz = clamp(z, 0, 15);
    let idx = ix + iy * 16 + iz * 256;
    let v = chunk_data[idx];
    if (p.invert == 1u) {
        return -v;
    }
    return v;
}

fn sample_sdf(pos: vec3<f32>) -> f32 {
    let x0 = i32(floor(pos.x));
    let y0 = i32(floor(pos.y));
    let z0 = i32(floor(pos.z));
    let x1 = x0 + 1;
    let y1 = y0 + 1;
    let z1 = z0 + 1;
    let fx = pos.x - f32(x0);
    let fy = pos.y - f32(y0);
    let fz = pos.z - f32(z0);

    let v000 = load_raw(x0, y0, z0);
    let v100 = load_raw(x1, y0, z0);
    let v010 = load_raw(x0, y1, z0);
    let v110 = load_raw(x1, y1, z0);
    let v001 = load_raw(x0, y0, z1);
    let v101 = load_raw(x1, y0, z1);
    let v011 = load_raw(x0, y1, z1);
    let v111 = load_raw(x1, y1, z1);

    let v00 = mix(v000, v100, fx);
    let v10 = mix(v010, v110, fx);
    let v01 = mix(v001, v101, fx);
    let v11 = mix(v011, v111, fx);
    let v0 = mix(v00, v10, fy);
    let v1 = mix(v01, v11, fy);
    return mix(v0, v1, fz);
}

fn normal_at(pos: vec3<f32>) -> vec3<f32> {
    let e = 1.0;
    let dx = sample_sdf(pos + vec3<f32>(e, 0.0, 0.0)) - sample_sdf(pos - vec3<f32>(e, 0.0, 0.0));
    let dy = sample_sdf(pos + vec3<f32>(0.0, e, 0.0)) - sample_sdf(pos - vec3<f32>(0.0, e, 0.0));
    let dz = sample_sdf(pos + vec3<f32>(0.0, 0.0, e)) - sample_sdf(pos - vec3<f32>(0.0, 0.0, e));
    let n = vec3<f32>(dx, dy, dz);
    let l = length(n);
    if (l < 1e-6) { return vec3<f32>(0.0, 1.0, 0.0); }
    return n / l;
}

// Minimal interpolation (linear) between two corner values
fn vertex_interp(iso_lvl: f32, p1: vec3<f32>, p2: vec3<f32>, valp1: f32, valp2: f32) -> vec3<f32> {
    if (abs(iso_lvl - valp1) < 0.00001) { return p1; }
    if (abs(iso_lvl - valp2) < 0.00001) { return p2; }
    if (abs(valp1 - valp2) < 0.00001) { return p1; }

    let mu = (iso_lvl - valp1) / (valp2 - valp1);
    return p1 + mu * (p2 - p1);
}

@compute @workgroup_size(4, 4, 4)
fn extract_surface(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = i32(gid.x);
    let y = i32(gid.y);
    let z = i32(gid.z);

    // Stop at 15 because a cube requires testing (x, x+1)
    if (x >= 15 || y >= 15 || z >= 15) { return; }

    // 1. Gather values for the 8 corners of the current voxel cell
    // Corner order matches the classic `triTable`/`edgeTable`.
    let val0 = load_raw(x, y, z);
    let val1 = load_raw(x + 1, y, z);
    let val2 = load_raw(x + 1, y + 1, z);
    let val3 = load_raw(x, y + 1, z);
    let val4 = load_raw(x, y, z + 1);
    let val5 = load_raw(x + 1, y, z + 1);
    let val6 = load_raw(x + 1, y + 1, z + 1);
    let val7 = load_raw(x, y + 1, z + 1);

    // 2. Determine cube index based on which vertices are below the iso-surface
    var cube_index = 0u;
    if (val0 < p.iso_value) { cube_index |= 1u; }
    if (val1 < p.iso_value) { cube_index |= 2u; }
    if (val2 < p.iso_value) { cube_index |= 4u; }
    if (val3 < p.iso_value) { cube_index |= 8u; }
    if (val4 < p.iso_value) { cube_index |= 16u; }
    if (val5 < p.iso_value) { cube_index |= 32u; }
    if (val6 < p.iso_value) { cube_index |= 64u; }
    if (val7 < p.iso_value) { cube_index |= 128u; }

    // 3. If completely inside or completely outside, skip.
    if (cube_index == 0u || cube_index == 255u) { return; }

    let edges = edge_table[cube_index];
    if (edges == 0u) { return; }

    let base = vec3<f32>(f32(x), f32(y), f32(z));
    let p0 = base + vec3<f32>(0.0, 0.0, 0.0);
    let p1 = base + vec3<f32>(1.0, 0.0, 0.0);
    let p2 = base + vec3<f32>(1.0, 1.0, 0.0);
    let p3 = base + vec3<f32>(0.0, 1.0, 0.0);
    let p4 = base + vec3<f32>(0.0, 0.0, 1.0);
    let p5 = base + vec3<f32>(1.0, 0.0, 1.0);
    let p6 = base + vec3<f32>(1.0, 1.0, 1.0);
    let p7 = base + vec3<f32>(0.0, 1.0, 1.0);

    var vertlist: array<vec3<f32>, 12>;
    if ((edges & 1u) != 0u) { vertlist[0] = vertex_interp(p.iso_value, p0, p1, val0, val1); }
    if ((edges & 2u) != 0u) { vertlist[1] = vertex_interp(p.iso_value, p1, p2, val1, val2); }
    if ((edges & 4u) != 0u) { vertlist[2] = vertex_interp(p.iso_value, p2, p3, val2, val3); }
    if ((edges & 8u) != 0u) { vertlist[3] = vertex_interp(p.iso_value, p3, p0, val3, val0); }
    if ((edges & 16u) != 0u) { vertlist[4] = vertex_interp(p.iso_value, p4, p5, val4, val5); }
    if ((edges & 32u) != 0u) { vertlist[5] = vertex_interp(p.iso_value, p5, p6, val5, val6); }
    if ((edges & 64u) != 0u) { vertlist[6] = vertex_interp(p.iso_value, p6, p7, val6, val7); }
    if ((edges & 128u) != 0u) { vertlist[7] = vertex_interp(p.iso_value, p7, p4, val7, val4); }
    if ((edges & 256u) != 0u) { vertlist[8] = vertex_interp(p.iso_value, p0, p4, val0, val4); }
    if ((edges & 512u) != 0u) { vertlist[9] = vertex_interp(p.iso_value, p1, p5, val1, val5); }
    if ((edges & 1024u) != 0u) { vertlist[10] = vertex_interp(p.iso_value, p2, p6, val2, val6); }
    if ((edges & 2048u) != 0u) { vertlist[11] = vertex_interp(p.iso_value, p3, p7, val3, val7); }

    let origin = vec3<f32>(f32(p.chunk_origin.x), f32(p.chunk_origin.y), f32(p.chunk_origin.z));
    let tri_base = i32(cube_index) * 16;
    var i: i32 = 0;
    loop {
        if (i >= 16) { break; }
        let e0 = tri_table[tri_base + i + 0];
        if (e0 == -1) { break; }
        let e1 = tri_table[tri_base + i + 1];
        let e2 = tri_table[tri_base + i + 2];

        let vi = reserve_vertices(3u);
        if (vi == 0xffffffffu) { break; }

        let v0 = vertlist[u32(e0)];
        let v1 = vertlist[u32(e1)];
        let v2 = vertlist[u32(e2)];

        let n0 = normal_at(v0);
        let n1 = normal_at(v1);
        let n2 = normal_at(v2);

        let w0 = (v0 + origin) * p.voxel_size;
        let w1 = (v1 + origin) * p.voxel_size;
        let w2 = (v2 + origin) * p.voxel_size;

        out_verts[vi + 0u] = Vertex(vec4<f32>(w0, 1.0), vec4<f32>(n0, 0.0));
        out_verts[vi + 1u] = Vertex(vec4<f32>(w1, 1.0), vec4<f32>(n1, 0.0));
        out_verts[vi + 2u] = Vertex(vec4<f32>(w2, 1.0), vec4<f32>(n2, 0.0));

        i = i + 3;
    }
}
"#;
