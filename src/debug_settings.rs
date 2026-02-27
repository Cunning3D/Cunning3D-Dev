use crate::settings::{
    SettingMeta, SettingScope, SettingValue, SettingsMerge, SettingsRegistry, SettingsStores,
};
use bevy::prelude::*;

#[derive(Resource, Clone)]
pub struct DebugSettings {
    pub gpu_text_stats: bool,
    pub gpu_text_stats_interval: f32,
    pub gpu_text_stats_verbose: bool,
    pub sdf_rect_stats: bool,
    pub sdf_rect_stats_interval: f32,
    pub sdf_rect_stats_verbose: bool,
    pub sdf_grid_stats: bool,
    pub sdf_grid_stats_interval: f32,
    pub sdf_grid_stats_verbose: bool,
    pub sdf_curve_stats: bool,
    pub sdf_curve_stats_interval: f32,
    pub sdf_curve_stats_verbose: bool,
    pub sdf_dashed_curve_stats: bool,
    pub sdf_dashed_curve_stats_interval: f32,
    pub sdf_dashed_curve_stats_verbose: bool,
    pub sdf_ui_stats: bool,
    pub sdf_ui_stats_interval: f32,

    // Viewport performance logs (Console tab)
    pub viewport_perf_stats: bool,
    pub viewport_perf_stats_interval: f32,
    pub viewport_perf_stutter_ms: f32,
    pub viewport_perf_only_interacting: bool,
    pub viewport_perf_stats_verbose: bool,
}

impl Default for DebugSettings {
    fn default() -> Self {
        Self {
            gpu_text_stats: false,
            gpu_text_stats_interval: 1.0,
            gpu_text_stats_verbose: false,
            sdf_rect_stats: false,
            sdf_rect_stats_interval: 1.0,
            sdf_rect_stats_verbose: false,
            sdf_grid_stats: false,
            sdf_grid_stats_interval: 1.0,
            sdf_grid_stats_verbose: false,
            sdf_curve_stats: false,
            sdf_curve_stats_interval: 1.0,
            sdf_curve_stats_verbose: false,
            sdf_dashed_curve_stats: false,
            sdf_dashed_curve_stats_interval: 1.0,
            sdf_dashed_curve_stats_verbose: false,
            sdf_ui_stats: false,
            sdf_ui_stats_interval: 1.0,
            viewport_perf_stats: false,
            viewport_perf_stats_interval: 1.0,
            viewport_perf_stutter_ms: 33.0,
            viewport_perf_only_interacting: true,
            viewport_perf_stats_verbose: false,
        }
    }
}

