//! Params tab: 3-column layout + drag-and-drop binding
use super::super::drag_payload::{ParamDragPayload, ParamTypeHint, PromotedParamDragPayload};
use super::super::editor_state::CDAEditorState;
use super::binding_card;
use crate::cunning_core::cda::{
    promoted_param::{ParamBinding, ParamChannel, PromotedParamType},
    CDAAsset, PromotedParam,
};
use bevy_egui::egui::{self, Color32, DragAndDrop, Id, RichText, Ui};
use uuid::Uuid;

pub fn draw(ui: &mut Ui, asset: &mut CDAAsset, cda_state: &mut CDAEditorState) {
    ui.columns(3, |cols| {
        draw_type_column(&mut cols[0], asset);
        draw_params_column(&mut cols[1], asset, cda_state);
        draw_binding_column(&mut cols[2], asset, cda_state);
    });
    ui.add_space(16.0);
    ui.separator();
    draw_internal_params(ui, asset, cda_state);
}

/// Left column: create types
fn draw_type_column(ui: &mut Ui, asset: &mut CDAAsset) {
    egui::Frame::NONE
        .fill(Color32::from_rgb(30, 30, 35))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Create Type").strong());
                ui.add_space(8.0);
                for (label, ptype) in [
                    (
                        "Float",
                        PromotedParamType::Float {
                            min: 0.0,
                            max: 10.0,
                            logarithmic: false,
                        },
                    ),
                    ("Int", PromotedParamType::Int { min: 0, max: 100 }),
                    ("Bool", PromotedParamType::Bool),
                    ("Toggle", PromotedParamType::Toggle),
                    ("Vec2", PromotedParamType::Vec2),
                    ("Vec3", PromotedParamType::Vec3),
                    ("Color", PromotedParamType::Color { has_alpha: false }),
                    ("String", PromotedParamType::String),
                    ("Button", PromotedParamType::Button),
                    ("Dropdown", PromotedParamType::Dropdown { items: vec![] }),
                    ("Angle", PromotedParamType::Angle),
                    ("File", PromotedParamType::FilePath { filters: vec![] }),
                ] {
                    if ui.button(label).clicked() {
                        let param = create_default_param(label, ptype);
                        asset.add_promoted_param(param);
                    }
                }
            });
        });
}

fn create_promoted_param_from_payload(payload: &ParamDragPayload) -> PromotedParam {
    let ptype = match payload.param_type {
        ParamTypeHint::Float => PromotedParamType::Float {
            min: 0.0,
            max: 10.0,
            logarithmic: false,
        },
        ParamTypeHint::Int => PromotedParamType::Int { min: 0, max: 100 },
        ParamTypeHint::Bool => PromotedParamType::Bool,
        ParamTypeHint::Vec2 => PromotedParamType::Vec2,
        ParamTypeHint::Vec3 => PromotedParamType::Vec3,
        ParamTypeHint::Vec4 => PromotedParamType::Vec4,
        ParamTypeHint::Color => PromotedParamType::Color { has_alpha: false },
        ParamTypeHint::String => PromotedParamType::String,
        ParamTypeHint::Unknown => PromotedParamType::Float {
            min: 0.0,
            max: 1.0,
            logarithmic: false,
        }, // Fallback
    };

    let mut param = create_default_param(&payload.param_name, ptype);

    // Auto-bind
    // If the drag source doesn't specify a channel (channel_index == None) and the target is multi-channel (e.g. Vec3),
    // bindings should ideally be matched channel-by-channel.
    // For simplicity, when the source has no explicit channel we bind source channel 0 to target channel 0, and so on.
    // However, ParamDragPayload without channel_index usually means the whole parameter.
    // Our ParamBinding needs target_channel (SOURCE channel index).

    if let Some(ch_idx) = payload.channel_index {
        // Bind one channel to one channel (default: first)
        if let Some(ch) = param.channels.get_mut(0) {
            ch.bindings.push(ParamBinding {
                target_node: payload.source_node,
                target_param: payload.param_name.clone(),
                target_channel: Some(ch_idx),
            });
        }
    } else {
        // Bind the whole parameter.
        // Try to infer: if the target has multiple channels, try binding corresponding source channels.
        // ParamBinding.target_channel is the source channel index.
        for (i, ch) in param.channels.iter_mut().enumerate() {
            ch.bindings.push(ParamBinding {
                target_node: payload.source_node,
                target_param: payload.param_name.clone(),
                target_channel: Some(i), // Assume the source also has channel i
            });
        }
    }

    param
}

