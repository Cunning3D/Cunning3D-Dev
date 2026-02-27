//! Phase 2: GPU-accelerated Mesh to SDF via Jump Flood Algorithm (JFA).
//! This extremely fast parallel algorithm takes a set of mesh triangles and
//! generates a continuous Signed Distance Field over a discrete 3D grid.

use crate::nodes::gpu::runtime::GpuRuntime;
use bevy::prelude::Vec3;
use bytemuck::{Pod, Zeroable};
use crate::sdf_engine::gpu_pool::SdfGpuPool;
use crate::cunning_core::core::geometry::sdf::CHUNK_SIZE;
use bevy::prelude::IVec3;

// ------------------------------------------------------------------------------------------
// 1. Data Structures & Parameters
// ------------------------------------------------------------------------------------------

/// Global parameters defining the grid bounds and current JFA execution step
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct JfaParams {
    pub grid_min: [f32; 3], // World space minimum bounds
    pub voxel_size: f32,    // Size of one voxel
    pub grid_max: [f32; 3], // World space maximum bounds
    pub step_size: i32,     // Current JFA step (N/2, N/4 ... 1)
    pub grid_res: [i32; 3], // Resolution of the 3D grid (x, y, z)
    pub num_triangles: u32, // Number of triangles in the input mesh
    /// World-space seeding band thickness around the surface.
    pub seed_band_world: f32,
    /// World-space surface mask thickness (for robust sign fill, should be ~1 voxel).
    pub surface_band_world: f32,
    /// Used when a voxel never receives a valid closest point.
    pub background_value: f32,
    pub _pad0: u32,
}

/// A compact triangle representation for the Compute Shader
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct JfaTriangle {
    pub v0: [f32; 3],
    pub _pad0: u32,
    pub v1: [f32; 3],
    pub _pad1: u32,
    pub v2: [f32; 3],
    pub _pad2: u32,
}

/// Propagation cell written by seed/flood passes.
///
/// Layout matches WGSL:
/// `cp_valid.xyz` = closest point (world), `cp_valid.w` = 1.0 valid else 0.0
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
struct JfaCell {
    cp_valid: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct JfaScatterParams {
    /// Voxel-space (index-space) origin of the dense JFA grid (global voxel indices).
    pub grid_min_vox: [i32; 4],
    /// Dense grid resolution in voxels (x, y, z).
    pub grid_res: [i32; 4],
    pub background_value: f32,
    pub work_items_count: u32,
    pub _pad0: [u32; 2],
}

// ------------------------------------------------------------------------------------------
// 2. WGPU JFA Controller Pipeline
// ------------------------------------------------------------------------------------------

pub struct GpuJfa {
    clear_ppl: wgpu::ComputePipeline,
    seed_ppl: wgpu::ComputePipeline,
    flood_ppl: wgpu::ComputePipeline,
    sign_ppl: wgpu::ComputePipeline,
    outside_seed_ppl: wgpu::ComputePipeline,
    outside_bfs_ppl: wgpu::ComputePipeline,
    scatter_ppl: wgpu::ComputePipeline,

    bgl_clear: wgpu::BindGroupLayout,
    bgl_seed: wgpu::BindGroupLayout,
    bgl_flood: wgpu::BindGroupLayout,
    bgl_sign: wgpu::BindGroupLayout,
    bgl_outside: wgpu::BindGroupLayout,
    bgl_scatter: wgpu::BindGroupLayout,

    params_buffer: wgpu::Buffer,
    scatter_params: wgpu::Buffer,

