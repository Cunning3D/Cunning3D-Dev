use crate::libs::geometry::mesh::GeoPrimitive;
use crate::nodes::NodeGraphResource;
use crate::render::point::PointMarker;
use crate::tabs_system::viewport_3d::group_highlight::GroupHighlightWireMaterial;
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use bevy::render::sync_world::SyncToRenderWorld;

// --- Components ---

/// Component to control group visualization on an entity.
/// Attach this to an entity that has a `Geometry` component.
#[derive(Component, Default)]
pub struct GroupVisualization {
    pub point_group: Option<String>,
    pub primitive_group: Option<String>,
    pub edge_group: Option<String>,
    pub vertex_group: Option<String>,
    pub geometry_version: u64, // Track geometry version to trigger updates
}

/// Marker for the overlay entity spawned by the visualization system.
#[derive(Component)]
pub struct GroupOverlayMarker;

/// Marker for specific types of overlays (to easier managing despawn)
#[derive(Component)]
pub struct PointOverlay;

#[derive(Component)]
pub struct EdgeOverlay;

#[derive(Component)]
pub struct VertexOverlay;

// --- Resources ---

/// Pre-loaded materials for group highlighting
#[derive(Resource)]
pub struct GroupHighlightMaterials {
    pub edge_wire_mat: Handle<GroupHighlightWireMaterial>,
}

impl FromWorld for GroupHighlightMaterials {
    fn from_world(world: &mut World) -> Self {
        // Edges: Reuse the same wireframe pipeline as primitive group highlight.
        let edge_wire_mat = {
            let mut wire_materials = world.resource_mut::<Assets<GroupHighlightWireMaterial>>();
            wire_materials.add(GroupHighlightWireMaterial {})
        };

        GroupHighlightMaterials { edge_wire_mat }
    }
}

// --- Plugin ---

pub struct GroupVisualizationPlugin;

impl Plugin for GroupVisualizationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GroupHighlightMaterials>().add_systems(
            Update,
            (
                sync_visualization_options,        // [NEW] Sync options to components
                update_group_visualization_system, // Combined system
            ),
        );
    }
}

// --- Systems ---

/// Syncs the global DisplayOptions (from UI) to the GroupVisualization components on entities.
fn sync_visualization_options(
    display_options: Res<crate::viewport_options::DisplayOptions>,
    mut query: Query<&mut GroupVisualization>,
) {
    if !display_options.is_changed() {
        return;
    }

    for mut viz in query.iter_mut() {
        if viz.point_group != display_options.overlays.point_group_viz {
            viz.point_group = display_options.overlays.point_group_viz.clone();
        }
        if viz.edge_group != display_options.overlays.edge_group_viz {
            viz.edge_group = display_options.overlays.edge_group_viz.clone();
        }
        if viz.vertex_group != display_options.overlays.vertex_group_viz {
            viz.vertex_group = display_options.overlays.vertex_group_viz.clone();
        }
        // Primitive Group is handled by existing primitive selection system (user instruction),
        // but if we were to handle it here, we would sync it too.
    }
}

