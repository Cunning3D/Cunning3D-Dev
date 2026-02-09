use crate::cunning_core::command::basic::{CmdSetDisplayNode, CmdToggleFlag};
use crate::gpu_text;
use crate::tabs_system::node_editor::cda;
use crate::{
    nodes::Node,
    tabs_system::{
        node_editor::{
            mathematic::{is_inside, radial_menu_metrics},
            state::{ButtonGeometry, MenuButton, RadialMenuAction},
            NodeEditorTab,
        },
        EditorTabContext,
    },
    theme::ModernTheme,
};
use bevy_egui::egui::{self, Align2, Color32, Pos2, Rect, Stroke, Vec2};
use egui_wgpu::sdf::GpuTextUniform;
use egui_wgpu::sdf::{create_sdf_quad_callback, SdfQuadUniform};

#[inline]
fn rot_ccw_90_vec(v: Vec2) -> Vec2 {
    Vec2::new(v.y, -v.x)
}

#[inline]
fn rot_ccw_90_pos(p: Pos2, center: Pos2) -> Pos2 {
    center + rot_ccw_90_vec(p - center)
}

#[inline]
fn anchor_from_offset(v: Vec2) -> Align2 {
    if v.x.abs() > v.y.abs() {
        if v.x < 0.0 {
            Align2::RIGHT_CENTER
        } else {
            Align2::LEFT_CENTER
        }
    } else {
        if v.y < 0.0 {
            Align2::CENTER_BOTTOM
        } else {
            Align2::CENTER_TOP
        }
    }
}

