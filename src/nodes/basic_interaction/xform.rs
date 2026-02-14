//! `transform` node logic.

use crate::{
    cunning_core::traits::node_interface::{
        GizmoContext, GizmoDrawBuffer, GizmoState, NodeInteraction, NodeOp, NodeParameters,
        ServiceProvider, XformGizmoMode,
    },
    gizmos::{GizmoActionQueue, GizmoBinding, GizmoMovedEvent},
    gizmos::standard::StandardGizmo,
    libs::algorithms::transform,
    libs::geometry::geo_ref::GeometryRef,
    mesh::Geometry,
    nodes::structs::{NodeGraphResource, NodeId},
    nodes::{
        parameter::{Parameter, ParameterUIType, ParameterValue},
        InputStyle, NodeStyle,
    },
    register_node,
};
use bevy::prelude::{EulerRot, Quat, Vec3};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Default)]
pub struct TransformNode;

impl NodeParameters for TransformNode {
    fn define_parameters() -> Vec<Parameter> {
        vec![
            Parameter::new(
                "translate",
                "Translate",
                "Transform",
                ParameterValue::Vec3(Vec3::ZERO),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "rotate",
                "Rotate",
                "Transform",
                ParameterValue::Vec3(Vec3::ZERO),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "scale",
                "Scale",
                "Transform",
                ParameterValue::Vec3(Vec3::ONE),
                ParameterUIType::Vec3Drag,
            ),
            Parameter::new(
                "uniform_scale",
                "Uniform Scale",
                "Transform",
                ParameterValue::Float(1.0),
                ParameterUIType::FloatSlider {
                    min: 0.01,
                    max: 10.0,
                },
            ),
        ]
    }
}

fn get_service_ref<T: 'static>(provider: &dyn ServiceProvider) -> Option<&T> {
    provider
        .get_service(TypeId::of::<T>())
        .and_then(|s| s.downcast_ref::<T>())
}

impl NodeOp for TransformNode {
    fn compute(&self, params: &[Parameter], inputs: &[Arc<dyn GeometryRef>]) -> Arc<Geometry> {
        let param_map: HashMap<String, ParameterValue> = params
            .iter()
            .map(|p| (p.name.clone(), p.value.clone()))
            .collect();

        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let default_input = Arc::new(Geometry::new());
        let input = mats.first().unwrap_or(&default_input);

        match apply_transform(input, &param_map) {
            Ok(geo) => Arc::new(geo),
            Err(_) => input.clone(),
        }
    }
}

impl NodeInteraction for TransformNode {
    fn has_hud(&self) -> bool {
        true
    }
    fn draw_hud(
        &self,
        ui: &mut bevy_egui::egui::Ui,
        services: &dyn ServiceProvider,
        _node_id: Uuid,
    ) {
        if let Some(gizmo_state) = get_service_ref::<GizmoState>(services) {
            ui.label(format!(
                "Xform Mode: {:?}   (Q Aggregate | W Move | E Scale | R All)",
                gizmo_state.xform_mode
            ));
            StandardGizmo::draw_status_hud(ui, gizmo_state);
        }
    }

    fn has_coverlay(&self) -> bool {
        true
    }
    fn draw_coverlay(
        &self,
        ui: &mut bevy_egui::egui::Ui,
        services: &dyn ServiceProvider,
        node_id: Uuid,
    ) {
        // MVP: show the same status HUD in coverlay to validate multi-coverlay stacking.
        self.draw_hud(ui, services, node_id);
    }

