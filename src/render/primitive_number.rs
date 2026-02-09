use bevy::core_pipeline::oit::OrderIndependentTransparencySettings;
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::pbr::ExtractedAtmosphere;
use bevy::render::mesh::allocator::SlabId;
use bevy::{
    asset::RenderAssetUsages,
    core_pipeline::core_3d::{Opaque3d, Opaque3dBatchSetKey, Opaque3dBinKey, CORE_3D_DEPTH_FORMAT},
    ecs::system::{lifetimeless::SRes, SystemParamItem},
    mesh::{
        Mesh, Mesh3d, MeshVertexBufferLayout, MeshVertexBufferLayoutRef, VertexBufferLayout,
        VertexFormat,
    },
    pbr::{
        MeshPipeline, MeshPipelineKey, RenderMeshInstances, SetMeshViewBindGroup,
        SetMeshViewBindingArrayBindGroup,
    },
    prelude::*,
    render::{
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::{RenderAsset, RenderAssets},
        render_phase::{
            AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, InputUniformIndex, PhaseItem,
            RenderCommand, RenderCommandResult, SetItemPipeline, TrackedRenderPass,
            ViewBinnedRenderPhases,
        },
        render_resource::{
            BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
            BindGroupLayoutEntry, BindingResource, BindingType, Buffer, BufferBindingType,
            BufferDescriptor, BufferInitDescriptor, BufferUsages, CompareFunction, DepthBiasState,
            DynamicUniformBuffer, Extent3d, IndexFormat, PipelineCache, PrimitiveTopology,
            RenderPipelineDescriptor, SamplerBindingType, ShaderStages, ShaderType,
            SpecializedMeshPipeline, SpecializedMeshPipelineError, SpecializedMeshPipelines,
            TextureDimension, TextureFormat, TextureSampleType, TextureViewDimension,
            VertexAttribute, VertexStepMode,
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::{MainEntity, RenderEntity},
        texture::GpuImage,
        view::{ExtractedView, RenderVisibleEntities},
        Extract, Render, RenderApp, RenderStartup, RenderSystems,
    },
};
use std::sync::atomic::{AtomicU32, Ordering};

// --- Assets ---

#[derive(Resource, Clone)]
pub struct PrimitiveNumberShader(pub Handle<Shader>);

#[derive(Resource, Clone)]
pub struct NumberAtlas(pub Handle<Image>);

// --- Plugin ---

pub struct CunningPrimitiveNumberPlugin;

impl Plugin for CunningPrimitiveNumberPlugin {
    fn build(&self, app: &mut App) {
        // 1. Shader
        let shader_handle = {
            let mut shaders = app.world_mut().resource_mut::<Assets<Shader>>();
            let shader = Shader::from_wgsl(
                include_str!("../../assets/shaders/cunning_primitive_number.wgsl"),
                "shaders/cunning_primitive_number.wgsl",
            );
            shaders.add(shader)
        };
        app.insert_resource(PrimitiveNumberShader(shader_handle));

        // 2. Atlas
        let atlas_handle = {
            let mut images = app.world_mut().resource_mut::<Assets<Image>>();
            let width = 40; // 10 digits * 4 pixels
            let height = 5;
            let mut data = vec![0u8; width * height]; // R8

            // 3x5 font data (1=set, 0=unset)
            let font = [
                [1, 1, 1, 1, 0, 1, 1, 0, 1, 1, 0, 1, 1, 1, 1], // 0
                [0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0], // 1
                [1, 1, 1, 0, 0, 1, 1, 1, 1, 1, 0, 0, 1, 1, 1], // 2
                [1, 1, 1, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 1, 1], // 3
                [1, 0, 1, 1, 0, 1, 1, 1, 1, 0, 0, 1, 0, 0, 1], // 4
                [1, 1, 1, 1, 0, 0, 1, 1, 1, 0, 0, 1, 1, 1, 1], // 5
                [1, 1, 1, 1, 0, 0, 1, 1, 1, 1, 0, 1, 1, 1, 1], // 6
                [1, 1, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1], // 7
                [1, 1, 1, 1, 0, 1, 1, 1, 1, 1, 0, 1, 1, 1, 1], // 8
                [1, 1, 1, 1, 0, 1, 1, 1, 1, 0, 0, 1, 1, 1, 1], // 9
            ];

            for (digit, pixels) in font.iter().enumerate() {
                let offset_x = digit * 4; // Spacing 1 pixel
                let offset_y = 0;
                for r in 0..5 {
                    for c in 0..3 {
                        if pixels[r * 3 + c] == 1 {
                            let x = offset_x + c;
                            let y = offset_y + r;
                            data[y * width + x] = 255;
                        }
                    }
                }
            }

            let image = Image::new(
                Extent3d {
                    width: width as u32,
                    height: height as u32,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                data,
                TextureFormat::R8Unorm,
                RenderAssetUsages::RENDER_WORLD,
            );
            images.add(image)
        };
        app.insert_resource(NumberAtlas(atlas_handle));
    }

    fn finish(&self, app: &mut App) {
        let handle = app.world().resource::<PrimitiveNumberShader>().0.clone();
        let atlas_handle = app.world().resource::<NumberAtlas>().0.clone();

        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.insert_resource(PrimitiveNumberShader(handle));
            render_app.insert_resource(NumberAtlas(atlas_handle));
            render_app
                .init_resource::<PrimitiveNumberPipeline>()
                .init_resource::<SpecializedMeshPipelines<PrimitiveNumberPipeline>>()
                .init_resource::<NumberUniforms>()
                .init_resource::<NumberQuadBuffer>()
                .add_render_command::<Opaque3d, DrawNumberPipeline>()
                .add_systems(ExtractSchedule, extract_primitive_numbers)
                .add_systems(RenderStartup, init_primitive_number_batch_entity)
                .add_systems(
                    Render,
                    prepare_number_uniforms.in_set(RenderSystems::PrepareBindGroups),
                )
                .add_systems(Render, queue_primitive_numbers.in_set(RenderSystems::Queue));
        }
    }
}

#[derive(Resource, Clone, Copy)]
pub(crate) struct PrimitiveNumberBatchEntity(Entity);

fn init_primitive_number_batch_entity(mut commands: Commands) {
    let e = commands.spawn_empty().id();
    commands.insert_resource(PrimitiveNumberBatchEntity(e));
}

// --- Components (Main World) ---

#[derive(Component, Clone, Debug)]
pub struct PrimitiveNumberMarker; // Tag for Entity that wants ID display

#[derive(Component, Clone, Debug)]
pub struct PrimitiveNumberData {
    pub values: Vec<u32>,     // IDs to display
    pub positions: Vec<Vec3>, // World positions
    pub color: Vec4,          // Color for this batch
}

// --- Components (Render World) ---

#[derive(Component)]
pub struct RenderPrimitiveNumber {
    pub values: Vec<u32>,
    pub positions: Vec<Vec3>,
    pub color: Vec4,
}

// --- Extraction ---

pub fn extract_primitive_numbers(
    mut commands: Commands,
    query: Extract<
        Query<(Entity, Option<RenderEntity>, &PrimitiveNumberData), With<PrimitiveNumberMarker>>,
    >,
) {
    static LAST_EXTRACT: AtomicU32 = AtomicU32::new(u32::MAX);
    let (mut total, mut miss_render, mut inserted) = (0u32, 0u32, 0u32);
    for (_main, render_entity, data) in &query {
        total += 1;
        let Some(render_entity) = render_entity else {
            miss_render += 1;
            continue;
        };
        commands
            .entity(render_entity)
            .insert(RenderPrimitiveNumber {
                values: data.values.clone(),
                positions: data.positions.clone(),
                color: data.color,
            });
        inserted += 1;
    }
    let packed = total | (miss_render << 12) | (inserted << 24);
    let prev = LAST_EXTRACT.swap(packed, Ordering::Relaxed);
    if prev != packed {
        info!(
            "[PrimNum] extract total={} miss_render_entity={} inserted={}",
            total, miss_render, inserted
        );
    }
}

// --- Pipeline ---

#[derive(Resource)]
pub struct PrimitiveNumberPipeline {
    pub shader: Handle<Shader>,
    pub mesh_pipeline: MeshPipeline,
    pub uniform_layout: BindGroupLayout,
    pub uniform_layout_desc: BindGroupLayoutDescriptor,
}

impl FromWorld for PrimitiveNumberPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>().clone();
        let shader = world.resource::<PrimitiveNumberShader>().0.clone();
        let mesh_pipeline = world.resource::<MeshPipeline>().clone();

        // Uniform Layout: Group 1 (Was 2)
        // Binding 0: Uniform (Model, Color, Size)
        // Binding 1: Texture (Atlas)
        // Binding 2: Sampler
        let entries = [
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: Some(std::num::NonZeroU64::new(96).unwrap()), // Mat4(64)+Vec4(16)+Vec2(8)+Vec2(8) = 96
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
        ];

        let label = "prim_num_layout";
        let uniform_layout = render_device.create_bind_group_layout(label, &entries);
        let uniform_layout_desc = BindGroupLayoutDescriptor::new(label, &entries);

        PrimitiveNumberPipeline {
            shader,
            mesh_pipeline,
            uniform_layout,
            uniform_layout_desc,
        }
    }
}

