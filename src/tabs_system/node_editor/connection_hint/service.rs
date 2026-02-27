use crate::libs::ai_service::native_tiny_model::prompt::TinyPromptBuilder;
use crate::libs::ai_service::native_tiny_model::{KnowledgeCache, TinyModelHost};
use bevy::prelude::*;
use std::time::{Duration, Instant};

#[derive(Resource, Default)]
pub struct ConnectionHintState {
    pub last_hovered_pair: Option<(String, String)>, // (SourceNode, TargetNode)
    pub hover_start_time: Option<Instant>,
    pub pending_request_id: Option<String>,
    pub current_hint: Option<String>,
}

pub fn connection_hint_system(
    mut state: ResMut<ConnectionHintState>,
    tiny_host: Res<TinyModelHost>,
    knowledge: Res<KnowledgeCache>,
    // In a real implementation, we'd read events or query UI state here to know what's being hovered.
    // For now, we assume an external system updates `state.last_hovered_pair`.
) {
    // 1. Check for response
    for resp in tiny_host.poll() {
        if Some(resp.id.clone()) == state.pending_request_id {
            state.current_hint = Some(resp.text);
            state.pending_request_id = None;
        }
    }

    // 2. Handle Debounce Trigger
    if let Some((source, target)) = state.last_hovered_pair.clone() {
        if let Some(start) = state.hover_start_time {
            if start.elapsed() > Duration::from_millis(200)
                && state.pending_request_id.is_none()
                && state.current_hint.is_none()
            {
                // Trigger Request
                let req_id = format!("hint_{}_{}", source, target);
                state.pending_request_id = Some(req_id.clone());

                // Build Prompt
                let source_node = knowledge
                    .get(&source)
                    .map(|n| n.name.as_str())
                    .unwrap_or(source.as_str());
                let source_type = knowledge
                    .get(&source)
                    .map(|n| n.io.output_type.as_str())
                    .unwrap_or("Unknown");

                let target_node = knowledge
                    .get(&target)
                    .map(|n| n.name.as_str())
                    .unwrap_or(target.as_str());
                let target_type = knowledge
                    .get(&target)
                    .map(|n| n.io.input_type.as_str())
                    .unwrap_or("Unknown");

                let prompt = TinyPromptBuilder::build_connection_hint(
                    source_node,
                    source_type,
                    target_node,
                    target_type,
                );

                tiny_host.request(&req_id, &prompt);
            }
        }
    } else {
        // Reset if no longer hovering
        state.hover_start_time = None;
        state.current_hint = None;
        state.pending_request_id = None;
    }
}
