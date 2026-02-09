use bevy::prelude::*;

#[derive(Resource, Default, Debug)]
pub struct NavigationInput {
    pub orbit_delta: Vec2,
    pub pan_delta: Vec2,
    pub zoom_delta: f32,
    pub fly_vector: Vec3,
    pub active: bool,
}

