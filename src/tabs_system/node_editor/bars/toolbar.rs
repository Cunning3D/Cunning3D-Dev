use crate::invalidator::RepaintCause;
use crate::tabs_system::node_editor::state::CopilotBackend;
use crate::tabs_system::node_editor::state::NodeEditorTab;
use crate::tabs_system::EditorTabContext;
use bevy_egui::egui::{self, Color32, RichText};
use std::time::Duration;

pub fn draw_top_bar(editor: &mut NodeEditorTab, ui: &mut egui::Ui, context: &mut EditorTabContext) {
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(4, 2))
        .fill(ui.style().visuals.window_fill)
        .stroke(ui.style().visuals.window_stroke)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(ui.available_height());
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal(|ui| {
                    // ---------------- Breadcrumb Navigation ----------------
                    let graph = &context.node_graph_res.0;
                    let mut clicked_level: Option<usize> = None;

                    // Root button
                    if ui
                        .add(
                            egui::Button::new(RichText::new("📁 Root").color(
                                if editor.cda_state.depth() == 0 {
                                    Color32::WHITE
                                } else {
                                    Color32::LIGHT_BLUE
                                },
                            ))
                            .frame(false),
                        )
                        .clicked()
                    {
                        clicked_level = Some(0);
                    }

                    let breadcrumb = editor.cda_state.breadcrumb();
                    let cur_graph: &crate::nodes::structs::NodeGraph = &graph;
                    for (i, cda_id) in breadcrumb.iter().enumerate() {
                        ui.label("›");
                        let name = cur_graph
                            .nodes
                            .get(cda_id)
                            .map(|n| n.name.as_str())
                            .unwrap_or("CDA");
                        let is_current = i == breadcrumb.len() - 1;
                        let _ = cur_graph;

                        if is_current {
                            ui.label(
                                RichText::new(format!("⬡ {}", name))
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                        } else {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(format!("⬡ {}", name))
                                            .color(Color32::LIGHT_BLUE),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                clicked_level = Some(i + 1);
                            }
                        }
                    }
                    if let Some(level) = clicked_level {
                        let mut restore_pan_zoom = None;
                        while editor.cda_state.depth() > level {
                            if let Some(popped) = editor.cda_state.exit_cda() {
                                restore_pan_zoom = Some((popped.parent_pan, popped.parent_zoom));
                                // Key: when leaving a CDA sub-graph, invalidate the parent CDA node output cache.
                                let root = &mut context.node_graph_res.0;
                                root.mark_dirty(popped.cda_node_id);
                                context.graph_changed_writer.write_default();
                            }
                        }
                        if let Some((p, z)) = restore_pan_zoom {
                            editor.pan = p;
                            editor.zoom = z;
                            editor.target_pan = p;
                            editor.target_zoom = z;
                            context.node_editor_state.pan = p;
                            context.node_editor_state.zoom = z;
                            context.node_editor_state.target_pan = p;
                            context.node_editor_state.target_zoom = z;
                            editor.cached_nodes_rev = 0; // Force refresh
                            editor.node_animations.clear();
                            editor.insertion_target = None;
                            editor.pending_connection_from = None;
                            editor.snapped_to_port = None;
                            context.ui_invalidator.request_repaint_after_tagged(
                                "node_editor/cda_exit",
                                Duration::ZERO,
                                RepaintCause::DataChanged,
                            );
                        }
                    }
                    // -------------------------------------------------------

                    ui.separator();

                    ui.add_space(20.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut editor.search_text)
                            .desired_width(120.0)
                            .hint_text("🔍 Find Node..."),
                    );

                    ui.menu_button("👁", |ui| {
                        ui.checkbox(&mut true, "Show Grid");
                        ui.checkbox(&mut false, "Snap to Grid");
                    });

                    if ui.button("Sticky Note").clicked() {
                        editor.create_sticky_note_request = true;
                    }
                    if ui.button("Network Box").clicked() {
                        editor.create_network_box_request = true;
                    }
                    if ui
                        .button("Promote Sticky")
                        .on_hover_text("Add voice/text prompt note for Copilot")
                        .clicked()
                    {
                        editor.create_promote_note_request = true;
                    }
                    ui.separator();

                    if ui
                        .button("⬡ CDA")
                        .on_hover_text("Create CDA from selected nodes")
                        .clicked()
                    {
                        editor.create_cda_request = true;
                    }

                    ui.separator();
                    egui::ComboBox::from_id_source("node_editor_copilot_backend")
                        .width(180.0)
                        .selected_text(match editor.copilot_backend {
                            CopilotBackend::LocalTiny => "Copilot: Local 1.7B",
                            CopilotBackend::LocalThink => "Copilot: Local 4B (Think)",
                            CopilotBackend::Gemini => "Copilot: Gemini",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut editor.copilot_backend,
                                CopilotBackend::LocalTiny,
                                "Local 1.7B (Fast)",
                            );
                            ui.selectable_value(
                                &mut editor.copilot_backend,
                                CopilotBackend::LocalThink,
                                "Local 4B (Think, Slow)",
                            );
                            ui.selectable_value(
                                &mut editor.copilot_backend,
                                CopilotBackend::Gemini,
                                "Gemini (Cloud)",
                            );
                        });
                    let _ = ui.button("⚄");
                    let _ = ui.button("⛶");
                });
            });
        });
}

pub fn draw_sidebar(editor: &mut NodeEditorTab, ui: &mut egui::Ui) {
    egui::Frame::NONE
        .inner_margin(egui::Margin::same(4))
        .fill(ui.style().visuals.window_fill)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(ui.available_height());
            ui.vertical_centered(|ui| {
                ui.separator();
                if ui.button("✂").clicked() {
                    editor.is_cutting = !editor.is_cutting;
                }
            });
        });
}
