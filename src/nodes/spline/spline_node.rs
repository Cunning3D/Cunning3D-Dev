use bevy::prelude::*;
use std::any::TypeId;
use std::sync::Arc;

use crate::cunning_core::traits::node_interface::{
    GizmoContext, GizmoDrawBuffer, GizmoPart, GizmoState, NodeInteraction, NodeOp,
    NodeParameters, ServiceProvider, XformGizmoMode,
};
use crate::gizmos::standard::StandardGizmo;
use crate::gizmos::{GizmoActionQueue, GizmoBinding, GizmoMovedEvent};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::{Attribute, BezierCurvePrim, GeoPrimitive, Geometry, VertexId};
use crate::nodes::parameter::{Parameter, ParameterUIType, ParameterValue};
use crate::nodes::structs::NodeGraphResource;
use crate::register_node;

use crate::libs::algorithms::algorithms_runtime::unity_spline::{
    calculate_knot_rotation, BezierKnot, MetaData, Spline, SplineContainer, TangentMode,
    CATMULL_ROM_TENSION,
};
use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::{
    HandleOrientation, PivotMode, SelectableElement, SplineSelectionState,
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

    fn draw_gizmos(
        &self,
        buffer: &mut GizmoDrawBuffer,
        context: &GizmoContext,
        gizmo_state: &mut GizmoState,
        services: &dyn ServiceProvider,
        node_id: uuid::Uuid,
    ) {
        let (Some(node_graph_res), Some(actions), Some(spline_tool_state)) = (
            get_service_ref::<NodeGraphResource>(services),
            get_service_ref::<GizmoActionQueue>(services),
            get_service_ref::<crate::nodes::spline::tool_state::SplineToolState>(services),
        ) else {
            return;
        };

        if spline_tool_state.selection.selected_elements.is_empty() {
            return;
        }

        let Some(node) = node_graph_res.0.nodes.get(&node_id) else {
            return;
        };
        let Some(container) = node
            .parameters
            .iter()
            .find(|p| p.name == "spline")
            .and_then(|p| match &p.value {
                ParameterValue::UnitySpline(c) => Some(c.clone()),
                _ => None,
            })
        else {
            return;
        };

        let ctx =
            update_spline_transform_ctx(spline_tool_state.ctx, &container, &spline_tool_state.selection);
        let mode = gizmo_state.xform_mode;
        let show_translate = matches!(
            mode,
            XformGizmoMode::Aggregate | XformGizmoMode::Move | XformGizmoMode::All
        );
        let show_scale = matches!(
            mode,
            XformGizmoMode::Aggregate | XformGizmoMode::Scale | XformGizmoMode::All
        );
        let show_rotate = matches!(mode, XformGizmoMode::Aggregate | XformGizmoMode::All);

        let mut changed_any = false;

        if show_translate {
            let mut gizmo_pos = ctx.pivot_position_world;
            if StandardGizmo::draw_translate(
                buffer,
                context,
                gizmo_state,
                &mut gizmo_pos,
                ctx.handle_rotation_world,
                node_id,
            ) {
                actions.push(GizmoMovedEvent {
                    binding: GizmoBinding::SplineSelectionTranslate { node_id },
                    new_position: gizmo_pos,
                });
                changed_any = true;
            }
        }

        if show_scale {
            let mut total_scale = Vec3::ONE;
            if StandardGizmo::draw_scale(
                buffer,
                context,
                gizmo_state,
                ctx.pivot_position_world,
                &mut total_scale,
                ctx.handle_rotation_world,
                node_id,
            ) {
                actions.push(GizmoMovedEvent {
                    binding: GizmoBinding::SplineSelectionScale { node_id },
                    new_position: total_scale,
                });
                changed_any = true;
            }
        }

        if show_rotate {
            let mut base_rot_deg = quat_to_euler_yxz_deg(ctx.handle_rotation_world);
            if gizmo_state.active_node_id == Some(node_id)
                && matches!(
                    gizmo_state.active_part,
                    Some(
                        GizmoPart::RotateX
                            | GizmoPart::RotateY
                            | GizmoPart::RotateZ
                            | GizmoPart::RotateScreen
                    )
                )
            {
                if let Some(v) = gizmo_state.initial_transform_pos {
                    base_rot_deg = v;
                }
            }

            let mut rotate_abs_deg = base_rot_deg;
            if StandardGizmo::draw_rotate(
                buffer,
                context,
                gizmo_state,
                ctx.pivot_position_world,
                &mut rotate_abs_deg,
                node_id,
            ) {
                actions.push(GizmoMovedEvent {
                    binding: GizmoBinding::SplineSelectionRotate { node_id },
                    // Send total drag delta (from drag-start orientation). Handler converts to frame-step delta.
                    new_position: rotate_abs_deg - base_rot_deg,
                });
                changed_any = true;
            }
        }

        if changed_any {
            gizmo_state.graph_modified = true;
        }
    }
}

