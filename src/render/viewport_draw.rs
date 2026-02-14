//! Viewport drawing systems for grid, labels, and gizmos.
//!
//! This module contains systems for drawing viewport overlays:
//! - Grid lines (major, minor, axis)
//! - Grid labels
//! - Point/primitive/vertex numbers
//! - Normal vectors
//! - Selection highlights

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use bevy::render::sync_world::SyncToRenderWorld;

use crate::{
    gizmos::{GridAxisGizmos, GridGizmos, GridMajorGizmos},
    nodes::NodeGraphResource,
    render::primitive_number::{PrimitiveNumberData, PrimitiveNumberMarker},
    scene::components::FinalWireframeTag,
    tabs_system::viewport_3d::{
        grid::grid_params::grid_params,
        ViewportLayout,
    },
    tabs_system::{TabViewer, Viewport3DTab},
    ui::{ComponentSelectionMode, UiState},
    viewport_options::{DisplayOptions, ViewportViewMode},
    MainCamera,
};

#[derive(Component)]
pub(crate) struct PointNumbersTag;
#[derive(Component)]
pub(crate) struct PrimitiveNumbersTag;
#[derive(Component)]
pub(crate) struct VertexNumbersTag;

/// Draws the viewport grid with minor, major, and axis lines.
pub(crate) fn draw_grid(
    mut minor_gizmos: Gizmos<GridGizmos>,
    mut major_gizmos: Gizmos<GridMajorGizmos>,
    mut axis_gizmos: Gizmos<GridAxisGizmos>,
    display_options: Res<DisplayOptions>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    viewport_layout: Res<ViewportLayout>,
) {
    if !display_options.grid.show || display_options.view_mode == ViewportViewMode::UV {
        return;
    }
    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };
    let vp = viewport_layout
        .logical_rect
        .map(|r| r.size())
        .unwrap_or_default();
    if vp.x <= 1.0 || vp.y <= 1.0 {
        return;
    }
    let Some(p) = grid_params(
        camera,
        camera_transform,
        vp,
        display_options.grid.major_target_px,
    ) else {
        return;
    };
    let (minor_step, center, half_extent, draw_minor) =
        (p.minor_step, p.center, p.half_extent, p.draw_minor);

    // Houdini-like Grid Colors (Monochrome, distinct by width)
    let minor_c = Color::srgba(0.4, 0.4, 0.4, 0.1); // Very faint
    let major_c = Color::srgba(0.5, 0.5, 0.5, 0.3); // Visible
    let axis_c = Color::srgba(0.6, 0.6, 0.6, 0.5); // Slightly stronger axis

    let i0 = ((center.x - half_extent) / minor_step).floor() as i32;
    let i1 = ((center.x + half_extent) / minor_step).ceil() as i32;
    let k0 = ((center.z - half_extent) / minor_step).floor() as i32;
    let k1 = ((center.z + half_extent) / minor_step).ceil() as i32;

    // Density-based fading for minor lines
    let mut fade = 1.0;
    if let Some(ndc0) = camera.world_to_ndc(camera_transform, center) {
        if let Some(ndc1) = camera.world_to_ndc(camera_transform, center + Vec3::X * minor_step) {
            let px0 = Vec2::new((ndc0.x + 1.0) * 0.5 * vp.x, (1.0 - ndc0.y) * 0.5 * vp.y);
            let px1 = Vec2::new((ndc1.x + 1.0) * 0.5 * vp.x, (1.0 - ndc1.y) * 0.5 * vp.y);
            let px_per_minor = (px1 - px0).length();
            fade = ((px_per_minor - 5.0) / 15.0).clamp(0.0, 1.0);
        }
    }

    let mut minor_c_faded = minor_c;
    minor_c_faded = minor_c_faded.with_alpha(minor_c.alpha() * fade);

    for i in i0..=i1 {
        if !draw_minor && i % 5 != 0 {
            continue;
        }
        let x = i as f32 * minor_step;
        if i == 0 {
            axis_gizmos.line(
                Vec3::new(x, 0.0, center.z - half_extent),
                Vec3::new(x, 0.0, center.z + half_extent),
                axis_c,
            );
        } else if i % 5 == 0 {
            major_gizmos.line(
                Vec3::new(x, 0.0, center.z - half_extent),
                Vec3::new(x, 0.0, center.z + half_extent),
                major_c,
            );
        } else if fade > 0.01 {
            minor_gizmos.line(
                Vec3::new(x, 0.0, center.z - half_extent),
                Vec3::new(x, 0.0, center.z + half_extent),
                minor_c_faded,
            );
        }
    }
    for k in k0..=k1 {
        if !draw_minor && k % 5 != 0 {
            continue;
        }
        let z = k as f32 * minor_step;
        if k == 0 {
            axis_gizmos.line(
                Vec3::new(center.x - half_extent, 0.0, z),
                Vec3::new(center.x + half_extent, 0.0, z),
                axis_c,
            );
        } else if k % 5 == 0 {
            major_gizmos.line(
                Vec3::new(center.x - half_extent, 0.0, z),
                Vec3::new(center.x + half_extent, 0.0, z),
                major_c,
            );
        } else if fade > 0.01 {
            minor_gizmos.line(
                Vec3::new(center.x - half_extent, 0.0, z),
                Vec3::new(center.x + half_extent, 0.0, z),
                minor_c_faded,
            );
        }
    }
}