impl SpecializedMeshPipeline for PrimitiveNumberPipeline {
    type Key = MeshPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh_pipeline.specialize(key, layout)?;

        descriptor.vertex.shader = self.shader.clone();
        descriptor.fragment.as_mut().unwrap().shader = self.shader.clone();

        // [FIX] Simplify BindGroup Layout
        // Group 0: View (Keep)
        // Group 1: Custom Primitive Number Uniforms (Replace Mesh/Material)
        descriptor.layout.truncate(1);
        descriptor.layout.push(self.uniform_layout_desc.clone()); // [FIX] Use Descriptor

        // Vertex Buffers
        descriptor.vertex.buffers.clear();

        // 1. Instance Buffer (Per Number)
        // We'll construct this manually in prepare/queue
        // Layout: Pos(12) + Value(4) + Count+Scale(8) + Color(16) = 40 bytes
        descriptor.vertex.buffers.push(VertexBufferLayout {
            array_stride: 40,
            step_mode: VertexStepMode::Instance,
            attributes: vec![
                VertexAttribute {
                    format: VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 1,
                }, // Pos
                VertexAttribute {
                    format: VertexFormat::Uint32,
                    offset: 12,
                    shader_location: 2,
                }, // Value
                VertexAttribute {
                    format: VertexFormat::Float32x2,
                    offset: 16,
                    shader_location: 3,
                }, // Count+Scale
                VertexAttribute {
                    format: VertexFormat::Float32x4,
                    offset: 24,
                    shader_location: 4,
                }, // Color
            ],
        });

