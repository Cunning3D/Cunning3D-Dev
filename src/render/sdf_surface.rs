//! GPU SDF surface preview renderer (Marching Cubes -> GPU vertex buffer -> indirect draw).
//!
//! Goal: zero CPU triangle generation in the hot path. We upload sparse SDF chunks to GPU,
//! run batched Marching Cubes in a compute pass, and render the resulting triangles with
//! `draw_indirect`.

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::{
    core_3d::{Opaque3d, CORE_3D_DEPTH_FORMAT},
    oit::OrderIndependentTransparencySettings,
    prepass::ViewPrepassTextures,
};
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::system::SystemParam;
use bevy::ecs::system::{lifetimeless::SRes, SystemParamItem};
use bevy::mesh::{Mesh, Mesh3d, MeshVertexBufferLayoutRef, VertexBufferLayout, VertexFormat};
use bevy::pbr::{
    ExtractedAtmosphere, MeshPipeline, MeshPipelineKey, RenderMeshInstances, SetMeshViewBindGroup,
    SetMeshViewBindingArrayBindGroup,
};
use bevy::prelude::*;
use bevy::render::{
    batching::gpu_preprocessing::GpuPreprocessingSupport,
    mesh::{allocator::MeshAllocator, RenderMesh},
    render_asset::RenderAssets,
    render_phase::{
        AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, InputUniformIndex, PhaseItem,
        RenderCommand, RenderCommandResult, SetItemPipeline, TrackedRenderPass,
        ViewBinnedRenderPhases,
    },
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
    sync_world::RenderEntity,
    view::{ExtractedView, RenderVisibleEntities},
    Extract, Render, RenderApp, RenderSystems,
};
use bytemuck::{Pod, Zeroable};
use rustc_hash::FxHashMap;
use std::sync::{Arc, Mutex};

use crate::cunning_core::core::geometry::sdf::SdfGrid;
use crate::sdf::SdfHandle;
use crate::sdf_engine::gpu_marching_cubes_batched::{
    McBatchParams, McVertex, SDF_MC_BATCH_ARGS_WGSL, SDF_MC_BATCH_WGSL,
};
use crate::sdf_engine::gpu_pool::{neighbor_index, SdfWorkItem, CHUNK_VOXELS, MISSING_SLOT};
use crate::sdf::SdfChunk;

pub struct CunningSdfSurfacePlugin;

#[derive(Resource, Clone)]
pub struct SdfSurfaceShader(pub Handle<Shader>);

// ---------------- Main World -> Render World: brush stroke queue ----------------

#[derive(Clone, Copy, Debug, Default)]
pub struct SdfBrushStroke {
    /// Arc pointer of `SdfHandle.grid` (stable identity).
    pub target_ptr: usize,
    pub a_world: Vec3,
    pub b_world: Vec3,
    pub radius_world: f32,
    pub smooth_k_world: f32,
    /// 0 = union/add, 1 = subtract, 2 = intersect
    pub mode: u32,
}

#[derive(Resource, Clone)]
pub struct SdfBrushStrokeQueueShared(pub Arc<Mutex<Vec<SdfBrushStroke>>>);

impl Default for SdfBrushStrokeQueueShared {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}

/// Dummy mesh used only to get the entity into the visible list + provide a stable asset id for binning.
#[derive(Resource, Clone)]
pub struct SdfSurfaceDummyMesh(pub Handle<Mesh>);

impl FromWorld for SdfSurfaceDummyMesh {
    fn from_world(world: &mut World) -> Self {
        let mut meshes = world.resource_mut::<Assets<Mesh>>();
        let mut m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        m.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0f32, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]],
        );
        m.insert_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            vec![[0.0f32, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
        );
        Self(meshes.add(m))
    }
}

/// Main-world marker for a GPU SDF surface preview.
#[derive(Component, Clone)]
pub struct SdfSurfaceViz {
    pub handle: SdfHandle,
    pub iso_value: f32,
    pub invert: bool,
    pub color: Vec4,
    /// 0 = PBR (scene lights), 1 = Clay/MatCap-like (camera-stable)
    pub shading_mode: u32,
    /// PBR perceptual roughness in [0..1]. (Also influences clay highlight sharpness.)
    pub roughness: f32,
    /// Rim light strength in [0..1].
    pub rim_strength: f32,
    /// Cavity/dirt strength in [0..1].
    pub cavity_strength: f32,
    /// Rim exponent (typ. 2..6).
    pub rim_power: f32,
    /// Clay specular boost in [0..2].
    pub clay_spec: f32,
}

impl Default for SdfSurfaceViz {
    fn default() -> Self {
        Self {
            handle: SdfHandle::new(SdfGrid::new(0.1, 0.0)),
            iso_value: 0.0,
            invert: false,
            color: Vec4::new(0.6, 0.6, 0.65, 1.0),
            shading_mode: 1,
            roughness: 0.78,
            rim_strength: 0.35,
            cavity_strength: 0.30,
            rim_power: 3.0,
            clay_spec: 1.0,
        }
    }
}

impl Plugin for CunningSdfSurfacePlugin {
    fn build(&self, app: &mut App) {
        let render_h = {
            let mut shaders = app.world_mut().resource_mut::<Assets<Shader>>();
            shaders.add(Shader::from_wgsl(
                include_str!("../../assets/shaders/cunning_sdf_surface.wgsl"),
                "shaders/cunning_sdf_surface.wgsl",
            ))
        };
        app.insert_resource(SdfSurfaceShader(render_h));
        app.init_resource::<SdfSurfaceDummyMesh>();
        app.init_resource::<SdfBrushStrokeQueueShared>();
        app.add_systems(Startup, sdf_surface_smoke_startup);
        app.add_systems(Update, sdf_surface_smoke_quit_update);
    }

    fn finish(&self, app: &mut App) {
        let shader = app.world().resource::<SdfSurfaceShader>().0.clone();
        let strokes = app.world().resource::<SdfBrushStrokeQueueShared>().clone();
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.insert_resource(SdfSurfaceShader(shader));
            render_app.insert_resource(strokes);
            render_app
                .init_resource::<SdfSurfacePipeline>()
                .init_resource::<SpecializedMeshPipelines<SdfSurfacePipeline>>()
                .init_resource::<SdfSurfaceGpuCache>()
                .add_render_command::<Opaque3d, DrawSdfSurfacePipeline>()
                .add_systems(ExtractSchedule, extract_sdf_surfaces)
                .add_systems(Render, queue_sdf_surfaces.in_set(RenderSystems::Queue));
        }
    }
}

