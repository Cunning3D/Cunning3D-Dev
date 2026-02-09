use crate::tabs_system::node_editor::mathematic::lines_intersect;
use bevy_egui::egui::Vec2;
use bevy_egui::egui::{Color32, Pos2, Stroke};

pub fn detect_shake(history: &[(f64, Pos2)]) -> bool {
    const SHAKE_WINDOW: f64 = 0.15;
    const MIN_CHANGES: usize = 3;
    const MIN_DIST_SQ: f32 = 5.0 * 5.0;

    if history.len() < MIN_CHANGES + 1 {
        return false;
    }

    let now = history.last().unwrap().0;
    let recent_history: Vec<_> = history
        .iter()
        .filter(|(t, _)| now - *t <= SHAKE_WINDOW)
        .collect();

    if recent_history.len() < MIN_CHANGES + 1 {
        return false;
    }

    let mut direction_changes = 0;
    let mut last_dx: f32 = 0.0;

    for points in recent_history.windows(2) {
        let p1 = points[0].1;
        let p2 = points[1].1;
        let dist_sq = p1.distance_sq(p2);

        if dist_sq < MIN_DIST_SQ {
            continue;
        }

        let dx = p2.x - p1.x;
        if last_dx.signum() != dx.signum() && dx.abs() > 1.0 {
            direction_changes += 1;
        }
        last_dx = dx;
    }

    direction_changes >= MIN_CHANGES
}

pub fn does_path_intersect_bezier(path: &[Pos2], p1: Pos2, p4: Pos2) -> bool {
    if path.len() < 2 {
        return false;
    }
    let control_offset = bevy_egui::egui::Vec2::new(0.0, (p4.y - p1.y).abs() * 0.4);
    let p2 = p1 + control_offset;
    let p3 = p4 - control_offset;
    let curve = bevy_egui::egui::epaint::CubicBezierShape::from_points_stroke(
        [p1, p2, p3, p4],
        false,
        Color32::TRANSPARENT,
        Stroke::NONE,
    );

    let bezier_segments: Vec<Pos2> = curve.flatten(Some(5.0));

    for path_segment in path.windows(2) {
        for bezier_segment in bezier_segments.windows(2) {
            if lines_intersect(
                path_segment[0],
                path_segment[1],
                bezier_segment[0],
                bezier_segment[1],
            ) {
                return true;
            }
        }
    }

    false
}
