//! Node Properties Tab with section-based grouping and Conditional Visibility.
use bevy_egui::egui::{self, Color32, FontId, TextStyle};
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::cunning_core::cda::library::global_cda_library;
use crate::cunning_core::command::basic::CmdAddNode;
use crate::cunning_core::command::basic::CmdRemoveConnections;
use crate::cunning_core::command::basic::CmdSetConnectionOrders;
use crate::cunning_core::command::basic::CmdSetParam;
use crate::cunning_core::command::Command;
use crate::tabs_system::node_editor::cda::drag_payload::ParamDragPayload;
use crate::{
    nodes::parameter::{ParameterUIType, ParameterValue},
    nodes::{NodeId, NodeType},
    tabs_system::{EditorTab, EditorTabContext},
};
use egui_wgpu::sdf::{
    create_gpu_text_callback, create_sdf_rect_callback, GpuTextUniform, SdfRectUniform,
};

const PARAM_BACKGROUND_ROUNDING: f32 = 4.0;
const FILE_BROWSE_BUTTON_WIDTH: f32 = 60.0;
const MIN_COMPONENT_WIDTH: f32 = 1.0;
const PARAM_CONTROL_ROW_HEIGHT_FACTOR: f32 = 1.2;
const GPU_BTN_PAD: f32 = 6.0;

