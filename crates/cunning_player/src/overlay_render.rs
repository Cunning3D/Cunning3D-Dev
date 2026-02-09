use bevy::{
    core_pipeline::core_3d::{Opaque3d, CORE_3D_DEPTH_FORMAT},
    ecs::system::{lifetimeless::SRes, SystemParamItem},
    mesh::{Mesh, Mesh3d, MeshVertexBufferLayoutRef, VertexBufferLayout, VertexFormat},
    pbr::{MeshPipeline, MeshPipelineKey, RenderMeshInstances, SetMeshViewBindGroup, SetMeshViewBindingArrayBindGroup},
    prelude::*,
    render::{
        batching::gpu_preprocessing::GpuPreprocessingSupport,
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::RenderAssets,
        render_phase::{AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, PhaseItem, RenderCommand, RenderCommandResult, SetItemPipeline, TrackedRenderPass, ViewBinnedRenderPhases},
        render_resource::{VertexAttribute, VertexStepMode, *},
        renderer::{RenderDevice, RenderQueue},
        sync_world::RenderEntity,
        view::{ExtractedView, RenderVisibleEntities},
        Extract, Render, RenderApp, RenderSystems,
    },
};
use bevy::core_pipeline::{oit::OrderIndependentTransparencySettings, prepass::ViewPrepassTextures};
use bevy::pbr::ExtractedAtmosphere;
use bevy::render::render_resource::BufferId;

#[derive(Component, Clone, Copy, Debug)]
pub struct PointMarker;

#[derive(Component, Clone, Copy, Debug)]
pub struct NormalMarker;

#[derive(Component, Clone, Copy, Debug)]
pub struct NormalColor(pub Color);

#[derive(Resource, Clone)]
struct PointShader(pub Handle<Shader>);

#[derive(Resource, Clone)]
struct NormalShader(pub Handle<Shader>);

pub struct CunningPointPlugin;
pub struct CunningNormalPlugin;

impl Plugin for CunningPointPlugin {
    fn build(&self, app: &mut App) {
        let shader = Shader::from_wgsl(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/shaders/cunning_point_v2.wgsl")), "shaders/cunning_point_v2.wgsl");
        let handle = { let mut shaders = app.world_mut().resource_mut::<Assets<Shader>>(); shaders.add(shader) };
        app.insert_resource(PointShader(handle));
    }
    fn finish(&self, app: &mut App) {
        let handle = app.world().resource::<PointShader>().0.clone();
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return; };
        render_app
            .insert_resource(PointShader(handle))
            .init_resource::<PointPipeline>()
            .init_resource::<SpecializedMeshPipelines<PointPipeline>>()
            .init_resource::<PointUniforms>()
            .init_resource::<PointQuadBuffer>()
            .add_render_command::<Opaque3d, DrawPointPipeline>()
            .add_systems(ExtractSchedule, extract_points)
            .add_systems(Render, prepare_point_uniforms.in_set(RenderSystems::PrepareBindGroups))
            .add_systems(Render, queue_points.in_set(RenderSystems::Queue));
    }
}

impl Plugin for CunningNormalPlugin {
    fn build(&self, app: &mut App) {
        let shader = Shader::from_wgsl(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/shaders/cunning_normal_v2.wgsl")), "shaders/cunning_normal_v2.wgsl");
        let handle = { let mut shaders = app.world_mut().resource_mut::<Assets<Shader>>(); shaders.add(shader) };
        app.insert_resource(NormalShader(handle));
    }
    fn finish(&self, app: &mut App) {
        let handle = app.world().resource::<NormalShader>().0.clone();
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return; };
        render_app
            .insert_resource(NormalShader(handle))
            .init_resource::<NormalPipeline>()
            .init_resource::<SpecializedMeshPipelines<NormalPipeline>>()
            .init_resource::<NormalUniforms>()
            .init_resource::<NormalTriangleBuffer>()
            .add_render_command::<Opaque3d, DrawNormalPipeline>()
            .add_systems(ExtractSchedule, extract_normals)
            .add_systems(Render, prepare_normal_uniforms.in_set(RenderSystems::PrepareBindGroups))
            .add_systems(Render, queue_normals.in_set(RenderSystems::Queue));
    }
}

#[derive(Component)]
struct RenderPoint { mesh_handle: Handle<Mesh>, transform: GlobalTransform }

