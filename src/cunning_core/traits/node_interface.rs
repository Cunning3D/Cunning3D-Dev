use crate::libs::geometry::geo_ref::GeometryRef;
use crate::nodes::parameter::Parameter;
use bevy_egui::egui;
use std::any::{Any, TypeId};
use std::sync::Arc;
use uuid::Uuid;

/// Defines the compute logic of a node.
pub trait NodeOp: Send + Sync {
    /// Execute the node operation.
    /// Returns the generated/modified Geometry.
    /// Inputs are immutable (Arc). Output is wrapped in Arc.
    fn compute(
        &self,
        params: &[Parameter],
        inputs: &[Arc<dyn GeometryRef>],
    ) -> Arc<crate::mesh::Geometry>;
}

/// Defines the parameters for a node.
pub trait NodeParameters {
    fn define_parameters() -> Vec<Parameter>;
}

/// A helper trait to access resources/services without depending on specific Context structs.
pub trait ServiceProvider {
    fn get_service(&self, service_type: TypeId) -> Option<&dyn Any>;
}

impl dyn ServiceProvider + '_ {
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.get_service(TypeId::of::<T>())
            .and_then(|s| s.downcast_ref::<T>())
    }
}

use bevy::prelude::{Color, Resource, Transform, Vec2, Vec3};

pub struct GizmoContext {
    pub ray_origin: Vec3,
    pub ray_direction: Vec3,
    pub mouse_left_pressed: bool,
    pub mouse_left_just_pressed: bool,
    pub mouse_left_just_released: bool,
    pub cursor_pos: Vec2, // Viewport relative
    pub cam_pos: Vec3,
    pub cam_up: Vec3,
    pub cam_rotation: bevy::prelude::Quat, // Added for Planar Billboarding
    pub is_orthographic: bool,
    pub scale_factor: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GizmoPart {
    TranslateX,
    TranslateY,
    TranslateZ,
    TranslatePlanarXY,
    TranslatePlanarXZ,
    TranslatePlanarYZ,
    ScaleX,
    ScaleY,
    ScaleZ,
    ScaleUniform,
    RotateX,
    RotateY,
    RotateZ,
    RotateScreen,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum XformGizmoMode {
    #[default]
    Aggregate,
    Move,
    Scale,
    All,
}

#[derive(Resource, Default)]
pub struct GizmoState {
    pub active_node_id: Option<Uuid>,
    pub active_part: Option<GizmoPart>,
    pub drag_start_pos: Option<Vec3>,
    pub drag_start_ray: Option<(Vec3, Vec3)>, // Origin, Direction
    pub initial_transform_pos: Option<Vec3>,
    pub xform_mode: XformGizmoMode,
    pub graph_modified: bool,
}

// --- V5 Retained Gizmo Types ---

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GizmoPrimitive {
    Cylinder, // Shaft
    Cone,     // Arrow Head
    Cube,     // Scale Handle / Center
    Sphere,   // Rotate Handle
    Plane,    // Planar Handle
}

pub enum GizmoCommand {
    Mesh {
        primitive: GizmoPrimitive,
        transform: Transform,
        color: Color,
    },
    Line {
        start: Vec3,
        end: Vec3,
        color: Color,
    },
}

#[derive(Resource, Default)]
pub struct GizmoDrawBuffer {
    pub commands: Vec<GizmoCommand>,
}

impl GizmoDrawBuffer {
    pub fn clear(&mut self) {
        self.commands.clear();
    }

    pub fn add(&mut self, cmd: GizmoCommand) {
        self.commands.push(cmd);
    }

    pub fn draw_mesh(&mut self, primitive: GizmoPrimitive, transform: Transform, color: Color) {
        self.add(GizmoCommand::Mesh {
            primitive,
            transform,
            color,
        });
    }

    pub fn draw_line(&mut self, start: Vec3, end: Vec3, color: Color) {
        self.add(GizmoCommand::Line { start, end, color });
    }
}

/// Defines the interaction capabilities of a node.
/// Nodes can implement this to provide HUDs, Gizmos, and Input handling.
pub trait NodeInteraction: Send + Sync {
    // Default implementations allow nodes to opt-out of specific interactions

    fn has_hud(&self) -> bool {
        false
    }
    fn draw_hud(&self, _ui: &mut egui::Ui, _services: &dyn ServiceProvider, _node_id: Uuid) {}

    // Updated Signature: Uses GizmoDrawBuffer instead of Gizmos
    fn draw_gizmos(
        &self,
        _buffer: &mut GizmoDrawBuffer,
        _context: &GizmoContext,
        _gizmo_state: &mut GizmoState,
        _services: &dyn ServiceProvider,
        _node_id: Uuid,
    ) {
    }

    /// Coverlay is a stronger, tool-style overlay used as a parameter UI complement for high-interaction nodes.
    fn has_coverlay(&self) -> bool {
        false
    }
    fn draw_coverlay(&self, _ui: &mut egui::Ui, _services: &dyn ServiceProvider, _node_id: Uuid) {}

    // Future:
    // fn handle_input(&mut self, input: &InputContext);
}