/// Draws point numbers (indices) at each point position.
pub(crate) fn draw_point_numbers_system(
    mut commands: Commands,
    node_graph_res: Res<NodeGraphResource>,
    display_options: Res<DisplayOptions>,
    old_markers: Query<Entity, With<PointNumbersTag>>,
) {
    if !display_options.overlays.show_point_numbers {
        for entity in &old_markers {
            commands.entity(entity).despawn();
        }
        return;
    }
    let node_graph = &node_graph_res.0;
    let geo = &node_graph.final_geometry;
    if let Some(positions) = geo.get_point_attribute("@P").and_then(|a| a.as_slice::<Vec3>()) {
        let count = positions.len();
        let mut values = Vec::with_capacity(count);
        let mut pos_vec = Vec::with_capacity(count);
        for (i, &p) in positions.iter().enumerate() {
            values.push(i as u32);
            pos_vec.push(p);
        }
        if values.is_empty() {
            for entity in &old_markers {
                commands.entity(entity).despawn();
            }
            return;
        }
        let data = PrimitiveNumberData {
            values,
            positions: pos_vec,
            color: Vec4::new(0.3, 0.7, 1.0, 1.0),
        };
        let mut updated = false;
        for entity in &old_markers {
            commands.entity(entity).insert(data.clone());
            updated = true;
        }
        if !updated {
            commands.spawn((
                Name::new("PointNumbers"),
                FinalWireframeTag,
                PrimitiveNumberMarker,
                PointNumbersTag,
                data,
                SyncToRenderWorld,
                Transform::default(),
                Visibility::Visible,
            ));
        }
    }
}

pub(crate) fn draw_primitive_numbers_system(
    mut commands: Commands,
    node_graph_res: Res<NodeGraphResource>,
    display_options: Res<DisplayOptions>,
    old_markers: Query<Entity, With<PrimitiveNumbersTag>>,
) {
    if !display_options.overlays.show_primitive_numbers {
        for entity in &old_markers {
            commands.entity(entity).despawn();
        }
        return;
    }
    let node_graph = &node_graph_res.0;
    let geo = &node_graph.final_geometry;
    if let Some(positions) = geo.get_point_attribute("@P").and_then(|a| a.as_slice::<Vec3>()) {
        let primitives = geo.primitives();
        let vertices = geo.vertices();
        let points = geo.points();
        let count = primitives.len();
        let mut values = Vec::with_capacity(count);
        let mut pos_vec = Vec::with_capacity(count);
        for (i, prim) in primitives.iter().enumerate() {
            let mut center = Vec3::ZERO;
            let mut v_count = 0.0;
            for vid in prim.vertices() {
                if let Some(v) = vertices.get((*vid).into()) {
                    if let Some(pid) = points.get_dense_index(v.point_id.into()) {
                        if let Some(p) = positions.get(pid) {
                            center += *p;
                            v_count += 1.0;
                        }
                    }
                }
            }
            if v_count > 0.0 {
                center /= v_count;
            }
            values.push(i as u32);
            pos_vec.push(center);
        }
        if values.is_empty() {
            for entity in &old_markers {
                commands.entity(entity).despawn();
            }
            return;
        }
        let data = PrimitiveNumberData {
            values,
            positions: pos_vec,
            color: Vec4::new(1.0, 0.9, 0.2, 1.0),
        };
        let mut updated = false;
        for entity in &old_markers {
            commands.entity(entity).insert(data.clone());
            updated = true;
        }
        if !updated {
            commands.spawn((
                Name::new("PrimitiveNumbers"),
                FinalWireframeTag,
                PrimitiveNumberMarker,
                PrimitiveNumbersTag,
                data,
                SyncToRenderWorld,
                Transform::default(),
                Visibility::Visible,
            ));
        }
    }
}

