// Grid Label Component
use super::grid_params::{grid_params, GridParams};
use crate::tabs_system::viewport_3d::ViewportLayout;
use crate::viewport_options::{DisplayOptions, ViewportViewMode};
use crate::MainCamera;
use bevy::prelude::*;

#[derive(Component)]
pub struct GridLabel;

// New System: Update Grid Labels (World Space Text2d)
pub fn update_grid_labels_system(
    mut commands: Commands,
    display_options: Res<DisplayOptions>,
    camera_query: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    viewport_layout: Res<ViewportLayout>,
    mut label_query: Query<
        (
            Entity,
            &mut Text2d,
            &mut TextFont,
            &mut TextColor,
            &mut Transform,
            &mut Visibility,
        ),
        With<GridLabel>,
    >,
) {
    // 1. Check if we should draw
    if !display_options.grid.show
        || !display_options.grid.show_labels
        || display_options.view_mode == ViewportViewMode::UV
    {
        // Hide all labels
        for (_, _, _, _, _, mut vis) in &mut label_query {
            *vis = Visibility::Hidden;
        }
        return;
    }

    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };
    let vp = viewport_layout
        .logical_rect
        .map(|r| r.size())
        .unwrap_or_default();
    if vp.x <= 1.0 || vp.y <= 1.0 {
        return;
    }

    // 2. Calculate Grid Params
    let Some(p) = grid_params(
        camera,
        camera_transform,
        vp,
        display_options.grid.major_target_px,
    ) else {
        return;
    };
    let major_step = p.major_step;

    // 3. Generate Desired Labels (only along X and Z axes)
    let mut desired_labels = Vec::new();

    // Colors matching axes (Houdini Style: Dark Gray, not colored)
    let x_axis_color = Color::srgba(0.6, 0.6, 0.6, 0.8);
    let z_axis_color = Color::srgba(0.6, 0.6, 0.6, 0.8);

    // Scale factor for text (adjust as needed for readability)
    let text_scale = major_step * 0.02;

    // X-Axis Labels (at Z=0)
    let i0 = ((p.center.x - p.half_extent) / major_step).ceil() as i32;
    let i1 = ((p.center.x + p.half_extent) / major_step).floor() as i32;

    for i in i0..=i1 {
        // Draw 0 on X axis
        let x = i as f32 * major_step;
        // Lift slightly Y=0.01 to avoid Z-fighting with grid lines
        desired_labels.push((
            Vec3::new(x, 0.01, 0.0),
            format_val(x, major_step),
            x_axis_color,
        ));
    }

    // Z-Axis Labels (at X=0)
    let k0 = ((p.center.z - p.half_extent) / major_step).ceil() as i32;
    let k1 = ((p.center.z + p.half_extent) / major_step).floor() as i32;

    for k in k0..=k1 {
        if k == 0 {
            continue;
        } // Skip 0 on Z axis to avoid overlap (X axis draws it)
        let z = k as f32 * major_step;
        desired_labels.push((
            Vec3::new(0.0, 0.01, z),
            format_val(z, major_step),
            z_axis_color,
        ));
    }

    // 4. Update Entities (Pool)
    let mut iter = label_query.iter_mut();

    for (pos, text, color) in desired_labels {
        if let Some((_, mut t_text, mut t_font, mut t_color, mut t_trans, mut t_vis)) = iter.next()
        {
            // Update existing
            t_text.0 = text;
            t_font.font_size = 12.0;
            t_color.0 = color;

            // Lying flat: Rotate -90 deg around X
            *t_trans = Transform::from_translation(pos)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::splat(text_scale));

            *t_vis = Visibility::Inherited;
        } else {
            // Spawn new
            commands.spawn((
                Text2d::new(text),
                TextFont::from_font_size(12.0),
                TextColor(color),
                Transform::from_translation(pos)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                    .with_scale(Vec3::splat(text_scale)),
                GridLabel,
                Visibility::Inherited,
            ));
        }
    }

    // Hide remaining
    for (_, _, _, _, _, mut vis) in iter {
        *vis = Visibility::Hidden;
    }
}

fn format_val(val: f32, major_step: f32) -> String {
    let (v, unit) = if major_step >= 1.0 {
        (val, "")
    } else if major_step >= 0.01 {
        (val * 100.0, "cm")
    } else {
        (val * 1000.0, "mm")
    };
    let unit_suffix = if unit.is_empty() {
        "".to_string()
    } else {
        unit.to_string()
    };

    // Remove .0 for integers
    if (v - v.round()).abs() < 1e-5 {
        format!("{:.0}{}", v, unit_suffix)
    } else if v.abs() >= 1000.0 {
        format!("{:.0}{}", v, unit_suffix)
    } else if v.abs() >= 10.0 {
        format!("{:.0}{}", v, unit_suffix)
    } else {
        format!("{:.1}{}", v, unit_suffix)
    }
}
