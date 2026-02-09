use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::oit::OrderIndependentTransparencySettings;
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::ecs::system::SystemChangeTick;
use bevy::{
    core_pipeline::core_3d::{Opaque3d, Opaque3dBatchSetKey, Opaque3dBinKey},
    ecs::system::{lifetimeless::SRes, SystemParamItem},
    mesh::{Mesh3d, MeshVertexBufferLayoutRef},
    pbr::{
        ExtractedAtmosphere, MeshPipeline, MeshPipelineKey, RenderMeshInstances, SetMeshBindGroup,
        SetMeshViewBindGroup, SetMeshViewBindingArrayBindGroup,
    },
    prelude::*,
    render::{
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::{PrepareAssetError, RenderAsset, RenderAssetPlugin, RenderAssets},
        render_phase::{
            AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, PhaseItem, RenderCommand,
            RenderCommandResult, SetItemPipeline, TrackedRenderPass, ViewBinnedRenderPhases,
        },
        render_resource::{
            BlendState, Buffer, BufferInitDescriptor, BufferUsages, CompareFunction, IndexFormat,
            PipelineCache, PrimitiveTopology, RenderPipelineDescriptor, SpecializedMeshPipeline,
            SpecializedMeshPipelineError, SpecializedMeshPipelines,
        },
        renderer::RenderDevice,
        sync_world::MainEntity,
        view::ExtractedView,
        view::RenderVisibleEntities,
        Extract, Render, RenderApp, RenderSystems,
    },
};
use std::collections::HashMap;

/// Asset: wireframe topology (LineList indices only)
#[derive(Asset, TypePath, Debug, Clone)]
pub struct WireframeTopology {
    pub indices: Vec<u32>,
}
impl WireframeTopology {
    pub fn new(indices: Vec<u32>) -> Self {
        Self { indices }
    }
}

/// GPU representation of wireframe topology
pub struct GpuWireframeTopology {
    pub index_buffer: Buffer,
    pub index_count: u32,
}

impl RenderAsset for GpuWireframeTopology {
    type SourceAsset = WireframeTopology;
    type Param = SRes<RenderDevice>;
    fn asset_usage(_: &Self::SourceAsset) -> RenderAssetUsages {
        RenderAssetUsages::default()
    }
    fn prepare_asset(
        src: Self::SourceAsset,
        _: bevy::asset::AssetId<Self::SourceAsset>,
        dev: &mut SystemParamItem<Self::Param>,
        _: Option<&Self>,
    ) -> Result<Self, PrepareAssetError<Self::SourceAsset>> {
        let index_buffer = dev.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("Wireframe Index Buffer"),
            contents: bytemuck::cast_slice(&src.indices),
            usage: BufferUsages::INDEX,
        });
        Ok(GpuWireframeTopology {
            index_buffer,
            index_count: src.indices.len() as u32,
        })
    }
}

/// Marker component for wireframe rendering
#[derive(Component, Clone, Debug)]
pub struct WireframeMarker {
    pub topology: Handle<WireframeTopology>,
}

#[derive(Resource, Clone)]
pub struct WireframeShader(pub Handle<Shader>);
#[derive(Resource, Clone)]
pub struct WireframeShaderUV(pub Handle<Shader>);

pub struct CunningWireframePlugin;

impl Plugin for CunningWireframePlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<WireframeTopology>()
            .add_plugins(RenderAssetPlugin::<GpuWireframeTopology>::default());
        let (handle, handle_uv) = {
            let mut shaders = app.world_mut().resource_mut::<Assets<Shader>>();
            let h1 = shaders.add(Shader::from_wgsl(
                include_str!("../../assets/shaders/cunning_wireframe_v2.wgsl"),
                "shaders/cunning_wireframe_v2.wgsl",
            ));
            let h2 = shaders.add(Shader::from_wgsl(
                include_str!("../../assets/shaders/cunning_wireframe_uv.wgsl"),
                "shaders/cunning_wireframe_uv.wgsl",
            ));
            (h1, h2)
        };
        app.insert_resource(WireframeShader(handle));
        app.insert_resource(WireframeShaderUV(handle_uv));
    }

    fn finish(&self, app: &mut App) {
        let handle = app.world().resource::<WireframeShader>().0.clone();
        let handle_uv = app.world().resource::<WireframeShaderUV>().0.clone();
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.insert_resource(WireframeShader(handle));
            render_app.insert_resource(WireframeShaderUV(handle_uv));
            render_app
                .init_resource::<ExtractedWireframes>()
                .init_resource::<WireframePipeline>()
                .init_resource::<SpecializedMeshPipelines<WireframePipeline>>()
                .add_render_command::<Opaque3d, DrawWireframe>()
                .add_systems(ExtractSchedule, extract_wireframes)
                .add_systems(Render, queue_wireframes.in_set(RenderSystems::Queue));
        }
    }
}

