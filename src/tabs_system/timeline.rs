use crate::invalidator::{RepaintCause, UiInvalidator};
use crate::tabs_system::rect_hash::mix_rect;
use crate::ui::TimelineState;
use bevy::prelude::{Res, ResMut, Time};
use bevy::time::Real;
use bevy::window::RequestRedraw;
use bevy_egui::egui::{
    self, Align, Align2, Color32, FontId, Layout, Pos2, Sense, Stroke, Ui, Vec2,
};
use std::time::Duration;

/// Constants for visual styling
const TIMELINE_HEIGHT: f32 = 30.0;
const TICK_HEIGHT_MAJOR: f32 = 12.0;
const TICK_HEIGHT_MINOR: f32 = 6.0;
const PLAYHEAD_WIDTH: f32 = 2.0;

pub fn show_timeline_panel(ui: &mut Ui, state: &mut TimelineState) {
    // Kept for compatibility; timeline must be driven via UiInvalidator in Reactive mode.
    // Intentionally no-op without invalidator.
    let _ = (ui, state);
}

pub fn show_timeline_panel_with_invalidator(
    ui: &mut Ui,
    state: &mut TimelineState,
    inv: &mut UiInvalidator,
) {
    ui.vertical(|ui| {
        // --- Top Row: Controls + Main Timeline ---
        ui.horizontal(|ui| {
            // 1. Playback Controls (Fixed Width Group)
            ui.group(|ui| {
                ui.style_mut().spacing.item_spacing = Vec2::new(2.0, 0.0);

                let btn_size = Vec2::splat(20.0);

                if ui.add(egui::Button::new("⏮").min_size(btn_size)).clicked() {
                    state.current_frame = state.start_frame;
                    state.play_started_at = None;
                }
                if ui.add(egui::Button::new("◀").min_size(btn_size)).clicked() {
                    state.current_frame = (state.current_frame - 1.0).max(state.start_frame);
                    state.play_started_at = None;
                }

                let play_icon = if state.is_playing { "⏸" } else { "▶" };
                if ui
                    .add(egui::Button::new(play_icon).min_size(btn_size))
                    .clicked()
                {
                    let was = state.is_playing;
                    state.is_playing = !state.is_playing;
                    state.play_started_at = None;
                    if !was && state.is_playing {
                        inv.request_repaint_after_tagged(
                            "timeline/play",
                            Duration::ZERO,
                            RepaintCause::Animation,
                        );
                    }
                }

                if ui.add(egui::Button::new("▶").min_size(btn_size)).clicked() {
                    state.current_frame = (state.current_frame + 1.0).min(state.end_frame);
                    state.play_started_at = None;
                }
                if ui.add(egui::Button::new("⏭").min_size(btn_size)).clicked() {
                    state.current_frame = state.end_frame;
                    state.play_started_at = None;
                }
            });

            // 2. Current Frame Input (Fixed Width)
            ui.add(
                egui::DragValue::new(&mut state.current_frame)
                    .speed(0.1)
                    .clamp_range(state.start_frame..=state.end_frame)
                    .suffix(""),
            );

            // 3. The Timeline Track (Fills remaining width)
            draw_timeline_track(ui, state);
        });

        // --- Bottom Row: Range Settings (Miniature) ---
        ui.horizontal(|ui| {
            ui.style_mut().spacing.item_spacing = Vec2::new(10.0, 0.0);

            // Start/End Range Inputs
            ui.label("Start:");
            ui.add(egui::DragValue::new(&mut state.start_frame).speed(1.0));

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::DragValue::new(&mut state.end_frame).speed(1.0));
                ui.label("End:");

                ui.separator();
                ui.label(format!("FPS: {:.0}", state.fps));
            });
        });
    });
}

