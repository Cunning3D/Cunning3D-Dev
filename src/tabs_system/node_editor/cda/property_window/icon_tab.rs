//! Icon标签页：图标、颜色、标签
use crate::cunning_core::cda::CDAAsset;
use bevy_egui::egui::{self, Color32, TextEdit, Ui, Vec2};

pub fn draw(ui: &mut Ui, asset: &mut CDAAsset) {
    egui::ScrollArea::vertical()
        .max_height(ui.available_height())
        .show(ui, |ui| {
            ui.add_space(8.0);

            // 图标
            egui::CollapsingHeader::new("节点图标")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // 预览
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
                            // 图标路径
                            let mut icon = asset.icon.clone().unwrap_or_default();
                            ui.horizontal(|ui| {
                                ui.label("图标:");
                                if ui
                                    .add(
                                        TextEdit::singleline(&mut icon)
                                            .desired_width(200.0)
                                            .hint_text("emoji 或路径"),
                                    )
                                    .changed()
                                {
                                    asset.icon = if icon.is_empty() { None } else { Some(icon) };
                                }
                            });

                            ui.horizontal(|ui| {
                                if ui.small_button("📁 选择图片...").clicked() {
                                    // TODO: 文件选择器
                                }
                                if ui.small_button("🎨 使用Emoji").clicked() {
                                    asset.icon = Some("🛤️".to_string());
                                }
                                if ui.small_button("✖ 清除").clicked() {
                                    asset.icon = None;
                                }
                            });
                        });
                    });
                });

            ui.add_space(16.0);

            // 节点颜色
            egui::CollapsingHeader::new("节点颜色")
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

                        if ui.small_button("✖ 清除").clicked() {
                            asset.color = None;
                        } else {
                            asset.color = Some(color);
                        }
                    });
                });

            ui.add_space(16.0);

            // 标签
            egui::CollapsingHeader::new("标签")
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

                        // 添加标签
                        if ui.small_button("+ 添加").clicked() {
                            asset.tags.push("新标签".to_string());
                        }
                    });

                    // 编辑最后一个标签
                    if let Some(last) = asset.tags.last_mut() {
                        ui.horizontal(|ui| {
                            ui.label("编辑:");
                            ui.text_edit_singleline(last);
                        });
                    }
                });

            ui.add_space(16.0);
        });
}
