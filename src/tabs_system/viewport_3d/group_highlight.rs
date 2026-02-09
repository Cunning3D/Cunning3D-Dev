use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, PrimitiveTopology, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

use crate::libs::geometry::group::ElementGroupMask;
use crate::libs::geometry::ids::{PointId, VertexId};
use crate::libs::geometry::mesh::{Attribute, GeoPrimitive, PrimitiveType};
use crate::mesh::Geometry;
use crate::nodes::NodeGraphResource;

// --- Custom Material for Highlight Fill ---
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct GroupHighlightMaterial {}

impl Material for GroupHighlightMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/group_highlight.wgsl".into()
    }

    fn vertex_shader() -> ShaderRef {
        "shaders/group_highlight.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(depth_stencil) = &mut descriptor.depth_stencil {
            depth_stencil.depth_compare = bevy::render::render_resource::CompareFunction::Always;
            depth_stencil.depth_write_enabled = false;
            // Bias fill slightly to appear above geometry but below wireframe
            depth_stencil.bias.constant = -5;
            depth_stencil.bias.slope_scale = -1.0;
        }
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

// --- Custom Material for Highlight Wireframe ---
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct GroupHighlightWireMaterial {}

impl Material for GroupHighlightWireMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/group_highlight_wire.wgsl".into()
    }

    fn vertex_shader() -> ShaderRef {
        "shaders/group_highlight_wire.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend // Switch to Blend to ensure correct sorting with fill
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(depth_stencil) = &mut descriptor.depth_stencil {
            depth_stencil.depth_compare = bevy::render::render_resource::CompareFunction::Always;
            depth_stencil.depth_write_enabled = false;
            // Bias wireframe even more to appear on top of fill
            depth_stencil.bias.constant = -10;
            depth_stencil.bias.slope_scale = -2.0;
        }
        descriptor.primitive.cull_mode = None;
        descriptor.primitive.topology = PrimitiveTopology::LineList;
        Ok(())
    }
}

/// Marker component for the group highlight entity
#[derive(Component)]
pub struct GroupHighlightMesh {
    pub dirty_id: u64,
    pub node_id: uuid::Uuid,
}

/// System to handle the visualization of the selected group in the active node
pub fn update_group_highlight_system(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GroupHighlightMaterial>>,
    mut materials_wire: ResMut<Assets<GroupHighlightWireMaterial>>,
    node_graph_res: Res<NodeGraphResource>,
    query_highlight: Query<(Entity, &GroupHighlightMesh)>,
) {
    let node_graph = &node_graph_res.0;

    // 1. Identify DISPLAY Node (not selected node)
    let Some(display_node_id) = node_graph.display_node else {
        // No display node -> Clear
        for (e, _) in query_highlight.iter() {
            commands.entity(e).despawn();
        }
        return;
    };

    let Some(node) = node_graph.nodes.get(&display_node_id) else {
        return;
    };

    // 2. Check if it's a Group Create Node
    if node.node_type.name() != "Group Create" {
        for (e, _) in query_highlight.iter() {
            commands.entity(e).despawn();
        }
        return;
    }

    // 3. Get Geometry
    let geo_arc = &node_graph.final_geometry;
    let current_dirty_id = geo_arc.dirty_id;

    // 4. Check if update is needed
    let mut needs_update = false;
    let mut has_any = false;

    for (_, highlight) in query_highlight.iter() {
        has_any = true;
        if highlight.dirty_id != current_dirty_id || highlight.node_id != display_node_id {
            needs_update = true;
            break;
        }
    }

    if !has_any {
        needs_update = true;
    }

    if !needs_update {
        return;
    }

    // Clear everything before rebuild (handling both fill and wire entities)
    for (e, _) in query_highlight.iter() {
        commands.entity(e).despawn();
    }

    // 5. Rebuild
    let group_name = node
        .parameters
        .iter()
        .find(|p| p.name == "group_name")
        .and_then(|p| match &p.value {
            crate::nodes::parameter::ParameterValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or("group1".to_string());

    let group_type = node
        .parameters
        .iter()
        .find(|p| p.name == "group_type")
        .and_then(|p| match &p.value {
            crate::nodes::parameter::ParameterValue::Int(i) => Some(*i),
            _ => None,
        })
        .unwrap_or(0);

    if group_type != 1 {
        return;
    }

    if let Some(mask) = geo_arc.get_primitive_group(&group_name) {
        if mask.count_ones() == 0 {
            return;
        }

        let fill_mesh_opt = build_primitive_overlay_mesh(&geo_arc, mask);
        let wire_mesh_opt = build_primitive_wireframe_mesh(&geo_arc, mask);

        // Spawn Fill Entity
        if let Some(mesh) = fill_mesh_opt {
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials.add(GroupHighlightMaterial {})),
                Transform::default(),
                GlobalTransform::default(),
                Visibility::default(),
                GroupHighlightMesh {
                    dirty_id: current_dirty_id,
                    node_id: display_node_id,
                },
            ));
        }

        // Spawn Wireframe Entity
        if let Some(mesh) = wire_mesh_opt {
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials_wire.add(GroupHighlightWireMaterial {})),
                Transform::default(),
                GlobalTransform::default(),
                Visibility::default(),
                GroupHighlightMesh {
                    dirty_id: current_dirty_id,
                    node_id: display_node_id,
                },
            ));
        }
    }
}