register_node!("Spline", "Spline", UnitySplineNode, UnitySplineNode);

#[inline]
fn get_service_ref<T: 'static>(provider: &dyn ServiceProvider) -> Option<&T> {
    provider
        .get_service(TypeId::of::<T>())
        .and_then(|s| s.downcast_ref::<T>())
}

#[inline]
fn quat_to_euler_yxz_deg(q: Quat) -> Vec3 {
    let (y, x, z) = q.to_euler(bevy::prelude::EulerRot::YXZ);
    Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
}

fn update_spline_transform_ctx(
    mut ctx: crate::libs::algorithms::algorithms_runtime::unity_spline::editor::TransformContext,
    c: &SplineContainer,
    sel: &SplineSelectionState,
) -> crate::libs::algorithms::algorithms_runtime::unity_spline::editor::TransformContext {
    let spline_owner_knot = |e: SelectableElement| match e {
        SelectableElement::Knot(k) => Some((k.spline_index, k.knot_index)),
        SelectableElement::Tangent(t) => Some((t.spline_index, t.knot_index)),
    };

    ctx.handle_rotation_world = match ctx.handle_orientation {
        HandleOrientation::Global => Quat::IDENTITY,
        HandleOrientation::Parent => c.local_to_world.to_scale_rotation_translation().1,
        HandleOrientation::Element => {
            let (spline_index, knot_index) = sel
                .active_element
                .and_then(spline_owner_knot)
                .unwrap_or((0, 0));
            if spline_index < c.splines.len() && knot_index < c.splines[spline_index].count() {
                let parent = c.local_to_world.to_scale_rotation_translation().1;
                parent * c.splines[spline_index].knots[knot_index].rotation
            } else {
                Quat::IDENTITY
            }
        }
    };

    ctx.pivot_position_world = match ctx.pivot_mode {
        PivotMode::Pivot => {
            if let Some((si, ki)) = sel.active_element.and_then(spline_owner_knot) {
                if si < c.splines.len() && ki < c.splines[si].count() {
                    c.local_to_world.transform_point3(c.splines[si].knots[ki].position)
                } else {
                    ctx.pivot_position_world
                }
            } else {
                ctx.pivot_position_world
            }
        }
        PivotMode::Center => {
            let mut sum = Vec3::ZERO;
            let mut n = 0.0f32;
            let mut seen: std::collections::HashSet<(usize, usize)> =
                std::collections::HashSet::new();
            for &e in sel.selected_elements.iter() {
                if let Some((si, ki)) = spline_owner_knot(e) {
                    if !seen.insert((si, ki)) {
                        continue;
                    }
                    if si < c.splines.len() && ki < c.splines[si].count() {
                        sum += c.local_to_world.transform_point3(c.splines[si].knots[ki].position);
                        n += 1.0;
                    }
                }
            }
            if n > 0.0 {
                sum / n
            } else {
                ctx.pivot_position_world
            }
        }
    };

    ctx
}
