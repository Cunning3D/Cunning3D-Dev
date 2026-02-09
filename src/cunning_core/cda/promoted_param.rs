//! 参数提升定义：将CDA内部节点参数暴露到资产顶层，支持通道级绑定
use crate::nodes::structs::NodeId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 提升的参数定义
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct PromotedParam {
    pub id: Uuid,
    pub name: String,  // 内部名（唯一标识）
    pub label: String, // UI显示标签
    pub group: String, // 参数分组
    pub order: i32,    // 排序权重
    pub param_type: PromotedParamType,
    pub ui_config: ParamUIConfig,
    pub channels: Vec<ParamChannel>, // 通道（Vec3有3个，Float有1个）
}

impl PromotedParam {
    pub fn new_float(name: &str, default: f32) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            label: name.to_string(),
            group: "Main".to_string(),
            order: 0,
            param_type: PromotedParamType::Float {
                min: 0.0,
                max: 10.0,
                logarithmic: false,
            },
            ui_config: ParamUIConfig::default(),
            channels: vec![ParamChannel::new("", default as f64)],
        }
    }

    pub fn new_int(name: &str, default: i32) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            label: name.to_string(),
            group: "Main".to_string(),
            order: 0,
            param_type: PromotedParamType::Int { min: 0, max: 100 },
            ui_config: ParamUIConfig::default(),
            channels: vec![ParamChannel::new("", default as f64)],
        }
    }

    pub fn new_bool(name: &str, default: bool) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            label: name.to_string(),
            group: "Main".to_string(),
            order: 0,
            param_type: PromotedParamType::Toggle,
            ui_config: ParamUIConfig::default(),
            channels: vec![ParamChannel::new("", if default { 1.0 } else { 0.0 })],
        }
    }

    pub fn new_vec3(name: &str, default: [f32; 3]) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            label: name.to_string(),
            group: "Main".to_string(),
            order: 0,
            param_type: PromotedParamType::Vec3,
            ui_config: ParamUIConfig::default(),
            channels: vec![
                ParamChannel::new("x", default[0] as f64),
                ParamChannel::new("y", default[1] as f64),
                ParamChannel::new("z", default[2] as f64),
            ],
        }
    }

    pub fn new_color(name: &str, default: [f32; 3]) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            label: name.to_string(),
            group: "Main".to_string(),
            order: 0,
            param_type: PromotedParamType::Color { has_alpha: false },
            ui_config: ParamUIConfig::default(),
            channels: vec![
                ParamChannel::new("r", default[0] as f64),
                ParamChannel::new("g", default[1] as f64),
                ParamChannel::new("b", default[2] as f64),
            ],
        }
    }

    pub fn new_dropdown(name: &str, items: Vec<(&str, i32)>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            label: name.to_string(),
            group: "Main".to_string(),
            order: 0,
            param_type: PromotedParamType::Dropdown {
                items: items
                    .into_iter()
                    .map(|(l, v)| DropdownItem {
                        label: l.to_string(),
                        value: v,
                    })
                    .collect(),
            },
            ui_config: ParamUIConfig::default(),
            channels: vec![ParamChannel::new("", 0.0)],
        }
    }

    pub fn with_label(mut self, label: &str) -> Self {
        self.label = label.to_string();
        self
    }
    pub fn with_group(mut self, group: &str) -> Self {
        self.group = group.to_string();
        self
    }
    pub fn with_order(mut self, order: i32) -> Self {
        self.order = order;
        self
    }
    pub fn with_range(mut self, min: f32, max: f32) -> Self {
        if let PromotedParamType::Float { logarithmic, .. } = &self.param_type {
            self.param_type = PromotedParamType::Float {
                min,
                max,
                logarithmic: *logarithmic,
            };
        }
        self
    }

    /// 获取通道数量
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// 添加绑定到指定通道
    pub fn add_binding(&mut self, channel_idx: usize, binding: ParamBinding) {
        if let Some(ch) = self.channels.get_mut(channel_idx) {
            ch.bindings.push(binding);
        }
    }

    /// 移除绑定
    pub fn remove_binding(&mut self, channel_idx: usize, target_node: NodeId, target_param: &str) {
        if let Some(ch) = self.channels.get_mut(channel_idx) {
            ch.bindings
                .retain(|b| !(b.target_node == target_node && b.target_param == target_param));
        }
    }

    /// 获取所有绑定总数
    pub fn total_bindings(&self) -> usize {
        self.channels.iter().map(|c| c.bindings.len()).sum()
    }
}