fn split_group(g: &str) -> Vec<&str> {
    g.split('/')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect()
}

fn list_children_groups(asset: &CDAAsset, prefix: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in &asset.promoted_params {
        let segs: Vec<&str> = split_group(&p.group);
        if prefix.len() > segs.len() {
            continue;
        }
        if !prefix
            .iter()
            .enumerate()
            .all(|(i, s)| segs.get(i).map(|x| *x == s.as_str()).unwrap_or(false))
        {
            continue;
        }
        if let Some(n) = segs.get(prefix.len()) {
            if !out.iter().any(|x| x == n) {
                out.push((*n).to_string());
            }
        }
    }
    out.sort();
    out
}

fn match_group_path(group: &str, path: &[String]) -> bool {
    if path.is_empty() {
        return true;
    }
    let segs: Vec<&str> = split_group(group);
    if path.len() > segs.len() {
        return false;
    }
    path.iter()
        .enumerate()
        .all(|(i, s)| segs.get(i).map(|x| *x == s.as_str()).unwrap_or(false))
}

fn ensure_folder_path_valid(asset: &CDAAsset, path: &mut Vec<String>) {
    while !path.is_empty()
        && list_children_groups(asset, &path[..path.len().saturating_sub(1)])
            .iter()
            .all(|c| c != path.last().unwrap())
    {
        path.pop();
    }
    while !list_children_groups(asset, path).is_empty()
        && list_children_groups(asset, path).iter().all(|_| false)
    {
        break;
    }
}

fn normalize_orders(asset: &mut CDAAsset) {
    for (i, p) in asset.promoted_params.iter_mut().enumerate() {
        p.order = i as i32;
    }
}

fn unique_promoted_name(asset: &CDAAsset, base: &str) -> String {
    if !asset.promoted_params.iter().any(|p| p.name == base) {
        return base.to_string();
    }
    for i in 1..10_000 {
        let n = format!("{}_{}", base, i);
        if !asset.promoted_params.iter().any(|p| p.name == n) {
            return n;
        }
    }
    format!("{}_copy", base)
}

fn clone_param_for_paste(asset: &CDAAsset, p: &PromotedParam) -> PromotedParam {
    let mut c = p.clone();
    c.id = Uuid::new_v4();
    c.name = unique_promoted_name(asset, &format!("{}_copy", p.name));
    c.label = format!("{} Copy", p.label);
    c
}

