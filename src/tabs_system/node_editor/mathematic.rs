use crate::node_editor_settings::NodeEditorSettings;
use bevy_egui::egui::{Pos2, Rect, Vec2};

#[inline]
pub fn radial_menu_metrics(node_rect: Rect, zoom: f32, s: &NodeEditorSettings) -> (f32, f32, f32) {
    let max_node_dim = node_rect.width().max(node_rect.height());
    let inner_base =
        (s.radial_inner_padding_base.max(0.0) * zoom).max(s.radial_inner_padding_min.max(0.0));
    let thickness_base =
        (s.radial_thickness_base.max(0.0) * zoom).max(s.radial_thickness_min.max(0.0));
    let inner_padding = inner_base + max_node_dim * s.radial_inner_padding_ratio.max(0.0);
    let thickness = thickness_base + max_node_dim * s.radial_thickness_ratio.max(0.0);
    let outer_half_size =
        max_node_dim * 0.5 + inner_padding + thickness + s.radial_activation_margin.max(0.0);
    (inner_padding, thickness, outer_half_size)
}

/// A helper function to calculate the menu's bounding box without drawing it.
pub fn calculate_menu_rect(node_rect: Rect, zoom: f32, s: &NodeEditorSettings) -> Rect {
    calculate_actual_menu_rect(node_rect, zoom, s)
}

/// A helper function to calculate the menu's overall bounding box for hover activation.
pub fn calculate_actual_menu_rect(node_rect: Rect, zoom: f32, s: &NodeEditorSettings) -> Rect {
    let node_center = node_rect.center();
    let (_, _, outer_half_size) = radial_menu_metrics(node_rect, zoom, s);
    Rect::from_center_size(node_center, Vec2::splat(outer_half_size * 2.0))
}

/// Checks for alignment between two node rectangles and returns the necessary adjustment vector.
/// It also populates a list of lines to be drawn for visual feedback.
pub fn check_and_apply_snap(
    dragged_rect: Rect, // in canvas space
    static_rect: Rect,  // in canvas space
    zoom: f32,
    threshold: f32, // in screen space
    editor_origin: Pos2,
    pan: Vec2,
    snap_lines: &mut Vec<(Pos2, Pos2)>,
) -> Vec2 {
    let dragged_center = dragged_rect.center();
    let static_center = static_rect.center();

    // --- Check Vertical Snapping (centers align vertically) ---
    let diff_x = (static_center.x - dragged_center.x) * zoom;
    if diff_x.abs() < threshold {
        let adjustment_x_canvas = diff_x / zoom;

        // Create snap line in screen space
        let line_x = (dragged_center.x + adjustment_x_canvas) * zoom + editor_origin.x + pan.x;
        let y1 = dragged_rect.top().min(static_rect.top()) * zoom + editor_origin.y + pan.y;
        let y2 = dragged_rect.bottom().max(static_rect.bottom()) * zoom + editor_origin.y + pan.y;
        snap_lines.push((Pos2::new(line_x, y1), Pos2::new(line_x, y2)));

        return Vec2::new(adjustment_x_canvas, 0.0);
    }

    // --- Check Horizontal Snapping (centers align horizontally) ---
    let diff_y = (static_center.y - dragged_center.y) * zoom;
    if diff_y.abs() < threshold {
        let adjustment_y_canvas = diff_y / zoom;

        // Create snap line in screen space
        let line_y = (dragged_center.y + adjustment_y_canvas) * zoom + editor_origin.y + pan.y;
        let x1 = dragged_rect.left().min(static_rect.left()) * zoom + editor_origin.x + pan.x;
        let x2 = dragged_rect.right().max(static_rect.right()) * zoom + editor_origin.x + pan.x;
        snap_lines.push((Pos2::new(x1, line_y), Pos2::new(x2, line_y)));

        return Vec2::new(0.0, adjustment_y_canvas);
    }

    Vec2::ZERO
}

// Helper to check if pointer is inside a convex polygon
pub fn is_inside(poly: &[Pos2], pointer_pos: Pos2) -> bool {
    let mut intersections = 0;
    for i in 0..poly.len() {
        let p1 = poly[i];
        let p2 = poly[(i + 1) % poly.len()];
        if (p1.y > pointer_pos.y) != (p2.y > pointer_pos.y) {
            let x = (p2.x - p1.x) * (pointer_pos.y - p1.y) / (p2.y - p1.y) + p1.x;
            if pointer_pos.x < x {
                intersections += 1;
            }
        }
    }
    intersections % 2 != 0
}

pub fn lines_intersect(p1: Pos2, p2: Pos2, p3: Pos2, p4: Pos2) -> bool {
    let d = (p2.x - p1.x) * (p4.y - p3.y) - (p2.y - p1.y) * (p4.x - p3.x);
    if d == 0.0 {
        return false;
    }
    let t = ((p3.x - p1.x) * (p4.y - p3.y) - (p3.y - p1.y) * (p4.x - p3.x)) / d;
    let u = -((p2.x - p1.x) * (p3.y - p1.y) - (p2.y - p1.y) * (p3.x - p1.x)) / d;

    t >= 0.0 && t <= 1.0 && u >= 0.0 && u <= 1.0
}

pub fn get_bezier_bbox(p0: Pos2, p3: Pos2) -> Rect {
    let p1 = p0 + Vec2::new(50.0, 0.0);
    let p2 = p3 - Vec2::new(50.0, 0.0);
    Rect::from_points(&[p0, p1, p2, p3])
}

pub fn rect_distance_sq(rect_a: Rect, rect_b: Rect) -> f32 {
    let a_min = rect_a.min;
    let a_max = rect_a.max;
    let b_min = rect_b.min;
    let b_max = rect_b.max;

    let dx = (a_min.x - b_max.x).max(0.0) + (b_min.x - a_max.x).max(0.0);
    let dy = (a_min.y - b_max.y).max(0.0) + (b_min.y - a_max.y).max(0.0);

    dx * dx + dy * dy
}

pub fn point_to_segment_distance_sq(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let l2 = ab.length_sq();
    if l2 == 0.0 {
        return ap.length_sq();
    }
    let t = (ap.dot(ab) / l2).clamp(0.0, 1.0);
    let projection = a + t * ab;
    (p - projection).length_sq()
}

pub fn point_to_bezier_distance_sq(point: Pos2, p1: Pos2, p4: Pos2) -> f32 {
    // Horizontal-flow wires: control points bend along X.
    let control_offset = Vec2::new((p4.x - p1.x).abs() * 0.4, 0.0);
    let p2 = p1 + control_offset;
    let p3 = p4 - control_offset;
    let curve = bevy_egui::egui::epaint::CubicBezierShape::from_points_stroke(
        [p1, p2, p3, p4],
        false,
        bevy_egui::egui::Color32::TRANSPARENT,
        bevy_egui::egui::Stroke::NONE,
    );

    let points: Vec<Pos2> = curve.flatten(Some(5.0));
    let mut min_dist_sq = f32::MAX;

    for window in points.windows(2) {
        let dist_sq = point_to_segment_distance_sq(point, window[0], window[1]);
        if dist_sq < min_dist_sq {
            min_dist_sq = dist_sq;
        }
    }
    min_dist_sq
}