fn sdf_surface_smoke_startup(mut commands: Commands, dummy: Res<SdfSurfaceDummyMesh>) {
    if std::env::var_os("C3D_SDF_SMOKE").is_none() {
        return;
    }
    // Minimal SDF for render pipeline smoke testing (no node graph required).
    // One chunk at origin containing a sphere, iso=0.
    let voxel_size = 0.1f32;
    let background_value = 1.0f32;
    let mut grid = SdfGrid::new(voxel_size, background_value);
    let mut ch = SdfChunk::new(background_value);

    let center = Vec3::new(8.0, 8.0, 8.0) * voxel_size;
    let radius = 0.6f32;
    for z in 0..16 {
        for y in 0..16 {
            for x in 0..16 {
                let p = (Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5)) * voxel_size;
                let d = (p - center).length() - radius;
                ch.set(x, y, z, d.clamp(-background_value, background_value));
            }
        }
    }
    grid.chunks.insert(IVec3::ZERO, ch);

    let handle = SdfHandle::new(grid);
    commands.spawn((
        Name::new("SDF Surface Smoke"),
        Mesh3d(dummy.0.clone()),
        bevy::camera::visibility::NoFrustumCulling,
        bevy::render::sync_world::SyncToRenderWorld,
        SdfSurfaceViz {
            handle,
            iso_value: 0.0,
            invert: false,
            color: Vec4::new(0.65, 0.65, 0.70, 1.0),
            ..Default::default()
        },
        Transform::default(),
        GlobalTransform::default(),
        Visibility::Visible,
        bevy::camera::visibility::InheritedVisibility::default(),
        bevy::camera::visibility::ViewVisibility::default(),
    ));
    eprintln!("[c3d][sdf_surface] SMOKE spawned (set C3D_SDF_DEBUG_READBACK=1 to read out_count)");
}

fn sdf_surface_smoke_quit_update(
    mut exit: MessageWriter<bevy::app::AppExit>,
    mut frames: Local<u32>,
) {
    if std::env::var_os("C3D_SDF_SMOKE_QUIT").is_none() {
        return;
    }
    *frames = frames.saturating_add(1);
    // Allow a short window for render/extract/queue to run and print debug readback.
    if *frames == 1 {
        eprintln!("[c3d][sdf_surface] SMOKE_QUIT armed");
    }
    if *frames >= 240 {
        eprintln!("[c3d][sdf_surface] SMOKE_QUIT timeout exit");
        exit.write(bevy::app::AppExit::Success);
    }
}

// ---------------- Render World: extract ----------------

#[derive(Component, Clone)]
pub struct RenderSdfSurface {
    pub handle: SdfHandle,
    pub mesh_handle: Handle<Mesh>,
    pub iso_value: f32,
    pub invert: bool,
    pub color: Vec4,
    pub shading_mode: u32,
    pub roughness: f32,
    pub rim_strength: f32,
    pub cavity_strength: f32,
    pub rim_power: f32,
    pub clay_spec: f32,
}

pub fn extract_sdf_surfaces(
    mut commands: Commands,
    query: Extract<Query<(RenderEntity, &SdfSurfaceViz, &Mesh3d, &ViewVisibility)>>,
    mut dbg: Local<SdfSurfaceExtractDebug>,
) {
    puffin::profile_function!();
    let dbg_enabled = dbg.enabled();
    let dbg_force_extract = std::env::var_os("C3D_SDF_DEBUG_READBACK").is_some()
        || std::env::var_os("C3D_SDF_SMOKE").is_some();
    dbg.frame = dbg.frame.wrapping_add(1);
    let mut seen = 0usize;
    let mut inserted = 0usize;
    for (re, s, _mesh, vis) in &query {
        seen += 1;
        if !vis.get() && !dbg_force_extract {
            continue;
        }
        commands.entity(re).insert(RenderSdfSurface {
            handle: s.handle.clone(),
            mesh_handle: _mesh.0.clone(),
            iso_value: s.iso_value,
            invert: s.invert,
            color: s.color,
            shading_mode: s.shading_mode,
            roughness: s.roughness,
            rim_strength: s.rim_strength,
            cavity_strength: s.cavity_strength,
            rim_power: s.rim_power,
            clay_spec: s.clay_spec,
        });
        inserted += 1;
    }
    if dbg_enabled {
        commands.insert_resource(ExtractedSdfSurfaceStats {
            frame: dbg.frame,
            seen: seen as u32,
            inserted: inserted as u32,
        });
    }
    if dbg_enabled && dbg.should_print() {
        eprintln!("[c3d][sdf_surface] extract seen={} inserted={}", seen, inserted);
    }
}

#[derive(Resource, Default, Clone, Copy)]
struct ExtractedSdfSurfaceStats {
    frame: u64,
    seen: u32,
    inserted: u32,
}

// ---------------- Render World: pipeline ----------------

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, ShaderType)]
pub struct SdfUniform {
    pub model: Mat4,
    pub color: Vec4,
    /// x=mode (0/1), y=roughness, z=rim_strength, w=cavity_strength
    pub params0: Vec4,
    /// x=rim_power, y=clay_spec, z/w reserved
    pub params1: Vec4,
}

#[derive(Resource)]
pub struct SdfSurfacePipeline {
    pub shader: Handle<Shader>,
    pub mesh_pipeline: MeshPipeline,

    pub uniform_layout_desc: BindGroupLayoutDescriptor,

    pub mc_bgl: BindGroupLayout,
    pub mc_ppl: ComputePipeline,
    pub args_bgl: BindGroupLayout,
    pub args_ppl: ComputePipeline,
    pub brush_bgl: BindGroupLayout,
    pub brush_ppl: ComputePipeline,
    pub edge_table: Buffer,
    pub tri_table: Buffer,
}

