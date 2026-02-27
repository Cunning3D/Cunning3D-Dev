//! GPU voxel face renderer (extreme path).
//!
//! - CPU keeps voxel data in `VoxelEditCookCache` (node graph cook side).
//! - RenderApp pulls *dirty padded chunks* and uploads to RenderDevice.
//! - Compute shader generates face instances (one quad per exposed voxel face).
//! - Render shader draws a unit quad instanced by face buffer.
//!
//! This avoids CPU greedy meshing + CPU triangle mesh generation on every edit.

use bevy::core_pipeline::{
    oit::OrderIndependentTransparencySettings, prepass::ViewPrepassTextures,
};
use bevy::pbr::ExtractedAtmosphere;
use bevy::camera::primitives::{Frustum, Sphere};
use bevy::asset::RenderAssetUsages;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::system::SystemParam;
// NOTE: fully GPU-driven path (no CPU greedy fallback).
use bevy::{
    core_pipeline::core_3d::{Opaque3d, CORE_3D_DEPTH_FORMAT},
    ecs::system::{lifetimeless::SRes, SystemParamItem},
    mesh::{Indices, Mesh, Mesh3d, MeshVertexBufferLayoutRef, VertexBufferLayout, VertexFormat},
    pbr::{MeshPipeline, MeshPipelineKey, RenderMeshInstances, SetMeshViewBindGroup, SetMeshViewBindingArrayBindGroup},
    prelude::*,
    render::{
        batching::gpu_preprocessing::GpuPreprocessingSupport,
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::RenderAssets,
        render_phase::{
            AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, PhaseItem, RenderCommand,
            InputUniformIndex, RenderCommandResult, SetItemPipeline, TrackedRenderPass,
            ViewBinnedRenderPhases,
        },
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        sync_world::RenderEntity,
        view::{ExtractedView, RenderVisibleEntities},
        Extract, Render, RenderApp, RenderSystems,
    },
};
use bytemuck::{Pod, Zeroable};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use cunning_kernel::nodes::voxel::voxel_edit::{voxel_render_all_chunk_keys, voxel_render_chunks_gen, voxel_render_take_dirty};

pub struct CunningVoxelFacesPlugin;

const VOXEL_LOD_LEVELS: u8 = 4; // 0..3

#[inline]
fn lod_dim(base: u32, lod: u8) -> u32 {
    base >> lod.min(3)
}

#[derive(Resource, Clone)]
pub struct VoxelFacesShader(pub Handle<Shader>);

/// Per-frame stats for the voxel preview render path.
#[derive(Resource, Default, Clone, Debug)]
pub struct VoxelFacesStats {
    pub frame: u64,
    pub visible_chunks: u32,
    pub visible_nodes: u32,
    pub dirty_chunks_uploaded: u32,
    pub atlas_upload_bytes: u64,
    pub neighbor_upload_bytes: u64,
    pub palette_upload_bytes: u64,
    pub estimated_face_instances: u64,
    pub compute_dispatches_faces: u32,
    pub compute_dispatches_args: u32,
    pub submits: u32,
    pub draws: u32,
}

/// Shared stats bridge (RenderApp -> Main World UI).
#[derive(Resource, Clone)]
pub struct VoxelFacesStatsShared(pub Arc<Mutex<VoxelFacesStats>>);
impl Default for VoxelFacesStatsShared {
    fn default() -> Self { Self(Arc::new(Mutex::new(VoxelFacesStats::default()))) }
}

/// Debug config for `VoxelFacesStats`.
#[derive(Resource, Clone, Copy, Debug)]
pub struct VoxelFacesStatsConfig {
    /// Log stats every N frames (0 disables).
    pub log_every_frames: u64,
}

impl Default for VoxelFacesStatsConfig {
    fn default() -> Self {
        Self { log_every_frames: 0 }
    }
}

/// Distance-based LOD thresholds for voxel preview (world units, meters).
#[derive(Resource, Clone, Copy, Debug)]
pub struct VoxelLodConfig {
    /// Distances at which we switch to LOD1/2/3.
    pub thresholds: [f32; 3],
}

impl Default for VoxelLodConfig {
    fn default() -> Self {
        // LOD0 within 64m for reliable voxel editing; then switch every 64m.
        Self { thresholds: [64.0, 128.0, 192.0] }
    }
}

/// Greedy backend selection (near-set).
#[derive(Resource, Clone, Copy, Debug)]
pub struct VoxelGreedyBackendConfig;

impl Default for VoxelGreedyBackendConfig { fn default() -> Self { Self } }

impl Plugin for CunningVoxelFacesPlugin {
    fn build(&self, app: &mut App) {
        // Main-world shaders
        let render_h = {
            let mut shaders = app.world_mut().resource_mut::<Assets<Shader>>();
            let render_h = shaders.add(Shader::from_wgsl(
                include_str!("../../assets/shaders/cunning_voxel_faces.wgsl"),
                "shaders/cunning_voxel_faces.wgsl",
            ));
            render_h
        };
        app.insert_resource(VoxelFacesShader(render_h));

        app.init_resource::<VoxelFacesQuadMesh>();
        app.init_resource::<VoxelPreviewRoot>();
        app.init_resource::<VoxelFacesStatsShared>();
        app.add_systems(PostUpdate, sync_voxel_preview_entities_from_root_system);
    }

    fn finish(&self, app: &mut App) {
        let render_h = app.world().resource::<VoxelFacesShader>().0.clone();
        let stats_shared = app.world().resource::<VoxelFacesStatsShared>().clone();

        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.insert_resource(VoxelFacesShader(render_h));
            render_app.insert_resource(stats_shared);

            render_app
                .init_resource::<VoxelFacesPipeline>()
                .init_resource::<SpecializedMeshPipelines<VoxelFacesPipeline>>()
                .init_resource::<VoxelFacesUniforms>()
                .init_resource::<VoxelGpuChunks>()
                .init_resource::<VoxelFacesStats>()
                .init_resource::<VoxelFacesStatsConfig>()
                .init_resource::<VoxelLodConfig>()
                .init_resource::<VoxelGreedyBackendConfig>()
                .add_render_command::<Opaque3d, DrawVoxelFacesPipeline>()
                .add_systems(ExtractSchedule, extract_voxel_chunks)
                .add_systems(Render, prepare_voxel_uniforms.in_set(RenderSystems::PrepareBindGroups))
                .add_systems(Render, queue_voxel_chunks);
        }
    }
}

// ---------------- Main World: marker + chunk entity syncing ----------------

/// Root marker (optional) for voxel chunk entities.
#[derive(Component)]
pub struct VoxelRenderRootTag;

/// Marker on chunk entities to be GPU-rendered.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoxelRenderChunk {
    pub node_id: Uuid,
    pub chunk: IVec3,
}

/// Shared quad mesh handle used by voxel chunks.
#[derive(Resource, Clone)]
pub struct VoxelFacesQuadMesh(pub Handle<Mesh>);

