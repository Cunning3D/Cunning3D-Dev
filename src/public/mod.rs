use bevy::{
    input::mouse::{MouseMotion, MouseWheel},
    prelude::*,
};
use std::collections::HashSet;

use crate::{mesh::Attribute, mesh::Geometry, viewport_options::DisplayOptions};

#[derive(Component, Clone, Copy)]
pub struct PointTag;

pub fn spawn_point_entities<T: Component + Copy>(
    commands: &mut Commands,
    positions: &[Vec3],
    mesh_handle: &Handle<Mesh>,
    material_handle: &Handle<StandardMaterial>,
    tag: T,
) {
    for &position in positions {
        commands.spawn((
            Mesh3d(mesh_handle.clone()),
            MeshMaterial3d(material_handle.clone()),
            Transform::from_translation(position),
            Visibility::Inherited,
            tag,
        ));
    }
}

/// Determines which points are visible based on the backface culling setting.
pub fn get_visible_points(
    geo: &Geometry,
    _display_options: &DisplayOptions,
    _camera_transform: &GlobalTransform,
) -> HashSet<usize> {
    let mut visible_points = HashSet::new();
    let positions = match geo.get_point_position_attribute() {
        Some(p) => p,
        _ => return visible_points,
    };

    // Backface culling disabled/removed
    visible_points.extend(0..positions.len());
    visible_points
}
