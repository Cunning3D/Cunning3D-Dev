//! CDA creation dialog
use super::editor_state::{CDACreateInfo, CDAEditorState};
use crate::tabs_system::node_editor::cda;
use crate::tabs_system::EditorTabContext;
use bevy_egui::egui::{self, Window};

/// Draw CDA creation dialog
pub fn draw_create_dialog(
    ctx: &egui::Context,
    cda_state: &mut CDAEditorState,
    context: &EditorTabContext,
) {
    if !cda_state.create_dialog_open {
        return;
    }

    let mut should_close = false;
    let mut should_create = false;

    Window::new("Create CDA")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(400.0);

            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut cda_state.create_name);
            });

            ui.add_space(8.0);
            ui.separator();

            ui.label(format!(
                "Selected nodes: {}",
                cda_state.create_selected_nodes.len()
            ));

            let root_graph = &context.node_graph_res.0;
            let mut graph =
                cda::navigation::graph_snapshot_by_path(&root_graph, &cda_state.breadcrumb());
            egui::ScrollArea::vertical()
                .max_height(150.0)
                .show(ui, |ui| {
                    for node_id in &cda_state.create_selected_nodes {
                        if let Some(node) = graph.nodes.get(node_id) {
                            ui.horizontal(|ui| {
                                ui.label("•");
                                ui.label(&node.name);
                                ui.label(format!("({})", node.node_type.name()));
                            });
                        }
                    }
                });

            let result = crate::nodes::cda::cda_node::create_cda_from_nodes(
                &cda_state.create_name,
                &mut graph,
                &cda_state.create_selected_nodes,
                context.node_editor_settings,
            );
            let preview = &result.asset;

            ui.add_space(8.0);
            ui.collapsing("Input Interfaces (Auto-detected)", |ui| {
                if preview.inputs.is_empty() {
                    ui.label("(No external inputs)");
                } else {
                    for input in &preview.inputs {
                        ui.label(format!("• {}", input.name));
                    }
                }
            });
            ui.collapsing("Output Interfaces (Auto-detected)", |ui| {
                if preview.outputs.is_empty() {
                    ui.label("(No external outputs)");
                } else {
                    for output in &preview.outputs {
                        ui.label(format!("• {}", output.name));
                    }
                }
            });

            ui.add_space(16.0);

            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    should_close = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let can_create = !cda_state.create_name.trim().is_empty()
                        && !cda_state.create_selected_nodes.is_empty();
                    if ui
                        .add_enabled(can_create, egui::Button::new("✓ Create CDA"))
                        .clicked()
                    {
                        should_create = true;
                        should_close = true;
                    }
                });
            });
        });

    if should_create {
        cda_state.pending_create = Some(CDACreateInfo {
            name: cda_state.create_name.clone(),
            selected_nodes: cda_state.create_selected_nodes.clone(),
        });
    }
    if should_close {
        cda_state.close_create_dialog();
    }
}
