//! 绑定卡片：Parameter/Channels/Menu三个Tab
use super::super::drag_payload::ParamDragPayload;
use super::super::editor_state::{BindingCardTab, CDAEditorState};
use crate::cunning_core::cda::promoted_param::{
    DropdownItem, ParamChannel, PromotedParam, PromotedParamType,
};
use crate::cunning_core::cda::ParamBinding;
use bevy_egui::egui::{self, Color32, DragAndDrop, RichText, Ui};

pub fn draw(ui: &mut Ui, param: &mut PromotedParam, cda_state: &mut CDAEditorState) {
    // Tab栏
    ui.horizontal(|ui| {
        for (tab, label) in [
            (BindingCardTab::Parameter, "Parameter"),
            (BindingCardTab::Channels, "Channels"),
            (BindingCardTab::Menu, "Menu"),
        ] {
            let selected = cda_state.binding_card_tab == tab;
            let enabled = match tab {
                BindingCardTab::Menu => {
                    matches!(param.param_type, PromotedParamType::Dropdown { .. })
                }
                _ => true,
            };

            let text = if selected {
                RichText::new(label).strong().color(Color32::WHITE)
            } else if enabled {
                RichText::new(label).color(Color32::GRAY)
            } else {
                RichText::new(label).color(Color32::DARK_GRAY)
            };

            if ui
                .add_enabled(enabled, egui::SelectableLabel::new(selected, text))
                .clicked()
            {
                cda_state.binding_card_tab = tab;
            }
        }
    });

    ui.separator();
    ui.add_space(4.0);

    match cda_state.binding_card_tab {
        BindingCardTab::Parameter => draw_parameter_tab(ui, param),
        BindingCardTab::Channels => draw_channels_tab(ui, param, cda_state),
        BindingCardTab::Menu => draw_menu_tab(ui, param),
    }
}

/// Parameter标签页：修改参数类型和基本属性
fn draw_parameter_tab(ui: &mut Ui, param: &mut PromotedParam) {
    egui::ScrollArea::vertical()
        .max_height(ui.available_height())
        .show(ui, |ui| {
            let edit_w = 220.0;
            egui::Grid::new("param_props")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Name:");
                    ui.add(egui::TextEdit::singleline(&mut param.name).desired_width(edit_w));
                    ui.end_row();

                    ui.label("Label:");
                    ui.add(egui::TextEdit::singleline(&mut param.label).desired_width(edit_w));
                    ui.end_row();

                    ui.label("Group:");
                    ui.add(egui::TextEdit::singleline(&mut param.group).desired_width(edit_w));
                    ui.end_row();

                    ui.label("Order:");
                    ui.add(egui::DragValue::new(&mut param.order));
                    ui.end_row();

                    ui.label("Type:");
                    let before = std::mem::discriminant(&param.param_type);
                    egui::ComboBox::from_id_source("param_type")
                        .selected_text(param.param_type.display_name())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut param.param_type,
                                PromotedParamType::Float {
                                    min: 0.0,
                                    max: 10.0,
                                    logarithmic: false,
                                },
                                "Float",
                            );
                            ui.selectable_value(
                                &mut param.param_type,
                                PromotedParamType::Int { min: 0, max: 100 },
                                "Int",
                            );
                            ui.selectable_value(
                                &mut param.param_type,
                                PromotedParamType::Bool,
                                "Bool",
                            );
                            ui.selectable_value(
                                &mut param.param_type,
                                PromotedParamType::Toggle,
                                "Toggle",
                            );
                            ui.selectable_value(
                                &mut param.param_type,
                                PromotedParamType::Button,
                                "Button",
                            );
                            ui.selectable_value(
                                &mut param.param_type,
                                PromotedParamType::Vec2,
                                "Vec2",
                            );
                            ui.selectable_value(
                                &mut param.param_type,
                                PromotedParamType::Vec3,
                                "Vec3",
                            );
                            ui.selectable_value(
                                &mut param.param_type,
                                PromotedParamType::Vec4,
                                "Vec4",
                            );
                            ui.selectable_value(
                                &mut param.param_type,
                                PromotedParamType::Color { has_alpha: false },
                                "Color",
                            );
                            ui.selectable_value(
                                &mut param.param_type,
                                PromotedParamType::String,
                                "String",
                            );
                            ui.selectable_value(
                                &mut param.param_type,
                                PromotedParamType::Angle,
                                "Angle",
                            );
                            ui.selectable_value(
                                &mut param.param_type,
                                PromotedParamType::Dropdown { items: vec![] },
                                "Dropdown",
                            );
                        });
                    if before != std::mem::discriminant(&param.param_type) {
                        sync_channels(param);
                    }
                    ui.end_row();
                });

            ui.add_space(8.0);

            // 类型特定配置
            match &mut param.param_type {
                PromotedParamType::Float {
                    min,
                    max,
                    logarithmic,
                } => {
                    ui.horizontal(|ui| {
                        ui.label("Range:");
                        ui.add(egui::DragValue::new(min).speed(0.1));
                        ui.label("~");
                        ui.add(egui::DragValue::new(max).speed(0.1));
                    });
                    ui.checkbox(logarithmic, "Logarithmic");
                }
                PromotedParamType::Int { min, max } => {
                    ui.horizontal(|ui| {
                        ui.label("Range:");
                        ui.add(egui::DragValue::new(min));
                        ui.label("~");
                        ui.add(egui::DragValue::new(max));
                    });
                }
                PromotedParamType::Color { has_alpha } => {
                    ui.checkbox(has_alpha, "Has Alpha");
                }
                _ => {}
            }
            sync_channels(param);

            ui.add_space(8.0);

            // UI配置
            ui.collapsing("UI Config", |ui| {
                ui.checkbox(&mut param.ui_config.visible, "Visible");
                ui.checkbox(&mut param.ui_config.enabled, "Enabled");
                ui.checkbox(&mut param.ui_config.lock_range, "Lock Range");

                let mut tooltip = param.ui_config.tooltip.clone().unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.label("Tooltip:");
                    if ui
                        .add(egui::TextEdit::singleline(&mut tooltip).desired_width(edit_w))
                        .changed()
                    {
                        param.ui_config.tooltip = if tooltip.is_empty() {
                            None
                        } else {
                            Some(tooltip)
                        };
                    }
                });

                let mut condition = param.ui_config.condition.clone().unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.label("Visible If:");
                    if ui
                        .add(egui::TextEdit::singleline(&mut condition).desired_width(edit_w))
                        .on_hover_text("Example: 'enable_noise == 1' or 'type == 2'")
                        .changed()
                    {
                        param.ui_config.condition = if condition.is_empty() {
                            None
                        } else {
                            Some(condition)
                        };
                    }
                });
            });
        });
}

