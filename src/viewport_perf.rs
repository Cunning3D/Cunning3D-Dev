use bevy::prelude::*;
use std::time::{Duration, Instant};

use crate::{console::ConsoleLog, debug_settings::DebugSettings, input::NavigationInput};

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportPerfSection {
    EditorUi = 0,
    FloatingUi = 1,
    CameraSync = 2,
    InputMapping = 3,
    CameraControl = 4,
    ComputeDispatch = 5,
    ComputeReceive = 6,
    SceneUpdate = 7,
    Gizmos = 8,
    OverlayUvGrid = 9,
    OverlayGridLabels = 10,
    OverlayTemplateWireframes = 11,
    OverlayHighlights = 12,
    OverlayNumbers = 13,
}

pub const SECTION_COUNT: usize = 14;

pub const ALL_SECTIONS: [ViewportPerfSection; SECTION_COUNT] = [
    ViewportPerfSection::EditorUi,
    ViewportPerfSection::FloatingUi,
    ViewportPerfSection::CameraSync,
    ViewportPerfSection::InputMapping,
    ViewportPerfSection::CameraControl,
    ViewportPerfSection::ComputeDispatch,
    ViewportPerfSection::ComputeReceive,
    ViewportPerfSection::SceneUpdate,
    ViewportPerfSection::Gizmos,
    ViewportPerfSection::OverlayUvGrid,
    ViewportPerfSection::OverlayGridLabels,
    ViewportPerfSection::OverlayTemplateWireframes,
    ViewportPerfSection::OverlayHighlights,
    ViewportPerfSection::OverlayNumbers,
];

impl ViewportPerfSection {
    pub const fn label(self) -> &'static str {
        match self {
            ViewportPerfSection::EditorUi => "editor_ui",
            ViewportPerfSection::FloatingUi => "floating_ui",
            ViewportPerfSection::CameraSync => "camera_sync",
            ViewportPerfSection::InputMapping => "input",
            ViewportPerfSection::CameraControl => "camera",
            ViewportPerfSection::ComputeDispatch => "cook_dispatch",
            ViewportPerfSection::ComputeReceive => "cook_receive",
            ViewportPerfSection::SceneUpdate => "scene_update",
            ViewportPerfSection::Gizmos => "gizmos",
            ViewportPerfSection::OverlayUvGrid => "overlay_uv_grid",
            ViewportPerfSection::OverlayGridLabels => "overlay_grid_labels",
            ViewportPerfSection::OverlayTemplateWireframes => "overlay_template_wire",
            ViewportPerfSection::OverlayHighlights => "overlay_highlights",
            ViewportPerfSection::OverlayNumbers => "overlay_numbers",
        }
    }

    pub const fn is_overlay(self) -> bool {
        matches!(
            self,
            ViewportPerfSection::OverlayUvGrid
                | ViewportPerfSection::OverlayGridLabels
                | ViewportPerfSection::OverlayTemplateWireframes
                | ViewportPerfSection::OverlayHighlights
                | ViewportPerfSection::OverlayNumbers
        )
    }
}

#[derive(Resource)]
pub struct ViewportPerfTrace {
    pub enabled: bool,
    cur_ms: [f32; SECTION_COUNT],
    cur_dt_ms: f32,

    frames: u32,
    dt_sum_ms: f64,
    dt_max_ms: f32,
    stutter_frames: u32,

    sum_ms: [f64; SECTION_COUNT],
    max_ms: [f32; SECTION_COUNT],

    last_log_time_s: f64,
    last_interacting: bool,
}

impl Default for ViewportPerfTrace {
    fn default() -> Self {
        Self {
            enabled: false,
            cur_ms: [0.0; SECTION_COUNT],
            cur_dt_ms: 0.0,
            frames: 0,
            dt_sum_ms: 0.0,
            dt_max_ms: 0.0,
            stutter_frames: 0,
            sum_ms: [0.0; SECTION_COUNT],
            max_ms: [0.0; SECTION_COUNT],
            last_log_time_s: 0.0,
            last_interacting: false,
        }
    }
}

impl ViewportPerfTrace {
    pub fn begin_frame(&mut self, dt_ms: f32, enabled: bool) {
        self.enabled = enabled;
        self.cur_dt_ms = dt_ms.max(0.0);
        self.cur_ms = [0.0; SECTION_COUNT];
    }

    #[inline]
    pub fn record(&mut self, section: ViewportPerfSection, d: Duration) {
        if !self.enabled {
            return;
        }
        let ms = d.as_secs_f64() * 1000.0;
        self.cur_ms[section as usize] += ms as f32;
    }