    // We retain max resolution capabilities to avoid reallocating ping-pong buffers constantly.
    max_cells: u32,
    ping_buffer: wgpu::Buffer,
    pong_buffer: wgpu::Buffer,
    seed_best_dist: wgpu::Buffer,
    surface_mask: wgpu::Buffer,
    outside_mask: wgpu::Buffer,
    queue_meta: wgpu::Buffer,
    queue: wgpu::Buffer,
    sdf_out: wgpu::Buffer,
    staging_buffer: wgpu::Buffer,
}

impl GpuJfa {
    pub fn new(rt: &GpuRuntime) -> Self {
        let dev = rt.device();
        let clear_sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c3d_sdf_jfa_clear_wgsl"),
            source: wgpu::ShaderSource::Wgsl(SDF_JFA_CLEAR_WGSL.into()),
        });
        let seed_sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c3d_sdf_jfa_seed_wgsl"),
            source: wgpu::ShaderSource::Wgsl(SDF_JFA_SEED_WGSL.into()),
        });
        let flood_sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c3d_sdf_jfa_flood_wgsl"),
            source: wgpu::ShaderSource::Wgsl(SDF_JFA_FLOOD_WGSL.into()),
        });
        let sign_sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c3d_sdf_jfa_sign_wgsl"),
            source: wgpu::ShaderSource::Wgsl(SDF_JFA_SIGN_WGSL.into()),
        });
        let outside_sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c3d_sdf_jfa_outside_wgsl"),
            source: wgpu::ShaderSource::Wgsl(SDF_JFA_OUTSIDE_WGSL.into()),
        });
        let scatter_sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c3d_sdf_jfa_scatter_wgsl"),
            source: wgpu::ShaderSource::Wgsl(SDF_JFA_SCATTER_WGSL.into()),
        });

        // --- Layout for Clear Pass (Resets ping/pong + seed_best_dist + sdf_out) ---
        let bgl_clear = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_sdf_jfa_clear_bgl"),
            entries: &[
                // Params
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
                // ping (write)
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
                // pong (write)
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
                // seed_best_dist (atomic u32 bits)
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
                // sdf_out (write)
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
                // surface_mask (atomic)
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // outside_mask (atomic)
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // queue_meta (head/tail atomics)
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
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

        // --- Layout for Seed Pass (Reads Triangles, Writes to Ping) ---
        let bgl_seed = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_sdf_jfa_seed_bgl"),
            entries: &[
                // Params
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
                // Triangles (Read Only)
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
                // Seed state buffer (ReadWrite)
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
                // Per-voxel best distance (atomic u32 bits)
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
                // surface_mask (atomic u32)
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

        // --- Layout for Flood & Sign Pass (Ping Pong between two state buffers) ---
        let bgl_flood = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_sdf_jfa_flood_bgl"),
            entries: &[
                // Params
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
                // Source State (Read Only)
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
                // Target State (Write Only)
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

        // --- Layout for Sign Pass (Read final state, output signed distance) ---
        let bgl_sign = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_sdf_jfa_sign_bgl"),
            entries: &[
                // Params
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
                // final state (read)
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
                // surface_mask (read)
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
                // outside_mask (read)
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // sdf_out (write)
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

        let bgl_outside = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_sdf_jfa_outside_bgl"),
            entries: &[
                // Params
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
                // surface_mask (read)
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
                // outside_mask (read_write)
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
                // queue_meta (read_write)
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
                // queue (read_write)
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

        // --- Layout for Scatter Pass (Dense sdf_out -> chunk_pool) ---
        let bgl_scatter = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("c3d_sdf_jfa_scatter_bgl"),
            entries: &[
                // scatter params
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
                // work_items (read)
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
                // sdf_in (read)
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
                // chunk_pool (write)
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
            ],
        });

        // Pipelines
        let clear_ppl = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_jfa_clear_pipeline"),
            layout: Some(
                &dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&bgl_clear],
                    immediate_size: 0,
                }),
            ),
            module: &clear_sm,
            entry_point: Some("clear_pass"),
            compilation_options: Default::default(),
            cache: None,
        });

        let seed_ppl = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_jfa_seed_pipeline"),
            layout: Some(
                &dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&bgl_seed],
                    immediate_size: 0,
                }),
            ),
            module: &seed_sm,
            entry_point: Some("seed_pass"),
            compilation_options: Default::default(),
            cache: None,
        });

        let flood_ppl = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_jfa_flood_pipeline"),
            layout: Some(
                &dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&bgl_flood],
                    immediate_size: 0,
                }),
            ),
            module: &flood_sm,
            entry_point: Some("flood_pass"),
            compilation_options: Default::default(),
            cache: None,
        });

        let sign_ppl = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_jfa_sign_pipeline"),
            layout: Some(
                &dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&bgl_sign],
                    immediate_size: 0,
                }),
            ),
            module: &sign_sm,
            entry_point: Some("sign_pass"),
            compilation_options: Default::default(),
            cache: None,
        });

        let outside_seed_ppl = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_jfa_outside_seed_pipeline"),
            layout: Some(
                &dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&bgl_outside],
                    immediate_size: 0,
                }),
            ),
            module: &outside_sm,
            entry_point: Some("outside_seed"),
            compilation_options: Default::default(),
            cache: None,
        });

        let outside_bfs_ppl = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_jfa_outside_bfs_pipeline"),
            layout: Some(
                &dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&bgl_outside],
                    immediate_size: 0,
                }),
            ),
            module: &outside_sm,
            entry_point: Some("outside_bfs"),
            compilation_options: Default::default(),
            cache: None,
        });

        let scatter_ppl = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_jfa_scatter_pipeline"),
            layout: Some(
                &dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&bgl_scatter],
                    immediate_size: 0,
                }),
            ),
            module: &scatter_sm,
            entry_point: Some("scatter_pass"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Allocate default 64x64x64 buffer (can be resized dynamically later)
        let max_cells = 64 * 64 * 64;
        let params_buffer = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_params"),
            size: std::mem::size_of::<JfaParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scatter_params = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_scatter_params"),
            size: std::mem::size_of::<JfaScatterParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cell_bytes = std::mem::size_of::<JfaCell>() as u64;
        let ping_buffer = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_ping"),
            size: (max_cells as u64) * cell_bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pong_buffer = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_pong"),
            size: (max_cells as u64) * cell_bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let seed_best_dist = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_seed_best_dist"),
            size: (max_cells as u64) * 4u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let surface_mask = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_surface_mask"),
            size: (max_cells as u64) * 4u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let outside_mask = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_outside_mask"),
            size: (max_cells as u64) * 4u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let queue_meta = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_queue_meta"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let queue = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_queue"),
            size: (max_cells as u64) * 4u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let sdf_out = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_sdf_out"),
            size: (max_cells as u64) * 4u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging_buffer = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_staging"),
            size: (max_cells as u64) * 4u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            clear_ppl,
            seed_ppl,
            flood_ppl,
            sign_ppl,
            outside_seed_ppl,
            outside_bfs_ppl,
            scatter_ppl,
            bgl_clear,
            bgl_seed,
            bgl_flood,
            bgl_sign,
            bgl_outside,
            bgl_scatter,
            params_buffer,
            scatter_params,
            ping_buffer,
            pong_buffer,
            seed_best_dist,
            surface_mask,
            outside_mask,
            queue_meta,
            queue,
            sdf_out,
            staging_buffer,
            max_cells,
        }
    }

    fn ensure_capacity(&mut self, rt: &GpuRuntime, cells_needed: u32) {
        if self.max_cells >= cells_needed {
            return;
        }
        self.max_cells = cells_needed.next_power_of_two();
        let dev = rt.device();
        let cell_bytes = std::mem::size_of::<JfaCell>() as u64;
        let state_size = (self.max_cells as u64) * cell_bytes;

        self.ping_buffer = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_ping_resized"),
            size: state_size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.pong_buffer = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_pong_resized"),
            size: state_size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.seed_best_dist = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_seed_best_dist_resized"),
            size: (self.max_cells as u64) * 4u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.surface_mask = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_surface_mask_resized"),
            size: (self.max_cells as u64) * 4u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.outside_mask = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_outside_mask_resized"),
            size: (self.max_cells as u64) * 4u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue_meta = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_queue_meta_resized"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_queue_resized"),
            size: (self.max_cells as u64) * 4u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        self.sdf_out = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_sdf_out_resized"),
            size: (self.max_cells as u64) * 4u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        self.staging_buffer = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_staging_resized"),
            size: (self.max_cells as u64) * 4u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }

    fn encode_jfa_to_sdf_out(
        &mut self,
        dev: &wgpu::Device,
        q: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
        tri_buffer: &wgpu::Buffer,
        triangles_len: u32,
        total_cells: u32,
        res_x: i32,
        res_y: i32,
        res_z: i32,
        params: &mut JfaParams,
    ) {
        // --- CLEAR PASS ---
        q.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(params));
        let clear_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3d_jfa_clear_bg"),
            layout: &self.bgl_clear,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.ping_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.pong_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.seed_best_dist.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.sdf_out.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: self.surface_mask.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: self.outside_mask.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: self.queue_meta.as_entire_binding() },
            ],
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("JFA Clear"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.clear_ppl);
            pass.set_bind_group(0, &clear_bg, &[]);
            pass.dispatch_workgroups((total_cells + 255) / 256, 1, 1);
        }

        // --- SEED PASS ---
        let seed_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3d_jfa_seed_bg"),
            layout: &self.bgl_seed,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: tri_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.ping_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.seed_best_dist.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.surface_mask.as_entire_binding() },
            ],
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("JFA Seed"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.seed_ppl);
            pass.set_bind_group(0, &seed_bg, &[]);
            let tri_wg = (triangles_len + 63) / 64;
            pass.dispatch_workgroups(tri_wg.max(1), 1, 1);
        }

        // --- FLOOD PASSES ---
        let max_dim = res_x.max(res_y).max(res_z);
        let mut pow2 = 1u32;
        while pow2 < max_dim.max(1) as u32 {
            pow2 <<= 1;
        }
        let mut step = (pow2 / 2).max(1) as i32;
        let mut ping_pong_toggle = true; // true = read ping, write pong

        while step >= 1 {
            params.step_size = step;
            q.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(params));

            let (src_buf, dst_buf) = if ping_pong_toggle {
                (&self.ping_buffer, &self.pong_buffer)
            } else {
                (&self.pong_buffer, &self.ping_buffer)
            };

            let flood_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("c3d_jfa_flood_bg"),
                layout: &self.bgl_flood,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.params_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: src_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: dst_buf.as_entire_binding() },
                ],
            });

            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("JFA Flood"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.flood_ppl);
                pass.set_bind_group(0, &flood_bg, &[]);
                pass.dispatch_workgroups((total_cells + 255) / 256, 1, 1);
            }

            step /= 2;
            ping_pong_toggle = !ping_pong_toggle;
        }

        let final_state = if !ping_pong_toggle {
            &self.pong_buffer
        } else {
            &self.ping_buffer
        };

        // --- ROBUST SIGN: outside fill on surface mask (orientation-free) ---
        let outside_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3d_jfa_outside_bg"),
            layout: &self.bgl_outside,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.surface_mask.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.outside_mask.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.queue_meta.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.queue.as_entire_binding() },
            ],
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("JFA Outside Seed"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.outside_seed_ppl);
            pass.set_bind_group(0, &outside_bg, &[]);
            pass.dispatch_workgroups((total_cells + 255) / 256, 1, 1);
        }
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("JFA Outside BFS"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.outside_bfs_ppl);
            pass.set_bind_group(0, &outside_bg, &[]);
            let bfs_wg = ((total_cells + 255) / 256).min(2048).max(1u32);
            pass.dispatch_workgroups(bfs_wg, 1, 1);
        }

        let sign_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3d_jfa_sign_bg"),
            layout: &self.bgl_sign,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: final_state.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.surface_mask.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.outside_mask.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.sdf_out.as_entire_binding() },
            ],
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("JFA Sign"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.sign_ppl);
            pass.set_bind_group(0, &sign_bg, &[]);
            pass.dispatch_workgroups((total_cells + 255) / 256, 1, 1);
        }
    }

    /// Converts a mesh to an SDF Grid block synchronously for editor/baking use.
    pub fn compute_mesh_to_sdf(
        &mut self,
        rt: &GpuRuntime,
        triangles: &[JfaTriangle],
        grid_min: Vec3,
        grid_max: Vec3,
        voxel_size: f32,
    ) -> Vec<f32> {
        let dev = rt.device();
        let q = rt.queue();

        // 1. Setup Grid Resolution
        let extent = grid_max - grid_min;
        let res_x = (extent.x / voxel_size).ceil() as i32 + 1;
        let res_y = (extent.y / voxel_size).ceil() as i32 + 1;
        let res_z = (extent.z / voxel_size).ceil() as i32 + 1;
        let total_cells = (res_x * res_y * res_z) as u32;

        self.ensure_capacity(rt, total_cells);

        // Upload Triangles
        let tri_buffer = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_tri_buffer"),
            size: (triangles.len() * std::mem::size_of::<JfaTriangle>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        q.write_buffer(&tri_buffer, 0, bytemuck::cast_slice(triangles));

        // Initial Params
        let mut params = JfaParams {
            grid_min: grid_min.to_array(),
            voxel_size,
            grid_max: grid_max.to_array(),
            step_size: 0,
            grid_res: [res_x, res_y, res_z],
            num_triangles: triangles.len() as u32,
            seed_band_world: voxel_size.max(0.00001) * 1.5,
            surface_band_world: voxel_size.max(0.00001) * 0.9,
            background_value: 3.402823e38,
            _pad0: 0,
        };

        let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("c3d_jfa_enc"),
        });

        self.encode_jfa_to_sdf_out(
            dev,
            q,
            &mut enc,
            &tri_buffer,
            triangles.len() as u32,
            total_cells,
            res_x,
            res_y,
            res_z,
            &mut params,
        );

        // Copy SDF to staging
        enc.copy_buffer_to_buffer(
            &self.sdf_out,
            0,
            &self.staging_buffer,
            0,
            (total_cells as u64) * 4u64,
        );

        q.submit([enc.finish()]);

        // Synchronous Readback (Blocking)
        let (tx, rx) = std::sync::mpsc::channel();
        let staging_slice = self.staging_buffer.slice(0..(total_cells as u64) * 4u64);
        staging_slice.map_async(wgpu::MapMode::Read, move |res| {
            tx.send(res).unwrap();
        });

        dev.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();

        let mapped = staging_slice.get_mapped_range();
        let distances: Vec<f32> = bytemuck::cast_slice::<u8, f32>(&mapped).to_vec();

        drop(mapped);
        self.staging_buffer.unmap();

        distances
    }

    /// Computes a dense SDF over the axis-aligned chunk region and scatters it into `pool.chunk_pool`
    /// for the provided `chunk_keys` (via `pool.work_items`).
    ///
    /// This is intended for Clay Edit / sculpt workflows: keep SDF resident on the GPU and let
    /// extraction/rendering (Marching Cubes) operate on the same `SdfGpuPool`.
    pub fn compute_mesh_to_pool_chunks(
        &mut self,
        rt: &GpuRuntime,
        pool: &mut SdfGpuPool,
        triangles: &[JfaTriangle],
        chunk_keys: &[IVec3],
        voxel_size: f32,
        background_value: f32,
    ) {
        if triangles.is_empty() || chunk_keys.is_empty() {
            return;
        }

        let dev = rt.device();
        let q = rt.queue();

        // Ensure work-items (and thus slots) exist for the target chunk set.
        let work_items_count = pool.upload_work_items(rt, chunk_keys);
        if work_items_count == 0 {
            return;
        }
        let keys = &chunk_keys[..(work_items_count as usize).min(chunk_keys.len())];

        // Chunk-aligned dense grid bounds in voxel index-space.
        let mut min_ck = keys[0];
        let mut max_ck = keys[0];
        for &ck in keys.iter().skip(1) {
            min_ck = IVec3::new(min_ck.x.min(ck.x), min_ck.y.min(ck.y), min_ck.z.min(ck.z));
            max_ck = IVec3::new(max_ck.x.max(ck.x), max_ck.y.max(ck.y), max_ck.z.max(ck.z));
        }
        let min_vox = min_ck * CHUNK_SIZE;
        let dims_ck = (max_ck - min_ck) + IVec3::ONE;
        let dims_vox = dims_ck * CHUNK_SIZE;

        let res_x = dims_vox.x.max(1);
        let res_y = dims_vox.y.max(1);
        let res_z = dims_vox.z.max(1);
        let total_cells_u64 = (res_x as u64) * (res_y as u64) * (res_z as u64);
        let total_cells = u32::try_from(total_cells_u64).unwrap_or(u32::MAX);
        self.ensure_capacity(rt, total_cells);

        // World bounds for JFA evaluation (voxel centers use +0.5 in WGSL).
        let grid_min = Vec3::new(min_vox.x as f32, min_vox.y as f32, min_vox.z as f32) * voxel_size;
        let grid_max = grid_min + Vec3::new(res_x as f32, res_y as f32, res_z as f32) * voxel_size;

        // Upload triangles.
        let tri_buffer = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c3d_jfa_tri_buffer"),
            size: (triangles.len() * std::mem::size_of::<JfaTriangle>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        q.write_buffer(&tri_buffer, 0, bytemuck::cast_slice(triangles));

        let mut params = JfaParams {
            grid_min: grid_min.to_array(),
            voxel_size,
            grid_max: grid_max.to_array(),
            step_size: 0,
            grid_res: [res_x, res_y, res_z],
            num_triangles: triangles.len() as u32,
            seed_band_world: voxel_size.max(0.00001) * 1.5,
            surface_band_world: voxel_size.max(0.00001) * 0.9,
            background_value,
            _pad0: 0,
        };

        let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("c3d_jfa_pool_enc"),
        });

        self.encode_jfa_to_sdf_out(
            dev,
            q,
            &mut enc,
            &tri_buffer,
            triangles.len() as u32,
            total_cells,
            res_x,
            res_y,
            res_z,
            &mut params,
        );

        // Scatter dense SDF into chunk pool.
        let sp = JfaScatterParams {
            grid_min_vox: [min_vox.x, min_vox.y, min_vox.z, 0],
            grid_res: [res_x, res_y, res_z, 0],
            background_value,
            work_items_count,
            _pad0: [0; 2],
        };
        q.write_buffer(&self.scatter_params, 0, bytemuck::bytes_of(&sp));

        let scatter_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c3d_jfa_scatter_bg"),
            layout: &self.bgl_scatter,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.scatter_params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: pool.work_items.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.sdf_out.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: pool.chunk_pool.as_entire_binding() },
            ],
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("JFA Scatter"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.scatter_ppl);
            pass.set_bind_group(0, &scatter_bg, &[]);
            pass.dispatch_workgroups(work_items_count, 1, 1);
        }

        q.submit([enc.finish()]);
        let _ = dev.poll(wgpu::PollType::Poll);
    }
}

