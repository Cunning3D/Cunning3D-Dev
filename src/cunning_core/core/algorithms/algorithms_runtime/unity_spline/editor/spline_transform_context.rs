use bevy::math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PivotMode { Pivot, Center }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandleOrientation { Global, Parent, Element }

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct SnappingFlags { pub incremental_snap_active: bool, pub grid_snap_active: bool }

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TransformContext {
    pub pivot_mode: PivotMode,
    pub handle_orientation: HandleOrientation,
    pub pivot_position_world: Vec3,
    pub handle_rotation_world: Quat,
    pub move_snap: Vec3,
    pub snapping: SnappingFlags,
}

impl Default for TransformContext {
    fn default() -> Self {
        Self {
            pivot_mode: PivotMode::Pivot,
            handle_orientation: HandleOrientation::Element,
            pivot_position_world: Vec3::ZERO,
            handle_rotation_world: Quat::IDENTITY,
            move_snap: Vec3::ONE,
            snapping: SnappingFlags::default(),
        }
    }
}

pub trait TransformContextAdapter {
    fn get_handle_size(&self, world_pos: Vec3) -> f32;
}