fn sync_channels(param: &mut PromotedParam) {
    let names = param.param_type.channel_names();
    let needs = param.channels.len() != names.len()
        || names
            .iter()
            .enumerate()
            .any(|(i, n)| param.channels.get(i).map(|c| c.name.as_str()) != Some(*n));
    if !needs {
        return;
    }
    param.channels = names
        .into_iter()
        .map(|n| ParamChannel::new(n, 0.0))
        .collect();
}

/// Channels标签页：通道级绑定管理
fn draw_channels_tab(ui: &mut Ui, param: &mut PromotedParam, _cda_state: &mut CDAEditorState) {
    ui.add_sized(
        [ui.available_width(), 0.0],
        egui::Label::new(
            RichText::new(format!("{} - {} 个通道", param.label, param.channels.len())).strong(),
        )
        .truncate(),
    );
    ui.add_space(8.0);

    egui::ScrollArea::vertical()
        .max_height(ui.available_height())
        .show(ui, |ui| {
            for (_ch_idx, channel) in param.channels.iter_mut().enumerate() {
                egui::Frame::NONE
                    .fill(Color32::from_rgb(40, 40, 45))
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let ch_name = if channel.name.is_empty() {
                                "Value"
                            } else {
                                &channel.name
                            };
                            ui.label(RichText::new(ch_name.to_uppercase()).strong());
                            ui.label(RichText::new("(default)").weak().small());
                            match &param.param_type {
                                PromotedParamType::Int { .. }
                                | PromotedParamType::Dropdown { .. } => {
                                    let mut v = channel.default_value as i32;
                                    if ui.add(egui::DragValue::new(&mut v)).changed() {
                                        channel.default_value = v as f64;
                                    }
                                }
                                PromotedParamType::Bool | PromotedParamType::Toggle => {
                                    let mut v = channel.default_value != 0.0;
                                    if ui.checkbox(&mut v, "").changed() {
                                        channel.default_value = if v { 1.0 } else { 0.0 };
                                    }
                                }
                                PromotedParamType::String | PromotedParamType::FilePath { .. } => {
                                    ui.label(RichText::new("(n/a)").weak().small());
                                }
                                _ => {
                                    ui.add(
                                        egui::DragValue::new(&mut channel.default_value).speed(0.1),
                                    );
                                }
                            }
                        });

                        ui.add_space(4.0);
                        ui.separator();
                        ui.add_space(4.0);

                        if channel.bindings.is_empty() {
                            ui.label(RichText::new("(无绑定)").weak().italics());
                        } else {
                            let mut to_remove = None;
                            for (i, binding) in channel.bindings.iter().enumerate() {
                                egui::Frame::NONE
                                    .fill(Color32::from_rgb(46, 46, 52))
                                    .corner_radius(3.0)
                                    .inner_margin(egui::Margin::symmetric(6, 4))
                                    .show(ui, |ui| {
                                        ui.vertical(|ui| {
                                            ui.horizontal(|ui| {
                                                ui.label("├─");
                                                ui.add_sized(
                                                    [ui.available_width(), 0.0],
                                                    egui::Label::new(&binding.target_param)
                                                        .truncate(),
                                                )
                                                .on_hover_text(&binding.target_param);
                                                ui.with_layout(
                                                    egui::Layout::right_to_left(
                                                        egui::Align::Center,
                                                    ),
                                                    |ui| {
                                                        if ui.small_button("×").clicked() {
                                                            to_remove = Some(i);
                                                        }
                                                    },
                                                );
                                            });
                                            ui.horizontal_wrapped(|ui| {
                                                if let Some(ch) = binding.target_channel {
                                                    ui.label(
                                                        RichText::new(format!("[{}]", ch)).small(),
                                                    );
                                                }
                                                ui.label(
                                                    RichText::new(format!(
                                                        "node {}",
                                                        &binding.target_node.to_string()[..8]
                                                    ))
                                                    .weak()
                                                    .small(),
                                                );
                                            });
                                        });
                                    });
                            }
                            if let Some(i) = to_remove {
                                channel.bindings.remove(i);
                            }
                        }

                        // 拖拽放置区（从Properties/内部参数列表拖进来）
                        ui.add_space(4.0);
                        let frame = egui::Frame::NONE
                            .fill(Color32::from_rgb(50, 50, 55))
                            .corner_radius(2.0)
                            .inner_margin(egui::Margin::symmetric(8, 4));
                        let (_inner, payload) =
                            ui.dnd_drop_zone::<ParamDragPayload, _>(frame, |ui| {
                                ui.label(RichText::new("拖拽参数到此绑定").weak().small());
                            });

                        if DragAndDrop::has_payload_of_type::<ParamDragPayload>(ui.ctx()) {
                            let rect = _inner.response.rect;
                            let pointer_in_rect = ui
                                .input(|i| i.pointer.interact_pos())
                                .map_or(false, |p| rect.contains(p));

                            if pointer_in_rect {
                                let ok = DragAndDrop::payload::<ParamDragPayload>(ui.ctx())
                                    .as_ref()
                                    .map(|p| p.param_type.can_bind_to(&param.param_type))
                                    .unwrap_or(false);
                                let c = if ok {
                                    Color32::LIGHT_GREEN
                                } else {
                                    Color32::from_rgb(255, 120, 120)
                                };
                                ui.painter().rect_stroke(
                                    rect,
                                    2.0,
                                    egui::Stroke::new(2.0, c),
                                    egui::StrokeKind::Inside,
                                );
                            }
                        }

                        if let Some(p) = payload {
                            let p = &*p;
                            if p.param_type.can_bind_to(&param.param_type) {
                                let binding = ParamBinding {
                                    target_node: p.source_node,
                                    target_param: p.param_name.clone(),
                                    target_channel: p.channel_index,
                                };
                                if !channel.bindings.iter().any(|b| {
                                    b.target_node == binding.target_node
                                        && b.target_param == binding.target_param
                                        && b.target_channel == binding.target_channel
                                }) {
                                    channel.bindings.push(binding);
                                }
                            }
                        }
                    });

                ui.add_space(8.0);
            }
        });
}

