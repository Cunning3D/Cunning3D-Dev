//! Exports tab: host-selectable intermediate outputs/params.
use crate::cunning_core::cda::asset::{CdaExport, CdaExportKind, CdaExportsMode};
use crate::cunning_core::cda::CDAAsset;
use crate::nodes::parameter::ParameterValue;
use crate::nodes::structs::NodeType;
use bevy_egui::egui::{self, Color32, RichText, Ui};

fn clamp(s: &str, max: usize) -> String {
    if s.chars().count() <= max { return s.to_string(); }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn node_label(asset: &CDAAsset, id: crate::nodes::structs::NodeId) -> String {
    asset
        .inner_graph
        .nodes
        .get(&id)
        .map(|n| {
            let name = n.name.trim();
            let ty = n.node_type.name();
            if name.is_empty() { ty.to_string() } else { format!("{name} ({ty})") }
        })
        .unwrap_or_else(|| format!("{id}"))
}

fn normalize_orders(xs: &mut [CdaExport]) { for (i, e) in xs.iter_mut().enumerate() { e.order = i as i32; } }

fn unique_name(asset: &CDAAsset, base: &str) -> String {
    if !asset.exports.iter().any(|e| e.name == base) { return base.to_string(); }
    for i in 1..10_000 {
        let n = format!("{base}_{i}");
        if !asset.exports.iter().any(|e| e.name == n) { return n; }
    }
    format!("{base}_x")
}

fn is_vec_like(v: &ParameterValue) -> Option<&'static [&'static str]> {
    match v {
        ParameterValue::Vec2(_) => Some(&["x", "y"]),
        ParameterValue::Vec3(_) | ParameterValue::Color(_) => Some(&["x", "y", "z"]),
        ParameterValue::Vec4(_) | ParameterValue::Color4(_) => Some(&["x", "y", "z", "w"]),
        _ => None,
    }
}