        // 2. Quad Template Buffer (Per Vertex)
        // Layout: Pos(Vec3, where Z is digit index) -> Stride = 12
        descriptor.vertex.buffers.push(VertexBufferLayout {
            array_stride: 12,
            step_mode: VertexStepMode::Vertex,
            attributes: vec![VertexAttribute {
                format: VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            }],
        });

        descriptor.primitive.topology = PrimitiveTopology::TriangleList;
        descriptor.primitive.cull_mode = None;

        // Depth Bias to prevent Z-fighting
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_write_enabled = false; // [DEBUG] Disable depth write
            ds.depth_compare = CompareFunction::Always; // [DEBUG] Always draw on top
            ds.bias = DepthBiasState::default();
        }

        Ok(descriptor)
    }
}

// --- Uniforms & Buffers ---

#[derive(ShaderType)]
pub struct NumberUniform {
    pub model: Mat4,
    pub color: Vec4, // Deprecated in favor of instance color, but kept for struct padding/compat
    pub font_texture_size: Vec2,
    pub glyph_size: Vec2,
}

#[derive(Resource, Default)]
pub struct NumberUniforms {
    pub buffer: DynamicUniformBuffer<NumberUniform>,
    pub bind_group: Option<BindGroup>,
}

#[derive(Resource)]
pub struct NumberQuadBuffer {
    pub vertex_buffer: Buffer,
    pub index_buffer: Buffer,
    pub instance_buffer: Buffer, // Dynamic growable buffer? For now fixed max or re-create
    pub instance_count: u32,
    pub capacity: usize,
}

impl FromWorld for NumberQuadBuffer {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();

        // Template Vertices: 6 Quads for up to 6 digits (0..999999)
        // Each Quad has 4 verts. Total 24 verts.
        // Pos.z stores the digit index (0..5)
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let max_digits = 6;

        for i in 0..max_digits {
            let z = i as f32;
            let base_v = (i * 4) as u16;
            // 0: 0,0  1: 1,0  2: 1,1  3: 0,1
            vertices.push([0.0, 0.0, z]);
            vertices.push([1.0, 0.0, z]);
            vertices.push([1.0, 1.0, z]);
            vertices.push([0.0, 1.0, z]);

            indices.push(base_v + 0);
            indices.push(base_v + 1);
            indices.push(base_v + 2);
            indices.push(base_v + 2);
            indices.push(base_v + 3);
            indices.push(base_v + 0);
        }

        let vertex_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("Number Template VB"),
            contents: bytemuck::cast_slice(&vertices),
            usage: BufferUsages::VERTEX,
        });

        let index_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("Number Template IB"),
            contents: bytemuck::cast_slice(&indices),
            usage: BufferUsages::INDEX,
        });

        // Initial small instance buffer
        let initial_cap = 1024;
        let instance_buffer = render_device.create_buffer(&BufferDescriptor {
            label: Some("Number Instance Buffer"),
            size: (initial_cap * 40) as u64, // Stride 40
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        NumberQuadBuffer {
            vertex_buffer,
            index_buffer,
            instance_buffer,
            instance_count: 0,
            capacity: initial_cap,
        }
    }
}