/// Middle column: promoted parameter list
fn draw_params_column(ui: &mut Ui, asset: &mut CDAAsset, cda_state: &mut CDAEditorState) {
    egui::Frame::NONE
        .fill(Color32::from_rgb(35, 35, 40))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.label(RichText::new("Exposed Parameters").strong());
            ui.add_space(8.0);

            ensure_folder_path_valid(asset, &mut cda_state.params_folder_path);
            ui.horizontal_wrapped(|ui| {
                let all_sel = cda_state.params_folder_path.is_empty();
                if ui.selectable_label(all_sel, "All").clicked() {
                    cda_state.params_folder_path.clear();
                }
                for g in list_children_groups(asset, &[]) {
                    let sel = cda_state
                        .params_folder_path
                        .get(0)
                        .map(|s| s == &g)
                        .unwrap_or(false);
                    if ui.selectable_label(sel, &g).clicked() {
                        cda_state.params_folder_path.truncate(0);
                        cda_state.params_folder_path.push(g);
                    }
                }
            });
            let mut prefix: Vec<String> = Vec::new();
            for depth in 0..cda_state.params_folder_path.len() {
                prefix = cda_state.params_folder_path[..=depth].to_vec();
                let children = list_children_groups(asset, &prefix);
                if children.is_empty() {
                    break;
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label("›");
                    for g in children {
                        let sel = cda_state
                            .params_folder_path
                            .get(depth + 1)
                            .map(|s| s == &g)
                            .unwrap_or(false);
                        if ui.selectable_label(sel, &g).clicked() {
                            cda_state.params_folder_path.truncate(depth + 1);
                            cda_state.params_folder_path.push(g);
                        }
                    }
                });
            }
            ui.add_space(6.0);

            let has_drag = DragAndDrop::has_payload_of_type::<ParamDragPayload>(ui.ctx());
            let dragging = DragAndDrop::payload::<ParamDragPayload>(ui.ctx());
            if let Some(p) = &dragging {
                ui.add_sized(
                    [ui.available_width(), 0.0],
                    egui::Label::new(
                        RichText::new(format!("Drag: {}", p.display_label))
                            .small()
                            .color(Color32::from_rgb(180, 200, 255)),
                    )
                    .wrap(),
                );
                ui.add_space(4.0);
            }
            let row_h = ui.spacing().interact_size.y.max(28.0);
            let drop_frame = egui::Frame::NONE; // The whole column is a create zone: release = auto-create + bind

            let (inner, payload) = ui.dnd_drop_zone::<ParamDragPayload, _>(drop_frame, |ui| {
                let mut to_remove = None;
                let mut to_copy: Option<Uuid> = None;
                let mut to_paste_after: Option<usize> = None;
                let mut to_reorder: Option<(Uuid, usize)> = None;
                let inner = egui::ScrollArea::vertical()
                    .max_height(ui.available_height())
                    .show(ui, |ui| {
                        let idxs: Vec<usize> = asset
                            .promoted_params
                            .iter()
                            .enumerate()
                            .filter(|(_, p)| {
                                match_group_path(&p.group, &cda_state.params_folder_path)
                            })
                            .map(|(i, _)| i)
                            .collect();
                        for i in idxs {
                            let param = &asset.promoted_params[i];
                            let is_selected = cda_state.selected_param_id == Some(param.id);
                            let mut delete_clicked = false;
                            let frame = egui::Frame::NONE
                                .fill(if is_selected {
                                    Color32::from_rgb(60, 80, 120)
                                } else {
                                    Color32::from_rgb(45, 45, 50)
                                })
                                .corner_radius(4.0)
                                .inner_margin(egui::Margin::same(6));
                            let (row, dropped) =
                                ui.dnd_drop_zone::<PromotedParamDragPayload, _>(frame, |ui| {
                                    ui.set_height(row_h);
                                    ui.with_layout(
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.label(RichText::new("≡").weak());
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&param.label).strong(),
                                                )
                                                .truncate(),
                                            );
                                            ui.label(
                                                RichText::new(format!(
                                                    "({})",
                                                    param.param_type.display_name()
                                                ))
                                                .weak()
                                                .small(),
                                            );
                                            let binding_count = param.total_bindings();
                                            if binding_count > 0 {
                                                ui.label(
                                                    RichText::new(format!("●{}", binding_count))
                                                        .small()
                                                        .color(Color32::LIGHT_GREEN),
                                                );
                                            }
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if ui.small_button("×").clicked() {
                                                        delete_clicked = true;
                                                        to_remove = Some(i);
                                                    }
                                                },
                                            );
                                        },
                                    );
                                });
                            if let Some(d) = dropped {
                                if d.param_id != param.id {
                                    to_reorder = Some((d.param_id, i));
                                }
                            }
                            let click = ui.interact(
                                row.response.rect,
                                Id::new(("cda_promoted_param_row", param.id)),
                                egui::Sense::click(),
                            );
                            if click.clicked() && !delete_clicked {
                                cda_state.selected_param_id = Some(param.id);
                            }
                            click.context_menu(|ui| {
                                if ui.button("Copy").clicked() {
                                    to_copy = Some(param.id);
                                    ui.close_kind(egui::UiKind::Menu);
                                }
                                if cda_state.param_clipboard.is_some()
                                    && ui.button("Paste After").clicked()
                                {
                                    to_paste_after = Some(i);
                                    ui.close_kind(egui::UiKind::Menu);
                                }
                            });
                            ui.add_space(4.0);
                        }
                        if asset.promoted_params.is_empty() {
                            ui.add_space(6.0);
                            ui.label(RichText::new("Drag here: release to create a new parameter").weak());
                        }
                    });
                if let Some((drag_id, target_i)) = to_reorder {
                    if let Some(from_i) = asset.promoted_params.iter().position(|p| p.id == drag_id)
                    {
                        if from_i != target_i {
                            let p = asset.promoted_params.remove(from_i);
                            let ins = if from_i < target_i {
                                target_i.saturating_sub(1)
                            } else {
                                target_i
                            };
                            asset.promoted_params.insert(ins, p);
                            normalize_orders(asset);
                        }
                    }
                }
                if let Some(id) = to_copy {
                    if let Some(p) = asset.promoted_params.iter().find(|p| p.id == id) {
                        cda_state.param_clipboard = Some(p.clone());
                    }
                }
                if let Some(i) = to_paste_after {
                    if let Some(c) = cda_state.param_clipboard.clone() {
                        let ins = (i + 1).min(asset.promoted_params.len());
                        asset
                            .promoted_params
                            .insert(ins, clone_param_for_paste(asset, &c));
                        normalize_orders(asset);
                    }
                }
                if let Some(i) = to_remove {
                    asset.promoted_params.remove(i);
                    cda_state.selected_param_id = None;
                    normalize_orders(asset);
                }
                inner
            });

            if has_drag {
                let rect = inner.response.rect.intersect(ui.clip_rect());
                if ui
                    .input(|i| i.pointer.interact_pos())
                    .map_or(false, |p| rect.contains(p))
                {
                    ui.painter().rect_stroke(
                        rect,
                        4.0,
                        egui::Stroke::new(2.0, Color32::from_rgb(100, 200, 100)),
                        egui::StrokeKind::Inside,
                    );
                }
            }

            if let Some(p) = payload {
                asset.add_promoted_param(create_promoted_param_from_payload(&*p));
                normalize_orders(asset);
            }
        });
}

