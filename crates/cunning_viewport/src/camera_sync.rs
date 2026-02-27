use bevy::camera::{RenderTarget, Viewport};
use bevy::prelude::*;
use bevy::window::WindowRef;

use crate::{layout::ViewportLayout, viewport_options::DisplayOptions, MainCamera};

pub fn sync_main_camera_viewport(
    viewport_layout: Res<ViewportLayout>,
    windows: Query<(Entity, &Window)>,
    primary_windows: Query<Entity, With<bevy::window::PrimaryWindow>>,
    mut camera_query: Query<(&mut Camera, &mut Projection, &Transform, &mut RenderTarget), With<MainCamera>>,
    mut display_options: ResMut<DisplayOptions>,
) {
    let Ok((mut camera, mut projection, transform, mut target)) = camera_query.single_mut() else { return; };
    let Some(logical_rect) = viewport_layout.logical_rect else {
        *target = RenderTarget::Window(WindowRef::Primary);
        camera.viewport = None;
        return;
    };

    let target_window_entity = viewport_layout.window_entity.or_else(|| primary_windows.iter().next());
    let Some(target_window_entity) = target_window_entity else {
        camera.is_active = false;
        *target = RenderTarget::Window(WindowRef::Primary);
        camera.viewport = None;
        return;
    };

    let Ok((entity, window)) = windows.get(target_window_entity) else {
        camera.is_active = primary_windows.iter().next().is_some();
        *target = RenderTarget::Window(WindowRef::Primary);
        camera.viewport = None;
        return;
    };

    if display_options.camera_rotation != transform.rotation {
        // Camera rotation is derived state; avoid invalidating DisplayOptions on every drag frame.
        display_options.bypass_change_detection().camera_rotation = transform.rotation;
    }

    let scale_factor = window.scale_factor() as f32;
    if scale_factor <= 0.0 { return; }

    let width = logical_rect.width();
    let height = logical_rect.height();
    if width <= 1.0 || height <= 1.0 { return; }

    let physical_width = (width * scale_factor) as u32;
    let physical_height = (height * scale_factor) as u32;

    let window_physical_height = window.physical_height();
    let window_physical_width = window.physical_width();

    let mut x = (logical_rect.min.x * scale_factor) as u32;
    let mut y = (logical_rect.min.y * scale_factor) as u32;
    x = x.min(window_physical_width);
    y = y.min(window_physical_height);

    let max_width = window_physical_width.saturating_sub(x);
    let max_height = window_physical_height.saturating_sub(y);
    if max_width == 0 || max_height == 0 || x >= window_physical_width || y >= window_physical_height { return; }

    let final_width = physical_width.min(max_width).max(1);
    let final_height = physical_height.min(max_height).max(1);

    *target = RenderTarget::Window(WindowRef::Entity(entity));
    camera.viewport = Some(Viewport { physical_position: UVec2::new(x, y), physical_size: UVec2::new(final_width, final_height), ..Default::default() });

    if let Projection::Perspective(ref mut perspective) = *projection {
        if physical_height > 0 { perspective.aspect_ratio = physical_width as f32 / physical_height as f32; }
    }
}