pub fn draw_radial_menu(
    ui: &mut egui::Ui,
    node_rect: Rect,
    theme: &ModernTheme,
    node: &Node,
    zoom: f32,
    anim_scale: f32,
    s: &crate::node_editor_settings::NodeEditorSettings,
    info_state: &crate::tabs_system::node_editor::state::InfoPanelState,
) -> Option<RadialMenuAction> {
    let painter = ui.painter();
    let pointer_pos = ui.ctx().pointer_interact_pos().unwrap_or(Pos2::ZERO);
    let mut clicked_action: Option<RadialMenuAction> = None;

    let node_center = node_rect.center();
    let max_node_dim = node_rect.width().max(node_rect.height());

    let (inner_padding, thickness, _outer_half_size) = radial_menu_metrics(node_rect, zoom, s);
    let gap =
        ((thickness * s.radial_gap_factor.max(0.0)).max(s.radial_gap_min.max(0.0))) * anim_scale;

    let final_inner_half_size = max_node_dim / 2.0 + inner_padding;
    let final_outer_half_size = final_inner_half_size + thickness;

    let inner_half_size = final_inner_half_size * anim_scale;
    let outer_half_size = final_outer_half_size * anim_scale;

    let inner_rect = Rect::from_center_size(node_center, Vec2::splat(inner_half_size * 2.0));
    let outer_rect = Rect::from_center_size(node_center, Vec2::splat(outer_half_size * 2.0));

    // Visual CCW 90° rotation of the square-ring UI (keep text/logo unrotated).
    let mut buttons = calculate_button_geometries(outer_rect, inner_rect, gap);
    for b in buttons.iter_mut() {
        for p in b.points.iter_mut() {
            *p = rot_ccw_90_pos(*p, node_center);
        }
    }

    let is_primary_clicked = ui.input(|i| i.pointer.primary_clicked());
    let is_primary_down = ui.input(|i| i.pointer.primary_down());

    for button_geom in &buttons {
        let is_hovered = is_inside(&button_geom.points, pointer_pos);
        let is_down = is_hovered && is_primary_down;
        let mut fill_color = theme.colors.panel_background;

        let (is_active, active_color) = match button_geom.button {
            MenuButton::Bypass => (node.is_bypassed, theme.colors.menu_button_bypass_active),
            MenuButton::Visible => (
                node.is_display_node,
                theme.colors.menu_button_visible_active,
            ),
            MenuButton::Temp => (node.is_template, theme.colors.menu_button_template_active),
            MenuButton::Lock => (node.is_locked, theme.colors.menu_button_lock_active),
            MenuButton::Information => (
                info_state.active_node_id == Some(node.id)
                    || info_state.pinned_nodes.contains(&node.id),
                Color32::from_rgb(0, 150, 255),
            ),
            _ => (false, Color32::TRANSPARENT),
        };

        if is_active {
            fill_color = active_color;
        }
        if is_hovered {
            fill_color = fill_color.linear_multiply(1.5);
        }
        if is_down {
            fill_color = fill_color.linear_multiply(0.7);
        }

        let final_fill_color = fill_color.linear_multiply(anim_scale);

        if is_hovered && is_primary_clicked {
            clicked_action = Some(match button_geom.button {
                MenuButton::Bypass => RadialMenuAction::ToggleBypass,
                MenuButton::Visible => RadialMenuAction::ToggleDisplay,
                MenuButton::Temp => RadialMenuAction::ToggleTemplate,
                MenuButton::Lock => RadialMenuAction::ToggleLock,
                MenuButton::Information => RadialMenuAction::ToggleInfo,
                _ => continue,
            });
        }

        if button_geom.points.len() == 4 {
            // Ensure CCW order for SDF quad inside-test.
            let mut pts = button_geom.points.clone();
            let area2 = (pts[0].x * pts[1].y - pts[1].x * pts[0].y)
                + (pts[1].x * pts[2].y - pts[2].x * pts[1].y)
                + (pts[2].x * pts[3].y - pts[3].x * pts[2].y)
                + (pts[3].x * pts[0].y - pts[0].x * pts[3].y);
            if area2 < 0.0 {
                pts.reverse();
            }

            let min_x = pts.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
            let min_y = pts.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            let max_x = pts.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
            let max_y = pts.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
            let rect = Rect::from_min_max(Pos2::new(min_x, min_y), Pos2::new(max_x, max_y));

            let screen_size = ui.ctx().screen_rect().size();
            let fill_rgba = bevy_egui::egui::Rgba::from(final_fill_color).to_array();
            let border_rgba =
                bevy_egui::egui::Rgba::from(Color32::WHITE.linear_multiply(anim_scale)).to_array();
            let uniform = SdfQuadUniform {
                p01: [pts[0].x, pts[0].y, pts[1].x, pts[1].y],
                p23: [pts[2].x, pts[2].y, pts[3].x, pts[3].y],
                fill_color: fill_rgba,
                border_color: border_rgba,
                params: [1.0, 1.0, 0.0, 0.0],
                screen_params: [screen_size.x, screen_size.y, 0.0, 0.0],
            };
            let frame_id = ui.ctx().cumulative_frame_nr();
            painter.add(create_sdf_quad_callback(
                rect.expand(8.0),
                uniform,
                frame_id,
            ));
        } else {
            painter.add(egui::Shape::convex_polygon(
                button_geom.points.clone(),
                final_fill_color,
                Stroke::new(1.0, Color32::WHITE.linear_multiply(anim_scale)),
            ));
        }

        let font_px = 12.0 * zoom.sqrt();
        let font_id = egui::FontId::proportional(font_px);
        let label_color = theme.colors.secondary_text.linear_multiply(anim_scale);
        let base = match button_geom.button {
            MenuButton::Bypass => button_geom.points[0],
            MenuButton::Blame => button_geom.points[1],
            MenuButton::Information => {
                let top = button_geom.points[1];
                let bottom = button_geom.points[0];
                Pos2::new((top.x + bottom.x) * 0.5, (top.y + bottom.y) * 0.5)
            }
            MenuButton::Visible => button_geom.points[0],
            MenuButton::Temp => button_geom.points[1],
            MenuButton::Lock => button_geom.points[1],
            MenuButton::Parm => button_geom.points[0],
        };
        let off0 = match button_geom.button {
            MenuButton::Bypass | MenuButton::Blame => Vec2::new(0.0, -gap),
            MenuButton::Information => Vec2::new(-gap, 0.0),
            MenuButton::Visible | MenuButton::Temp => Vec2::new(gap, 0.0),
            MenuButton::Lock | MenuButton::Parm => Vec2::new(0.0, gap),
        };
        let off = rot_ccw_90_vec(off0);
        let label_pos = base + off;
        let label_anchor = anchor_from_offset(off);
        let label_text = if matches!(button_geom.button, MenuButton::Information) {
            "i".to_string()
        } else {
            format!("{:?}", button_geom.button)
        };
        let galley = ui.fonts_mut(|f| f.layout_no_wrap(label_text.clone(), font_id, label_color));
        let r = label_anchor.anchor_size(label_pos, galley.size());
        let frame_id = ui.ctx().cumulative_frame_nr();
        gpu_text::paint(
            painter,
            GpuTextUniform {
                text: label_text,
                pos: r.min,
                color: label_color,
                font_px,
                bounds: r.size(),
                family: 0,
            },
            frame_id,
        );
    }

    clicked_action
}