#[derive(Resource, Default)]
pub struct ExtractedWireframes(pub HashMap<MainEntity, ExtractedWireframe>);

#[derive(Clone)]
pub struct ExtractedWireframe {
    pub topology_handle: Handle<WireframeTopology>,
    pub is_uv_mode: bool,
    pub is_ghost_mode: bool,
}

pub fn extract_wireframes(
    mut commands: Commands,
    query: Extract<Query<(Entity, &WireframeMarker, &ViewVisibility), With<Mesh3d>>>,
    display_options: Extract<Option<Res<crate::viewport_options::DisplayOptions>>>,
) {
    let (is_uv_mode, is_ghost_mode) = display_options
        .as_ref()
        .map(|o| {
            (
                o.view_mode == crate::viewport_options::ViewportViewMode::UV,
                o.wireframe_ghost_mode,
            )
        })
        .unwrap_or((false, false));
    let mut out = HashMap::with_capacity(query.iter().len());
    for (entity, marker, visibility) in &query {
        if !visibility.get() {
            continue;
        }
        out.insert(
            MainEntity::from(entity),
            ExtractedWireframe {
                topology_handle: marker.topology.clone(),
                is_uv_mode,
                is_ghost_mode,
            },
        );
    }
    commands.insert_resource(ExtractedWireframes(out));
}

#[derive(Resource)]
pub struct WireframePipeline {
    pub shader: Handle<Shader>,
    pub shader_uv: Handle<Shader>,
    pub mesh_pipeline: MeshPipeline,
}

