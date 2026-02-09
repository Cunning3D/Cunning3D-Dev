use bevy::camera::{RenderTarget, Viewport};
use bevy::prelude::*;
use bevy::window::WindowRef;

use crate::tabs_system::viewport_3d::ViewportLayout;

#[inline]
fn same_window_target(a: &RenderTarget, b: &RenderTarget) -> bool {
    match (a, b) {
        (RenderTarget::Window(WindowRef::Primary), RenderTarget::Window(WindowRef::Primary)) => {
            true
        }
        (
            RenderTarget::Window(WindowRef::Entity(ae)),
            RenderTarget::Window(WindowRef::Entity(be)),
        ) => ae == be,
        _ => false,
    }
}

pub fn sync_main_camera_viewport(
    viewport_layout: Res<ViewportLayout>,
    windows: Query<(Entity, &Window)>,
    primary_windows: Query<Entity, With<bevy::window::PrimaryWindow>>,
    mut camera_query: Query<
        (&mut Camera, &mut Projection, &Transform, &mut RenderTarget),
        With<crate::MainCamera>,
    >,
    mut display_options: ResMut<crate::viewport_options::DisplayOptions>,
) {
    let Ok((mut camera, mut projection, transform, mut target)) = camera_query.single_mut() else {
        return;
    };
    let Some(logical_rect) = viewport_layout.logical_rect else {
        let desired_target = RenderTarget::Window(WindowRef::Primary);
        if !same_window_target(&*target, &desired_target) {
            *target = desired_target;
        }
        if camera.viewport.is_some() {
            camera.viewport = None;
        }
        return;
    };
    // Bevy 0.18: never fall back to `Entity::PLACEHOLDER` for window targets.
    let target_window_entity = viewport_layout
        .window_entity
        .or_else(|| primary_windows.iter().next());
    let Some(target_window_entity) = target_window_entity else {
        camera.is_active = false;
        let desired_target = RenderTarget::Window(WindowRef::Primary);
        if !same_window_target(&*target, &desired_target) {
            *target = desired_target;
        }
        if camera.viewport.is_some() {
            camera.viewport = None;
        }
        return;
    };

    let Ok((entity, window)) = windows.get(target_window_entity) else {
        // The viewport window got closed or isn't ready yet
        // Keep target stable (Primary) until the window exists.
        camera.is_active = primary_windows.iter().next().is_some();
        let desired_target = RenderTarget::Window(WindowRef::Primary);
        if !same_window_target(&*target, &desired_target) {
            *target = desired_target;
        }
        if camera.viewport.is_some() {
            camera.viewport = None;
        }
        return;
    };

    // Sync rotation for Gizmo UI
    if display_options.camera_rotation != transform.rotation {
        display_options.camera_rotation = transform.rotation;
    }

    let scale_factor = window.scale_factor() as f32;
    if scale_factor <= 0.0 {
        return;
    }

    let width = logical_rect.width();
    let height = logical_rect.height();
    if width <= 1.0 || height <= 1.0 {
        return;
    }

    let physical_width = (width * scale_factor).round() as u32;
    let physical_height = (height * scale_factor).round() as u32;

    let window_physical_height = window.physical_height();
    let window_physical_width = window.physical_width();

    // Bevy viewport origin is top-left. Egui logical rect is also top-left.
    // So we should NOT flip Y here.
    let mut x = (logical_rect.min.x * scale_factor).round().max(0.0) as u32;
    let mut y = (logical_rect.min.y * scale_factor).round().max(0.0) as u32;

    // Clamp position to window bounds
    x = x.min(window_physical_width);
    y = y.min(window_physical_height);

    // Compute max size that still fits inside the window from this position
    let max_width = window_physical_width.saturating_sub(x);
    let max_height = window_physical_height.saturating_sub(y);

    // If nothing fits, skip updating the viewport to avoid an invalid rect
    if max_width == 0
        || max_height == 0
        || x >= window_physical_width
        || y >= window_physical_height
    {
        return;
    }

    let final_width = physical_width.min(max_width).max(1);
    let final_height = physical_height.min(max_height).max(1);

    let desired_target = RenderTarget::Window(WindowRef::Entity(entity));
    if !same_window_target(&*target, &desired_target) {
        *target = desired_target;
    }
    let desired_viewport = Viewport {
        physical_position: UVec2::new(x, y),
        physical_size: UVec2::new(final_width, final_height),
        ..Default::default()
    };
    let needs_vp = camera
        .viewport
        .as_ref()
        .map(|vp| {
            vp.physical_position != desired_viewport.physical_position
                || vp.physical_size != desired_viewport.physical_size
        })
        .unwrap_or(true);
    if needs_vp {
        camera.viewport = Some(desired_viewport);
    }

    if let Projection::Perspective(ref mut perspective) = *projection {
        if final_height > 0 {
            let ar = final_width as f32 / final_height as f32;
            if (perspective.aspect_ratio - ar).abs() > 1e-6 {
                perspective.aspect_ratio = ar;
            }
        }
    }
}
