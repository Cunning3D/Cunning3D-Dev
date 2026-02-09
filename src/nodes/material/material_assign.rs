use bevy::prelude::*;
use std::sync::Arc;

use crate::cunning_core::traits::node_interface::{NodeOp, NodeParameters};
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::libs::geometry::group::ElementGroupMask;
use crate::libs::geometry::ids::AttributeId;
use crate::mesh::{Attribute, Geometry};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;

#[derive(Default)]
pub struct MaterialAssignNode;

impl NodeParameters for MaterialAssignNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "group",
                "Group",
                "General",
                ParameterValue::String("".into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "prim_attr",
                "Primitive Attribute",
                "General",
                ParameterValue::String(attrs::SHOP_MATERIALPATH.into()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "int_key",
                "Int Key",
                "General",
                ParameterValue::Int(0),
                ParameterUIType::IntSlider {
                    min: -2147483648,
                    max: 2147483647,
                },
            ),
        ]
    }
}

impl NodeOp for MaterialAssignNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let default_geo = Arc::new(Geometry::new());
        let in_geo = mats.first().unwrap_or(&default_geo);
        let mat_geo = mats.get(1).unwrap_or(&default_geo);
        let mut out = in_geo.fork();

        let group_str = get_param_string(params, "group");
        let mut prim_attr = get_param_string(params, "prim_attr");
        if prim_attr.trim().is_empty() {
            prim_attr = attrs::SHOP_MATERIALPATH.into();
        }
        if !prim_attr.starts_with('@') {
            prim_attr = format!("@{}", prim_attr.trim());
        }
        let int_key = get_param_int(params, "int_key", 0);

        let mat_id = mat_geo
            .get_detail_attribute(attrs::MAT_ID)
            .and_then(|a| a.as_slice::<String>())
            .and_then(|v| v.get(0))
            .cloned();
        let Some(mat_id) = mat_id.filter(|s| !s.trim().is_empty()) else {
            return Arc::new(Geometry::new());
        };

        let prim_count = out.primitives().len();
        let selection = if group_str.trim().is_empty() {
            let mut m = ElementGroupMask::new(prim_count);
            m.invert();
            m
        } else {
            out.primitive_groups
                .get(&AttributeId::from(group_str.as_str()))
                .cloned()
                .unwrap_or_else(|| {
                    crate::nodes::group::utils::parse_pattern(group_str.as_str(), prim_count)
                })
        };
        if selection.is_empty() {
            return Arc::new(out);
        }

        // Write primitive key: if attribute already exists as i32, write int_key; else write string mat_id.
        if out
            .get_primitive_attribute(prim_attr.as_str())
            .and_then(|a| a.as_slice::<i32>())
            .is_some()
        {
            if out
                .get_primitive_attribute_mut(prim_attr.as_str())
                .and_then(|a| a.as_mut_slice::<i32>())
                .is_none()
            {
                out.insert_primitive_attribute(
                    prim_attr.clone(),
                    Attribute::new(vec![0i32; prim_count]),
                );
            }
            let Some(dst) = out
                .get_primitive_attribute_mut(prim_attr.as_str())
                .and_then(|a| a.as_mut_slice::<i32>())
            else {
                return Arc::new(Geometry::new());
            };
            for di in selection.iter_ones() {
                if let Some(v) = dst.get_mut(di) {
                    *v = int_key;
                }
            }

            // Copy material desc into matlib keyed by int (string form). Strict: missing -> empty output.
            let pfx = format!("__cunning.matlib.{}.", int_key);
            if copy_detail::<String>(
                mat_geo,
                &format!("{pfx}basecolor_tex"),
                attrs::MAT_BASECOLOR_TEX,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
            if copy_detail::<String>(
                mat_geo,
                &format!("{pfx}normal_tex"),
                attrs::MAT_NORMAL_TEX,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
            if copy_detail::<String>(
                mat_geo,
                &format!("{pfx}orm_tex"),
                attrs::MAT_ORM_TEX,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
            if copy_detail::<String>(
                mat_geo,
                &format!("{pfx}emissive_tex"),
                attrs::MAT_EMISSIVE_TEX,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
            if copy_detail::<Vec4>(
                mat_geo,
                &format!("{pfx}basecolor_tint"),
                attrs::MAT_BASECOLOR_TINT,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
            if copy_detail::<f32>(
                mat_geo,
                &format!("{pfx}roughness"),
                attrs::MAT_ROUGHNESS,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
            if copy_detail::<f32>(
                mat_geo,
                &format!("{pfx}metallic"),
                attrs::MAT_METALLIC,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
            if copy_detail::<Vec3>(
                mat_geo,
                &format!("{pfx}emissive"),
                attrs::MAT_EMISSIVE,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
        } else {
            if out
                .get_primitive_attribute(prim_attr.as_str())
                .and_then(|a| a.as_slice::<String>())
                .is_none()
            {
                out.insert_primitive_attribute(
                    prim_attr.clone(),
                    Attribute::new(vec![String::new(); prim_count]),
                );
            }
            let Some(dst) = out
                .get_primitive_attribute_mut(prim_attr.as_str())
                .and_then(|a| a.as_mut_slice::<String>())
            else {
                return Arc::new(Geometry::new());
            };
            for di in selection.iter_ones() {
                if let Some(s) = dst.get_mut(di) {
                    *s = mat_id.clone();
                }
            }

            // Copy material desc into a detail "matlib" namespace (per id). Strict: missing -> empty output.
            let pfx = format!("__cunning.matlib.{}.", mat_id);
            if copy_detail::<String>(
                mat_geo,
                &format!("{pfx}basecolor_tex"),
                attrs::MAT_BASECOLOR_TEX,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
            if copy_detail::<String>(
                mat_geo,
                &format!("{pfx}normal_tex"),
                attrs::MAT_NORMAL_TEX,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
            if copy_detail::<String>(
                mat_geo,
                &format!("{pfx}orm_tex"),
                attrs::MAT_ORM_TEX,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
            if copy_detail::<String>(
                mat_geo,
                &format!("{pfx}emissive_tex"),
                attrs::MAT_EMISSIVE_TEX,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
            if copy_detail::<Vec4>(
                mat_geo,
                &format!("{pfx}basecolor_tint"),
                attrs::MAT_BASECOLOR_TINT,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
            if copy_detail::<f32>(
                mat_geo,
                &format!("{pfx}roughness"),
                attrs::MAT_ROUGHNESS,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
            if copy_detail::<f32>(
                mat_geo,
                &format!("{pfx}metallic"),
                attrs::MAT_METALLIC,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
            if copy_detail::<Vec3>(
                mat_geo,
                &format!("{pfx}emissive"),
                attrs::MAT_EMISSIVE,
                &mut out,
            )
            .is_none()
            {
                return Arc::new(Geometry::new());
            }
        }

        // Record which primitive attribute drives material bucketing.
        out.insert_detail_attribute(attrs::MAT_BY, Attribute::new(vec![prim_attr.clone()]));

        Arc::new(out)
    }
}

fn copy_detail<T: Clone + Default + Send + Sync + 'static + std::fmt::Debug>(
    src: &Geometry,
    dst_name: &str,
    src_name: &str,
    dst: &mut Geometry,
) -> Option<()> {
    let v = src.get_detail_attribute(src_name)?.to_vec::<T>()?;
    dst.insert_detail_attribute(dst_name.to_string(), Attribute::new(v));
    Some(())
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
fn get_param_int(params: &[Parameter], name: &str, default: i32) -> i32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| {
            if let ParameterValue::Int(v) = &p.value {
                Some(*v)
            } else {
                None
            }
        })
        .unwrap_or(default)
}

register_node!("Material Assign", "Material", MaterialAssignNode; inputs: &["in:0", "in:1"], outputs: &["out:0"], style: crate::cunning_core::registries::node_registry::InputStyle::NamedPorts);
