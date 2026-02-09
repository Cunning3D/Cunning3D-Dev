pub mod backend;
pub mod knowledge_cache;
pub mod prefix_cache;
pub mod prompt;

pub use backend::{TinyModelHost, TinyRequest, TinyResponse};
pub use knowledge_cache::{KnowledgeCache, NodeKnowledge};

use bevy::prelude::*;

pub struct NativeTinyModelPlugin;

impl Plugin for NativeTinyModelPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(knowledge_cache::KnowledgeCachePlugin)
            .insert_resource(TinyModelHost::new());
    }
}