pub fn apply_from_settings(reg: &SettingsRegistry, stores: &SettingsStores, s: &mut DebugSettings) {
    let get = |id: &str| {
        reg.get(id).and_then(|m| {
            Some(SettingsMerge::resolve(m, stores.project.get(id), stores.user.get(id)).1)
        })
    };
    if let Some(SettingValue::Bool(v)) = get("debug.render.gpu_text_stats") {
        s.gpu_text_stats = v;
    }
    if let Some(SettingValue::F32(v)) = get("debug.render.gpu_text_stats_interval") {
        s.gpu_text_stats_interval = v;
    }
    if let Some(SettingValue::Bool(v)) = get("debug.render.gpu_text_stats_verbose") {
        s.gpu_text_stats_verbose = v;
    }
    if let Some(SettingValue::Bool(v)) = get("debug.render.sdf_rect_stats") {
        s.sdf_rect_stats = v;
    }
    if let Some(SettingValue::F32(v)) = get("debug.render.sdf_rect_stats_interval") {
        s.sdf_rect_stats_interval = v;
    }
    if let Some(SettingValue::Bool(v)) = get("debug.render.sdf_rect_stats_verbose") {
        s.sdf_rect_stats_verbose = v;
    }
    if let Some(SettingValue::Bool(v)) = get("debug.render.sdf_grid_stats") {
        s.sdf_grid_stats = v;
    }
    if let Some(SettingValue::F32(v)) = get("debug.render.sdf_grid_stats_interval") {
        s.sdf_grid_stats_interval = v;
    }
    if let Some(SettingValue::Bool(v)) = get("debug.render.sdf_grid_stats_verbose") {
        s.sdf_grid_stats_verbose = v;
    }
    if let Some(SettingValue::Bool(v)) = get("debug.render.sdf_curve_stats") {
        s.sdf_curve_stats = v;
    }
    if let Some(SettingValue::F32(v)) = get("debug.render.sdf_curve_stats_interval") {
        s.sdf_curve_stats_interval = v;
    }
    if let Some(SettingValue::Bool(v)) = get("debug.render.sdf_curve_stats_verbose") {
        s.sdf_curve_stats_verbose = v;
    }
    if let Some(SettingValue::Bool(v)) = get("debug.render.sdf_dashed_curve_stats") {
        s.sdf_dashed_curve_stats = v;
    }
    if let Some(SettingValue::F32(v)) = get("debug.render.sdf_dashed_curve_stats_interval") {
        s.sdf_dashed_curve_stats_interval = v;
    }
    if let Some(SettingValue::Bool(v)) = get("debug.render.sdf_dashed_curve_stats_verbose") {
        s.sdf_dashed_curve_stats_verbose = v;
    }
    if let Some(SettingValue::Bool(v)) = get("debug.render.sdf_ui_stats") {
        s.sdf_ui_stats = v;
    }
    if let Some(SettingValue::F32(v)) = get("debug.render.sdf_ui_stats_interval") {
        s.sdf_ui_stats_interval = v;
    }

    if let Some(SettingValue::Bool(v)) = get("debug.viewport.perf_stats") {
        s.viewport_perf_stats = v;
    }
    if let Some(SettingValue::F32(v)) = get("debug.viewport.perf_stats_interval") {
        s.viewport_perf_stats_interval = v;
    }
    if let Some(SettingValue::F32(v)) = get("debug.viewport.perf_stutter_ms") {
        s.viewport_perf_stutter_ms = v;
    }
    if let Some(SettingValue::Bool(v)) = get("debug.viewport.perf_only_interacting") {
        s.viewport_perf_only_interacting = v;
    }
    if let Some(SettingValue::Bool(v)) = get("debug.viewport.perf_stats_verbose") {
        s.viewport_perf_stats_verbose = v;
    }
}

pub fn sync_from_settings_stores(
    reg: Res<SettingsRegistry>,
    stores: Res<SettingsStores>,
    mut s: ResMut<DebugSettings>,
) {
    if !(reg.is_changed() || stores.is_changed() || s.is_added()) {
        return;
    }
    apply_from_settings(&reg, &stores, &mut s);
}

