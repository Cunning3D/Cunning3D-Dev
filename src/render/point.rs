use bevy::core_pipeline::{
    oit::OrderIndependentTransparencySettings, prepass::ViewPrepassTextures,
};
use bevy::pbr::ExtractedAtmosphere;
use bevy::{
    core_pipeline::core_3d::{Opaque3d, CORE_3D_DEPTH_FORMAT},
    ecs::system::{lifetimeless::SRes, SystemParamItem},
    mesh::{Mesh, Mesh3d, MeshVertexBufferLayoutRef, VertexBufferLayout, VertexFormat},
    pbr::{
        MeshPipeline, MeshPipelineKey, RenderMeshInstances, SetMeshViewBindGroup,
        SetMeshViewBindingArrayBindGroup,
    },
    prelude::*,
    render::{
        batching::gpu_preprocessing::GpuPreprocessingSupport,
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::RenderAssets,
        render_phase::{
            AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, PhaseItem, RenderCommand,
            RenderCommandResult, SetItemPipeline, TrackedRenderPass, ViewBinnedRenderPhases,
        },
        render_resource::{VertexAttribute, VertexStepMode, *},
        renderer::{RenderDevice, RenderQueue},
        sync_world::RenderEntity,
        view::{ExtractedView, RenderVisibleEntities},
        Extract, Render, RenderApp, RenderSystems,
    },
};

#[derive(Resource, Clone)]
pub struct PointShader(pub Handle<Shader>);

pub struct CunningPointPlugin;

impl Plugin for CunningPointPlugin {
    fn build(&self, app: &mut App) {
        // Load Shader in Main World where Assets<Shader> exists
        let mut shaders = app.world_mut().resource_mut::<Assets<Shader>>();
        let shader = Shader::from_wgsl(
            include_str!("../../assets/shaders/cunning_point_v2.wgsl"),
            "shaders/cunning_point_v2.wgsl",
        );
        let handle = shaders.add(shader);
        app.insert_resource(PointShader(handle));
    }

    fn finish(&self, app: &mut App) {
        // Transfer handle to Render App
        let handle = app.world().resource::<PointShader>().0.clone();

        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.insert_resource(PointShader(handle));

            render_app
                .init_resource::<PointPipeline>()
                .init_resource::<SpecializedMeshPipelines<PointPipeline>>()
                .init_resource::<PointUniforms>()
                .init_resource::<PointQuadBuffer>()
                .add_render_command::<Opaque3d, DrawPointPipeline>()
                .add_systems(ExtractSchedule, extract_points)
                .add_systems(
                    Render,
                    prepare_point_uniforms.in_set(RenderSystems::PrepareBindGroups),
                )
                .add_systems(Render, queue_points.in_set(RenderSystems::Queue));
        }
    }
}

#[derive(Component)]
pub struct RenderPoint {
    pub mesh_handle: Handle<Mesh>,
    pub transform: GlobalTransform,
}

#[derive(Component, Clone, Copy, Debug)]
pub struct PointMarker;

pub fn extract_points(
    mut commands: Commands,
    query: Extract<
        Query<(RenderEntity, &Mesh3d, &ViewVisibility, &GlobalTransform), With<PointMarker>>,
    >,
) {
    for (render_entity, mesh, visibility, transform) in &query {
        if !visibility.get() {
            continue;
        }
        commands.entity(render_entity).insert(RenderPoint {
            mesh_handle: mesh.0.clone(),
            transform: *transform,
        });
    }
}

#[derive(Resource)]
pub struct PointPipeline {
    pub shader: Handle<Shader>,
    pub mesh_pipeline: MeshPipeline,
    pub uniform_layout: BindGroupLayout,
    pub uniform_layout_desc: bevy::render::render_resource::BindGroupLayoutDescriptor,
}

impl FromWorld for PointPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>().clone();
        let shader = world.resource::<PointShader>().0.clone();
        let mesh_pipeline = world.resource::<MeshPipeline>().clone();

        let entries = [BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::VERTEX,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: Some(std::num::NonZeroU64::new(64).unwrap()),
            },
            count: None,
        }];
        let uniform_layout =
            render_device.create_bind_group_layout("point_uniform_layout", &entries);
        let uniform_layout_desc = bevy::render::render_resource::BindGroupLayoutDescriptor::new(
            "point_uniform_layout",
            &entries,
        );

        PointPipeline {
            shader,
            mesh_pipeline,
            uniform_layout,
            uniform_layout_desc,
        }
    }
}

