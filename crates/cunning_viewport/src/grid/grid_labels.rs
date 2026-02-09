use bevy::prelude::*;

use crate::{grid::grid_params::grid_params, viewport_options::{DisplayOptions, ViewportViewMode}, MainCamera, ViewportRenderState};

#[derive(Component)]
pub struct GridLabel;

pub fn update_grid_labels_system(
    mut commands: Commands,
    display_options: Res<DisplayOptions>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    viewport_render_state: Res<ViewportRenderState>,
    mut label_query: Query<(Entity, &mut Text2d, &mut TextFont, &mut TextColor, &mut Transform, &mut Visibility), With<GridLabel>>,
) {
    if !display_options.grid.show || !display_options.grid.show_labels || display_options.view_mode == ViewportViewMode::UV {
        for (_, _, _, _, _, mut vis) in &mut label_query { *vis = Visibility::Hidden; }
        return;
    }

    let Ok((camera, camera_transform)) = camera_query.single() else { return; };
    let vp = viewport_render_state.0.lock().unwrap().viewport_size;
    if vp.x <= 1.0 || vp.y <= 1.0 { return; }

    let Some(p) = grid_params(camera, camera_transform, vp, display_options.grid.major_target_px) else { return; };
    let major_step = p.major_step;

    let mut desired_labels = Vec::new();
    let x_axis_color = Color::srgba(0.6, 0.6, 0.6, 0.8);
    let z_axis_color = Color::srgba(0.6, 0.6, 0.6, 0.8);
    let text_scale = major_step * 0.02;

    let i0 = ((p.center.x - p.half_extent) / major_step).ceil() as i32;
    let i1 = ((p.center.x + p.half_extent) / major_step).floor() as i32;
    for i in i0..=i1 {
        let x = i as f32 * major_step;
        desired_labels.push((Vec3::new(x, 0.01, 0.0), format_val(x, major_step), x_axis_color));
    }

    let k0 = ((p.center.z - p.half_extent) / major_step).ceil() as i32;
    let k1 = ((p.center.z + p.half_extent) / major_step).floor() as i32;
    for k in k0..=k1 {
        if k == 0 { continue; }
        let z = k as f32 * major_step;
        desired_labels.push((Vec3::new(0.0, 0.01, z), format_val(z, major_step), z_axis_color));
    }

    let mut iter = label_query.iter_mut();
    for (pos, text, color) in desired_labels {
        if let Some((_, mut t_text, mut t_font, mut t_color, mut t_trans, mut t_vis)) = iter.next() {
            t_text.0 = text;
            t_font.font_size = FontSize::Px(12.0);
            t_color.0 = color;
            *t_trans = Transform::from_translation(pos).with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)).with_scale(Vec3::splat(text_scale));
            *t_vis = Visibility::Inherited;
        } else {
            commands.spawn((
                Text2d::new(text),
                TextFont::from_font_size(12.0),
                TextColor(color),
                Transform::from_translation(pos).with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)).with_scale(Vec3::splat(text_scale)),
                GridLabel,
                Visibility::Inherited,
            ));
        }
    }

    for (_, _, _, _, _, mut vis) in iter { *vis = Visibility::Hidden; }
}

fn format_val(val: f32, major_step: f32) -> String {
    let (v, unit) = if major_step >= 1.0 { (val, "") } else if major_step >= 0.01 { (val * 100.0, "cm") } else { (val * 1000.0, "mm") };
    if (v - v.round()).abs() < 1e-5 { format!("{:.0}{unit}", v) } else if v.abs() >= 10.0 { format!("{:.0}{unit}", v) } else { format!("{:.1}{unit}", v) }
}

