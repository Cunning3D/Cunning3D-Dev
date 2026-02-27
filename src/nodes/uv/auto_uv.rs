use crate::libs::geometry::geo_ref::GeometryRef;
use crate::{
    cunning_core::traits::node_interface::{NodeOp, NodeParameters},
    libs::algorithms::algorithms_runtime::uv_auto::{
        auto_uv_smart_flatten, auto_uv_smart_project, AutoUvSmartFlattenOptions, AutoUvSmartProjectOptions,
    },
    mesh::{Attribute, Geometry},
    nodes::parameter::{Parameter, ParameterUIType, ParameterValue},
    register_node,
};
use std::sync::Arc;

#[derive(Default)]
pub struct AutoUvNode;

impl NodeParameters for AutoUvNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "backend",
                "Method",
                "Settings",
                ParameterValue::Int(2),
                ParameterUIType::Dropdown {
                    choices: vec![
                        ("Smart Flatten (Rust)".to_string(), 2),
                        ("Smart Planar (Rust)".to_string(), 0),
                        ("Disabled".to_string(), 1),
                    ],
                },
            ),
            Parameter::new(
                "max_angle",
                "Max Angle",
                "Settings",
                ParameterValue::Float(66.0),
                ParameterUIType::FloatSlider { min: 0.0, max: 180.0 },
            ),
            Parameter::new(
                "padding",
                "Padding",
                "Settings",
                ParameterValue::Float(0.02),
                ParameterUIType::FloatSlider { min: 0.0, max: 0.2 },
            ),
        ]
    }
}

impl NodeOp for AutoUvNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let default_input = Arc::new(Geometry::new());
        let input = mats.first().unwrap_or(&default_input);

        let console = crate::console::global_console();

        let backend = params
            .iter()
            .find(|p| p.name == "backend")
            .and_then(|p| match &p.value {
                ParameterValue::Int(i) => Some(*i),
                _ => None,
            })
            .unwrap_or(0);

        let max_angle = params
            .iter()
            .find(|p| p.name == "max_angle")
            .and_then(|p| match p.value {
                ParameterValue::Float(f) => Some(f),
                _ => None,
            })
            .unwrap_or(66.0)
            .clamp(0.0, 180.0);

        let padding = params
            .iter()
            .find(|p| p.name == "padding")
            .and_then(|p| match p.value {
                ParameterValue::Float(f) => Some(f),
                _ => None,
            })
            .unwrap_or(0.02)
            .clamp(0.0, 1.0);

        if backend == 1 {
            if let Some(c) = console {
                c.info("Auto UV: disabled");
            }
            return input.clone();
        }

        let res = match backend {
            0 => auto_uv_smart_project(
                input.as_ref(),
                AutoUvSmartProjectOptions {
                    max_angle_deg: max_angle,
                    padding,
                },
            ),
            2 => auto_uv_smart_flatten(
                input.as_ref(),
                AutoUvSmartFlattenOptions {
                    max_angle_deg: max_angle,
                    padding,
                    ..Default::default()
                },
            ),
            _ => {
                if let Some(c) = console {
                    c.info("Auto UV: disabled");
                }
                return input.clone();
            }
        };

        match res {
            Ok(out) => {
                if let Some(c) = console {
                    c.info(format!(
                        "Auto UV: {} (points={}, tris={}, charts={}, padding={:.4}, max_angle={:.1})",
                        if backend == 2 { "smart flatten" } else { "smart project" },
                        out.points, out.triangles, out.charts, padding, max_angle
                    ));
                }
                let mut geo = input.fork();
                geo.insert_vertex_attribute("@uv", Attribute::new(out.uvs));
                Arc::new(geo)
            }
            Err(err) => {
                if let Some(c) = console {
                    c.error(format!("Auto UV failed: {err}"));
                }
                input.clone()
            }
        }
    }
}

register_node!("Auto UV", "UV", AutoUvNode);
