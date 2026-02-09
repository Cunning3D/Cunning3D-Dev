use crate::invalidator::RepaintCause;
use crate::tabs_system::node_editor::state::NodeEditorTab;
use crate::tabs_system::EditorTabContext;
use bevy_egui::egui::{
    Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
};
use std::time::Duration;

/// Draws the Head-Up Display (HUD) for navigation controls.
/// Layout: [Pan Joystick] [Zoom Slider] (Left -> Right)
/// Location: Top-Right of the editor.
pub fn draw_hud(
    ui: &mut Ui,
    state: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    // --- Layout Constants ---
    let margin = 20.0;
    let gap = 12.0;

    // Joystick
    let joystick_radius = 40.0;
    let joystick_size = joystick_radius * 2.0;

    // Zoom Slider
    let slider_width = 30.0;
    let slider_height = joystick_size + 10.0; // Slightly taller than joystick diameter

    // Calculate Top-Right Anchor Position
    // We anchor everything relative to the top-right corner of the canvas
    let anchor_x = editor_rect.max.x - margin;
    let anchor_y = editor_rect.min.y + margin;

    // 2. Zoom Slider (Rightmost)
    let zoom_rect_min_x = anchor_x - slider_width;
    let zoom_rect_min_y = anchor_y;
    let zoom_rect = Rect::from_min_size(
        Pos2::new(zoom_rect_min_x, zoom_rect_min_y),
        Vec2::new(slider_width, slider_height),
    );

    // 1. Pan Joystick (Left of Zoom Slider)
    let joystick_center_x = zoom_rect_min_x - gap - joystick_radius;
    let joystick_center_y = zoom_rect_min_y + (slider_height / 2.0); // Center vertically with slider
    let joystick_center = Pos2::new(joystick_center_x, joystick_center_y);

    // --- Draw Components ---
    draw_pan_joystick(ui, state, context, joystick_center, joystick_radius);
    draw_zoom_slider(ui, state, context, zoom_rect, editor_rect);
}