// --- Prepare & Queue ---

#[derive(Component)]
pub struct NumberUniformOffset {
    pub offset: u32,
}

pub fn prepare_number_uniforms(
    mut commands: Commands,
    mut uniforms: ResMut<NumberUniforms>,
    mut quad_buffer: ResMut<NumberQuadBuffer>,
    render_items: Query<(Entity, &RenderPrimitiveNumber)>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline: Res<PrimitiveNumberPipeline>,
    atlas: Res<NumberAtlas>,
    gpu_images: Res<RenderAssets<GpuImage>>,
) {
    static LAST_PREPARE: AtomicU32 = AtomicU32::new(u32::MAX);
    uniforms.buffer.clear();
    uniforms.bind_group = None;

    let mut all_instances = Vec::new();
    let mut items = 0u32;

    // Push ONE uniform (Identity) for everyone to share (View/Proj handled by View bindgroup)
    let uniform_offset = uniforms.buffer.push(&NumberUniform {
        model: Mat4::IDENTITY,
        color: Vec4::ONE, // Unused by shader now
        font_texture_size: Vec2::new(40.0, 5.0),
        glyph_size: Vec2::new(3.0, 5.0),
    });

    for (entity, item) in render_items.iter() {
        items += 1;
        // Tag entity with the uniform offset (shared)
        commands.entity(entity).insert(NumberUniformOffset {
            offset: uniform_offset as u32,
        });

        for (i, &val) in item.values.iter().enumerate() {
            if i >= item.positions.len() {
                break;
            }
            let pos = item.positions[i];

            // Calc digits
            let digits = if val == 0 {
                1
            } else {
                (val as f32).log10().floor() as u32 + 1
            };
            let scale = 1.0; // [DEBUG] HUGE SCALE

            // Write to instance struct bytes
            // Layout: Pos(12) + Value(4) + Count+Scale(8) + Color(16) = 40 bytes
            all_instances.extend_from_slice(bytemuck::cast_slice(&[pos.x, pos.y, pos.z]));
            all_instances.extend_from_slice(bytemuck::cast_slice(&[val]));
            all_instances.extend_from_slice(bytemuck::cast_slice(&[digits as f32, scale]));
            all_instances.extend_from_slice(bytemuck::cast_slice(&[
                item.color.x,
                item.color.y,
                item.color.z,
                item.color.w,
            ]));
        }
    }

    uniforms.buffer.write_buffer(&render_device, &render_queue);

    // Update Instance Buffer
    if all_instances.len() > 0 {
        let count = all_instances.len() / 40; // 40 bytes per instance
        quad_buffer.instance_count = count as u32;

        if count > quad_buffer.capacity {
            quad_buffer.capacity = count.max(quad_buffer.capacity * 2);
            quad_buffer.instance_buffer = render_device.create_buffer(&BufferDescriptor {
                label: Some("Number Instance Buffer Resized"),
                size: (quad_buffer.capacity * 40) as u64,
                usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        render_queue.write_buffer(&quad_buffer.instance_buffer, 0, &all_instances);
    } else {
        quad_buffer.instance_count = 0;
    }

    // Create Bind Group
    let got_gpu_image = gpu_images.get(&atlas.0).is_some();
    if let Some(gpu_image) = gpu_images.get(&atlas.0) {
        if let Some(buffer_binding) = uniforms.buffer.binding() {
            let bind_group = render_device.create_bind_group(
                "prim_num_bind_group",
                &pipeline.uniform_layout,
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: buffer_binding,
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(&gpu_image.texture_view),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::Sampler(&gpu_image.sampler),
                    },
                ],
            );
            uniforms.bind_group = Some(bind_group);
        }
    }
    let packed = items
        | ((quad_buffer.instance_count.min(4095)) << 12)
        | ((got_gpu_image as u32) << 24)
        | ((uniforms.bind_group.is_some() as u32) << 25);
    let prev = LAST_PREPARE.swap(packed, Ordering::Relaxed);
    if prev != packed {
        info!(
            "[PrimNum] prepare items={} instances={} gpu_image={} bind_group={}",
            items,
            quad_buffer.instance_count,
            got_gpu_image,
            uniforms.bind_group.is_some()
        );
    }
}

pub fn queue_primitive_numbers(
    opaque_3d_draw_functions: Res<DrawFunctions<Opaque3d>>,
    pipeline: Res<PrimitiveNumberPipeline>,
    mut pipelines: ResMut<SpecializedMeshPipelines<PrimitiveNumberPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    mut opaque_render_phases: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    views: Query<(
        &ExtractedView,
        &Msaa,
        Option<&ViewPrepassTextures>,
        bevy::ecs::query::Has<OrderIndependentTransparencySettings>,
        bevy::ecs::query::Has<ExtractedAtmosphere>,
    )>,
    quad_buffer: Res<NumberQuadBuffer>,
    batch_entity: Res<PrimitiveNumberBatchEntity>,
    ticks: bevy::ecs::system::SystemChangeTick,
) {
    if quad_buffer.instance_count == 0 {
        return;
    }

    let draw_function = opaque_3d_draw_functions
        .read()
        .get_id::<DrawNumberPipeline>()
        .unwrap();
    static LAST_QUEUE: AtomicU32 = AtomicU32::new(u32::MAX);
    let mut added = 0u32;

    for (view, msaa, prepass_textures, has_oit, has_atmosphere) in views.iter() {
        let mut view_key = MeshPipelineKey::from_msaa_samples(msaa.samples())
            | MeshPipelineKey::from_hdr(view.hdr);
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

        // [FIXED] Provide buffer layout to new()
        let dummy_buffer_layout = VertexBufferLayout {
            array_stride: 0,
            step_mode: VertexStepMode::Vertex,
            attributes: vec![],
        };
        let layout = MeshVertexBufferLayoutRef(std::sync::Arc::new(MeshVertexBufferLayout::new(
            vec![],
            dummy_buffer_layout,
        )));

        let Ok(pipeline_id) = pipelines.specialize(&pipeline_cache, &pipeline, view_key, &layout)
        else {
            continue;
        };

        let Some(phase) = opaque_render_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };

        let draw_entity = batch_entity.0;

        phase.add(
            Opaque3dBatchSetKey {
                pipeline: pipeline_id,
                draw_function,
                material_bind_group_index: None,
                vertex_slab: SlabId::default(),
                index_slab: None,
                lightmap_slab: None,
            },
            Opaque3dBinKey {
                asset_id: bevy::asset::AssetId::<Mesh>::default().untyped(),
            },
            (draw_entity, MainEntity::from(Entity::PLACEHOLDER)),
            InputUniformIndex(0),
            BinnedRenderPhaseType::UnbatchableMesh,
            ticks.this_run(),
        );
        added += 1;
    }
    let prev = LAST_QUEUE.swap(added, Ordering::Relaxed);
    if prev != added {
        info!("[PrimNum] queue phase_add={}", added);
    }
}