/// Menu标签页：下拉菜单项定义
fn draw_menu_tab(ui: &mut Ui, param: &mut PromotedParam) {
    let PromotedParamType::Dropdown { items } = &mut param.param_type else {
        ui.label("此参数类型不支持菜单配置");
        return;
    };

    ui.label(RichText::new("菜单项定义").strong());
    ui.add_space(8.0);

    egui::ScrollArea::vertical()
        .max_height(ui.available_height())
        .show(ui, |ui| {
            let mut to_remove = None;

            egui::Grid::new("menu_items")
                .num_columns(4)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("值");
                    ui.label("显示标签");
                    ui.label("");
                    ui.label("");
                    ui.end_row();

                    let item_count = items.len();
                    for (i, item) in items.iter_mut().enumerate() {
                        ui.add(egui::DragValue::new(&mut item.value));
                        ui.add(egui::TextEdit::singleline(&mut item.label).desired_width(160.0));

                        if ui.small_button("×").clicked() {
                            to_remove = Some(i);
                        }
                        ui.horizontal(|ui| {
                    if ui.small_button("↑").clicked() && i > 0 { /* TODO: 上移 */ }
                    if ui.small_button("↓").clicked() && i < item_count - 1 { /* TODO: 下移 */ }
                });
                        ui.end_row();
                    }
                });

            if let Some(i) = to_remove {
                items.remove(i);
            }

            ui.add_space(8.0);
            if ui.button("+ 添加选项").clicked() {
                let next_value = items.iter().map(|i| i.value).max().unwrap_or(-1) + 1;
                items.push(DropdownItem {
                    value: next_value,
                    label: format!("Option {}", next_value),
                });
            }
        });
}