#[inline]
fn import_asset_file(p: &Path, subdir: &str) -> Option<String> {
    let assets = std::env::current_dir().ok()?.join("assets");
    if p.starts_with(&assets) {
        return Some(
            p.strip_prefix(&assets)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    let dst_dir = assets.join(subdir);
    let _ = fs::create_dir_all(&dst_dir);
    let name = p.file_name()?.to_string_lossy().to_string();
    let mut dst = dst_dir.join(&name);
    if dst.exists() {
        let stem = p.file_stem()?.to_string_lossy();
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        for i in 1..4096u32 {
            let n = if ext.is_empty() {
                format!("{}_{}", stem, i)
            } else {
                format!("{}_{}.{}", stem, i, ext)
            };
            let try_dst = dst_dir.join(n);
            if !try_dst.exists() {
                dst = try_dst;
                break;
            }
        }
    }
    fs::copy(p, &dst).ok()?;
    Some(
        dst.strip_prefix(&assets)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/"),
    )
}

#[derive(Default)]
pub struct NodePropertiesTab {
    preset_selected: Option<String>,
    preset_name_input: String,
    pending_preset_action: Option<PresetAction>,
}

#[inline]
fn hash_uuid_u64(id: uuid::Uuid) -> u64 {
    let v = id.as_u128();
    (v as u64) ^ ((v >> 64) as u64).rotate_left(17)
}

#[inline]
#[allow(dead_code)]
fn hash_str_u64(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

#[inline]
#[allow(dead_code)]
fn hash_group_u64(
    group_name: &str,
    params: &[crate::cunning_core::traits::parameter::Parameter],
) -> u64 {
    let mut h = hash_str_u64(group_name) ^ (params.len() as u64).rotate_left(7);
    for p in params {
        h ^= hash_uuid_u64(p.id);
        h = h.rotate_left(11);
    }
    h
}

#[derive(Clone)]
struct NodePropsSnapshot {
    id: NodeId,
    name: String,
    node_type: NodeType,
    position: egui::Pos2,
    size: egui::Vec2,
    parameters: Vec<crate::cunning_core::traits::parameter::Parameter>,
}

enum PresetAction {
    Apply(String),
    Save(String),
    Delete(String),
}

impl EditorTab for NodePropertiesTab {
    fn title(&self) -> egui::WidgetText {
        "Properties".into()
    }

    fn retained_key(&self, _ui: &egui::Ui, context: &EditorTabContext) -> u64 {
        // Retained UI must rebuild when selection changes (click happens outside this tab).
        // Otherwise the cached "no selection" (or previous node) UI can get replayed forever.
        let sel = context
            .ui_state
            .selected_nodes
            .iter()
            .next()
            .copied()
            .or(context.ui_state.last_selected_node_id)
            .map(hash_uuid_u64)
            .unwrap_or(0);
        context.graph_revision ^ sel.rotate_left(1)
    }

    fn ui(&mut self, ui: &mut egui::Ui, context: &mut EditorTabContext) {
        // Fix for 3D Viewport bleed-through
        ui.painter()
            .rect_filled(ui.clip_rect(), 0.0, ui.visuals().panel_fill);

        gpu_label(
            ui,
            "Node Properties",
            ui.style()
                .text_styles
                .get(&TextStyle::Heading)
                .cloned()
                .unwrap_or_else(|| FontId::proportional(20.0)),
            ui.visuals().text_color(),
        );
        ui.separator();

        let Some(selected_node_id) = context
            .ui_state
            .selected_nodes
            .iter()
            .next()
            .copied()
            .or(context.ui_state.last_selected_node_id)
        else {
            gpu_label(
                ui,
                "No node selected.",
                FontId::proportional(14.0),
                ui.visuals().text_color(),
            );
            return;
        };

        let mut changed = false;
        let mut reorder_merge: Option<Vec<crate::nodes::ConnectionId>> = None;
        let mut remove_merge: Option<Vec<crate::nodes::ConnectionId>> = None;
        let preset_action = self.pending_preset_action.take();
        let mut snapshot = {
            let node_graph = &context.node_graph_res.0;
            crate::tabs_system::node_editor::cda::navigation::with_graph_by_path(
                &node_graph,
                &context.node_editor_state.cda_path,
                |g| {
                    g.nodes.get(&selected_node_id).map(|n| NodePropsSnapshot {
                        id: n.id,
                        name: n.name.clone(),
                        node_type: n.node_type.clone(),
                        position: n.position,
                        size: n.size,
                        parameters: n.parameters.clone(),
                    })
                },
            )
        };

        if let Some(node) = snapshot.as_mut() {
            // If we are editing inside a CDA definition, pull the current asset (for link highlight/tooltips).
            let in_cda_edit = !context.node_editor_state.cda_path.is_empty();
            let cda_asset = if !in_cda_edit {
                None
            } else {
                let root_graph = &context.node_graph_res.0;
                let uuid =
                    crate::tabs_system::node_editor::cda::navigation::current_cda_uuid_by_path(
                        &root_graph,
                        &context.node_editor_state.cda_path,
                    );
                uuid.and_then(|id| global_cda_library().and_then(|lib| lib.get(id)))
            };

            // Header card
            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::symmetric(8, 6))
                .rounding(6.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let icon = match &node.node_type {
                            NodeType::CDA(_) => "⬡",
                            NodeType::Merge => "⊕",
                            _ => "◆",
                        };
                        gpu_label(
                            ui,
                            icon,
                            FontId::proportional(16.0),
                            ui.visuals().selection.bg_fill,
                        );
                        ui.push_id(("node_name", node.id), |ui| {
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut node.name)
                                    .font(FontId::proportional(15.0))
                                    .frame(false)
                                    .min_size(egui::vec2(80.0, 0.0)),
                            );
                            if r.changed() {
                                changed = true;
                            }
                        });
                    });
                    ui.add_space(2.0);
                    let type_txt = format!("{}", node.node_type.name());
                    let r = ui.add(
                        egui::Label::new(
                            egui::RichText::new(&type_txt)
                                .size(12.0)
                                .color(ui.visuals().weak_text_color()),
                        )
                        .sense(egui::Sense::click()),
                    );
                    r.on_hover_text(format!("ID: {}", node.id)); // UUID as tooltip
                });

            ui.separator();
            if matches!(&node.node_type, NodeType::Generic(s) if s == "ForEach Begin") {
                let create_meta = ui
                    .horizontal(|ui| ui.button("Create Meta Node").clicked())
                    .inner;
                if create_meta {
                    let block_id = node
                        .parameters
                        .iter()
                        .find(|p| p.name == "block_id")
                        .and_then(|p| {
                            if let crate::nodes::parameter::ParameterValue::String(s) = &p.value {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();
                    if !block_id.is_empty() {
                        let pos = node.position + egui::vec2(0.0, node.size.y + 60.0);
                        let mut meta = crate::ui::prepare_generic_node(
                            context.node_registry,
                            context.node_editor_settings,
                            pos,
                            "ForEach Meta",
                        );
                        if let Some(p) = meta.parameters.iter_mut().find(|p| p.name == "block_id") {
                            p.value = crate::nodes::parameter::ParameterValue::String(block_id);
                        }
                        let meta_id = meta.id;
                        let root = &mut context.node_graph_res.0;
                        let path = context.node_editor_state.cda_path.clone();
                        let st = &mut *context.node_editor_state;
                        crate::tabs_system::node_editor::cda::navigation::with_graph_by_path_mut(
                            root,
                            &path,
                            |g| st.execute(Box::new(CmdAddNode { node: meta }), g),
                        );
                        context.ui_state.selected_nodes.clear();
                        context.ui_state.selected_nodes.insert(meta_id);
                        context.ui_state.last_selected_node_id = Some(meta_id);
                        context.graph_changed_writer.write_default();
                        context
                            .ui_invalidator
                            .request_repaint(crate::invalidator::RepaintCause::DataChanged);
                    }
                }
                ui.separator();
            }
            if matches!(node.node_type, NodeType::Merge) {
                ui.collapsing("Inputs", |ui| {
                    let root_graph = &context.node_graph_res.0;
                    let conns: Vec<(crate::nodes::ConnectionId, String)> =
                        crate::tabs_system::node_editor::cda::navigation::with_graph_by_path(
                            &root_graph,
                            &context.node_editor_state.cda_path,
                            |g| {
                                let mut v: Vec<_> = g
                                    .connections
                                    .values()
                                    .filter(|c| c.to_node == node.id && c.to_port == "Input")
                                    .map(|c| (c.order, c.id, c.from_node, c.from_port.clone()))
                                    .collect();
                                v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
                                v.into_iter()
                                    .map(|(_ord, id, from_id, from_port)| {
                                        let name = g
                                            .nodes
                                            .get(&from_id)
                                            .map(|n| n.name.as_str())
                                            .unwrap_or("?");
                                        (id, format!("{} :: {}", name, from_port))
                                    })
                                    .collect()
                            },
                        );
                    if conns.is_empty() {
                        ui.label("(No inputs)");
                        return;
                    }
                    for (i, (cid, label)) in conns.iter().cloned().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("✕").clicked() {
                                        remove_merge.get_or_insert_with(Vec::new).push(cid);
                                    }
                                    if i + 1 < conns.len() && ui.small_button("↓").clicked() {
                                        let mut ids: Vec<_> =
                                            conns.iter().map(|(id, _)| *id).collect();
                                        ids.swap(i, i + 1);
                                        reorder_merge = Some(ids);
                                    }
                                    if i > 0 && ui.small_button("↑").clicked() {
                                        let mut ids: Vec<_> =
                                            conns.iter().map(|(id, _)| *id).collect();
                                        ids.swap(i, i - 1);
                                        reorder_merge = Some(ids);
                                    }
                                },
                            );
                        });
                    }
                });
                ui.separator();
            }
            if let NodeType::CDA(cda) = &node.node_type {
                let names = crate::cunning_core::cda::library::global_cda_library()
                    .and_then(|lib| lib.def_guard(cda.asset_ref.uuid))
                    .map(|g| g.asset().preset_names())
                    .unwrap_or_default();
                if self.preset_selected.is_none() && !names.is_empty() {
                    self.preset_selected = Some(names[0].clone());
                }
                ui.collapsing("Presets", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Preset:");
                        egui::ComboBox::from_id_source(("cda_preset_combo", node.id))
                            .selected_text(
                                self.preset_selected
                                    .clone()
                                    .unwrap_or_else(|| "(None)".into()),
                            )
                            .show_ui(ui, |ui| {
                                for n in &names {
                                    ui.selectable_value(
                                        &mut self.preset_selected,
                                        Some(n.clone()),
                                        n,
                                    );
                                }
                            });
                        if ui.button("Apply").clicked() {
                            if let Some(n) = self.preset_selected.clone() {
                                self.pending_preset_action = Some(PresetAction::Apply(n));
                            }
                        }
                        if ui.button("Delete").clicked() {
                            if let Some(n) = self.preset_selected.clone() {
                                self.pending_preset_action = Some(PresetAction::Delete(n));
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Save As:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.preset_name_input)
                                .desired_width(ui.available_width().min(220.0)),
                        );
                        if ui.button("Save").clicked() {
                            let n = self.preset_name_input.trim().to_string();
                            if !n.is_empty() {
                                self.pending_preset_action = Some(PresetAction::Save(n));
                                self.preset_name_input.clear();
                            }
                        }
                    });
                });
                ui.separator();
            }

            // Pre-calculate values for visibility check
            let param_values: std::collections::HashMap<String, ParameterValue> = node
                .parameters
                .iter()
                .map(|p| (p.name.clone(), p.value.clone()))
                .collect();

            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut groups = node
                    .parameters
                    .iter()
                    .map(|p| p.group.clone())
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                groups.sort();
                for group in groups {
                    let mut idxs: Vec<usize> = Vec::new();
                    for (i, p) in node.parameters.iter().enumerate() {
                        if p.group != group {
                            continue;
                        }
                        if let Some(cond) = &p.visible_condition {
                            if !check_visibility(cond, &param_values) {
                                continue;
                            }
                        }
                        idxs.push(i);
                    }
                    if idxs.is_empty() {
                        continue;
                    }
                    let title = group.to_ascii_uppercase();
                    egui_extras::section(
                        ui,
                        ("node_props_group", node.id, &group),
                        &title,
                        group != "Debug",
                        |ui| {
                            egui::Grid::new(("node_props_grid", node.id, &group))
                                .num_columns(2)
                                .show(ui, |ui| {
                                    for i in idxs {
                                        let param = &mut node.parameters[i];
                                        let show_label = !matches!(
                                            param.ui_type,
                                            ParameterUIType::Separator
                                                | ParameterUIType::CurvePoints
                                                | ParameterUIType::Button
                                                | ParameterUIType::BusyButton { .. }
                                        );
                                        if show_label {
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    gpu_label(
                                                        ui,
                                                        format!("{}:", param.label),
                                                        FontId::proportional(13.0),
                                                        ui.visuals().text_color(),
                                                    );
                                                },
                                            );
                                        } else {
                                            ui.allocate_space(egui::vec2(
                                                0.0,
                                                ui.spacing().interact_size.y,
                                            ));
                                        }
                                        let r = ui.push_id(param.id, |ui| {
                                            ui.horizontal(|ui| {
                                                // Drag handle only in CDA edit mode
                                                if in_cda_edit {
                                                    let v = param.value.clone();
                                                    let drag_id = egui::Id::new((
                                                        "cda_param_drag",
                                                        node.id,
                                                        param.name.as_str(),
                                                        param.id,
                                                    ));
                                                    ui.dnd_drag_source(
                                                        drag_id,
                                                        ParamDragPayload::new(
                                                            node.id,
                                                            &param.name,
                                                            &v,
                                                        )
                                                        .with_label(&param.label),
                                                        |ui| {
                                                            ui.label(
                                                                egui::RichText::new("≡")
                                                                    .color(Color32::GRAY),
                                                            );
                                                        },
                                                    );
                                                }
                                                if let Some(a) = &cda_asset {
                                                    let links = a.promoted_links_for_param(
                                                        node.id,
                                                        &param.name,
                                                    );
                                                    if !links.is_empty() {
                                                        let addr = format!(
                                                            "cda://{}/node/{}/param/{}",
                                                            a.id, node.id, param.name
                                                        );
                                                        let txt = egui::RichText::new("LINK")
                                                            .small()
                                                            .color(Color32::from_rgb(
                                                                120, 200, 255,
                                                            ));
                                                        let r = ui
                                                            .add(
                                                                egui::Label::new(txt)
                                                                    .sense(egui::Sense::click()),
                                                            )
                                                            .on_hover_text(format!(
                                                                "{}\n{}",
                                                                links.join("\n"),
                                                                addr
                                                            ));
                                                        if r.clicked() {
                                                            ui.ctx().copy_text(addr.clone());
                                                        }
                                                    }
                                                }
                                                show_parameter(
                                                    ui,
                                                    &param.name,
                                                    &param.label,
                                                    &mut param.value,
                                                    &param.ui_type,
                                                    &param_values,
                                                )
                                            })
                                            .inner
                                        });
                                        changed |= r.inner;
                                        ui.end_row();
                                    }
                                });
                        },
                    );
                    ui.add_space(6.0);
                }
            });
        } else {
            gpu_label(
                ui,
                "No node selected.",
                FontId::proportional(14.0),
                ui.visuals().text_color(),
            );
        }

        if let Some(ids) = remove_merge.take() {
            let path = context.node_editor_state.cda_path.clone();
            let mut cmd = CmdRemoveConnections::new(ids);
            {
                let root_graph = &mut context.node_graph_res.0;
                crate::tabs_system::node_editor::cda::navigation::with_graph_by_path_mut(
                    root_graph,
                    &path,
                    |node_graph| cmd.apply(node_graph),
                );
            }
            context.node_editor_state.record(Box::new(cmd));
            context.graph_changed_writer.write_default();
        }

        if let Some(ids) = reorder_merge.take() {
            let path = context.node_editor_state.cda_path.clone();
            let mut cmd = CmdSetConnectionOrders::new(
                selected_node_id,
                crate::nodes::PortId::from("Input"),
                ids,
            );
            {
                let root_graph = &mut context.node_graph_res.0;
                crate::tabs_system::node_editor::cda::navigation::with_graph_by_path_mut(
                    root_graph,
                    &path,
                    |node_graph| cmd.apply(node_graph),
                );
            }
            context.node_editor_state.record(Box::new(cmd));
            context.graph_changed_writer.write_default();
        }

        if changed || preset_action.is_some() || self.pending_preset_action.is_some() {
            if let Some(snapshot) = snapshot.take() {
                let node_graph = &mut context.node_graph_res.0;
                let path = context.node_editor_state.cda_path.clone();
                let mut param_cmds: Vec<(uuid::Uuid, ParameterValue, ParameterValue)> = Vec::new();
                crate::tabs_system::node_editor::cda::navigation::with_graph_by_path_mut(
                    node_graph,
                    &path,
                    |g| {
                        if let Some(node) = g.nodes.get_mut(&selected_node_id) {
                            node.name = snapshot.name;
                            for p in snapshot.parameters {
                                if let Some(dst) = node.parameters.iter().find(|d| d.id == p.id) {
                                    if dst.value != p.value {
                                        param_cmds.push((p.id, dst.value.clone(), p.value.clone()));
                                    }
                                }
                            }
                            if let NodeType::CDA(data) = &node.node_type {
                                let a = preset_action.or_else(|| self.pending_preset_action.take());
                                if let Some(a) = a {
                                    if let Some(lib) =
                                        crate::cunning_core::cda::library::global_cda_library()
                                    {
                                        if let Some(mut def) = lib.def_guard(data.asset_ref.uuid) {
                                            let asset = def.asset_mut();
                                            match a {
                                                PresetAction::Save(name) => {
                                                    let mut values: BTreeMap<
                                                        String,
                                                        ParameterValue,
                                                    > = BTreeMap::new();
                                                    for p in &node.parameters {
                                                        values.insert(
                                                            p.name.clone(),
                                                            p.value.clone(),
                                                        );
                                                    }
                                                    asset.upsert_preset(name, values);
                                                }
                                                PresetAction::Apply(name) => {
                                                    if let Some(preset) = asset.get_preset(&name) {
                                                        for (k, v) in &preset.values {
                                                            if let Some(p) = node
                                                                .parameters
                                                                .iter_mut()
                                                                .find(|p| p.name == *k)
                                                            {
                                                                p.value = v.clone();
                                                            }
                                                        }
                                                    }
                                                }
                                                PresetAction::Delete(name) => {
                                                    asset.remove_preset(&name);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        for (pid, old, new) in param_cmds.drain(..) {
                            context.node_editor_state.execute(
                                Box::new(CmdSetParam::new(selected_node_id, pid, old, new)),
                                g,
                            );
                        }
                        g.mark_dirty(selected_node_id);
                    },
                );
                context.graph_changed_writer.write_default();
            }
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn check_visibility(
    condition: &str,
    params: &std::collections::HashMap<String, ParameterValue>,
) -> bool {
    let condition = condition.trim();
    if condition.is_empty() {
        return true;
    }

    let (key, op, target_val_str) = if let Some((k, v)) = condition.split_once("==") {
        (k.trim(), "==", v.trim())
    } else if let Some((k, v)) = condition.split_once("!=") {
        (k.trim(), "!=", v.trim())
    } else if condition.starts_with('!') {
        (condition[1..].trim(), "==", "0") // !param => param == 0
    } else {
        (condition, "==", "1") // param => param == 1 (truthy)
    };

    let Some(val) = params.get(key) else {
        return true;
    };

    let current_val = match val {
        ParameterValue::Float(v) => *v,
        ParameterValue::Int(v) => *v as f32,
        ParameterValue::Bool(v) => {
            if *v {
                1.0
            } else {
                0.0
            }
        }
        ParameterValue::Vec2(v) => v.x,
        _ => 0.0,
    };

    let target_val = match target_val_str {
        "true" | "True" => 1.0,
        "false" | "False" => 0.0,
        s => s.parse::<f32>().unwrap_or(0.0),
    };

    let epsilon = 0.0001;
    let eq = (current_val - target_val).abs() < epsilon;

    match op {
        "==" => eq,
        "!=" => !eq,
        _ => true,
    }
}

fn gpu_label(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    font_id: FontId,
    color: Color32,
) -> egui::Response {
    let text = text.into();
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), font_id.clone(), color));
    let (rect, resp) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let frame_id = ui.ctx().cumulative_frame_nr();
        let family = if matches!(font_id.family, egui::FontFamily::Monospace) {
            1
        } else {
            0
        };
        p.add(create_gpu_text_callback(
            p.clip_rect(),
            GpuTextUniform {
                text,
                pos: rect.min,
                color,
                font_px: font_id.size,
                bounds: galley.size(),
                family,
            },
            frame_id,
        ));
    }
    resp
}

fn gpu_text_button(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    font_id: FontId,
    min_size: egui::Vec2,
) -> egui::Response {
    let text = text.into();
    let color = ui.visuals().text_color();
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), font_id.clone(), color));
    let desired = egui::vec2(
        (galley.size().x + GPU_BTN_PAD * 2.0).max(min_size.x),
        (galley.size().y + GPU_BTN_PAD * 2.0).max(min_size.y),
    );
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&resp);
        let frame_id = ui.ctx().cumulative_frame_nr();
        let screen = ui.ctx().screen_rect().size();
        sdf_box(
            ui.painter(),
            rect,
            visuals.rounding().nw as f32,
            visuals.bg_fill,
            visuals.bg_stroke.width,
            visuals.bg_stroke.color,
            frame_id,
            screen,
        );
        let pos = egui::pos2(
            rect.center().x - galley.size().x * 0.5,
            rect.center().y - galley.size().y * 0.5,
        );
        let frame_id = ui.ctx().cumulative_frame_nr();
        let family = if matches!(font_id.family, egui::FontFamily::Monospace) {
            1
        } else {
            0
        };
        ui.painter().add(create_gpu_text_callback(
            ui.painter().clip_rect(),
            GpuTextUniform {
                text,
                pos,
                color,
                font_px: font_id.size,
                bounds: galley.size(),
                family,
            },
            frame_id,
        ));
    }
    resp
}