pub(crate) fn draw_vertex_numbers_system(
    mut commands: Commands,
    node_graph_res: Res<NodeGraphResource>,
    display_options: Res<DisplayOptions>,
    old_markers: Query<Entity, With<VertexNumbersTag>>,
) {
    if !display_options.overlays.show_vertex_numbers {
        for entity in &old_markers {
            commands.entity(entity).despawn();
        }
        return;
    }
    let node_graph = &node_graph_res.0;
    let geo = &node_graph.final_geometry;
    if let Some(positions) = geo.get_point_attribute("@P").and_then(|a| a.as_slice::<Vec3>()) {
        let vertices = geo.vertices();
        let points = geo.points();
        let count = vertices.len();
        let mut values = Vec::with_capacity(count);
        let mut pos_vec = Vec::with_capacity(count);
        for (i, v) in vertices.iter().enumerate() {
            if let Some(pid) = points.get_dense_index(v.point_id.into()) {
                if let Some(p) = positions.get(pid) {
                    values.push(i as u32);
                    pos_vec.push(*p);
                }
            }
        }
        if values.is_empty() {
            for entity in &old_markers {
                commands.entity(entity).despawn();
            }
            return;
        }
        let data = PrimitiveNumberData {
            values,
            positions: pos_vec,
            color: Vec4::new(0.3, 1.0, 0.3, 1.0),
        };
        let mut updated = false;
        for entity in &old_markers {
            commands.entity(entity).insert(data.clone());
            updated = true;
        }
        if !updated {
            commands.spawn((
                Name::new("VertexNumbers"),
                FinalWireframeTag,
                PrimitiveNumberMarker,
                VertexNumbersTag,
                data,
                SyncToRenderWorld,
                Transform::default(),
                Visibility::Visible,
            ));
        }
    }
}

/// Draws grid labels (coordinate numbers) on major grid lines.
pub(crate) fn draw_grid_labels_system(
    display_options: Res<DisplayOptions>,
    mut egui_contexts: EguiContexts,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    tab_viewer: Res<TabViewer>,
    viewport_layout: Res<ViewportLayout>,
) {
    if !display_options.grid.show
        || !display_options.grid.show_labels
        || display_options.view_mode == ViewportViewMode::UV
    {
        return;
    }
    let viewport_rect = if let Some(rect) = tab_viewer
        .dock_state
        .iter_all_tabs()
        .find_map(|((_s, _n), tab)| tab.as_any().downcast_ref::<Viewport3DTab>())
        .and_then(|t| t.viewport_rect)
    {
        rect
    } else {
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };
    let vp = viewport_layout
        .logical_rect
        .map(|r| r.size())
        .unwrap_or_default();
    if vp.x <= 1.0 || vp.y <= 1.0 {
        return;
    }
    let Some(p) = grid_params(
        camera,
        camera_transform,
        vp,
        display_options.grid.major_target_px,
    ) else {
        return;
    };
    let Some(ctx) = egui_contexts.try_ctx_mut() else {
        return;
    };
    let mut painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        "grid_labels".into(),
    ));
    painter.set_clip_rect(viewport_rect);
    let half = p.half_extent;
    let i0 = ((p.center.x - half) / p.major_step).floor() as i32;
    let i1 = ((p.center.x + half) / p.major_step).ceil() as i32;
    let k0 = ((p.center.z - half) / p.major_step).floor() as i32;
    let k1 = ((p.center.z + half) / p.major_step).ceil() as i32;
    let max_labels = 32i32;
    let sx = ((i1 - i0 + 1).max(1) / max_labels).max(1);
    let sz = ((k1 - k0 + 1).max(1) / max_labels).max(1);
    for i in (i0..=i1).step_by(sx as usize) {
        let x = i as f32 * p.major_step;
        draw_grid_label_gpu(
            &mut painter,
            camera,
            camera_transform,
            viewport_rect,
            Vec3::new(x, 0.0, 0.0),
            x,
            p.major_step,
            true,
        );
    }
    for k in (k0..=k1).step_by(sz as usize) {
        if k == 0 {
            continue;
        }
        let z = k as f32 * p.major_step;
        draw_grid_label_gpu(
            &mut painter,
            camera,
            camera_transform,
            viewport_rect,
            Vec3::new(0.0, 0.0, z),
            z,
            p.major_step,
            false,
        );
    }
}

