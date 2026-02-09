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
use std::sync::atomic::{AtomicU32, Ordering};

static LAST_EXTRACT_NORMALS: AtomicU32 = AtomicU32::new(u32::MAX);
static LAST_QUEUE_NORMALS: AtomicU32 = AtomicU32::new(u32::MAX);

#[derive(Resource, Clone)]
pub struct NormalShader(pub Handle<Shader>);

pub struct CunningNormalPlugin;

impl Plugin for CunningNormalPlugin {
    fn build(&self, app: &mut App) {
        // Load Shader in Main World
        let mut shaders = app.world_mut().resource_mut::<Assets<Shader>>();
        let shader = Shader::from_wgsl(
            include_str!("../../assets/shaders/cunning_normal_v2.wgsl"),
            "shaders/cunning_normal_v2.wgsl",
        );
        let handle = shaders.add(shader);
        app.insert_resource(NormalShader(handle));
    }

    fn finish(&self, app: &mut App) {
        // Transfer handle to Render App
        let handle = app.world().resource::<NormalShader>().0.clone();

        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.insert_resource(NormalShader(handle));

            render_app
                .init_resource::<NormalPipeline>()
                .init_resource::<SpecializedMeshPipelines<NormalPipeline>>()
                .init_resource::<NormalUniforms>()
                .init_resource::<NormalTriangleBuffer>()
                .add_render_command::<Opaque3d, DrawNormalPipeline>()
                .add_systems(ExtractSchedule, extract_normals)
                .add_systems(
                    Render,
                    prepare_normal_uniforms.in_set(RenderSystems::PrepareBindGroups),
                )
                .add_systems(Render, queue_normals.in_set(RenderSystems::Queue));
        }
    }
}

#[derive(Component)]
pub struct RenderNormal {
    pub mesh_handle: Handle<Mesh>,
    pub transform: GlobalTransform,
    pub color: Color,
}

#[derive(Component, Clone, Copy, Debug)]
pub struct NormalMarker;

#[derive(Component, Clone, Copy, Debug)]
pub struct NormalColor(pub Color);

pub fn extract_normals(
    mut commands: Commands,
    query: Extract<
        Query<
            (
                Entity,
                Option<RenderEntity>,
                &Mesh3d,
                Option<&ViewVisibility>,
                Option<&GlobalTransform>,
                Option<&NormalColor>,
            ),
            With<NormalMarker>,
        >,
    >,
) {
    let (mut total, mut missing_render_entity, mut missing_view_vis, mut visible, mut inserted) =
        (0u32, 0u32, 0u32, 0u32, 0u32);
    let mut missing_global = 0u32;
    for (_main_entity, render_entity, mesh, view_vis, transform, color) in &query {
        total += 1;
        let Some(render_entity) = render_entity else {
            missing_render_entity += 1;
            continue;
        };
        let Some(view_vis) = view_vis else {
            missing_view_vis += 1;
            continue;
        };
        let Some(transform) = transform else {
            missing_global += 1;
            continue;
        };
        if !view_vis.get() {
            continue;
        }
        visible += 1;
        commands.entity(render_entity).insert(RenderNormal {
            mesh_handle: mesh.0.clone(),
            transform: *transform,
            color: color.map(|c| c.0).unwrap_or(Color::WHITE),
        });
        inserted += 1;
    }
    let packed = total
        | (missing_render_entity << 8)
        | (missing_view_vis << 16)
        | (missing_global << 20)
        | (inserted << 24);
    let prev = LAST_EXTRACT_NORMALS.swap(packed, Ordering::Relaxed);
    if prev != packed {
        info!(
            "[Normal] extract_normals total={} miss_render_entity={} miss_view_vis={} miss_global={} visible={} inserted={}",
            total, missing_render_entity, missing_view_vis, missing_global, visible, inserted
        );
    }
}

#[derive(Resource)]
pub struct NormalPipeline {
    pub shader: Handle<Shader>,
    pub mesh_pipeline: MeshPipeline,
    pub uniform_layout: BindGroupLayout,
    pub uniform_layout_desc: bevy::render::render_resource::BindGroupLayoutDescriptor,
}