fn update_group_visualization_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut GroupVisualization), With<Mesh3d>>,
    children_query: Query<&Children>,
    overlay_query: Query<Entity, Or<(With<PointOverlay>, With<EdgeOverlay>, With<VertexOverlay>)>>,
    materials: Res<GroupHighlightMaterials>,
    mut meshes: ResMut<Assets<Mesh>>,
    node_graph_res: Res<NodeGraphResource>,
) {
    let node_graph = &node_graph_res.0;
    let geometry = &node_graph.final_geometry;

    for (entity, mut viz) in query.iter_mut() {
        let is_dirty = viz.geometry_version != geometry.dirty_id || viz.is_changed();

        if !is_dirty {
            continue;
        }
        viz.geometry_version = geometry.dirty_id;

        // 1. Despawn existing Overlays
        if let Ok(children) = children_query.get(entity) {
            for &child in children {
                if overlay_query.contains(child) {
                    commands.entity(child).despawn();
                }
            }
        }

        // --- Points ---
        if let Some(group_name) = &viz.point_group {
            if group_name != "None" && !group_name.is_empty() {
                if let Some(mask) = geometry.get_point_group(group_name) {
                    if let Some(positions) = geometry.get_point_position_attribute() {
                        let mut pts = Vec::with_capacity(mask.count_ones());
                        for i in mask.iter_ones() {
                            if let Some(p) = positions.get(i) {
                                pts.push(*p);
                            }
                        }
                        if !pts.is_empty() {
                            let mut m = Mesh::new(
                                PrimitiveTopology::PointList,
                                RenderAssetUsages::default(),
                            );
                            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pts);
                            let mesh_handle = meshes.add(m);
                            commands.entity(entity).with_children(|parent| {
                                parent.spawn((
                                    Mesh3d(mesh_handle),
                                    PointMarker,
                                    SyncToRenderWorld,
                                    Transform::default(),
                                    Visibility::Visible,
                                    GroupOverlayMarker,
                                    PointOverlay,
                                ));
                            });
                        }
                    }
                }
            }
        }

        // --- Edges ---
        if let Some(group_name) = &viz.edge_group {
            if group_name != "None" && !group_name.is_empty() {
                let Some(positions) = geometry.get_point_position_attribute() else {
                    continue;
                };

                if let Some(edge_mask) = geometry.get_edge_group(group_name) {
                    let edge_count = geometry.edges().len();
                    if edge_mask.len() != edge_count {
                        continue;
                    }
                    let vertex_positions: Vec<[f32; 3]> =
                        positions.iter().map(|p| p.to_array()).collect();
                    let mut indices: Vec<u32> =
                        Vec::with_capacity(edge_mask.count_ones().saturating_mul(2));
                    for ei in edge_mask.iter_ones() {
                        let Some(eid) = geometry.edges().get_id_from_dense(ei) else {
                            continue;
                        };
                        let Some(e) = geometry.edges().get(eid) else {
                            continue;
                        };
                        let (Some(p0i), Some(p1i)) = (
                            geometry.points().get_dense_index(e.p0.into()),
                            geometry.points().get_dense_index(e.p1.into()),
                        ) else {
                            continue;
                        };
                        indices.push(p0i as u32);
                        indices.push(p1i as u32);
                    }
                    if !indices.is_empty() {
                        let mut mesh =
                            Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default());
                        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertex_positions);
                        mesh.insert_indices(bevy::mesh::Indices::U32(indices));
                        let mesh_handle = meshes.add(mesh);
                        commands.entity(entity).with_children(|parent| {
                            parent.spawn((
                                Mesh3d(mesh_handle),
                                MeshMaterial3d(materials.edge_wire_mat.clone()),
                                Transform::default(),
                                Visibility::Visible,
                                GroupOverlayMarker,
                                EdgeOverlay,
                            ));
                        });
                    }
                }
            }
        }

        // --- Vertices ---
        if let Some(group_name) = &viz.vertex_group {
            if group_name != "None" && !group_name.is_empty() {
                if let Some(mask) = geometry.get_vertex_group(group_name) {
                    let Some(positions) = geometry.get_point_position_attribute() else {
                        continue;
                    };
                    let mut pts = Vec::with_capacity(mask.count_ones());
                    for vi in mask.iter_ones() {
                        let Some(vid) = geometry.vertices().get_id_from_dense(vi) else {
                            continue;
                        };
                        let Some(v) = geometry.vertices().get(vid) else {
                            continue;
                        };
                        let Some(pi) = geometry.points().get_dense_index(v.point_id.into()) else {
                            continue;
                        };
                        let Some(p) = positions.get(pi) else {
                            continue;
                        };
                        pts.push(*p);
                    }
                    if !pts.is_empty() {
                        let mut m =
                            Mesh::new(PrimitiveTopology::PointList, RenderAssetUsages::default());
                        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pts);
                        let mesh_handle = meshes.add(m);
                        commands.entity(entity).with_children(|parent| {
                            parent.spawn((
                                Mesh3d(mesh_handle),
                                PointMarker,
                                SyncToRenderWorld,
                                Transform::default(),
                                Visibility::Visible,
                                GroupOverlayMarker,
                                VertexOverlay,
                            ));
                        });
                    }
                }
            }
        }
    }
}