pub fn timeline_playback_system(
    time: Res<Time<Real>>,
    mut state: ResMut<TimelineState>,
    mut inv: ResMut<UiInvalidator>,
    mut redraw_writer: bevy::ecs::prelude::MessageWriter<RequestRedraw>,
) {
    if !state.is_playing {
        state.play_started_at = None;
        return;
    }
    let fps = state.fps.max(1.0);
    let now = time.elapsed_secs_f64();
    let start = match state.play_started_at {
        Some(t) => t,
        None => {
            state.play_started_frame = state.current_frame;
            state.play_started_at = Some(now);
            now
        }
    };
    let t = (now - start).clamp(0.0, 10.0) as f32;
    state.current_frame = (state.play_started_frame + t * fps).min(state.end_frame);
    if state.current_frame >= state.end_frame - f32::EPSILON {
        state.current_frame = state.end_frame;
        state.is_playing = false;
        state.play_started_at = None;
        return;
    }
    // 触发 bevy_ui 刷新 (Reactive 模式)
    redraw_writer.write(RequestRedraw);
    // 保留 egui 刷新 (兼容)
    inv.request_repaint_after_tagged(
        "timeline/play",
        Duration::from_secs_f32(1.0 / 60.0),
        RepaintCause::Animation,
    );
}

fn draw_timeline_track(ui: &mut Ui, state: &mut TimelineState) {
    let available_width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(available_width, TIMELINE_HEIGHT),
        Sense::click_and_drag(),
    );

    // --- Interaction ---
    if response.dragged() || response.clicked() {
        if let Some(pointer_pos) = response.interact_pointer_pos() {
            let relative_x = pointer_pos.x - rect.min.x;
            let t = (relative_x / rect.width()).clamp(0.0, 1.0);
            let frame_range = state.end_frame - state.start_frame;
            state.current_frame = state.start_frame + frame_range * t;
            state.current_frame = state.current_frame.round(); // Snap to integer frames for now
            state.play_started_at = None;
        }
    }

    // --- Painting ---
    let start_i = state.start_frame as i32;
    let end_i = state.end_frame as i32;
    let total = end_i - start_i;
    if total <= 0 {
        return;
    }
    let key = mix_rect(
        (state.start_frame.to_bits() as u64) ^ (state.end_frame.to_bits() as u64).rotate_left(17),
        rect,
    );
    ui.allocate_ui_at_rect(rect, |ui| {
        ui.push_id(("timeline_track_bg", key), |ui| {
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, Color32::from_gray(30));
            painter.rect_stroke(
                rect,
                0.0,
                Stroke::new(1.0, Color32::from_gray(60)),
                egui::StrokeKind::Inside,
            );
            let wpf = rect.width() / total as f32;
            let step = if wpf > 20.0 {
                1
            } else if wpf > 5.0 {
                5
            } else if wpf > 2.0 {
                10
            } else {
                24
            };
            for f in start_i..=end_i {
                let x = rect.min.x + (f - start_i) as f32 * wpf;
                if x > rect.max.x {
                    break;
                }
                if f % step == 0 {
                    painter.line_segment(
                        [
                            Pos2::new(x, rect.min.y),
                            Pos2::new(x, rect.min.y + TICK_HEIGHT_MAJOR),
                        ],
                        Stroke::new(1.0, Color32::from_gray(120)),
                    );
                    if f % (step * 2) == 0 || wpf > 10.0 {
                        let pos = Pos2::new(x + 2.0, rect.min.y + 2.0);
                        let font_px = 10.0;
                        let color = Color32::from_gray(180);
                        painter.text(
                            pos,
                            Align2::LEFT_TOP,
                            f.to_string(),
                            FontId::proportional(font_px),
                            color,
                        );
                    }
                } else if wpf > 3.0 {
                    painter.line_segment(
                        [
                            Pos2::new(x, rect.min.y),
                            Pos2::new(x, rect.min.y + TICK_HEIGHT_MINOR),
                        ],
                        Stroke::new(1.0, Color32::from_gray(70)),
                    );
                }
            }
        });
    });
    let denom = (state.end_frame - state.start_frame).max(1.0);
    let t = ((state.current_frame - state.start_frame) / denom).clamp(0.0, 1.0);
    let x = rect.min.x + t * rect.width();
    ui.painter().line_segment(
        [Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)],
        Stroke::new(PLAYHEAD_WIDTH, Color32::from_rgb(255, 165, 0)),
    );
}
