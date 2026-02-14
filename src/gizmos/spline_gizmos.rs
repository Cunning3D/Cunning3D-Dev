use crate::gizmos::material::GizmoOverlayExt;
use crate::gizmos::renderer::GizmoMaterial;
use crate::gizmos::{GizmoBinding, GizmoColor, GizmoInteraction, GizmoTag};
use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::spline_cache_utility::get_sampled_positions;
use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::spline_selection_utility::is_selectable_tangent;
use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::SelectableElement;
use crate::libs::algorithms::algorithms_runtime::unity_spline::{
    are_tangents_modifiable, BezierTangent, SelectableKnot, SplineContainer,
};
use crate::nodes::parameter::ParameterValue;
use crate::nodes::spline::tool_state::SplineToolState;
use crate::ui::UiState;
use crate::{
    nodes::{NodeId, NodeType},
    NodeGraphResource,
};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;

#[derive(Default)]
pub(crate) struct SplineGizmoCache {
    node_id: Option<NodeId>,
    container: Option<SplineContainer>,
    samples_per_curve: usize,
    polylines: Vec<Vec<Vec3>>,
    knot_mesh: Option<Handle<Mesh>>,
    tangent_mesh: Option<Handle<Mesh>>,
}

pub fn sync_spline_gizmos(
    mut commands: Commands,
    ui_state: Res<UiState>,
    node_graph_res: Res<NodeGraphResource>,
    spline_tool_state: Res<SplineToolState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GizmoMaterial>>,
    mut giz_front: Gizmos<crate::gizmos::SelectedCurveGizmos>,
    mut giz_xray: Gizmos<crate::gizmos::SelectedCurveXrayGizmos>,
    gizmo_query: Query<(Entity, &GizmoBinding, Option<&GizmoInteraction>), With<GizmoTag>>,
    mut cache: Local<SplineGizmoCache>,
) {
    let selected_node_id = if ui_state.selected_nodes.len() == 1 {
        ui_state.selected_nodes.iter().next().copied()
    } else {
        None
    };
    let mut current: Option<(NodeId, SplineContainer)> = None;
    if let Some(node_id) = selected_node_id {
        let node_graph = &node_graph_res.0;
        if let Some(node) = node_graph.nodes.get(&node_id) {
            if matches!(node.node_type, NodeType::Spline) {
                if let Some(param) = node.parameters.iter().find(|p| p.name == "spline") {
                    if let ParameterValue::UnitySpline(c) = &param.value {
                        current = Some((node_id, c.clone()));
                    }
                }
            }
        }
    }

    let mut existing: std::collections::HashMap<GizmoBinding, (Entity, bool)> =
        std::collections::HashMap::new();
    for (entity, binding, interaction) in gizmo_query.iter() {
        match binding {
            GizmoBinding::SplineKnot { node_id, .. }
            | GizmoBinding::SplineTangent { node_id, .. } => {
                if Some(*node_id) == selected_node_id {
                    let is_dragged = interaction.map_or(false, |i| i.is_dragged);
                    existing.insert(binding.clone(), (entity, is_dragged));
                } else {
                    commands.entity(entity).despawn();
                }
            }
            _ => {}
        }
    }

    let Some((node_id, c)) = current else {
        return;
    };
    if c.splines.is_empty() {
        return;
    }
    if cache.knot_mesh.is_none() {
        cache.knot_mesh = Some(meshes.add(teardrop_mesh_unit()));
    }
    if cache.tangent_mesh.is_none() {
        cache.tangent_mesh = Some(meshes.add(Mesh::from(Sphere::new(1.0))));
    }

    // Draw polyline preview (all splines).
    let samples = 30usize;
    if cache.node_id != Some(node_id)
        || cache.samples_per_curve != samples
        || cache.container.as_ref() != Some(&c)
    {
        cache.node_id = Some(node_id);
        cache.samples_per_curve = samples;
        cache.container = Some(c.clone());
        cache.polylines.clear();
        for si in 0..c.splines.len() {
            let mut s = c.splines[si].clone();
            cache
                .polylines
                .push(get_sampled_positions(&mut s, c.local_to_world, samples));
        }
    }
    let col_front = Color::srgb(0.6, 0.9, 1.0);
    let col_xray = Color::srgba(0.6, 0.9, 1.0, 0.3);
    for pts in cache.polylines.iter() {
        if pts.len() < 2 {
            continue;
        }
        for i in 0..(pts.len() - 1) {
            giz_front.line(pts[i], pts[i + 1], col_front);
            giz_xray.line(pts[i], pts[i + 1], col_xray);
        }
    }
    if let Some(h) = spline_tool_state.hovered_curve {
        giz_front.sphere(h.world_pos, 0.035, Color::srgb(1.0, 0.9, 0.2));
        giz_xray.sphere(h.world_pos, 0.035, Color::srgba(1.0, 0.9, 0.2, 0.2));
    }

    let mut required = std::collections::HashSet::new();
    for si in 0..c.splines.len() {
        for ki in 0..c.splines[si].count() {
            required.insert(GizmoBinding::SplineKnot {
                node_id,
                spline_index: si,
                knot_index: ki,
            });
            let mode = c.splines[si].meta[ki].mode;
            let sel_k = SelectableElement::Knot(SelectableKnot {
                spline_index: si,
                knot_index: ki,
            });
            let show_tangents = spline_tool_state
                .selection
                .is_selected_or_adjacent_to_selected(sel_k);
            if show_tangents && are_tangents_modifiable(mode) {
                if is_selectable_tangent(&c.splines[si], ki, BezierTangent::In) {
                    required.insert(GizmoBinding::SplineTangent {
                        node_id,
                        spline_index: si,
                        knot_index: ki,
                        tangent: BezierTangent::In,
                    });
                }
                if is_selectable_tangent(&c.splines[si], ki, BezierTangent::Out) {
                    required.insert(GizmoBinding::SplineTangent {
                        node_id,
                        spline_index: si,
                        knot_index: ki,
                        tangent: BezierTangent::Out,
                    });
                }
            }
        }
    }

    let keys: Vec<_> = existing.keys().cloned().collect();
    for k in keys {
        if !required.contains(&k) {
            if let Some((e, _)) = existing.remove(&k) {
                commands.entity(e).despawn();
            }
        }
    }

    for si in 0..c.splines.len() {
        for ki in 0..c.splines[si].count() {
            let knot = c.splines[si].knots[ki];
            let pos = c.local_to_world.transform_point3(knot.position);
            let sel_k = SelectableElement::Knot(SelectableKnot {
                spline_index: si,
                knot_index: ki,
            });
            let is_active = spline_tool_state.selection.is_active(sel_k);
            let is_selected = spline_tool_state.selection.contains(sel_k);
            let base_col = if is_active {
                Color::srgb(1.0, 0.5, 0.0)
            } else if is_selected {
                Color::srgb(1.0, 0.9, 0.2)
            } else {
                Color::srgb(0.6, 0.9, 1.0)
            };
            // World-space teardrop: flat in local XZ plane, tip points along local +Z.
            // Orientation matches Unity semantics: derived from knot.rotation + tangent_out (no neighbor fallback).
            let parent_rot = c.local_to_world.to_scale_rotation_translation().1;
            let knot_rot_world = parent_rot * knot.rotation;
            let mut fwd = knot_rot_world.mul_vec3(knot.tangent_out);
            if fwd.length_squared() < 1e-8 {
                // When tangent_out is zero, use the knot's rotation as the only source of orientation.
                fwd = knot_rot_world.mul_vec3(Vec3::Z);
            }
            fwd = fwd.normalize_or_zero();
            let mut up = knot_rot_world.mul_vec3(Vec3::Y).normalize_or_zero();
            let mut right = up.cross(fwd);
            if right.length_squared() < 1e-8 {
                // Degenerate (up parallel to fwd): resolve using knot's local X axis, still no neighbor-based fallback.
                right = knot_rot_world.mul_vec3(Vec3::X).normalize_or_zero();
                up = fwd.cross(right).normalize_or_zero();
            } else {
                right = right.normalize_or_zero();
                up = fwd.cross(right).normalize_or_zero();
            }
            let rot = Quat::from_mat3(&Mat3::from_cols(right, up, fwd));
            update_or_spawn_gizmo_with_mesh(
                &mut commands,
                &mut existing,
                cache.knot_mesh.as_ref().unwrap(),
                &mut materials,
                GizmoBinding::SplineKnot {
                    node_id,
                    spline_index: si,
                    knot_index: ki,
                },
                pos,
                rot,
                Vec3::splat(0.06),
                base_col,
            );

            let mode = c.splines[si].meta[ki].mode;
            let show_tangents = spline_tool_state
                .selection
                .is_selected_or_adjacent_to_selected(sel_k);
            if show_tangents && are_tangents_modifiable(mode) {
                for &t in &[BezierTangent::In, BezierTangent::Out] {
                    if !is_selectable_tangent(&c.splines[si], ki, t) {
                        continue;
                    }
                    let tan_local = if t == BezierTangent::In {
                        knot.tangent_in
                    } else {
                        knot.tangent_out
                    };
                    let tan_world = c
                        .local_to_world
                        .transform_point3(knot.position + knot.rotation.mul_vec3(tan_local));
                    let sel_t = SelectableElement::Tangent(crate::libs::algorithms::algorithms_runtime::unity_spline::SelectableTangent { spline_index: si, knot_index: ki, tangent: t });
                    let is_active_t = spline_tool_state.selection.is_active(sel_t);
                    let is_selected_t = spline_tool_state.selection.contains(sel_t);
                    let tan_col = if is_active_t {
                        Color::srgb(1.0, 0.5, 0.0)
                    } else if is_selected_t {
                        Color::srgb(1.0, 0.9, 0.2)
                    } else {
                        Color::srgb(0.2, 0.9, 0.8)
                    };
                    update_or_spawn_gizmo_with_mesh(
                        &mut commands,
                        &mut existing,
                        cache.tangent_mesh.as_ref().unwrap(),
                        &mut materials,
                        GizmoBinding::SplineTangent {
                            node_id,
                            spline_index: si,
                            knot_index: ki,
                            tangent: t,
                        },
                        tan_world,
                        Quat::IDENTITY,
                        Vec3::splat(0.035),
                        tan_col,
                    );
                    giz_front.line(pos, tan_world, Color::srgba(0.2, 0.9, 0.8, 0.9));
                    giz_xray.line(pos, tan_world, Color::srgba(0.2, 0.9, 0.8, 0.25));
                }
            }
        }
    }
}