/// Helper function to draw a single grid label in GPU space.
fn draw_grid_label_gpu(
    painter: &mut egui::Painter,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    viewport_rect: egui::Rect,
    world: Vec3,
    v_m: f32,
    major_step: f32,
    is_x_axis: bool,
) {
    let Some(ndc) = camera.world_to_ndc(camera_transform, world) else {
        return;
    };
    if ndc.z <= 0.0 {
        return;
    }
    let x = viewport_rect.min.x + (ndc.x + 1.0) * 0.5 * viewport_rect.width();
    let y = viewport_rect.min.y + (1.0 - ndc.y) * 0.5 * viewport_rect.height();
    let label = if major_step >= 1.0 {
        format!("{:.0}", v_m)
    } else if major_step >= 0.1 {
        format!("{:.1}", v_m)
    } else if major_step >= 0.01 {
        format!("{:.2}", v_m)
    } else {
        format!("{:.3}", v_m)
    };
    let fg = if is_x_axis {
        egui::Color32::from_rgb(245, 245, 245)
    } else {
        egui::Color32::from_rgb(232, 232, 232)
    };
    let outline = egui::Color32::from_rgba_unmultiplied(0, 0, 0, 250);
    let font_id = egui::FontId::proportional(16.0);
    let galley = painter.layout(label.clone(), font_id.clone(), fg, f32::INFINITY);
    let size = galley.size();
    let baseline_nudge = if is_x_axis { -10.0 } else { 10.0 };
    let rect = egui::Rect::from_min_size(
        egui::pos2((x - size.x * 0.5).round(), (y - size.y * 0.5 + baseline_nudge).round()),
        size,
    );
    if viewport_rect.intersects(rect) {
        let bg = rect.expand2(egui::vec2(5.0, 3.0));
        painter.rect_filled(
            bg,
            egui::CornerRadius::same(4),
            egui::Color32::from_rgba_unmultiplied(8, 8, 8, 175),
        );
        // Multi-pass outline for a "sticker / texture" bold look.
        let outline_galley = painter.layout(label.clone(), font_id.clone(), outline, f32::INFINITY);
        let outline_offsets = [
            egui::vec2(-1.0, 0.0),
            egui::vec2(1.0, 0.0),
            egui::vec2(0.0, -1.0),
            egui::vec2(0.0, 1.0),
            egui::vec2(-1.0, -1.0),
            egui::vec2(1.0, -1.0),
            egui::vec2(-1.0, 1.0),
            egui::vec2(1.0, 1.0),
        ];
        for offset in outline_offsets {
            painter.galley(rect.min + offset, outline_galley.clone(), outline);
        }
        let fill_galley = painter.layout(label, font_id, fg, f32::INFINITY);
        painter.galley(rect.min + egui::vec2(0.0, 0.5), fill_galley, fg);
        painter.galley(rect.min, galley, fg);
    }
}