fn register_debug_settings(reg: &mut SettingsRegistry) {
    let d = DebugSettings::default();
    reg.upsert(SettingMeta {
        id: "debug.render.gpu_text_stats".into(),
        path: "General/Debug/Rendering".into(),
        label: "GPU Text Stats".into(),
        help: "Log GPU text batching stats to Console".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(d.gpu_text_stats),
        min: None,
        max: None,
        step: None,
        keywords: vec![
            "gpu".into(),
            "text".into(),
            "stats".into(),
            "drawcall".into(),
        ],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.gpu_text_stats_interval".into(),
        path: "General/Debug/Rendering".into(),
        label: "GPU Text Stats Interval".into(),
        help: "Seconds between log lines".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.gpu_text_stats_interval),
        min: Some(0.05),
        max: Some(10.0),
        step: Some(0.05),
        keywords: vec!["interval".into(), "seconds".into()],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.gpu_text_stats_verbose".into(),
        path: "General/Debug/Rendering".into(),
        label: "GPU Text Stats Verbose".into(),
        help: "Include extra details in log".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(d.gpu_text_stats_verbose),
        min: None,
        max: None,
        step: None,
        keywords: vec!["verbose".into(), "details".into()],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.sdf_rect_stats".into(),
        path: "General/Debug/Rendering".into(),
        label: "SDF Rect Stats".into(),
        help: "Log SDF rect batching stats to Console".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(d.sdf_rect_stats),
        min: None,
        max: None,
        step: None,
        keywords: vec![
            "sdf".into(),
            "rect".into(),
            "stats".into(),
            "drawcall".into(),
        ],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.sdf_rect_stats_interval".into(),
        path: "General/Debug/Rendering".into(),
        label: "SDF Rect Stats Interval".into(),
        help: "Seconds between log lines".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.sdf_rect_stats_interval),
        min: Some(0.05),
        max: Some(10.0),
        step: Some(0.05),
        keywords: vec!["interval".into(), "seconds".into()],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.sdf_rect_stats_verbose".into(),
        path: "General/Debug/Rendering".into(),
        label: "SDF Rect Stats Verbose".into(),
        help: "Include extra details in log".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(d.sdf_rect_stats_verbose),
        min: None,
        max: None,
        step: None,
        keywords: vec!["verbose".into(), "details".into()],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.sdf_grid_stats".into(),
        path: "General/Debug/Rendering".into(),
        label: "SDF Grid Stats".into(),
        help: "Log SDF grid batching stats to Console".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(d.sdf_grid_stats),
        min: None,
        max: None,
        step: None,
        keywords: vec![
            "sdf".into(),
            "grid".into(),
            "stats".into(),
            "drawcall".into(),
        ],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.sdf_grid_stats_interval".into(),
        path: "General/Debug/Rendering".into(),
        label: "SDF Grid Stats Interval".into(),
        help: "Seconds between log lines".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.sdf_grid_stats_interval),
        min: Some(0.05),
        max: Some(10.0),
        step: Some(0.05),
        keywords: vec!["interval".into(), "seconds".into()],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.sdf_grid_stats_verbose".into(),
        path: "General/Debug/Rendering".into(),
        label: "SDF Grid Stats Verbose".into(),
        help: "Include extra details in log".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(d.sdf_grid_stats_verbose),
        min: None,
        max: None,
        step: None,
        keywords: vec!["verbose".into(), "details".into()],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.sdf_curve_stats".into(),
        path: "General/Debug/Rendering".into(),
        label: "SDF Curve Stats".into(),
        help: "Log SDF curve batching stats to Console".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(d.sdf_curve_stats),
        min: None,
        max: None,
        step: None,
        keywords: vec![
            "sdf".into(),
            "curve".into(),
            "stats".into(),
            "drawcall".into(),
        ],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.sdf_curve_stats_interval".into(),
        path: "General/Debug/Rendering".into(),
        label: "SDF Curve Stats Interval".into(),
        help: "Seconds between log lines".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.sdf_curve_stats_interval),
        min: Some(0.05),
        max: Some(10.0),
        step: Some(0.05),
        keywords: vec!["interval".into(), "seconds".into()],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.sdf_curve_stats_verbose".into(),
        path: "General/Debug/Rendering".into(),
        label: "SDF Curve Stats Verbose".into(),
        help: "Include extra details in log".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(d.sdf_curve_stats_verbose),
        min: None,
        max: None,
        step: None,
        keywords: vec!["verbose".into(), "details".into()],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.sdf_dashed_curve_stats".into(),
        path: "General/Debug/Rendering".into(),
        label: "SDF DashedCurve Stats".into(),
        help: "Log SDF dashed curve batching stats to Console".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(d.sdf_dashed_curve_stats),
        min: None,
        max: None,
        step: None,
        keywords: vec![
            "sdf".into(),
            "dashed".into(),
            "curve".into(),
            "stats".into(),
            "drawcall".into(),
        ],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.sdf_dashed_curve_stats_interval".into(),
        path: "General/Debug/Rendering".into(),
        label: "SDF DashedCurve Stats Interval".into(),
        help: "Seconds between log lines".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.sdf_dashed_curve_stats_interval),
        min: Some(0.05),
        max: Some(10.0),
        step: Some(0.05),
        keywords: vec!["interval".into(), "seconds".into()],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.sdf_dashed_curve_stats_verbose".into(),
        path: "General/Debug/Rendering".into(),
        label: "SDF DashedCurve Stats Verbose".into(),
        help: "Include extra details in log".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(d.sdf_dashed_curve_stats_verbose),
        min: None,
        max: None,
        step: None,
        keywords: vec!["verbose".into(), "details".into()],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.sdf_ui_stats".into(),
        path: "General/Debug/Rendering".into(),
        label: "SDF UI Stats".into(),
        help: "Log batched SDF UI primitive stats (Rect/Circle/Ellipse/Curve) to Console".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(d.sdf_ui_stats),
        min: None,
        max: None,
        step: None,
        keywords: vec!["sdf".into(), "ui".into(), "stats".into(), "drawcall".into()],
    });
    reg.upsert(SettingMeta {
        id: "debug.render.sdf_ui_stats_interval".into(),
        path: "General/Debug/Rendering".into(),
        label: "SDF UI Stats Interval".into(),
        help: "Seconds between log lines".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.sdf_ui_stats_interval),
        min: Some(0.05),
        max: Some(10.0),
        step: Some(0.05),
        keywords: vec!["interval".into(), "seconds".into()],
    });

    reg.upsert(SettingMeta {
        id: "debug.viewport.perf_stats".into(),
        path: "General/Debug/Viewport".into(),
        label: "Viewport Perf Stats".into(),
        help: "Log viewport interaction frame breakdown (UI/scene/compute/overlays) to Console tab".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(d.viewport_perf_stats),
        min: None,
        max: None,
        step: None,
        keywords: vec![
            "viewport".into(),
            "perf".into(),
            "stats".into(),
            "frame".into(),
            "stutter".into(),
        ],
    });
    reg.upsert(SettingMeta {
        id: "debug.viewport.perf_stats_interval".into(),
        path: "General/Debug/Viewport".into(),
        label: "Viewport Perf Interval".into(),
        help: "Seconds between summary log lines (during interaction).".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.viewport_perf_stats_interval),
        min: Some(0.1),
        max: Some(10.0),
        step: Some(0.1),
        keywords: vec!["interval".into(), "seconds".into()],
    });
    reg.upsert(SettingMeta {
        id: "debug.viewport.perf_stutter_ms".into(),
        path: "General/Debug/Viewport".into(),
        label: "Viewport Stutter ms".into(),
        help: "Log an immediate VP_STUTTER line when frame delta >= this threshold.".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.viewport_perf_stutter_ms),
        min: Some(5.0),
        max: Some(500.0),
        step: Some(1.0),
        keywords: vec!["stutter".into(), "threshold".into(), "ms".into()],
    });
    reg.upsert(SettingMeta {
        id: "debug.viewport.perf_only_interacting".into(),
        path: "General/Debug/Viewport".into(),
        label: "Perf Only Interacting".into(),
        help: "Only log/accumulate while the viewport is navigating (orbit/pan/zoom/gizmo).".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(d.viewport_perf_only_interacting),
        min: None,
        max: None,
        step: None,
        keywords: vec!["interaction".into(), "viewport".into()],
    });
    reg.upsert(SettingMeta {
        id: "debug.viewport.perf_stats_verbose".into(),
        path: "General/Debug/Viewport".into(),
        label: "Viewport Perf Verbose".into(),
        help: "Log one line per measured section (avg/max) on each summary flush.".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(d.viewport_perf_stats_verbose),
        min: None,
        max: None,
        step: None,
        keywords: vec!["verbose".into(), "details".into()],
    });
}

crate::register_settings_provider!("debug", register_debug_settings);
