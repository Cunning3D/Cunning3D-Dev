pub mod service;
pub mod ui;

use bevy::prelude::*;

pub struct ConnectionHintPlugin;

impl Plugin for ConnectionHintPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<service::ConnectionHintState>()
            .add_systems(Update, service::connection_hint_system);
    }
}
