use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::{
    SplineSelectionState, TransformContext,
};
use crate::libs::algorithms::algorithms_runtime::unity_spline::DrawingDirection;
use crate::nodes::NodeId;
use bevy::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplineTransformTool {
    Move,
    Rotate,
    Scale,
}

impl Default for SplineTransformTool {
    fn default() -> Self {
        Self::Move
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplineAxisConstraint {
    None,
    X,
    Y,
    Z,
}

impl Default for SplineAxisConstraint {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HoveredCurve {
    pub spline_index: usize,
    pub curve_index: usize,
    pub t: f32,
    pub world_pos: Vec3,
    pub dist: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct DirectDragKnot {
    pub node_id: NodeId,
    pub spline_index: usize,
    pub knot_index: usize,
    pub plane_origin_world: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplineEditMode {
    Edit,
    Draw,
}

impl Default for SplineEditMode {
    fn default() -> Self {
        Self::Edit
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SplineDrawState {
    pub node_id: NodeId,
    pub spline_index: usize,
    pub dir: DrawingDirection,
    pub allow_delete_if_no_curves: bool,
}

#[derive(Resource, Default, Clone, Debug)]
pub struct SplineToolState {
    pub mode: SplineEditMode,
    pub tool: SplineTransformTool,
    pub axis: SplineAxisConstraint,
    pub selection: SplineSelectionState,
    pub ctx: TransformContext,
    pub hovered_curve: Option<HoveredCurve>,
    pub drag_last_world: Option<Vec3>,
    pub drag_last_scalar: Option<f32>,
    pub direct_drag_knot: Option<DirectDragKnot>,
    pub draw_state: Option<SplineDrawState>,
}
