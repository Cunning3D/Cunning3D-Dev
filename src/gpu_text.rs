use bevy::prelude::*;
use bevy_egui::egui::{self};
use bevy_egui::EguiContexts;
use egui_wgpu::sdf::{create_gpu_text_callback, GpuTextUniform};

use crate::{console, debug_settings};

pub fn paint(p: &egui::Painter, uniform: GpuTextUniform, frame_id: u64) {
    p.add(create_gpu_text_callback(p.clip_rect(), uniform, frame_id));
}

pub(crate) fn install_gpu_text_renderer_system(mut q: Query<&mut bevy_egui::EguiContext>) {
    // Ensure symbol/Unicode coverage for icon glyphs (e.g. MV-style symbols) in GPU text.
    // This is applied before UI draws, so the first FontSystem creation picks it up.
    let _ = q;
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let mut t = egui_wgpu::sdf::gpu_text_tuning_get();
        t.load_system_fonts = true;
        egui_wgpu::sdf::gpu_text_tuning_set(t);
    });
}

pub(crate) fn debug_log_gpu_text_stats(
    time: Res<Time>,
    s: Res<debug_settings::DebugSettings>,
    _console_log: Res<console::ConsoleLog>,
    mut egui_contexts: EguiContexts,
    mut last_gpu: Local<f64>,
    mut last_gpu_frame: Local<u64>,
    mut last_rect: Local<f64>,
    mut last_rect_frame: Local<u64>,
    mut last_grid: Local<f64>,
    mut last_grid_frame: Local<u64>,
    mut last_curve: Local<f64>,
    mut last_curve_frame: Local<u64>,
    mut last_dashed: Local<f64>,
    mut last_dashed_frame: Local<u64>,
    mut last_ui: Local<f64>,
    mut last_ui_frame: Local<u64>,
) {
    if !(s.gpu_text_stats
        || s.sdf_rect_stats
        || s.sdf_grid_stats
        || s.sdf_curve_stats
        || s.sdf_dashed_curve_stats
        || s.sdf_ui_stats)
    {
        return;
    }
    let now = time.elapsed_secs_f64();
    if s.gpu_text_stats {
        let interval = s.gpu_text_stats_interval.clamp(0.05, 10.0) as f64;
        if now - *last_gpu >= interval {
            *last_gpu = now;
            egui_wgpu::sdf::gpu_text_set_verbose_details_enabled(s.gpu_text_stats_verbose);
            let st = egui_wgpu::sdf::gpu_text_last_stats();
            if st.frame_id != 0 && st.frame_id != *last_gpu_frame {
                *last_gpu_frame = st.frame_id;
                if s.gpu_text_stats_verbose {
                    debug!(
                        "GPU_TEXT frame={} texts={} clip_regions={} drawcalls={} verts={}",
                        st.frame_id, st.texts, st.clip_regions, st.drawcalls, st.verts
                    );
                    let causes = egui_contexts.ctx_mut().repaint_causes();
                    if !causes.is_empty() {
                        let mut msg = String::from("EGUI repaint causes: ");
                        for (i, c) in causes.iter().take(6).enumerate() {
                            if i != 0 {
                                msg.push_str(", ");
                            }
                            msg.push_str(&c.to_string());
                        }
                        debug!("{msg}");
                    }
                    let (fid, batches) = egui_wgpu::sdf::gpu_text_last_batch_details();
                    if fid == st.frame_id && !batches.is_empty() {
                        for (i, b) in batches.iter().enumerate() {
                            let [x, y, w, h] = b.scissor;
                            debug!("GPU_TEXT  batch#{i} scissor=({x},{y},{w},{h}) callbacks={} glyphs={} verts={}", b.callbacks, b.glyphs, b.verts);
                        }
                    }
                } else {
                    debug!(
                        "GPU_TEXT texts={} drawcalls={} verts={}",
                        st.texts, st.drawcalls, st.verts
                    );
                }
            }
        }
    }
    if s.sdf_rect_stats {
        let interval = s.sdf_rect_stats_interval.clamp(0.05, 10.0) as f64;
        if now - *last_rect >= interval {
            *last_rect = now;
            egui_wgpu::sdf::sdf_rect_set_verbose_details_enabled(s.sdf_rect_stats_verbose);
            let st = egui_wgpu::sdf::sdf_rect_last_stats();
            if st.frame_id != 0 && st.frame_id != *last_rect_frame {
                *last_rect_frame = st.frame_id;
                if s.sdf_rect_stats_verbose {
                    debug!(
                        "SDF_RECT frame={} instances={} clip_regions={} drawcalls={}",
                        st.frame_id, st.instances, st.clip_regions, st.drawcalls
                    );
                    let (fid, batches) = egui_wgpu::sdf::sdf_rect_last_batch_details();
                    if fid == st.frame_id && !batches.is_empty() {
                        for (i, b) in batches.iter().enumerate() {
                            let [x, y, w, h] = b.scissor;
                            debug!(
                                "SDF_RECT  batch#{i} scissor=({x},{y},{w},{h}) instances={}",
                                b.instances
                            );
                        }
                    }
                } else {
                    debug!(
                        "SDF_RECT instances={} drawcalls={}",
                        st.instances, st.drawcalls
                    );
                }
            }
        }
    }

    if s.sdf_grid_stats {
        let interval = s.sdf_grid_stats_interval.clamp(0.05, 10.0) as f64;
        if now - *last_grid >= interval {
            *last_grid = now;
            egui_wgpu::sdf::sdf_grid_set_verbose_details_enabled(s.sdf_grid_stats_verbose);
            let st = egui_wgpu::sdf::sdf_grid_last_stats();
            if st.frame_id != 0 && st.frame_id != *last_grid_frame {
                *last_grid_frame = st.frame_id;
                if s.sdf_grid_stats_verbose {
                    debug!(
                        "SDF_GRID frame={} instances={} clip_regions={} drawcalls={}",
                        st.frame_id, st.instances, st.clip_regions, st.drawcalls
                    );
                    let (fid, batches) = egui_wgpu::sdf::sdf_grid_last_batch_details();
                    if fid == st.frame_id && !batches.is_empty() {
                        for (i, b) in batches.iter().enumerate() {
                            let [x, y, w, h] = b.scissor;
                            debug!(
                                "SDF_GRID  batch#{i} scissor=({x},{y},{w},{h}) instances={}",
                                b.instances
                            );
                        }
                    }
                } else {
                    debug!(
                        "SDF_GRID instances={} drawcalls={}",
                        st.instances, st.drawcalls
                    );
                }
            }
        }
    }

    if s.sdf_curve_stats {
        let interval = s.sdf_curve_stats_interval.clamp(0.05, 10.0) as f64;
        if now - *last_curve >= interval {
            *last_curve = now;
            egui_wgpu::sdf::sdf_curve_set_verbose_details_enabled(s.sdf_curve_stats_verbose);
            let st = egui_wgpu::sdf::sdf_curve_last_stats();
            if st.frame_id != 0 && st.frame_id != *last_curve_frame {
                *last_curve_frame = st.frame_id;
                if s.sdf_curve_stats_verbose {
                    debug!(
                        "SDF_CURVE frame={} instances={} clip_regions={} drawcalls={}",
                        st.frame_id, st.instances, st.clip_regions, st.drawcalls
                    );
                    let (fid, batches) = egui_wgpu::sdf::sdf_curve_last_batch_details();
                    if fid == st.frame_id && !batches.is_empty() {
                        for (i, b) in batches.iter().enumerate() {
                            let [x, y, w, h] = b.scissor;
                            debug!(
                                "SDF_CURVE  batch#{i} scissor=({x},{y},{w},{h}) instances={}",
                                b.instances
                            );
                        }
                    }
                } else {
                    debug!(
                        "SDF_CURVE instances={} drawcalls={}",
                        st.instances, st.drawcalls
                    );
                }
            }
        }
    }

    if s.sdf_dashed_curve_stats {
        let interval = s.sdf_dashed_curve_stats_interval.clamp(0.05, 10.0) as f64;
        if now - *last_dashed >= interval {
            *last_dashed = now;
            egui_wgpu::sdf::sdf_dashed_curve_set_verbose_details_enabled(
                s.sdf_dashed_curve_stats_verbose,
            );
            let st = egui_wgpu::sdf::sdf_dashed_curve_last_stats();
            if st.frame_id != 0 && st.frame_id != *last_dashed_frame {
                *last_dashed_frame = st.frame_id;
                if s.sdf_dashed_curve_stats_verbose {
                    debug!(
                        "SDF_DASHED frame={} instances={} clip_regions={} drawcalls={}",
                        st.frame_id, st.instances, st.clip_regions, st.drawcalls
                    );
                    let (fid, batches) = egui_wgpu::sdf::sdf_dashed_curve_last_batch_details();
                    if fid == st.frame_id && !batches.is_empty() {
                        for (i, b) in batches.iter().enumerate() {
                            let [x, y, w, h] = b.scissor;
                            debug!(
                                "SDF_DASHED  batch#{i} scissor=({x},{y},{w},{h}) instances={}",
                                b.instances
                            );
                        }
                    }
                } else {
                    debug!(
                        "SDF_DASHED instances={} drawcalls={}",
                        st.instances, st.drawcalls
                    );
                }
            }
        }
    }
    if s.sdf_ui_stats {
        let interval = s.sdf_ui_stats_interval.clamp(0.05, 10.0) as f64;
        if now - *last_ui >= interval {
            *last_ui = now;
            let st = egui_wgpu::sdf::sdf_ui_last_stats();
            if st.frame_id != 0 && st.frame_id != *last_ui_frame {
                *last_ui_frame = st.frame_id;
                debug!("SDF_UI frame={} rects={} circles={} ellipses={} curves={} clip_runs={} drawcalls={}", st.frame_id, st.rects, st.circles, st.ellipses, st.curves, st.clip_runs, st.drawcalls);
            }
        }
    }
}