fn extract_points(mut commands: Commands, query: Extract<Query<(RenderEntity, &Mesh3d, &ViewVisibility, &GlobalTransform), With<PointMarker>>>) {
    for (re, mesh, vis, tf) in &query { if vis.get() { commands.entity(re).insert(RenderPoint { mesh_handle: mesh.0.clone(), transform: *tf }); } }
}

#[derive(Resource)]
struct PointPipeline { shader: Handle<Shader>, mesh_pipeline: MeshPipeline, uniform_layout: BindGroupLayout, uniform_layout_desc: BindGroupLayoutDescriptor }

impl FromWorld for PointPipeline {
    fn from_world(world: &mut World) -> Self {
        let rd = world.resource::<RenderDevice>().clone();
        let shader = world.resource::<PointShader>().0.clone();
        let mesh_pipeline = world.resource::<MeshPipeline>().clone();
        let entries = [BindGroupLayoutEntry { binding: 0, visibility: ShaderStages::VERTEX, ty: BindingType::Buffer { ty: BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: Some(std::num::NonZeroU64::new(64).unwrap()) }, count: None }];
        let uniform_layout = rd.create_bind_group_layout("point_uniform_layout", &entries);
        Self { shader, mesh_pipeline, uniform_layout, uniform_layout_desc: BindGroupLayoutDescriptor::new("point_uniform_layout", &entries) }
    }
}

impl SpecializedMeshPipeline for PointPipeline {
    type Key = MeshPipelineKey;
    fn specialize(&self, key: Self::Key, layout: &MeshVertexBufferLayoutRef) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut d = self.mesh_pipeline.specialize(key, layout)?;
        d.vertex.shader = self.shader.clone();
        if let Some(f) = d.fragment.as_mut() { f.shader = self.shader.clone(); }
        if d.depth_stencil.is_none() { d.depth_stencil = Some(DepthStencilState { format: CORE_3D_DEPTH_FORMAT, depth_write_enabled: false, depth_compare: CompareFunction::GreaterEqual, stencil: StencilState::default(), bias: DepthBiasState::default() }); }
        d.set_layout(2, self.uniform_layout_desc.clone());
        let mut inst = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(2)])?;
        inst.step_mode = VertexStepMode::Instance;
        d.vertex.buffers.clear();
        d.vertex.buffers.push(inst);
        d.vertex.buffers.push(VertexBufferLayout { array_stride: 20, step_mode: VertexStepMode::Vertex, attributes: vec![VertexAttribute { format: VertexFormat::Float32x3, offset: 0, shader_location: 0 }, VertexAttribute { format: VertexFormat::Float32x2, offset: 12, shader_location: 1 }] });
        d.primitive.topology = PrimitiveTopology::TriangleList;
        d.primitive.cull_mode = None;
        if let Some(ds) = d.depth_stencil.as_mut() { ds.depth_write_enabled = false; ds.bias = DepthBiasState { constant: 50, slope_scale: 1.0, clamp: 0.0 }; }
        Ok(d)
    }
}

#[derive(ShaderType)]
struct PointUniform { model: Mat4 }

#[derive(Resource, Default)]
struct PointUniforms { buffer: DynamicUniformBuffer<PointUniform>, bind_group: Option<BindGroup>, buffer_id: Option<BufferId> }

#[derive(Component)]
struct PointUniformOffset { offset: u32 }

fn prepare_point_uniforms(mut commands: Commands, mut u: ResMut<PointUniforms>, pts: Query<(Entity, &RenderPoint)>, rd: Res<RenderDevice>, rq: Res<RenderQueue>, pipe: Res<PointPipeline>) {
    u.buffer.clear();
    for (e, p) in pts.iter() { let off = u.buffer.push(&PointUniform { model: p.transform.to_matrix() }); commands.entity(e).insert(PointUniformOffset { offset: off as u32 }); }
    u.buffer.write_buffer(&rd, &rq);
    let id = u.buffer.buffer().map(|b| b.id());
    if id != u.buffer_id { u.buffer_id = id; u.bind_group = None; }
    if u.bind_group.is_none() { if let Some(b) = u.buffer.binding() { u.bind_group = Some(rd.create_bind_group("point_bind_group", &pipe.uniform_layout, &[BindGroupEntry { binding: 0, resource: b }])); } }
}