impl FromWorld for SdfSurfacePipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let shader = world.resource::<SdfSurfaceShader>().0.clone();
        let mesh_pipeline = world.resource::<MeshPipeline>().clone();

        let uniform_entries = [BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: Some(<SdfUniform as ShaderType>::min_size()),
            },
            count: None,
        }];
        let uniform_layout_desc =
            BindGroupLayoutDescriptor::new("sdf_surface_uniform_layout", &uniform_entries);

        // Marching Cubes tables (GPU storage buffers; avoids huge WGSL constants).
        let edge_table_u32: Vec<u32> = marching_cubes::tables::EDGE_TABLE
            .iter()
            .map(|&v| v as u16 as u32)
            .collect();
        let tri_table_i32: Vec<i32> = marching_cubes::tables::TRI_TABLE
            .iter()
            .flat_map(|row| row.iter().copied())
            .map(|v| v as i32)
            .collect();

        let edge_table = render_device.create_buffer_with_data(&wgpu::util::BufferInitDescriptor {
            label: Some("c3d_sdf_mc_edge_table"),
            contents: bytemuck::cast_slice(&edge_table_u32),
            usage: BufferUsages::STORAGE,
        });
        let tri_table = render_device.create_buffer_with_data(&wgpu::util::BufferInitDescriptor {
            label: Some("c3d_sdf_mc_tri_table"),
            contents: bytemuck::cast_slice(&tri_table_i32),
            usage: BufferUsages::STORAGE,
        });

        let mc_entries = [
            // params
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // chunk_pool (read)
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // work_items (read)
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // out_verts (write)
            BindGroupLayoutEntry {
                binding: 3,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // out_count (atomic)
            BindGroupLayoutEntry {
                binding: 4,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // edge_table
            BindGroupLayoutEntry {
                binding: 5,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // tri_table
            BindGroupLayoutEntry {
                binding: 6,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        let mc_bgl = render_device.create_bind_group_layout("c3d_sdf_mc_bgl", &mc_entries);
        let mc_sm = unsafe {
            render_device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("c3d_sdf_mc_wgsl"),
                source: wgpu::ShaderSource::Wgsl(SDF_MC_BATCH_WGSL.into()),
            })
        };
        let mc_pl = render_device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("c3d_sdf_mc_pl"),
            bind_group_layouts: &[&mc_bgl],
            immediate_size: 0,
        });
        let mc_ppl = render_device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_sdf_mc_ppl"),
            layout: Some(&mc_pl),
            module: &mc_sm,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Args compute: out_count -> indirect args.
        let args_entries = [
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        let args_bgl =
            render_device.create_bind_group_layout("c3d_sdf_mc_args_bgl", &args_entries);
        let args_sm = unsafe {
            render_device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("c3d_sdf_mc_args_wgsl"),
                source: wgpu::ShaderSource::Wgsl(SDF_MC_BATCH_ARGS_WGSL.into()),
            })
        };
        let args_pl = render_device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("c3d_sdf_mc_args_pl"),
            bind_group_layouts: &[&args_bgl],
            immediate_size: 0,
        });
        let args_ppl = render_device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_sdf_mc_args_ppl"),
            layout: Some(&args_pl),
            module: &args_sm,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Brush compute: in-place edits on chunk_pool (hot path).
        let brush_entries = [
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 3,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 4,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        let brush_bgl =
            render_device.create_bind_group_layout("c3d_sdf_brush_bgl", &brush_entries);
        let brush_sm = unsafe {
            render_device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("c3d_sdf_brush_wgsl"),
                source: wgpu::ShaderSource::Wgsl(SDF_BRUSH_WGSL.into()),
            })
        };
        let brush_pl = render_device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("c3d_sdf_brush_pl"),
            bind_group_layouts: &[&brush_bgl],
            immediate_size: 0,
        });
        let brush_ppl = render_device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c3d_sdf_brush_ppl"),
            layout: Some(&brush_pl),
            module: &brush_sm,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            shader,
            mesh_pipeline,
            uniform_layout_desc,
            mc_bgl,
            mc_ppl,
            args_bgl,
            args_ppl,
            brush_bgl,
            brush_ppl,
            edge_table,
            tri_table,
        }
    }
}

impl SpecializedMeshPipeline for SdfSurfacePipeline {
    type Key = MeshPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh_pipeline.specialize(key, layout)?;

        descriptor.vertex.shader = self.shader.clone();
        if let Some(f) = descriptor.fragment.as_mut() {
            f.shader = self.shader.clone();
        }

        if descriptor.depth_stencil.is_none() {
            descriptor.depth_stencil = Some(DepthStencilState {
                format: CORE_3D_DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: CompareFunction::GreaterEqual,
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            });
        }

        // Group 2: our SDF uniform (replaces mesh uniform usage).
        // Keep only the view layouts from the base mesh pipeline, then replace group 2 with our own.
        // If we leave extra layouts (e.g. material), wgpu validation will reject draws because we
        // never set those bind groups in `DrawSdfSurfacePipeline`.
        let view_layout_0 = descriptor
            .layout
            .get(0)
            .cloned()
            .unwrap_or_else(|| BindGroupLayoutDescriptor::new("sdf_surface_view0_missing", &[]));
        let view_layout_1 = descriptor
            .layout
            .get(1)
            .cloned()
            .unwrap_or_else(|| BindGroupLayoutDescriptor::new("sdf_surface_view1_missing", &[]));
        descriptor.layout.clear();
        descriptor.layout.push(view_layout_0);
        descriptor.layout.push(view_layout_1);
        descriptor.layout.push(self.uniform_layout_desc.clone());

        descriptor.vertex.buffers.clear();
        descriptor.vertex.buffers.push(VertexBufferLayout {
            array_stride: std::mem::size_of::<McVertex>() as u64,
            step_mode: VertexStepMode::Vertex,
            attributes: vec![
                VertexAttribute {
                    format: VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 0,
                },
                VertexAttribute {
                    format: VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 1,
                },
            ],
        });

        descriptor.primitive.topology = PrimitiveTopology::TriangleList;
        descriptor.primitive.cull_mode = None;
        Ok(descriptor)
    }
}