impl FromWorld for NormalPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>().clone();
        let shader = world.resource::<NormalShader>().0.clone();
        let mesh_pipeline = world.resource::<MeshPipeline>().clone();

        let entries = [BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::VERTEX,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: Some(std::num::NonZeroU64::new(80).unwrap()),
            },
            count: None,
        }];
        let uniform_layout =
            render_device.create_bind_group_layout("normal_uniform_layout", &entries);
        let uniform_layout_desc = bevy::render::render_resource::BindGroupLayoutDescriptor::new(
            "normal_uniform_layout",
            &entries,
        );

        NormalPipeline {
            shader,
            mesh_pipeline,
            uniform_layout,
            uniform_layout_desc,
        }
    }
}

impl SpecializedMeshPipeline for NormalPipeline {
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

        // Replace Bind Group 1 (Mesh) with our Custom Uniform Layout
        // Bevy 0.18 MeshPipeline bind groups: 0=view(main), 1=view(binding array), 2=mesh
        // Use group 2 for our custom normal uniform.
        descriptor.set_layout(2, self.uniform_layout_desc.clone());

        // Use Bevy layout lookup; if mesh lacks NORMAL, skip by returning MissingVertexAttributeError.
        let mut instance_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(1),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(2),
        ])?;
        instance_layout.step_mode = VertexStepMode::Instance;

        descriptor.vertex.buffers.clear();
        descriptor.vertex.buffers.push(instance_layout);

        // 2. Triangle Buffer (Base Width, Length Factor) as slot 1
        descriptor.vertex.buffers.push(VertexBufferLayout {
            array_stride: 8, // f32x2
            step_mode: VertexStepMode::Vertex,
            attributes: vec![VertexAttribute {
                format: VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            }],
        });

        descriptor.primitive.topology = PrimitiveTopology::TriangleList;
        descriptor.primitive.cull_mode = None;

        // Match the core 3d pass depth attachment to avoid wgpu validation errors.
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_write_enabled = false;
            ds.depth_compare = CompareFunction::GreaterEqual;
        }

        Ok(descriptor)
    }
}

#[derive(ShaderType)]
pub struct NormalUniform {
    pub model: Mat4,
    pub color: Vec4,
}

#[derive(Resource, Default)]
pub struct NormalUniforms {
    pub buffer: DynamicUniformBuffer<NormalUniform>,
    pub bind_group: Option<BindGroup>,
}

#[derive(Component)]
pub struct NormalUniformOffset {
    pub offset: u32,
}

pub fn prepare_normal_uniforms(
    mut commands: Commands,
    mut normal_uniforms: ResMut<NormalUniforms>,
    render_normals: Query<(Entity, &RenderNormal)>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    normal_pipeline: Res<NormalPipeline>,
) {
    normal_uniforms.buffer.clear();

    for (entity, normal) in render_normals.iter() {
        let transform = normal.transform.to_matrix();
        let c = normal.color.to_linear();
        let color_linear = Vec4::new(c.red, c.green, c.blue, c.alpha);

        let offset = normal_uniforms.buffer.push(&NormalUniform {
            model: transform,
            color: color_linear,
        });

        commands.entity(entity).insert(NormalUniformOffset {
            offset: offset as u32,
        });
    }

    normal_uniforms
        .buffer
        .write_buffer(&render_device, &render_queue);

    if let Some(buffer) = normal_uniforms.buffer.binding() {
        let bind_group = render_device.create_bind_group(
            "normal_bind_group",
            &normal_pipeline.uniform_layout,
            &[BindGroupEntry {
                binding: 0,
                resource: buffer,
            }],
        );
        normal_uniforms.bind_group = Some(bind_group);
    }
}

// Custom Render Command to bind our new Group 1
pub struct SetNormalBindGroup<const I: usize>;
impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetNormalBindGroup<I> {
    type Param = (SRes<NormalUniforms>,);
    type ViewQuery = ();
    type ItemQuery = &'static NormalUniformOffset;

    fn render<'w>(
        _item: &P,
        _view: (),
        offset: Option<&'w NormalUniformOffset>,
        (normal_uniforms,): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(offset) = offset else {
            return RenderCommandResult::Failure("missing NormalUniformOffset");
        };
        let normal_uniforms = normal_uniforms.into_inner();
        let Some(bind_group) = normal_uniforms.bind_group.as_ref() else {
            return RenderCommandResult::Failure("missing NormalUniforms bind_group");
        };

        pass.set_bind_group(I, bind_group, &[offset.offset]);
        RenderCommandResult::Success
    }
}