struct SetPointBindGroup<const I: usize>;
impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetPointBindGroup<I> {
    type Param = (SRes<PointUniforms>,);
    type ViewQuery = ();
    type ItemQuery = &'static PointUniformOffset;
    fn render<'w>(_item: &P, _view: (), off: Option<&'w PointUniformOffset>, (u,): SystemParamItem<'w, '_, Self::Param>, pass: &mut TrackedRenderPass<'w>) -> RenderCommandResult {
        let Some(off) = off else { return RenderCommandResult::Failure("missing PointUniformOffset"); };
        let u = u.into_inner();
        let Some(bg) = u.bind_group.as_ref() else { return RenderCommandResult::Failure("missing PointUniforms bind_group"); };
        pass.set_bind_group(I, bg, &[off.offset]);
        RenderCommandResult::Success
    }
}

struct DrawPoint;
impl<P: PhaseItem> RenderCommand<P> for DrawPoint {
    type Param = (SRes<RenderAssets<RenderMesh>>, SRes<MeshAllocator>, SRes<PointQuadBuffer>);
    type ViewQuery = ();
    type ItemQuery = &'static RenderPoint;
    fn render<'w>(_item: &P, _view: (), rp: Option<&'w RenderPoint>, (meshes, alloc, quad): SystemParamItem<'w, '_, Self::Param>, pass: &mut TrackedRenderPass<'w>) -> RenderCommandResult {
        let Some(rp) = rp else { return RenderCommandResult::Failure("missing RenderPoint"); };
        let meshes = meshes.into_inner();
        let alloc = alloc.into_inner();
        let quad = quad.into_inner();
        let mesh_id = rp.mesh_handle.id();
        let Some(render_mesh) = meshes.get(mesh_id) else { return RenderCommandResult::Failure("mesh not prepared"); };
        let Some(vs) = alloc.mesh_vertex_slice(&mesh_id) else { return RenderCommandResult::Success; };
        let stride = render_mesh.layout.0.layout().array_stride as u64;
        let start = vs.range.start as u64 * stride;
        let end = vs.range.end as u64 * stride;
        pass.set_vertex_buffer(0, vs.buffer.slice(start..end));
        pass.set_vertex_buffer(1, quad.vertex_buffer.slice(..));
        pass.draw(0..6, 0..render_mesh.vertex_count);
        RenderCommandResult::Success
    }
}

type DrawPointPipeline = (SetItemPipeline, SetMeshViewBindGroup<0>, SetMeshViewBindingArrayBindGroup<1>, SetPointBindGroup<2>, DrawPoint);

#[derive(Resource)]
struct PointQuadBuffer { vertex_buffer: Buffer }

impl FromWorld for PointQuadBuffer {
    fn from_world(world: &mut World) -> Self {
        let rd = world.resource::<RenderDevice>();
        let v: [[f32; 5]; 6] = [
            [-0.5, -0.5, 0.0, 0.0, 1.0], [0.5, -0.5, 0.0, 1.0, 1.0], [0.5, 0.5, 0.0, 1.0, 0.0],
            [-0.5, -0.5, 0.0, 0.0, 1.0], [0.5, 0.5, 0.0, 1.0, 0.0], [-0.5, 0.5, 0.0, 0.0, 0.0],
        ];
        Self { vertex_buffer: rd.create_buffer_with_data(&BufferInitDescriptor { label: Some("Point Quad Vertex Buffer"), contents: bytemuck::cast_slice(&v), usage: BufferUsages::VERTEX }) }
    }
}

