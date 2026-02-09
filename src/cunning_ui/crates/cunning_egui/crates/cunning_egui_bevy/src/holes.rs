use crate::{EguiOcclusionRects, EguiSettings, egui};
use bevy::prelude::*;
use bevy_camera::{Camera, RenderTarget};
use bevy_window::{PrimaryWindow, Window, WindowRef};
use bevy_ui::{
    ComputedNode, ComputedUiTargetCamera, UiGlobalTransform, UiSystems,
};
use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

/// Marks a UI node as an area that should be punched out from egui (render + pointer input).
#[derive(Component, Clone, Copy, Default, Reflect)]
#[reflect(Component, Default)]
pub struct EguiHole;

/// Marks a UI node as an area that should NOT be punched out from egui.
#[derive(Component, Clone, Copy, Default, Reflect)]
#[reflect(Component, Default)]
pub struct EguiHoleDisable;

/// Controls how egui holes are generated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum EguiHolesMode {
    /// Create holes for all eligible UI nodes.
    AutoAll,
    /// Only create holes for nodes explicitly marked with `EguiHole`.
    ManualOnly,
}

impl Default for EguiHolesMode {
    fn default() -> Self { Self::AutoAll }
}

#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource, Default)]
/// Settings for egui hole punching.
pub struct EguiHolesSettings {
    /// Whether egui hole punching is enabled.
    pub enabled: bool,
    /// Whether holes are auto-generated or manual.
    pub mode: EguiHolesMode,
}

impl Default for EguiHolesSettings {
    fn default() -> Self { Self { enabled: true, mode: EguiHolesMode::AutoAll } }
}

#[derive(Resource, Default)]
struct EguiHolesCache {
    hash: HashMap<u32, u64>,
}

/// Plugin that keeps `EguiOcclusionRects` in sync with bevy_ui/bevy_cgui layout.
pub struct EguiHolesPlugin;

impl Plugin for EguiHolesPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<EguiHole>()
            .register_type::<EguiHoleDisable>()
            .register_type::<EguiHolesSettings>()
            .init_resource::<EguiHolesSettings>()
            .init_resource::<EguiHolesCache>();

        #[cfg(feature = "holes_cgui")]
        app.add_systems(
            PostUpdate,
            sync_egui_occlusion
                .after(UiSystems::Layout)
                .after(bevy_cgui::UiSystems::Layout),
        );

        #[cfg(not(feature = "holes_cgui"))]
        app.add_systems(PostUpdate, sync_egui_occlusion.after(UiSystems::Layout));
    }
}

fn window_from_target(
    target: &RenderTarget,
    primary: Option<Entity>,
) -> Option<Entity> {
    match target {
        RenderTarget::Window(WindowRef::Primary) => primary,
        RenderTarget::Window(WindowRef::Entity(win)) => Some(*win),
        _ => None,
    }
}

fn rect_from_node(size: Vec2, translation: Vec2, div: f32) -> Option<egui::Rect> {
    if div <= 0.0 || size.x <= 0.0 || size.y <= 0.0 { return None; }
    let size = egui::vec2(size.x / div, size.y / div);
    let min = egui::pos2((translation.x / div) - size.x * 0.5, (translation.y / div) - size.y * 0.5);
    Some(egui::Rect::from_min_size(min, size))
}

fn auto_wants_hole(mode: EguiHolesMode, hole: Option<&EguiHole>, wants_input: bool) -> bool {
    match mode {
        EguiHolesMode::ManualOnly => hole.is_some(),
        EguiHolesMode::AutoAll => hole.is_some() || wants_input,
    }
}

fn is_too_large_auto_hole(size: Vec2, target: Vec2) -> bool {
    if target.x <= 0.0 || target.y <= 0.0 { return false; }
    let area = size.x.max(0.0) * size.y.max(0.0);
    let t_area = target.x * target.y;
    area / t_area > 0.50
}