// --- Render Command ---

pub struct SetNumberBindGroup<const I: usize>;
impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetNumberBindGroup<I> {
    type Param = (SRes<NumberUniforms>,);
    type ViewQuery = ();
    type ItemQuery = &'static NumberUniformOffset;

    fn render<'w>(
        _item: &P,
        _view: (),
        offset: Option<&'w NumberUniformOffset>,
        (uniforms,): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        // ... Bind Group Logic ...
        // We need to create bind group if not exists.
        // For now, fail if not exists.
        let uniforms = uniforms.into_inner();
        if let Some(bg) = &uniforms.bind_group {
            pass.set_bind_group(I, bg, &[offset.map(|o| o.offset).unwrap_or(0)]);
            RenderCommandResult::Success
        } else {
            RenderCommandResult::Failure("No Number BindGroup")
        }
    }
}

pub struct DrawNumberBatched;
impl<P: PhaseItem> RenderCommand<P> for DrawNumberBatched {
    type Param = (SRes<NumberQuadBuffer>,);
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: (),
        _: Option<()>,
        (buffer,): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let buffer = buffer.into_inner();
        if buffer.instance_count == 0 {
            return RenderCommandResult::Success;
        }

        pass.set_vertex_buffer(0, buffer.instance_buffer.slice(..));
        pass.set_vertex_buffer(1, buffer.vertex_buffer.slice(..));
        pass.set_index_buffer(buffer.index_buffer.slice(..), IndexFormat::Uint16);

        // 6 digits * 6 indices = 36 indices max per instance
        // Actually we draw 6 quads?
        // Indices are for 6 quads = 36 indices.
        pass.draw_indexed(0..36, 0, 0..buffer.instance_count);

        RenderCommandResult::Success
    }
}

pub type DrawNumberPipeline = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetNumberBindGroup<1>, // [FIX] Now at Group 1
    DrawNumberBatched,
);