fn queue_points(
    opaque: Res<DrawFunctions<Opaque3d>>,
    pipe: Res<PointPipeline>,
    mut pipes: ResMut<SpecializedMeshPipelines<PointPipeline>>,
    cache: Res<PipelineCache>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    inst: Res<RenderMeshInstances>,
    alloc: Res<MeshAllocator>,
    _gpu: Res<GpuPreprocessingSupport>,
    ticks: bevy::ecs::system::SystemChangeTick,
    pts: Query<&RenderPoint>,
    mut phases: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    views: Query<(&ExtractedView, &Msaa, &RenderVisibleEntities, Option<&ViewPrepassTextures>, bevy::ecs::query::Has<OrderIndependentTransparencySettings>, bevy::ecs::query::Has<ExtractedAtmosphere>)>,
) {
    let draw = opaque.read().get_id::<DrawPointPipeline>().unwrap();
    for (view, msaa, visible, prepass, has_oit, has_atm) in views.iter() {
        let mut view_key = MeshPipelineKey::from_msaa_samples(msaa.samples()) | MeshPipelineKey::from_hdr(view.hdr);
        if has_oit { view_key |= MeshPipelineKey::OIT_ENABLED; }
        if has_atm { view_key |= MeshPipelineKey::ATMOSPHERE; }
        if let Some(pt) = prepass {
            if pt.depth.is_some() { view_key |= MeshPipelineKey::DEPTH_PREPASS; }
            if pt.normal.is_some() { view_key |= MeshPipelineKey::NORMAL_PREPASS; }
            if pt.motion_vectors.is_some() { view_key |= MeshPipelineKey::MOTION_VECTOR_PREPASS; }
            if pt.deferred.is_some() { view_key |= MeshPipelineKey::DEFERRED_PREPASS; }
        }
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else { continue; };
        for (re, ve) in visible.iter::<Mesh3d>() {
            if pts.get(*re).is_err() { continue; }
            let Some(qd) = inst.render_mesh_queue_data(*ve) else { continue; };
            let Some(mesh) = render_meshes.get(qd.mesh_asset_id) else { continue; };
            let key = view_key | MeshPipelineKey::from_primitive_topology(PrimitiveTopology::TriangleList);
            let Ok(pipeline) = pipes.specialize(&cache, &pipe, key, &mesh.layout) else { continue; };
            let (vs, is) = alloc.mesh_slabs(&qd.mesh_asset_id);
            let batch_set_key = bevy::core_pipeline::core_3d::Opaque3dBatchSetKey { pipeline, draw_function: draw, material_bind_group_index: None, vertex_slab: vs.unwrap_or_default(), index_slab: is, lightmap_slab: qd.shared.lightmap_slab_index.map(|i| *i) };
            let bin_key = bevy::core_pipeline::core_3d::Opaque3dBinKey { asset_id: qd.mesh_asset_id.into() };
            phase.add(batch_set_key, bin_key, (*re, *ve), qd.current_uniform_index, BinnedRenderPhaseType::UnbatchableMesh, ticks.this_run());
        }
    }
}

#[derive(Component)]
struct RenderNormal { mesh_handle: Handle<Mesh>, transform: GlobalTransform, color: Color }

fn extract_normals(mut commands: Commands, query: Extract<Query<(Entity, Option<RenderEntity>, &Mesh3d, Option<&ViewVisibility>, Option<&GlobalTransform>, Option<&NormalColor>), With<NormalMarker>>>) {
    for (_main, re, mesh, vv, tf, c) in &query {
        let (Some(re), Some(vv), Some(tf)) = (re, vv, tf) else { continue; };
        if !vv.get() { continue; }
        commands.entity(re).insert(RenderNormal { mesh_handle: mesh.0.clone(), transform: *tf, color: c.map(|v| v.0).unwrap_or(Color::WHITE) });
    }
}

#[derive(Resource)]
struct NormalPipeline { shader: Handle<Shader>, mesh_pipeline: MeshPipeline, uniform_layout: BindGroupLayout, uniform_layout_desc: BindGroupLayoutDescriptor }

impl FromWorld for NormalPipeline {
    fn from_world(world: &mut World) -> Self {
        let rd = world.resource::<RenderDevice>().clone();
        let shader = world.resource::<NormalShader>().0.clone();
        let mesh_pipeline = world.resource::<MeshPipeline>().clone();
        let entries = [BindGroupLayoutEntry { binding: 0, visibility: ShaderStages::VERTEX, ty: BindingType::Buffer { ty: BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: Some(std::num::NonZeroU64::new(80).unwrap()) }, count: None }];
        let uniform_layout = rd.create_bind_group_layout("normal_uniform_layout", &entries);
        Self { shader, mesh_pipeline, uniform_layout, uniform_layout_desc: BindGroupLayoutDescriptor::new("normal_uniform_layout", &entries) }
    }
}