// ------------------------------------------------------------------------------------------
// WGSL Shader Payload (Jump Flood Algorithm)
// ------------------------------------------------------------------------------------------
const SDF_JFA_CLEAR_WGSL: &str = r#"
struct Params {
    grid_min: vec3<f32>,
    voxel_size: f32,
    grid_max: vec3<f32>,
    step_size: i32,
    grid_res: vec3<i32>,
    num_triangles: u32,
    seed_band_world: f32,
    surface_band_world: f32,
    background_value: f32,
    _pad0: u32,
};

struct Cell {
    cp_valid: vec4<f32>,
};

struct QueueMeta {
    head: atomic<u32>,
    tail: atomic<u32>,
    _pad0: vec2<u32>,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> ping: array<Cell>;
@group(0) @binding(2) var<storage, read_write> pong: array<Cell>;
@group(0) @binding(3) var<storage, read_write> seed_best_dist: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read_write> sdf_out: array<f32>;
@group(0) @binding(5) var<storage, read_write> surface_mask: array<atomic<u32>>;
@group(0) @binding(6) var<storage, read_write> outside_mask: array<atomic<u32>>;
@group(0) @binding(7) var<storage, read_write> qmeta: QueueMeta;

@compute @workgroup_size(256)
fn clear_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = i32(gid.x);
    let total = p.grid_res.x * p.grid_res.y * p.grid_res.z;
    if (idx >= total) { return; }
    ping[idx].cp_valid = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    pong[idx].cp_valid = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    atomicStore(&seed_best_dist[idx], 0x7f7fffffu);
    atomicStore(&surface_mask[idx], 0u);
    atomicStore(&outside_mask[idx], 0u);
    sdf_out[idx] = 0.0;
    if (idx == 0) {
        atomicStore(&qmeta.head, 0u);
        atomicStore(&qmeta.tail, 0u);
    }
}
"#;