impl FromWorld for WireframePipeline {
    fn from_world(world: &mut World) -> Self {
        WireframePipeline {
            shader: world.resource::<WireframeShader>().0.clone(),
            shader_uv: world.resource::<WireframeShaderUV>().0.clone(),
            mesh_pipeline: world.resource::<MeshPipeline>().clone(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct WireframePipelineKey {
    pub mesh_key: MeshPipelineKey,
    pub is_uv_mode: bool,
    pub is_ghost_mode: bool,
}

impl SpecializedMeshPipeline for WireframePipeline {
    type Key = WireframePipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh_pipeline.specialize(key.mesh_key, layout)?;
        let shader = if key.is_uv_mode {
            self.shader_uv.clone()
        } else {
            self.shader.clone()
        };
        descriptor.vertex.shader = shader.clone();
        descriptor.fragment.as_mut().unwrap().shader = shader;

        // Ghost mode toggle (depth sampling is still gated by DEPTH_PREPASS in shader)
        if key.is_ghost_mode {
            descriptor.vertex.shader_defs.push("GHOST_MODE".into());
            descriptor
                .fragment
                .as_mut()
                .unwrap()
                .shader_defs
                .push("GHOST_MODE".into());
        }

        // Alpha blending for wireframe overlay
        if let Some(fragment) = descriptor.fragment.as_mut() {
            if let Some(target) = fragment.targets.get_mut(0).and_then(|t| t.as_mut()) {
                target.blend = Some(BlendState::ALPHA_BLENDING);
            }
        }

        descriptor.primitive.topology = PrimitiveTopology::LineList;
        descriptor.primitive.cull_mode = None;

        // MSAA helps line edges a lot; enable alpha-to-coverage to improve perceived smoothness on thin lines.
        descriptor.multisample.alpha_to_coverage_enabled = true;

        // Depth test
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_write_enabled = false;
            ds.depth_compare = if key.is_ghost_mode {
                CompareFunction::Always
            } else {
                CompareFunction::GreaterEqual
            };
        }

        Ok(descriptor)
    }
}

pub struct DrawWireframeIndices;

impl<P: PhaseItem> RenderCommand<P> for DrawWireframeIndices {
    type Param = (
        SRes<RenderMeshInstances>,
        SRes<RenderAssets<RenderMesh>>,
        SRes<MeshAllocator>,
        SRes<RenderAssets<GpuWireframeTopology>>,
        SRes<ExtractedWireframes>,
    );
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        item: &P,
        _: (),
        _: Option<()>,
        (mesh_instances, meshes, mesh_allocator, topologies, wireframes): SystemParamItem<
            'w,
            '_,
            Self::Param,
        >,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let main = item.main_entity();
        let Some(wf) = wireframes.into_inner().0.get(&main) else {
            return RenderCommandResult::Failure("missing ExtractedWireframe");
        };
        let mesh_instances = mesh_instances.into_inner();
        let Some(mesh_asset_id) = mesh_instances.mesh_asset_id(main) else {
            return RenderCommandResult::Failure("mesh instance missing");
        };
        let meshes = meshes.into_inner();
        let mesh_allocator = mesh_allocator.into_inner();
        if meshes.get(mesh_asset_id).is_none() {
            return RenderCommandResult::Failure("mesh not prepared");
        }
        let Some(vs) = mesh_allocator.mesh_vertex_slice(&mesh_asset_id) else {
            return RenderCommandResult::Success;
        };
        let Some(gpu_topology) = topologies.into_inner().get(wf.topology_handle.id()) else {
            return RenderCommandResult::Failure("topology not prepared");
        };
        if gpu_topology.index_count == 0
            || gpu_topology.index_buffer.size() == 0
            || vs.buffer.size() == 0
        {
            return RenderCommandResult::Success;
        }
        pass.set_vertex_buffer(0, vs.buffer.slice(..));
        pass.set_index_buffer(gpu_topology.index_buffer.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(
            0..gpu_topology.index_count,
            vs.range.start as i32,
            item.batch_range().clone(),
        );
        RenderCommandResult::Success
    }
}

pub type DrawWireframe = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetMeshBindGroup<2>,
    DrawWireframeIndices,
);

/// Queue wireframes with single-pass Ghost mode using Depth Prepass (Bevy standard approach)
pub fn queue_wireframes(
    opaque_3d_draw_functions: Res<DrawFunctions<Opaque3d>>,
    wireframe_pipeline: Res<WireframePipeline>,
    mut pipelines: ResMut<SpecializedMeshPipelines<WireframePipeline>>,
    pipeline_cache: Res<PipelineCache>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    render_mesh_instances: Res<RenderMeshInstances>,
    mesh_allocator: Res<MeshAllocator>,
    ticks: SystemChangeTick,
    wireframes: Res<ExtractedWireframes>,
    mut opaque_render_phases: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    // IMPORTANT: match MeshViewBindGroup layout decision (based on ViewPrepassTextures/OIT/Atmosphere),
    // otherwise wgpu will panic with bind group layout mismatch (binding 20 etc).
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
        .get_id::<DrawWireframe>()
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

        let mut has_depth_prepass = false;
        if let Some(pt) = prepass_textures {
            if pt.depth.is_some() {
                view_key |= MeshPipelineKey::DEPTH_PREPASS;
                has_depth_prepass = true;
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
            let Some(wf) = wireframes.0.get(visible_entity) else {
                continue;
            };

            // Check for Ghost Mode requirements
            if wf.is_ghost_mode && !has_depth_prepass {
                // Warn once per frame/entity if needed, but for now just proceed.
                // This explains why it might be solid.
            }

            let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(*visible_entity)
            else {
                continue;
            };
            let Some(mesh) = render_meshes.get(mesh_instance.mesh_asset_id) else {
                continue;
            };

            let key =
                view_key | MeshPipelineKey::from_primitive_topology(PrimitiveTopology::LineList);
            let wireframe_key = WireframePipelineKey {
                mesh_key: key,
                is_uv_mode: wf.is_uv_mode,
                is_ghost_mode: wf.is_ghost_mode,
            };
            let Ok(pipeline) = pipelines.specialize(
                &pipeline_cache,
                &wireframe_pipeline,
                wireframe_key,
                &mesh.layout,
            ) else {
                continue;
            };

            let (vertex_slab, index_slab) = mesh_allocator.mesh_slabs(&mesh_instance.mesh_asset_id);
            let batch_set_key = Opaque3dBatchSetKey {
                pipeline,
                draw_function,
                material_bind_group_index: None,
                vertex_slab: vertex_slab.unwrap_or_default(),
                index_slab,
                lightmap_slab: mesh_instance.shared.lightmap_slab_index.map(|i| *i),
            };
            let bin_key = Opaque3dBinKey {
                asset_id: mesh_instance.mesh_asset_id.into(),
            };
            phase.add(
                batch_set_key,
                bin_key,
                (*render_entity, *visible_entity),
                mesh_instance.current_uniform_index,
                BinnedRenderPhaseType::UnbatchableMesh,
                ticks.this_run(),
            );
        }
    }
}