fn update_or_spawn_gizmo_with_mesh(
    commands: &mut Commands,
    existing: &mut std::collections::HashMap<GizmoBinding, (Entity, bool)>,
    mesh: &Handle<Mesh>,
    materials: &mut ResMut<Assets<GizmoMaterial>>,
    binding: GizmoBinding,
    pos: Vec3,
    rot: Quat,
    scale: Vec3,
    color: Color,
) {
    if let Some(&(entity, is_dragged)) = existing.get(&binding) {
        let _ = is_dragged; // Spline gizmos are driven by spline data; keep pose updating even while dragged.
        commands.entity(entity).insert(
            Transform::from_translation(pos)
                .with_rotation(rot)
                .with_scale(scale),
        );
        commands.entity(entity).insert(GizmoColor {
            normal: color,
            hover: Color::srgb(0.0, 1.0, 1.0),
            active: Color::srgb(1.0, 0.5, 0.0),
        });
    } else {
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
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(pos)
                .with_rotation(rot)
                .with_scale(scale),
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
}

fn teardrop_mesh_unit() -> Mesh {
    // Flat teardrop in local XZ plane, facing +Y. Tip points along local +Z.
    // World orientation is controlled by Transform rotation, and size by Transform scale.
    let segs: usize = 18;
    let tip = Vec3::new(0.0, 0.0, 1.0);
    let center = Vec3::new(0.0, 0.0, -0.2);
    let r = 0.6;

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(segs + 1);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(segs + 1);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(segs + 1);
    let mut indices: Vec<u32> = Vec::with_capacity(segs * 3);

    positions.push([tip.x, tip.y, tip.z]);
    normals.push([0.0, 1.0, 0.0]);
    uvs.push([0.5, 1.0]);

    for i in 0..segs {
        let t = i as f32 / segs as f32 * std::f32::consts::TAU;
        let p = center + Vec3::new(t.cos() * r, 0.0, t.sin() * r);
        positions.push([p.x, p.y, p.z]);
        normals.push([0.0, 1.0, 0.0]);
        uvs.push([
            0.5 + (p.x / (2.0 * r)).clamp(-0.5, 0.5),
            0.5 + ((p.z - center.z) / (2.0 * r)).clamp(-0.5, 0.5),
        ]);
    }

    for i in 1..segs {
        indices.push(0);
        indices.push(i as u32);
        indices.push((i + 1) as u32);
    }
    indices.push(0);
    indices.push(segs as u32);
    indices.push(1);

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}