pub(crate) fn draw_template_wireframes_system(
    node_graph_res: Res<NodeGraphResource>,
    mut egui_contexts: EguiContexts,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    tab_viewer: Res<TabViewer>,
) {
    let Some(viewport_rect) = tab_viewer
        .dock_state
        .iter_all_tabs()
        .find_map(|((_s, _n), tab)| tab.as_any().downcast_ref::<Viewport3DTab>())
        .and_then(|t| t.viewport_rect)
    else {
        return;
    };
    let Some(ctx) = egui_contexts.try_ctx_mut() else { return; };
    let mut painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, "template_wireframes".into()));
    painter.set_clip_rect(viewport_rect);
    let node_graph = &node_graph_res.0;
    let Ok((camera, camera_transform)) = camera_query.single() else { return; };
    for node in node_graph.nodes.values() {
        if !node.is_template { continue; }
        let Some(geo) = node_graph.geometry_cache.get(&node.id) else { continue; };
        let Some(positions) = geo.get_point_attribute("@P").and_then(|a| a.as_slice::<Vec3>()) else { continue; };
        let Some(normals) = geo.get_vertex_attribute("@N").and_then(|a| a.as_slice::<Vec3>()) else { continue; };
        let primitive_is_front_facing: Vec<bool> = geo
            .primitives()
            .values()
            .iter()
            .map(|primitive| {
                let vertices = primitive.vertices();
                if vertices.is_empty() { return false; }
                let (center, avg_normal) = vertices.iter().fold((Vec3::ZERO, Vec3::ZERO), |(pos_acc, norm_acc), &v_idx| {
                    let point_idx = geo
                        .vertices()
                        .get(v_idx.into())
                        .and_then(|v| geo.points().get_dense_index(v.point_id.into()))
                        .unwrap_or(0);
                    let pos = positions.get(point_idx).copied().unwrap_or(Vec3::ZERO);
                    let v_dense = geo.vertices().get_dense_index(v_idx.into()).unwrap_or(0);
                    let norm = normals.get(v_dense).copied().unwrap_or(Vec3::ZERO);
                    (pos_acc + pos, norm_acc + norm)
                });
                let count = vertices.len() as f32;
                let center = center / count;
                let avg_normal = (avg_normal / count).normalize_or_zero();
                let cam_to_prim = (center - camera_transform.translation()).normalize_or_zero();
                avg_normal.dot(cam_to_prim) < 0.0
            })
            .collect();
        let edge_map = geo.build_edge_to_primitive_map();
        for (edge, prim_indices) in edge_map.iter() {
            let is_front_facing = prim_indices.iter().any(|&pi| {
                geo.primitives()
                    .get_dense_index(pi.into())
                    .map(|i| primitive_is_front_facing[i])
                    .unwrap_or(false)
            });
            let color = if is_front_facing { egui::Color32::from_gray(200) } else { egui::Color32::from_gray(80) };
            let p1_idx = geo.points().get_dense_index(edge.0.into());
            let p2_idx = geo.points().get_dense_index(edge.1.into());
            if let (Some(p1), Some(p2)) = (p1_idx.and_then(|i| positions.get(i)), p2_idx.and_then(|i| positions.get(i))) {
                if let (Some(ndc1), Some(ndc2)) = (
                    camera.world_to_ndc(camera_transform, *p1),
                    camera.world_to_ndc(camera_transform, *p2),
                ) {
                    if ndc1.z > 0.0 && ndc2.z > 0.0 {
                        let pos1 = egui::pos2(
                            viewport_rect.min.x + (ndc1.x + 1.0) / 2.0 * viewport_rect.width(),
                            viewport_rect.min.y + (1.0 - ndc1.y) / 2.0 * viewport_rect.height(),
                        );
                        let pos2 = egui::pos2(
                            viewport_rect.min.x + (ndc2.x + 1.0) / 2.0 * viewport_rect.width(),
                            viewport_rect.min.y + (1.0 - ndc2.y) / 2.0 * viewport_rect.height(),
                        );
                        painter.line_segment([pos1, pos2], egui::Stroke::new(1.0, color));
                    }
                }
            }
        }
    }
}

