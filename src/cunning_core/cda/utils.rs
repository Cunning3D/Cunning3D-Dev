use crate::cunning_core::cda::{ParamChannel, PromotedParamType};
use crate::nodes::parameter::{ParameterUIType, ParameterValue};

#[inline]
pub(crate) fn value_to_channels(v: &ParameterValue) -> Vec<f64> {
    match v {
        ParameterValue::Float(f) => vec![*f as f64],
        ParameterValue::Int(i) => vec![*i as f64],
        ParameterValue::Bool(b) => vec![if *b { 1.0 } else { 0.0 }],
        ParameterValue::Vec2(v) => vec![v.x as f64, v.y as f64],
        ParameterValue::Vec3(v) | ParameterValue::Color(v) => {
            vec![v.x as f64, v.y as f64, v.z as f64]
        }
        ParameterValue::Vec4(v) | ParameterValue::Color4(v) => {
            vec![v.x as f64, v.y as f64, v.z as f64, v.w as f64]
        }
        _ => Vec::new(),
    }
}

pub(crate) fn apply_channel_value(value: &mut ParameterValue, channel: Option<usize>, val: f64) {
    let v = val as f32;
    match value {
        ParameterValue::Float(f) => *f = v,
        ParameterValue::Int(i) => *i = val as i32,
        ParameterValue::Bool(b) => *b = val != 0.0,
        ParameterValue::Vec2(p) => match channel {
            Some(0) => p.x = v,
            Some(1) => p.y = v,
            _ => {
                p.x = v;
                p.y = v;
            }
        },
        ParameterValue::Vec3(p) | ParameterValue::Color(p) => match channel {
            Some(0) => p.x = v,
            Some(1) => p.y = v,
            Some(2) => p.z = v,
            _ => {
                p.x = v;
                p.y = v;
                p.z = v;
            }
        },
        ParameterValue::Vec4(p) | ParameterValue::Color4(p) => match channel {
            Some(0) => p.x = v,
            Some(1) => p.y = v,
            Some(2) => p.z = v,
            Some(3) => p.w = v,
            _ => {
                p.x = v;
                p.y = v;
                p.z = v;
                p.w = v;
            }
        },
        _ => {}
    }
}

pub(crate) fn promoted_type_to_ui(pt: &PromotedParamType) -> ParameterUIType {
    match pt {
        PromotedParamType::Float { min, max, .. } => ParameterUIType::FloatSlider {
            min: *min,
            max: *max,
        },
        PromotedParamType::Int { min, max } => ParameterUIType::IntSlider {
            min: *min,
            max: *max,
        },
        PromotedParamType::Bool | PromotedParamType::Toggle => ParameterUIType::Toggle,
        PromotedParamType::Button => ParameterUIType::Button,
        PromotedParamType::Vec2 => ParameterUIType::Vec2Drag,
        PromotedParamType::Vec3 => ParameterUIType::Vec3Drag,
        PromotedParamType::Vec4 => ParameterUIType::Vec4Drag,
        PromotedParamType::Color { has_alpha } => ParameterUIType::Color {
            show_alpha: *has_alpha,
        },
        PromotedParamType::String => ParameterUIType::String,
        PromotedParamType::Dropdown { items } => ParameterUIType::Dropdown {
            choices: items.iter().map(|i| (i.label.clone(), i.value)).collect(),
        },
        PromotedParamType::Angle => ParameterUIType::FloatSlider {
            min: -180.0,
            max: 180.0,
        },
        PromotedParamType::FilePath { filters } => ParameterUIType::FilePath {
            filters: filters.clone(),
        },
        _ => ParameterUIType::String,
    }
}

pub(crate) fn promoted_channels_to_value(
    pt: &PromotedParamType,
    channels: &[ParamChannel],
) -> ParameterValue {
    match pt {
        PromotedParamType::Float { .. } | PromotedParamType::Angle => ParameterValue::Float(
            channels
                .first()
                .map(|c| c.default_value as f32)
                .unwrap_or(0.0),
        ),
        PromotedParamType::Int { .. } | PromotedParamType::Dropdown { .. } => ParameterValue::Int(
            channels
                .first()
                .map(|c| c.default_value as i32)
                .unwrap_or(0),
        ),
        PromotedParamType::Bool | PromotedParamType::Toggle => ParameterValue::Bool(
            channels
                .first()
                .map(|c| c.default_value != 0.0)
                .unwrap_or(false),
        ),
        PromotedParamType::Button => ParameterValue::Int(0),
        PromotedParamType::Vec2 => ParameterValue::Vec2(bevy::prelude::Vec2::new(
            channels
                .get(0)
                .map(|c| c.default_value as f32)
                .unwrap_or(0.0),
            channels
                .get(1)
                .map(|c| c.default_value as f32)
                .unwrap_or(0.0),
        )),
        PromotedParamType::Vec3 => ParameterValue::Vec3(bevy::prelude::Vec3::new(
            channels
                .get(0)
                .map(|c| c.default_value as f32)
                .unwrap_or(0.0),
            channels
                .get(1)
                .map(|c| c.default_value as f32)
                .unwrap_or(0.0),
            channels
                .get(2)
                .map(|c| c.default_value as f32)
                .unwrap_or(0.0),
        )),
        PromotedParamType::Vec4 => ParameterValue::Vec4(bevy::prelude::Vec4::new(
            channels
                .get(0)
                .map(|c| c.default_value as f32)
                .unwrap_or(0.0),
            channels
                .get(1)
                .map(|c| c.default_value as f32)
                .unwrap_or(0.0),
            channels
                .get(2)
                .map(|c| c.default_value as f32)
                .unwrap_or(0.0),
            channels
                .get(3)
                .map(|c| c.default_value as f32)
                .unwrap_or(0.0),
        )),
        PromotedParamType::Color { has_alpha: false } => {
            ParameterValue::Color(bevy::prelude::Vec3::new(
                channels
                    .get(0)
                    .map(|c| c.default_value as f32)
                    .unwrap_or(0.0),
                channels
                    .get(1)
                    .map(|c| c.default_value as f32)
                    .unwrap_or(0.0),
                channels
                    .get(2)
                    .map(|c| c.default_value as f32)
                    .unwrap_or(0.0),
            ))
        }
        PromotedParamType::Color { has_alpha: true } => {
            ParameterValue::Color4(bevy::prelude::Vec4::new(
                channels
                    .get(0)
                    .map(|c| c.default_value as f32)
                    .unwrap_or(0.0),
                channels
                    .get(1)
                    .map(|c| c.default_value as f32)
                    .unwrap_or(0.0),
                channels
                    .get(2)
                    .map(|c| c.default_value as f32)
                    .unwrap_or(0.0),
                channels
                    .get(3)
                    .map(|c| c.default_value as f32)
                    .unwrap_or(1.0),
            ))
        }
        PromotedParamType::String | PromotedParamType::FilePath { .. } => {
            ParameterValue::String(String::new())
        }
        _ => ParameterValue::Float(0.0),
    }
}