fn draw_pan_joystick(
    ui: &mut Ui,
    state: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    center: Pos2,
    radius: f32,
) {
    let painter = ui.painter();
    let joystick_id = ui.make_persistent_id("hud_pan_joystick");

    // Interaction
    // We make the interaction area slightly larger for touch friendliness
    let interact_rect = Rect::from_center_size(center, Vec2::splat(radius * 2.2));
    let response = ui.interact(interact_rect, joystick_id, Sense::drag());

    // --- Visual Style ---
    let bg_color = Color32::from_rgba_premultiplied(20, 20, 20, 200);
    let border_color = Color32::from_gray(80);
    let border_stroke = Stroke::new(1.0, border_color);
    let arrow_color = Color32::from_gray(120);
    let active_color = Color32::from_rgb(100, 150, 255); // Blue-ish for active state

    // 1. Base (Outer Ring)
    painter.circle_filled(center, radius, bg_color);
    painter.circle_stroke(center, radius, border_stroke);

    // 2. Directional Arrows
    let arrow_dist = radius * 0.65;
    let arrow_size = 5.0;

    let draw_arrow = |angle_deg: f32, offset: Vec2| {
        let rot = angle_deg.to_radians();
        let p = center + offset;
        let tip = p + Vec2::new(rot.cos(), rot.sin()) * arrow_size;
        // Two base points to form a triangle
        let base_angle_1 = rot + 2.5; // roughly 140 degrees
        let base_angle_2 = rot - 2.5;
        let p2 = p + Vec2::new(base_angle_1.cos(), base_angle_1.sin()) * arrow_size;
        let p3 = p + Vec2::new(base_angle_2.cos(), base_angle_2.sin()) * arrow_size;

        painter.add(Shape::convex_polygon(
            vec![tip, p2, p3],
            arrow_color,
            Stroke::NONE,
        ));
    };

    draw_arrow(0.0, Vec2::new(arrow_dist, 0.0)); // Right
    draw_arrow(90.0, Vec2::new(0.0, arrow_dist)); // Down
    draw_arrow(180.0, Vec2::new(-arrow_dist, 0.0)); // Left
    draw_arrow(270.0, Vec2::new(0.0, -arrow_dist)); // Up

    // 3. Logic & Thumb Stick
    let mut thumb_offset = Vec2::ZERO;
    let mut is_active = false;

    if response.dragged() {
        if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
            let mut vec = pointer_pos - center;
            // Clamp to radius
            if vec.length() > radius {
                vec = vec.normalized() * radius;
            }
            thumb_offset = vec;
            is_active = true;

            // Apply Pan Movement
            // Sensitivity needs to be high enough to be useful
            let sensitivity = 15.0;
            let deadzone = 5.0; // pixels

            if thumb_offset.length() > deadzone {
                // Invert vector because dragging "right" usually means "look right",
                // which means moving the viewport coordinates (pan) to the LEFT.
                // However, in 2D canvas, "dragging the world".
                // If I pull the stick RIGHT, I expect the VIEW to move RIGHT (seeing what's on the right).
                // So I need to subtract from pan.
                let move_vec = (thumb_offset / radius) * sensitivity;
                state.pan -= move_vec;
                state.target_pan = state.pan;
                let moved = ui.input(|i| {
                    i.events.iter().any(|e| {
                        matches!(
                            e,
                            bevy_egui::egui::Event::PointerMoved(_)
                                | bevy_egui::egui::Event::MouseMoved(_)
                        )
                    })
                });
                if moved {
                    context.ui_invalidator.request_repaint_after_tagged(
                        "node_editor/hud_drag",
                        Duration::ZERO,
                        RepaintCause::Input,
                    );
                }
            }
        }
    }

    // Draw Thumb
    let thumb_radius = radius * 0.35;
    let thumb_pos = center + thumb_offset;

    let thumb_color = if is_active {
        active_color
    } else {
        Color32::from_gray(180)
    };

    // Thumb Shadow/Glow
    if is_active {
        painter.circle_filled(
            thumb_pos,
            thumb_radius + 2.0,
            active_color.gamma_multiply(0.3),
        );
    }

    painter.circle_filled(thumb_pos, thumb_radius, thumb_color);
    painter.circle_stroke(thumb_pos, thumb_radius, Stroke::new(1.0, Color32::BLACK));

    // Decorate Thumb (grip lines)
    painter.circle_stroke(
        thumb_pos,
        thumb_radius * 0.6,
        Stroke::new(1.0, Color32::from_black_alpha(50)),
    );
}