impl SpecializedMeshPipeline for NormalPipeline {
    type Key = MeshPipelineKey;
    fn specialize(&self, key: Self::Key, layout: &MeshVertexBufferLayoutRef) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut d = self.mesh_pipeline.specialize(key, layout)?;
        d.vertex.shader = self.shader.clone();
        if let Some(f) = d.fragment.as_mut() { f.shader = self.shader.clone(); }
        if d.depth_stencil.is_none() { d.depth_stencil = Some(DepthStencilState { format: CORE_3D_DEPTH_FORMAT, depth_write_enabled: false, depth_compare: CompareFunction::GreaterEqual, stencil: StencilState::default(), bias: DepthBiasState::default() }); }
        d.set_layout(2, self.uniform_layout_desc.clone());
        let mut inst = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(1), Mesh::ATTRIBUTE_NORMAL.at_shader_location(2)])?;
        inst.step_mode = VertexStepMode::Instance;
        d.vertex.buffers.clear();
        d.vertex.buffers.push(inst);
        d.vertex.buffers.push(VertexBufferLayout { array_stride: 8, step_mode: VertexStepMode::Vertex, attributes: vec![VertexAttribute { format: VertexFormat::Float32x2, offset: 0, shader_location: 0 }] });
        d.primitive.topology = PrimitiveTopology::TriangleList;
        d.primitive.cull_mode = None;
        if let Some(ds) = d.depth_stencil.as_mut() { ds.depth_write_enabled = false; ds.depth_compare = CompareFunction::GreaterEqual; }
        Ok(d)
    }
}

#[derive(ShaderType)]
struct NormalUniform { model: Mat4, color: Vec4 }

#[derive(Resource, Default)]
struct NormalUniforms { buffer: DynamicUniformBuffer<NormalUniform>, bind_group: Option<BindGroup>, buffer_id: Option<BufferId> }

#[derive(Component)]
struct NormalUniformOffset { offset: u32 }

fn prepare_normal_uniforms(mut commands: Commands, mut u: ResMut<NormalUniforms>, ns: Query<(Entity, &RenderNormal)>, rd: Res<RenderDevice>, rq: Res<RenderQueue>, pipe: Res<NormalPipeline>) {
    u.buffer.clear();
    for (e, n) in ns.iter() {
        let c = n.color.to_linear();
        let off = u.buffer.push(&NormalUniform { model: n.transform.to_matrix(), color: Vec4::new(c.red, c.green, c.blue, c.alpha) });
        commands.entity(e).insert(NormalUniformOffset { offset: off as u32 });
    }
    u.buffer.write_buffer(&rd, &rq);
    let id = u.buffer.buffer().map(|b| b.id());
    if id != u.buffer_id { u.buffer_id = id; u.bind_group = None; }
    if u.bind_group.is_none() { if let Some(b) = u.buffer.binding() { u.bind_group = Some(rd.create_bind_group("normal_bind_group", &pipe.uniform_layout, &[BindGroupEntry { binding: 0, resource: b }])); } }
}

struct SetNormalBindGroup<const I: usize>;
impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetNormalBindGroup<I> {
    type Param = (SRes<NormalUniforms>,);
    type ViewQuery = ();
    type ItemQuery = &'static NormalUniformOffset;
    fn render<'w>(_item: &P, _view: (), off: Option<&'w NormalUniformOffset>, (u,): SystemParamItem<'w, '_, Self::Param>, pass: &mut TrackedRenderPass<'w>) -> RenderCommandResult {
        let Some(off) = off else { return RenderCommandResult::Failure("missing NormalUniformOffset"); };
        let u = u.into_inner();
        let Some(bg) = u.bind_group.as_ref() else { return RenderCommandResult::Failure("missing NormalUniforms bind_group"); };
        pass.set_bind_group(I, bg, &[off.offset]);
        RenderCommandResult::Success
    }
}

struct DrawNormal;
impl<P: PhaseItem> RenderCommand<P> for DrawNormal {
    type Param = (SRes<RenderAssets<RenderMesh>>, SRes<MeshAllocator>, SRes<NormalTriangleBuffer>);
    type ViewQuery = ();
    type ItemQuery = &'static RenderNormal;
    fn render<'w>(_item: &P, _view: (), rn: Option<&'w RenderNormal>, (meshes, alloc, tri): SystemParamItem<'w, '_, Self::Param>, pass: &mut TrackedRenderPass<'w>) -> RenderCommandResult {
        let Some(rn) = rn else { return RenderCommandResult::Failure("missing RenderNormal"); };
        let meshes = meshes.into_inner();
        let alloc = alloc.into_inner();
        let tri = tri.into_inner();
        let mesh_id = rn.mesh_handle.id();
        let Some(render_mesh) = meshes.get(mesh_id) else { return RenderCommandResult::Failure("mesh not prepared"); };
        let Some(vs) = alloc.mesh_vertex_slice(&mesh_id) else { return RenderCommandResult::Success; };
        let stride = render_mesh.layout.0.layout().array_stride as u64;
        let start = vs.range.start as u64 * stride;
        let end = vs.range.end as u64 * stride;
        pass.set_vertex_buffer(0, vs.buffer.slice(start..end));
        pass.set_vertex_buffer(1, tri.vertex_buffer.slice(..));
        pass.draw(0..3, 0..render_mesh.vertex_count);
        RenderCommandResult::Success
    }
}