/// 参数通道（Vec3有xyz三个通道，Float只有一个）
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ParamChannel {
    pub name: String, // "x", "y", "z", "r", "g", "b" 或 "" (单通道)
    pub default_value: f64,
    pub bindings: Vec<ParamBinding>,
}

impl ParamChannel {
    pub fn new(name: &str, default: f64) -> Self {
        Self {
            name: name.to_string(),
            default_value: default,
            bindings: Vec::new(),
        }
    }
}

/// 参数绑定：指向内部节点的某个参数的某个通道
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ParamBinding {
    pub target_node: NodeId,
    pub target_param: String,
    pub target_channel: Option<usize>, // 目标参数的通道索引（向量类型时）
}

impl ParamBinding {
    pub fn new(node: NodeId, param: &str) -> Self {
        Self {
            target_node: node,
            target_param: param.to_string(),
            target_channel: None,
        }
    }
    pub fn with_channel(mut self, ch: usize) -> Self {
        self.target_channel = Some(ch);
        self
    }
}

/// 参数类型定义
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum PromotedParamType {
    Float {
        min: f32,
        max: f32,
        logarithmic: bool,
    },
    Int {
        min: i32,
        max: i32,
    },
    Bool,
    Toggle,
    Button,
    Vec2,
    Vec3,
    Vec4,
    Color {
        has_alpha: bool,
    },
    String,
    Angle,
    Dropdown {
        items: Vec<DropdownItem>,
    },
    Ramp,
    FilePath {
        filters: Vec<String>,
    },
}

impl PromotedParamType {
    /// 获取类型名称（用于UI显示）
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Float { .. } => "Float",
            Self::Int { .. } => "Int",
            Self::Bool => "Bool",
            Self::Toggle => "Toggle",
            Self::Button => "Button",
            Self::Vec2 => "Vec2",
            Self::Vec3 => "Vec3",
            Self::Vec4 => "Vec4",
            Self::Color { .. } => "Color",
            Self::String => "String",
            Self::Angle => "Angle",
            Self::Dropdown { .. } => "Dropdown",
            Self::Ramp => "Ramp",
            Self::FilePath { .. } => "File Path",
        }
    }

    /// 获取通道数
    pub fn channel_count(&self) -> usize {
        match self {
            Self::Vec2 => 2,
            Self::Vec3 | Self::Color { has_alpha: false } => 3,
            Self::Vec4 | Self::Color { has_alpha: true } => 4,
            _ => 1,
        }
    }

    /// 获取通道名称
    pub fn channel_names(&self) -> Vec<&'static str> {
        match self {
            Self::Vec2 => vec!["x", "y"],
            Self::Vec3 => vec!["x", "y", "z"],
            Self::Vec4 => vec!["x", "y", "z", "w"],
            Self::Color { has_alpha: false } => vec!["r", "g", "b"],
            Self::Color { has_alpha: true } => vec!["r", "g", "b", "a"],
            _ => vec![""],
        }
    }
}

/// 下拉菜单项
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct DropdownItem {
    pub value: i32,
    pub label: String,
}

/// 参数UI配置
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ParamUIConfig {
    pub visible: bool,
    pub enabled: bool,
    pub lock_range: bool,
    pub tooltip: Option<String>,
    pub condition: Option<String>,
}

impl Default for ParamUIConfig {
    fn default() -> Self {
        Self {
            visible: true,
            enabled: true,
            lock_range: false,
            tooltip: None,
            condition: None,
        }
    }
}