    #[inline]
    pub fn current_ms(&self, section: ViewportPerfSection) -> f32 {
        self.cur_ms[section as usize]
    }

    pub fn current_overlay_ms(&self) -> f32 {
        let mut sum = 0.0;
        for sec in ALL_SECTIONS {
            if sec.is_overlay() {
                sum += self.cur_ms[sec as usize];
            }
        }
        sum
    }

    fn accumulate_current_frame(&mut self, stutter_ms: f32) {
        self.frames = self.frames.saturating_add(1);
        self.dt_sum_ms += self.cur_dt_ms as f64;
        self.dt_max_ms = self.dt_max_ms.max(self.cur_dt_ms);
        if self.cur_dt_ms >= stutter_ms {
            self.stutter_frames = self.stutter_frames.saturating_add(1);
        }

        for i in 0..SECTION_COUNT {
            let v = self.cur_ms[i] as f64;
            self.sum_ms[i] += v;
            self.max_ms[i] = self.max_ms[i].max(v as f32);
        }
    }

    fn reset_accumulation(&mut self) {
        self.frames = 0;
        self.dt_sum_ms = 0.0;
        self.dt_max_ms = 0.0;
        self.stutter_frames = 0;
        self.sum_ms = [0.0; SECTION_COUNT];
        self.max_ms = [0.0; SECTION_COUNT];
    }
}

pub struct PerfScope<'a> {
    perf: &'a mut ViewportPerfTrace,
    section: ViewportPerfSection,
    start: Instant,
}

impl<'a> PerfScope<'a> {
    #[inline]
    pub fn new(perf: &'a mut ViewportPerfTrace, section: ViewportPerfSection) -> Self {
        Self {
            perf,
            section,
            start: Instant::now(),
        }
    }
}

impl Drop for PerfScope<'_> {
    fn drop(&mut self) {
        self.perf.record(self.section, self.start.elapsed());
    }
}

pub struct WorldPerfScope {
    world: *mut World,
    section: ViewportPerfSection,
    start: Instant,
}

impl WorldPerfScope {
    #[inline]
    pub fn new(world: &mut World, section: ViewportPerfSection) -> Self {
        Self {
            world,
            section,
            start: Instant::now(),
        }
    }
}

impl Drop for WorldPerfScope {
    fn drop(&mut self) {
        // SAFETY: This scope is intended to be created as the first local in a `&mut World` system
        // function. It drops last, after all other borrows are released.
        unsafe {
            let world = &mut *self.world;
            if let Some(mut perf) = world.get_resource_mut::<ViewportPerfTrace>() {
                perf.record(self.section, self.start.elapsed());
            }
        }
    }
}

#[inline]
pub fn world_scope(world: &mut World, section: ViewportPerfSection) -> WorldPerfScope {
    WorldPerfScope::new(world, section)
}

pub fn begin_frame_system(
    time: Res<Time>,
    s: Res<DebugSettings>,
    mut perf: ResMut<ViewportPerfTrace>,
) {
    // Keep this system extremely cheap; it runs every frame.
    let enabled = s.viewport_perf_stats;
    let dt_ms = (time.delta_secs_f64() * 1000.0) as f32;
    perf.begin_frame(dt_ms, enabled);
}

fn viewport_is_interacting(
    nav_input: &NavigationInput,
    viewport_interaction: &crate::ViewportInteractionState,
    viewport_layout: &crate::tabs_system::viewport_3d::ViewportLayout,
) -> bool {
    let viewport_active = viewport_layout.logical_rect.is_some();
    if !viewport_active {
        return false;
    }
    nav_input.active
        || nav_input.zoom_delta != 0.0
        || nav_input.orbit_delta.length_squared() != 0.0
        || nav_input.pan_delta.length_squared() != 0.0
        || nav_input.fly_vector.length_squared() != 0.0
        || viewport_interaction.is_gizmo_dragging
        || viewport_interaction.is_right_button_dragged
        || viewport_interaction.is_middle_button_dragged
        || viewport_interaction.is_alt_left_button_dragged
}

