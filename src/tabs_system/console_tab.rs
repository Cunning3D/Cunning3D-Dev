//! Console tab: log-only view (no retained/perf debug UI).

use crate::tabs_system::{EditorTab, EditorTabContext};
use bevy_egui::egui;

#[derive(Default)]
pub struct ConsoleTab {
    auto_scroll: bool,
}

impl EditorTab for ConsoleTab {
    fn ui(&mut self, ui: &mut egui::Ui, context: &mut EditorTabContext) {
        // Paint an opaque background because the dock style can be transparent for viewport holes.
        ui.painter()
            .rect_filled(ui.clip_rect(), 0.0, ui.visuals().panel_fill);

        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.auto_scroll, "Auto-scroll");
                if ui.button("Copy All").clicked() {
                    let text = context.console_log.get_all_text();
                    ui.output_mut(|o| o.commands.push(egui::OutputCommand::CopyText(text)));
                }
                if ui.button("Clear").clicked() {
                    context.console_log.clear();
                }
            });
        });

        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(self.auto_scroll)
            .show(ui, |ui| {
                let entries = context.console_log.get_entries();
                if entries.is_empty() {
                    ui.colored_label(egui::Color32::GRAY, "No log messages yet...");
                } else {
                    for e in entries {
                        ui.horizontal(|ui| {
                            ui.colored_label(
                                egui::Color32::DARK_GRAY,
                                format!("[{}]", e.timestamp),
                            );
                            ui.colored_label(e.level.color(), format!("{:?}", e.level));
                            ui.label(&e.message);
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
