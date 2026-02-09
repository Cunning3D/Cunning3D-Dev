use crate::cunning_core::plugin_system::PluginSystem;
use crate::gizmos::material::GizmoOverlayExt;
use crate::gizmos::renderer::GizmoMaterial;
use crate::gizmos::{GizmoBinding, GizmoColor, GizmoInteraction, GizmoTag};
use crate::ui::UiState;
use crate::{nodes::NodeId, NodeGraphResource};
use bevy::prelude::*;

pub fn sync_plugin_gizmos(
    mut commands: Commands,
    ui_state: Res<UiState>,
    node_graph_res: Res<NodeGraphResource>,
    plugin_system: Option<Res<PluginSystem>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GizmoMaterial>>,
    mut line_gizmos: Gizmos<crate::gizmos::SelectedCurveGizmos>,
    // Query existing gizmos
    gizmo_query: Query<(Entity, &GizmoBinding, Option<&GizmoInteraction>), With<GizmoTag>>,
) {
    let Some(plugin_system) = plugin_system else {
        return;
    };
    let selected_node_id = if ui_state.selected_nodes.len() == 1 {
        ui_state.selected_nodes.iter().next().copied()
    } else {
        None
    };
    let Some(node_id) = selected_node_id else {
        cleanup_plugin_gizmos(&mut commands, &gizmo_query);
        return;
    };

    let node_type_key = {
        let ng = &node_graph_res.0;
        ng.nodes
            .get(&node_id)
            .map(|n| n.node_type.name().to_string())
            .unwrap_or_default()
    };
    if node_type_key.is_empty() {
        cleanup_plugin_gizmos(&mut commands, &gizmo_query);
        return;
    }

    // Only for plugin-backed nodes that actually expose interaction.
    if plugin_system.interaction_shared(&node_type_key).is_none() {
        cleanup_plugin_gizmos(&mut commands, &gizmo_query);
        return;
    }

    // Gather current plugin gizmo cmds.
    let cmds = plugin_system.plugin_gizmo_build(&node_graph_res, &node_type_key, node_id, 256);

    // Existing gizmos for this node
    let mut existing: std::collections::HashMap<(u32, u32), (Entity, bool)> =
        std::collections::HashMap::new(); // (node, pick_id) -> (entity, dragged)
    for (entity, binding, interaction) in gizmo_query.iter() {
        if let GizmoBinding::PluginPick {
            node_id: nid,
            pick_id,
        } = binding
        {
            if *nid == node_id {
                existing.insert(
                    (*pick_id, 0),
                    (entity, interaction.map_or(false, |i| i.is_dragged)),
                );
            } else {
                commands.entity(entity).despawn();
            }
        }
    }

    // Spawn/update from cmds
    let mut required: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for c in cmds.iter() {
        match c.tag {
            crate::cunning_core::plugin_system::c_api::CGizmoCmdTag::Line => {
                let a = Vec3::new(c.p0[0], c.p0[1], c.p0[2]);
                let b = Vec3::new(c.p1[0], c.p1[1], c.p1[2]);
                let col = Color::srgba(
                    c.color_rgba[0],
                    c.color_rgba[1],
                    c.color_rgba[2],
                    c.color_rgba[3],
                );
                line_gizmos.line(a, b, col);
            }
            crate::cunning_core::plugin_system::c_api::CGizmoCmdTag::Mesh => {
                required.insert(c.pick_id);
                let binding = GizmoBinding::PluginPick {
                    node_id,
                    pick_id: c.pick_id,
                };
                let pos = Vec3::new(
                    c.transform.translation[0],
                    c.transform.translation[1],
                    c.transform.translation[2],
                );
                let scale = Vec3::new(
                    c.transform.scale[0],
                    c.transform.scale[1],
                    c.transform.scale[2],
                );
                let rot = Quat::from_xyzw(
                    c.transform.rotation_xyzw[0],
                    c.transform.rotation_xyzw[1],
                    c.transform.rotation_xyzw[2],
                    c.transform.rotation_xyzw[3],
                );
                let tf = Transform::from_translation(pos)
                    .with_rotation(rot)
                    .with_scale(scale);
                let col = Color::srgba(
                    c.color_rgba[0],
                    c.color_rgba[1],
                    c.color_rgba[2],
                    c.color_rgba[3],
                );
                let is_sphere = matches!(
                    c.primitive,
                    crate::cunning_core::plugin_system::c_api::CGizmoPrimitive::Sphere
                );
                update_or_spawn_gizmo(
                    &mut commands,
                    &mut existing,
                    &mut meshes,
                    &mut materials,
                    binding,
                    tf,
                    col,
                    is_sphere,
                );
            }
        }
    }

    // Remove unused
    for (pick_id, _z) in existing.keys().cloned().collect::<Vec<_>>() {
        if !required.contains(&pick_id) {
            if let Some((entity, _)) = existing.remove(&(pick_id, 0)) {
                commands.entity(entity).despawn();
            }
        }
    }
}

fn cleanup_plugin_gizmos(
    commands: &mut Commands,
    q: &Query<(Entity, &GizmoBinding, Option<&GizmoInteraction>), With<GizmoTag>>,
) {
    for (e, b, _i) in q.iter() {
        if matches!(b, GizmoBinding::PluginPick { .. }) {
            commands.entity(e).despawn();
        }
    }
}

fn update_or_spawn_gizmo(
    commands: &mut Commands,
    existing: &mut std::collections::HashMap<(u32, u32), (Entity, bool)>,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<GizmoMaterial>>,
    binding: GizmoBinding,
    transform: Transform,
    color: Color,
    is_sphere: bool,
) {
    let pick_id = if let GizmoBinding::PluginPick { pick_id, .. } = &binding {
        *pick_id
    } else {
        0
    };
    if let Some(&(entity, dragged)) = existing.get(&(pick_id, 0)) {
        if !dragged {
            commands.entity(entity).insert(transform);
        }
        return;
    }
    let mesh = if is_sphere {
        meshes.add(Mesh::from(Sphere::new(1.0)))
    } else {
        meshes.add(Mesh::from(Cuboid::from_size(Vec3::ONE)))
    };
    let material = materials.add(GizmoMaterial {
        base: StandardMaterial {
            base_color: color,
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        },
        extension: GizmoOverlayExt::default(),
    });
    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        transform,
        Visibility::Inherited,
        GizmoTag,
        binding,
        GizmoInteraction::default(),
        GizmoColor {
            normal: color,
            hover: Color::srgb(0.0, 1.0, 1.0),
            active: Color::srgb(1.0, 0.5, 0.0),
        },
        bevy::light::NotShadowCaster,
    ));
}
