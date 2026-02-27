//! Icon tab: icon, color, tags
use crate::cunning_core::cda::CDAAsset;
use bevy_egui::egui::{self, Color32, TextEdit, Ui, Vec2};

pub fn draw(ui: &mut Ui, asset: &mut CDAAsset) {
    egui::ScrollArea::vertical()
        .max_height(ui.available_height())
        .show(ui, |ui| {
            ui.add_space(8.0);

            // Icon
            egui::CollapsingHeader::new("Node Icon")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Preview
                        egui::Frame::NONE
                            .fill(Color32::from_rgb(40, 40, 45))
                            .corner_radius(4.0)
                            .show(ui, |ui| {
                                ui.set_min_size(Vec2::splat(64.0));
                                ui.centered_and_justified(|ui| {
                                    let icon_text = asset.icon.as_deref().unwrap_or("⬡");
                                    ui.label(egui::RichText::new(icon_text).size(32.0));
                                });
                            });

                        ui.vertical(|ui| {
                            // Icon path
                            let mut icon = asset.icon.clone().unwrap_or_default();
                            ui.horizontal(|ui| {
                                ui.label("Icon:");
                                if ui
                                    .add(
                                        TextEdit::singleline(&mut icon)
                                            .desired_width(200.0)
                                            .hint_text("emoji or path"),
                                    )
                                    .changed()
                                {
                                    asset.icon = if icon.is_empty() { None } else { Some(icon) };
                                }
                            });

                            ui.horizontal(|ui| {
                                if ui.small_button("📁 Choose image...").clicked() {
                                    // TODO: file picker
                                }
                                if ui.small_button("🎨 Use emoji").clicked() {
                                    asset.icon = Some("🛤️".to_string());
                                }
                                if ui.small_button("✖ Clear").clicked() {
                                    asset.icon = None;
                                }
                            });
                        });
                    });
                });

            ui.add_space(16.0);

            // Node color
            egui::CollapsingHeader::new("Node Color")
                .default_open(true)
                .show(ui, |ui| {
                    let mut color = asset.color.unwrap_or([0.2, 0.4, 0.8]);

                    ui.horizontal(|ui| {
                        let mut c32 = egui::Color32::from_rgb(
                            (color[0] * 255.0) as u8,
                            (color[1] * 255.0) as u8,
                            (color[2] * 255.0) as u8,
                        );
                        if ui.color_edit_button_srgba(&mut c32).changed() {
                            color = [
                                c32.r() as f32 / 255.0,
                                c32.g() as f32 / 255.0,
                                c32.b() as f32 / 255.0,
                            ];
                        }

                        // Manual RGB sliders
                        ui.label("R:");
                        ui.add(
                            egui::DragValue::new(&mut color[0])
                                .range(0.0..=1.0)
                                .speed(0.01),
                        );
                        ui.label("G:");
                        ui.add(
                            egui::DragValue::new(&mut color[1])
                                .range(0.0..=1.0)
                                .speed(0.01),
                        );
                        ui.label("B:");
                        ui.add(
                            egui::DragValue::new(&mut color[2])
                                .range(0.0..=1.0)
                                .speed(0.01),
                        );

                        if ui.small_button("✖ Clear").clicked() {
                            asset.color = None;
                        } else {
                            asset.color = Some(color);
                        }
                    });
                });

            ui.add_space(16.0);

            // Tags
            egui::CollapsingHeader::new("Tags")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let mut to_remove = None;
                        for (i, tag) in asset.tags.iter().enumerate() {
                            egui::Frame::NONE
                                .fill(Color32::from_rgb(60, 60, 70))
                                .corner_radius(4.0)
                                .inner_margin(egui::Margin::symmetric(6, 2))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(tag);
                                        if ui.small_button("×").clicked() {
                                            to_remove = Some(i);
                                        }
                                    });
                                });
                        }
                        if let Some(i) = to_remove {
                            asset.tags.remove(i);
                        }

                        // Add tag
                        if ui.small_button("+ Add").clicked() {
                            asset.tags.push("New tag".to_string());
                        }
                    });

                    // Edit the last tag
                    if let Some(last) = asset.tags.last_mut() {
                        ui.horizontal(|ui| {
                            ui.label("Edit:");
                            ui.text_edit_singleline(last);
                        });
                    }
                });

            ui.add_space(16.0);
        });
}