pub struct DrawNormal;

impl<P: PhaseItem> RenderCommand<P> for DrawNormal {
    type Param = (
        SRes<RenderAssets<RenderMesh>>,
        SRes<MeshAllocator>,
        SRes<NormalTriangleBuffer>,
    );
    type ViewQuery = ();
    type ItemQuery = &'static RenderNormal;

    fn render<'w>(
        _item: &P,
        _view: (),
        render_normal: Option<&'w RenderNormal>,
        (meshes, mesh_allocator, triangle_buffer): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(render_normal) = render_normal else {
            return RenderCommandResult::Failure("missing RenderNormal");
        };
        let meshes = meshes.into_inner();
        let mesh_allocator = mesh_allocator.into_inner();
        let mesh_id = render_normal.mesh_handle.id();
        let Some(render_mesh) = meshes.get(mesh_id) else {
            return RenderCommandResult::Failure("mesh not prepared");
        };
        let Some(vs) = mesh_allocator.mesh_vertex_slice(&mesh_id) else {
            return RenderCommandResult::Success;
        };
        let triangle_buffer = triangle_buffer.into_inner();

        // 1. Bind Instance Buffer (Main Mesh Position + Normal)
        let stride = render_mesh.layout.0.layout().array_stride as u64;
        let start = vs.range.start as u64 * stride;
        let end = vs.range.end as u64 * stride;
        pass.set_vertex_buffer(0, vs.buffer.slice(start..end));

        // 2. Bind Triangle Buffer (Vertices)
        pass.set_vertex_buffer(1, triangle_buffer.vertex_buffer.slice(..));

        // 3. Draw
        // 3 vertices for Triangle (Tapered Line), N instances for Normals
        pass.draw(0..3, 0..render_mesh.vertex_count);

        RenderCommandResult::Success
    }
}

pub type DrawNormalPipeline = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetNormalBindGroup<2>, // Our custom group
    DrawNormal,
);

#[derive(Resource)]
pub struct NormalTriangleBuffer {
    pub vertex_buffer: Buffer,
}

impl FromWorld for NormalTriangleBuffer {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();

        // Triangle Buffer: (Width Offset, Length Factor)
        // Base Left:  (-0.5, 0.0)
        // Base Right: ( 0.5, 0.0)
        // Tip:        ( 0.0, 1.0)
        let vertices: [[f32; 2]; 3] = [[-0.5, 0.0], [0.5, 0.0], [0.0, 1.0]];

        let vertex_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("Normal Triangle Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: BufferUsages::VERTEX,
        });

        NormalTriangleBuffer { vertex_buffer }
    }
}

pub fn queue_normals(
    opaque_3d_draw_functions: Res<DrawFunctions<Opaque3d>>,
    normal_pipeline: Res<NormalPipeline>,
    mut pipelines: ResMut<SpecializedMeshPipelines<NormalPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    render_mesh_instances: Res<RenderMeshInstances>,
    mesh_allocator: Res<MeshAllocator>,
    _gpu_preprocessing_support: Res<GpuPreprocessingSupport>,
    ticks: bevy::ecs::system::SystemChangeTick,
    render_normals: Query<&RenderNormal>,
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
        .get_id::<DrawNormalPipeline>()
        .unwrap();
    let mut added = 0u32;

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
            if render_normals.get(*render_entity).is_err() {
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
                pipelines.specialize(&pipeline_cache, &normal_pipeline, key, &mesh.layout)
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
                // Use UnbatchableMesh because we use custom Vertex Buffers (Triangle + Instance)
                // which breaks standard mesh batching logic.
                BinnedRenderPhaseType::UnbatchableMesh,
                ticks.this_run(),
            );
            added += 1;
        }
    }
    let prev = LAST_QUEUE_NORMALS.swap(added, Ordering::Relaxed);
    if prev != added {
        info!("[Normal] queue_normals phase_add={}", added);
    }
}