// ---------------- Render World: GPU cache (per surface entity) ----------------

#[derive(Clone)]
struct SdfSurfaceGpuEntry {
    handle_ptr: usize,
    iso_value: f32,
    invert: bool,

    // Pool (dense slots for active chunks in this surface).
    chunk_pool: Buffer,
    work_items: Buffer,
    slot_map: FxHashMap<IVec3, u32>,
    slot_work_items_cpu: Vec<SdfWorkItem>,
    cap_chunks: u32,
    active_chunks: u32,

    // Marching Cubes output.
    params: Buffer,
    out_verts: Buffer,
    out_count: Buffer,
    indirect_args: Buffer,
    cap_vertices: u32,

    mc_bg: BindGroup,
    args_bg: BindGroup,

    // Render uniform.
    uniform_buf: Buffer,
    render_bg: BindGroup,

    // Debug readback (optional).
    dbg_readback: Buffer,
    dbg_readback_armed: bool,

    // Brush (in-place edits).
    brush_params: Buffer,
    brush_work_items: Buffer,
    brush_dirty_slots: Buffer,
    brush_dirty_count: Buffer,
    brush_bg: BindGroup,

    dirty: bool,
}

#[derive(Resource, Default)]
pub struct SdfSurfaceGpuCache {
    entries: FxHashMap<Entity, SdfSurfaceGpuEntry>,
}

fn make_surface_entry(
    render_device: &RenderDevice,
    pipeline_cache: &PipelineCache,
    pipeline: &SdfSurfacePipeline,
    chunk_count: u32,
    cap_vertices: u32,
) -> SdfSurfaceGpuEntry {
    let chunk_count = chunk_count.max(1);
    let cap_vertices = cap_vertices.max(1);

    let chunk_pool = render_device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_sdf_surface_chunk_pool"),
        size: (chunk_count as u64) * (CHUNK_VOXELS as u64) * 4u64,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let work_items = render_device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_sdf_surface_work_items"),
        size: (chunk_count as u64) * (std::mem::size_of::<SdfWorkItem>() as u64),
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let params = render_device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_sdf_surface_mc_params"),
        size: std::mem::size_of::<McBatchParams>() as u64,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let out_verts = render_device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_sdf_surface_out_verts"),
        size: (cap_vertices as u64) * (std::mem::size_of::<McVertex>() as u64),
        usage: BufferUsages::STORAGE | BufferUsages::VERTEX,
        mapped_at_creation: false,
    });
    let out_count = render_device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_sdf_surface_out_count"),
        size: 4,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let indirect_args = render_device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_sdf_surface_indirect_args"),
        size: 16,
        usage: BufferUsages::STORAGE | BufferUsages::INDIRECT | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let dbg_readback = render_device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_sdf_surface_dbg_readback"),
        // [0..4): out_count u32, [16..32): indirect args
        size: 32,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mc_bg = render_device.create_bind_group(
        "c3d_sdf_surface_mc_bg",
        &pipeline.mc_bgl,
        &[
            BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: chunk_pool.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 2,
                resource: work_items.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 3,
                resource: out_verts.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 4,
                resource: out_count.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 5,
                resource: pipeline.edge_table.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 6,
                resource: pipeline.tri_table.as_entire_binding(),
            },
        ],
    );
    let args_bg = render_device.create_bind_group(
        "c3d_sdf_surface_args_bg",
        &pipeline.args_bgl,
        &[
            BindGroupEntry {
                binding: 0,
                resource: out_count.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: indirect_args.as_entire_binding(),
            },
        ],
    );

    let uniform_buf = render_device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_sdf_surface_uniform"),
        size: <SdfUniform as ShaderType>::min_size().get(),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    // IMPORTANT: Render pipelines are created via Bevy's `PipelineCache`, which caches and
    // reuses `BindGroupLayout` objects keyed by `BindGroupLayoutDescriptor`. A bind group must
    // be created with the *exact* `BindGroupLayout` object used by the pipeline layout.
    // Therefore, obtain the layout from the `PipelineCache` rather than creating a fresh one.
    let render_bgl = pipeline_cache.get_bind_group_layout(&pipeline.uniform_layout_desc);
    let render_bg = render_device.create_bind_group(
        "c3d_sdf_surface_render_bg",
        &render_bgl,
        &[BindGroupEntry {
            binding: 0,
            resource: uniform_buf.as_entire_binding(),
        }],
    );

    // Brush resources (cap at chunk_count; each workgroup handles one chunk).
    let brush_params = render_device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_sdf_surface_brush_params"),
        size: std::mem::size_of::<SdfBrushParams>() as u64,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let brush_work_items = render_device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_sdf_surface_brush_work_items"),
        size: (chunk_count as u64) * (std::mem::size_of::<SdfWorkItem>() as u64),
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let brush_dirty_slots = render_device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_sdf_surface_brush_dirty_slots"),
        size: (chunk_count as u64) * 4u64,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let brush_dirty_count = render_device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("c3d_sdf_surface_brush_dirty_count"),
        size: 4,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let brush_bg = render_device.create_bind_group(
        "c3d_sdf_surface_brush_bg",
        &pipeline.brush_bgl,
        &[
            BindGroupEntry {
                binding: 0,
                resource: brush_params.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: chunk_pool.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 2,
                resource: brush_work_items.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 3,
                resource: brush_dirty_slots.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 4,
                resource: brush_dirty_count.as_entire_binding(),
            },
        ],
    );

    SdfSurfaceGpuEntry {
        handle_ptr: 0,
        iso_value: 0.0,
        invert: false,
        chunk_pool,
        work_items,
        slot_map: FxHashMap::default(),
        slot_work_items_cpu: Vec::new(),
        cap_chunks: chunk_count,
        active_chunks: 0,
        params,
        out_verts,
        out_count,
        indirect_args,
        cap_vertices,
        mc_bg,
        args_bg,
        uniform_buf,
        render_bg,
        dbg_readback,
        dbg_readback_armed: true,
        brush_params,
        brush_work_items,
        brush_dirty_slots,
        brush_dirty_count,
        brush_bg,
        dirty: true,
    }
}

