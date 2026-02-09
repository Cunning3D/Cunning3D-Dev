//! CDA拖拽载荷定义（用于egui DragAndDrop）
use crate::nodes::structs::NodeId;

/// 内部节点参数拖拽载荷
#[derive(Clone, Debug)]
pub struct ParamDragPayload {
    pub source_node: NodeId,          // 来源节点
    pub param_name: String,           // 参数名
    pub channel_index: Option<usize>, // 通道索引（向量类型时）
    pub param_type: ParamTypeHint,    // 类型提示（用于匹配）
    pub display_label: String,        // 显示标签
}

/// 参数类型提示（用于拖拽匹配）
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
    /// 从ParameterValue推断类型
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

    /// 检查是否可以绑定到目标类型
    pub fn can_bind_to(&self, target: &crate::cunning_core::cda::PromotedParamType) -> bool {
        use crate::cunning_core::cda::PromotedParamType;
        match (self, target) {
            // Float可以绑定到Float、Angle、Vec的单个通道
            (Self::Float, PromotedParamType::Float { .. }) => true,
            (Self::Float, PromotedParamType::Angle) => true,
            // Int可以绑定到Int、Dropdown
            (Self::Int, PromotedParamType::Int { .. }) => true,
            (Self::Int, PromotedParamType::Dropdown { .. }) => true,
            // Bool可以绑定到Bool、Toggle
            (Self::Bool, PromotedParamType::Bool) => true,
            (Self::Bool, PromotedParamType::Toggle) => true,
            // Vec3可以绑定到Vec3、Color
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

    /// 检查是否可以绑定到单个通道
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

/// 暴露参数拖拽载荷（用于重新排序）
#[derive(Clone, Debug)]
pub struct PromotedParamDragPayload {
    pub param_id: uuid::Uuid,
    pub param_name: String,
}
