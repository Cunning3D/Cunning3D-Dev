//! CDA drag payload definitions (for egui DragAndDrop)
use crate::nodes::structs::NodeId;

/// Drag payload for internal node parameters
#[derive(Clone, Debug)]
pub struct ParamDragPayload {
    pub source_node: NodeId,          // Source node
    pub param_name: String,           // Parameter name
    pub channel_index: Option<usize>, // Channel index (for vector types)
    pub param_type: ParamTypeHint,    // Type hint (for matching)
    pub display_label: String,        // Display label
}

/// Parameter type hint (for drag-and-drop matching)
#[derive(Clone, Debug, PartialEq)]
pub enum ParamTypeHint {
    Float,
    Int,
    Bool,
    Vec2,
    Vec3,
    Vec4,
    Color,
    String,
    Unknown,
}

impl ParamTypeHint {
    /// Infer type from ParameterValue
    pub fn from_value(value: &crate::nodes::parameter::ParameterValue) -> Self {
        use crate::nodes::parameter::ParameterValue;
        match value {
            ParameterValue::Float(_) => Self::Float,
            ParameterValue::Int(_) => Self::Int,
            ParameterValue::Bool(_) => Self::Bool,
            ParameterValue::Vec2(_) => Self::Vec2,
            ParameterValue::Vec3(_) => Self::Vec3,
            ParameterValue::Vec4(_) => Self::Vec4,
            ParameterValue::Color(_) | ParameterValue::Color4(_) => Self::Color,
            ParameterValue::String(_) => Self::String,
            _ => Self::Unknown,
        }
    }

    /// Check whether it can be bound to the target type
    pub fn can_bind_to(&self, target: &crate::cunning_core::cda::PromotedParamType) -> bool {
        use crate::cunning_core::cda::PromotedParamType;
        match (self, target) {
            // Float can bind to Float, Angle, or a single channel of Vec
            (Self::Float, PromotedParamType::Float { .. }) => true,
            (Self::Float, PromotedParamType::Angle) => true,
            // Int can bind to Int or Dropdown
            (Self::Int, PromotedParamType::Int { .. }) => true,
            (Self::Int, PromotedParamType::Dropdown { .. }) => true,
            // Bool can bind to Bool or Toggle
            (Self::Bool, PromotedParamType::Bool) => true,
            (Self::Bool, PromotedParamType::Toggle) => true,
            // Vec3 can bind to Vec3 or Color
            (Self::Vec3, PromotedParamType::Vec3) => true,
            (Self::Vec3, PromotedParamType::Color { .. }) => true,
            (Self::Color, PromotedParamType::Color { .. }) => true,
            (Self::Color, PromotedParamType::Vec3) => true,
            // Vec4
            (Self::Vec4, PromotedParamType::Vec4) => true,
            (Self::Vec4, PromotedParamType::Color { has_alpha: true }) => true,
            // Vec2
            (Self::Vec2, PromotedParamType::Vec2) => true,
            // String
            (Self::String, PromotedParamType::String) => true,
            (Self::String, PromotedParamType::FilePath { .. }) => true,
            _ => false,
        }
    }

    /// Check whether it can bind to a single channel
    pub fn can_bind_to_channel(&self) -> bool {
        matches!(self, Self::Float | Self::Int)
    }
}

impl ParamDragPayload {
    pub fn new(node: NodeId, param: &str, value: &crate::nodes::parameter::ParameterValue) -> Self {
        Self {
            source_node: node,
            param_name: param.to_string(),
            channel_index: None,
            param_type: ParamTypeHint::from_value(value),
            display_label: param.to_string(),
        }
    }

    pub fn with_channel(mut self, ch: usize) -> Self {
        self.channel_index = Some(ch);
        self.display_label = format!("{}[{}]", self.param_name, ch);
        self
    }

    pub fn with_label(mut self, label: &str) -> Self {
        self.display_label = label.to_string();
        self
    }
}

/// Drag payload for promoted parameters (used for reordering)
#[derive(Clone, Debug)]
pub struct PromotedParamDragPayload {
    pub param_id: uuid::Uuid,
    pub param_name: String,
}