fn upload_full_grid(entry: &mut SdfSurfaceGpuEntry, render_queue: &RenderQueue, grid: &SdfGrid) {
    entry.slot_map.clear();
    entry.slot_work_items_cpu.clear();
    entry.active_chunks = grid.chunks.len().min(u32::MAX as usize) as u32;
    let chunk_count = entry.active_chunks;

    // Dense slots [0..chunk_count). Stable for this upload.
    let mut keys: Vec<IVec3> = grid.chunks.keys().copied().collect();
    keys.sort_by_key(|k| (k.x, k.y, k.z));

    for (i, ck) in keys.iter().enumerate() {
        entry.slot_map.insert(*ck, i as u32);
    }

    // Upload chunk data.
    let bytes_per_chunk: u64 = (CHUNK_VOXELS as u64) * 4u64;
    for (i, ck) in keys.iter().enumerate() {
        if let Some(ch) = grid.chunks.get(ck) {
            let off = (i as u64) * bytes_per_chunk;
            render_queue.write_buffer(&entry.chunk_pool, off, bytemuck::cast_slice(&ch.data));
        }
    }

    // Upload work items (neighbors from slot_map).
    let mut items: Vec<SdfWorkItem> = Vec::with_capacity(chunk_count as usize);
    for ck in keys.iter().take(chunk_count as usize) {
        let slot = entry.slot_map.get(ck).copied().unwrap_or(MISSING_SLOT);
        let mut wi = SdfWorkItem::default();
        wi.chunk_key = [ck.x, ck.y, ck.z, 0];
        wi.slot = slot;
        wi.neighbor_slots.fill(MISSING_SLOT);
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let nk = *ck + IVec3::new(dx, dy, dz);
                    let ns = entry.slot_map.get(&nk).copied().unwrap_or(MISSING_SLOT);
                    wi.neighbor_slots[neighbor_index(dx, dy, dz)] = ns;
                }
            }
        }
        items.push(wi);
    }
    render_queue.write_buffer(&entry.work_items, 0, bytemuck::cast_slice(&items));
    entry.slot_work_items_cpu = items;
}

// ---------------- Render World: render commands ----------------

pub struct SetSdfBindGroup<const I: usize>;
impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetSdfBindGroup<I> {
    type Param = ();
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: (),
        _item_query: Option<()>,
        _param: SystemParamItem<'w, '_, Self::Param>,
        _pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        RenderCommandResult::Success
    }
}

pub struct DrawSdfSurface;
impl<P: PhaseItem> RenderCommand<P> for DrawSdfSurface {
    type Param = SRes<SdfSurfaceGpuCache>;
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        item: &P,
        _view: (),
        _item_query: Option<()>,
        cache: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let e = item.entity();
        let cache = cache.into_inner();
        let Some(g) = cache.entries.get(&e) else {
            return RenderCommandResult::Failure("missing sdf gpu entry");
        };
        pass.set_bind_group(2, &g.render_bg, &[]);
        pass.set_vertex_buffer(0, g.out_verts.slice(..));
        pass.draw_indirect(&g.indirect_args, 0);
        RenderCommandResult::Success
    }
}

pub type DrawSdfSurfacePipeline = (
    SetItemPipeline,
    bevy::pbr::SetMeshViewBindGroup<0>,
    bevy::pbr::SetMeshViewBindingArrayBindGroup<1>,
    SetSdfBindGroup<2>,
    DrawSdfSurface,
);

// ---------------- Render World: queue (compute + enqueue draw items) ----------------

