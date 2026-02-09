use crate::cunning_core::cda::library::global_cda_library;
use crate::nodes::structs::NodeType;
use crate::tabs_system::node_editor::icons as node_icons;
use crate::tabs_system::EditorTabContext;
use crate::viewport_options::{
    CameraRotateEvent, CameraViewDirection, SetCameraViewEvent, ViewportViewMode,
};
use bevy_egui::egui::{self, epaint::PathShape};
use egui_wgpu::sdf::{
    create_gpu_text_callback, create_sdf_circle_callback, GpuTextUniform, SdfCircleUniform,
};
use std::f32::consts::PI;

pub fn draw_hud(ui: &mut egui::Ui, context: &mut EditorTabContext, rect: egui::Rect) {
    draw_viewport_gizmo(ui, context, rect);
    draw_active_node_hud(ui, context, rect);
}

fn draw_active_node_hud(ui: &mut egui::Ui, context: &mut EditorTabContext, rect: egui::Rect) {
    let node_id = context
        .ui_state
        .last_selected_node_id
        .or_else(|| context.ui_state.selected_nodes.iter().next().copied());
    let Some(node_id) = node_id else {
        return;
    };
    let (node_name, node_type_key, node_type_label, hud_node_id) = {
        let g = &context.node_graph_res.0;
        let Some(n) = g.nodes.get(&node_id) else {
            return;
        };
        if let NodeType::CDA(data) = &n.node_type {
            let Some(lib) = global_cda_library() else {
                return;
            };
            let _ = lib.ensure_loaded(&data.asset_ref);
            let Some(a) = lib.get(data.asset_ref.uuid) else {
                return;
            };
            let mut hud_id = data.coverlay_hud;
            if hud_id.is_none() {
                let mut huds = a.hud_units.clone();
                huds.sort_by_key(|u| (u.order, u.node_id));
                hud_id = huds
                    .iter()
                    .find(|u| u.is_default)
                    .map(|u| u.node_id)
                    .or_else(|| huds.first().map(|u| u.node_id));
            }
            let Some(hud_id) = hud_id else {
                return;
            };
            let Some(inner) = a.inner_graph.nodes.get(&hud_id) else {
                return;
            };
            let key = inner.node_type.name().to_string();
            let label = context.node_registry.display_name(&key);
            let title = a
                .hud_units
                .iter()
                .find(|u| u.node_id == hud_id)
                .map(|u| u.label.as_str())
                .unwrap_or(label.as_str());
            (
                format!(
                    "{} / {}",
                    if a.name.is_empty() {
                        "CDA"
                    } else {
                        a.name.as_str()
                    },
                    title
                ),
                key,
                label,
                hud_id,
            )
        } else {
            let key = n.node_type.name().to_string();
            let label = context.node_registry.display_name(&key);
            (
                if n.name.is_empty() {
                    label.clone()
                } else {
                    n.name.clone()
                },
                key,
                label,
                node_id,
            )
        }
    };
    let icon = node_icons::icon_for_node_name(&node_name, true);
    let pos = egui::pos2(rect.min.x + 14.0, rect.min.y + 14.0);
    egui::Area::new(egui::Id::new("viewport_active_node_hud"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ui.ctx(), |ui| {
            // Transparent HUD: do NOT paint any background so the 3D viewport shows through.
            let frame = egui::Frame::NONE.inner_margin(egui::Margin::symmetric(10, 8));
            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    let icon_size = egui::vec2(18.0, 18.0);
                    ui.add(egui::Image::new(icon).fit_to_exact_size(icon_size));
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(node_name.as_str()).strong().size(16.0));
                    if !node_type_label.is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(node_type_label.as_str()).weak().small());
                    }
                });
                // Custom node HUD (e.g. Curve tool) if provided by interaction factory.
                let map = context.node_registry.nodes.read().unwrap();
                if let Some(desc) = map
                    .get(&node_type_key)
                    .and_then(|d| d.interaction_factory.as_ref())
                {
                    desc().draw_hud(ui, context, hud_node_id);
                }
            });
        });
}

