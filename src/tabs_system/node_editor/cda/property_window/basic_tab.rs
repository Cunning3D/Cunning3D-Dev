//! Basic标签页：基础信息 + 输入输出定义
use crate::cunning_core::cda::CDAAsset;
use bevy_egui::egui::{self, TextEdit, Ui};

pub fn draw(ui: &mut Ui, asset: &mut CDAAsset) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        let w = ui.available_width();
        ui.add_space(8.0);

        // 基础信息
        egui::CollapsingHeader::new("基础信息")
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("basic_info_grid")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("名称:");
                        ui.text_edit_singleline(&mut asset.name);
                        ui.end_row();

                        ui.label("版本:");
                        let mut version = asset.version as i32;
                        if ui
                            .add(egui::DragValue::new(&mut version).clamp_range(1..=9999))
                            .changed()
                        {
                            asset.version = version.max(1) as u32;
                        }
                        ui.end_row();

                        ui.label("作者:");
                        let mut author = asset.author.clone().unwrap_or_default();
                        if ui.text_edit_singleline(&mut author).changed() {
                            asset.author = if author.is_empty() {
                                None
                            } else {
                                Some(author)
                            };
                        }
                        ui.end_row();

                        ui.label("描述:");
                        ui.add(
                            TextEdit::multiline(&mut asset.description)
                                .desired_rows(3)
                                .desired_width(w),
                        );
                        ui.end_row();
                    });
            });

        ui.add_space(16.0);

        // 输入定义
        egui::CollapsingHeader::new("输入定义")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("输入数量:");
                    let mut count = asset.inputs.len();
                    if ui
                        .add(egui::DragValue::new(&mut count).clamp_range(0..=16))
                        .changed()
                    {
                        asset.set_input_count(count);
                    }
                });

                if !asset.inputs.is_empty() {
                    ui.add_space(4.0);
                    egui::Grid::new("inputs_grid")
                        .num_columns(3)
                        .striped(true)
                        .show(ui, |ui| {
                            ui.label("#");
                            ui.label("名称");
                            ui.label("标签");
                            ui.end_row();

                            for (i, input) in asset.inputs.iter_mut().enumerate() {
                                ui.label(format!("{}", i));
                                ui.text_edit_singleline(&mut input.name);
                                ui.text_edit_singleline(&mut input.label);
                                ui.end_row();
                            }
                        });
                }
            });

        ui.add_space(16.0);

        // 输出定义
        egui::CollapsingHeader::new("输出定义")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("输出数量:");
                    let mut count = asset.outputs.len();
                    if ui
                        .add(egui::DragValue::new(&mut count).clamp_range(0..=16))
                        .changed()
                    {
                        asset.set_output_count(count);
                    }
                });

                if !asset.outputs.is_empty() {
                    ui.add_space(4.0);
                    egui::Grid::new("outputs_grid")
                        .num_columns(3)
                        .striped(true)
                        .show(ui, |ui| {
                            ui.label("#");
                            ui.label("名称");
                            ui.label("标签");
                            ui.end_row();

                            for (i, output) in asset.outputs.iter_mut().enumerate() {
                                ui.label(format!("{}", i));
                                ui.text_edit_singleline(&mut output.name);
                                ui.text_edit_singleline(&mut output.label);
                                ui.end_row();
                            }
                        });
                }
            });

        ui.add_space(16.0);
    });
}