impl FromWorld for VoxelFacesQuadMesh {
    fn from_world(world: &mut World) -> Self {
        let mut meshes = world.resource_mut::<Assets<Mesh>>();
        // Unit quad in XY plane, [0..1]^2, z=0
        let mut m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        m.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [0.0f32, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
        );
        m.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            vec![[0.0f32, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        );
        m.insert_indices(Indices::U16(vec![0, 1, 2, 2, 3, 0]));
        Self(meshes.add(m))
    }
}

#[derive(Resource, Default)]
pub struct VoxelPreviewRoot {
    pub node_id: Option<Uuid>,
    pub last_node_id: Option<Uuid>,
    pub last_chunks_gen: u64,
    pub voxel_size: f32,
    pub missing_frames: u8,
    pub chunks: HashMap<IVec3, Entity>,
    pub free_chunks: Vec<Entity>,
}

fn sync_voxel_preview_entities_from_root_system(
    mut commands: Commands,
    quad: Res<VoxelFacesQuadMesh>,
    mut root_state: ResMut<VoxelPreviewRoot>,
) {
    let Some(node_id) = root_state.node_id else {
        root_state.missing_frames = root_state.missing_frames.saturating_add(1);
        const MISSING_LIMIT: u8 = 5;
        if root_state.missing_frames < MISSING_LIMIT {
            return;
        }
        let drained: Vec<Entity> = root_state.chunks.drain().map(|(_, e)| e).collect();
        for e in drained {
            commands.entity(e).remove::<VoxelRenderChunk>().insert(Visibility::Hidden);
            root_state.free_chunks.push(e);
        }
        root_state.last_node_id = None;
        root_state.voxel_size = 0.0;
        root_state.missing_frames = 0;
        return;
    };
    root_state.missing_frames = 0;
    if root_state.last_node_id != Some(node_id) {
        let drained: Vec<Entity> = root_state.chunks.drain().map(|(_, e)| e).collect();
        for e in drained {
            commands.entity(e).remove::<VoxelRenderChunk>().insert(Visibility::Hidden);
            root_state.free_chunks.push(e);
        }
        root_state.last_node_id = Some(node_id);
        root_state.last_chunks_gen = 0;
    }
    let voxel_size = root_state.voxel_size.max(0.001);
    let gen = voxel_render_chunks_gen(node_id);
    if gen == root_state.last_chunks_gen {
        return;
    }
    root_state.last_chunks_gen = gen;

    // Diff-sync chunk entities from CPU cache (avoid full-scene blink).
    let keys = voxel_render_all_chunk_keys(node_id);
    let desired: HashSet<IVec3> = keys.iter().copied().collect();

    // Despawn removed chunks.
    let mut removed: Vec<IVec3> = Vec::new();
    for ck in root_state.chunks.keys().copied() {
        if !desired.contains(&ck) {
            removed.push(ck);
        }
    }
    for ck in removed {
        if let Some(e) = root_state.chunks.remove(&ck) {
            commands.entity(e).remove::<VoxelRenderChunk>().insert(Visibility::Hidden);
            root_state.free_chunks.push(e);
        }
    }

    // Spawn added chunks.
    for ck in keys {
        let base_ws = ck.as_vec3() * (cunning_kernel::geometry::voxel::CHUNK_SIZE as f32) * voxel_size;
        if root_state.chunks.contains_key(&ck) {
            continue;
        }
        let e = if let Some(e) = root_state.free_chunks.pop() {
            commands.entity(e).insert((
                Name::new(format!("SdfChunk {:?}", ck)),
                VoxelRenderChunk { node_id, chunk: ck },
                Mesh3d(quad.0.clone()),
                Transform::from_translation(base_ws),
                Visibility::Visible,
                bevy::camera::visibility::InheritedVisibility::default(),
                bevy::camera::visibility::ViewVisibility::default(),
                bevy::camera::visibility::NoFrustumCulling,
                bevy::render::sync_world::SyncToRenderWorld,
            ));
            e
        } else {
            commands
                .spawn((
                    Name::new(format!("SdfChunk {:?}", ck)),
                    VoxelRenderChunk { node_id, chunk: ck },
                    Mesh3d(quad.0.clone()),
                    Transform::from_translation(base_ws),
                    GlobalTransform::default(),
                    Visibility::Visible,
                    bevy::camera::visibility::InheritedVisibility::default(),
                    bevy::camera::visibility::ViewVisibility::default(),
                    bevy::camera::visibility::NoFrustumCulling,
                    bevy::render::sync_world::SyncToRenderWorld,
                ))
                .id()
        };
        root_state.chunks.insert(ck, e);
    }
}

// ---------------- Render World: extraction ----------------

#[derive(Component)]
pub struct RenderSdfChunk {
    pub node_id: Uuid,
    pub chunk: IVec3,
    pub mesh_handle: Handle<Mesh>,
    pub transform: GlobalTransform,
}

#[derive(Resource, Clone, Copy)]
pub struct ExtractedVoxelVizOptions {
    pub wire_px: f32,
    pub display_mode: u32,
    pub ghost: bool,
}

/// Per-node dirty snapshot passed through extraction.
#[derive(Resource, Default)]
pub struct ExtractedVoxelDirty {
    pub per_node: HashMap<Uuid, VoxelDirtySnapshot>,
}

#[derive(Clone)]
pub struct VoxelDirtySnapshot {
    pub voxel_size: f32,
    pub palette_rgba: Option<Vec<[f32; 4]>>,
    pub dirty_raw_chunks: Vec<(IVec3, Vec<u32>, u32)>,
    pub all_chunk_keys: Vec<IVec3>,
}

pub fn extract_voxel_chunks(
    mut commands: Commands,
    query: Extract<Query<(RenderEntity, &VoxelRenderChunk, &Mesh3d, &ViewVisibility, &GlobalTransform)>>,
    quad: Extract<Option<Res<VoxelFacesQuadMesh>>>,
    display_options: Extract<Option<Res<cunning_viewport::viewport_options::DisplayOptions>>>,
) {
    puffin::profile_function!();
    let quad_mesh = quad.as_ref().map(|q| q.0.clone());
    let mut nodes: HashSet<Uuid> = HashSet::new();
    for (re, ch, mesh, vis, tr) in &query {
        if !vis.get() {
            continue;
        }
        nodes.insert(ch.node_id);
        commands.entity(re).insert(RenderSdfChunk {
            node_id: ch.node_id,
            chunk: ch.chunk,
            mesh_handle: quad_mesh.clone().unwrap_or_else(|| mesh.0.clone()),
            transform: *tr,
        });
    }

    let (wire_px, display_mode, ghost) = display_options
        .as_ref()
        .map(|o| {
            let dm = match o.final_geometry_display_mode {
                cunning_viewport::viewport_options::DisplayMode::Shaded => 0u32,
                cunning_viewport::viewport_options::DisplayMode::Wireframe => 1u32,
                cunning_viewport::viewport_options::DisplayMode::ShadedAndWireframe => 2u32,
            };
            (o.overlays.voxel_grid_line_px, dm, o.wireframe_ghost_mode)
        })
        .unwrap_or((0.1, 2, false));
    commands.insert_resource(ExtractedVoxelVizOptions {
        wire_px: wire_px.max(0.0),
        display_mode,
        ghost,
    });

    let mut per_node = HashMap::new();
    for id in nodes {
        if let Some(d) = voxel_render_take_dirty(id) {
            per_node.insert(
                id,
                VoxelDirtySnapshot {
                    voxel_size: d.voxel_size,
                    palette_rgba: d.palette_rgba,
                    dirty_raw_chunks: d.dirty_raw_chunks,
                    all_chunk_keys: d.all_chunk_keys,
                },
            );
        }
    }
    commands.insert_resource(ExtractedVoxelDirty { per_node });
}

// ---------------- Render World: pipeline ----------------

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, ShaderType)]
pub struct VoxelUniform {
    pub model: Mat4,
    /// xyz = chunk base world, w = voxel_size
    pub chunk_base_world: Vec4,
    pub wire_params: Vec4,
}

#[derive(Resource, Default)]
pub struct VoxelFacesUniforms {
    pub buffer: DynamicUniformBuffer<VoxelUniform>,
    pub bind_groups: HashMap<Uuid, BindGroup>, // per node palette + uniform buffer
    pub uniform_buffer_id: Option<BufferId>,
}

#[derive(Component)]
pub struct VoxelUniformOffset(pub u32);

#[derive(Resource)]
pub struct VoxelFacesPipeline {
    pub shader: Handle<Shader>,
    pub mesh_pipeline: MeshPipeline,
    pub uniform_layout: BindGroupLayout,
    pub uniform_layout_desc: BindGroupLayoutDescriptor,
    pub compute_bgl: BindGroupLayout,
    pub compute_ppl_faces: ComputePipeline,
    pub compute_ppl_greedy: ComputePipeline,
    pub compute_ppl_args: ComputePipeline,
    pub lod_bgl: BindGroupLayout,
    pub compute_ppl_lod: ComputePipeline,
}

impl FromWorld for VoxelFacesPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let shader = world.resource::<VoxelFacesShader>().0.clone();
        let mesh_pipeline = world.resource::<MeshPipeline>().clone();

        let uniform_entries = [
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: Some(
                        <VoxelUniform as ShaderType>::min_size(),
                    ),
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        let uniform_layout = render_device.create_bind_group_layout("voxel_faces_uniform_layout", &uniform_entries);
        let uniform_layout_desc = BindGroupLayoutDescriptor::new("voxel_faces_uniform_layout", &uniform_entries);

        let compute_entries = [
            // params (slot_idx, chunk_dim, max_out)
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
            // atlas: all chunks voxels (slot_count * CHUNK_SIZE³ u32s)
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
            // neighbor_table: slot_count * 6 i32s (-1 = no neighbor)
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
            // header (atomic counter)
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
            // faces out
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
            // indirect args out
            BindGroupLayoutEntry {
                binding: 5,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        let compute_bgl = render_device.create_bind_group_layout("voxel_faces_compute_bgl", &compute_entries);

        let compute_module = unsafe { render_device.create_shader_module(ShaderModuleDescriptor {
            label: Some("voxel_faces_compute_sm"),
            source: ShaderSource::Wgsl(VOXEL_FACES_COMPUTE_WGSL.into()),
        }) };
        let compute_pl = render_device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("voxel_faces_compute_pl"),
            bind_group_layouts: &[&compute_bgl],
            immediate_size: 0,
        });
        let compute_ppl_faces = render_device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("voxel_faces_compute_faces"),
            layout: Some(&compute_pl),
            module: &compute_module,
            entry_point: Some("gen_faces"),
            compilation_options: Default::default(),
            cache: None,
        });
        let compute_ppl_greedy = render_device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("voxel_faces_compute_greedy"),
            layout: Some(&compute_pl),
            module: &compute_module,
            entry_point: Some("gen_greedy"),
            compilation_options: Default::default(),
            cache: None,
        });
        let compute_ppl_args = render_device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("voxel_faces_compute_args"),
            layout: Some(&compute_pl),
            module: &compute_module,
            entry_point: Some("build_args"),
            compilation_options: Default::default(),
            cache: None,
        });

        let lod_entries = [
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
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        let lod_bgl = render_device.create_bind_group_layout("voxel_lod_downsample_bgl", &lod_entries);
        let lod_module = unsafe { render_device.create_shader_module(ShaderModuleDescriptor {
            label: Some("voxel_lod_downsample_sm"),
            source: ShaderSource::Wgsl(VOXEL_LOD_DOWNSAMPLE_WGSL.into()),
        }) };
        let lod_pl = render_device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("voxel_lod_downsample_pl"),
            bind_group_layouts: &[&lod_bgl],
            immediate_size: 0,
        });
        let compute_ppl_lod = render_device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("voxel_lod_downsample"),
            layout: Some(&lod_pl),
            module: &lod_module,
            entry_point: Some("downsample"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            shader,
            mesh_pipeline,
            uniform_layout,
            uniform_layout_desc,
            compute_bgl,
            compute_ppl_faces,
            compute_ppl_greedy,
            compute_ppl_args,
            lod_bgl,
            compute_ppl_lod,
        }
    }
}

impl SpecializedMeshPipeline for VoxelFacesPipeline {
    type Key = MeshPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh_pipeline.specialize(key, layout)?;

        descriptor.vertex.shader = self.shader.clone();
        descriptor.fragment.as_mut().unwrap().shader = self.shader.clone();

        if descriptor.depth_stencil.is_none() {
            descriptor.depth_stencil = Some(DepthStencilState {
                format: CORE_3D_DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: CompareFunction::GreaterEqual,
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            });
        }

        // Group 2: our voxel uniform + palette (replaces mesh uniform usage).
        descriptor.set_layout(2, self.uniform_layout_desc.clone());