const SDF_JFA_SEED_WGSL: &str = r#"
struct Params {
    grid_min: vec3<f32>,
    voxel_size: f32,
    grid_max: vec3<f32>,
    step_size: i32,
    grid_res: vec3<i32>,
    num_triangles: u32,
    seed_band_world: f32,
    surface_band_world: f32,
    background_value: f32,
    _pad0: u32,
};

struct Triangle {
    v0: vec3<f32>, _p0: u32,
    v1: vec3<f32>, _p1: u32,
    v2: vec3<f32>, _p2: u32,
};

struct Cell {
    cp_valid: vec4<f32>,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> tris: array<Triangle>;
@group(0) @binding(2) var<storage, read_write> grid_out: array<Cell>;
@group(0) @binding(3) var<storage, read_write> seed_best_dist: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read_write> surface_mask: array<atomic<u32>>;

fn get_idx(x: i32, y: i32, z: i32) -> i32 {
    return x + y * p.grid_res.x + z * p.grid_res.x * p.grid_res.y;
}

fn get_voxel_center(x: i32, y: i32, z: i32) -> vec3<f32> {
    return p.grid_min + vec3<f32>(f32(x) + 0.5, f32(y) + 0.5, f32(z) + 0.5) * p.voxel_size;
}

// --- Point to Triangle Distance Math ---
fn closest_point_on_triangle(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>) -> vec3<f32> {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if (d1 <= 0.0 && d2 <= 0.0) { return a; }

    let bp = p - b;
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if (d3 >= 0.0 && d4 <= d3) { return b; }

    let vc = d1*d4 - d3*d2;
    if (vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0) {
        let v = d1 / (d1 - d3);
        return a + v * ab;
    }

    let cp = p - c;
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if (d6 >= 0.0 && d5 <= d6) { return c; }

    let vb = d5*d2 - d1*d6;
    if (vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0) {
        let w = d2 / (d2 - d6);
        return a + w * ac;
    }

    let va = d3*d6 - d5*d4;
    if (va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0) {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return b + w * (c - b);
    }

    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    return a + ab * v + ac * w;
}

fn update_seed(idx: i32, cp: vec3<f32>, dist: f32) {
    let new_bits = bitcast<u32>(dist);
    var old = atomicLoad(&seed_best_dist[idx]);
    loop {
        if (new_bits >= old) { break; }
        let r = atomicCompareExchangeWeak(&seed_best_dist[idx], old, new_bits);
        if (r.exchanged) {
            grid_out[idx].cp_valid = vec4<f32>(cp, 1.0);
            break;
        }
        old = r.old_value;
    }
}

// ============================================================================
// Seed Pass (Thread per Triangle) — narrow-band voxelization into initial seeds
// ============================================================================
@compute @workgroup_size(64)
fn seed_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let ti = gid.x;
    if (ti >= p.num_triangles) { return; }
    let t = tris[ti];
    let a = t.v0;
    let b = t.v1;
    let c = t.v2;

