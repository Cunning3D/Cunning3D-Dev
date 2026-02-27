//! Defines the parameter system for nodes.
use bevy::prelude::{IVec2, Vec2, Vec3, Vec4};
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use crate::libs::algorithms::algorithms_runtime::unity_spline::SplineContainer as UnitySplineContainer;

pub type ParameterId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PointMode {
    Corner,
    Bezier,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CurveControlPoint {
    pub id: Uuid,
    pub position: Vec3,
    pub mode: PointMode,
    pub handle_in: Vec3,
    pub handle_out: Vec3,
    pub weight: f32, // For NURBS
}

impl CurveControlPoint {
    pub fn new(position: Vec3) -> Self {
        Self {
            id: Uuid::new_v4(),
            position,
            mode: PointMode::Corner,
            handle_in: Vec3::new(-1.0, 0.0, 0.0),
            handle_out: Vec3::new(1.0, 0.0, 0.0),
            weight: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CurveType {
    Polygon,
    Bezier,
    Nurbs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CurveData {
    pub points: Vec<CurveControlPoint>,
    pub is_closed: bool,
    pub curve_type: CurveType,
}

impl Default for CurveData {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            is_closed: false,
            curve_type: CurveType::Polygon,
        }
    }
}

/// Defines the value of a parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParameterValue {
    Float(f32),
    Int(i32),
    Vec2(Vec2),
    Vec3(Vec3),
    Vec4(Vec4),
    IVec2(IVec2),
    String(String),
    Color(Vec3), // RGB, [0-1] range
    Color4(Vec4), // RGBA, [0-1] range
    Bool(bool),
    Curve(CurveData),
    UnitySpline(UnitySplineContainer),
    Volume(crate::sdf::SdfHandle),
}

/// Defines how a parameter should be displayed in the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParameterUIType {
    FloatSlider { min: f32, max: f32 },
    IntSlider { min: i32, max: i32 },
    Vec2Drag,
    Vec3Drag,
    Vec4Drag,
    IVec2Drag,
    String,
    Toggle,
    Button,
    /// A button that is disabled while `busy_param` is truthy. Intended for async jobs.
    BusyButton {
        busy_param: String,
        busy_label: String,
        #[serde(default)]
        busy_label_param: Option<String>,
    },
    Dropdown { choices: Vec<(String, i32)> },
    Color { show_alpha: bool },
    Separator,
    CurvePoints, // Special UI for displaying curve point list/info
    UnitySpline, // Special UI for Unity-isomorphic spline editing
    Code, // Syntax-highlighted code editor
    FilePath { filters: Vec<String> }, // File picker, filters e.g. ["fbx", "obj", "gltf"]
}

/// The core parameter structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub id: ParameterId,
    pub name: String,
    pub label: String,
    pub group: String,
    pub value: ParameterValue,
    pub ui_type: ParameterUIType,
    #[serde(default)]
    pub visible_condition: Option<String>,
}

impl Parameter {
    pub fn new(
        name: &str,
        label: &str,
        group: &str,
        value: ParameterValue,
        ui_type: ParameterUIType,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            label: label.to_string(),
            group: group.to_string(),
            value,
            ui_type,
            visible_condition: None,
        }
    }

    pub fn with_condition(mut self, condition: impl Into<String>) -> Self {
        self.visible_condition = Some(condition.into());
        self
    }
}
