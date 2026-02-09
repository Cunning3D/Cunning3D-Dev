use bevy::prelude::{Entity, Resource};
use bevy_egui::egui;

#[derive(Resource, Default, Clone)]
pub struct ViewportLayout {
    pub window_entity: Option<Entity>,
    pub logical_rect: Option<egui::Rect>,
}