    let inv_vs = 1.0 / max(p.voxel_size, 1e-9);
    let band_vox = max(p.seed_band_world * inv_vs, 0.0);

    let minw = min(a, min(b, c));
    let maxw = max(a, max(b, c));
    let minv = (minw - p.grid_min) * inv_vs - vec3<f32>(band_vox, band_vox, band_vox);
    let maxv = (maxw - p.grid_min) * inv_vs + vec3<f32>(band_vox, band_vox, band_vox);

    var x0 = i32(floor(minv.x));
    var y0 = i32(floor(minv.y));
    var z0 = i32(floor(minv.z));
    var x1 = i32(ceil(maxv.x));
    var y1 = i32(ceil(maxv.y));
    var z1 = i32(ceil(maxv.z));

    x0 = clamp(x0, 0, p.grid_res.x - 1);
    y0 = clamp(y0, 0, p.grid_res.y - 1);
    z0 = clamp(z0, 0, p.grid_res.z - 1);
    x1 = clamp(x1, 0, p.grid_res.x - 1);
    y1 = clamp(y1, 0, p.grid_res.y - 1);
    z1 = clamp(z1, 0, p.grid_res.z - 1);

    var z = z0;
    loop {
        if (z > z1) { break; }
        var y = y0;
        loop {
            if (y > y1) { break; }
            var x = x0;
            loop {
                if (x > x1) { break; }
                let idx = get_idx(x, y, z);
                let center = get_voxel_center(x, y, z);
                let cp = closest_point_on_triangle(center, a, b, c);
                let d = distance(center, cp);
                if (d <= p.surface_band_world) {
                    atomicOr(&surface_mask[idx], 1u);
                }
                if (d <= p.seed_band_world) {
                    update_seed(idx, cp, d);
                }
                x = x + 1;
            }
            y = y + 1;
        }
        z = z + 1;
    }
}
"#;

