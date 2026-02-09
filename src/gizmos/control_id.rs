use bevy::prelude::*;

/// Minimal Unity-like control state: nearest (hover) + hot (active drag).
#[derive(Resource, Debug, Clone)]
pub struct ControlIdState {
    pub nearest_entity: Option<Entity>,
    pub nearest_dist_sq: f32,
    pub nearest_proj: f32,
    pub hot_entity: Option<Entity>,
}

impl Default for ControlIdState {
    fn default() -> Self {
        Self {
            nearest_entity: None,
            nearest_dist_sq: f32::MAX,
            nearest_proj: f32::MAX,
            hot_entity: None,
        }
    }
}

impl ControlIdState {
    #[inline]
    pub fn begin_frame(&mut self) {
        self.nearest_entity = None;
        self.nearest_dist_sq = f32::MAX;
        self.nearest_proj = f32::MAX;
    }

    #[inline]
    pub fn consider(&mut self, entity: Entity, dist_sq: f32, proj: f32) {
        if !dist_sq.is_finite() || dist_sq < 0.0 || !proj.is_finite() || proj < 0.0 {
            return;
        }
        if dist_sq < self.nearest_dist_sq
            || (dist_sq == self.nearest_dist_sq && proj < self.nearest_proj)
        {
            self.nearest_dist_sq = dist_sq;
            self.nearest_proj = proj;
            self.nearest_entity = Some(entity);
        }
    }
}