fn calculate_button_geometries(outer: Rect, inner: Rect, gap: f32) -> Vec<ButtonGeometry> {
    let mut buttons = Vec::new();
    let o = [
        outer.left_top(),
        outer.right_top(),
        outer.right_bottom(),
        outer.left_bottom(),
    ];
    let i = [
        inner.left_top(),
        inner.right_top(),
        inner.right_bottom(),
        inner.left_bottom(),
    ];
    let half_gap = gap / 2.0;

    let t_mid = (o[0].x + o[1].x) / 2.0;
    let t_left_outer = Pos2::new(t_mid - half_gap, o[0].y);
    let t_right_outer = Pos2::new(t_mid + half_gap, o[1].y);
    let t_left_inner = Pos2::new(t_mid - half_gap, i[0].y);
    let t_right_inner = Pos2::new(t_mid + half_gap, i[1].y);

    buttons.push(ButtonGeometry {
        button: MenuButton::Bypass,
        points: vec![o[0], t_left_outer, t_left_inner, i[0]],
    });
    buttons.push(ButtonGeometry {
        button: MenuButton::Blame,
        points: vec![t_right_outer, o[1], i[1], t_right_inner],
    });

    let r_mid = (o[1].y + o[2].y) / 2.0;
    let r_top_outer = Pos2::new(o[1].x, r_mid - half_gap);
    let r_bottom_outer = Pos2::new(o[2].x, r_mid + half_gap);
    let r_top_inner = Pos2::new(i[1].x, r_mid - half_gap);
    let r_bottom_inner = Pos2::new(i[2].x, r_mid + half_gap);

    buttons.push(ButtonGeometry {
        button: MenuButton::Visible,
        points: vec![o[1], r_top_outer, r_top_inner, i[1]],
    });
    buttons.push(ButtonGeometry {
        button: MenuButton::Temp,
        points: vec![r_bottom_outer, o[2], i[2], r_bottom_inner],
    });

    let b_mid = (o[3].x + o[2].x) / 2.0;
    let b_left_outer = Pos2::new(b_mid - half_gap, o[3].y);
    let b_right_outer = Pos2::new(b_mid + half_gap, o[2].y);
    let b_left_inner = Pos2::new(b_mid - half_gap, i[3].y);
    let b_right_inner = Pos2::new(b_mid + half_gap, i[2].y);

    buttons.push(ButtonGeometry {
        button: MenuButton::Lock,
        points: vec![o[3], b_left_outer, b_left_inner, i[3]],
    });
    buttons.push(ButtonGeometry {
        button: MenuButton::Parm,
        points: vec![b_right_outer, o[2], i[2], b_right_inner],
    });

    buttons.push(ButtonGeometry {
        button: MenuButton::Information,
        points: vec![o[3], o[0], i[0], i[3]],
    });

    buttons
}