#[derive(SystemParam)]
pub struct QueueSdfSurfaceParams<'w, 's> {
    opaque_3d_draw_functions: Res<'w, DrawFunctions<Opaque3d>>,
    pipeline: Res<'w, SdfSurfacePipeline>,
    pipelines: ResMut<'w, SpecializedMeshPipelines<SdfSurfacePipeline>>,
    pipeline_cache: Res<'w, PipelineCache>,
    render_device: Res<'w, RenderDevice>,
    render_queue: Res<'w, RenderQueue>,
    strokes: Res<'w, SdfBrushStrokeQueueShared>,
    mesh_allocator: Res<'w, MeshAllocator>,
    render_meshes: Res<'w, RenderAssets<RenderMesh>>,
    render_mesh_instances: Res<'w, RenderMeshInstances>,
    _gpu_preprocessing_support: Res<'w, GpuPreprocessingSupport>,
    opaque_render_phases: ResMut<'w, ViewBinnedRenderPhases<Opaque3d>>,
    ticks: bevy::ecs::system::SystemChangeTick,
    views: Query<
        'w,
        's,
        (
            &'static ExtractedView,
            &'static Msaa,
            Option<&'static ViewPrepassTextures>,
            Has<OrderIndependentTransparencySettings>,
            Has<ExtractedAtmosphere>,
            &'static RenderVisibleEntities,
        ),
    >,
    surfaces: Query<'w, 's, (Entity, &'static RenderSdfSurface)>,
    cache: ResMut<'w, SdfSurfaceGpuCache>,
    extract_stats: Option<Res<'w, ExtractedSdfSurfaceStats>>,
    dbg: Local<'s, SdfSurfaceQueueDebug>,
}

pub fn queue_sdf_surfaces(mut p: QueueSdfSurfaceParams) {
    puffin::profile_function!();
    let QueueSdfSurfaceParams {
        opaque_3d_draw_functions,
        pipeline,
        mut pipelines,
        pipeline_cache,
        render_device,
        render_queue,
        strokes,
        mesh_allocator,
        render_meshes,
        render_mesh_instances,
        _gpu_preprocessing_support,
        mut opaque_render_phases,
        ticks,
        views,
        surfaces,
        mut cache,
        extract_stats,
        mut dbg,
    } = p;

    let dbg_enabled = dbg.enabled();
    let dbg_readback = std::env::var_os("C3D_SDF_DEBUG_READBACK").is_some();
    dbg.frame = dbg.frame.wrapping_add(1);
    let mut pending_readbacks: Vec<(usize, Buffer)> = Vec::new();

    // Drain strokes from the shared queue (Main World -> Render World bridge).
    let mut drained: Vec<SdfBrushStroke> = Vec::new();
    if let Ok(mut q) = strokes.0.try_lock() {
        drained.append(&mut *q);
    }

    // Prune dead entries.
    let mut alive: rustc_hash::FxHashSet<Entity> = rustc_hash::FxHashSet::default();
    for (e, _) in &surfaces {
        alive.insert(e);
    }
    cache.entries.retain(|e, _| alive.contains(e));

    // Upload/compute (only when inputs changed). Batched into one submit for the whole frame.
    let mut did_any_compute = false;
    let mut enc = render_device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("c3d_sdf_surface_compute_enc"),
    });

    let mut surfaces_n = 0usize;
    for (e, s) in &surfaces {
        surfaces_n += 1;
        let ptr = Arc::as_ptr(&s.handle.grid) as usize;
        let iso = s.iso_value;
        let inv = s.invert;
        let surface_strokes: Vec<SdfBrushStroke> = drained
            .iter()
            .copied()
            .filter(|st| st.target_ptr == ptr)
            .collect();

        let entry = cache.entries.entry(e).or_insert_with(|| {
            // Conservative defaults; resized on upload.
            make_surface_entry(&render_device, &pipeline_cache, &pipeline, 64, 1_000_000)
        });

        let mut need_upload = false;
        let mut need_compute = false;
        let need_brush = !surface_strokes.is_empty();

        if entry.handle_ptr != ptr {
            entry.handle_ptr = ptr;
            need_upload = true;
            need_compute = true;
        }
        if (entry.iso_value - iso).abs() > f32::EPSILON || entry.invert != inv {
            entry.iso_value = iso;
            entry.invert = inv;
            need_compute = true;
        }
        if entry.dirty {
            need_compute = true;
        }

        if need_upload {
            if let Ok(grid) = s.handle.grid.read() {
                if dbg_enabled && dbg.should_print() {
                    eprintln!(
                        "[c3d][sdf_surface] upload chunks={} iso={} inv={}",
                        grid.chunks.len(),
                        iso,
                        inv
                    );
                }
                let need_chunks = grid.chunks.len().min(u32::MAX as usize) as u32;
                if need_chunks > entry.cap_chunks {
                    let cap_vertices = entry.cap_vertices.max(1_000_000);
                    *entry = make_surface_entry(
                        &render_device,
                        &pipeline_cache,
                        &pipeline,
                        need_chunks.max(1),
                        cap_vertices,
                    );
                    entry.handle_ptr = ptr;
                    entry.iso_value = iso;
                    entry.invert = inv;
                }
                upload_full_grid(entry, &render_queue, &grid);
            }
        }

        // Update render uniform every frame (cheap).
        let mut shading_mode = s.shading_mode;
        if let Ok(v) = std::env::var("C3D_SDF_SHADE") {
            let v = v.to_ascii_lowercase();
            if v == "pbr" || v == "0" {
                shading_mode = 0;
            } else if v == "clay" || v == "matcap" || v == "1" {
                shading_mode = 1;
            }
        }
        let u = SdfUniform {
            model: s.handle.transform,
            color: s.color,
            params0: Vec4::new(
                shading_mode as f32,
                s.roughness.clamp(0.0, 1.0),
                s.rim_strength.clamp(0.0, 1.0),
                s.cavity_strength.clamp(0.0, 1.0),
            ),
            params1: Vec4::new(s.rim_power.max(0.0), s.clay_spec.max(0.0), 0.0, 0.0),
        };
        render_queue
            .write_buffer(&entry.uniform_buf, 0, bytemuck::bytes_of(&u));

        // Apply brush strokes (in-place updates to `chunk_pool`) before Marching Cubes.
        if need_brush {
            let Ok(grid) = s.handle.grid.read() else { continue; };
            // Ensure CPU-side work-item cache exists (needed for subset selection).
            if entry.slot_work_items_cpu.is_empty() && !grid.chunks.is_empty() {
                upload_full_grid(entry, &render_queue, &grid);
            }

            // Precompute transform (world -> local SDF space).
            let inv_model = s.handle.transform.inverse();
            let scale = approx_uniform_scale(&s.handle.transform).max(0.00001);

            let mut did_apply_brush = false;
            for st in &surface_strokes {
                let a_local = inv_model.transform_point3(st.a_world);
                let b_local = inv_model.transform_point3(st.b_world);
                let radius_local = (st.radius_world / scale).max(0.0);
                let smooth_k_local = (st.smooth_k_world / scale).max(0.0);

                let vs = grid.voxel_size.max(0.00001);
                let r_vox = radius_local / vs;
                let a_vox = a_local / vs;
                let b_vox = b_local / vs;
                let min_vox_f = Vec3::new(
                    a_vox.x.min(b_vox.x),
                    a_vox.y.min(b_vox.y),
                    a_vox.z.min(b_vox.z),
                ) - Vec3::splat(r_vox);
                let max_vox_f = Vec3::new(
                    a_vox.x.max(b_vox.x),
                    a_vox.y.max(b_vox.y),
                    a_vox.z.max(b_vox.z),
                ) + Vec3::splat(r_vox);
                let min_vox = min_vox_f.floor().as_ivec3();
                let max_vox = max_vox_f.floor().as_ivec3();
                let min_ck = IVec3::new(
                    min_vox.x.div_euclid(16),
                    min_vox.y.div_euclid(16),
                    min_vox.z.div_euclid(16),
                );
                let max_ck = IVec3::new(
                    max_vox.x.div_euclid(16),
                    max_vox.y.div_euclid(16),
                    max_vox.z.div_euclid(16),
                );

                let mut brush_items: Vec<SdfWorkItem> = Vec::new();
                for z in min_ck.z..=max_ck.z {
                    for y in min_ck.y..=max_ck.y {
                        for x in min_ck.x..=max_ck.x {
                            let ck = IVec3::new(x, y, z);
                            let Some(&slot) = entry.slot_map.get(&ck) else { continue; };
                            let Some(wi) = entry.slot_work_items_cpu.get(slot as usize).copied()
                            else {
                                continue;
                            };
                            brush_items.push(wi);
                        }
                    }
                }
                if brush_items.is_empty() {
                    continue;
                }

                let wi = brush_items.len().min(u32::MAX as usize) as u32;
                render_queue.write_buffer(
                    &entry.brush_work_items,
                    0,
                    bytemuck::cast_slice(&brush_items),
                );
                render_queue
                    .write_buffer(&entry.brush_dirty_count, 0, bytemuck::bytes_of(&0u32));

                let p = SdfBrushParams {
                    a_local: [a_local.x, a_local.y, a_local.z, 0.0],
                    b_local: [b_local.x, b_local.y, b_local.z, 0.0],
                    radius_local,
                    voxel_size: vs,
                    smooth_k: smooth_k_local,
                    mode: st.mode,
                    work_items_count: wi,
                    max_dirty: wi.min(entry.cap_chunks.max(1)),
                    _pad0: [0, 0],
                };
                render_queue.write_buffer(&entry.brush_params, 0, bytemuck::bytes_of(&p));

                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("c3d_sdf_surface_brush_pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline.brush_ppl);
                pass.set_bind_group(0, &entry.brush_bg, &[]);
                pass.dispatch_workgroups(wi.max(1), 1, 1);
                did_any_compute = true;
                did_apply_brush = true;
            }
            if did_apply_brush {
                entry.dirty = true;
                need_compute = true;
            }
        }

        if need_compute {
            let Ok(grid) = s.handle.grid.read() else { continue; };
            let wi = entry.active_chunks;
            let dispatch = wi.max(1);
            let max_vertices = entry.cap_vertices.max(1);
            if dbg_enabled && dbg.should_print() {
                eprintln!(
                    "[c3d][sdf_surface] mc wi={} dispatch={} max_verts={} voxel_size={} bg={} iso={} inv={}",
                    wi,
                    dispatch,
                    max_vertices,
                    grid.voxel_size,
                    grid.background_value,
                    iso,
                    inv
                );
            }
            let params = McBatchParams {
                voxel_size: grid.voxel_size.max(0.00001),
                iso_value: iso,
                background_value: grid.background_value,
                invert: if inv { 1 } else { 0 },
                work_items_count: wi,
                max_vertices,
                _pad0: [0, 0],
            };
            render_queue
                .write_buffer(&entry.params, 0, bytemuck::bytes_of(&params));
            render_queue
                .write_buffer(&entry.out_count, 0, bytemuck::bytes_of(&0u32));

            {
                if wi != 0 {
                    let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("c3d_sdf_surface_mc_pass"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&pipeline.mc_ppl);
                    pass.set_bind_group(0, &entry.mc_bg, &[]);
                    pass.dispatch_workgroups(dispatch, 1, 1);
                }
            }
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("c3d_sdf_surface_args_pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline.args_ppl);
                pass.set_bind_group(0, &entry.args_bg, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }

            if dbg_readback && entry.dbg_readback_armed {
                // Copy small debug outputs for terminal logging without requiring GPU debuggers.
                enc.copy_buffer_to_buffer(&entry.out_count, 0, &entry.dbg_readback, 0, 4);
                enc.copy_buffer_to_buffer(&entry.indirect_args, 0, &entry.dbg_readback, 16, 16);
                pending_readbacks.push((ptr, entry.dbg_readback.clone()));
                entry.dbg_readback_armed = false;
            }

            entry.dirty = false;
            did_any_compute = true;
        }
    }

    if did_any_compute {
        render_queue.submit(std::iter::once(enc.finish()));
    }
    if dbg_readback && !pending_readbacks.is_empty() {
        use std::sync::mpsc;
        use std::time::Duration;

        // Synchronous readback (debug-only): guarantees terminal output without depending on
        // ongoing polling from subsequent frames.
        for (ptr, rb) in pending_readbacks.drain(..) {
            let slice = rb.slice(..);
            let (tx, rx) = mpsc::channel::<Result<(), wgpu::BufferAsyncError>>();
            slice.map_async(wgpu::MapMode::Read, move |res| {
                let _ = tx.send(res);
            });
            let _ = render_device.wgpu_device().poll(wgpu::PollType::wait_indefinitely());
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(Ok(())) => {
                    let mapped = rb.slice(..).get_mapped_range();
                    let cnt = u32::from_le_bytes([mapped[0], mapped[1], mapped[2], mapped[3]]);
                    let vc = u32::from_le_bytes([mapped[16], mapped[17], mapped[18], mapped[19]]);
                    let ic = u32::from_le_bytes([mapped[20], mapped[21], mapped[22], mapped[23]]);
                    eprintln!(
                        "[c3d][sdf_surface] readback ptr={} out_count={} indirect(vertex_count={}, instance_count={})",
                        ptr,
                        cnt,
                        vc,
                        ic
                    );
                    drop(mapped);
                    rb.unmap();
                    if std::env::var_os("C3D_SDF_SMOKE_QUIT").is_some() {
                        std::process::exit(0);
                    }
                }
                Ok(Err(_)) => {
                    eprintln!("[c3d][sdf_surface] readback ptr={} map_async failed", ptr);
                    rb.unmap();
                }
                Err(_) => {
                    eprintln!("[c3d][sdf_surface] readback ptr={} timed out", ptr);
                    rb.unmap();
                }
            }
        }
    }

    // Queue render items per view.
    let draw_function = opaque_3d_draw_functions
        .read()
        .get_id::<DrawSdfSurfacePipeline>()
        .unwrap();

    let mut phase_add = 0usize;
    for (view, msaa, prepass_textures, has_oit, has_atmosphere, visible) in &views {
        let mut view_key =
            MeshPipelineKey::from_msaa_samples(msaa.samples()) | MeshPipelineKey::from_hdr(view.hdr);
        if has_oit {
            view_key |= MeshPipelineKey::OIT_ENABLED;
        }
        if has_atmosphere {
            view_key |= MeshPipelineKey::ATMOSPHERE;
        }
        if let Some(pt) = prepass_textures {
            if pt.depth.is_some() {
                view_key |= MeshPipelineKey::DEPTH_PREPASS;
            }
            if pt.normal.is_some() {
                view_key |= MeshPipelineKey::NORMAL_PREPASS;
            }
            if pt.motion_vectors.is_some() {
                view_key |= MeshPipelineKey::MOTION_VECTOR_PREPASS;
            }
            if pt.deferred.is_some() {
                view_key |= MeshPipelineKey::DEFERRED_PREPASS;
            }
        }

        let Some(phase) = opaque_render_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };

        for (render_entity, visible_entity) in visible.iter::<Mesh3d>() {
            let Ok((_e, s)) = surfaces.get(*render_entity) else { continue; };

            let mut mesh_asset_id = s.mesh_handle.id();

            let mut current_uniform_index = InputUniformIndex::default();
            let mut lightmap_slab = None;
            if let Some(mi) = render_mesh_instances.render_mesh_queue_data(*visible_entity) {
                current_uniform_index = mi.current_uniform_index;
                lightmap_slab = mi.shared.lightmap_slab_index.map(|i| *i);
                mesh_asset_id = mi.shared.mesh_asset_id;
            }

            let Some(mesh) = render_meshes.get(mesh_asset_id) else { continue; };
            let (vertex_slab, index_slab) = mesh_allocator.mesh_slabs(&mesh_asset_id);

            let key = view_key
                | MeshPipelineKey::from_primitive_topology(PrimitiveTopology::TriangleList);
            let Ok(pipeline_id) =
                pipelines.specialize(&pipeline_cache, &pipeline, key, &mesh.layout)
            else {
                continue;
            };

            let batch_set_key = bevy::core_pipeline::core_3d::Opaque3dBatchSetKey {
                pipeline: pipeline_id,
                draw_function,
                material_bind_group_index: None,
                vertex_slab: vertex_slab.unwrap_or_default(),
                index_slab,
                lightmap_slab,
            };
            let bin_key = bevy::core_pipeline::core_3d::Opaque3dBinKey {
                asset_id: mesh_asset_id.into(),
            };
            phase.add(
                batch_set_key,
                bin_key,
                (*render_entity, *visible_entity),
                current_uniform_index,
                BinnedRenderPhaseType::UnbatchableMesh,
                ticks.this_run(),
            );
            phase_add += 1;
        }
    }
    if dbg_enabled && dbg.should_print() {
        if let Some(s) = extract_stats.as_deref() {
            eprintln!(
                "[c3d][sdf_surface] extract_stats frame={} seen={} inserted={}",
                s.frame, s.seen, s.inserted
            );
        } else {
            eprintln!("[c3d][sdf_surface] extract_stats <missing>");
        }
        eprintln!(
            "[c3d][sdf_surface] queue surfaces={} cache_entries={} did_compute={} phase_add={}",
            surfaces_n,
            cache.entries.len(),
            did_any_compute,
            phase_add
        );
    }
}