type DrawNormalPipeline = (SetItemPipeline, SetMeshViewBindGroup<0>, SetMeshViewBindingArrayBindGroup<1>, SetNormalBindGroup<2>, DrawNormal);

#[derive(Resource)]
struct NormalTriangleBuffer { vertex_buffer: Buffer }

impl FromWorld for NormalTriangleBuffer {
    fn from_world(world: &mut World) -> Self {
        let rd = world.resource::<RenderDevice>();
        let v: [[f32; 2]; 3] = [[-0.5, 0.0], [0.5, 0.0], [0.0, 1.0]];
        Self { vertex_buffer: rd.create_buffer_with_data(&BufferInitDescriptor { label: Some("Normal Triangle Vertex Buffer"), contents: bytemuck::cast_slice(&v), usage: BufferUsages::VERTEX }) }
    }
}

fn queue_normals(
    opaque: Res<DrawFunctions<Opaque3d>>,
    pipe: Res<NormalPipeline>,
    mut pipes: ResMut<SpecializedMeshPipelines<NormalPipeline>>,
    cache: Res<PipelineCache>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    inst: Res<RenderMeshInstances>,
    alloc: Res<MeshAllocator>,
    _gpu: Res<GpuPreprocessingSupport>,
    ticks: bevy::ecs::system::SystemChangeTick,
    ns: Query<&RenderNormal>,
    mut phases: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    views: Query<(&ExtractedView, &Msaa, &RenderVisibleEntities, Option<&ViewPrepassTextures>, bevy::ecs::query::Has<OrderIndependentTransparencySettings>, bevy::ecs::query::Has<ExtractedAtmosphere>)>,
) {
    let draw = opaque.read().get_id::<DrawNormalPipeline>().unwrap();
    for (view, msaa, visible, prepass, has_oit, has_atm) in views.iter() {
        let mut view_key = MeshPipelineKey::from_msaa_samples(msaa.samples()) | MeshPipelineKey::from_hdr(view.hdr);
        if has_oit { view_key |= MeshPipelineKey::OIT_ENABLED; }
        if has_atm { view_key |= MeshPipelineKey::ATMOSPHERE; }
        if let Some(pt) = prepass {
            if pt.depth.is_some() { view_key |= MeshPipelineKey::DEPTH_PREPASS; }
            if pt.normal.is_some() { view_key |= MeshPipelineKey::NORMAL_PREPASS; }
            if pt.motion_vectors.is_some() { view_key |= MeshPipelineKey::MOTION_VECTOR_PREPASS; }
            if pt.deferred.is_some() { view_key |= MeshPipelineKey::DEFERRED_PREPASS; }
        }
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else { continue; };
        for (re, ve) in visible.iter::<Mesh3d>() {
            if ns.get(*re).is_err() { continue; }
            let Some(qd) = inst.render_mesh_queue_data(*ve) else { continue; };
            let Some(mesh) = render_meshes.get(qd.mesh_asset_id) else { continue; };
            let key = view_key | MeshPipelineKey::from_primitive_topology(PrimitiveTopology::TriangleList);
            let Ok(pipeline) = pipes.specialize(&cache, &pipe, key, &mesh.layout) else { continue; };
            let (vs, is) = alloc.mesh_slabs(&qd.mesh_asset_id);
            let batch_set_key = bevy::core_pipeline::core_3d::Opaque3dBatchSetKey { pipeline, draw_function: draw, material_bind_group_index: None, vertex_slab: vs.unwrap_or_default(), index_slab: is, lightmap_slab: qd.shared.lightmap_slab_index.map(|i| *i) };
            let bin_key = bevy::core_pipeline::core_3d::Opaque3dBinKey { asset_id: qd.mesh_asset_id.into() };
            phase.add(batch_set_key, bin_key, (*re, *ve), qd.current_uniform_index, BinnedRenderPhaseType::UnbatchableMesh, ticks.this_run());
        }
    }
}