pub fn handle_radial_menu(
    editor: &mut NodeEditorTab,
    ui: &mut egui::Ui,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    let hovered_node_id = context.ui_state.radial_menu_state.node_id;
    let mut should_keep_open = false;

    if let Some(node_id) = hovered_node_id {
        let mut action_to_perform = None;
        let mut node_rect_for_action: Option<Rect> = None;

        {
            let root_graph = &context.node_graph_res.0;
            let node_graph = cda::navigation::graph_snapshot_by_path(
                &root_graph,
                &editor.cda_state.breadcrumb(),
            );
            if let Some(node) = node_graph.nodes.get(&node_id) {
                let node_rect = Rect::from_min_size(
                    editor_rect.min + node.position.to_vec2() * editor.zoom + editor.pan,
                    node.size * editor.zoom,
                );
                node_rect_for_action = Some(node_rect);

                // Calculate Menu Rect to check hover persistence (must match draw_radial_menu)
                let s = context.node_editor_settings;
                let menu_rect =
                    crate::tabs_system::node_editor::mathematic::calculate_actual_menu_rect(
                        node_rect,
                        editor.zoom,
                        s,
                    );

                if let Some(ptr) = ui.ctx().pointer_interact_pos() {
                    if menu_rect.contains(ptr) {
                        should_keep_open = true;
                    }
                }

                // Draw Menu
                if let Some(action) = draw_radial_menu(
                    ui,
                    node_rect,
                    context.theme,
                    node,
                    editor.zoom,
                    1.0,
                    context.node_editor_settings,
                    &editor.info_panel_state,
                ) {
                    action_to_perform = Some(action);
                }
            }
        }

        if let Some(action) = action_to_perform {
            let root_graph = &mut context.node_graph_res.0;
            cda::navigation::with_current_graph_mut(
                root_graph,
                &editor.cda_state,
                |node_graph| match action {
                    RadialMenuAction::ToggleBypass => {
                        if let Some(n) = node_graph.nodes.get(&node_id) {
                            context.node_editor_state.execute(
                                Box::new(CmdToggleFlag::new(
                                    node_id,
                                    CmdToggleFlag::BYPASS,
                                    n.is_bypassed,
                                    !n.is_bypassed,
                                )),
                                node_graph,
                            );
                            context.graph_changed_writer.write_default();
                        }
                    }
                    RadialMenuAction::ToggleDisplay => {
                        let old = node_graph.display_node;
                        let new = if node_graph.display_node == Some(node_id) {
                            None
                        } else {
                            Some(node_id)
                        };
                        context
                            .node_editor_state
                            .execute(Box::new(CmdSetDisplayNode::new(old, new)), node_graph);
                        context.graph_changed_writer.write_default();
                    }
                    RadialMenuAction::ToggleTemplate => {
                        if let Some(n) = node_graph.nodes.get(&node_id) {
                            context.node_editor_state.execute(
                                Box::new(CmdToggleFlag::new(
                                    node_id,
                                    CmdToggleFlag::TEMPLATE,
                                    n.is_template,
                                    !n.is_template,
                                )),
                                node_graph,
                            );
                            context.graph_changed_writer.write_default();
                        }
                    }
                    RadialMenuAction::ToggleLock => {
                        if let Some(n) = node_graph.nodes.get(&node_id) {
                            context.node_editor_state.execute(
                                Box::new(CmdToggleFlag::new(
                                    node_id,
                                    CmdToggleFlag::LOCK,
                                    n.is_locked,
                                    !n.is_locked,
                                )),
                                node_graph,
                            );
                            context.graph_changed_writer.write_default();
                        }
                    }
                    RadialMenuAction::ToggleInfo => {
                        // Open a native OS window (Bevy Window) hosting a floating NodeInfoTab.
                        let anchor = node_rect_for_action.unwrap_or(editor_rect);
                        let rect = egui::Rect::from_min_size(
                            anchor.right_top() + egui::vec2(20.0, 0.0),
                            egui::vec2(460.0, 640.0),
                        );
                        context.open_node_info_window_writer.write(
                            crate::ui::OpenNodeInfoWindowEvent {
                                node_id,
                                initial_rect: rect,
                            },
                        );
                    }
                },
            );
            // Reactive mode: ensure we get a follow-up frame so display/flags update visually and caches rebuild.
            editor.cached_nodes_rev = 0;
            editor.geometry_rev = editor.geometry_rev.wrapping_add(1);
            context.ui_invalidator.request_repaint_after_tagged(
                "node_editor/radial_action",
                std::time::Duration::ZERO,
                crate::invalidator::RepaintCause::DataChanged,
            );
        }
    }

    if !should_keep_open {
        context.ui_state.radial_menu_state.node_id = None;
    }
}