fn hash_rects(rects: &mut [egui::Rect]) -> u64 {
    rects.sort_by(|a, b| {
        a.min.x.partial_cmp(&b.min.x)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.min.y.partial_cmp(&b.min.y).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut hasher = DefaultHasher::new();
    for r in rects.iter() {
        r.min.x.to_bits().hash(&mut hasher);
        r.min.y.to_bits().hash(&mut hasher);
        r.max.x.to_bits().hash(&mut hasher);
        r.max.y.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(feature = "holes_cgui")]
fn sync_egui_occlusion(
    windows: Query<&Window>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    cameras: Query<(Entity, &RenderTarget), With<Camera>>,
    egui_settings: Res<EguiSettings>,
    settings: Res<EguiHolesSettings>,
    mut occ: ResMut<EguiOcclusionRects>,
    mut cache: ResMut<EguiHolesCache>,
    q_ui: Query<(
        &ComputedNode,
        &UiGlobalTransform,
        Option<&ComputedUiTargetCamera>,
        Option<&bevy_ui::ComputedUiRenderTargetInfo>,
        Option<&EguiHole>,
        Option<&EguiHoleDisable>,
    )>,
    q_cgui: Query<(
        &bevy_cgui::ComputedNode,
        &bevy_cgui::ui_transform::UiGlobalTransform,
        Option<&bevy_cgui::ComputedUiTargetCamera>,
        Option<&bevy_cgui::ComputedUiRenderTargetInfo>,
        Option<&EguiHole>,
        Option<&EguiHoleDisable>,
    )>,
) {
    if !settings.enabled {
        occ.0.clear();
        cache.hash.clear();
        return;
    }
    let primary = primary_window.single().ok();
    let mut camera_to_window = HashMap::new();
    for (cam, target) in cameras.iter() {
        if let Some(win) = window_from_target(target, primary) {
            camera_to_window.insert(cam, win);
        }
    }
    let mut per_window: HashMap<u32, Vec<egui::Rect>> = HashMap::new();
    let div = egui_settings.scale_factor;

    // bevy_ui: keep AutoAll conservative to avoid nuking egui (manual holes only).
    for (cn, gt, cam, target_info, hole, disable) in q_ui.iter() {
        if disable.is_some() { continue; }
        if !auto_wants_hole(settings.mode, hole, false) { continue; }
        let window = cam
            .and_then(|c| c.get())
            .and_then(|c| camera_to_window.get(&c).copied())
            .or(primary);
        let Some(window) = window else { continue; };
        let (_, _, t) = gt.to_scale_angle_translation();
        let inv = cn.inverse_scale_factor;
        if settings.mode == EguiHolesMode::AutoAll && hole.is_none() {
            let target = target_info.map(|ti| ti.logical_size()).or_else(|| windows.get(window).ok().map(|w| Vec2::new(w.width(), w.height())));
            if let Some(target) = target {
                let size = cn.size * inv;
                if target.x > 0.0 && target.y > 0.0 && size.x >= target.x * 0.98 && size.y >= target.y * 0.98 { continue; }
            }
        }
        let Some(rect) = rect_from_node(cn.size * inv, t * inv, div) else { continue; };
        per_window.entry(window.index().index()).or_default().push(rect);
    }

    // bevy_cgui: AutoAll = all nodes (with guards); ManualOnly = marked nodes.
    for (cn, gt, cam, target_info, hole, disable) in q_cgui.iter() {
        if disable.is_some() { continue; }
        if !auto_wants_hole(settings.mode, hole, true) { continue; }
        let window = cam
            .and_then(|c| c.get())
            .and_then(|c| camera_to_window.get(&c).copied())
            .or(primary);
        let Some(window) = window else { continue; };
        let (_, _, t) = gt.to_scale_angle_translation();
        let inv = cn.inverse_scale_factor;
        if settings.mode == EguiHolesMode::AutoAll && hole.is_none() {
            let target = target_info
                .map(|ti| ti.logical_size())
                .or_else(|| windows.get(window).ok().map(|w| Vec2::new(w.width(), w.height())));
            if let Some(target) = target {
                let size = cn.size * inv;
                // Avoid nuking egui: ignore near-fullscreen & very large auto-holes.
                if target.x > 0.0
                    && target.y > 0.0
                    && (size.x >= target.x * 0.98 && size.y >= target.y * 0.98 || is_too_large_auto_hole(size, target))
                {
                    continue;
                }
            }
        }
        let Some(rect) = rect_from_node(cn.size * inv, t * inv, div) else { continue; };
        per_window.entry(window.index().index()).or_default().push(rect);
    }
    update_occlusion(&mut occ, &mut cache, per_window);
}

#[cfg(not(feature = "holes_cgui"))]
fn sync_egui_occlusion(
    windows: Query<&Window>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    cameras: Query<(Entity, &RenderTarget), With<Camera>>,
    egui_settings: Res<EguiSettings>,
    settings: Res<EguiHolesSettings>,
    mut occ: ResMut<EguiOcclusionRects>,
    mut cache: ResMut<EguiHolesCache>,
    q_ui: Query<(
        &ComputedNode,
        &UiGlobalTransform,
        Option<&ComputedUiTargetCamera>,
        Option<&bevy_ui::ComputedUiRenderTargetInfo>,
        Option<&EguiHole>,
        Option<&EguiHoleDisable>,
    )>,
) {
    if !settings.enabled {
        occ.0.clear();
        cache.hash.clear();
        return;
    }
    let primary = primary_window.single().ok();
    let mut camera_to_window = HashMap::new();
    for (cam, target) in cameras.iter() {
        if let Some(win) = window_from_target(target, primary) {
            camera_to_window.insert(cam, win);
        }
    }
    let mut per_window: HashMap<u32, Vec<egui::Rect>> = HashMap::new();
    let div = egui_settings.scale_factor;
    for (cn, gt, cam, target_info, hole, disable) in q_ui.iter() {
        if disable.is_some() { continue; }
        if !auto_wants_hole(settings.mode, hole, false) { continue; }
        let window = cam
            .and_then(|c| c.get())
            .and_then(|c| camera_to_window.get(&c).copied())
            .or(primary);
        let Some(window) = window else { continue; };
        let (_, _, t) = gt.to_scale_angle_translation();
        let inv = cn.inverse_scale_factor;
        if settings.mode == EguiHolesMode::AutoAll && hole.is_none() {
            let target = target_info.map(|ti| ti.logical_size()).or_else(|| windows.get(window).ok().map(|w| Vec2::new(w.width(), w.height())));
            if let Some(target) = target {
                let size = cn.size * inv;
                if target.x > 0.0 && target.y > 0.0 && size.x >= target.x * 0.98 && size.y >= target.y * 0.98 { continue; }
            }
        }
        let Some(rect) = rect_from_node(cn.size * inv, t * inv, div) else { continue; };
        per_window.entry(window.index().index()).or_default().push(rect);
    }
    update_occlusion(&mut occ, &mut cache, per_window);
}

fn update_occlusion(
    occ: &mut EguiOcclusionRects,
    cache: &mut EguiHolesCache,
    mut per_window: HashMap<u32, Vec<egui::Rect>>,
) {
    let mut new_hash = HashMap::new();
    for (win, rects) in per_window.iter_mut() {
        if rects.is_empty() { continue; }
        let hash = hash_rects(rects.as_mut_slice());
        new_hash.insert(*win, hash);
        if cache.hash.get(win).copied() != Some(hash) {
            #[cfg(debug_assertions)]
            bevy::log::warn!("EGUI_HOLES_UPDATE win={} count={} first={:?}", win, rects.len(), rects.get(0).map(|r| (r.min.x, r.min.y, r.width(), r.height())));
            occ.0.insert(*win, rects.clone());
        }
    }
    for win in cache.hash.keys().copied().collect::<Vec<_>>() {
        if !new_hash.contains_key(&win) {
            #[cfg(debug_assertions)]
            bevy::log::warn!("EGUI_HOLES_CLEAR win={}", win);
            occ.0.remove(&win);
        }
    }
    cache.hash = new_hash;
}
