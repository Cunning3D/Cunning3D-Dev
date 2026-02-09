use crate::cunning_core::cda::asset::{CdaCoverlayUnit, CdaHudUnit};
use crate::cunning_core::cda::CDAAsset;
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::node_editor_settings::NodeEditorSettings;
use crate::nodes::structs::NodeType;
use crate::ui::prepare_generic_node;
use bevy_egui::egui::{self, Ui};

fn node_label(asset: &CDAAsset, id: crate::nodes::structs::NodeId) -> String {
    asset
        .inner_graph
        .nodes
        .get(&id)
        .map(|n| {
            let name = n.name.trim();
            let ty = n.node_type.name();
            if name.is_empty() {
                ty.to_string()
            } else {
                format!("{name} ({ty})")
            }
        })
        .unwrap_or_else(|| format!("{id}"))
}

fn candidates(
    asset: &CDAAsset,
    reg: &NodeRegistry,
    want_coverlay: bool,
) -> Vec<crate::nodes::structs::NodeId> {
    let map = reg.nodes.read().unwrap();
    let mut out: Vec<_> = asset
        .inner_graph
        .nodes
        .iter()
        .filter_map(|(id, n)| {
            match &n.node_type {
                NodeType::CDAInput(_) | NodeType::CDAOutput(_) => return None,
                _ => {}
            }
            let key = n.node_type.name().to_string();
            let Some(desc) = map.get(&key) else {
                return None;
            };
            if want_coverlay {
                if !desc.coverlay_kinds.is_empty() { return Some(*id); }
                let Some(f) = desc.interaction_factory.as_ref() else { return None; };
                let i = f();
                i.has_coverlay().then_some(*id)
            } else {
                let Some(f) = desc.interaction_factory.as_ref() else { return None; };
                let i = f();
                i.has_hud().then_some(*id)
            }
        })
        .collect();
    out.sort();
    out
}

