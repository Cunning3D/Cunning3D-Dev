use crate::cunning_core::traits::pane_interface::PaneTab;
use crate::tabs_system::EditorTabContext;
use bevy_egui::egui;

#[derive(Default)]
pub struct ConsoleTab {
    auto_scroll: bool,
    show_detail_trace: bool,
    server: Option<puffin_http::Server>,
}

impl PaneTab for ConsoleTab {
    fn ui(&mut self, ui: &mut egui::Ui, context: &mut EditorTabContext) {
        let console_log = context.console_log;

        egui::CollapsingHeader::new("Detail Trace")
            .default_open(self.show_detail_trace)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut on = puffin::are_scopes_on();
                    if ui
                        .selectable_label(on, if on { "⏸ Puffin ON" } else { "▶ Puffin OFF" })
                        .clicked()
                    {
                        on = !on;
                        puffin::set_scopes_on(on);
                    }
                    if self.server.is_some() {
                        if ui.button("Stop Server").clicked() {
                            self.server = None;
                        }
                    } else if ui.button("Start Server :8585").clicked() {
                        puffin::set_scopes_on(true);
                        self.server = puffin_http::Server::new("127.0.0.1:8585").ok();
                    }
                    if ui.button("Clear Perf Stats").clicked() {
                        if let Some(p) = context.perf_monitor.as_deref_mut() {
                            p.clear();
                        }
                    }
                });
                ui.label("Viewer: `cargo install puffin_viewer && puffin_viewer --url 127.0.0.1:8585`");

                let mut sys_stats_toggle: Option<bool> = None;
                if let Some(p) = context.perf_monitor.as_deref() {
                    ui.horizontal(|ui| {
                        let mut on = p.sys_stats_enabled;
                        if ui.checkbox(&mut on, "Sys stats").on_hover_text("Enable periodic CPU/memory sampling (may cause periodic frame drops on some machines).").changed() {
                            sys_stats_toggle = Some(on);
                        }
                        ui.separator();
                        ui.label(format!("CPU: {:.1}%", p.cpu_usage));
                        ui.separator();
                        ui.label(format!("Mem: {:.1}/{:.1} GiB (avail {:.1} GiB)", (p.used_mem as f64) / (1024.0 * 1024.0 * 1024.0), (p.total_mem as f64) / (1024.0 * 1024.0 * 1024.0), (p.available_mem as f64) / (1024.0 * 1024.0 * 1024.0)));
                        ui.separator();
                        ui.label(format!("Cook tracked: {}", p.node_cook_times.len()));
                    });
                    ui.add_space(6.0);

                    let mut rows: Vec<(crate::nodes::NodeId, &crate::cunning_core::profiling::ComputeRecord)> =
                        p.node_cook_times.iter().map(|(k, v)| (*k, v)).collect();
                    rows.sort_by(|a, b| b.1.duration.cmp(&a.1.duration));

                    let name_of = |id: crate::nodes::NodeId, cx: &EditorTabContext| -> String {
                        cx.node_graph_res
                            .0
                            .nodes
                            .get(&id)
                            .map(|n| n.name.clone())
                            .unwrap_or_else(|| id.to_string())
                    };

                    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                        egui::Grid::new("console_detail_trace_grid").striped(true).show(ui, |ui| {
                            ui.strong("Node");
                            ui.strong("ms");
                            ui.strong("count");
                            ui.strong("thread");
                            ui.end_row();
                            for (id, r) in rows.iter().take(200) {
                                ui.label(name_of(*id, context));
                                ui.label(format!("{:.3}", r.duration.as_secs_f64() * 1000.0));
                                ui.label(p.node_cook_counts.get(id).copied().unwrap_or(0).to_string());
                                ui.label(&r.thread_name);
                                ui.end_row();
                            }
                        });
                    });
                } else {
                    ui.colored_label(egui::Color32::YELLOW, "PerformanceMonitor not available.");
                }
                if let Some(v) = sys_stats_toggle { if let Some(p) = context.perf_monitor.as_deref_mut() { p.sys_stats_enabled = v; } }
                ui.separator();
            });

        ui.horizontal(|ui| {
            if ui.button("Clear").clicked() {
                console_log.clear();
            }

            if ui.button("Copy All").clicked() {
                let text = console_log.get_all_text();
                ui.output_mut(|o| o.commands.push(egui::OutputCommand::CopyText(text)));
            }

            ui.checkbox(&mut self.auto_scroll, "Auto-scroll");
        });

        ui.separator();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(self.auto_scroll)
            .show(ui, |ui| {
                let entries = console_log.get_entries();

                if entries.is_empty() {
                    ui.colored_label(egui::Color32::GRAY, "No log messages yet...");
                } else {
                    for entry in entries {
                        ui.horizontal(|ui| {
                            ui.colored_label(
                                egui::Color32::DARK_GRAY,
                                format!("[{}]", entry.timestamp),
                            );
                            ui.colored_label(entry.level.color(), format!("{:?}", entry.level));
                            ui.label(&entry.message);
                        });
                    }
                }
            });
    }

    fn title(&self) -> egui::WidgetText {
        "Console".into()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// --- V5.0 ZERO-TOUCH REGISTRATION ---
use crate::register_pane;
register_pane!("Console", ConsoleTab);