impl SpecializedMeshPipeline for PointPipeline {
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
                depth_write_enabled: false,
                depth_compare: CompareFunction::GreaterEqual,
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            });
        }

        // Bevy 0.18 MeshPipeline bind groups: 0=view(main), 1=view(binding array), 2=mesh
        // Use group 2 for our custom point uniform.
        descriptor.set_layout(2, self.uniform_layout_desc.clone());

        let mut instance_layout = layout
            .0
            .get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(2)])?;
        instance_layout.step_mode = VertexStepMode::Instance;

        descriptor.vertex.buffers.clear();
        descriptor.vertex.buffers.push(instance_layout);

        // 2. Quad Buffer (Standard) as slot 1
        descriptor.vertex.buffers.push(VertexBufferLayout {
            array_stride: 20,
            step_mode: VertexStepMode::Vertex,
            attributes: vec![
                VertexAttribute {
                    format: VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                }, // Quad Pos
                VertexAttribute {
                    format: VertexFormat::Float32x2,
                    offset: 12,
                    shader_location: 1,
                }, // Quad UV
            ],
        });

        descriptor.primitive.topology = PrimitiveTopology::TriangleList; // We are drawing Quads (Triangles)
        descriptor.primitive.cull_mode = None;

        // Zero-Copy Magic: Depth Bias to draw points on top of surfaces
        // WebGL Fix: Remove aggressive depth bias for now to debug visibility
        // Bevy 0.18 Reverse Z: Positive bias brings it closer.
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_write_enabled = false;
            ds.bias = bevy::render::render_resource::DepthBiasState {
                constant: 50,
                slope_scale: 1.0,
                clamp: 0.0,
            };
        }

        Ok(descriptor)
    }
}

#[derive(ShaderType)]
pub struct PointUniform {
    pub model: Mat4,
}

#[derive(Resource, Default)]
pub struct PointUniforms {
    pub buffer: DynamicUniformBuffer<PointUniform>,
    pub bind_group: Option<BindGroup>,
}

#[derive(Component)]
pub struct PointUniformOffset {
    pub offset: u32,
}

pub fn prepare_point_uniforms(
    mut commands: Commands,
    mut point_uniforms: ResMut<PointUniforms>,
    render_points: Query<(Entity, &RenderPoint)>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    point_pipeline: Res<PointPipeline>,
) {
    point_uniforms.buffer.clear();

    let mut _count = 0;
    for (entity, point) in render_points.iter() {
        let transform = point.transform.to_matrix();
        let offset = point_uniforms
            .buffer
            .push(&PointUniform { model: transform });

        commands.entity(entity).insert(PointUniformOffset {
            offset: offset as u32,
        });
        _count += 1;
    }

    point_uniforms
        .buffer
        .write_buffer(&render_device, &render_queue);

    if let Some(buffer) = point_uniforms.buffer.binding() {
        let bind_group = render_device.create_bind_group(
            "point_bind_group",
            &point_pipeline.uniform_layout,
            &[BindGroupEntry {
                binding: 0,
                resource: buffer,
            }],
        );
        point_uniforms.bind_group = Some(bind_group);
    }
}

// Custom Render Command to bind our new Group 1
pub struct SetPointBindGroup<const I: usize>;
impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetPointBindGroup<I> {
    type Param = (SRes<PointUniforms>,);
    type ViewQuery = ();
    type ItemQuery = &'static PointUniformOffset;

    fn render<'w>(
        _item: &P,
        _view: (),
        offset: Option<&'w PointUniformOffset>,
        (point_uniforms,): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(offset) = offset else {
            return RenderCommandResult::Failure("missing PointUniformOffset");
        };
        let point_uniforms = point_uniforms.into_inner();
        let Some(bind_group) = point_uniforms.bind_group.as_ref() else {
            return RenderCommandResult::Failure("missing PointUniforms bind_group");
        };

        pass.set_bind_group(I, bind_group, &[offset.offset]);
        RenderCommandResult::Success
    }
}

pub struct DrawPoint;

impl<P: PhaseItem> RenderCommand<P> for DrawPoint {
    type Param = (
        SRes<RenderAssets<RenderMesh>>,
        SRes<MeshAllocator>,
        SRes<PointQuadBuffer>,
    );
    type ViewQuery = ();
    type ItemQuery = &'static RenderPoint;

    fn render<'w>(
        _item: &P,
        _view: (),
        render_point: Option<&'w RenderPoint>,
        (meshes, mesh_allocator, quad_buffer): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(render_point) = render_point else {
            return RenderCommandResult::Failure("missing RenderPoint");
        };
        let meshes = meshes.into_inner();
        let mesh_allocator = mesh_allocator.into_inner();
        let mesh_id = render_point.mesh_handle.id();
        let Some(render_mesh) = meshes.get(mesh_id) else {
            return RenderCommandResult::Failure("mesh not prepared");
        };
        let Some(vs) = mesh_allocator.mesh_vertex_slice(&mesh_id) else {
            return RenderCommandResult::Success;
        };
        let quad_buffer = quad_buffer.into_inner();

