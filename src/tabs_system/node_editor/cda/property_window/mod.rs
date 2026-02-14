//! CDA property window
pub mod basic_tab;
pub mod binding_card;
pub mod exports_tab;
pub mod help_tab;
pub mod icon_tab;
pub mod overlay_tab;
pub mod params_tab;

use super::editor_state::{CDAEditorState, CDAPropertyTab};
use crate::cunning_core::cda::library::global_cda_library;
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::node_editor_settings::NodeEditorSettings;
use crate::nodes::cda::cda_node::sync_asset_io_nodes;
use crate::nodes::structs::{NodeGraph, NodeType};
use bevy_egui::egui::{self, Color32, RichText, Window};

/// Draw CDA property window
pub fn draw_property_window(
    ctx: &egui::Context,
    graph: &mut NodeGraph,
    cda_state: &mut CDAEditorState,
    settings: &NodeEditorSettings,
    node_registry: &NodeRegistry,
) {
    if !cda_state.property_window_open {
        return;
    }

    let Some(cda_node_id) = cda_state.property_target else {
        return;
    };
    let Some(_) = graph.nodes.get(&cda_node_id) else {
        cda_state.close_property_window();
        return;
    };

    let mut open = true;

    // kept for compatibility with previous signature; no longer used
    let (asset_ref, title_name) = {
        let node = graph.nodes.get(&cda_node_id).unwrap();
        let NodeType::CDA(cda_data) = &node.node_type else {
            cda_state.close_property_window();
            return;
        };
        (
            cda_data.asset_ref.clone(),
            if cda_data.name.is_empty() {
                "CDA".to_string()
            } else {
                cda_data.name.clone()
            },
        )
    };

    let Some(lib) = global_cda_library() else {
        cda_state.close_property_window();
        return;
    };
    if lib.ensure_loaded(&asset_ref).is_err() {
        cda_state.close_property_window();
        return;
    }
    let Some(mut def) = lib.def_guard(asset_ref.uuid) else {
        cda_state.close_property_window();
        return;
    };
    {
        let title = format!("⬡ {} - Properties", title_name);
        let screen = ctx.content_rect();
        let pad = egui::vec2(16.0, 16.0);

        Window::new(title)
            .open(&mut open)
            .constrain_to(screen)
            .max_size(screen.size() - pad)
            .max_height(screen.height() - pad.y)
            .vscroll(true)
            .default_size([760.0, 520.0])
            .resizable(true)
            .show(ctx, |ui| {
                let bad = def.asset().count_invalid_bindings();
                if bad > 0 {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("⚠ {} invalid bindings", bad))
                                .color(Color32::from_rgb(255, 120, 120)),
                        );
                        if ui.button("Cleanup").clicked() {
                            let _ = def.asset_mut().cleanup_invalid_bindings();
                        }
                    });
                    ui.separator();
                }
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (CDAPropertyTab::Basic, "Basic"),
                        (CDAPropertyTab::Params, "Params"),
                        (CDAPropertyTab::Exports, "Exports"),
                        (CDAPropertyTab::Overlay, "Overlay"),
                        (CDAPropertyTab::Help, "Help"),
                        (CDAPropertyTab::Icon, "Icon"),
                    ] {
                        let selected = cda_state.property_tab == tab;
                        let text = if selected {
                            RichText::new(label).strong().color(Color32::WHITE)
                        } else {
                            RichText::new(label).color(Color32::GRAY)
                        };
                        if ui.selectable_label(selected, text).clicked() {
                            cda_state.property_tab = tab;
                        }
                    }
                });

                ui.separator();

                match cda_state.property_tab {
                    CDAPropertyTab::Basic => basic_tab::draw(ui, def.asset_mut()),
                    CDAPropertyTab::Params => params_tab::draw(ui, def.asset_mut(), cda_state),
                    CDAPropertyTab::Exports => exports_tab::draw(ui, def.asset_mut()),
                    CDAPropertyTab::Overlay => {
                        overlay_tab::draw(ui, def.asset_mut(), node_registry, settings)
                    }
                    CDAPropertyTab::Help => help_tab::draw(ui, def.asset_mut()),
                    CDAPropertyTab::Icon => icon_tab::draw(ui, def.asset_mut()),
                }
            });

        // Live preview: when editing promoted params, push their defaults into bound internal node params.
        if matches!(cda_state.property_tab, CDAPropertyTab::Params) {
            def.asset_mut().apply_promoted_defaults_to_inner();
        }

        let (_ch, renames) = sync_asset_io_nodes(def.asset_mut(), settings);
        let _ = renames; // CDA external ports are keyed by iface.id now; renames never migrate connections.
    }

    // One-time upgrade: migrate old name-based external connections to stable iface.id port keys.
    {
        let a = def.asset();
        let mut changed = false;
        for c in graph.connections.values_mut() {
            if c.to_node == cda_node_id && !c.to_port.as_str().starts_with("cda:") {
                if let Some(i) = a.inputs.iter().find(|i| i.name == c.to_port.as_str()) {
                    c.to_port = crate::nodes::PortId::from(i.port_key().as_str());
                    changed = true;
                }
            }
            if c.from_node == cda_node_id && !c.from_port.as_str().starts_with("cda:") {
                if let Some(o) = a.outputs.iter().find(|o| o.name == c.from_port.as_str()) {
                    c.from_port = crate::nodes::PortId::from(o.port_key().as_str());
                    changed = true;
                }
            }
        }
        if changed {
            graph.mark_dirty(cda_node_id);
        }
    }

    // IMPORTANT: drop the CDA def lock before rebuilding the instance node UI ports/params.
    // `node.rebuild_ports()` and `node.rebuild_parameters()` read from `global_cda_library()` (same mutex),
    // so keeping `def_guard` alive here would deadlock the UI thread.
    drop(def);

    // Rebuild ports to reflect any changes made in the property window
    if let Some(node) = graph.nodes.get_mut(&cda_node_id) {
        node.rebuild_ports();
        node.rebuild_parameters();
    }

    if !open {
        cda_state.close_property_window();
    }
}