        // Vertex buffers:
        // slot 0: quad mesh (pos+uv)
        let quad_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
        ])?;

        // slot 1: instance buffer (FaceInstance)
        let instance_layout = VertexBufferLayout {
            array_stride: std::mem::size_of::<FaceInstance>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: vec![
                VertexAttribute { format: VertexFormat::Sint32x3, offset: 0, shader_location: 2 },
                VertexAttribute { format: VertexFormat::Uint32, offset: 12, shader_location: 3 },
                VertexAttribute { format: VertexFormat::Uint32, offset: 16, shader_location: 4 },
                VertexAttribute { format: VertexFormat::Uint32, offset: 20, shader_location: 5 },
                VertexAttribute { format: VertexFormat::Uint32, offset: 24, shader_location: 6 },
                VertexAttribute { format: VertexFormat::Uint32, offset: 28, shader_location: 7 },
            ],
        };

        descriptor.vertex.buffers.clear();
        descriptor.vertex.buffers.push(quad_layout);
        descriptor.vertex.buffers.push(instance_layout);

        descriptor.primitive.topology = PrimitiveTopology::TriangleList;
        descriptor.primitive.cull_mode = None;
        Ok(descriptor)
    }
}

// ---------------- Render World: GPU buffers per chunk ----------------

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub struct FaceInstance {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub dir: u32,
    pub pi: u32,
    pub lod: u32,
    pub span_u: u32,
    pub span_v: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct ComputeParams {
    slot_idx: u32,
    chunk_dim: u32,
    max_out: u32,
    lod: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct LodParams {
    slot_count: u32,
    src_dim: u32,
    dst_dim: u32,
    _pad0: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Header {
    count: u32,
    _pad0: [u32; 3],
    _pad1: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct DrawIndexedIndirect {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

/// Per-chunk GPU output buffers (faces, args, header).
#[derive(Clone)]
struct GpuChunkOut {
    slot: u32,
    lod: u8,
    _pad0: [u8; 3],
    slot_version: u64,
    atlas_gen: u64,
    params: Buffer,
    header: Buffer,
    faces: Buffer,
    args: Buffer,
    bind_group: BindGroup,
    max_out: u32,
}

#[derive(Clone)]
struct GpuLodStep {
    params: Buffer,
    bind_group: BindGroup,
    dst_dim: u32,
}

/// Per-node chunk atlas (all chunks in one big buffer).
#[derive(Default)]
struct NodeAtlas {
    /// Raw voxel atlases per LOD: slot_count * dim³ u32s
    atlas_lods: Vec<Option<Buffer>>,
    /// Neighbor index table: slot_count * 6 i32s (-1 = no neighbor)
    neighbor_table: Option<Buffer>,
    /// Chunk key -> slot index
    slot_map: HashMap<IVec3, u32>,
    /// Slot versions (incremented when slot data changes).
    slot_versions: Vec<u64>,
    /// Current capacity in slots
    capacity: u32,
    /// Generation counter (increments when atlas buffers are reallocated).
    atlas_gen: u64,
    /// Downsample steps 0->1,1->2,2->3 for this node.
    lod_steps: Vec<Option<GpuLodStep>>,
    /// Order-independent hash of current alive chunk keys (for neighbor-table rebuild gating).
    alive_hash: u64,
    /// Alive chunk keys (for incremental neighbor updates).
    alive_keys: HashSet<IVec3>,
    /// Cached neighbor data (slot_count * 6).
    neighbor_data: Vec<i32>,
    /// Dirty neighbor slots (for partial GPU updates).
    neighbor_dirty_slots: Vec<u32>,
    /// Temp storage for alive diffs.
    alive_diff: Vec<IVec3>,
}

#[derive(Resource, Default)]
pub struct VoxelGpuChunks {
    /// Per-node atlas
    atlases: HashMap<Uuid, NodeAtlas>,
    /// (node, chunk, lod) -> output buffers
    outputs: HashMap<(Uuid, IVec3, u8), GpuChunkOut>,
    /// Current per-chunk lod selection (updated in queue).
    current_lod: HashMap<(Uuid, IVec3), u8>,
    /// node -> palette buffer
    palette: HashMap<Uuid, Buffer>,
    /// node -> voxel size
    node_voxel_size: HashMap<Uuid, f32>,
}

// ---------------- Render World: uniforms ----------------

pub fn prepare_voxel_uniforms(
    mut commands: Commands,
    mut uniforms: ResMut<VoxelFacesUniforms>,
    render_chunks: Query<(Entity, &RenderSdfChunk)>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline: Res<VoxelFacesPipeline>,
    extracted_dirty: Option<Res<ExtractedVoxelDirty>>,
    extracted_viz: Option<Res<ExtractedVoxelVizOptions>>,
    mut gpu: ResMut<VoxelGpuChunks>,
    mut stats: ResMut<VoxelFacesStats>,
) {
    puffin::profile_function!();
    uniforms.buffer.clear();

    let (wire_px, display_mode, ghost) = extracted_viz
        .as_ref()
        .map(|v| (v.wire_px, v.display_mode, v.ghost))
        .unwrap_or((0.5, 2, false));

    let mut palette_replaced_nodes: Vec<Uuid> = Vec::new();
    if let Some(d) = extracted_dirty.as_ref() {
        for (id, snap) in d.per_node.iter() {
            gpu.node_voxel_size.insert(*id, snap.voxel_size);

            if let Some(pal) = snap.palette_rgba.as_ref() {
                let req_size = (pal.len() * std::mem::size_of::<[f32; 4]>()) as u64;
                let need_new = gpu
                    .palette
                    .get(id)
                    .map(|b| b.size() < req_size)
                    .unwrap_or(true);

                if need_new {
                    let buf = render_device.create_buffer_with_data(&BufferInitDescriptor {
                        label: Some("voxel_palette_buf"),
                        contents: bytemuck::cast_slice(pal),
                        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                    });
                    gpu.palette.insert(*id, buf);
                    palette_replaced_nodes.push(*id);
                    stats.palette_upload_bytes = stats.palette_upload_bytes.saturating_add(req_size);
                } else if let Some(buf) = gpu.palette.get(id) {
                    render_queue.write_buffer(buf, 0, bytemuck::cast_slice(pal));
                    stats.palette_upload_bytes = stats.palette_upload_bytes.saturating_add(req_size);
                }
            }
        }
    }

    let mut visible_node_ids: Vec<Uuid> = Vec::new();
    for (entity, ch) in render_chunks.iter() {
        visible_node_ids.push(ch.node_id);
        let vs = gpu.node_voxel_size.get(&ch.node_id).copied().unwrap_or(0.1);
        let u = VoxelUniform {
            model: ch.transform.to_matrix(),
            // chunk base is already in `model` translation (see main-world spawn).
            chunk_base_world: Vec4::new(0.0, 0.0, 0.0, vs),
            wire_params: Vec4::new(wire_px, display_mode as f32, if ghost { 1.0 } else { 0.0 }, 0.0),
        };
        let offset = uniforms.buffer.push(&u) as u32;
        commands.entity(entity).insert(VoxelUniformOffset(offset));
    }

    uniforms.buffer.write_buffer(&render_device, &render_queue);

    let cur_uniform_buffer_id = uniforms.buffer.buffer().map(|b| b.id());
    if cur_uniform_buffer_id != uniforms.uniform_buffer_id {
        uniforms.uniform_buffer_id = cur_uniform_buffer_id;
        uniforms.bind_groups.clear();
    }
    for id in palette_replaced_nodes {
        uniforms.bind_groups.remove(&id);
    }

    visible_node_ids.sort_unstable();
    visible_node_ids.dedup();

    // Collect nodes needing bind groups while we can still immutably borrow uniforms
    let nodes_needing_bg: Vec<Uuid> = visible_node_ids
        .iter()
        .filter(|id| !uniforms.bind_groups.contains_key(id))
        .copied()
        .collect();
    
    if let Some(binding) = uniforms.buffer.binding() {
        // Create all bind groups first
        let mut new_bind_groups: Vec<(Uuid, BindGroup)> = Vec::new();
        for node_id in nodes_needing_bg {
            let Some(pal_buf) = gpu.palette.get(&node_id) else { continue; };
            let bg = render_device.create_bind_group(
                "voxel_faces_bg",
                &pipeline.uniform_layout,
                &[
                    BindGroupEntry { binding: 0, resource: binding.clone() },
                    BindGroupEntry { binding: 1, resource: pal_buf.as_entire_binding() },
                ],
            );
            new_bind_groups.push((node_id, bg));
        }
        // Now insert (binding borrow is dropped)
        for (node_id, bg) in new_bind_groups {
            uniforms.bind_groups.insert(node_id, bg);
        }
    }
}

pub struct SetVoxelBindGroup<const I: usize>;

impl<const I: usize, P: PhaseItem> RenderCommand<P> for SetVoxelBindGroup<I> {
    type Param = SRes<VoxelFacesUniforms>;
    type ViewQuery = ();
    type ItemQuery = (&'static RenderSdfChunk, &'static VoxelUniformOffset);

    fn render<'w>(
        _item: &P,
        _view: (),
        item_query: Option<(&'w RenderSdfChunk, &'w VoxelUniformOffset)>,
        uniforms: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some((ch, off)) = item_query else {
            return RenderCommandResult::Failure("missing RenderSdfChunk/VoxelUniformOffset");
        };
        let uniforms = uniforms.into_inner();
        let Some(bg) = uniforms.bind_groups.get(&ch.node_id) else {
            return RenderCommandResult::Failure("missing voxel bind group");
        };
        pass.set_bind_group(I, bg, &[off.0]);
        RenderCommandResult::Success
    }
}

// ---------------- Render World: draw command ----------------

pub struct DrawVoxelFaces;

impl<P: PhaseItem> RenderCommand<P> for DrawVoxelFaces {
    type Param = (
        SRes<RenderAssets<RenderMesh>>,
        SRes<MeshAllocator>,
        SRes<VoxelGpuChunks>,
    );
    type ViewQuery = ();
    type ItemQuery = &'static RenderSdfChunk;

    fn render<'w>(
        _item: &P,
        _view: (),
        item: Option<&'w RenderSdfChunk>,
        (meshes, mesh_allocator, gpu): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(item) = item else {
            return RenderCommandResult::Failure("missing RenderSdfChunk");
        };
        let gpu = gpu.into_inner();
        let lod = gpu.current_lod.get(&(item.node_id, item.chunk)).copied().unwrap_or(0);
        let key = (item.node_id, item.chunk, lod);
        let Some(g) = gpu.outputs.get(&key) else { return RenderCommandResult::Success; };

        let meshes = meshes.into_inner();
        let mesh_allocator = mesh_allocator.into_inner();
        let mesh_id = item.mesh_handle.id();
        let Some(render_mesh) = meshes.get(mesh_id) else {
            return RenderCommandResult::Failure("quad mesh not prepared");
        };
        let Some(vs) = mesh_allocator.mesh_vertex_slice(&mesh_id) else {
            return RenderCommandResult::Success;
        };
        let stride = render_mesh.layout.0.layout().array_stride as u64;
        let start = vs.range.start as u64 * stride;
        let end = vs.range.end as u64 * stride;
        pass.set_vertex_buffer(0, vs.buffer.slice(start..end));

        let Some(is) = mesh_allocator.mesh_index_slice(&mesh_id) else {
            return RenderCommandResult::Failure("quad mesh missing indices");
        };
        // Our quad mesh is built with u16 indices.
        let index_format = IndexFormat::Uint16;
        let index_size = 2u64;
        let istart = is.range.start as u64 * index_size;
        let iend = is.range.end as u64 * index_size;
        pass.set_index_buffer(is.buffer.slice(istart..iend), index_format);

        pass.set_vertex_buffer(1, g.faces.slice(..));
        // Indirect draw is written by compute.
        pass.draw_indexed_indirect(&g.args, 0);
        RenderCommandResult::Success
    }
}

pub type DrawVoxelFacesPipeline = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetVoxelBindGroup<2>,
    DrawVoxelFaces,
);

// ---------------- Render World: queue (compute + enqueue draw items) ----------------

#[derive(Default)]
struct VoxelFacesQueueScratch {
    dirty_keys: Vec<(Uuid, IVec3, u8)>,
    visible_nodes: HashSet<Uuid>,
    nodes_to_downsample: Vec<Uuid>,
    chunks_to_create: Vec<(Uuid, IVec3, u32, u32)>,
    atlas_bufs: Vec<Buffer>,
    greedy_keys: Vec<(Uuid, IVec3, u8)>,
    face_keys: Vec<(Uuid, IVec3, u8)>,
}

const NEIGHBOR_OFFSETS: [(i32, i32, i32); 6] = [
    (1, 0, 0),   // +X
    (-1, 0, 0),  // -X
    (0, 1, 0),   // +Y
    (0, -1, 0),  // -Y
    (0, 0, 1),   // +Z
    (0, 0, -1),  // -Z
];

#[inline]
fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[inline]
fn hash_alive_keys(keys: &[IVec3]) -> u64 {
    let mut h = (keys.len() as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93);
    for k in keys {
        let v = (k.x as u32 as u64).wrapping_mul(0xA24B_AED4_963E_E407)
            ^ (k.y as u32 as u64).wrapping_mul(0x9FB2_1C65_1E98_DF25)
            ^ (k.z as u32 as u64).wrapping_mul(0xC3A5_C85C_97CB_3127);
        h ^= mix64(v);
    }
    h
}

#[inline]
fn chunk_coord_i(p: IVec3, cs: i32) -> IVec3 { IVec3::new(p.x.div_euclid(cs), p.y.div_euclid(cs), p.z.div_euclid(cs)) }
#[inline]
fn chunk_local_i(p: IVec3, cs: i32) -> IVec3 { IVec3::new(p.x.rem_euclid(cs), p.y.rem_euclid(cs), p.z.rem_euclid(cs)) }
#[inline]
fn chunk_idx_i(lp: IVec3, cs: i32) -> usize { (lp.z as usize) * (cs as usize) * (cs as usize) + (lp.y as usize) * (cs as usize) + (lp.x as usize) }
#[inline]
fn get_pi_u32(chunks: &HashMap<IVec3, Vec<u32>>, cs: i32, p: IVec3) -> u32 {
    let ck = chunk_coord_i(p, cs);
    let lp = chunk_local_i(p, cs);
    chunks.get(&ck).and_then(|v| v.get(chunk_idx_i(lp, cs)).copied()).unwrap_or(0)
}

#[inline]
fn greedy_mesh_mask(mask: &mut [i32], w: usize, h: usize, mut emit: impl FnMut(usize, usize, usize, usize, i32)) {
    for y in 0..h {
        let mut x = 0usize;
        while x < w {
            let v = mask[y * w + x];
            if v == 0 { x += 1; continue; }
            let mut ww = 1usize;
            while x + ww < w && mask[y * w + x + ww] == v { ww += 1; }
            let mut hh = 1usize;
            'outer: while y + hh < h {
                for xx in 0..ww { if mask[(y + hh) * w + x + xx] != v { break 'outer; } }
                hh += 1;
            }
            emit(x, y, ww, hh, v);
            for yy in 0..hh { for xx in 0..ww { mask[(y + yy) * w + x + xx] = 0; } }
            x += ww;
        }
    }
}

#[derive(SystemParam)]
pub struct QueueSdfChunksParams<'w, 's> {
    opaque_3d_draw_functions: Res<'w, DrawFunctions<Opaque3d>>,
    pipeline: Res<'w, VoxelFacesPipeline>,
    pipelines: ResMut<'w, SpecializedMeshPipelines<VoxelFacesPipeline>>,
    pipeline_cache: Res<'w, PipelineCache>,
    render_meshes: Res<'w, RenderAssets<RenderMesh>>,
    render_mesh_instances: Res<'w, RenderMeshInstances>,
    mesh_allocator: Res<'w, MeshAllocator>,
    _gpu_preprocessing_support: Res<'w, GpuPreprocessingSupport>,
    ticks: bevy::ecs::system::SystemChangeTick,
    extracted_dirty: Option<Res<'w, ExtractedVoxelDirty>>,
    gpu: ResMut<'w, VoxelGpuChunks>,
    render_device: Res<'w, RenderDevice>,
    render_queue: Res<'w, RenderQueue>,
    render_chunks: Query<'w, 's, &'static RenderSdfChunk>,
    opaque_render_phases: ResMut<'w, ViewBinnedRenderPhases<Opaque3d>>,
    stats: ResMut<'w, VoxelFacesStats>,
    stats_shared: Res<'w, VoxelFacesStatsShared>,
    stats_cfg: Res<'w, VoxelFacesStatsConfig>,
    lod_cfg: Res<'w, VoxelLodConfig>,
    _greedy_cfg: Res<'w, VoxelGreedyBackendConfig>,
    scratch: Local<'s, VoxelFacesQueueScratch>,
    views: Query<
        'w,
        's,
        (
            &'static ExtractedView,
            Option<&'static Frustum>,
            &'static Msaa,
            &'static RenderVisibleEntities,
            Option<&'static ViewPrepassTextures>,
            bevy::ecs::query::Has<OrderIndependentTransparencySettings>,
            bevy::ecs::query::Has<ExtractedAtmosphere>,
        ),
    >,
}

pub fn queue_voxel_chunks(mut p: QueueSdfChunksParams) {
    puffin::profile_function!();
    let QueueSdfChunksParams {
        opaque_3d_draw_functions,
        pipeline,
        mut pipelines,
        pipeline_cache,
        render_meshes,
        render_mesh_instances,
        mesh_allocator,
        _gpu_preprocessing_support,
        ticks,
        extracted_dirty,
        mut gpu,
        render_device,
        render_queue,
        render_chunks,
        mut opaque_render_phases,
        mut stats,
        stats_shared,
        stats_cfg,
        lod_cfg,
        _greedy_cfg,
        mut scratch,
        views,
    } = p;
    let gpu = gpu.into_inner();
    // Reset per-frame counters (queue runs once per render frame).
    stats.frame = stats.frame.saturating_add(1);
    stats.visible_chunks = 0;
    stats.visible_nodes = 0;
    stats.dirty_chunks_uploaded = 0;
    stats.atlas_upload_bytes = 0;
    stats.neighbor_upload_bytes = 0;
    stats.estimated_face_instances = 0;
    stats.compute_dispatches_faces = 0;
    stats.compute_dispatches_args = 0;
    stats.submits = 0;
    stats.draws = 0;
    gpu.current_lod.clear();

    let draw_function = opaque_3d_draw_functions
        .read()
        .get_id::<DrawVoxelFacesPipeline>()
        .unwrap();

    let cs = cunning_kernel::geometry::voxel::CHUNK_SIZE as u32;
    let voxels_per_chunk = (cs * cs * cs) as usize;
    let max_faces_hard = 6u32.saturating_mul(cs.saturating_pow(3));

    // Upload dirty raw chunks to atlas + dispatch compute.
    if let Some(d) = extracted_dirty.as_ref() {
        scratch.dirty_keys.clear();
        scratch.visible_nodes.clear();
        scratch.nodes_to_downsample.clear();
        scratch.chunks_to_create.clear();

        for (node_id, snap) in d.per_node.iter() {
            scratch.visible_nodes.insert(*node_id);
            // Ensure atlas exists and has enough capacity
            let all_keys = &snap.all_chunk_keys;
            let needed_slots = all_keys.len().max(1);
            ensure_node_atlas(&mut gpu.atlases, *node_id, needed_slots as u32, cs, &render_device, &render_queue);

            let atlas = gpu.atlases.get_mut(node_id).unwrap();

            // Assign slots to any new chunks
            for ck in all_keys.iter() {
                if !atlas.slot_map.contains_key(ck) {
                    let slot = atlas.slot_map.len() as u32;
                    atlas.slot_map.insert(*ck, slot);
                    atlas.slot_versions.push(0);
                }
            }

            let slot_count = atlas.slot_map.len();
            // Alive chunks affect neighbor table; keep slot_map stable, but treat non-alive as air.
            let alive_hash = hash_alive_keys(all_keys);
            let need_neighbor_update = atlas.alive_hash != alive_hash || atlas.neighbor_data.len() != slot_count * 6;
            if need_neighbor_update {
                let new_alive: HashSet<IVec3> = all_keys.iter().copied().collect();

                let want_len = slot_count * 6;
                if atlas.neighbor_data.len() != want_len {
                    atlas.neighbor_data.resize(want_len, -1);
                    atlas.neighbor_data.fill(-1);
                }

                atlas.alive_diff.clear();
                for ck in new_alive.iter() {
                    if !atlas.alive_keys.contains(ck) { atlas.alive_diff.push(*ck); }
                }
                for ck in atlas.alive_keys.iter() {
                    if !new_alive.contains(ck) { atlas.alive_diff.push(*ck); }
                }

                atlas.neighbor_dirty_slots.clear();
                let dirs7 = [
                    IVec3::ZERO,
                    IVec3::X, IVec3::NEG_X,
                    IVec3::Y, IVec3::NEG_Y,
                    IVec3::Z, IVec3::NEG_Z,
                ];
                for ck in atlas.alive_diff.iter().copied() {
                    for d7 in dirs7 {
                        let ck2 = ck + d7;
                        let Some(&slot) = atlas.slot_map.get(&ck2) else { continue; };
                        let base = (slot as usize) * 6;
                        if !new_alive.contains(&ck2) {
                            atlas.neighbor_data[base..base + 6].fill(-1);
                            atlas.neighbor_dirty_slots.push(slot);
                            continue;
                        }
                        for (dir, (dx, dy, dz)) in NEIGHBOR_OFFSETS.iter().enumerate() {
                            let nck = ck2 + IVec3::new(*dx, *dy, *dz);
                            let v = if new_alive.contains(&nck) {
                                atlas.slot_map.get(&nck).copied().map(|s| s as i32).unwrap_or(-1)
                            } else { -1 };
                            atlas.neighbor_data[base + dir] = v;
                        }
                        atlas.neighbor_dirty_slots.push(slot);
                    }
                }

                if let Some(ref neighbor_buf) = atlas.neighbor_table {
                    atlas.neighbor_dirty_slots.sort_unstable();
                    atlas.neighbor_dirty_slots.dedup();
                    let mut i = 0usize;
                    while i < atlas.neighbor_dirty_slots.len() {
                        let start_slot = atlas.neighbor_dirty_slots[i] as usize;
                        let mut end_slot = start_slot;
                        while i + 1 < atlas.neighbor_dirty_slots.len() && atlas.neighbor_dirty_slots[i + 1] as usize == end_slot + 1 {
                            i += 1;
                            end_slot += 1;
                        }
                        let start_i = start_slot * 6;
                        let end_i = (end_slot + 1) * 6;
                        let bytes = ((end_i - start_i) * 4) as u64;
                        let offset = (start_i * 4) as u64;
                        if offset + bytes <= neighbor_buf.size() {
                            render_queue.write_buffer(neighbor_buf, offset, bytemuck::cast_slice(&atlas.neighbor_data[start_i..end_i]));
                            stats.neighbor_upload_bytes = stats.neighbor_upload_bytes.saturating_add(bytes);
                        }
                        i += 1;
                    }
                }

                atlas.alive_hash = alive_hash;
                atlas.alive_keys = new_alive;
            }

            // Ensure per-node LOD downsample steps exist and update their params.
            let slot_count_u32 = slot_count as u32;
            for src_lod in 0..(VOXEL_LOD_LEVELS - 1) as usize {
                let src_dim = lod_dim(cs, src_lod as u8);
                let dst_dim = lod_dim(cs, (src_lod as u8) + 1);
                let src_buf = atlas.atlas_lods.get(src_lod).and_then(|b| b.as_ref()).unwrap();
                let dst_buf = atlas.atlas_lods.get(src_lod + 1).and_then(|b| b.as_ref()).unwrap();
                if atlas.lod_steps.get(src_lod).and_then(|s| s.as_ref()).is_none() {
                    let params = LodParams { slot_count: slot_count_u32, src_dim, dst_dim, _pad0: 0 };
                    let params_buf = render_device.create_buffer_with_data(&BufferInitDescriptor {
                        label: Some("voxel_lod_params"),
                        contents: bytemuck::bytes_of(&params),
                        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                    });
                    let bind_group = render_device.create_bind_group(
                        "voxel_lod_downsample_bg",
                        &pipeline.lod_bgl,
                        &[
                            BindGroupEntry { binding: 0, resource: params_buf.as_entire_binding() },
                            BindGroupEntry { binding: 1, resource: src_buf.as_entire_binding() },
                            BindGroupEntry { binding: 2, resource: dst_buf.as_entire_binding() },
                        ],
                    );
                    if atlas.lod_steps.len() <= src_lod {
                        atlas.lod_steps.resize_with((VOXEL_LOD_LEVELS - 1) as usize, || None);
                    }
                    atlas.lod_steps[src_lod] = Some(GpuLodStep { params: params_buf, bind_group, dst_dim });
                } else if let Some(step) = atlas.lod_steps.get(src_lod).and_then(|s| s.as_ref()) {
                    let params = LodParams { slot_count: slot_count_u32, src_dim, dst_dim, _pad0: 0 };
                    render_queue.write_buffer(&step.params, 0, bytemuck::bytes_of(&params));
                }
            }

            // Upload dirty raw chunks to atlas and collect info
            for (ck, raw, solid) in snap.dirty_raw_chunks.iter() {
                if *solid == 0 {
                    continue;
                }
                let Some(&slot) = atlas.slot_map.get(ck) else { continue; };
                let offset = (slot as usize) * voxels_per_chunk * 4;
                if let Some(ref atlas_buf) = atlas.atlas_lods.get(0).and_then(|b| b.as_ref()) {
                    if offset + raw.len() * 4 <= atlas_buf.size() as usize {
                        render_queue.write_buffer(atlas_buf, offset as u64, bytemuck::cast_slice(raw));
                        stats.atlas_upload_bytes = stats.atlas_upload_bytes.saturating_add((raw.len() * 4) as u64);
                    }
                }
                if (slot as usize) < atlas.slot_versions.len() {
                    atlas.slot_versions[slot as usize] = atlas.slot_versions[slot as usize].saturating_add(1);
                }

                let wanted = ((*solid).saturating_mul(6)).clamp(1, max_faces_hard);
                stats.estimated_face_instances = stats.estimated_face_instances.saturating_add(wanted as u64);
                scratch.chunks_to_create.push((*node_id, *ck, slot, *solid));
            }

            if !snap.dirty_raw_chunks.is_empty() {
                scratch.nodes_to_downsample.push(*node_id);
            }
        }
        stats.visible_nodes = scratch.visible_nodes.len().min(u32::MAX as usize) as u32;

        // Create/resize outputs for all LODs of dirty chunks.
        let chunks_to_create = std::mem::take(&mut scratch.chunks_to_create);
        for (node_id, ck, slot, solid0) in chunks_to_create.iter().copied() {
            let Some(atlas) = gpu.atlases.get(&node_id) else { continue; };
            let Some(neighbor_buf) = atlas.neighbor_table.as_ref() else { continue; };
            scratch.atlas_bufs.clear();
            scratch.atlas_bufs.reserve(VOXEL_LOD_LEVELS as usize);
            for lod in 0..VOXEL_LOD_LEVELS as usize {
                let Some(buf) = atlas.atlas_lods.get(lod).and_then(|b| b.as_ref()) else { scratch.atlas_bufs.clear(); break; };
                scratch.atlas_bufs.push(buf.clone());
            }
            if scratch.atlas_bufs.len() != VOXEL_LOD_LEVELS as usize { continue; }
            let neighbor_buf = neighbor_buf.clone();
            for lod in 0..VOXEL_LOD_LEVELS {
                let dim = lod_dim(cs, lod).max(1);
                let solid_est = if lod == 0 { solid0 } else {
                    let s = 1u32 << (3u32.saturating_mul(lod as u32)).min(24);
                    solid0.saturating_add(s - 1) / s
                };
                let max_faces_hard_lod = 6u32.saturating_mul(dim.saturating_pow(3));
                let wanted = solid_est.saturating_mul(6).clamp(1, max_faces_hard_lod);
                let max_out = wanted.next_power_of_two().min(max_faces_hard_lod);
                let key = (node_id, ck, lod);
                let atlas_gen = atlas.atlas_gen;
                ensure_chunk_output(&mut gpu.outputs, &render_device, &render_queue, &pipeline, &scratch.atlas_bufs[lod as usize], &neighbor_buf, key, slot, dim, max_out, atlas_gen);
                if let Some(out) = gpu.outputs.get(&key) {
                    let cur_ver = atlas.slot_versions.get(slot as usize).copied().unwrap_or(0);
                    if out.slot_version != cur_ver {
                        render_queue.write_buffer(&out.header, 0, bytemuck::bytes_of(&Header { count: 0, _pad0: [0; 3], _pad1: [0; 4] }));
                    }
                }
                scratch.dirty_keys.push(key);
            }
        }
        scratch.dirty_keys.sort_unstable_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.x.cmp(&b.1.x))
                .then_with(|| a.1.y.cmp(&b.1.y))
                .then_with(|| a.1.z.cmp(&b.1.z))
                .then_with(|| a.2.cmp(&b.2))
        });
        scratch.dirty_keys.dedup();
        stats.dirty_chunks_uploaded = scratch.dirty_keys.len().min(u32::MAX as usize) as u32;

        // Dispatch compute for LOD downsample + dirty chunks
        if !scratch.dirty_keys.is_empty() || !scratch.nodes_to_downsample.is_empty() {
            let mut enc = render_device.create_command_encoder(&CommandEncoderDescriptor {
                label: Some("voxel_faces_compute_encoder"),
            });
            if !scratch.nodes_to_downsample.is_empty() {
                let mut pass = enc.begin_compute_pass(&ComputePassDescriptor {
                    label: Some("voxel_lod_downsample"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline.compute_ppl_lod);
                for node_id in scratch.nodes_to_downsample.iter() {
                    let Some(atlas) = gpu.atlases.get(node_id) else { continue; };
                    let slot_count = atlas.slot_map.len() as u32;
                    if slot_count == 0 { continue; }
                    for step in atlas.lod_steps.iter().filter_map(|s| s.as_ref()) {
                        let d = step.dst_dim.max(1);
                        let wgx = (d + 3) / 4;
                        let wgy = (d + 3) / 4;
                        let total_z = slot_count.saturating_mul(d);
                        let wgz = (total_z + 3) / 4;
                        pass.set_bind_group(0, &step.bind_group, &[]);
                        pass.dispatch_workgroups(wgx, wgy, wgz);
                    }
                }
            }
            {
                scratch.greedy_keys.clear();
                scratch.face_keys.clear();
                let dirty_keys = scratch.dirty_keys.clone();
                for k in dirty_keys.into_iter() {
                    if k.2 == 0 { scratch.greedy_keys.push(k); } else { scratch.face_keys.push(k); }
                }

                if !scratch.greedy_keys.is_empty() {
                    let mut pass = enc.begin_compute_pass(&ComputePassDescriptor {
                        label: Some("voxel_faces_gen_greedy"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&pipeline.compute_ppl_greedy);
                    for key in scratch.greedy_keys.iter() {
                        let out = gpu.outputs.get(key).unwrap();
                        pass.set_bind_group(0, &out.bind_group, &[]);
                        let d = lod_dim(cs, key.2).max(1);
                        let total = 3u32.saturating_mul(d.saturating_add(1));
                        // New greedy kernel: one workgroup per slice.
                        pass.dispatch_workgroups(total, 1, 1);
                        stats.compute_dispatches_faces = stats.compute_dispatches_faces.saturating_add(1);
                    }
                }

                if !scratch.face_keys.is_empty() {
                    let mut pass = enc.begin_compute_pass(&ComputePassDescriptor {
                        label: Some("voxel_faces_gen_faces"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&pipeline.compute_ppl_faces);
                    for key in scratch.face_keys.iter() {
                        let out = gpu.outputs.get(key).unwrap();
                        pass.set_bind_group(0, &out.bind_group, &[]);
                        let d = lod_dim(cs, key.2).max(1);
                        let n = d.saturating_pow(3);
                        let wg = (n + 255) / 256;
                        pass.dispatch_workgroups(wg, 1, 1);
                        stats.compute_dispatches_faces = stats.compute_dispatches_faces.saturating_add(1);
                    }
                }
            }
            {
                let mut pass = enc.begin_compute_pass(&ComputePassDescriptor {
                    label: Some("voxel_faces_build_args"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline.compute_ppl_args);
                for key in scratch.dirty_keys.iter() {
                    let out = gpu.outputs.get(key).unwrap();
                    pass.set_bind_group(0, &out.bind_group, &[]);
                    pass.dispatch_workgroups(1, 1, 1);
                    stats.compute_dispatches_args = stats.compute_dispatches_args.saturating_add(1);
                }
            }
            render_queue.submit(std::iter::once(enc.finish()));
            stats.submits = stats.submits.saturating_add(1);

            // Update last-built slot versions for these outputs.
            for key in scratch.dirty_keys.iter() {
                if let Some(out) = gpu.outputs.get_mut(key) {
                    if let Some(atlas) = gpu.atlases.get(&key.0) {
                        out.slot_version = atlas.slot_versions.get(out.slot as usize).copied().unwrap_or(0);
                    }
                }
            }
        }
    }

    // Queue render items per view.
    for (view, frustum, msaa, visible, prepass_textures, has_oit, has_atmosphere) in views.iter() {
        let cam_pos = view.world_from_view.translation();
        let mut view_key = MeshPipelineKey::from_msaa_samples(msaa.samples()) | MeshPipelineKey::from_hdr(view.hdr);
        if has_oit { view_key |= MeshPipelineKey::OIT_ENABLED; }
        if has_atmosphere { view_key |= MeshPipelineKey::ATMOSPHERE; }
        if let Some(pt) = prepass_textures {
            if pt.depth.is_some() { view_key |= MeshPipelineKey::DEPTH_PREPASS; }
            if pt.normal.is_some() { view_key |= MeshPipelineKey::NORMAL_PREPASS; }
            if pt.motion_vectors.is_some() { view_key |= MeshPipelineKey::MOTION_VECTOR_PREPASS; }
            if pt.deferred.is_some() { view_key |= MeshPipelineKey::DEFERRED_PREPASS; }
        }
        let Some(phase) = opaque_render_phases.get_mut(&view.retained_view_entity) else { continue; };

        for (render_entity, visible_entity) in visible.iter::<Mesh3d>() {
            let Ok(ch) = render_chunks.get(*render_entity) else { continue; };
            let vs = gpu.node_voxel_size.get(&ch.node_id).copied().unwrap_or(0.1).max(0.001);
            let base = ch.transform.translation();
            let c = base + Vec3::splat((cs as f32) * 0.5 * vs);
            if let Some(f) = frustum {
                let s = Sphere { center: bevy::math::Vec3A::new(c.x, c.y, c.z), radius: (cs as f32) * vs * 0.866_025_4 };
                if !f.intersects_sphere(&s, true) { continue; }
            }
            let d = (cam_pos - c).length();
            let t = lod_cfg.thresholds;
            let lod = if d < t[0] { 0 } else if d < t[1] { 1 } else if d < t[2] { 2 } else { 3 };
            gpu.current_lod.insert((ch.node_id, ch.chunk), lod);
            stats.visible_chunks = stats.visible_chunks.saturating_add(1);
            let mut mesh_asset_id = ch.mesh_handle.id();
            let mut current_uniform_index = InputUniformIndex::default();
            let mut lightmap_slab = None;
            if let Some(mi) = render_mesh_instances.render_mesh_queue_data(*visible_entity) {
                mesh_asset_id = mi.shared.mesh_asset_id;
                current_uniform_index = mi.current_uniform_index;
                lightmap_slab = mi.shared.lightmap_slab_index.map(|i| *i);
            }
            let Some(mesh) = render_meshes.get(mesh_asset_id) else { continue; };

            let key = view_key | MeshPipelineKey::from_primitive_topology(PrimitiveTopology::TriangleList);
            let Ok(pipeline_id) = pipelines.specialize(&pipeline_cache, &pipeline, key, &mesh.layout) else { continue; };
            let (vertex_slab, index_slab) = mesh_allocator.mesh_slabs(&mesh_asset_id);
            let batch_set_key = bevy::core_pipeline::core_3d::Opaque3dBatchSetKey {
                pipeline: pipeline_id,
                draw_function,
                material_bind_group_index: None,
                vertex_slab: vertex_slab.unwrap_or_default(),
                index_slab,
                lightmap_slab,
            };
            let bin_key = bevy::core_pipeline::core_3d::Opaque3dBinKey { asset_id: mesh_asset_id.into() };
            phase.add(
                batch_set_key,
                bin_key,
                (*render_entity, *visible_entity),
                current_uniform_index,
                BinnedRenderPhaseType::UnbatchableMesh,
                ticks.this_run(),
            );
            stats.draws = stats.draws.saturating_add(1);
        }
    }

    let _ = _greedy_cfg;

    if stats_cfg.log_every_frames > 0 && stats.frame % stats_cfg.log_every_frames == 0 {
        info!(
            "voxel_faces stats f={} visible_chunks={} dirty_uploaded={} atlas_kb={:.1} neigh_kb={:.1} pal_kb={:.1} est_faces={} disp_faces={} disp_args={} submits={} draws={}",
            stats.frame,
            stats.visible_chunks,
            stats.dirty_chunks_uploaded,
            (stats.atlas_upload_bytes as f64) / 1024.0,
            (stats.neighbor_upload_bytes as f64) / 1024.0,
            (stats.palette_upload_bytes as f64) / 1024.0,
            stats.estimated_face_instances,
            stats.compute_dispatches_faces,
            stats.compute_dispatches_args,
            stats.submits,
            stats.draws,
        );
    }
    if let Ok(mut g) = stats_shared.0.lock() { *g = stats.clone(); };
}

fn ensure_node_atlas(
    atlases: &mut HashMap<Uuid, NodeAtlas>,
    node_id: Uuid,
    needed_slots: u32,
    chunk_dim: u32,
    render_device: &RenderDevice,
    render_queue: &RenderQueue,
) {
    let entry = atlases.entry(node_id).or_insert_with(NodeAtlas::default);
    if entry.atlas_lods.is_empty() {
        entry.atlas_lods = (0..VOXEL_LOD_LEVELS).map(|_| None).collect();
    }
    if entry.lod_steps.is_empty() {
        entry.lod_steps = (0..(VOXEL_LOD_LEVELS - 1)).map(|_| None).collect();
    }
    if entry.capacity >= needed_slots {
        return;
    }
    entry.atlas_gen = entry.atlas_gen.wrapping_add(1);
    // Grow atlas capacity (next power of two)
    let min_cap = if cfg!(target_arch = "wasm32") { 64 } else { 8 };
    let new_capacity = needed_slots.next_power_of_two().max(min_cap);
    let old_lods = entry.atlas_lods.clone();
    let old_neighbor = entry.neighbor_table.clone();
    let old_cap = entry.capacity;

    let mut new_lods: Vec<Option<Buffer>> = Vec::with_capacity(VOXEL_LOD_LEVELS as usize);
    for lod in 0..VOXEL_LOD_LEVELS {
        let d = lod_dim(chunk_dim, lod);
        let voxels_per_slot = (d * d * d) as u64;
        let atlas_size = new_capacity as u64 * voxels_per_slot * 4;
        new_lods.push(Some(render_device.create_buffer(&BufferDescriptor {
            label: Some("voxel_atlas_lod"),
            size: atlas_size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })));
    }
    let neighbor_size = new_capacity as u64 * 6 * 4;
    let new_neighbor = Some(render_device.create_buffer(&BufferDescriptor {
        label: Some("voxel_neighbor_table"),
        size: neighbor_size,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    }));

    // Preserve old contents (slot indices remain stable).
    if old_cap > 0 {
        let mut enc = render_device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("voxel_atlas_grow_copy"),
        });
        for lod in 0..VOXEL_LOD_LEVELS as usize {
            let src = old_lods.get(lod).and_then(|o| o.as_ref()).cloned();
            if let (Some(ref src), Some(ref dst)) = (&src, &new_lods[lod]) {
                let copy_bytes = src.size().min(dst.size());
                if copy_bytes > 0 {
                    enc.copy_buffer_to_buffer(src, 0, dst, 0, copy_bytes);
                }
            }
        }
        if let (Some(ref src), Some(ref dst)) = (&old_neighbor, &new_neighbor) {
            let copy_bytes = src.size().min(dst.size());
            if copy_bytes > 0 {
                enc.copy_buffer_to_buffer(src, 0, dst, 0, copy_bytes);
            }
        }
        render_queue.submit(std::iter::once(enc.finish()));
    }

    entry.atlas_lods = new_lods;
    entry.neighbor_table = new_neighbor;
    entry.capacity = new_capacity;
    for s in entry.lod_steps.iter_mut() {
        *s = None;
    }
}

fn ensure_chunk_output(
    outputs: &mut HashMap<(Uuid, IVec3, u8), GpuChunkOut>,
    render_device: &RenderDevice,
    render_queue: &RenderQueue,
    pipeline: &VoxelFacesPipeline,
    atlas: &Buffer,
    neighbor_table: &Buffer,
    key: (Uuid, IVec3, u8),
    slot: u32,
    chunk_dim: u32,
    max_out: u32,
    atlas_gen: u64,
) {
    let lod = key.2;
    let params = |slot: u32, chunk_dim: u32, max_out: u32| ComputeParams { slot_idx: slot, chunk_dim, max_out, lod: lod as u32 };

    if let Some(cur) = outputs.get_mut(&key) {
        let need_bg_rebuild = cur.atlas_gen != atlas_gen;
        if cur.max_out < max_out {
            cur.faces = render_device.create_buffer(&BufferDescriptor {
                label: Some("voxel_faces_out_faces"),
                size: (std::mem::size_of::<FaceInstance>() as u64) * (max_out as u64),
                usage: BufferUsages::STORAGE | BufferUsages::VERTEX,
                mapped_at_creation: false,
            });
            cur.max_out = max_out;
            cur.bind_group = render_device.create_bind_group(
                "voxel_faces_compute_bg",
                &pipeline.compute_bgl,
                &[
                    BindGroupEntry { binding: 0, resource: cur.params.as_entire_binding() },
                    BindGroupEntry { binding: 1, resource: atlas.as_entire_binding() },
                    BindGroupEntry { binding: 2, resource: neighbor_table.as_entire_binding() },
                    BindGroupEntry { binding: 3, resource: cur.header.as_entire_binding() },
                    BindGroupEntry { binding: 4, resource: cur.faces.as_entire_binding() },
                    BindGroupEntry { binding: 5, resource: cur.args.as_entire_binding() },
                ],
            );
            cur.atlas_gen = atlas_gen;
        } else if need_bg_rebuild {
            cur.bind_group = render_device.create_bind_group(
                "voxel_faces_compute_bg",
                &pipeline.compute_bgl,
                &[
                    BindGroupEntry { binding: 0, resource: cur.params.as_entire_binding() },
                    BindGroupEntry { binding: 1, resource: atlas.as_entire_binding() },
                    BindGroupEntry { binding: 2, resource: neighbor_table.as_entire_binding() },
                    BindGroupEntry { binding: 3, resource: cur.header.as_entire_binding() },
                    BindGroupEntry { binding: 4, resource: cur.faces.as_entire_binding() },
                    BindGroupEntry { binding: 5, resource: cur.args.as_entire_binding() },
                ],
            );
            cur.atlas_gen = atlas_gen;
        }
        cur.slot = slot;
        render_queue.write_buffer(&cur.params, 0, bytemuck::bytes_of(&params(slot, chunk_dim, cur.max_out)));
        return;
    }

    let params_buf = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("voxel_faces_params"),
        contents: bytemuck::bytes_of(&params(slot, chunk_dim, max_out)),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });

    let header = render_device.create_buffer(&BufferDescriptor {
        label: Some("voxel_faces_header"),
        size: std::mem::size_of::<Header>() as u64,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let faces = render_device.create_buffer(&BufferDescriptor {
        label: Some("voxel_faces_out_faces"),
        size: (std::mem::size_of::<FaceInstance>() as u64) * (max_out as u64),
        usage: BufferUsages::STORAGE | BufferUsages::VERTEX,
        mapped_at_creation: false,
    });
    let args = render_device.create_buffer(&BufferDescriptor {
        label: Some("voxel_faces_indirect_args"),
        size: std::mem::size_of::<DrawIndexedIndirect>() as u64,
        usage: BufferUsages::STORAGE | BufferUsages::INDIRECT | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let bind_group = render_device.create_bind_group(
        "voxel_faces_compute_bg",
        &pipeline.compute_bgl,
        &[
            BindGroupEntry { binding: 0, resource: params_buf.as_entire_binding() },
            BindGroupEntry { binding: 1, resource: atlas.as_entire_binding() },
            BindGroupEntry { binding: 2, resource: neighbor_table.as_entire_binding() },
            BindGroupEntry { binding: 3, resource: header.as_entire_binding() },
            BindGroupEntry { binding: 4, resource: faces.as_entire_binding() },
            BindGroupEntry { binding: 5, resource: args.as_entire_binding() },
        ],
    );

    outputs.insert(key, GpuChunkOut {
        slot,
        lod,
        _pad0: [0; 3],
        slot_version: 0,
        atlas_gen,
        params: params_buf,
        header,
        faces,
        args,
        bind_group,
        max_out,
    });
}

const VOXEL_FACES_COMPUTE_WGSL: &str = r#"
// Params: slot_idx = which slot this chunk is in, chunk_dim = CHUNK_SIZE, max_out = max faces
struct Params { slot_idx: u32, chunk_dim: u32, max_out: u32, lod: u32 };
struct Header { count: atomic<u32>, _pad: vec3<u32> };
struct FaceInstance {
  x: i32, y: i32, z: i32, dir: u32,
  pi: u32, lod: u32, span_u: u32, span_v: u32
};
struct DrawIndexedIndirect {
  index_count: u32,
  instance_count: u32,
  first_index: u32,
  base_vertex: i32,
  first_instance: u32
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> atlas: array<u32>;
@group(0) @binding(2) var<storage, read> neighbors: array<i32>;
@group(0) @binding(3) var<storage, read_write> h: Header;
@group(0) @binding(4) var<storage, read_write> out_faces: array<FaceInstance>;
@group(0) @binding(5) var<storage, read_write> out_args: DrawIndexedIndirect;

var<workgroup> scan: array<u32, 256>;
var<workgroup> wg_base: u32;
var<workgroup> wg_total: u32;
var<workgroup> gmask: array<i32, 256>;

// Load voxel from a slot at local coords (x,y,z) in [0..chunk_dim)
fn load_slot(slot: u32, x: i32, y: i32, z: i32) -> u32 {
  let d = i32(p.chunk_dim);
  let voxels_per_slot = u32(d * d * d);
  let local_idx = u32(z * d * d + y * d + x);
  return atlas[slot * voxels_per_slot + local_idx];
}

// Load voxel at local coords, handling boundary by looking at neighbor slots
// dir: 0=+X, 1=-X, 2=+Y, 3=-Y, 4=+Z, 5=-Z
fn load_neighbor_voxel(x: i32, y: i32, z: i32, dir: u32) -> u32 {
  let d = i32(p.chunk_dim);
  var nx = x; var ny = y; var nz = z;
  switch dir {
    case 0u: { nx = x + 1; }
    case 1u: { nx = x - 1; }
    case 2u: { ny = y + 1; }
    case 3u: { ny = y - 1; }
    case 4u: { nz = z + 1; }
    case 5u: { nz = z - 1; }
    default: {}
  }
  // Check if still inside this chunk
  if (nx >= 0 && nx < d && ny >= 0 && ny < d && nz >= 0 && nz < d) {
    return load_slot(p.slot_idx, nx, ny, nz);
  }
  // Outside: look up neighbor slot
  let neighbor_slot = neighbors[p.slot_idx * 6u + dir];
  if (neighbor_slot < 0) {
    return 0u; // No neighbor = air
  }
  // Wrap coords into neighbor chunk
  var wx = nx; var wy = ny; var wz = nz;
  if (nx < 0) { wx = d - 1; } else if (nx >= d) { wx = 0; }
  if (ny < 0) { wy = d - 1; } else if (ny >= d) { wy = 0; }
  if (nz < 0) { wz = d - 1; } else if (nz >= d) { wz = 0; }
  return load_slot(u32(neighbor_slot), wx, wy, wz);
}

@compute @workgroup_size(256)
fn gen_faces(
  @builtin(global_invocation_id) gid: vec3<u32>,
  @builtin(local_invocation_id) lid: vec3<u32>
) {
  let d = i32(p.chunk_dim);
  let n = u32(d * d * d);
  let i = gid.x;
  let tid = lid.x;

  var ix: i32 = 0;
  var iy: i32 = 0;
  var iz: i32 = 0;
  var pi: u32 = 0u;
  var c: u32 = 0u;
  var f0: bool = false;
  var f1: bool = false;
  var f2: bool = false;
  var f3: bool = false;
  var f4: bool = false;
  var f5: bool = false;

  if (i < n) {
    ix = i32(i % u32(d));
    iy = i32((i / u32(d)) % u32(d));
    iz = i32(i / (u32(d) * u32(d)));
    pi = load_slot(p.slot_idx, ix, iy, iz);
    if (pi != 0u) {
      f0 = (load_neighbor_voxel(ix, iy, iz, 0u) == 0u);
      f1 = (load_neighbor_voxel(ix, iy, iz, 1u) == 0u);
      f2 = (load_neighbor_voxel(ix, iy, iz, 2u) == 0u);
      f3 = (load_neighbor_voxel(ix, iy, iz, 3u) == 0u);
      f4 = (load_neighbor_voxel(ix, iy, iz, 4u) == 0u);
      f5 = (load_neighbor_voxel(ix, iy, iz, 5u) == 0u);
      c =
        select(0u, 1u, f0) +
        select(0u, 1u, f1) +
        select(0u, 1u, f2) +
        select(0u, 1u, f3) +
        select(0u, 1u, f4) +
        select(0u, 1u, f5);
    }
  }

  scan[tid] = c;
  workgroupBarrier();

  // Blelloch exclusive scan (256 lanes).
  for (var offset = 1u; offset < 256u; offset = offset * 2u) {
    let idx = (tid + 1u) * offset * 2u - 1u;
    if (idx < 256u) { scan[idx] = scan[idx] + scan[idx - offset]; }
    workgroupBarrier();
  }
  if (tid == 0u) {
    wg_total = scan[255u];
    scan[255u] = 0u;
  }
  workgroupBarrier();
  var offset_i: i32 = 128;
  loop {
    if (offset_i <= 0) { break; }
    let off = u32(offset_i);
    let idx = (tid + 1u) * off * 2u - 1u;
    if (idx < 256u) {
      let t = scan[idx - off];
      scan[idx - off] = scan[idx];
      scan[idx] = scan[idx] + t;
    }
    workgroupBarrier();
    offset_i = offset_i / 2;
  }

  if (tid == 0u && wg_total > 0u) {
    wg_base = atomicAdd(&h.count, wg_total);
  } else if (tid == 0u) {
    wg_base = 0u;
  }
  workgroupBarrier();
  let base = wg_base;

  if (c == 0u || pi == 0u) { return; }
  var out_i = base + scan[tid];
  if (f0) { if (out_i < p.max_out) { out_faces[out_i] = FaceInstance(ix, iy, iz, 0u, pi, p.lod, 1u, 1u); } out_i = out_i + 1u; }
  if (f1) { if (out_i < p.max_out) { out_faces[out_i] = FaceInstance(ix, iy, iz, 1u, pi, p.lod, 1u, 1u); } out_i = out_i + 1u; }
  if (f2) { if (out_i < p.max_out) { out_faces[out_i] = FaceInstance(ix, iy, iz, 2u, pi, p.lod, 1u, 1u); } out_i = out_i + 1u; }
  if (f3) { if (out_i < p.max_out) { out_faces[out_i] = FaceInstance(ix, iy, iz, 3u, pi, p.lod, 1u, 1u); } out_i = out_i + 1u; }
  if (f4) { if (out_i < p.max_out) { out_faces[out_i] = FaceInstance(ix, iy, iz, 4u, pi, p.lod, 1u, 1u); } out_i = out_i + 1u; }
  if (f5) { if (out_i < p.max_out) { out_faces[out_i] = FaceInstance(ix, iy, iz, 5u, pi, p.lod, 1u, 1u); } out_i = out_i + 1u; }
}

fn emit_quad(x: i32, y: i32, z: i32, dir: u32, pi: u32, span_u: u32, span_v: u32) {
  let i = atomicAdd(&h.count, 1u);
  if (i >= p.max_out) { return; }
  out_faces[i] = FaceInstance(x, y, z, dir, pi, p.lod, span_u, span_v);
}

// Greedy on GPU (one workgroup per slice; chunk_dim <= 16).
@compute @workgroup_size(256)
fn gen_greedy(
  @builtin(workgroup_id) wid: vec3<u32>,
  @builtin(local_invocation_id) lid: vec3<u32>
) {
  let d = i32(p.chunk_dim);
  if (d <= 0) { return; }
  let slices = d + 1;
  let total = 3 * slices;
  let idx = i32(wid.x);
  if (idx >= total) { return; }
  let axis = idx / slices;
  let s = idx - axis * slices;

  let mi = i32(lid.x);
  var val: i32 = 0;
  if (mi < 256) {
    let u = mi % 16;
    let v = mi / 16;
    if (u < d && v < d) {
      var a: u32 = 0u;
      var b: u32 = 0u;
      if (axis == 0) {
        if (s == 0) { a = load_neighbor_voxel(0, u, v, 1u); } else { a = load_slot(p.slot_idx, s - 1, u, v); }
        if (s == d) { b = load_neighbor_voxel(d - 1, u, v, 0u); } else { b = load_slot(p.slot_idx, s, u, v); }
      } else if (axis == 1) {
        if (s == 0) { a = load_neighbor_voxel(u, 0, v, 3u); } else { a = load_slot(p.slot_idx, u, s - 1, v); }
        if (s == d) { b = load_neighbor_voxel(u, d - 1, v, 2u); } else { b = load_slot(p.slot_idx, u, s, v); }
      } else {
        if (s == 0) { a = load_neighbor_voxel(u, v, 0, 5u); } else { a = load_slot(p.slot_idx, u, v, s - 1); }
        if (s == d) { b = load_neighbor_voxel(u, v, d - 1, 4u); } else { b = load_slot(p.slot_idx, u, v, s); }
      }
      val = select(select(0, -i32(b), a == 0u && b != 0u), i32(a), a != 0u && b == 0u);
    }
    gmask[mi] = val;
  }
  workgroupBarrier();

  if (lid.x != 0u) { return; }
  for (var y = 0; y < d; y = y + 1) {
    var x = 0;
    loop {
      if (x >= d) { break; }
      let v = gmask[y * 16 + x];
      if (v == 0) { x = x + 1; continue; }
      var ww = 1;
      loop {
        if (x + ww >= d) { break; }
        if (gmask[y * 16 + x + ww] != v) { break; }
        ww = ww + 1;
      }
      var hh = 1;
      loop {
        if (y + hh >= d) { break; }
        var ok = true;
        for (var xx = 0; xx < ww; xx = xx + 1) {
          if (gmask[(y + hh) * 16 + x + xx] != v) { ok = false; }
        }
        if (!ok) { break; }
        hh = hh + 1;
      }

      let pi = u32(select(-v, v, v > 0));
      if (pi != 0u) {
        var cx: i32 = 0; var cy: i32 = 0; var cz: i32 = 0; var dir: u32 = 0u;
        if (axis == 0) {
          if (v > 0) { dir = 0u; cx = s - 1; cy = x; cz = y; }
          else { dir = 1u; cx = s; cy = x; cz = y; }
        } else if (axis == 1) {
          if (v > 0) { dir = 2u; cx = x; cy = s - 1; cz = y; }
          else { dir = 3u; cx = x; cy = s; cz = y; }
        } else {
          if (v > 0) { dir = 4u; cx = x; cy = y; cz = s - 1; }
          else { dir = 5u; cx = x; cy = y; cz = s; }
        }
        emit_quad(cx, cy, cz, dir, pi, u32(ww), u32(hh));
      }

      for (var yy = 0; yy < hh; yy = yy + 1) {
        for (var xx = 0; xx < ww; xx = xx + 1) {
          gmask[(y + yy) * 16 + x + xx] = 0;
        }
      }
      x = x + ww;
    }
  }
}

@compute @workgroup_size(1)
fn build_args(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (gid.x != 0u) { return; }
  let cnt = atomicLoad(&h.count);
  out_args.index_count = 6u;
  out_args.instance_count = min(cnt, p.max_out);
  out_args.first_index = 0u;
  out_args.base_vertex = 0;
  out_args.first_instance = 0u;
}
"#;

const VOXEL_LOD_DOWNSAMPLE_WGSL: &str = r#"
// Downsample src_dim -> dst_dim for all slots (slot-major layout).
struct Params { slot_count: u32, src_dim: u32, dst_dim: u32, _pad0: u32 };
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;

fn idx(slot: u32, dim: u32, x: u32, y: u32, z: u32) -> u32 {
  let vox = dim * dim * dim;
  return slot * vox + z * dim * dim + y * dim + x;
}

@compute @workgroup_size(4, 4, 4)
fn downsample(@builtin(global_invocation_id) gid: vec3<u32>) {
  let x = gid.x;
  let y = gid.y;
  let z_all = gid.z;
  if (x >= p.dst_dim || y >= p.dst_dim) { return; }
  let slot = z_all / p.dst_dim;
  let z = z_all % p.dst_dim;
  if (slot >= p.slot_count || z >= p.dst_dim) { return; }

  let sx = x * 2u;
  let sy = y * 2u;
  let sz = z * 2u;
  var out: u32 = 0u;
  for (var dz = 0u; dz < 2u; dz = dz + 1u) {
    for (var dy = 0u; dy < 2u; dy = dy + 1u) {
      for (var dx = 0u; dx < 2u; dx = dx + 1u) {
        let v = src[idx(slot, p.src_dim, sx + dx, sy + dy, sz + dz)];
        if (out == 0u && v != 0u) { out = v; }
      }
    }
  }
  dst[idx(slot, p.dst_dim, x, y, z)] = out;
}
"#;
