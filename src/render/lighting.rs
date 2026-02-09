use bevy::prelude::*;
use crate::viewport_options::{DisplayOptions, ViewportLightingMode};
use crate::MainCamera;

#[derive(Component)]
pub struct Headlight;

/// Updates the headlight position and rotation to match the main camera.
pub fn update_headlight_transform_system(
    camera_query: Query<&GlobalTransform, With<MainCamera>>, 
    mut headlight_query: Query<&mut Transform, With<Headlight>>,
) {
    if let Some(cam_transform) = camera_query.iter().next() {
        for mut light_transform in headlight_query.iter_mut() {
            *light_transform = cam_transform.compute_transform();
        }
    }
}

/// Controls the visibility of the headlight and scene lights based on the selected lighting mode.
pub fn viewport_lighting_control_system(
    mut commands: Commands,
    display_options: Res<DisplayOptions>,
    headlight_query: Query<Entity, With<Headlight>>,
) {
    // Ensure headlight exists
    if headlight_query.is_empty() {
        commands.spawn((
            DirectionalLight {
                color: Color::WHITE,
                illuminance: 3000.0,
                // shadows_enabled: false,
                ..default()
            },
            Headlight,
            Transform::IDENTITY,
            GlobalTransform::IDENTITY,
            Visibility::default(), 
            Name::new("Viewport Headlight"),
        ));
    }
}

pub fn viewport_lighting_update_system(
    display_options: Res<DisplayOptions>,
    mut headlight_query: Query<(&mut DirectionalLight, &mut Visibility), With<Headlight>>,
    mut scene_lights_query: Query<(&mut PointLight, &mut Visibility, Option<&mut DirectionalLight>, Option<&mut SpotLight>), (Without<Headlight>, With<GlobalTransform>)>, 
) {
    let mode = display_options.lighting_mode;

    // 1. Configure Headlight
    for (mut light, mut vis) in headlight_query.iter_mut() {
        match mode {
            ViewportLightingMode::HeadlightOnly => {
                light.illuminance = 3000.0;
                // light.shadows_enabled = false;
                *vis = Visibility::Inherited;
            }
            ViewportLightingMode::FullLighting | ViewportLightingMode::FullLightingWithShadow => {
                *vis = Visibility::Hidden; 
            }
        }
    }
}

pub fn update_scene_point_lights(
    display_options: Res<DisplayOptions>,
    mut query: Query<(&mut PointLight, &mut Visibility), Without<Headlight>>,
) {
    let mode = display_options.lighting_mode;
    let target_vis = match mode {
        ViewportLightingMode::HeadlightOnly => Visibility::Hidden,
        _ => Visibility::Inherited,
    };
    let shadows = match mode {
        ViewportLightingMode::FullLightingWithShadow => true,
        _ => false,
    };

    for (mut light, mut vis) in query.iter_mut() {
        if *vis != target_vis { *vis = target_vis; }
        // if light.shadows_enabled != shadows { light.shadows_enabled = shadows; } 
    }
}

pub fn update_scene_dir_lights(
    display_options: Res<DisplayOptions>,
    mut query: Query<(&mut DirectionalLight, &mut Visibility), Without<Headlight>>,
) {
    let mode = display_options.lighting_mode;
    let target_vis = match mode {
        ViewportLightingMode::HeadlightOnly => Visibility::Hidden,
        _ => Visibility::Inherited,
    };
    let shadows = match mode {
        ViewportLightingMode::FullLightingWithShadow => true,
        _ => false,
    };

    for (mut light, mut vis) in query.iter_mut() {
        if *vis != target_vis { *vis = target_vis; }
        // if light.shadows_enabled != shadows { light.shadows_enabled = shadows; } 
    }
}

pub fn update_scene_spot_lights(
    display_options: Res<DisplayOptions>,
    mut query: Query<(&mut SpotLight, &mut Visibility), Without<Headlight>>,
) {
    let mode = display_options.lighting_mode;
    let target_vis = match mode {
        ViewportLightingMode::HeadlightOnly => Visibility::Hidden,
        _ => Visibility::Inherited,
    };
    let shadows = match mode {
        ViewportLightingMode::FullLightingWithShadow => true,
        _ => false,
    };

    for (mut light, mut vis) in query.iter_mut() {
        if *vis != target_vis { *vis = target_vis; }
        // if light.shadows_enabled != shadows { light.shadows_enabled = shadows; } 
    }
}