const SDF_JFA_FLOOD_WGSL: &str = r#"
struct Params {
    grid_min: vec3<f32>,
    voxel_size: f32,
    grid_max: vec3<f32>,
    step_size: i32,
    grid_res: vec3<i32>,
    num_triangles: u32,
    seed_band_world: f32,
    surface_band_world: f32,
    background_value: f32,
    _pad0: u32,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> grid_in: array<Cell>;
@group(0) @binding(2) var<storage, read_write> grid_out: array<Cell>;

struct Cell {
    cp_valid: vec4<f32>,
};

fn get_idx(x: i32, y: i32, z: i32) -> i32 {
    return x + y * p.grid_res.x + z * p.grid_res.x * p.grid_res.y;
}

fn get_voxel_center(x: i32, y: i32, z: i32) -> vec3<f32> {
    return p.grid_min + vec3<f32>(f32(x) + 0.5, f32(y) + 0.5, f32(z) + 0.5) * p.voxel_size;
}

// ============================================================================
// Flood Pass (Thread per Voxel)
// ============================================================================
@compute @workgroup_size(256)
fn flood_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = i32(gid.x);
    let total = p.grid_res.x * p.grid_res.y * p.grid_res.z;
    if (idx >= total) { return; }

    let x = idx % p.grid_res.x;
    let y = (idx / p.grid_res.x) % p.grid_res.y;
    let z = idx / (p.grid_res.x * p.grid_res.y);

    let center = get_voxel_center(x, y, z);

    // Read current state
    var best_cp = grid_in[idx].cp_valid.xyz;
    var best_valid = grid_in[idx].cp_valid.w;
    var best_dist = 3.402823e38;
    if (best_valid > 0.0) {
        best_dist = distance(center, best_cp);
    }