pub fn draw(ui: &mut Ui, asset: &mut CDAAsset) {
    ui.label("Host-selectable exports (advanced CDA evaluation).");
    ui.add_space(6.0);

    ui.horizontal(|ui| {
        ui.label("Mode:");
        let bb = asset.exports_mode == CdaExportsMode::BlackBox;
        if ui.radio(bb, "BlackBox").clicked() { asset.exports_mode = CdaExportsMode::BlackBox; }
        if ui.radio(!bb, "Advanced").clicked() { asset.exports_mode = CdaExportsMode::Advanced; }
    });
    if asset.exports_mode == CdaExportsMode::BlackBox {
        ui.add_space(4.0);
        ui.label(RichText::new("BlackBox: only declared CDA outputs are used at runtime.").color(Color32::GRAY));
        ui.add_space(10.0);
    }

    let mut nodes: Vec<_> = asset
        .inner_graph
        .nodes
        .iter()
        .filter_map(|(id, n)| match &n.node_type { NodeType::CDAInput(_) | NodeType::CDAOutput(_) => None, _ => Some(*id) })
        .collect();
    nodes.sort();

    // Cache labels to avoid borrowing `asset` immutably inside `exports.iter_mut()` UI closures.
    let mut node_labels = std::collections::HashMap::with_capacity(asset.inner_graph.nodes.len());
    for (id, n) in asset.inner_graph.nodes.iter() {
        let name = n.name.trim();
        let ty = n.node_type.name();
        let s = if name.is_empty() { ty.to_string() } else { format!("{name} ({ty})") };
        node_labels.insert(*id, s);
    }

    let mut add_node: Option<crate::nodes::structs::NodeId> = None;
    let mut add_kind_is_param = false;
    let mut add_param: Option<String> = None;
    let mut add_channel: Option<u32> = None;

    egui::CollapsingHeader::new("Add export")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Node:");
                egui::ComboBox::from_id_salt("cda_exports_add_node")
                    .selected_text("(Select node)")
                    .show_ui(ui, |ui| {
                        for id in &nodes {
                            if ui.selectable_label(false, clamp(&node_label(asset, *id), 48)).clicked() {
                                add_node = Some(*id);
                            }
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Kind:");
                ui.radio_value(&mut add_kind_is_param, false, "NodeOutput");
                ui.radio_value(&mut add_kind_is_param, true, "NodeParam");
            });
            if add_kind_is_param {
                if let Some(nid) = add_node {
                    let params = asset.list_node_params(nid);
                    ui.horizontal(|ui| {
                        ui.label("Param:");
                        egui::ComboBox::from_id_salt("cda_exports_add_param")
                            .selected_text("(Select param)")
                            .show_ui(ui, |ui| {
                                for (name, v) in &params {
                                    let txt = if is_vec_like(v).is_some() { format!("{name} (vec)") } else { (*name).to_string() };
                                    if ui.selectable_label(false, clamp(&txt, 40)).clicked() {
                                        add_param = Some((*name).to_string());
                                        add_channel = None;
                                    }
                                }
                            });
                    });
                    if let Some(pn) = add_param.as_deref() {
                        if let Some((_n, v)) = params.into_iter().find(|(n, _)| *n == pn) {
                            if let Some(chs) = is_vec_like(v) {
                                ui.horizontal(|ui| {
                                    ui.label("Channel:");
                                    let mut sel = add_channel.map(|c| c as i32).unwrap_or(-1);
                                    egui::ComboBox::from_id_salt("cda_exports_add_channel")
                                        .selected_text(if sel < 0 { "all" } else { chs.get(sel as usize).copied().unwrap_or("?") })
                                        .show_ui(ui, |ui| {
                                            if ui.selectable_label(sel < 0, "all").clicked() { add_channel = None; }
                                            for (i, c) in chs.iter().enumerate() {
                                                if ui.selectable_label(sel == i as i32, *c).clicked() { add_channel = Some(i as u32); }
                                            }
                                        });
                                });
                            }
                        }
                    }
                }
            }

            let can_add = add_node.is_some() && (!add_kind_is_param || add_param.is_some());
            if ui.add_enabled(can_add, egui::Button::new("Add")).clicked() {
                let nid = add_node.unwrap();
                let base = if add_kind_is_param {
                    format!("param_{}_{}", nid.to_string().split('-').next().unwrap_or("n"), add_param.clone().unwrap())
                } else {
                    format!("out_{}", nid.to_string().split('-').next().unwrap_or("n"))
                };
                let name = unique_name(asset, &base);
                let label = node_label(asset, nid);
                let kind = if add_kind_is_param {
                    CdaExportKind::NodeParam { node_id: nid, param: add_param.unwrap(), channel: add_channel }
                } else {
                    CdaExportKind::NodeOutput { node_id: nid }
                };
                asset.exports.push(CdaExport { name, label, order: asset.exports.len() as i32, kind });
                normalize_orders(&mut asset.exports);
            }
        });

    ui.add_space(10.0);
    ui.separator();
    ui.add_space(6.0);

    ui.label(RichText::new("Exports list").strong());
    if asset.exports.is_empty() {
        ui.label(RichText::new("(No exports defined)").color(Color32::GRAY));
        return;
    }

    let mut remove: Option<usize> = None;
    let mut up: Option<usize> = None;
    let mut down: Option<usize> = None;
    let len = asset.exports.len();
    for (i, e) in asset.exports.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("#{i}"));
            ui.label(match &e.kind {
                CdaExportKind::NodeOutput { .. } => RichText::new("NodeOutput").color(Color32::LIGHT_BLUE),
                CdaExportKind::NodeParam { .. } => RichText::new("NodeParam").color(Color32::LIGHT_GREEN),
            });
            ui.label("Name:");
            ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(160.0));
            ui.label("Label:");
            ui.add(egui::TextEdit::singleline(&mut e.label).desired_width(200.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("✕").clicked() { remove = Some(i); }
                if i + 1 < len && ui.small_button("↓").clicked() { down = Some(i); }
                if i > 0 && ui.small_button("↑").clicked() { up = Some(i); }
            });
        });
        ui.horizontal(|ui| {
            ui.add_space(52.0);
            match &e.kind {
                CdaExportKind::NodeOutput { node_id } => ui.label(format!(
                    "Node: {}",
                    clamp(node_labels.get(node_id).map(|s| s.as_str()).unwrap_or("?"), 64)
                )),
                CdaExportKind::NodeParam { node_id, param, channel } => {
                    let ch = channel.map(|c| format!(" ch{c}")).unwrap_or_default();
                    ui.label(format!(
                        "Node: {}  Param: {}{}",
                        clamp(node_labels.get(node_id).map(|s| s.as_str()).unwrap_or("?"), 48),
                        param,
                        ch
                    ))
                }
            }
        });
        ui.add_space(4.0);
    }
    if let Some(i) = remove { asset.exports.remove(i); normalize_orders(&mut asset.exports); }
    else if let Some(i) = up { asset.exports.swap(i, i - 1); normalize_orders(&mut asset.exports); }
    else if let Some(i) = down { asset.exports.swap(i, i + 1); normalize_orders(&mut asset.exports); }
}

