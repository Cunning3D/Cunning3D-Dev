use crate::cunning_core::cda::library::global_cda_library;
use crate::cunning_core::traits::node_interface::ServiceProvider;
use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::spline_selection_utility::is_selectable_tangent;
use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::SelectableElement;
use crate::libs::algorithms::algorithms_runtime::unity_spline::{
    BezierTangent, SelectableKnot, TangentMode,
};
use crate::nodes::parameter::ParameterValue;
use crate::nodes::spline::tool_state::SplineEditMode;
use crate::nodes::spline::tool_state::SplineToolState;
use crate::nodes::structs::NodeType;
use crate::nodes::NodeGraphResource;
use crate::tabs_system::hud_actions::HudAction;
use bevy::prelude::{Quat, Vec3};
use bevy_egui::egui;

pub fn draw_spline_hud(
    ui: &mut egui::Ui,
    services: &dyn ServiceProvider,
    actions: Option<&crate::tabs_system::hud_actions::HudActionQueue>,
    node_id: uuid::Uuid,
) {
    let active_color = egui::Color32::from_rgb(255, 180, 0);
    let inactive_color = egui::Color32::from_gray(90);

    let Some(st) = services.get::<SplineToolState>() else {
        ui.label("Spline (Unity) HUD:");
        ui.label(egui::RichText::new("No spline tool state").color(egui::Color32::GRAY));
        return;
    };

    // --- Top: Draw Spline Mode (pressed/unpressed; W exits) ---
    if let Some(q) = actions {
        let is_draw = st.mode == SplineEditMode::Draw;
        let resp = ui.add(egui::Button::new("Draw Spline Mode").selected(is_draw));
        if resp.clicked() {
            q.push(HudAction::SetSplineEditMode(if is_draw {
                SplineEditMode::Edit
            } else {
                SplineEditMode::Draw
            }));
        }
    } else {
        ui.add_enabled(false, egui::Button::new("Draw Spline Mode"));
    }

    ui.add_space(4.0);
    ui.separator();

    // --- Knot Inspector (Unity-like): active knot only ---
    let active_knot = st.selection.active_element.and_then(|e| match e {
        SelectableElement::Knot(k) => Some(k),
        SelectableElement::Tangent(t) => Some(SelectableKnot {
            spline_index: t.spline_index,
            knot_index: t.knot_index,
        }),
    });
    if active_knot.is_none() {
        ui.label(
            egui::RichText::new("No Knot Selected")
                .color(egui::Color32::GRAY)
                .italics(),
        );
    } else {
        let ksel = active_knot.unwrap();
        let mut knot_world_pos = Vec3::ZERO;
        let mut knot_world_rot = Quat::IDENTITY;
        let mut mode = TangentMode::Linear;
        let mut tin = Vec3::ZERO;
        let mut tout = Vec3::ZERO;
        let mut has_data = false;
        let mut cur_spline: Option<
            crate::libs::algorithms::algorithms_runtime::unity_spline::SplineContainer,
        > = None;
        if let Some(ng) = services.get::<NodeGraphResource>().map(|r| &r.0) {
            let inst = services
                .get::<crate::ui::UiState>()
                .and_then(|ui| {
                    ui.last_selected_node_id
                        .or_else(|| ui.selected_nodes.iter().next().copied())
                })
                .and_then(|id| {
                    ng.nodes.get(&id).and_then(|n| {
                        if matches!(n.node_type, NodeType::CDA(_)) {
                            Some(id)
                        } else {
                            None
                        }
                    })
                });
            if let Some(inst_id) = inst {
                if let Some(inst_n) = ng.nodes.get(&inst_id) {
                    if let NodeType::CDA(data) = &inst_n.node_type {
                        if let Some(v) = data
                            .inner_param_overrides
                            .get(&node_id)
                            .and_then(|m| m.get("spline"))
                        {
                            if let ParameterValue::UnitySpline(c) = v {
                                cur_spline = Some(c.clone());
                            }
                        }
                        if cur_spline.is_none() {
                            if let Some(lib) = global_cda_library() {
                                let _ = lib.ensure_loaded(&data.asset_ref);
                                if let Some(a) = lib.get(data.asset_ref.uuid) {
                                    cur_spline = a
                                        .inner_graph
                                        .nodes
                                        .get(&node_id)
                                        .and_then(|n| {
                                            n.parameters.iter().find(|p| p.name == "spline")
                                        })
                                        .and_then(|p| {
                                            if let ParameterValue::UnitySpline(c) = &p.value {
                                                Some(c.clone())
                                            } else {
                                                None
                                            }
                                        });
                                }
                            }
                        }
                    }
                }
            } else if let Some(node) = ng.nodes.get(&node_id) {
                cur_spline = node
                    .parameters
                    .iter()
                    .find(|p| p.name == "spline")
                    .and_then(|p| {
                        if let ParameterValue::UnitySpline(c) = &p.value {
                            Some(c.clone())
                        } else {
                            None
                        }
                    });
            }
        }
        if let Some(c) = &cur_spline {
            if ksel.spline_index < c.splines.len()
                && ksel.knot_index < c.splines[ksel.spline_index].count()
            {
                let k = c.splines[ksel.spline_index].knots[ksel.knot_index];
                knot_world_pos = c.local_to_world.transform_point3(k.position);
                knot_world_rot = c.local_to_world.to_scale_rotation_translation().1 * k.rotation;
                mode = c.splines[ksel.spline_index].meta[ksel.knot_index].mode;
                tin = k.tangent_in;
                tout = k.tangent_out;
                has_data = true;
            }
        }

        ui.label(
            egui::RichText::new(format!(
                "Knot {} (Spline {}) selected",
                ksel.knot_index, ksel.spline_index
            ))
            .color(active_color),
        );
        ui.add_space(4.0);

        if has_data {
            // Position (world)
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Position")
                        .color(inactive_color)
                        .size(12.0),
                );
                let mut p = knot_world_pos;
                let changed = ui
                    .add(egui::DragValue::new(&mut p.x).speed(0.1).prefix("X "))
                    .changed()
                    | ui.add(egui::DragValue::new(&mut p.y).speed(0.1).prefix("Y "))
                        .changed()
                    | ui.add(egui::DragValue::new(&mut p.z).speed(0.1).prefix("Z "))
                        .changed();
                if changed {
                    if let Some(q) = actions {
                        q.push(HudAction::SetSplineActiveKnotTransform {
                            node_id,
                            spline_index: ksel.spline_index,
                            knot_index: ksel.knot_index,
                            pos_world: Some(p),
                            rot_world: None,
                        });
                    }
                }
            });

            // Rotation (world euler degrees)
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Rotation")
                        .color(inactive_color)
                        .size(12.0),
                );
                let (mut rx, mut ry, mut rz) = knot_world_rot.to_euler(bevy::math::EulerRot::XYZ);
                let mut e = Vec3::new(rx.to_degrees(), ry.to_degrees(), rz.to_degrees());
                let changed = ui
                    .add(egui::DragValue::new(&mut e.x).speed(1.0).prefix("X "))
                    .changed()
                    | ui.add(egui::DragValue::new(&mut e.y).speed(1.0).prefix("Y "))
                        .changed()
                    | ui.add(egui::DragValue::new(&mut e.z).speed(1.0).prefix("Z "))
                        .changed();
                if changed {
                    rx = e.x.to_radians();
                    ry = e.y.to_radians();
                    rz = e.z.to_radians();
                    let rw = Quat::from_euler(bevy::math::EulerRot::XYZ, rx, ry, rz);
                    if let Some(q) = actions {
                        q.push(HudAction::SetSplineActiveKnotTransform {
                            node_id,
                            spline_index: ksel.spline_index,
                            knot_index: ksel.knot_index,
                            pos_world: None,
                            rot_world: Some(rw),
                        });
                    }
                }
            });

            ui.add_space(4.0);
            ui.separator();

            // Mode strip (Unity: Bezier button maps to Mirrored category)
            ui.horizontal(|ui| {
                let is_lin = mode == TangentMode::Linear;
                let is_auto = mode == TangentMode::AutoSmooth;
                let is_bez = matches!(
                    mode,
                    TangentMode::Broken | TangentMode::Continuous | TangentMode::Mirrored
                );
                let b_lin = ui.add(egui::Button::new("Linear").selected(is_lin));
                let b_auto = ui.add(egui::Button::new("Auto").selected(is_auto));
                let b_bez = ui.add(egui::Button::new("Bezier").selected(is_bez));
                if let Some(q) = actions {
                    if b_lin.clicked() {
                        q.push(HudAction::SetSplineActiveKnotBezierMode {
                            node_id,
                            spline_index: ksel.spline_index,
                            knot_index: ksel.knot_index,
                            mode: TangentMode::Linear,
                        });
                    }
                    if b_auto.clicked() {
                        q.push(HudAction::SetSplineActiveKnotBezierMode {
                            node_id,
                            spline_index: ksel.spline_index,
                            knot_index: ksel.knot_index,
                            mode: TangentMode::AutoSmooth,
                        });
                    }
                    if b_bez.clicked() {
                        q.push(HudAction::SetSplineActiveKnotBezierMode {
                            node_id,
                            spline_index: ksel.spline_index,
                            knot_index: ksel.knot_index,
                            mode: TangentMode::Mirrored,
                        });
                    }
                }
            });

            // Bezier dropdown + In/Out lengths
            let tangents_mod =
                crate::libs::algorithms::algorithms_runtime::unity_spline::are_tangents_modifiable(
                    mode,
                );
            let in_sel = cur_spline
                .as_ref()
                .map(|c| {
                    is_selectable_tangent(
                        &c.splines[ksel.spline_index],
                        ksel.knot_index,
                        BezierTangent::In,
                    )
                })
                .unwrap_or(true);
            let out_sel = cur_spline
                .as_ref()
                .map(|c| {
                    is_selectable_tangent(
                        &c.splines[ksel.spline_index],
                        ksel.knot_index,
                        BezierTangent::Out,
                    )
                })
                .unwrap_or(true);

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Bezier")
                        .color(inactive_color)
                        .size(12.0),
                );
                let mut idx = match mode {
                    TangentMode::Mirrored => 0,
                    TangentMode::Continuous => 1,
                    TangentMode::Broken => 2,
                    _ => 0,
                };
                let mut changed = false;
                ui.add_enabled_ui(tangents_mod, |ui| {
                    egui::ComboBox::from_id_salt("bezier_mode")
                        .selected_text(match idx {
                            0 => "Mirrored",
                            1 => "Continuous",
                            _ => "Broken",
                        })
                        .show_ui(ui, |ui| {
                            changed |= ui.selectable_value(&mut idx, 0, "Mirrored").changed();
                            changed |= ui.selectable_value(&mut idx, 1, "Continuous").changed();
                            changed |= ui.selectable_value(&mut idx, 2, "Broken").changed();
                        });
                });
                if tangents_mod && changed {
                    let m = match idx {
                        1 => TangentMode::Continuous,
                        2 => TangentMode::Broken,
                        _ => TangentMode::Mirrored,
                    };
                    if let Some(q) = actions {
                        q.push(HudAction::SetSplineActiveKnotBezierMode {
                            node_id,
                            spline_index: ksel.spline_index,
                            knot_index: ksel.knot_index,
                            mode: m,
                        });
                    }
                }
            });

            let in_len = tin.length();
            let out_len = tout.length();
            ui.collapsing("In", |ui| {
                let mut v = if mode == TangentMode::Linear {
                    0.0
                } else {
                    -in_len
                };
                let enabled = tangents_mod && in_sel;
                let changed = ui
                    .add_enabled(enabled, egui::DragValue::new(&mut v).speed(0.1))
                    .changed();
                if changed {
                    if let Some(q) = actions {
                        q.push(HudAction::SetSplineActiveKnotTangentLength {
                            node_id,
                            spline_index: ksel.spline_index,
                            knot_index: ksel.knot_index,
                            tangent: BezierTangent::In,
                            length: v.abs(),
                        });
                    }
                }
            });
            ui.collapsing("Out", |ui| {
                let mut v = if mode == TangentMode::Linear {
                    0.0
                } else {
                    out_len
                };
                let enabled = tangents_mod && out_sel;
                let changed = ui
                    .add_enabled(enabled, egui::DragValue::new(&mut v).speed(0.1))
                    .changed();
                if changed {
                    if let Some(q) = actions {
                        q.push(HudAction::SetSplineActiveKnotTangentLength {
                            node_id,
                            spline_index: ksel.spline_index,
                            knot_index: ksel.knot_index,
                            tangent: BezierTangent::Out,
                            length: v.abs(),
                        });
                    }
                }
            });
        }
    }

    ui.separator();
    ui.label(format!("Tool: {:?}    (W/E/R)", st.tool));
    ui.label(format!(
        "Draw: {}    (W exits)",
        st.mode == SplineEditMode::Draw
    ));
    ui.label(format!(
        "Pivot: {:?}    Handle: {:?}",
        st.ctx.pivot_mode, st.ctx.handle_orientation
    ));
    ui.label(format!(
        "MoveSnap: {:?}    Incremental: {}",
        st.ctx.move_snap, st.ctx.snapping.incremental_snap_active
    ));
}
