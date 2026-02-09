use crate::cunning_core::cda::library::global_cda_library;
use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::transform_operation::apply_translation_ctx;
use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::SelectableElement;
use crate::libs::algorithms::algorithms_runtime::unity_spline::{BezierTangent, TangentMode};
use crate::nodes::parameter::ParameterValue;
use crate::nodes::spline::tool_state::SplineEditMode;
use crate::nodes::structs::NodeType;
use crate::nodes::NodeId;
use crate::tabs_system::node_editor::cda;
use crate::{GraphChanged, NodeGraphResource};
use bevy::prelude::*;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub enum HudAction {
    SetSplineEditMode(SplineEditMode),
    SetSplineSelectedTangentMode {
        node_id: NodeId,
        mode: TangentMode,
    },
    SetSplineActiveKnotTransform {
        node_id: NodeId,
        spline_index: usize,
        knot_index: usize,
        pos_world: Option<Vec3>,
        rot_world: Option<Quat>,
    },
    SetSplineActiveKnotBezierMode {
        node_id: NodeId,
        spline_index: usize,
        knot_index: usize,
        mode: TangentMode,
    },
    SetSplineActiveKnotTangentLength {
        node_id: NodeId,
        spline_index: usize,
        knot_index: usize,
        tangent: BezierTangent,
        length: f32,
    },
}

#[derive(Resource, Default)]
pub struct HudActionQueue(Mutex<Vec<HudAction>>);