#[derive(Default)]
struct SdfSurfaceExtractDebug {
    frame: u64,
}

impl SdfSurfaceExtractDebug {
    fn enabled(&self) -> bool {
        std::env::var_os("C3D_SDF_DEBUG").is_some()
    }
    fn should_print(&self) -> bool {
        // throttle (every ~60 frames)
        (self.frame % 60) == 1
    }
}

#[derive(Default)]
struct SdfSurfaceQueueDebug {
    frame: u64,
}

impl SdfSurfaceQueueDebug {
    fn enabled(&self) -> bool {
        std::env::var_os("C3D_SDF_DEBUG").is_some()
    }
    fn should_print(&self) -> bool {
        (self.frame % 60) == 1
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SdfBrushParams {
    a_local: [f32; 4],
    b_local: [f32; 4],
    radius_local: f32,
    voxel_size: f32,
    smooth_k: f32,
    /// 0 = union/add, 1 = subtract, 2 = intersect
    mode: u32,
    work_items_count: u32,
    max_dirty: u32,
    _pad0: [u32; 2],
}

#[inline]
fn approx_uniform_scale(m: &Mat4) -> f32 {
    let x = Vec3::new(m.x_axis.x, m.x_axis.y, m.x_axis.z).length();
    let y = Vec3::new(m.y_axis.x, m.y_axis.y, m.y_axis.z).length();
    let z = Vec3::new(m.z_axis.x, m.z_axis.y, m.z_axis.z).length();
    (x + y + z) * (1.0 / 3.0)
}

const SDF_BRUSH_WGSL: &str = r#"
const CHUNK_SIZE: u32 = 16u;
const CHUNK_VOXELS: u32 = 4096u;
const MISSING_SLOT: u32 = 0xffffffffu;

struct Params {
  a_local: vec4<f32>,
  b_local: vec4<f32>,
  radius_local: f32,
  voxel_size: f32,
  smooth_k: f32,
  mode: u32,
  work_items_count: u32,
  max_dirty: u32,
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

fn sd_capsule(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>, r: f32) -> f32 {
  let pa = p - a;
  let ba = b - a;
  let denom = max(dot(ba, ba), 1e-8);
  let h = clamp(dot(pa, ba) / denom, 0.0, 1.0);
  return length(pa - ba * h) - r;
}

@compute @workgroup_size(256)
fn main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
  let wi = wg.x;
  if (wi >= p.work_items_count) { return; }

  let item = work_items[wi];
  let slot = item.slot;
  if (slot == MISSING_SLOT) { return; }

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
    let local_pos = (vec3<f32>(f32(gp.x) + 0.5, f32(gp.y) + 0.5, f32(gp.z) + 0.5)) * p.voxel_size;

    let old_v = chunk_pool[base + i];
    let b = sd_capsule(local_pos, p.a_local.xyz, p.b_local.xyz, p.radius_local);

    var new_v = old_v;
    if (p.mode == 0u) {
      new_v = smin_poly(old_v, b, p.smooth_k);
    } else if (p.mode == 1u) {
      new_v = smax_poly(old_v, -b, p.smooth_k);
    } else if (p.mode == 2u) {
      new_v = smax_poly(old_v, b, p.smooth_k);
    }

    chunk_pool[base + i] = new_v;

    i = i + 256u;
  }

  // Mark the whole chunk as dirty once. (Any brush stroke can move the iso-surface even without sign flips.)
  if (lid.x == 0u) {
    let out_i = atomicAdd(&dirty_count, 1u);
    if (out_i < p.max_dirty) {
      dirty_slots[out_i] = slot;
    }
  }
}
"#;
