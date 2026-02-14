use bevy::prelude::*;
use std::any::TypeId;
use std::sync::Arc;

use crate::cunning_core::traits::node_interface::{
    NodeInteraction, NodeOp, NodeParameters, ServiceProvider,
};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::{Attribute, BezierCurvePrim, GeoPrimitive, Geometry, VertexId};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::register_node;

use crate::libs::algorithms::algorithms_runtime::unity_spline::{
    calculate_knot_rotation, BezierKnot, MetaData, Spline, SplineContainer, TangentMode,
    CATMULL_ROM_TENSION,
};
use crate::libs::geometry::attrs;
use std::collections::HashMap;

#[derive(Default)]
pub struct UnitySplineNode;

fn default_container() -> SplineContainer {
    let mut c = SplineContainer::default();
    c.splines.push(Spline::default());
    c.splines[0].knots.clear();
    c.splines[0].meta.clear();
    c
}

#[inline]
fn sample_vec3_attr(attr: Option<&Attribute>, index: usize) -> Option<Vec3> {
    let a = attr?;
    if let Some(s) = a.as_slice::<Vec3>() {
        return s.get(index).copied();
    }
    if let Some(pb) = a.as_paged::<Vec3>() {
        return pb.get(index);
    }
    None
}

fn try_container_from_polyline_inputs(inputs: &[Arc<dyn GeometryRef>]) -> Option<SplineContainer> {
    for input in inputs {
        let geo = input.materialize();
        let p_attr = geo.get_point_attribute(attrs::P);
        if p_attr.is_none() {
            continue;
        }

        let mut container = SplineContainer::default();
        for prim in geo.primitives().iter() {
            let GeoPrimitive::Polyline(line) = prim else {
                continue;
            };
            if line.vertices.len() < 2 {
                continue;
            }

            let mut positions: Vec<Vec3> = Vec::with_capacity(line.vertices.len());
            for vid in &line.vertices {
                let Some(v) = geo.vertices().get((*vid).into()) else {
                    continue;
                };
                let Some(pidx) = geo.points().get_dense_index(v.point_id.into()) else {
                    continue;
                };
                let Some(pos) = sample_vec3_attr(p_attr, pidx) else {
                    continue;
                };
                positions.push(pos);
            }
            if positions.len() < 2 {
                continue;
            }

            let mut spline = Spline::default();
            spline.closed = line.closed;
            for i in 0..positions.len() {
                let p = positions[i];
                let prev = if i > 0 {
                    positions[i - 1]
                } else if spline.closed {
                    positions[positions.len() - 1]
                } else {
                    p
                };
                let next = if i + 1 < positions.len() {
                    positions[i + 1]
                } else if spline.closed {
                    positions[0]
                } else {
                    p
                };
                spline.knots.push(BezierKnot {
                    position: p,
                    tangent_in: Vec3::ZERO,
                    tangent_out: Vec3::ZERO,
                    rotation: calculate_knot_rotation(prev, p, next, Vec3::Y),
                });
                spline
                    .meta
                    .push(MetaData::new(TangentMode::Linear, CATMULL_ROM_TENSION));
            }
            container.splines.push(spline);
        }

        if !container.splines.is_empty() {
            return Some(container);
        }
    }
    None
}

impl NodeParameters for UnitySplineNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "spline",
                "Spline",
                "Spline",
                ParameterValue::UnitySpline(default_container()),
                ParameterUIType::UnitySpline,
            ),
            Parameter::new(
                "spline_blob_key",
                "Blob Key",
                "Internal",
                ParameterValue::String(String::new()),
                ParameterUIType::String,
            ),
            Parameter::new(
                "spline_source_basis",
                "Source Basis",
                "Internal",
                ParameterValue::Int(1),
                ParameterUIType::IntSlider { min: 0, max: 1 },
            ),
        ]
    }
}