fn draw_viewport_gizmo(ui: &mut egui::Ui, context: &mut EditorTabContext, rect: egui::Rect) {
    use bevy::prelude::*;

    let gizmo_size = 130.0;
    let margin_right = 75.0;
    let margin_top = 20.0;
    let gizmo_center = egui::pos2(
        rect.max.x - margin_right - gizmo_size / 2.0,
        rect.min.y + margin_top + gizmo_size / 2.0,
    );

    let camera_quat = context.display_options.camera_rotation;
    let is_uv_mode = matches!(context.display_options.view_mode, ViewportViewMode::UV);
    let is_u_menu_mode = matches!(context.display_options.view_mode, ViewportViewMode::UV | ViewportViewMode::NodeImage);

    let layer_id = egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("viewport_gizmo_layer"),
    );
    let painter = egui::Painter::new(ui.ctx().clone(), layer_id, ui.clip_rect());
    let frame_id = (ui.input(|i| i.time) * 1000.0) as u64;
    let screen_size = ui.ctx().screen_rect().size();

    let visual_radius_px = (gizmo_size / 2.0) * 0.55;

    // 0. Drag Interaction (Background)
    // Declared first so it's behind other interactive widgets (like buttons)
    let interact_rect =
        egui::Rect::from_center_size(gizmo_center, egui::vec2(gizmo_size * 1.5, gizmo_size * 1.5));
    let interact_resp = ui.interact(
        interact_rect,
        ui.id().with("gizmo_interact"),
        egui::Sense::click_and_drag(),
    );

    // --- Global Hover ---
    let mouse_pos = ui
        .input(|i| i.pointer.hover_pos())
        .unwrap_or(egui::Pos2::ZERO);
    let dist_to_center = mouse_pos.distance(gizmo_center);
    let is_global_hover = dist_to_center < gizmo_size;
    let global_hover_factor = ui
        .ctx()
        .animate_bool(ui.id().with("gizmo_global_hover"), is_global_hover);

    let apply_global_style = |color: egui::Color32| -> egui::Color32 {
        // Reduced opacity for better transparency ("too white")
        // Idle: 0.3, Active: 0.85
        let alpha_factor = 0.3 + 0.55 * global_hover_factor;
        let r = (color.r() as f32 * alpha_factor) as u8;
        let g = (color.g() as f32 * alpha_factor) as u8;
        let b = (color.b() as f32 * alpha_factor) as u8;
        let a = (color.a() as f32 * alpha_factor) as u8;
        egui::Color32::from_rgba_premultiplied(r, g, b, a)
    };
    let stroke_white =
        bevy_egui::egui::Rgba::from(apply_global_style(egui::Color32::WHITE)).to_array();

    // 1. Function Buttons (U / Pure)
    let mut uv_clicked = false;
    let mut pure_clicked = false;

    {
        let btn_radius = 12.0;
        let btn_gap = 8.0;
        let uv_pos = gizmo_center + egui::vec2(-visual_radius_px - 8.0, -visual_radius_px - 12.0);

        // --- UV Button ---
        let uv_rect =
            egui::Rect::from_center_size(uv_pos, egui::vec2(btn_radius * 2.0, btn_radius * 2.0));
        let uv_resp = ui.allocate_rect(uv_rect, egui::Sense::click());

        let base_uv_color = if is_u_menu_mode {
            egui::Color32::from_rgb(255, 180, 0)
        } else if uv_resp.hovered() {
            egui::Color32::from_rgb(240, 240, 240)
        } else {
            egui::Color32::from_rgb(220, 220, 220)
        };
        let uv_color = apply_global_style(base_uv_color);

        let u = SdfCircleUniform {
            center: [uv_pos.x, uv_pos.y],
            radius: btn_radius,
            border_width: 1.5,
            fill_color: bevy_egui::egui::Rgba::from(uv_color).to_array(),
            border_color: stroke_white,
            softness: 1.0,
            _pad0: 0.0,
            screen_size: [screen_size.x, screen_size.y],
            _pad1: [0.0; 2],
            _pad2: [0.0; 2],
        };
        painter.add(create_sdf_circle_callback(
            uv_rect.expand(2.0),
            painter.clip_rect(),
            u,
            frame_id,
        ));

        let text_color = apply_global_style(egui::Color32::BLACK);
        let galley = ui.fonts_mut(|f| {
            f.layout_no_wrap(
                "U".to_string(),
                egui::FontId::proportional(14.0),
                text_color,
            )
        });
        let r = egui::Align2::CENTER_CENTER.anchor_size(uv_pos, galley.size());
        painter.add(create_gpu_text_callback(
            painter.clip_rect(),
            GpuTextUniform {
                text: "U".to_string(),
                pos: r.min,
                color: text_color,
                font_px: 14.0,
                bounds: r.size(),
                family: 0,
            },
            frame_id,
        ));

        if uv_resp.clicked() {
            uv_clicked = true;
        }

        // View Mode Menu (U)
        {
            let mut chosen: Option<ViewportViewMode> = None;
            egui::Popup::menu(&uv_resp)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    ui.set_min_width(180.0);
                    ui.label("View Mode");
                    ui.separator();
                    if ui
                        .selectable_label(matches!(context.display_options.view_mode, ViewportViewMode::Perspective), "Perspective")
                        .clicked()
                    {
                        chosen = Some(ViewportViewMode::Perspective);
                        ui.close_menu();
                    }
                    if ui
                        .selectable_label(matches!(context.display_options.view_mode, ViewportViewMode::UV), "UV")
                        .clicked()
                    {
                        chosen = Some(ViewportViewMode::UV);
                        ui.close_menu();
                    }
                    if ui
                        .selectable_label(matches!(context.display_options.view_mode, ViewportViewMode::NodeImage), "Selected Node Image")
                        .clicked()
                    {
                        chosen = Some(ViewportViewMode::NodeImage);
                        ui.close_menu();
                    }
                });
            if let Some(m) = chosen {
                context.display_options.view_mode = m;
                uv_clicked = false; // don't auto-toggle after explicit selection
            }
        }

        // --- Pure Mode Button ---
        if is_uv_mode {
            let pure_pos = uv_pos + egui::vec2(0.0, btn_radius * 2.0 + btn_gap);
            let pure_rect = egui::Rect::from_center_size(
                pure_pos,
                egui::vec2(btn_radius * 2.0, btn_radius * 2.0),
            );
            let pure_resp = ui.allocate_rect(pure_rect, egui::Sense::click());

            let base_pure_color = if context.display_options.uv_pure_mode {
                egui::Color32::from_rgb(100, 255, 200)
            } else if pure_resp.hovered() {
                egui::Color32::from_gray(240)
            } else {
                egui::Color32::from_gray(220)
            };
            let pure_color = apply_global_style(base_pure_color);

            let u_pure = SdfCircleUniform {
                center: [pure_pos.x, pure_pos.y],
                radius: btn_radius * 0.7,
                border_width: 1.0,
                fill_color: bevy_egui::egui::Rgba::from(pure_color).to_array(),
                border_color: stroke_white,
                softness: 1.0,
                _pad0: 0.0,
                screen_size: [screen_size.x, screen_size.y],
                _pad1: [0.0; 2],
                _pad2: [0.0; 2],
            };
            painter.add(create_sdf_circle_callback(
                pure_rect.expand(2.0),
                painter.clip_rect(),
                u_pure,
                frame_id,
            ));

            if pure_resp.clicked() {
                pure_clicked = true;
            }
        }
    }

    // 2. ViewCube Core Geometry & Interaction
    let corners = [
        Vec3::new(-1.0, -1.0, -1.0),
        Vec3::new(1.0, -1.0, -1.0),
        Vec3::new(-1.0, 1.0, -1.0),
        Vec3::new(1.0, 1.0, -1.0),
        Vec3::new(-1.0, -1.0, 1.0),
        Vec3::new(1.0, -1.0, 1.0),
        Vec3::new(-1.0, 1.0, 1.0),
        Vec3::new(1.0, 1.0, 1.0),
    ];

    let edges = [
        (4, 5),
        (5, 7),
        (7, 6),
        (6, 4), // Front
        (0, 1),
        (1, 3),
        (3, 2),
        (2, 0), // Back
        (0, 4),
        (1, 5),
        (3, 7),
        (2, 6), // Connecting
    ];

    struct FaceDef {
        normal: Vec3,
        right: Vec3,
        up: Vec3,
        label: &'static str,
        direction: CameraViewDirection,
        indices: [usize; 4],
    }

    let faces = [
        FaceDef {
            normal: Vec3::Z,
            right: Vec3::X,
            up: Vec3::Y,
            label: "Front",
            direction: CameraViewDirection::Front,
            indices: [6, 7, 5, 4],
        },
        FaceDef {
            normal: Vec3::NEG_Z,
            right: Vec3::NEG_X,
            up: Vec3::Y,
            label: "Back",
            direction: CameraViewDirection::Back,
            indices: [3, 2, 0, 1],
        },
        FaceDef {
            normal: Vec3::X,
            right: Vec3::NEG_Z,
            up: Vec3::Y,
            label: "Right",
            direction: CameraViewDirection::Right,
            indices: [7, 3, 1, 5],
        },
        FaceDef {
            normal: Vec3::NEG_X,
            right: Vec3::Z,
            up: Vec3::Y,
            label: "Left",
            direction: CameraViewDirection::Left,
            indices: [2, 6, 4, 0],
        },
        FaceDef {
            normal: Vec3::Y,
            right: Vec3::X,
            up: Vec3::NEG_Z,
            label: "Top",
            direction: CameraViewDirection::Top,
            indices: [6, 2, 3, 7],
        },
        FaceDef {
            normal: Vec3::NEG_Y,
            right: Vec3::X,
            up: Vec3::Z,
            label: "Bottom",
            direction: CameraViewDirection::Bottom,
            indices: [4, 5, 1, 0],
        },
    ];

    let view_inv = camera_quat.inverse();
    let proj_dist = 10.0;
    let scale_base = 2.5;

    let mut proj_points = [egui::Pos2::ZERO; 8];
    for (i, p) in corners.iter().enumerate() {
        let p_view = view_inv * (*p);
        let scale = 1.0 / (proj_dist - p_view.z).max(0.1);
        let x = p_view.x * scale * scale_base * 2.0;
        let y = -p_view.y * scale * scale_base * 2.0;
        proj_points[i] = gizmo_center + egui::vec2(x, y) * (gizmo_size / 2.0);
    }

    let mut visible_faces = Vec::new();
    let mut is_single_view = false;
    for (i, face) in faces.iter().enumerate() {
        let normal_view = view_inv * face.normal;
        if normal_view.z > 0.2 {
            let poly = vec![
                proj_points[face.indices[0]],
                proj_points[face.indices[1]],
                proj_points[face.indices[2]],
                proj_points[face.indices[3]],
            ];
            visible_faces.push((i, face, poly, normal_view.z));
        }
        if normal_view.z > 0.99 {
            is_single_view = true;
        }
    }
    visible_faces.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal));

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum HoverTarget {
        None,
        Face(usize),
        Edge(usize),
        Corner(usize),
    }
    let mut hover_target = HoverTarget::None;

    // Drag Logic (using interact_resp from above)
    if interact_resp.dragged() {
        let delta = interact_resp.drag_delta();
        if delta.length_sq() > 0.0 {
            let sensitivity = 0.005;
            let mut yaw = -delta.x * sensitivity;
            let mut pitch = -delta.y * sensitivity;

            // "Intent" Locking (Axis Snapping)
            // If movement is clearly horizontal (dominant X), kill pitch.
            // If movement is clearly vertical (dominant Y), kill yaw.
            // "Clear intent" threshold: one axis is > 2.0x the other.
            let intent_ratio = 2.5;
            if delta.x.abs() > delta.y.abs() * intent_ratio {
                pitch = 0.0;
            } else if delta.y.abs() > delta.x.abs() * intent_ratio {
                yaw = 0.0;
            }

            // Turntable Logic (Maya Style):
            // Horizontal drag -> Rotate around World Y (Yaw)
            // Vertical drag -> Rotate around Local X (Pitch)

            // 1. World Yaw in Local Space: R_local_yaw = Cam_Inv * R_world_y * Cam
            // Note: camera_quat is the current camera rotation (World <- Local)
            let yaw_world_in_local =
                camera_quat.inverse() * Quat::from_rotation_y(yaw) * camera_quat;

            // 2. Local Pitch (around camera's right axis)
            let pitch_local = Quat::from_rotation_x(pitch);

            // 3. Combine: Apply World Yaw (in local space) THEN Local Pitch
            let rot = yaw_world_in_local * pitch_local;

            context.camera_rotate_writer.write(CameraRotateEvent {
                rotation: rot,
                immediate: true,
            });
        }
    } else if let Some(mouse_pos) = ui.input(|i| i.pointer.hover_pos()) {
        let mut visible_corner_indices = std::collections::HashSet::new();
        let mut visible_edge_indices = std::collections::HashSet::new();
        for (f_idx, _, _, _) in &visible_faces {
            for &idx in &faces[*f_idx].indices {
                visible_corner_indices.insert(idx);
            }
            for k in 0..4 {
                let c1 = faces[*f_idx].indices[k];
                let c2 = faces[*f_idx].indices[(k + 1) % 4];
                if let Some(e_idx) = edges
                    .iter()
                    .position(|e| (e.0 == c1 && e.1 == c2) || (e.0 == c2 && e.1 == c1))
                {
                    visible_edge_indices.insert(e_idx);
                }
            }
        }

        // 1. Corners
        for &c_idx in &visible_corner_indices {
            if mouse_pos.distance(proj_points[c_idx]) < 8.0 {
                // Reduced from 12.0 for precision
                hover_target = HoverTarget::Corner(c_idx);
                break;
            }
        }
        // 2. Edges
        if hover_target == HoverTarget::None {
            for &e_idx in &visible_edge_indices {
                let (c1, c2) = edges[e_idx];
                let p1 = proj_points[c1];
                let p2 = proj_points[c2];
                let segment = p2 - p1;
                let len_sq = segment.length_sq();
                if len_sq > 0.0 {
                    let t = ((mouse_pos - p1).dot(segment) / len_sq).clamp(0.0, 1.0);
                    if mouse_pos.distance(p1 + segment * t) < 5.0 {
                        // Reduced from 8.0 for precision
                        hover_target = HoverTarget::Edge(e_idx);
                        break;
                    }
                }
            }
        }
        // 3. Faces
        if hover_target == HoverTarget::None {
            for (f_idx, _, poly, _) in visible_faces.iter().rev() {
                let mut inside = false;
                let n = poly.len();
                for i in 0..n {
                    let j = (i + 1) % n;
                    if ((poly[i].y > mouse_pos.y) != (poly[j].y > mouse_pos.y))
                        && (mouse_pos.x
                            < (poly[j].x - poly[i].x) * (mouse_pos.y - poly[i].y)
                                / (poly[j].y - poly[i].y)
                                + poly[i].x)
                    {
                        inside = !inside;
                    }
                }
                if inside {
                    hover_target = HoverTarget::Face(*f_idx);
                    break;
                }
            }
        }
    }

    if interact_resp.clicked() {
        match hover_target {
            HoverTarget::Face(idx) => {
                context
                    .set_camera_view_writer
                    .write(SetCameraViewEvent(faces[idx].direction));
            }
            HoverTarget::Corner(idx) => {
                let dir = corners[idx].normalize();
                context
                    .set_camera_view_writer
                    .write(SetCameraViewEvent(CameraViewDirection::Custom(dir)));
            }
            HoverTarget::Edge(idx) => {
                let (c1, c2) = edges[idx];
                let mid = (corners[c1] + corners[c2]).normalize();
                context
                    .set_camera_view_writer
                    .write(SetCameraViewEvent(CameraViewDirection::Custom(mid)));
            }
            _ => {}
        }
    }

    let axis_color = |dir: Vec3| {
        let x = dir.x.abs();
        let y = dir.y.abs();
        let z = dir.z.abs();
        if x > 0.9 {
            egui::Color32::from_rgb(220, 60, 60)
        } else if y > 0.9 {
            egui::Color32::from_rgb(60, 220, 60)
        } else if z > 0.9 {
            egui::Color32::from_rgb(60, 80, 220)
        } else {
            egui::Color32::from_rgb(0, 120, 215)
        }
    };

    for (f_idx, face, poly, z) in &visible_faces {
        let is_hovered = match hover_target {
            HoverTarget::Face(i) => i == *f_idx,
            _ => false,
        };
        let brightness = z.max(0.2);

        let base_val = (240.0 * (0.7 + 0.3 * brightness)).clamp(0.0, 255.0) as u8;
        let mut fill_color = egui::Color32::from_gray(base_val);

        if is_hovered {
            let tint = axis_color(face.normal);
            let r = (fill_color.r() as f32 * 0.7 + tint.r() as f32 * 0.3) as u8;
            let g = (fill_color.g() as f32 * 0.7 + tint.g() as f32 * 0.3) as u8;
            let b = (fill_color.b() as f32 * 0.7 + tint.b() as f32 * 0.3) as u8;
            fill_color = egui::Color32::from_rgb(r, g, b);
        }

        fill_color = apply_global_style(fill_color);
        let stroke_color = fill_color.linear_multiply(0.6);
        let stroke_width = 1.0 + 2.0 * z.max(0.0).powi(2);

        // Generate Rounded Polygon (Screen-space rounding)
        // This creates "Real Rounded Corners" instead of just thick strokes.
        let mut rounded_poly = Vec::new();
        let corner_radius: f32 = 5.0; // Radius in pixels

        for i in 0..4 {
            let p = poly[i];
            let prev = poly[(i + 3) % 4];
            let next = poly[(i + 1) % 4];

            let v_prev = (prev - p).normalized();
            let v_next = (next - p).normalized();
            let dist_prev = p.distance(prev);
            let dist_next = p.distance(next);

            // Clamp radius to avoid edge overlapping on small faces
            let r = corner_radius.min(dist_prev * 0.45).min(dist_next * 0.45);

            // Quadratic Bezier approximation for arc
            // P0 = Start, P1 = Control(Corner), P2 = End
            let p0 = p + v_prev * r;
            let p2 = p + v_next * r;

            let steps = 4;
            for s in 0..=steps {
                let t = s as f32 / steps as f32;
                // Quadratic Bezier formula
                let v = (1.0 - t).powi(2) * p0.to_vec2()
                    + 2.0 * (1.0 - t) * t * p.to_vec2()
                    + t.powi(2) * p2.to_vec2();
                rounded_poly.push(egui::pos2(v.x, v.y));
            }
        }

        // Gradient Fill (Mesh with Triangle Fan)
        let mut mesh = egui::Mesh::default();

        // Add Center Vertex (Fan Center)
        let center_vec =
            (poly[0].to_vec2() + poly[1].to_vec2() + poly[2].to_vec2() + poly[3].to_vec2()) / 4.0;
        let center_pos = egui::pos2(center_vec.x, center_vec.y);

        let c_rel_y = (center_pos.y - (gizmo_center.y - gizmo_size * 0.6)) / (gizmo_size * 1.2);
        let c_grad = 1.0 - c_rel_y.clamp(0.0, 1.0) * 0.5;
        let c_color = fill_color.linear_multiply(c_grad);

        mesh.vertices.push(egui::epaint::Vertex {
            pos: center_pos,
            uv: egui::epaint::WHITE_UV,
            color: c_color,
        });

        // Add Rim Vertices (Rounded Poly)
        for &p in &rounded_poly {
            let rel_y = (p.y - (gizmo_center.y - gizmo_size * 0.6)) / (gizmo_size * 1.2);
            let grad = 1.0 - rel_y.clamp(0.0, 1.0) * 0.5;
            let v_color = fill_color.linear_multiply(grad);
            mesh.vertices.push(egui::epaint::Vertex {
                pos: p,
                uv: egui::epaint::WHITE_UV,
                color: v_color,
            });
        }

        // Indices (Fan)
        let n_verts = rounded_poly.len();
        for i in 0..n_verts {
            mesh.add_triangle(0, 1 + i as u32, 1 + ((i + 1) % n_verts) as u32);
        }
        painter.add(egui::Shape::mesh(mesh));

        // Stroke (Frame) - using rounded path
        painter.add(egui::Shape::Path(PathShape {
            points: rounded_poly.clone(),
            closed: true,
            fill: egui::Color32::TRANSPARENT,
            stroke: egui::Stroke::new(stroke_width, stroke_color).into(),
        }));

        if is_hovered {
            let inner_poly: Vec<egui::Pos2> = rounded_poly
                .iter()
                .map(|p| center_pos + (*p - center_pos) * 0.7)
                .collect();
            let highlight_color = apply_global_style(axis_color(face.normal));
            painter.add(egui::Shape::Path(PathShape {
                points: inner_poly,
                closed: true,
                fill: highlight_color.linear_multiply(0.5),
                stroke: egui::Stroke::NONE.into(),
            }));
        }

        let font_size = 48.0;
        let text_color = apply_global_style(egui::Color32::from_gray(60));
        let galley = ui.fonts_mut(|f| {
            f.layout_no_wrap(
                face.label.to_string(),
                egui::FontId::proportional(font_size),
                text_color,
            )
        });
        let shapes = vec![egui::epaint::ClippedShape {
            clip_rect: egui::Rect::EVERYTHING,
            shape: egui::Shape::galley(egui::Pos2::ZERO, galley.clone(), text_color),
        }];
        let clipped_meshes = ui.ctx().tessellate(shapes, ui.ctx().pixels_per_point());
        for clipped in clipped_meshes {
            if let egui::epaint::Primitive::Mesh(mut mesh) = clipped.primitive {
                let text_size = galley.size();
                let face_center_local = (corners[face.indices[0]]
                    + corners[face.indices[1]]
                    + corners[face.indices[2]]
                    + corners[face.indices[3]])
                    / 4.0;
                for v in &mut mesh.vertices {
                    let local_x = v.pos.x - text_size.x / 2.0;
                    let local_y = v.pos.y - text_size.y / 2.0;
                    let text_scale = 0.012;
                    let offset_3d =
                        face.right * local_x * text_scale + (-face.up) * local_y * text_scale;
                    let p_world = face_center_local + offset_3d + face.normal * 0.02;
                    let p_view = view_inv * p_world;
                    let scale = 1.0 / (proj_dist - p_view.z).max(0.1);
                    v.pos = gizmo_center
                        + egui::vec2(p_view.x, -p_view.y)
                            * scale
                            * scale_base
                            * 2.0
                            * (gizmo_size / 2.0);
                    v.color = text_color;
                }
                painter.add(egui::Shape::mesh(mesh));
            }
        }
    }

    if let HoverTarget::Edge(idx) = hover_target {
        let (c1, c2) = edges[idx];
        let p1 = proj_points[c1];
        let p2 = proj_points[c2];
        let dir = (corners[c1] + corners[c2]).normalize();
        let color = apply_global_style(axis_color(dir));
        painter.add(egui::Shape::line_segment(
            [p1, p2],
            egui::Stroke::new(4.0, color),
        ));
    }

    if let HoverTarget::Corner(idx) = hover_target {
        let p = proj_points[idx];
        let dir = corners[idx].normalize();
        let color = apply_global_style(axis_color(dir));
        painter.add(egui::Shape::circle_filled(p, 6.0, color));
    }

    // 3. Single View Controls (Maya Style)
    if is_single_view {
        let tri_dist = visual_radius_px + 6.0;
        let tri_size = 8.0;
        let arrow_color_idle = apply_global_style(egui::Color32::from_gray(180));
        let arrow_color_hover = apply_global_style(egui::Color32::WHITE);

        let mut draw_triangle = |dir: egui::Vec2, is_orbit_y: bool, angle: f32| {
            let center = gizmo_center + dir * tri_dist;
            let p1 = center - dir * tri_size; // Tip
            let dir_rot90 = egui::vec2(-dir.y, dir.x);
            let p2 = center + dir * tri_size + dir_rot90 * tri_size * 0.6; // Base 1
            let p3 = center + dir * tri_size - dir_rot90 * tri_size * 0.6; // Base 2
            let pts = vec![p1, p2, p3];

            let rect = egui::Rect::from_min_max(
                egui::pos2(center.x - tri_size, center.y - tri_size),
                egui::pos2(center.x + tri_size, center.y + tri_size),
            );
            let resp = ui.allocate_rect(rect, egui::Sense::click());
            let color = if resp.hovered() {
                arrow_color_hover
            } else {
                arrow_color_idle
            };

            painter.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));

            if resp.clicked() {
                let rot = if is_orbit_y {
                    camera_quat.inverse() * Quat::from_rotation_y(angle) * camera_quat
                } else {
                    Quat::from_rotation_x(angle)
                };
                context.camera_rotate_writer.write(CameraRotateEvent {
                    rotation: rot,
                    immediate: false,
                });
            }
        };

        draw_triangle(egui::vec2(0.0, -1.0), false, PI / 2.0);
        draw_triangle(egui::vec2(0.0, 1.0), false, -PI / 2.0);
        draw_triangle(egui::vec2(-1.0, 0.0), true, -PI / 2.0);
        draw_triangle(egui::vec2(1.0, 0.0), true, PI / 2.0);

        let rot_arrow_center =
            gizmo_center + egui::vec2(gizmo_size * 0.5 + 20.0, -gizmo_size * 0.5 - 5.0);
        let arrow_radius = 10.0;

        let mut draw_curve_arrow = |is_cw: bool, offset: egui::Vec2| {
            let center = rot_arrow_center + offset;
            let rect = egui::Rect::from_center_size(center, egui::vec2(24.0, 24.0));
            let resp = ui.allocate_rect(rect, egui::Sense::click());
            let color = if resp.hovered() {
                arrow_color_hover
            } else {
                arrow_color_idle
            };

            let mut points = Vec::new();
            let steps = 12;
            let rot_offset = if is_cw { -PI / 4.0 } else { PI / 4.0 };
            let (base_start, base_end) = if is_cw { (PI, 0.0) } else { (0.0, PI) };
            let start_angle = base_start + rot_offset;
            let end_angle = base_end + rot_offset;

            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let angle = start_angle + (end_angle - start_angle) * t;
                points.push(center + egui::vec2(angle.cos(), -angle.sin()) * arrow_radius * 0.8);
            }

            painter.add(PathShape::line(
                points.clone(),
                egui::Stroke::new(2.0, color),
            ));

            let tip = *points.last().unwrap();
            let prev = points[points.len() - 2];
            let dir = (tip - prev).normalized();

            let head_len = 6.0;
            let head_width = 4.0;
            let normal = egui::vec2(-dir.y, dir.x);

            let head_pts = vec![
                tip,
                tip - dir * head_len + normal * head_width,
                tip - dir * head_len - normal * head_width,
            ];

            painter.add(egui::Shape::convex_polygon(
                head_pts,
                color,
                egui::Stroke::NONE,
            ));

            if resp.clicked() {
                let angle = if is_cw { -PI / 2.0 } else { PI / 2.0 };
                context.camera_rotate_writer.write(CameraRotateEvent {
                    rotation: Quat::from_rotation_z(angle),
                    immediate: false,
                });
            }
        };

        draw_curve_arrow(false, egui::vec2(-14.0, 0.0));
        draw_curve_arrow(true, egui::vec2(14.0, 0.0));
    }

    if uv_clicked {
        context.display_options.view_mode = if is_u_menu_mode { ViewportViewMode::Perspective } else { ViewportViewMode::UV };
    }
    if pure_clicked {
        context.display_options.uv_pure_mode = !context.display_options.uv_pure_mode;
    }
}