pub fn draw(ui: &mut Ui, asset: &mut CDAAsset, node_registry: &NodeRegistry, settings: &NodeEditorSettings) {
    ui.label("Overlay exposure");
    ui.add_space(6.0);

    let name_by_id: std::collections::HashMap<crate::nodes::structs::NodeId, String> = asset
        .inner_graph
        .nodes
        .keys()
        .copied()
        .map(|id| (id, node_label(asset, id)))
        .collect();
    let nl = |id: crate::nodes::structs::NodeId| -> String {
        name_by_id
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("{id}"))
    };

    // HUD (single-select)
    egui::CollapsingHeader::new("HUD (single-select)")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Add:");
                let mut add_id: Option<crate::nodes::structs::NodeId> = None;
                egui::ComboBox::from_id_salt("cda_overlay_hud_add")
                    .selected_text("(Select node)")
                    .show_ui(ui, |ui| {
                        for id in candidates(asset, node_registry, false) {
                            let already = asset.hud_units.iter().any(|u| u.node_id == id);
                            let txt = node_label(asset, id);
                            if ui
                                .add_enabled(!already, egui::Button::selectable(false, txt))
                                .clicked()
                            {
                                add_id = Some(id);
                            }
                        }
                    });
                if let Some(id) = add_id {
                    asset.hud_units.push(CdaHudUnit {
                        node_id: id,
                        label: nl(id),
                        order: asset.hud_units.len() as i32,
                        is_default: asset.hud_units.is_empty(),
                    });
                }
            });

            if asset.hud_units.is_empty() {
                ui.label("(No HUD units exposed)");
                return;
            }

            let mut remove: Option<usize> = None;
            let mut move_up: Option<usize> = None;
            let mut move_down: Option<usize> = None;
            let mut set_default: Option<usize> = None;
            let hud_len = asset.hud_units.len();
            for (i, u) in asset.hud_units.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let on = u.is_default;
                    if ui.radio(on, "").clicked() {
                        set_default = Some(i);
                    }
                    ui.label(nl(u.node_id));
                    ui.add_space(8.0);
                    ui.label("Label:");
                    ui.add(egui::TextEdit::singleline(&mut u.label).desired_width(220.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").clicked() {
                            remove = Some(i);
                        }
                        if i + 1 < hud_len && ui.small_button("↓").clicked() {
                            move_down = Some(i);
                        }
                        if i > 0 && ui.small_button("↑").clicked() {
                            move_up = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = remove {
                asset.hud_units.remove(i);
            } else if let Some(i) = move_up {
                asset.hud_units.swap(i, i - 1);
            } else if let Some(i) = move_down {
                asset.hud_units.swap(i, i + 1);
            }
            if let Some(i) = set_default {
                for (j, u) in asset.hud_units.iter_mut().enumerate() {
                    u.is_default = j == i;
                }
            }
            if asset.hud_units.iter().all(|u| !u.is_default) {
                if let Some(f) = asset.hud_units.first_mut() {
                    f.is_default = true;
                }
            }
        });

    ui.add_space(10.0);

    // Coverlay (multi-select)
    egui::CollapsingHeader::new("Coverlay (multi-select)")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("➕ New Coverlay Panel").clicked() {
                    let mut min = egui::pos2(f32::MAX, f32::MAX);
                    let mut max = egui::pos2(f32::MIN, f32::MIN);
                    let mut any = false;
                    for n in asset.inner_graph.nodes.values() {
                        match &n.node_type {
                            NodeType::CDAInput(_) | NodeType::CDAOutput(_) => continue,
                            _ => {}
                        }
                        any = true;
                        min.x = min.x.min(n.position.x);
                        min.y = min.y.min(n.position.y);
                        max.x = max.x.max(n.position.x);
                        max.y = max.y.max(n.position.y);
                    }
                    let pos = if any {
                        egui::pos2(max.x + 260.0, min.y)
                    } else {
                        egui::pos2(200.0, 200.0)
                    };
                    let node = prepare_generic_node(node_registry, settings, pos, "Coverlay Panel");
                    let id = node.id;
                    asset.inner_graph.nodes.insert(id, node);
                    if !asset.coverlay_units.iter().any(|u| u.node_id == id) {
                        asset.coverlay_units.push(CdaCoverlayUnit {
                            node_id: id,
                            label: "Controls".to_string(),
                            icon: Some("🎛".to_string()),
                            order: asset.coverlay_units.len() as i32,
                            default_on: true,
                        });
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Add:");
                let mut add_id: Option<crate::nodes::structs::NodeId> = None;
                egui::ComboBox::from_id_salt("cda_overlay_coverlay_add")
                    .selected_text("(Select node)")
                    .show_ui(ui, |ui| {
                        for id in candidates(asset, node_registry, true) {
                            let already = asset.coverlay_units.iter().any(|u| u.node_id == id);
                            let txt = node_label(asset, id);
                            if ui
                                .add_enabled(!already, egui::Button::selectable(false, txt))
                                .clicked()
                            {
                                add_id = Some(id);
                            }
                        }
                    });
                if let Some(id) = add_id {
                    asset.coverlay_units.push(CdaCoverlayUnit {
                        node_id: id,
                        label: nl(id),
                        icon: None,
                        order: asset.coverlay_units.len() as i32,
                        default_on: true,
                    });
                }
            });

            if asset.coverlay_units.is_empty() {
                ui.label("(No coverlay units exposed)");
                return;
            }

            let mut remove: Option<usize> = None;
            let mut move_up: Option<usize> = None;
            let mut move_down: Option<usize> = None;
            let cov_len = asset.coverlay_units.len();
            for (i, u) in asset.coverlay_units.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut u.default_on, "");
                    ui.label(nl(u.node_id));
                    ui.add_space(8.0);
                    ui.label("Label:");
                    ui.add(egui::TextEdit::singleline(&mut u.label).desired_width(180.0));
                    ui.label("Icon:");
                    let mut icon_txt = u.icon.clone().unwrap_or_default();
                    if ui
                        .add(egui::TextEdit::singleline(&mut icon_txt).desired_width(60.0))
                        .changed()
                    {
                        u.icon = if icon_txt.trim().is_empty() {
                            None
                        } else {
                            Some(icon_txt.trim().to_string())
                        };
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").clicked() {
                            remove = Some(i);
                        }
                        if i + 1 < cov_len && ui.small_button("↓").clicked() {
                            move_down = Some(i);
                        }
                        if i > 0 && ui.small_button("↑").clicked() {
                            move_up = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = remove {
                asset.coverlay_units.remove(i);
            } else if let Some(i) = move_up {
                asset.coverlay_units.swap(i, i - 1);
            } else if let Some(i) = move_down {
                asset.coverlay_units.swap(i, i + 1);
            }
        });
}
