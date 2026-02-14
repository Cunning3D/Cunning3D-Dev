use crate::cunning_core::traits::node_interface::{
    GizmoContext, GizmoDrawBuffer, GizmoPart, GizmoState, NodeInteraction, NodeOp,
    NodeParameters, ServiceProvider,
};
use crate::cunning_core::ui::hud_standard;
use crate::gizmos::standard::StandardGizmo;
use crate::gizmos::{GizmoActionQueue, GizmoBinding, GizmoMovedEvent};
use crate::libs::geometry::attrs;
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::{Attribute, GeoPrimitive, Geometry, PolylinePrim};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::nodes::structs::NodeGraphResource;
use crate::register_node;
use bevy::prelude::Vec3;
use std::any::TypeId;
use std::sync::Arc;
use uuid::Uuid;

const PARAM_LENGTH: &str = "length";
const PARAM_SEGMENTS: &str = "segments";

#[derive(Default, Clone)]
pub struct PolylineNode;

impl NodeParameters for PolylineNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                PARAM_LENGTH,
                "Length",
                "Polyline",
                ParameterValue::Float(1.0),
                ParameterUIType::FloatSlider {
                    min: 0.0,
                    max: 1000.0,
                },
            ),
            Parameter::new(
                PARAM_SEGMENTS,
                "Segments",
                "Polyline",
                ParameterValue::Int(8),
                ParameterUIType::IntSlider { min: 1, max: 512 },
            ),
        ]
    }
}

impl NodeOp for PolylineNode {
    fn compute(&self, params: &[Parameter], _inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let length = get_float(params, PARAM_LENGTH, 1.0).max(0.0);
        let segments = get_int(params, PARAM_SEGMENTS, 8).max(1) as usize;

        let mut geo = Geometry::new();
        let mut positions = Vec::with_capacity(segments + 1);
        let mut vertices = Vec::with_capacity(segments + 1);

        for i in 0..=segments {
            let t = i as f32 / segments as f32;
            let p = Vec3::new(0.0, 0.0, length * t);
            let pid = geo.add_point();
            vertices.push(geo.add_vertex(pid));
            positions.push(p);
        }

        geo.insert_point_attribute(attrs::P, Attribute::new_auto(positions));
        geo.add_primitive(GeoPrimitive::Polyline(PolylinePrim {
            vertices,
            closed: false,
        }));
        Arc::new(geo)
    }
}

impl NodeInteraction for PolylineNode {
    fn has_hud(&self) -> bool {
        true
    }

    fn draw_hud(
        &self,
        ui: &mut bevy_egui::egui::Ui,
        services: &dyn ServiceProvider,
        node_id: Uuid,
    ) {
        let Some(graph_res) = get_service_ref::<NodeGraphResource>(services) else {
            return;
        };
        let Some(actions) =
            get_service_ref::<crate::tabs_system::hud_actions::HudActionQueue>(services)
        else {
            return;
        };
        let Some(node) = graph_res.0.nodes.get(&node_id) else {
            return;
        };

        let mut length = get_float(&node.parameters, PARAM_LENGTH, 1.0).max(0.0);
        let mut segments = get_int(&node.parameters, PARAM_SEGMENTS, 8).max(1);
        let mut changed_length = false;
        let mut changed_segments = false;

        ui.group(|ui| {
            changed_length |= hud_standard::draw_float_slider_with_input(
                ui,
                "Length",
                &mut length,
                0.0..=1000.0,
                0.1,
            );
            changed_segments |=
                hud_standard::draw_int_slider_with_input(ui, "Segments", &mut segments, 1..=512);
        });

        if changed_length {
            actions.push(crate::tabs_system::hud_actions::HudAction::SetNodeParamFloat {
                node_id,
                param_name: PARAM_LENGTH.to_string(),
                value: length.max(0.0),
            });
        }
        if changed_segments {
            actions.push(crate::tabs_system::hud_actions::HudAction::SetNodeParamInt {
                node_id,
                param_name: PARAM_SEGMENTS.to_string(),
                value: segments.max(1),
            });
        }

        if let Some(gizmo_state) = get_service_ref::<GizmoState>(services) {
            StandardGizmo::draw_status_hud(ui, gizmo_state);
        }
    }

    fn draw_gizmos(
        &self,
        buffer: &mut GizmoDrawBuffer,
        context: &GizmoContext,
        gizmo_state: &mut GizmoState,
        services: &dyn ServiceProvider,
        node_id: Uuid,
    ) {
        let Some(graph_res) = get_service_ref::<NodeGraphResource>(services) else {
            return;
        };
        let Some(actions) = get_service_ref::<GizmoActionQueue>(services) else {
            return;
        };
        let Some(node) = graph_res.0.nodes.get(&node_id) else {
            return;
        };

        let mut length = get_float(&node.parameters, PARAM_LENGTH, 1.0).max(0.0);
        if StandardGizmo::draw_linear_scalar_handle(
            buffer,
            context,
            gizmo_state,
            Vec3::ZERO,
            Vec3::Z,
            &mut length,
            node_id,
            GizmoPart::TranslateZ,
            0.0,
        ) {
            actions.push(GizmoMovedEvent {
                binding: GizmoBinding::ParamFloat {
                    node_id,
                    param_name: PARAM_LENGTH.to_string(),
                },
                new_position: Vec3::new(length, 0.0, 0.0),
            });
            gizmo_state.graph_modified = true;
        }
    }
}

#[inline]
fn get_float(params: &[Parameter], name: &str, default: f32) -> f32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::Float(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(default)
}

#[inline]
fn get_int(params: &[Parameter], name: &str, default: i32) -> i32 {
    params
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            ParameterValue::Int(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(default)
}

#[inline]
fn get_service_ref<T: 'static>(provider: &dyn ServiceProvider) -> Option<&T> {
    provider
        .get_service(TypeId::of::<T>())
        .and_then(|s| s.downcast_ref::<T>())
}

register_node!("Polyline", "Spline", PolylineNode, PolylineNode);