    // Check 26 neighbors at `step_size` distance
    for (var dz = -1; dz <= 1; dz++) {
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                if (dx == 0 && dy == 0 && dz == 0) { continue; }

                let nx = x + dx * p.step_size;
                let ny = y + dy * p.step_size;
                let nz = z + dz * p.step_size;

                if (nx >= 0 && nx < p.grid_res.x && ny >= 0 && ny < p.grid_res.y && nz >= 0 && nz < p.grid_res.z) {
                    let n_idx = get_idx(nx, ny, nz);
                    let nd = grid_in[n_idx];
                    if (nd.cp_valid.w > 0.0) {
                        let dist_to_surface = distance(center, nd.cp_valid.xyz);
                        if (dist_to_surface < best_dist) {
                            best_dist = dist_to_surface;
                            best_cp = nd.cp_valid.xyz;
                            best_valid = 1.0;
                        }
                    }
                }
            }
        }
    }

    if (best_valid > 0.0) {
        grid_out[idx].cp_valid = vec4<f32>(best_cp, 1.0);
    } else {
        grid_out[idx].cp_valid = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
}
"#;

const SDF_JFA_SIGN_WGSL: &str = r#"
struct Params {
    grid_min: vec3<f32>,
    voxel_size: f32,
    grid_max: vec3<f32>,
    step_size: i32,
    grid_res: vec3<i32>,
    num_triangles: u32,
    seed_band_world: f32,
    surface_band_world: f32,
    background_value: f32,
    _pad0: u32,
};