fn draw_zoom_slider(
    ui: &mut Ui,
    state: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    rect: Rect,
    editor_rect: Rect,
) {
    let painter = ui.painter();
    let slider_id = ui.make_persistent_id("hud_zoom_slider");

    let response = ui.interact(rect, slider_id, Sense::drag());

    // --- Visual Style ---
    let bg_color = Color32::from_rgba_premultiplied(20, 20, 20, 200);
    let border_color = Color32::from_gray(80);
    let rounding = CornerRadius::from(12.0);
    let active_color = Color32::from_rgb(100, 150, 255);

    // 1. Background Pill
    painter.rect_filled(rect, rounding, bg_color);
    painter.rect_stroke(
        rect,
        rounding,
        Stroke::new(1.0, border_color),
        StrokeKind::Inside,
    );

    // 2. Icons
    let text_color = Color32::from_gray(150);
    let center_x = rect.center().x;

    // "+" at top
    painter.text(
        Pos2::new(center_x, rect.min.y + 12.0),
        Align2::CENTER_CENTER,
        "+",
        FontId::proportional(16.0),
        text_color,
    );
    // "-" at bottom
    painter.text(
        Pos2::new(center_x, rect.max.y - 12.0),
        Align2::CENTER_CENTER,
        "-",
        FontId::proportional(20.0), // Minus sign visually needs to be larger to match weight
        text_color,
    );

    // 3. Center Neutral Line (Faint)
    let center_y = rect.center().y;
    painter.line_segment(
        [
            Pos2::new(rect.min.x + 8.0, center_y),
            Pos2::new(rect.max.x - 8.0, center_y),
        ],
        Stroke::new(1.0, Color32::from_gray(60)),
    );

    // 4. Logic & Thumb
    // Elastic logic: The thumb moves with the mouse, but snaps back when released.
    // The displacement from center determines the Zoom Velocity.

    let mut thumb_offset_y = 0.0;
    let max_thumb_travel = (rect.height() / 2.0) - 20.0; // Limit travel so it doesn't cover icons
    let mut is_active = false;

    if response.dragged() {
        if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
            // Calculate distance from vertical center of the slider
            let dy = pointer_pos.y - center_y;

            // Clamp visual travel
            thumb_offset_y = dy.clamp(-max_thumb_travel, max_thumb_travel);
            is_active = true;

            // Apply Zoom
            // Drag Up (negative dy) -> Zoom In
            // Drag Down (positive dy) -> Zoom Out
            // Map offset to speed
            let speed_factor = 0.003;
            let zoom_change = -thumb_offset_y * speed_factor * state.zoom;

            let min_zoom = 0.1;
            let max_zoom = 5.0;

            let old_zoom = state.zoom;
            let new_zoom = (state.zoom + zoom_change).clamp(min_zoom, max_zoom);

            if (new_zoom - old_zoom).abs() > f32::EPSILON {
                // --- ZOOM FOCUS LOGIC ---
                // We want to zoom towards the "Center of Interest" in World Space.
                // Formula: new_pan = old_pan + world_focus * (old_zoom - new_zoom)

                // 1. Determine World Focus Point
                let world_focus_point = if !context.ui_state.selected_nodes.is_empty() {
                    // Average position of selected nodes
                    let mut sum_pos = Vec2::ZERO;
                    let mut count = 0.0;

                    for n in &state.cached_nodes {
                        if context.ui_state.selected_nodes.contains(&n.id) {
                            let node_center = n.position + n.size / 2.0;
                            sum_pos += node_center.to_vec2();
                            count += 1.0;
                        }
                    }

                    if count > 0.0 {
                        let v = sum_pos / count;
                        Pos2::new(v.x, v.y)
                    } else {
                        // Fallback if lock fails or nodes missing (unlikely)
                        let v = (editor_rect.center().to_vec2() - state.pan) / old_zoom;
                        Pos2::new(v.x, v.y)
                    }
                } else {
                    // No selection: Zoom towards center of screen (editor rect)
                    // WorldPos = (ScreenPos - Pan) / Zoom
                    let v = (editor_rect.center().to_vec2() - state.pan) / old_zoom;
                    Pos2::new(v.x, v.y)
                };

                // 2. Apply Pan Correction
                let world_vec = world_focus_point.to_vec2();
                let pan_correction = world_vec * (old_zoom - new_zoom);

                state.pan += pan_correction;
                state.zoom = new_zoom;

                state.target_pan = state.pan;
                state.target_zoom = state.zoom;
            }

            let moved = ui.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        bevy_egui::egui::Event::PointerMoved(_)
                            | bevy_egui::egui::Event::MouseMoved(_)
                    )
                })
            });
            if moved {
                context.ui_invalidator.request_repaint_after_tagged(
                    "node_editor/hud_drag",
                    Duration::ZERO,
                    RepaintCause::Input,
                );
            }
        }
    }

    // Draw Thumb
    let thumb_center = Pos2::new(center_x, center_y + thumb_offset_y);
    let thumb_size = Vec2::new(rect.width() - 8.0, 16.0); // Wide horizontal bar
    let thumb_rect = Rect::from_center_size(thumb_center, thumb_size);
    let thumb_rounding = CornerRadius::from(4.0);

    let thumb_fill = if is_active {
        active_color
    } else {
        Color32::from_gray(100)
    };

    if is_active {
        // Glow
        painter.rect_filled(
            thumb_rect.expand(2.0),
            thumb_rounding,
            active_color.gamma_multiply(0.3),
        );
    }

    painter.rect_filled(thumb_rect, thumb_rounding, thumb_fill);

    // Add 3 little grip lines on the thumb
    let grip_color = if is_active {
        Color32::BLACK
    } else {
        Color32::from_gray(50)
    };
    for i in -1..=1 {
        let y = thumb_center.y + (i as f32 * 3.0);
        painter.line_segment(
            [
                Pos2::new(thumb_rect.min.x + 4.0, y),
                Pos2::new(thumb_rect.max.x - 4.0, y),
            ],
            Stroke::new(1.0, grip_color),
        );
    }
}