        // 1. Bind Instance Buffer (Main Mesh Position)
        let stride = render_mesh.layout.0.layout().array_stride as u64;
        let start = vs.range.start as u64 * stride;
        let end = vs.range.end as u64 * stride;
        pass.set_vertex_buffer(0, vs.buffer.slice(start..end));

        // 2. Bind Quad Buffer (Vertices + Indices)
        pass.set_vertex_buffer(1, quad_buffer.vertex_buffer.slice(..));
        pass.set_index_buffer(quad_buffer.index_buffer.slice(..), IndexFormat::Uint16);

        // 3. Draw
        // 6 indices for Quad, N instances for Points
        pass.draw_indexed(0..6, 0, 0..render_mesh.vertex_count);

        RenderCommandResult::Success
    }
}

pub type DrawPointPipeline = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetPointBindGroup<2>, // Our custom group
    DrawPoint,
);

#[derive(Resource)]
pub struct PointQuadBuffer {
    pub vertex_buffer: Buffer,
    pub index_buffer: Buffer,
}

impl FromWorld for PointQuadBuffer {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();

        // Quad Vertices (Pos + UV)
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct QuadVertex {
            pos: [f32; 3],
            uv: [f32; 2],
        }

        let vertices = [
            QuadVertex {
                pos: [-1.0, -1.0, 0.0],
                uv: [0.0, 1.0],
            },
            QuadVertex {
                pos: [1.0, -1.0, 0.0],
                uv: [1.0, 1.0],
            },
            QuadVertex {
                pos: [1.0, 1.0, 0.0],
                uv: [1.0, 0.0],
            },
            QuadVertex {
                pos: [-1.0, 1.0, 0.0],
                uv: [0.0, 0.0],
            },
        ];

        let indices: [u16; 6] = [0, 1, 2, 2, 3, 0];

        let vertex_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("Point Quad Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: BufferUsages::VERTEX,
        });

        let index_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("Point Quad Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: BufferUsages::INDEX,
        });

        PointQuadBuffer {
            vertex_buffer,
            index_buffer,
        }
    }
}

pub fn queue_points(
    opaque_3d_draw_functions: Res<DrawFunctions<Opaque3d>>,
    point_pipeline: Res<PointPipeline>,
    mut pipelines: ResMut<SpecializedMeshPipelines<PointPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    render_mesh_instances: Res<RenderMeshInstances>,
    mesh_allocator: Res<MeshAllocator>,
    _gpu_preprocessing_support: Res<GpuPreprocessingSupport>,
    ticks: bevy::ecs::system::SystemChangeTick,
    render_points: Query<&RenderPoint>,
    mut opaque_render_phases: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    views: Query<(
        &ExtractedView,
        &Msaa,
        &RenderVisibleEntities,
        Option<&ViewPrepassTextures>,
        bevy::ecs::query::Has<OrderIndependentTransparencySettings>,
        bevy::ecs::query::Has<ExtractedAtmosphere>,
    )>,
) {
    let draw_function = opaque_3d_draw_functions
        .read()
        .get_id::<DrawPointPipeline>()
        .unwrap();

    for (view, msaa, visible, prepass_textures, has_oit, has_atmosphere) in views.iter() {
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
        let Some(phase) = opaque_render_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };

        for (render_entity, visible_entity) in visible.iter::<Mesh3d>() {
            if render_points.get(*render_entity).is_err() {
                continue;
            }

            let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(*visible_entity)
            else {
                continue;
            };
            let Some(mesh) = render_meshes.get(mesh_instance.mesh_asset_id) else {
                continue;
            };
            let key = view_key
                | MeshPipelineKey::from_primitive_topology(PrimitiveTopology::TriangleList);
            let Ok(pipeline) =
                pipelines.specialize(&pipeline_cache, &point_pipeline, key, &mesh.layout)
            else {
                continue;
            };
            let (vertex_slab, index_slab) = mesh_allocator.mesh_slabs(&mesh_instance.mesh_asset_id);
            let batch_set_key = bevy::core_pipeline::core_3d::Opaque3dBatchSetKey {
                pipeline,
                draw_function,
                material_bind_group_index: None,
                vertex_slab: vertex_slab.unwrap_or_default(),
                index_slab,
                lightmap_slab: mesh_instance.shared.lightmap_slab_index.map(|i| *i),
            };
            let bin_key = bevy::core_pipeline::core_3d::Opaque3dBinKey {
                asset_id: mesh_instance.mesh_asset_id.into(),
            };
            phase.add(
                batch_set_key,
                bin_key,
                (*render_entity, *visible_entity),
                mesh_instance.current_uniform_index,
                // Use UnbatchableMesh for custom instance rendering
                BinnedRenderPhaseType::UnbatchableMesh,
                ticks.this_run(),
            );
        }
    }
}
