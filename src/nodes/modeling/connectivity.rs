use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::algorithms::algorithms_runtime::connectivity as conn_rt;
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::group::ElementGroupMask;
use crate::libs::geometry::ids::AttributeId;
use crate::mesh::{Attribute, Geometry};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;
use bevy::prelude::*;
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Default)]
pub struct ConnectivityNode;

#[derive(Clone, Copy)]
enum ConnType {
    Point,
    Primitive,
}

#[derive(Clone, Copy)]
enum AttrType {
    Integer,
    String,
}

#[derive(Default)]
struct Dsu {
    parent: Vec<usize>,
    rank: Vec<u8>,
}
impl Dsu {
    #[inline]
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }
    #[inline]
    fn find(&mut self, x: usize) -> usize {
        let p = self.parent[x];
        if p == x {
            x
        } else {
            let r = self.find(p);
            self.parent[x] = r;
            r
        }
    }
    #[inline]
    fn union(&mut self, a: usize, b: usize) {
        let mut ra = self.find(a);
        let mut rb = self.find(b);
        if ra == rb {
            return;
        }
        let (rka, rkb) = (self.rank[ra], self.rank[rb]);
        if rka < rkb {
            std::mem::swap(&mut ra, &mut rb);
        }
        self.parent[rb] = ra;
        if rka == rkb {
            self.rank[ra] = rka.saturating_add(1);
        }
    }
}

impl NodeParameters for ConnectivityNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "connectivity_type",
                "Connectivity Type",
                "General",
                ParameterValue::Int(1),
                ParameterUIType::Dropdown {
                    choices: vec![("Point".into(), 0), ("Primitive".into(), 1)],
                },
            ),
            Parameter::new(
                "include_group",
                "Primitive Include Group",
                "General",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "attribute",
                "Attribute",
                "General",
                ParameterValue::String("class".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "attribute_type",
                "Attribute Type",
                "General",
                ParameterValue::Int(0),
                ParameterUIType::Dropdown {
                    choices: vec![("Integer".into(), 0), ("String".into(), 1)],
                },
            ),
            Parameter::new(
                "local_variable",
                "Local Variable",
                "General",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "seam_group",
                "Seam Group",
                "General",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "use_uv",
                "Use UV Connectivity",
                "UV",
                ParameterValue::Bool(false),
                ParameterUIType::Toggle,
            ),
            Parameter::new(
                "uv_attribute",
                "UV Attribute",
                "UV",
                ParameterValue::String(attrs::UV.into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "uv_tol",
                "UV Tolerance",
                "UV",
                ParameterValue::Float(1e-4),
                ParameterUIType::FloatSlider {
                    min: 0.0,
                    max: 1e-2,
                },
            ),
        ]
    }
}

impl NodeOp for ConnectivityNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let default_geo = Arc::new(Geometry::new());
        let input_geo = mats.first().unwrap_or(&default_geo);
        if input_geo.primitives().is_empty() {
            return input_geo.clone();
        }

        let ct = match get_param_int(params, "connectivity_type", 1) {
            0 => ConnType::Point,
            _ => ConnType::Primitive,
        };
        let include_group = get_param_string(params, "include_group", "");
        let out_attr = get_param_string(params, "attribute", "class");
        let at = match get_param_int(params, "attribute_type", 0) {
            1 => AttrType::String,
            _ => AttrType::Integer,
        };
        let seam_group = get_param_string(params, "seam_group", "");
        let mut use_uv = get_param_bool(params, "use_uv", false);
        let uv_name = get_param_string(params, "uv_attribute", attrs::UV);
        let uv_tol = get_param_float(params, "uv_tol", 1e-4).max(0.0f32);

        let prim_count = input_geo.primitives().len();
        let selection = if include_group.is_empty() {
            let mut m = ElementGroupMask::new(prim_count);
            m.invert();
            m
        } else {
            input_geo
                .primitive_groups
                .get(&AttributeId::from(include_group.as_str()))
                .cloned()
                .unwrap_or_else(|| {
                    crate::nodes::group::utils::parse_pattern(&include_group, prim_count)
                })
        };

        let seam_edges = conn_rt::build_seam_edge_set(input_geo, &seam_group);

        let uv_attr = if use_uv {
            match input_geo.get_vertex_attribute(uv_name.as_str()) {
                Some(a) => {
                    let ok = a.as_slice::<Vec2>().is_some() || a.as_paged::<Vec2>().is_some();
                    if !ok {
                        warn!("Connectivity: UV attribute '{}' has unsupported storage/type; UV Connectivity disabled.", uv_name);
                        use_uv = false;
                        None
                    } else {
                        Some(a)
                    }
                }
                None => {
                    warn!(
                        "Connectivity: UV attribute '{}' missing; UV Connectivity disabled.",
                        uv_name
                    );
                    use_uv = false;
                    None
                }
            }
        } else {
            None
        };

        let mut out = input_geo.fork();
        match ct {
            ConnType::Primitive => {
                let ids = conn_rt::connectivity_primitives(
                    input_geo,
                    &selection,
                    &seam_edges,
                    uv_attr,
                    uv_tol,
                );
                match at {
                    AttrType::Integer => out.insert_primitive_attribute(
                        out_attr,
                        Attribute::new_auto(ids.into_iter().map(|v| v as i32).collect()),
                    ),
                    AttrType::String => out.insert_primitive_attribute(
                        out_attr,
                        Attribute::new_auto(
                            ids.into_iter()
                                .map(|v| if v < 0 { String::new() } else { v.to_string() })
                                .collect(),
                        ),
                    ),
                }
            }
            ConnType::Point => {
                let ids = conn_rt::connectivity_points(
                    input_geo,
                    &selection,
                    &seam_edges,
                    uv_attr,
                    uv_tol,
                );
                match at {
                    AttrType::Integer => out.insert_point_attribute(
                        out_attr,
                        Attribute::new_auto(ids.into_iter().map(|v| v as i32).collect()),
                    ),
                    AttrType::String => out.insert_point_attribute(
                        out_attr,
                        Attribute::new_auto(
                            ids.into_iter()
                                .map(|v| if v < 0 { String::new() } else { v.to_string() })
                                .collect(),
                        ),
                    ),
                }
            }
        }
        Arc::new(out)
    }
}

#[inline]
fn get_param_string(params: &[Parameter], name: &str, default: &str) -> String {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| {
            if let ParameterValue::String(s) = &p.value {
                Some(s.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| default.to_string())
}
#[inline]
fn get_param_int(params: &[Parameter], name: &str, default: i32) -> i32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| {
            if let ParameterValue::Int(v) = p.value {
                Some(v)
            } else {
                None
            }
        })
        .unwrap_or(default)
}
#[inline]
fn get_param_float(params: &[Parameter], name: &str, default: f32) -> f32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| {
            if let ParameterValue::Float(v) = p.value {
                Some(v)
            } else {
                None
            }
        })
        .unwrap_or(default)
}
#[inline]
fn get_param_bool(params: &[Parameter], name: &str, default: bool) -> bool {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| {
            if let ParameterValue::Bool(v) = p.value {
                Some(v)
            } else {
                None
            }
        })
        .unwrap_or(default)
}

register_node!("Connectivity", "Modeling", ConnectivityNode);
