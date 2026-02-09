use bevy::prelude::*;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::{Attribute, Geometry};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;

#[derive(Default)]
pub struct QuickMaterialNode;

impl NodeParameters for QuickMaterialNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "name",
                "Material Name",
                "General",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "basecolor_tex",
                "BaseColor Texture",
                "Textures",
                ParameterValue::String("".into()),
                ParameterUIType::FilePath {
                    filters: super::tex_filters(),
                },
            ),
            Parameter::new(
                "normal_tex",
                "Normal Texture",
                "Textures",
                ParameterValue::String("".into()),
                ParameterUIType::FilePath {
                    filters: super::tex_filters(),
                },
            ),
            Parameter::new(
                "tint",
                "BaseColor Tint",
                "Params",
                ParameterValue::Color4(Vec4::ONE),
                ParameterUIType::Color { show_alpha: true },
            ),
            Parameter::new(
                "roughness",
                "Roughness",
                "Params",
                ParameterValue::Float(0.6),
                ParameterUIType::FloatSlider { min: 0.0, max: 1.0 },
            ),
            Parameter::new(
                "metallic",
                "Metallic",
                "Params",
                ParameterValue::Float(0.0),
                ParameterUIType::FloatSlider { min: 0.0, max: 1.0 },
            ),
        ]
    }
}

impl NodeOp for QuickMaterialNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let default_geo = Arc::new(Geometry::new());
        let input = mats.first().unwrap_or(&default_geo);
        let mut geo = input.fork();

        let name = get_param_string(params, "name");
        let basecolor_tex = get_param_string(params, "basecolor_tex");
        let normal_tex = get_param_string(params, "normal_tex");
        let tint = get_param_color4(params, "tint", Vec4::ONE);
        let roughness = get_param_float(params, "roughness", 0.6);
        let metallic = get_param_float(params, "metallic", 0.0);

        // Non-strict: allow partial texture sets (e.g. only basecolor, only normal, or none).

        let mut h = std::collections::hash_map::DefaultHasher::new();
        "quick".hash(&mut h);
        basecolor_tex.hash(&mut h);
        normal_tex.hash(&mut h);
        for b in tint.to_array().map(f32::to_bits) {
            b.hash(&mut h);
        }
        (roughness.to_bits(), metallic.to_bits()).hash(&mut h);
        let auto_id = format!("mat_{:016x}", h.finish());
        let mat_id = if name.trim().is_empty() {
            auto_id
        } else {
            name.trim().to_string()
        };

        geo.insert_detail_attribute(attrs::MAT_KIND, Attribute::new(vec!["quick".to_string()]));
        geo.insert_detail_attribute(attrs::MAT_ID, Attribute::new(vec![mat_id]));
        geo.insert_detail_attribute(
            attrs::MAT_BASECOLOR_TEX,
            Attribute::new(vec![basecolor_tex]),
        );
        geo.insert_detail_attribute(attrs::MAT_NORMAL_TEX, Attribute::new(vec![normal_tex]));
        geo.insert_detail_attribute(attrs::MAT_BASECOLOR_TINT, Attribute::new(vec![tint]));
        geo.insert_detail_attribute(attrs::MAT_ROUGHNESS, Attribute::new(vec![roughness]));
        geo.insert_detail_attribute(attrs::MAT_METALLIC, Attribute::new(vec![metallic]));

        Arc::new(geo)
    }
}

#[inline]
fn get_param_string(params: &[Parameter], name: &str) -> String {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| {
            if let ParameterValue::String(v) = &p.value {
                Some(v.clone())
            } else {
                None
            }
        })
        .unwrap_or_default()
}
#[inline]
fn get_param_float(params: &[Parameter], name: &str, default: f32) -> f32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| {
            if let ParameterValue::Float(v) = &p.value {
                Some(*v)
            } else {
                None
            }
        })
        .unwrap_or(default)
}
#[inline]
fn get_param_color4(params: &[Parameter], name: &str, default: Vec4) -> Vec4 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::Color4(v) => Some(*v),
            ParameterValue::Vec4(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(default)
}

register_node!("Quick Material", "Material", QuickMaterialNode);