    fn draw_gizmos(
        &self,
        buffer: &mut GizmoDrawBuffer,
        context: &GizmoContext,
        gizmo_state: &mut GizmoState,
        services: &dyn ServiceProvider,
        node_id: Uuid,
    ) {
        let (Some(node_graph_res), Some(actions)) = (
            get_service_ref::<NodeGraphResource>(services),
            get_service_ref::<GizmoActionQueue>(services),
        ) else {
            return;
        };

        let node_graph = &node_graph_res.0;
        let Some(node) = node_graph.nodes.get(&node_id) else { return; };

        // Calculate input centroid for auto-centering.
        let centroid = get_input_centroid(node_graph, node_id).unwrap_or(Vec3::ZERO);

        // Snapshot current params (immutable graph access).
        let mut translate = Vec3::ZERO;
        let mut scale = Vec3::ONE;
        let mut rotate_deg = Vec3::ZERO;
        for p in &node.parameters {
            match (&*p.name, &p.value) {
                ("translate", ParameterValue::Vec3(v)) => translate = *v,
                ("scale", ParameterValue::Vec3(v)) => scale = *v,
                ("rotate", ParameterValue::Vec3(v)) => rotate_deg = *v,
                _ => {}
            }
        }

        let current_rot = Quat::from_euler(
            EulerRot::YXZ,
            rotate_deg.y.to_radians(),
            rotate_deg.x.to_radians(),
            rotate_deg.z.to_radians(),
        );

        let mut changed_any = false;

        let mode = gizmo_state.xform_mode;
        let show_translate = matches!(
            mode,
            XformGizmoMode::Aggregate | XformGizmoMode::Move | XformGizmoMode::All
        );
        let show_scale = matches!(
            mode,
            XformGizmoMode::Aggregate | XformGizmoMode::Scale | XformGizmoMode::All
        );
        let show_rotate = matches!(
            mode,
            XformGizmoMode::Aggregate | XformGizmoMode::All
        );

        // 1. Translate
        let mut gizmo_pos = centroid + translate;
        if show_translate
            && StandardGizmo::draw_translate(
                buffer,
                context,
                gizmo_state,
                &mut gizmo_pos,
                current_rot,
                node_id,
            )
        {
            translate = gizmo_pos - centroid;
            actions.push(GizmoMovedEvent {
                binding: GizmoBinding::ParamVec3 {
                    node_id,
                    param_name: "translate".to_string(),
                },
                new_position: translate,
            });
            changed_any = true;
        }

        // 2. Scale
        let current_pos = centroid + translate;
        let mut new_scale = scale;
        if show_scale
            && StandardGizmo::draw_scale(
                buffer,
                context,
                gizmo_state,
                current_pos,
                &mut new_scale,
                current_rot,
                node_id,
            )
        {
            scale = new_scale;
            actions.push(GizmoMovedEvent {
                binding: GizmoBinding::ParamVec3 {
                    node_id,
                    param_name: "scale".to_string(),
                },
                new_position: scale,
            });
            changed_any = true;
        }

        // 3. Rotate
        let mut new_rot = rotate_deg;
        if show_rotate
            && StandardGizmo::draw_rotate(
                buffer,
                context,
                gizmo_state,
                current_pos,
                &mut new_rot,
                node_id,
            )
        {
            rotate_deg = new_rot;
            actions.push(GizmoMovedEvent {
                binding: GizmoBinding::ParamVec3 {
                    node_id,
                    param_name: "rotate".to_string(),
                },
                new_position: rotate_deg,
            });
            changed_any = true;
        }

        if changed_any {
            gizmo_state.graph_modified = true;
        }
    }
}

fn get_input_centroid(graph: &crate::nodes::NodeGraph, node_id: Uuid) -> Option<Vec3> {
    // Find connection to "Input" port
    let input_id = graph
        .connections
        .values()
        .find(|c| c.to_node == node_id && c.to_port == "Input")
        .map(|c| c.from_node)?;

    // Get cached geometry
    let geo = graph.geometry_cache.get(&input_id)?;

    // Calculate centroid
    let Some(pos) = geo
        .get_point_attribute("@P")
        .and_then(|a| a.as_slice::<Vec3>())
    else {
        return Some(Vec3::ZERO);
    };
    if pos.is_empty() {
        return Some(Vec3::ZERO);
    }
    Some(pos.iter().copied().sum::<Vec3>() / pos.len() as f32)
}

register_node!("Transform", "Primitives", TransformNode, TransformNode);

pub fn apply_transform(
    input_geo: &Geometry,
    parameters: &HashMap<String, ParameterValue>,
) -> Result<Geometry, String> {
    puffin::profile_function!();

    let translate = parameters
        .get("translate")
        .and_then(|p| match p {
            ParameterValue::Vec3(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(Vec3::ZERO);
    let rotate_deg = parameters
        .get("rotate")
        .and_then(|p| match p {
            ParameterValue::Vec3(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(Vec3::ZERO);
    let scale = parameters
        .get("scale")
        .and_then(|p| match p {
            ParameterValue::Vec3(v) => Some(*v),
            _ => None,
        })
        .unwrap_or(Vec3::ONE);
    let uniform_scale = parameters
        .get("uniform_scale")
        .and_then(|p| match p {
            ParameterValue::Float(f) => Some(*f),
            _ => None,
        })
        .unwrap_or(1.0);

    let final_scale = scale * uniform_scale;

    // Use the new algorithm library
    puffin::profile_scope!("transform_algorithm");
    Ok(transform::transform_geometry(
        input_geo,
        translate,
        rotate_deg,
        final_scale,
    ))
}

pub fn node_style() -> NodeStyle {
    NodeStyle::Normal
}

pub fn input_style() -> InputStyle {
    InputStyle::Individual
}