pub fn end_frame_log_system(
    time: Res<Time>,
    s: Res<DebugSettings>,
    nav_input: Res<NavigationInput>,
    viewport_interaction: Res<crate::ViewportInteractionState>,
    viewport_layout: Res<crate::tabs_system::viewport_3d::ViewportLayout>,
    console_log: Res<ConsoleLog>,
    mut perf: ResMut<ViewportPerfTrace>,
) {
    if !perf.enabled {
        perf.last_interacting = false;
        perf.reset_accumulation();
        return;
    }

    let now_s = time.elapsed_secs_f64();
    let interval_s = s.viewport_perf_stats_interval.clamp(0.1, 10.0) as f64;
    let stutter_ms = s.viewport_perf_stutter_ms.clamp(5.0, 500.0);

    let interacting = viewport_is_interacting(&nav_input, &viewport_interaction, &viewport_layout);
    let should_accumulate = if s.viewport_perf_only_interacting {
        interacting
    } else {
        true
    };

    // Accumulate only while "active" to avoid noisy idle samples.
    if should_accumulate {
        perf.accumulate_current_frame(stutter_ms);

        if perf.cur_dt_ms >= stutter_ms {
            // On stutter frames, log the current breakdown immediately.
            let mut rows: Vec<(ViewportPerfSection, f32)> =
                ALL_SECTIONS.iter().map(|&sec| (sec, perf.current_ms(sec))).collect();
            rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let mut msg = format!("VP_STUTTER dt={:.1}ms", perf.cur_dt_ms);
            for (sec, ms) in rows.into_iter().take(5) {
                if ms <= 0.01 {
                    continue;
                }
                msg.push_str(&format!(" {}={:.2}", sec.label(), ms));
            }
            console_log.debug(msg);
        }
    }

    // Flush summary when interaction ends, or on interval.
    let interaction_ended = perf.last_interacting && !should_accumulate;
    let interval_elapsed = (now_s - perf.last_log_time_s) >= interval_s;

    if (interaction_ended || interval_elapsed) && perf.frames > 0 {
        let frames = perf.frames.max(1) as f64;
        let dt_avg = perf.dt_sum_ms / frames;

        // Compute overlay sum stats.
        let (mut overlay_sum, mut overlay_max) = (0.0f64, 0.0f32);
        for sec in ALL_SECTIONS {
            if !sec.is_overlay() {
                continue;
            }
            overlay_sum += perf.sum_ms[sec as usize];
            overlay_max = overlay_max.max(perf.max_ms[sec as usize]);
        }
        let overlay_avg = overlay_sum / frames;

        if s.viewport_perf_stats_verbose {
            let mut msg = format!(
                "VP_PERF frames={} stutter={} dt_avg={:.2} dt_max={:.2} (ms)",
                perf.frames, perf.stutter_frames, dt_avg, perf.dt_max_ms
            );
            console_log.debug(msg);
            for sec in ALL_SECTIONS {
                let avg = perf.sum_ms[sec as usize] / frames;
                let max = perf.max_ms[sec as usize];
                if avg <= 0.01 && max <= 0.01 {
                    continue;
                }
                console_log.debug(format!(
                    "VP_PERF  {:<22} avg={:.2} max={:.2}",
                    sec.label(),
                    avg,
                    max
                ));
            }
        } else {
            // One-line summary with key buckets.
            let ui_avg = perf.sum_ms[ViewportPerfSection::EditorUi as usize] / frames;
            let ui_max = perf.max_ms[ViewportPerfSection::EditorUi as usize];
            let scene_avg = perf.sum_ms[ViewportPerfSection::SceneUpdate as usize] / frames;
            let scene_max = perf.max_ms[ViewportPerfSection::SceneUpdate as usize];
            let recv_avg = perf.sum_ms[ViewportPerfSection::ComputeReceive as usize] / frames;
            let recv_max = perf.max_ms[ViewportPerfSection::ComputeReceive as usize];
            let disp_avg = perf.sum_ms[ViewportPerfSection::ComputeDispatch as usize] / frames;
            let disp_max = perf.max_ms[ViewportPerfSection::ComputeDispatch as usize];
            let msg = format!(
                "VP_PERF frames={} stutter={} dt_avg={:.2} dt_max={:.2} ui_avg={:.2} ui_max={:.2} scene_avg={:.2} scene_max={:.2} cook_recv_avg={:.2} cook_recv_max={:.2} cook_disp_avg={:.2} cook_disp_max={:.2} overlay_avg={:.2} overlay_max={:.2}",
                perf.frames,
                perf.stutter_frames,
                dt_avg,
                perf.dt_max_ms,
                ui_avg,
                ui_max,
                scene_avg,
                scene_max,
                recv_avg,
                recv_max,
                disp_avg,
                disp_max,
                overlay_avg,
                overlay_max,
            );
            console_log.debug(msg);
        }

        perf.reset_accumulation();
        perf.last_log_time_s = now_s;
    }

    perf.last_interacting = should_accumulate;
}