struct Cell {
    cp_valid: vec4<f32>,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> grid_in: array<Cell>;
@group(0) @binding(2) var<storage, read> surface_mask: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read> outside_mask: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read_write> sdf_out: array<f32>;

fn voxel_center(x: i32, y: i32, z: i32) -> vec3<f32> {
    return p.grid_min + vec3<f32>(f32(x) + 0.5, f32(y) + 0.5, f32(z) + 0.5) * p.voxel_size;
}

fn idx_of(x: i32, y: i32, z: i32) -> i32 {
    return x + y * p.grid_res.x + z * p.grid_res.x * p.grid_res.y;
}

fn outside_at(idx: i32) -> bool {
    return atomicLoad(&outside_mask[idx]) != 0u;
}

fn surface_at(idx: i32) -> bool {
    return atomicLoad(&surface_mask[idx]) != 0u;
}

@compute @workgroup_size(256)
fn sign_pass(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = i32(gid.x);
    let total = p.grid_res.x * p.grid_res.y * p.grid_res.z;
    if (idx >= total) { return; }

    let x = idx % p.grid_res.x;
    let y = (idx / p.grid_res.x) % p.grid_res.y;
    let z = idx / (p.grid_res.x * p.grid_res.y);

    // Determine sign using an outside flood fill over a surface mask.
    var is_outside = outside_at(idx);
    if (surface_at(idx)) {
        // Surface voxels are blocked from flood fill; classify by neighbor majority.
        var c_out: u32 = 0u;
        if (x > 0 && outside_at(idx_of(x - 1, y, z))) { c_out = c_out + 1u; }
        if (x + 1 < p.grid_res.x && outside_at(idx_of(x + 1, y, z))) { c_out = c_out + 1u; }
        if (y > 0 && outside_at(idx_of(x, y - 1, z))) { c_out = c_out + 1u; }
        if (y + 1 < p.grid_res.y && outside_at(idx_of(x, y + 1, z))) { c_out = c_out + 1u; }
        if (z > 0 && outside_at(idx_of(x, y, z - 1))) { c_out = c_out + 1u; }
        if (z + 1 < p.grid_res.z && outside_at(idx_of(x, y, z + 1))) { c_out = c_out + 1u; }
        is_outside = (c_out >= 3u);
    }

    let sign = select(-1.0, 1.0, is_outside);

    let cell = grid_in[idx];
    if (cell.cp_valid.w <= 0.0) {
        sdf_out[idx] = p.background_value * sign;
        return;
    }

    let c = voxel_center(x, y, z);
    let d = distance(c, cell.cp_valid.xyz);
    sdf_out[idx] = d * sign;
}
"#;

const SDF_JFA_OUTSIDE_WGSL: &str = r#"
struct Params {
    grid_min: vec3<f32>,
    voxel_size: f32,
    grid_max: vec3<f32>,
    step_size: i32,
    grid_res: vec3<i32>,
    num_triangles: u32,
    seed_band_world: f32,
    surface_band_world: f32,
    background_value: f32,
    _pad0: u32,
};

struct QueueMeta {
    head: atomic<u32>,
    tail: atomic<u32>,
    _pad0: vec2<u32>,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> surface_mask: array<atomic<u32>>;
@group(0) @binding(2) var<storage, read_write> outside_mask: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read_write> qmeta: QueueMeta;
@group(0) @binding(4) var<storage, read_write> queue: array<u32>;

fn idx_of(x: i32, y: i32, z: i32) -> i32 {
    return x + y * p.grid_res.x + z * p.grid_res.x * p.grid_res.y;
}

fn surface_at(idx: i32) -> bool {
    return atomicLoad(&surface_mask[idx]) != 0u;
}

fn try_mark_outside(idx: i32) -> bool {
    let r = atomicCompareExchangeWeak(&outside_mask[idx], 0u, 1u);
    return r.exchanged;
}

@compute @workgroup_size(256)
fn outside_seed(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = i32(gid.x);
    let total = p.grid_res.x * p.grid_res.y * p.grid_res.z;
    if (idx >= total) { return; }
    if (surface_at(idx)) { return; }

    let x = idx % p.grid_res.x;
    let y = (idx / p.grid_res.x) % p.grid_res.y;
    let z = idx / (p.grid_res.x * p.grid_res.y);
    let boundary = (x == 0) || (y == 0) || (z == 0) || (x == p.grid_res.x - 1) || (y == p.grid_res.y - 1) || (z == p.grid_res.z - 1);
    if (!boundary) { return; }
    if (try_mark_outside(idx)) {
        let t = atomicAdd(&qmeta.tail, 1u);
        queue[t] = u32(idx);
    }
}

@compute @workgroup_size(256)
fn outside_bfs(@builtin(global_invocation_id) _gid: vec3<u32>) {
    loop {
        let qi = atomicAdd(&qmeta.head, 1u);
        if (qi >= atomicLoad(&qmeta.tail)) { break; }

        let idx = i32(queue[qi]);
        let x = idx % p.grid_res.x;
        let y = (idx / p.grid_res.x) % p.grid_res.y;
        let z = idx / (p.grid_res.x * p.grid_res.y);

        // 6-neighborhood for robust connectivity
        if (x > 0) {
            let ni = idx_of(x - 1, y, z);
            if (!surface_at(ni) && try_mark_outside(ni)) { let t = atomicAdd(&qmeta.tail, 1u); queue[t] = u32(ni); }
        }
        if (x + 1 < p.grid_res.x) {
            let ni = idx_of(x + 1, y, z);
            if (!surface_at(ni) && try_mark_outside(ni)) { let t = atomicAdd(&qmeta.tail, 1u); queue[t] = u32(ni); }
        }
        if (y > 0) {
            let ni = idx_of(x, y - 1, z);
            if (!surface_at(ni) && try_mark_outside(ni)) { let t = atomicAdd(&qmeta.tail, 1u); queue[t] = u32(ni); }
        }
        if (y + 1 < p.grid_res.y) {
            let ni = idx_of(x, y + 1, z);
            if (!surface_at(ni) && try_mark_outside(ni)) { let t = atomicAdd(&qmeta.tail, 1u); queue[t] = u32(ni); }
        }
        if (z > 0) {
            let ni = idx_of(x, y, z - 1);
            if (!surface_at(ni) && try_mark_outside(ni)) { let t = atomicAdd(&qmeta.tail, 1u); queue[t] = u32(ni); }
        }
        if (z + 1 < p.grid_res.z) {
            let ni = idx_of(x, y, z + 1);
            if (!surface_at(ni) && try_mark_outside(ni)) { let t = atomicAdd(&qmeta.tail, 1u); queue[t] = u32(ni); }
        }
    }
}
"#;

const SDF_JFA_SCATTER_WGSL: &str = r#"
const CHUNK_SIZE: u32 = 16u;
const CHUNK_VOXELS: u32 = 4096u;
const MISSING_SLOT: u32 = 0xffffffffu;

struct Params {
  grid_min_vox: vec4<i32>,
  grid_res: vec4<i32>,
  background_value: f32,
  work_items_count: u32,
  _pad0: vec2<u32>,
};

struct WorkItem {
  chunk_key: vec4<i32>,
  slot: u32,
  _pad0: array<u32, 3>,
  neighbor_slots: array<u32, 27>,
  _pad1: u32,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> work_items: array<WorkItem>;
@group(0) @binding(2) var<storage, read> sdf_in: array<f32>;
@group(0) @binding(3) var<storage, read_write> chunk_pool: array<f32>;

@compute @workgroup_size(256)
fn scatter_pass(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
  let wi = wg.x;
  if (wi >= p.work_items_count) { return; }

  let item = work_items[wi];
  let slot = item.slot;
  if (slot == MISSING_SLOT) { return; }
  let base = slot * CHUNK_VOXELS;

  let ck = vec3<i32>(item.chunk_key.x, item.chunk_key.y, item.chunk_key.z);
  let chunk_origin = ck * i32(CHUNK_SIZE);
  let grid_min = vec3<i32>(p.grid_min_vox.x, p.grid_min_vox.y, p.grid_min_vox.z);
  let res = vec3<i32>(p.grid_res.x, p.grid_res.y, p.grid_res.z);

  var i = lid.x;
  loop {
    if (i >= CHUNK_VOXELS) { break; }
    let x = i32(i % CHUNK_SIZE);
    let y = i32((i / CHUNK_SIZE) % CHUNK_SIZE);
    let z = i32(i / (CHUNK_SIZE * CHUNK_SIZE));
    let gv = vec3<i32>(chunk_origin.x + x, chunk_origin.y + y, chunk_origin.z + z);
    let dv = gv - grid_min;

    var v = p.background_value;
    if (dv.x >= 0 && dv.y >= 0 && dv.z >= 0 && dv.x < res.x && dv.y < res.y && dv.z < res.z) {
      let rx = u32(res.x);
      let ry = u32(res.y);
      let idx = u32(dv.x) + u32(dv.y) * rx + u32(dv.z) * rx * ry;
      v = sdf_in[idx];
    }
    chunk_pool[base + i] = v;
    i = i + 256u;
  }
}
"#;