impl NodeOp for UnitySplineNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let mut g = Geometry::new();
        let container = try_container_from_polyline_inputs(inputs).or_else(|| {
            params
                .iter()
                .find(|p| p.name == "spline")
                .and_then(|p| match &p.value {
                    ParameterValue::UnitySpline(c) => Some(c.clone()),
                    _ => None,
                })
        });
        let Some(container) = container else {
            return Arc::new(g);
        };
        if container.splines.is_empty() {
            return Arc::new(g);
        }

        // Deterministic link ids: stable by sorting link groups and entries (no implicit fallbacks).
        let mut links = container.links.all_links();
        links.retain(|l| l.len() >= 2);
        for l in links.iter_mut() {
            l.sort_by_key(|k| (k.spline, k.knot));
        }
        links.sort_by_key(|l| (l[0].spline, l[0].knot));
        let mut link_id_by_key: HashMap<(i32, i32), i32> = HashMap::new();
        for (li, l) in links.iter().enumerate() {
            for k in l {
                link_id_by_key.insert((k.spline, k.knot), li as i32);
            }
        }
        let has_links = !link_id_by_key.is_empty();

        let mut all_p: Vec<Vec3> = Vec::new();
        let mut all_tin: Vec<Vec3> = Vec::new();
        let mut all_tout: Vec<Vec3> = Vec::new();
        let mut all_rot: Vec<Quat> = Vec::new();
        let mut all_mode: Vec<i32> = Vec::new();
        let mut all_tension: Vec<f32> = Vec::new();
        let mut all_link: Vec<i32> = Vec::new();

        for (si, spline0) in container.splines.iter().enumerate() {
            let mut spline = spline0.clone();
            if spline.knots.len() < 2 {
                continue;
            }
            while spline.meta.len() < spline.knots.len() {
                spline
                    .meta
                    .push(MetaData::new(TangentMode::Broken, CATMULL_ROM_TENSION));
            }

            let mut vids: Vec<VertexId> = Vec::with_capacity(spline.knots.len());
            for (ki, knot0) in spline.knots.iter().enumerate() {
                let knot = knot0.transform(container.local_to_world);
                let pid = g.add_point();
                vids.push(g.add_vertex(pid));
                all_p.push(knot.position);
                all_tin.push(knot.tangent_in);
                all_tout.push(knot.tangent_out);
                all_rot.push(knot.rotation);
                all_mode.push(spline.meta[ki].mode as i32);
                all_tension.push(spline.meta[ki].tension);
                if has_links {
                    all_link.push(
                        link_id_by_key
                            .get(&(si as i32, ki as i32))
                            .copied()
                            .unwrap_or(-1),
                    );
                }
            }
            let _ = g.add_primitive(GeoPrimitive::BezierCurve(BezierCurvePrim {
                vertices: vids,
                closed: spline.closed,
            }));
        }

        if all_p.is_empty() {
            return Arc::new(g);
        }
        g.insert_point_attribute(attrs::P, Attribute::new_auto(all_p));
        g.insert_point_attribute(attrs::KNOT_TIN, Attribute::new_auto(all_tin));
        g.insert_point_attribute(attrs::KNOT_TOUT, Attribute::new_auto(all_tout));
        g.insert_point_attribute(attrs::KNOT_ROT, Attribute::new_auto(all_rot));
        g.insert_point_attribute(attrs::KNOT_MODE, Attribute::new_auto(all_mode));
        g.insert_point_attribute(attrs::KNOT_TENSION, Attribute::new_auto(all_tension));
        if has_links {
            g.insert_point_attribute(attrs::KNOT_LINK_ID, Attribute::new_auto(all_link));
        }
        Arc::new(g)
    }
}

impl NodeInteraction for UnitySplineNode {
    fn has_hud(&self) -> bool {
        true
    }
    fn draw_hud(
        &self,
        ui: &mut bevy_egui::egui::Ui,
        services: &dyn ServiceProvider,
        node_id: uuid::Uuid,
    ) {
        let actions = services
            .get_service(TypeId::of::<crate::tabs_system::hud_actions::HudActionQueue>())
            .and_then(|a| a.downcast_ref::<crate::tabs_system::hud_actions::HudActionQueue>());
        crate::nodes::spline::hud::draw_spline_hud(ui, services, actions, node_id);
    }

    fn has_coverlay(&self) -> bool {
        true
    }
    fn draw_coverlay(
        &self,
        ui: &mut bevy_egui::egui::Ui,
        services: &dyn ServiceProvider,
        node_id: uuid::Uuid,
    ) {
        // MVP: reuse the existing spline HUD UI as coverlay content.
        // Later: extend to a full tool panel (palette, modes, advanced widgets).
        self.draw_hud(ui, services, node_id);
    }
}

register_node!("Spline", "Spline", UnitySplineNode, UnitySplineNode);