pub(crate) fn highlight_selected_components(
    mut commands: Commands,
    ui_state: Res<UiState>,
    node_graph_res: Res<NodeGraphResource>,
    query_highlights: Query<Entity, Or<(With<crate::scene::components::HighlightPointTag>, With<crate::scene::components::HighlightPrimitiveTag>)>>,
    mut egui_contexts: EguiContexts,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    tab_viewer: Res<TabViewer>,
) {
    if ui_state.component_selection.indices.is_empty() {
        for entity in &query_highlights {
            commands.entity(entity).despawn();
        }
        return;
    }
    let Some(viewport_rect) = tab_viewer
        .dock_state
        .iter_all_tabs()
        .find_map(|((_s, _n), tab)| tab.as_any().downcast_ref::<Viewport3DTab>())
        .and_then(|t| t.viewport_rect)
    else {
        return;
    };
    let mut painter = egui_contexts.ctx_mut().layer_painter(egui::LayerId::new(egui::Order::Foreground, "highlights".into()));
    painter.set_clip_rect(viewport_rect);
    let Ok((camera, camera_transform)) = camera_query.single() else { return; };
    // If graph is temporarily busy, keep last highlights to avoid flicker.
    let node_graph = &node_graph_res.0;
    // Rebuild highlights.
    for entity in &query_highlights {
        commands.entity(entity).despawn();
    }
    if let Some(node_id) = ui_state.last_selected_node_id {
        if node_graph.nodes.get(&node_id).is_none() { return; }
        if let Some(geo) = node_graph.geometry_cache.get(&node_id) {
            match ui_state.component_selection.mode {
                ComponentSelectionMode::Points => {
                    if let Some(positions) = geo.get_point_attribute("@P").and_then(|a| a.as_slice::<Vec3>()) {
                        for index in &ui_state.component_selection.indices {
                            if let Some(pos) = positions.get(*index) {
                                if let Some(ndc) = camera.world_to_ndc(camera_transform, *pos) {
                                    if ndc.z > 0.0 {
                                        let screen_pos = egui::pos2(
                                            viewport_rect.min.x + (ndc.x + 1.0) / 2.0 * viewport_rect.width(),
                                            viewport_rect.min.y + (1.0 - ndc.y) / 2.0 * viewport_rect.height(),
                                        );
                                        painter.circle_filled(screen_pos, 3.0, egui::Color32::from_rgb(255, 0, 0));
                                    }
                                }
                            }
                        }
                    }
                }
                ComponentSelectionMode::Primitives => {
                    if let Some(positions) = geo.get_point_attribute("@P").and_then(|a| a.as_slice::<Vec3>()) {
                        for index in &ui_state.component_selection.indices {
                            if let Some(prim_id) = geo.primitives().get_id_from_dense(*index) {
                                if let Some(primitive) = geo.primitives().get(prim_id) {
                                    let points_2d: Vec<egui::Pos2> = primitive
                                        .vertices()
                                        .iter()
                                        .filter_map(|&v_idx| geo.vertices().get(v_idx.into()))
                                        .filter_map(|v| geo.points().get_dense_index(v.point_id.into()))
                                        .filter_map(|idx| positions.get(idx))
                                        .filter_map(|pos_3d| camera.world_to_ndc(camera_transform, *pos_3d))
                                        .filter(|ndc| ndc.z > 0.0)
                                        .map(|ndc| egui::pos2(
                                            viewport_rect.min.x + (ndc.x + 1.0) / 2.0 * viewport_rect.width(),
                                            viewport_rect.min.y + (1.0 - ndc.y) / 2.0 * viewport_rect.height(),
                                        ))
                                        .collect();
                                    if !points_2d.is_empty() {
                                        painter.add(egui::Shape::closed_line(points_2d, egui::Stroke::new(2.0, egui::Color32::YELLOW)));
                                    }
                                }
                            }
                        }
                    }
                }
                ComponentSelectionMode::Edges => {
                    if let Some(positions) = geo.get_point_attribute("@P").and_then(|a| a.as_slice::<Vec3>()) {
                        for index in &ui_state.component_selection.indices {
                            if let Some(edge_id) = geo.edges().get_id_from_dense(*index) {
                                if let Some(edge) = geo.edges().get(edge_id) {
                                    let p0_idx = geo.points().get_dense_index(edge.p0.into());
                                    let p1_idx = geo.points().get_dense_index(edge.p1.into());
                                    if let (Some(p0_i), Some(p1_i)) = (p0_idx, p1_idx) {
                                        if let (Some(p0), Some(p1)) = (positions.get(p0_i), positions.get(p1_i)) {
                                            if let (Some(ndc0), Some(ndc1)) = (
                                                camera.world_to_ndc(camera_transform, *p0),
                                                camera.world_to_ndc(camera_transform, *p1),
                                            ) {
                                                if ndc0.z > 0.0 && ndc1.z > 0.0 {
                                                    let s0 = egui::pos2(
                                                        viewport_rect.min.x + (ndc0.x + 1.0) / 2.0 * viewport_rect.width(),
                                                        viewport_rect.min.y + (1.0 - ndc0.y) / 2.0 * viewport_rect.height(),
                                                    );
                                                    let s1 = egui::pos2(
                                                        viewport_rect.min.x + (ndc1.x + 1.0) / 2.0 * viewport_rect.width(),
                                                        viewport_rect.min.y + (1.0 - ndc1.y) / 2.0 * viewport_rect.height(),
                                                    );
                                                    painter.line_segment([s0, s1], egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 255, 255)));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