fn build_primitive_overlay_mesh(geo: &Geometry, mask: &ElementGroupMask) -> Option<Mesh> {
    let positions = geo.get_point_position_attribute()?;

    if positions.is_empty() {
        return None;
    }

    let vertex_positions: Vec<[f32; 3]> = positions.iter().map(|p| p.to_array()).collect();
    let mut indices = Vec::with_capacity(mask.count_ones().saturating_mul(3));

    for prim_dense_idx in mask.iter_ones() {
        if let Some(prim_id) = geo.primitives().get_id_from_dense(prim_dense_idx) {
            if let Some(prim) = geo.primitives().get(prim_id) {
                if let GeoPrimitive::Polygon(poly) = prim {
                    let vertices = &poly.vertices;
                    if vertices.len() >= 3 {
                        let v0 = vertices[0];
                        for i in 1..vertices.len() - 1 {
                            let v1 = vertices[i];
                            let v2 = vertices[i + 1];

                            let get_p = |vid: VertexId| -> Option<u32> {
                                let v = geo.vertices().get(vid.into())?;
                                geo.points()
                                    .get_dense_index(v.point_id.into())
                                    .map(|i| i as u32)
                            };

                            if let (Some(p0), Some(p1), Some(p2)) =
                                (get_p(v0), get_p(v1), get_p(v2))
                            {
                                indices.push(p0);
                                indices.push(p1);
                                indices.push(p2);
                            }
                        }
                    }
                }
            }
        }
    }

    if indices.is_empty() {
        return None;
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertex_positions);
    mesh.insert_indices(Indices::U32(indices));
    Some(mesh)
}

fn build_primitive_wireframe_mesh(geo: &Geometry, mask: &ElementGroupMask) -> Option<Mesh> {
    let positions = geo.get_point_position_attribute()?;

    if positions.is_empty() {
        return None;
    }

    let vertex_positions: Vec<[f32; 3]> = positions.iter().map(|p| p.to_array()).collect();
    let mut indices = Vec::with_capacity(mask.count_ones().saturating_mul(2));

    for prim_dense_idx in mask.iter_ones() {
        if let Some(prim_id) = geo.primitives().get_id_from_dense(prim_dense_idx) {
            if let Some(prim) = geo.primitives().get(prim_id) {
                let vertices = prim.vertices();
                let num_verts = vertices.len();
                if num_verts < 2 {
                    continue;
                }

                let is_closed = matches!(prim, GeoPrimitive::Polygon(_));
                let limit = if is_closed { num_verts } else { num_verts - 1 };

                let get_p = |vid: VertexId| -> Option<u32> {
                    let v = geo.vertices().get(vid.into())?;
                    geo.points()
                        .get_dense_index(v.point_id.into())
                        .map(|i| i as u32)
                };

                for i in 0..limit {
                    let v1_idx = vertices[i];
                    let v2_idx = vertices[(i + 1) % num_verts];

                    if let (Some(p1), Some(p2)) = (get_p(v1_idx), get_p(v2_idx)) {
                        indices.push(p1);
                        indices.push(p2);
                    }
                }
            }
        }
    }

    if indices.is_empty() {
        return None;
    }

    let mut mesh = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertex_positions);
    mesh.insert_indices(Indices::U32(indices));
    Some(mesh)
}