impl HudActionQueue {
    pub fn push(&self, a: HudAction) {
        self.0.lock().unwrap().push(a);
    }
    fn drain(&self) -> Vec<HudAction> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

pub fn apply_hud_actions_system(
    q: Res<HudActionQueue>,
    mut spline_tool_state: ResMut<crate::nodes::spline::tool_state::SplineToolState>,
    mut node_graph_res: ResMut<NodeGraphResource>,
    node_editor_state: Res<crate::ui::NodeEditorState>,
    ui_state: Res<crate::ui::UiState>,
    mut graph_changed_writer: MessageWriter<GraphChanged>,
) {
    fn pick_inst(ui: &crate::ui::UiState, g: &crate::nodes::structs::NodeGraph) -> Option<NodeId> {
        let id = ui
            .last_selected_node_id
            .or_else(|| ui.selected_nodes.iter().next().copied())?;
        g.nodes.get(&id).and_then(|n| {
            if matches!(n.node_type, NodeType::CDA(_)) {
                Some(id)
            } else {
                None
            }
        })
    }
    fn base_spline(
        data: &crate::nodes::structs::CDANodeData,
        internal: NodeId,
    ) -> Option<crate::libs::algorithms::algorithms_runtime::unity_spline::SplineContainer> {
        if let Some(m) = data
            .inner_param_overrides
            .get(&internal)
            .and_then(|m| m.get("spline"))
        {
            if let ParameterValue::UnitySpline(c) = m {
                return Some(c.clone());
            }
        }
        let lib = global_cda_library()?;
        let _ = lib.ensure_loaded(&data.asset_ref);
        let a = lib.get(data.asset_ref.uuid)?;
        a.inner_graph
            .nodes
            .get(&internal)
            .and_then(|n| n.parameters.iter().find(|p| p.name == "spline"))
            .and_then(|p| {
                if let ParameterValue::UnitySpline(c) = &p.value {
                    Some(c.clone())
                } else {
                    None
                }
            })
    }
    fn store_spline(
        data: &mut crate::nodes::structs::CDANodeData,
        internal: NodeId,
        c: crate::libs::algorithms::algorithms_runtime::unity_spline::SplineContainer,
    ) {
        data.inner_param_overrides
            .entry(internal)
            .or_default()
            .insert("spline".to_string(), ParameterValue::UnitySpline(c));
    }

    let actions = q.drain();
    if actions.is_empty() {
        return;
    }

    let needs_root = actions
        .iter()
        .any(|a| !matches!(a, HudAction::SetSplineEditMode(_)));
    let mut root_guard: Option<&mut crate::nodes::NodeGraph> =
        if needs_root { Some(&mut node_graph_res.0) } else { None };

    let mut dirty_root: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
    let mut dirty_path: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
    for a in actions {
        match a {
            HudAction::SetSplineEditMode(m) => {
                spline_tool_state.mode = m;
                if m == SplineEditMode::Draw {
                    spline_tool_state.selection.clear();
                }
            }
            HudAction::SetSplineSelectedTangentMode { node_id, mode } => {
                let root = root_guard.as_mut().unwrap();
                if let Some(inst_id) = pick_inst(&ui_state, &root) {
                    let Some(inst) = root.nodes.get_mut(&inst_id) else {
                        continue;
                    };
                    let NodeType::CDA(data) = &mut inst.node_type else {
                        continue;
                    };
                    let Some(mut c) = base_spline(data, node_id) else {
                        continue;
                    };
                    for e in spline_tool_state
                        .selection
                        .selected_elements
                        .iter()
                        .copied()
                    {
                        if let SelectableElement::Knot(k) = e {
                            if k.spline_index < c.splines.len()
                                && k.knot_index < c.splines[k.spline_index].count()
                            {
                                let cur = c.splines[k.spline_index].meta[k.knot_index].mode;
                                let m = if cur == TangentMode::AutoSmooth
                                    && mode == TangentMode::Mirrored
                                {
                                    TangentMode::Continuous
                                } else {
                                    mode
                                };
                                c.splines[k.spline_index].set_tangent_mode_no_notify(
                                    k.knot_index,
                                    m,
                                    BezierTangent::Out,
                                );
                            }
                        }
                    }
                    store_spline(data, node_id, c);
                    dirty_root.insert(inst_id);
                } else {
                    let path = node_editor_state.cda_path.clone();
                    cda::navigation::with_graph_by_path_mut(root, &path, |ng| {
                        if let Some(node) = ng.nodes.get_mut(&node_id) {
                            if let Some(param) =
                                node.parameters.iter_mut().find(|p| p.name == "spline")
                            {
                                if let ParameterValue::UnitySpline(c) = &mut param.value {
                                    for e in spline_tool_state
                                        .selection
                                        .selected_elements
                                        .iter()
                                        .copied()
                                    {
                                        if let SelectableElement::Knot(k) = e {
                                            if k.spline_index < c.splines.len()
                                                && k.knot_index < c.splines[k.spline_index].count()
                                            {
                                                let cur = c.splines[k.spline_index].meta
                                                    [k.knot_index]
                                                    .mode;
                                                let m = if cur == TangentMode::AutoSmooth
                                                    && mode == TangentMode::Mirrored
                                                {
                                                    TangentMode::Continuous
                                                } else {
                                                    mode
                                                };
                                                c.splines[k.spline_index]
                                                    .set_tangent_mode_no_notify(
                                                        k.knot_index,
                                                        m,
                                                        BezierTangent::Out,
                                                    );
                                            }
                                        }
                                    }
                                    dirty_path.insert(node_id);
                                }
                            }
                        }
                    });
                }
            }
            HudAction::SetSplineActiveKnotTransform {
                node_id,
                spline_index,
                knot_index,
                pos_world,
                rot_world,
            } => {
                let root = root_guard.as_mut().unwrap();
                if let Some(inst_id) = pick_inst(&ui_state, &root) {
                    let Some(inst) = root.nodes.get_mut(&inst_id) else {
                        continue;
                    };
                    let NodeType::CDA(data) = &mut inst.node_type else {
                        continue;
                    };
                    let Some(mut c) = base_spline(data, node_id) else {
                        continue;
                    };
                    if spline_index < c.splines.len()
                        && knot_index < c.splines[spline_index].count()
                    {
                        if let Some(pw) = pos_world {
                            let ow = c.local_to_world.transform_point3(
                                c.splines[spline_index].knots[knot_index].position,
                            );
                            let d = pw - ow;
                            let sel = [SelectableElement::Knot(crate::libs::algorithms::algorithms_runtime::unity_spline::SelectableKnot { spline_index, knot_index })];
                            apply_translation_ctx(&mut c, &sel, d, spline_tool_state.ctx, None);
                        }
                        if let Some(rw) = rot_world {
                            let parent_rot = c.local_to_world.to_scale_rotation_translation().1;
                            let mut k = c.splines[spline_index].knots[knot_index];
                            k.rotation = parent_rot.inverse() * rw;
                            c.splines[spline_index].set_knot(knot_index, k, BezierTangent::Out);
                        }
                        store_spline(data, node_id, c);
                        dirty_root.insert(inst_id);
                    }
                } else {
                    let path = node_editor_state.cda_path.clone();
                    cda::navigation::with_graph_by_path_mut(root, &path, |ng| {
                        if let Some(node) = ng.nodes.get_mut(&node_id) {
                            if let Some(param) =
                                node.parameters.iter_mut().find(|p| p.name == "spline")
                            {
                                if let ParameterValue::UnitySpline(c) = &mut param.value {
                                    if spline_index < c.splines.len()
                                        && knot_index < c.splines[spline_index].count()
                                    {
                                        if let Some(pw) = pos_world {
                                            let ow = c.local_to_world.transform_point3(
                                                c.splines[spline_index].knots[knot_index].position,
                                            );
                                            let d = pw - ow;
                                            let sel = [SelectableElement::Knot(crate::libs::algorithms::algorithms_runtime::unity_spline::SelectableKnot { spline_index, knot_index })];
                                            apply_translation_ctx(
                                                c,
                                                &sel,
                                                d,
                                                spline_tool_state.ctx,
                                                None,
                                            );
                                        }
                                        if let Some(rw) = rot_world {
                                            let parent_rot =
                                                c.local_to_world.to_scale_rotation_translation().1;
                                            let mut k = c.splines[spline_index].knots[knot_index];
                                            k.rotation = parent_rot.inverse() * rw;
                                            c.splines[spline_index].set_knot(
                                                knot_index,
                                                k,
                                                BezierTangent::Out,
                                            );
                                        }
                                        dirty_path.insert(node_id);
                                    }
                                }
                            }
                        }
                    });
                }
            }
            HudAction::SetSplineActiveKnotBezierMode {
                node_id,
                spline_index,
                knot_index,
                mode,
            } => {
                let root = root_guard.as_mut().unwrap();
                if let Some(inst_id) = pick_inst(&ui_state, &root) {
                    let Some(inst) = root.nodes.get_mut(&inst_id) else {
                        continue;
                    };
                    let NodeType::CDA(data) = &mut inst.node_type else {
                        continue;
                    };
                    let Some(mut c) = base_spline(data, node_id) else {
                        continue;
                    };
                    if spline_index < c.splines.len()
                        && knot_index < c.splines[spline_index].count()
                    {
                        let cur = c.splines[spline_index].meta[knot_index].mode;
                        let m = if cur == TangentMode::AutoSmooth && mode == TangentMode::Mirrored {
                            TangentMode::Continuous
                        } else {
                            mode
                        };
                        c.splines[spline_index].set_tangent_mode_no_notify(
                            knot_index,
                            m,
                            BezierTangent::Out,
                        );
                        store_spline(data, node_id, c);
                        dirty_root.insert(inst_id);
                    }
                } else {
                    let path = node_editor_state.cda_path.clone();
                    cda::navigation::with_graph_by_path_mut(root, &path, |ng| {
                        if let Some(node) = ng.nodes.get_mut(&node_id) {
                            if let Some(param) =
                                node.parameters.iter_mut().find(|p| p.name == "spline")
                            {
                                if let ParameterValue::UnitySpline(c) = &mut param.value {
                                    if spline_index < c.splines.len()
                                        && knot_index < c.splines[spline_index].count()
                                    {
                                        let cur = c.splines[spline_index].meta[knot_index].mode;
                                        let m = if cur == TangentMode::AutoSmooth
                                            && mode == TangentMode::Mirrored
                                        {
                                            TangentMode::Continuous
                                        } else {
                                            mode
                                        };
                                        c.splines[spline_index].set_tangent_mode_no_notify(
                                            knot_index,
                                            m,
                                            BezierTangent::Out,
                                        );
                                        dirty_path.insert(node_id);
                                    }
                                }
                            }
                        }
                    });
                }
            }
            HudAction::SetSplineActiveKnotTangentLength {
                node_id,
                spline_index,
                knot_index,
                tangent,
                length,
            } => {
                let root = root_guard.as_mut().unwrap();
                if let Some(inst_id) = pick_inst(&ui_state, &root) {
                    let Some(inst) = root.nodes.get_mut(&inst_id) else {
                        continue;
                    };
                    let NodeType::CDA(data) = &mut inst.node_type else {
                        continue;
                    };
                    let Some(mut c) = base_spline(data, node_id) else {
                        continue;
                    };
                    if spline_index < c.splines.len()
                        && knot_index < c.splines[spline_index].count()
                    {
                        c.splines[spline_index].set_tangent_length(
                            knot_index,
                            tangent,
                            length.max(0.0),
                        );
                        store_spline(data, node_id, c);
                        dirty_root.insert(inst_id);
                    }
                } else {
                    let path = node_editor_state.cda_path.clone();
                    cda::navigation::with_graph_by_path_mut(root, &path, |ng| {
                        if let Some(node) = ng.nodes.get_mut(&node_id) {
                            if let Some(param) =
                                node.parameters.iter_mut().find(|p| p.name == "spline")
                            {
                                if let ParameterValue::UnitySpline(c) = &mut param.value {
                                    if spline_index < c.splines.len()
                                        && knot_index < c.splines[spline_index].count()
                                    {
                                        c.splines[spline_index].set_tangent_length(
                                            knot_index,
                                            tangent,
                                            length.max(0.0),
                                        );
                                        dirty_path.insert(node_id);
                                    }
                                }
                            }
                        }
                    });
                }
            }
        }
    }
    if !dirty_root.is_empty() || !dirty_path.is_empty() {
        let Some(root) = root_guard.as_mut() else {
            // Should not happen when dirty sets are non-empty.
            return;
        };
        for id in &dirty_root {
            root.mark_dirty(*id);
        }
        if !dirty_path.is_empty() {
            let path = node_editor_state.cda_path.clone();
            cda::navigation::with_graph_by_path_mut(
                &mut *root,
                &path,
                |ng: &mut crate::nodes::NodeGraph| {
                    for id in &dirty_path {
                        ng.mark_dirty(*id);
                    }
                },
            );
        }
        graph_changed_writer.write_default();
    }
}