/// Right column: selected parameter details
fn draw_binding_column(ui: &mut Ui, asset: &mut CDAAsset, cda_state: &mut CDAEditorState) {
    egui::Frame::NONE
        .fill(Color32::from_rgb(35, 35, 40))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            if let Some(param_id) = cda_state.selected_param_id {
                if let Some(param) = asset.promoted_params.iter_mut().find(|p| p.id == param_id) {
                    binding_card::draw(ui, param, cda_state);
                } else {
                    ui.label("Parameter not found");
                    cda_state.selected_param_id = None;
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Select a parameter to view details").weak());
                });
            }
        });
}

/// Render internal node parameter list (drag source)
fn draw_internal_params(ui: &mut Ui, asset: &CDAAsset, _cda_state: &mut CDAEditorState) {
    ui.label(RichText::new("Internal Node Parameters (drag upward to bind)").strong());
    ui.add_space(4.0);

    egui::ScrollArea::vertical()
        .max_height(200.0)
        .show(ui, |ui| {
            fn clamp(s: &str, max: usize) -> String {
                if s.chars().count() <= max {
                    return s.to_string();
                }
                let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
                out.push('…');
                out
            }
            for (node_id, name, type_name) in asset.list_internal_nodes() {
                let title = format!("{} ({})", clamp(&name, 24), clamp(&type_name, 28));
                egui::CollapsingHeader::new(title)
                    .default_open(false)
                    .show(ui, |ui| {
                        for (param_name, value) in asset.list_node_params(node_id) {
                            let is_bound = asset.is_param_bound(node_id, param_name, None);
                            let type_hint = ParamTypeHint::from_value(value);
                            let item_id = Id::new((node_id, param_name));

                            ui.dnd_drag_source(
                                item_id,
                                ParamDragPayload::new(node_id, param_name, value),
                                |ui| {
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("≡").color(Color32::GRAY));
                                            ui.add_sized(
                                                [ui.available_width(), 0.0],
                                                egui::Label::new(param_name).truncate(),
                                            );
                                        });
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label(
                                                RichText::new(format!("({:?})", type_hint))
                                                    .weak()
                                                    .small(),
                                            );
                                            if is_bound {
                                                ui.label(
                                                    RichText::new("●")
                                                        .color(Color32::LIGHT_GREEN)
                                                        .small(),
                                                );
                                                if let Some(target) =
                                                    asset.get_binding_target(node_id, param_name)
                                                {
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(format!("→ {}", target))
                                                                .small()
                                                                .color(Color32::LIGHT_BLUE),
                                                        )
                                                        .truncate(),
                                                    )
                                                    .on_hover_text(target);
                                                }
                                            }
                                        });
                                    })
                                },
                            );
                        }
                    });
            }
        });
}

fn create_default_param(name: &str, ptype: PromotedParamType) -> PromotedParam {
    let channel_names = ptype.channel_names();
    PromotedParam {
        id: Uuid::new_v4(),
        name: format!(
            "param_{}",
            Uuid::new_v4().to_string().split('-').next().unwrap()
        ),
        label: format!("New {}", name),
        group: "Main".to_string(),
        order: 0,
        param_type: ptype,
        ui_config: Default::default(),
        channels: channel_names
            .into_iter()
            .map(|n| ParamChannel::new(n, 0.0))
            .collect(),
    }
}