fn sdf_box(
    p: &egui::Painter,
    rect: egui::Rect,
    rounding: f32,
    fill: Color32,
    border_w: f32,
    border: Color32,
    frame_id: u64,
    screen: egui::Vec2,
) {
    let c = rect.center();
    let s = rect.size();
    let fill_rgba = egui::Rgba::from(fill).to_array();
    let border_rgba = egui::Rgba::from(border).to_array();
    let uniform = SdfRectUniform {
        center: [c.x, c.y],
        half_size: [s.x * 0.5, s.y * 0.5],
        corner_radii: [rounding; 4],
        fill_color: fill_rgba,
        shadow_color: [0.0; 4],
        shadow_blur: 0.0,
        _pad1: 0.0,
        shadow_offset: [0.0, 0.0],
        border_width: border_w,
        _pad2: [0.0; 3],
        border_color: border_rgba,
        screen_size: [screen.x, screen.y],
        _pad3: [0.0; 2],
    };
    p.add(create_sdf_rect_callback(
        rect.expand(2.0),
        uniform,
        frame_id,
    ));
}

fn show_parameter(
    ui: &mut egui::Ui,
    _param_name: &str,
    param_label: &str,
    value: &mut ParameterValue,
    ui_type: &ParameterUIType,
    param_values: &std::collections::HashMap<String, ParameterValue>,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let frame_id = ui.ctx().cumulative_frame_nr();
        let screen = ui.ctx().screen_rect().size();
        // Grid already draws label; only draw control widget here
        let avail_width = ui.available_width();
        let spacing = ui.spacing().item_spacing.x;
        let interact_height = ui.spacing().interact_size.y;
        let control_height = interact_height * PARAM_CONTROL_ROW_HEIGHT_FACTOR;

        match ui_type {
            ParameterUIType::Button => {
                let r = ui.add_sized([avail_width, control_height], egui::Button::new(param_label));
                if r.clicked() { match value { ParameterValue::Int(v) => *v += 1, _ => *value = ParameterValue::Int(1) }; changed = true; }
            }
            ParameterUIType::BusyButton { busy_param, busy_label, busy_label_param } => {
                let busy = param_values.get(busy_param).is_some_and(|v| match v { ParameterValue::Bool(b) => *b, ParameterValue::Int(i) => *i != 0, ParameterValue::Float(f) => *f != 0.0, _ => false });
                let txt = if busy {
                    busy_label_param
                        .as_deref()
                        .and_then(|n| param_values.get(n))
                        .and_then(|v| if let ParameterValue::String(s) = v { Some(s.as_str()) } else { None })
                        .unwrap_or_else(|| busy_label.as_str())
                } else {
                    param_label
                };
                let r = ui.add_enabled(!busy, egui::Button::new(txt).min_size(egui::vec2(avail_width, control_height)));
                if r.clicked() { match value { ParameterValue::Int(v) => *v += 1, _ => *value = ParameterValue::Int(1) }; changed = true; }
            }
            ParameterUIType::FloatSlider { min, max } => {
                if let ParameterValue::Float(val) = value {
                    let range = *min..=*max;
                    let size = egui::vec2(avail_width, control_height);
                    let (rect, _response) = ui.allocate_exact_size(size, egui::Sense::hover());

                    if ui.is_rect_visible(rect) {
                        let painter = ui.painter();
                        sdf_box(painter, rect, PARAM_BACKGROUND_ROUNDING, ui.visuals().extreme_bg_color, 0.0, Color32::TRANSPARENT, frame_id, screen);

                        let denom = *max - *min;
                        if denom > 0.0 {
                            let progress = ((*val - min) / denom).clamp(0.0, 1.0);
                            if progress > 0.0 {
                                let fill_width = rect.width() * progress;
                                let fill_rect = egui::Rect::from_min_size(
                                    rect.min,
                                    egui::vec2(fill_width, rect.height()),
                                );
                                let fill_color =
                                    ui.visuals().selection.bg_fill.linear_multiply(0.3);
                                sdf_box(painter, fill_rect, PARAM_BACKGROUND_ROUNDING, fill_color, 0.0, Color32::TRANSPARENT, frame_id, screen);
                            }
                        }
                    }

                    let mut child_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    {
                        let widgets = &mut child_ui.style_mut().visuals.widgets;
                        widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                        widgets.hovered.bg_fill = egui::Color32::TRANSPARENT;
                        widgets.active.bg_fill = egui::Color32::TRANSPARENT;
                    }

                    if child_ui
                        .add_sized(
                            rect.size(),
                            egui::DragValue::new(val)
                                .clamp_range(range)
                                .speed(0.01),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                }
            }
            ParameterUIType::IntSlider { min, max } => {
                if let ParameterValue::Int(val) = value {
                    let range = *min..=*max;
                    let size = egui::vec2(avail_width, interact_height);
                    let (rect, _response) = ui.allocate_exact_size(size, egui::Sense::hover());

                    if ui.is_rect_visible(rect) {
                        let painter = ui.painter();
                        sdf_box(painter, rect, PARAM_BACKGROUND_ROUNDING, ui.visuals().extreme_bg_color, 0.0, Color32::TRANSPARENT, frame_id, screen);

                        let denom = (*max - *min) as f32;
                        if denom > 0.0 {
                            let progress = ((*val as f32 - *min as f32) / denom).clamp(0.0, 1.0);
                            if progress > 0.0 {
                                let fill_width = rect.width() * progress;
                                let fill_rect = egui::Rect::from_min_size(
                                    rect.min,
                                    egui::vec2(fill_width, rect.height()),
                                );
                                let fill_color =
                                    ui.visuals().selection.bg_fill.linear_multiply(0.3);
                                sdf_box(painter, fill_rect, PARAM_BACKGROUND_ROUNDING, fill_color, 0.0, Color32::TRANSPARENT, frame_id, screen);
                            }
                        }
                    }

                    let mut child_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    {
                        let widgets = &mut child_ui.style_mut().visuals.widgets;
                        widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                        widgets.hovered.bg_fill = egui::Color32::TRANSPARENT;
                        widgets.active.bg_fill = egui::Color32::TRANSPARENT;
                    }

                    if child_ui
                        .add_sized(
                            rect.size(),
                            egui::DragValue::new(val).clamp_range(range),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                }
            }
            ParameterUIType::Dropdown { choices } => {
                if let ParameterValue::Int(val) = value {
                    let selected_label = choices
                        .iter()
                        .find(|(_, v)| v == val)
                        .map(|(s, _)| s.clone())
                        .unwrap_or_else(|| "Invalid".to_string());

                    let (rect, _) = ui.allocate_exact_size(egui::vec2(avail_width, control_height), egui::Sense::hover());
                    if ui.is_rect_visible(rect) { sdf_box(ui.painter(), rect, PARAM_BACKGROUND_ROUNDING, ui.visuals().extreme_bg_color, 1.0, ui.visuals().widgets.inactive.bg_stroke.color, frame_id, screen); }
                    let mut child_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    { let w = &mut child_ui.style_mut().visuals.widgets; for s in [&mut w.inactive, &mut w.hovered, &mut w.active] { s.bg_fill = Color32::TRANSPARENT; s.bg_stroke.color = Color32::TRANSPARENT; } }
                    egui::ComboBox::from_id_salt(_param_name)
                        .selected_text(selected_label)
                        .width(rect.width())
                        .show_ui(&mut child_ui, |ui| {
                            for (label, choice_value) in choices {
                                if ui
                                    .selectable_value(val, *choice_value, label)
                                    .changed()
                                {
                                    changed = true;
                                }
                            }
                        });
                }
            }
            ParameterUIType::Vec2Drag => {
                if let ParameterValue::Vec2(val) = value {
                    let parts = 2.0;
                    let size = egui::vec2(avail_width, control_height);
                    let (row_rect, _response) =
                        ui.allocate_exact_size(size, egui::Sense::hover());

                    let total = row_rect.width();
                    let w = ((total - (parts - 1.0) * spacing) / parts)
                        .max(MIN_COMPONENT_WIDTH);

                    let mut cursor = row_rect.min;

                    // x
                    {
                        let comp_rect = egui::Rect::from_min_size(
                            cursor,
                            egui::vec2(w, row_rect.height()),
                        );
                        if ui.is_rect_visible(comp_rect) { sdf_box(ui.painter(), comp_rect, PARAM_BACKGROUND_ROUNDING, ui.visuals().extreme_bg_color, 0.0, Color32::TRANSPARENT, frame_id, screen); }
                        let mut child_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(comp_rect)
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        { let widgets = &mut child_ui.style_mut().visuals.widgets; widgets.inactive.bg_fill = Color32::TRANSPARENT; widgets.hovered.bg_fill = Color32::TRANSPARENT; widgets.active.bg_fill = Color32::TRANSPARENT; }
                        if child_ui.add_sized(comp_rect.size(), egui::DragValue::new(&mut val.x).prefix("X:")).changed() { changed = true; }
                    }
                    cursor.x += w + spacing;
                    // y
                    {
                        let comp_rect = egui::Rect::from_min_size(cursor, egui::vec2(w, row_rect.height()));
                        if ui.is_rect_visible(comp_rect) { sdf_box(ui.painter(), comp_rect, PARAM_BACKGROUND_ROUNDING, ui.visuals().extreme_bg_color, 0.0, Color32::TRANSPARENT, frame_id, screen); }
                        let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(comp_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
                        { let widgets = &mut child_ui.style_mut().visuals.widgets; widgets.inactive.bg_fill = Color32::TRANSPARENT; widgets.hovered.bg_fill = Color32::TRANSPARENT; widgets.active.bg_fill = Color32::TRANSPARENT; }
                        if child_ui.add_sized(comp_rect.size(), egui::DragValue::new(&mut val.y).prefix("Y:")).changed() { changed = true; }
                    }
                }
            }
            ParameterUIType::Vec3Drag => {
                if let ParameterValue::Vec3(val) = value {
                    let parts = 3.0;
                    let size = egui::vec2(avail_width, control_height);
                    let (row_rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                    let total = row_rect.width();
                    let w = ((total - (parts - 1.0) * spacing) / parts).max(MIN_COMPONENT_WIDTH);
                    let mut cursor = row_rect.min;
                    // x
                    {
                        let comp_rect = egui::Rect::from_min_size(cursor, egui::vec2(w, row_rect.height()));
                        if ui.is_rect_visible(comp_rect) { sdf_box(ui.painter(), comp_rect, PARAM_BACKGROUND_ROUNDING, ui.visuals().extreme_bg_color, 0.0, Color32::TRANSPARENT, frame_id, screen); }
                        let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(comp_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
                        { let widgets = &mut child_ui.style_mut().visuals.widgets; widgets.inactive.bg_fill = Color32::TRANSPARENT; widgets.hovered.bg_fill = Color32::TRANSPARENT; widgets.active.bg_fill = Color32::TRANSPARENT; }
                        if child_ui.add_sized(comp_rect.size(), egui::DragValue::new(&mut val.x).prefix("X:")).changed() { changed = true; }
                    }
                    cursor.x += w + spacing;
                    // y
                    {
                        let comp_rect = egui::Rect::from_min_size(cursor, egui::vec2(w, row_rect.height()));
                        if ui.is_rect_visible(comp_rect) { sdf_box(ui.painter(), comp_rect, PARAM_BACKGROUND_ROUNDING, ui.visuals().extreme_bg_color, 0.0, Color32::TRANSPARENT, frame_id, screen); }
                        let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(comp_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
                        { let widgets = &mut child_ui.style_mut().visuals.widgets; widgets.inactive.bg_fill = Color32::TRANSPARENT; widgets.hovered.bg_fill = Color32::TRANSPARENT; widgets.active.bg_fill = Color32::TRANSPARENT; }
                        if child_ui.add_sized(comp_rect.size(), egui::DragValue::new(&mut val.y).prefix("Y:")).changed() { changed = true; }
                    }
                    cursor.x += w + spacing;
                    // z
                    {
                        let comp_rect = egui::Rect::from_min_size(cursor, egui::vec2(w, row_rect.height()));
                        if ui.is_rect_visible(comp_rect) { sdf_box(ui.painter(), comp_rect, PARAM_BACKGROUND_ROUNDING, ui.visuals().extreme_bg_color, 0.0, Color32::TRANSPARENT, frame_id, screen); }
                        let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(comp_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
                        { let widgets = &mut child_ui.style_mut().visuals.widgets; widgets.inactive.bg_fill = Color32::TRANSPARENT; widgets.hovered.bg_fill = Color32::TRANSPARENT; widgets.active.bg_fill = Color32::TRANSPARENT; }
                        if child_ui.add_sized(comp_rect.size(), egui::DragValue::new(&mut val.z).prefix("Z:")).changed() { changed = true; }
                    }
                }
            }
            ParameterUIType::Vec4Drag => {
                if let ParameterValue::Vec4(val) = value {
                    let mut v = [val.x, val.y, val.z, val.w];
                    let labels = ["X:", "Y:", "Z:", "W:"];
                    ui.horizontal(|ui| {
                        for i in 0..4 {
                            let mut d = v[i];
                            if ui.add(egui::DragValue::new(&mut d).speed(0.01).prefix(labels[i])).changed() { v[i] = d; changed = true; }
                        }
                    });
                    if changed { *val = bevy::prelude::Vec4::new(v[0], v[1], v[2], v[3]); }
                }
            }
            ParameterUIType::IVec2Drag => {
                if let ParameterValue::IVec2(val) = value {
                    let parts = 2.0;
                    let size = egui::vec2(avail_width, interact_height);
                    let (row_rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                    let total = row_rect.width();
                    let w = ((total - (parts - 1.0) * spacing) / parts).max(MIN_COMPONENT_WIDTH);
                    let mut cursor = row_rect.min;
                    // x
                    {
                        let comp_rect = egui::Rect::from_min_size(cursor, egui::vec2(w, row_rect.height()));
                        if ui.is_rect_visible(comp_rect) { sdf_box(ui.painter(), comp_rect, PARAM_BACKGROUND_ROUNDING, ui.visuals().extreme_bg_color, 0.0, Color32::TRANSPARENT, frame_id, screen); }
                        let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(comp_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
                        { let widgets = &mut child_ui.style_mut().visuals.widgets; widgets.inactive.bg_fill = Color32::TRANSPARENT; widgets.hovered.bg_fill = Color32::TRANSPARENT; widgets.active.bg_fill = Color32::TRANSPARENT; }
                        if child_ui.add_sized(comp_rect.size(), egui::DragValue::new(&mut val.x).prefix("X:")).changed() { changed = true; }
                    }
                    cursor.x += w + spacing;
                    // y
                    {
                        let comp_rect = egui::Rect::from_min_size(cursor, egui::vec2(w, row_rect.height()));
                        if ui.is_rect_visible(comp_rect) { sdf_box(ui.painter(), comp_rect, PARAM_BACKGROUND_ROUNDING, ui.visuals().extreme_bg_color, 0.0, Color32::TRANSPARENT, frame_id, screen); }
                        let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(comp_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
                        { let widgets = &mut child_ui.style_mut().visuals.widgets; widgets.inactive.bg_fill = Color32::TRANSPARENT; widgets.hovered.bg_fill = Color32::TRANSPARENT; widgets.active.bg_fill = Color32::TRANSPARENT; }
                        if child_ui.add_sized(comp_rect.size(), egui::DragValue::new(&mut val.y).prefix("Y:")).changed() { changed = true; }
                    }
                }
            }
            ParameterUIType::Toggle => {
                if let ParameterValue::Bool(val) = value {
                    // Grid draws label; only checkbox here
                    if ui.checkbox(val, "").changed() {
                        changed = true;
                    }
                }
            }
            ParameterUIType::String => {
                if let ParameterValue::String(val) = value {
                    if _param_name == "File Path" {
                        let btn_width = FILE_BROWSE_BUTTON_WIDTH;
                        let text_width =
                            (avail_width - btn_width - spacing).max(MIN_COMPONENT_WIDTH * 10.0);

                        let mut r = {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(text_width, control_height), egui::Sense::hover());
                            if ui.is_rect_visible(rect) { sdf_box(ui.painter(), rect, PARAM_BACKGROUND_ROUNDING, ui.visuals().extreme_bg_color, 1.0, ui.visuals().widgets.inactive.bg_stroke.color, frame_id, screen); }
                            let mut child_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                            );
                            { let w = &mut child_ui.style_mut().visuals.widgets; for s in [&mut w.inactive, &mut w.hovered, &mut w.active] { s.bg_fill = Color32::TRANSPARENT; s.bg_stroke.color = Color32::TRANSPARENT; } }
                            child_ui.add_sized(rect.size(), egui::TextEdit::singleline(val).frame(false))
                        };

                        #[cfg(not(target_arch = "wasm32"))]
                        if gpu_text_button(ui, "Browse...", FontId::proportional(14.0), egui::vec2(btn_width, control_height)).clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("fbx", &["fbx"])
                                .pick_file()
                            {
                                *val = path.display().to_string();
                                r.mark_changed();
                                changed = true;
                            }
                        }
                        changed = r.changed() || changed;
                    } else {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(avail_width, control_height), egui::Sense::hover());
                        if ui.is_rect_visible(rect) { sdf_box(ui.painter(), rect, PARAM_BACKGROUND_ROUNDING, ui.visuals().extreme_bg_color, 1.0, ui.visuals().widgets.inactive.bg_stroke.color, frame_id, screen); }
                        let mut child_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(rect)
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        { let w = &mut child_ui.style_mut().visuals.widgets; for s in [&mut w.inactive, &mut w.hovered, &mut w.active] { s.bg_fill = Color32::TRANSPARENT; s.bg_stroke.color = Color32::TRANSPARENT; } }
                        changed = child_ui.add_sized(rect.size(), egui::TextEdit::singleline(val).frame(false)).changed();
                    }
                }
            }
            ParameterUIType::Color { show_alpha } => {
                if *show_alpha {
                    match value {
                        ParameterValue::Color4(val) => {
                            let mut rgba = [val.x, val.y, val.z, val.w];
                            if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                                val.x = rgba[0]; val.y = rgba[1]; val.z = rgba[2]; val.w = rgba[3];
                                changed = true;
                            }
                        }
                        ParameterValue::Color(val) => {
                            let mut rgba = [val.x, val.y, val.z, 1.0];
                            if ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed() {
                                val.x = rgba[0]; val.y = rgba[1]; val.z = rgba[2];
                                changed = true;
                            }
                        }
                        _ => {}
                    }
                } else if let ParameterValue::Color(val) = value {
                    let mut rgb = [val.x, val.y, val.z];
                    if egui::color_picker::color_edit_button_rgb(ui, &mut rgb).changed() {
                        val.x = rgb[0];
                        val.y = rgb[1];
                        val.z = rgb[2];
                        changed = true;
                    }
                } else if let ParameterValue::Color4(val) = value {
                    let mut rgb = [val.x, val.y, val.z];
                    if egui::color_picker::color_edit_button_rgb(ui, &mut rgb).changed() {
                        val.x = rgb[0];
                        val.y = rgb[1];
                        val.z = rgb[2];
                        changed = true;
                    }
                }
            }
            ParameterUIType::FilePath { filters } => {
                if let ParameterValue::String(val) = value {
                    let btn_width = FILE_BROWSE_BUTTON_WIDTH;
                    let text_width = (avail_width - btn_width - spacing).max(MIN_COMPONENT_WIDTH * 10.0);
                    let mut r = {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(text_width, control_height), egui::Sense::hover());
                        if ui.is_rect_visible(rect) { sdf_box(ui.painter(), rect, PARAM_BACKGROUND_ROUNDING, ui.visuals().extreme_bg_color, 1.0, ui.visuals().widgets.inactive.bg_stroke.color, frame_id, screen); }
                        let mut child_ui = ui.new_child(
                            egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        { let w = &mut child_ui.style_mut().visuals.widgets; for s in [&mut w.inactive, &mut w.hovered, &mut w.active] { s.bg_fill = Color32::TRANSPARENT; s.bg_stroke.color = Color32::TRANSPARENT; } }
                        child_ui.add_sized(rect.size(), egui::TextEdit::singleline(val).frame(false))
                    };
                    #[cfg(not(target_arch = "wasm32"))]
                    if gpu_text_button(ui, "Browse...", FontId::proportional(14.0), egui::vec2(btn_width, control_height)).clicked() {
                        let mut d = rfd::FileDialog::new();
                        for f in filters { d = d.add_filter(f, &[f.as_str()]); }
                        if let Some(path) = d.pick_file() {
                            if let Some(v) = import_asset_file(&path, "textures") { *val = v; r.mark_changed(); changed = true; }
                        }
                    }
                    changed = r.changed() || changed;
                }
            }
            ParameterUIType::Separator => {
                ui.separator();
            }
            ParameterUIType::CurvePoints => {
                if let ParameterValue::Curve(curve_data) = value {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            gpu_label(ui, format!("Curve with {} points", curve_data.points.len()), FontId::proportional(14.0), ui.visuals().text_color());
                            if gpu_text_button(ui, "Clear", FontId::proportional(14.0), egui::vec2(0.0, ui.spacing().interact_size.y)).clicked() {
                                curve_data.points.clear();
                                changed = true;
                            }
                        });

                        // Curve Type Selector
                        ui.horizontal(|ui| {
                            gpu_label(ui, "Type:", FontId::proportional(14.0), ui.visuals().text_color());
                            egui::ComboBox::from_id_salt(format!("{}_type", _param_name))
                                .selected_text(format!("{:?}", curve_data.curve_type))
                                .show_ui(ui, |ui| {
                                    if ui.selectable_value(&mut curve_data.curve_type, crate::nodes::parameter::CurveType::Polygon, "Polygon").changed() {
                                        changed = true;
                                    }
                                    if ui.selectable_value(&mut curve_data.curve_type, crate::nodes::parameter::CurveType::Bezier, "Bezier").changed() {
                                        changed = true;
                                    }
                                    if ui.selectable_value(&mut curve_data.curve_type, crate::nodes::parameter::CurveType::Nurbs, "NURBS").changed() {
                                        changed = true;
                                    }
                                });
                        });

                        // Is Closed checkbox
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut curve_data.is_closed, "").changed() { changed = true; }
                            gpu_label(ui, "Close Curve", FontId::proportional(14.0), ui.visuals().text_color());
                        });
                    });
                }
            }
            ParameterUIType::UnitySpline => {
                if let ParameterValue::UnitySpline(s) = value {
                    use crate::libs::algorithms::algorithms_runtime::unity_spline::{BezierTangent, TangentMode};
                    use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::spline_selection_utility::is_selectable_tangent;
                    use crate::libs::algorithms::algorithms_runtime::unity_spline::SplineKnotIndex;
                    use crate::libs::algorithms::algorithms_runtime::unity_spline::{BezierKnot, MetaData, Spline};
                    use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::harness::{M4, Q4, SplineContainerSnapshot, V3};

                    ui.vertical(|ui| {
                        let mut reorder_req: Option<(usize, usize)> = None;
                        ui.horizontal(|ui| {
                            gpu_label(ui, "Spline Container", FontId::proportional(14.0), ui.visuals().text_color());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                #[cfg(not(target_arch = "wasm32"))]
                                {
                                    let err_id = ui.make_persistent_id(("unity_spline_snapshot_import_err", _param_name));
                                    let set_err = |ui: &egui::Ui, msg: String| ui.ctx().data_mut(|d| d.insert_persisted(err_id, msg));
                                    let import_raw = ui.small_button("Load Snapshot JSON").clicked();
                                    let import_xform = ui.small_button("Load JSON (Unity→Bevy)").clicked();
                                    let mut do_import = |ui: &egui::Ui, apply_unity_to_bevy: bool| {
                                        bevy::log::info!("[SplineImport] click param={} unity_to_bevy={}", _param_name, apply_unity_to_bevy);
                                        match rfd::FileDialog::new().add_filter("json", &["json"]).pick_file() {
                                            None => { bevy::log::info!("[SplineImport] cancelled"); }
                                            Some(path) => match std::fs::read_to_string(&path) {
                                                Err(e) => { bevy::log::info!("[SplineImport] read failed path={:?} err={}", path, e); set_err(ui, format!("Read failed: {e}")); }
                                                Ok(txt) => {
                                                    // Unity JSON may contain UTF-8 BOM (U+FEFF) at the start; serde_json rejects it.
                                                    let cleaned = txt.trim_start_matches('\u{feff}').trim();
                                                    match serde_json::from_str::<SplineContainerSnapshot>(cleaned) {
                                                        Err(e) => {
                                                            let head: String = cleaned.chars().take(16).collect();
                                                            bevy::log::info!("[SplineImport] parse failed path={:?} err={} head={:?}", path, e, head);
                                                            set_err(ui, format!("JSON parse failed: {e}"));
                                                        }
                                                        Ok(snap) => {
                                                            use cunning_kernel::coord::basis::{BasisId, map as basis_map};
                                                            let map = apply_unity_to_bevy.then(|| basis_map(BasisId::Unity, BasisId::InternalBevy)).flatten();

                                                            // Data-only import: copy knots/modes/links as-is (no sampling/baking).
                                                            let mut out = Vec::with_capacity(snap.splines.len());
                                                            for sp in &snap.splines {
                                                                let mut spline = Spline::default();
                                                                spline.closed = sp.closed;
                                                                spline.knots = sp.knots.iter().map(|k| {
                                                                    let p = bevy::math::Vec3::from_array(V3(k.position.0).0);
                                                                    let tin = bevy::math::Vec3::from_array(V3(k.tangent_in.0).0);
                                                                    let tout = bevy::math::Vec3::from_array(V3(k.tangent_out.0).0);
                                                                    let q = bevy::math::Quat::from_array(Q4(k.rotation.0).0);
                                                                    match map {
                                                                        Some(m) => BezierKnot { position: m.map_v3(p), tangent_in: m.map_v3(tin), tangent_out: m.map_v3(tout), rotation: m.map_q(q) },
                                                                        None => BezierKnot { position: p, tangent_in: tin, tangent_out: tout, rotation: q },
                                                                    }
                                                                }).collect();
                                                                spline.meta = sp.knots.iter().map(|k| MetaData::new(k.mode.0, k.tension)).collect();
                                                                out.push(spline);
                                                            }
                                                            s.splines = out;
                                                            s.links = Default::default();
                                                            let l2w = bevy::math::Mat4::from_cols_array_2d(&M4(snap.local_to_world.0).0);
                                                            s.local_to_world = match map { Some(m) => m.map_m4(l2w), None => l2w };
                                                            for g in snap.links {
                                                                if g.len() < 2 { continue; }
                                                                let base = SplineKnotIndex::new(g[0].0[0], g[0].0[1]);
                                                                for r in g.iter().skip(1) {
                                                                    let other = SplineKnotIndex::new(r.0[0], r.0[1]);
                                                                    s.link_knots(base, other);
                                                                }
                                                                s.set_linked_knot_position(base);
                                                            }
                                                            let knots_total: usize = s.splines.iter().map(|sp| sp.count()).sum();
                                                            let msg = format!("Imported: splines={} knots={} links={} unity_to_bevy={}", s.splines.len(), knots_total, s.links.all_links().len(), apply_unity_to_bevy);
                                                            bevy::log::info!("[SplineImport] ok path={:?} {}", path, msg);
                                                            set_err(ui, msg);
                                                            // mark the parameter changed
                                                            // SAFETY: we're still inside the parameter UI closure; this flag is handled by outer code.
                                                            // (No sampling/baking performed.)
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    };
                                    if import_raw { do_import(ui, false); changed = true; }
                                    if import_xform { do_import(ui, true); changed = true; }
                                }
                                if ui.small_button("+").clicked() { s.splines.push(Default::default()); changed = true; }
                                if ui.small_button("-").clicked() && !s.splines.is_empty() { s.remove_spline_at(s.splines.len() - 1); changed = true; }
                            });
                        });
                        let err_id = ui.make_persistent_id(("unity_spline_snapshot_import_err", _param_name));
                        if let Some(err) = ui.ctx().data_mut(|d| d.get_persisted::<String>(err_id)) {
                            if !err.is_empty() {
                                let ok = err.starts_with("Imported:");
                                ui.colored_label(if ok { Color32::LIGHT_GREEN } else { Color32::LIGHT_RED }, err);
                            }
                        }
                        ui.add_space(4.0);

                        for si in 0..s.splines.len() {
                            ui.push_id(("spline_container", _param_name, si), |ui| {
                                ui.collapsing(format!("Spline {}", si), |ui| {
                                    ui.horizontal(|ui| {
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if si + 1 < s.splines.len() && ui.small_button("↓").clicked() { reorder_req = Some((si, si + 1)); }
                                            if si > 0 && ui.small_button("↑").clicked() { reorder_req = Some((si, si - 1)); }
                                        });
                                    });
                                    ui.horizontal(|ui| {
                                        if ui.checkbox(&mut s.splines[si].closed, "").changed() { changed = true; }
                                        gpu_label(ui, "Closed", FontId::proportional(13.0), ui.visuals().text_color());
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.small_button("+").clicked() {
                                                s.splines[si].add(Default::default(), TangentMode::Broken, crate::libs::algorithms::algorithms_runtime::unity_spline::CATMULL_ROM_TENSION);
                                                changed = true;
                                            }
                                            if ui.small_button("-").clicked() && s.splines[si].count() > 0 {
                                                let n = s.splines[si].count();
                                                s.splines[si].remove_at(n - 1);
                                                changed = true;
                                            }
                                        });
                                    });

                                    let knot_count = s.splines[si].count();
                                    for ki in 0..knot_count {
                                        ui.push_id(("knot", ki), |ui| {
                                            ui.collapsing(format!("Knot [{}]", ki), |ui| {
                                                let mut knot = s.splines[si].knots[ki];
                                                let mut mode = s.splines[si].meta[ki].mode;

                                                ui.horizontal(|ui| {
                                                    gpu_label(ui, "Position", FontId::proportional(12.0), ui.visuals().text_color());
                                                    let mut p = knot.position;
                                                    let ch =
                                                        ui.add(egui::DragValue::new(&mut p.x).speed(0.1).prefix("X ")).changed() |
                                                        ui.add(egui::DragValue::new(&mut p.y).speed(0.1).prefix("Y ")).changed() |
                                                        ui.add(egui::DragValue::new(&mut p.z).speed(0.1).prefix("Z ")).changed();
                                                    if ch {
                                                        knot.position = p;
                                                        { let spline = &mut s.splines[si]; spline.set_knot(ki, knot, BezierTangent::Out); }
                                                        s.set_linked_knot_position(SplineKnotIndex::new(si as i32, ki as i32));
                                                        changed = true;
                                                    }
                                                });

                                                ui.horizontal(|ui| {
                                                    gpu_label(ui, "Rotation", FontId::proportional(12.0), ui.visuals().text_color());
                                                    let (mut rx, mut ry, mut rz) = knot.rotation.to_euler(bevy::math::EulerRot::XYZ);
                                                    let mut e = bevy::math::Vec3::new(rx.to_degrees(), ry.to_degrees(), rz.to_degrees());
                                                    let ch =
                                                        ui.add(egui::DragValue::new(&mut e.x).speed(1.0).prefix("X ")).changed() |
                                                        ui.add(egui::DragValue::new(&mut e.y).speed(1.0).prefix("Y ")).changed() |
                                                        ui.add(egui::DragValue::new(&mut e.z).speed(1.0).prefix("Z ")).changed();
                                                    if ch {
                                                        rx = e.x.to_radians(); ry = e.y.to_radians(); rz = e.z.to_radians();
                                                        knot.rotation = bevy::math::Quat::from_euler(bevy::math::EulerRot::XYZ, rx, ry, rz);
                                                        s.splines[si].set_knot(ki, knot, BezierTangent::Out);
                                                        changed = true;
                                                    }
                                                });

                                                ui.add_space(2.0);
                                                ui.horizontal(|ui| {
                                                    let is_lin = mode == TangentMode::Linear;
                                                    let is_auto = mode == TangentMode::AutoSmooth;
                                                    let is_bez = matches!(mode, TangentMode::Mirrored | TangentMode::Continuous | TangentMode::Broken);
                                                    let b_lin = ui.add(egui::Button::new("Linear").selected(is_lin));
                                                    let b_auto = ui.add(egui::Button::new("Auto").selected(is_auto));
                                                    let b_bez = ui.add(egui::Button::new("Bezier").selected(is_bez));
                                                    if b_lin.clicked() { s.splines[si].set_tangent_mode_no_notify(ki, TangentMode::Linear, BezierTangent::Out); changed = true; }
                                                    if b_auto.clicked() { s.splines[si].set_tangent_mode_no_notify(ki, TangentMode::AutoSmooth, BezierTangent::Out); changed = true; }
                                                    if b_bez.clicked() {
                                                        let m = if mode == TangentMode::AutoSmooth { TangentMode::Continuous } else { TangentMode::Mirrored };
                                                        s.splines[si].set_tangent_mode_no_notify(ki, m, BezierTangent::Out);
                                                        changed = true;
                                                    }
                                                    mode = s.splines[si].meta[ki].mode;
                                                });

                                                let tangents_mod = crate::libs::algorithms::algorithms_runtime::unity_spline::are_tangents_modifiable(mode);
                                                ui.horizontal(|ui| {
                                                    ui.label("Bezier");
                                                    let mut idx = match mode { TangentMode::Mirrored => 0, TangentMode::Continuous => 1, TangentMode::Broken => 2, _ => 0 };
                                                    let mut ch = false;
                                                    ui.add_enabled_ui(tangents_mod, |ui| {
                                                        egui::ComboBox::from_id_salt("bezier_mode")
                                                            .selected_text(match idx { 0 => "Mirrored", 1 => "Continuous", _ => "Broken" })
                                                            .show_ui(ui, |ui| {
                                                                ch |= ui.selectable_value(&mut idx, 0, "Mirrored").changed();
                                                                ch |= ui.selectable_value(&mut idx, 1, "Continuous").changed();
                                                                ch |= ui.selectable_value(&mut idx, 2, "Broken").changed();
                                                            });
                                                    });
                                                    if tangents_mod && ch {
                                                        let m = match idx { 1 => TangentMode::Continuous, 2 => TangentMode::Broken, _ => TangentMode::Mirrored };
                                                        s.splines[si].set_tangent_mode_no_notify(ki, m, BezierTangent::Out);
                                                        changed = true;
                                                    }
                                                });

                                                let in_sel = is_selectable_tangent(&s.splines[si], ki, BezierTangent::In);
                                                let out_sel = is_selectable_tangent(&s.splines[si], ki, BezierTangent::Out);
                                                knot = s.splines[si].knots[ki];
                                                let (in_len, out_len) = (knot.tangent_in.length(), knot.tangent_out.length());
                                                ui.horizontal(|ui| {
                                                    ui.label("In");
                                                    let mut v = if mode == TangentMode::Linear { 0.0 } else { -in_len };
                                                    let en = tangents_mod && in_sel;
                                                    if ui.add_enabled(en, egui::DragValue::new(&mut v).speed(0.1)).changed() {
                                                        s.splines[si].set_tangent_length(ki, BezierTangent::In, v.abs());
                                                        changed = true;
                                                    }
                                                });
                                                ui.horizontal(|ui| {
                                                    ui.label("Out");
                                                    let mut v = if mode == TangentMode::Linear { 0.0 } else { out_len };
                                                    let en = tangents_mod && out_sel;
                                                    if ui.add_enabled(en, egui::DragValue::new(&mut v).speed(0.1)).changed() {
                                                        s.splines[si].set_tangent_length(ki, BezierTangent::Out, v.abs());
                                                        changed = true;
                                                    }
                                                });
                                            });
                                        });
                                    }
                                });
                            });
                            ui.separator();
                        }

                        if let Some((from, to)) = reorder_req {
                            if s.reorder_spline(from, to) { changed = true; }
                        }
                    });
                }
            }
            ParameterUIType::Code => {
                if let ParameterValue::String(val) = value {
                     let w = ui.available_width();
                     let r = ui.add(
                         egui::TextEdit::multiline(val)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(w)
                            .desired_rows(10)
                     );
                     if r.changed() {
                         changed = true;
                     }
                }
            }
            _ => {}
        }
    });
    changed
}
