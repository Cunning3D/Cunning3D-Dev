use crate::settings::{SettingMeta, SettingScope, SettingValue, SettingsMerge};
use crate::tabs_system::{EditorTab, EditorTabContext};
use bevy_egui::egui;

#[derive(Default)]
pub struct SettingsPane {
    filter: String,
    cached_filter: String,
    // Cached left-tree view (rebuilt only when filter changes).
    // Structure: [(section, [(group, [setting_id...])...])...]
    cached_tree: Vec<(String, Vec<(String, Vec<String>)>)>,
    selected: Option<String>,
}

impl SettingsPane {
    fn allow(scope: SettingScope) -> bool {
        matches!(scope, SettingScope::User | SettingScope::Both)
    }

    fn value_label(v: &SettingValue) -> String {
        match v {
            SettingValue::Bool(v) => v.to_string(),
            SettingValue::I64(v) => v.to_string(),
            SettingValue::F32(v) => format!("{v:.3}"),
            SettingValue::String(v) | SettingValue::Enum(v) => v.clone(),
            SettingValue::Color32(rgba) => {
                format!("rgba({}, {}, {}, {})", rgba[0], rgba[1], rgba[2], rgba[3])
            }
            SettingValue::Vec2(v) => format!("({}, {})", v[0], v[1]),
        }
    }

    fn draw_value_editor(
        ui: &mut egui::Ui,
        v: &mut SettingValue,
        min: Option<f32>,
        max: Option<f32>,
        step: Option<f32>,
    ) -> bool {
        match v {
            SettingValue::Bool(b) => ui.checkbox(b, "").changed(),
            SettingValue::I64(i) => {
                let mut tmp = *i as i32;
                let ch = ui
                    .add(egui::DragValue::new(&mut tmp).speed(step.unwrap_or(1.0)))
                    .changed();
                if ch {
                    *i = tmp as i64;
                }
                ch
            }
            SettingValue::F32(x) => {
                let mut dv = egui::DragValue::new(x).speed(step.unwrap_or(0.1));
                if let Some(mn) = min {
                    dv = dv.clamp_range(mn..=max.unwrap_or(mn));
                }
                ui.add(dv).changed()
            }
            SettingValue::Enum(s) => {
                if s.eq_ignore_ascii_case("dark") || s.eq_ignore_ascii_case("light") {
                    let mut cur = if s.eq_ignore_ascii_case("light") {
                        1
                    } else {
                        0
                    };
                    let ch = ui
                        .horizontal(|ui| {
                            ui.radio_value(&mut cur, 0, "Dark").changed()
                                || ui.radio_value(&mut cur, 1, "Light").changed()
                        })
                        .inner;
                    if ch {
                        *s = if cur == 1 {
                            "Light".into()
                        } else {
                            "Dark".into()
                        };
                    }
                    ch
                } else {
                    ui.text_edit_singleline(s).changed()
                }
            }
            SettingValue::String(s) => ui.text_edit_singleline(s).changed(),
            SettingValue::Color32(rgba) => {
                let mut c =
                    egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
                let ch = ui.color_edit_button_srgba(&mut c).changed();
                if ch {
                    *rgba = [c.r(), c.g(), c.b(), c.a()];
                }
                ch
            }
            SettingValue::Vec2(v2) => {
                let mut x = v2[0];
                let mut y = v2[1];
                let ch = ui
                    .horizontal(|ui| {
                        let sx = ui
                            .add(egui::DragValue::new(&mut x).speed(step.unwrap_or(0.1)))
                            .changed();
                        let sy = ui
                            .add(egui::DragValue::new(&mut y).speed(step.unwrap_or(0.1)))
                            .changed();
                        sx || sy
                    })
                    .inner;
                if ch {
                    *v2 = [x, y];
                }
                ch
            }
        }
    }

    fn apply_ui_settings(
        meta: &SettingMeta,
        stores: &crate::settings::SettingsStores,
        reg: &crate::settings::SettingsRegistry,
        ui_settings: &mut crate::ui_settings::UiSettings,
    ) {
        let _ = meta;
        crate::ui_settings::apply_from_settings(reg, stores, ui_settings);
    }

    fn rebuild_tree_cache(&mut self, reg: &crate::settings::SettingsRegistry) {
        use std::collections::BTreeMap;
        let mut tree: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
        let q = self.filter.to_lowercase();
        for meta in reg.iter() {
            if !Self::allow(meta.scope) {
                continue;
            }
            if !q.is_empty() {
                let hay = format!("{} {} {} {}", meta.id, meta.path, meta.label, meta.help)
                    .to_lowercase();
                if !hay.contains(&q) {
                    continue;
                }
            }
            let mut it = meta.path.split('/').filter(|s| !s.is_empty());
            let a = it.next().unwrap_or("Misc").to_string();
            let b = it.next().unwrap_or("General").to_string();
            tree.entry(a)
                .or_default()
                .entry(b)
                .or_default()
                .push(meta.id.clone());
        }
        self.cached_tree = tree
            .into_iter()
            .map(|(a, bmap)| (a, bmap.into_iter().map(|(b, ids)| (b, ids)).collect()))
            .collect();
        self.cached_filter = self.filter.clone();
    }
}

impl EditorTab for SettingsPane {
    fn title(&self) -> egui::WidgetText {
        "Settings".into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut EditorTabContext) {
        // PERF: Settings has lots of collapsing headers; animated expansion causes multi-frame heavy text layout.
        // Disable animations in this pane only to avoid "triangle animation stutter".
        let old_style = (*ui.ctx().style()).clone();
        let mut restore_style = false;
        if old_style.animation_time > 0.0 {
            let mut s = old_style.clone();
            s.animation_time = 0.0;
            ui.ctx().set_style(s);
            restore_style = true;
        }

        let reg = cx.settings_registry;
        let stores = &mut *cx.settings_stores;

        egui::TopBottomPanel::top("settings_top").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Editor Preferences");
                ui.separator();
                ui.label("Search");
                ui.text_edit_singleline(&mut self.filter);
                ui.separator();
                ui.weak("No autosave.");
                if ui.button("Save Settings").clicked() {
                    stores.save_user();
                    stores.save_project();
                }
            });

            ui.separator();
            egui::CollapsingHeader::new("GPU Text (Live)")
                .default_open(true)
                .show(ui, |ui| {
                    let mut t = egui_wgpu::sdf::gpu_text_tuning_get();
                    let mut changed = false;

                    ui.label("实时预览：拖动立刻生效。字太粗就把 weight 调大（更细）。");

                    ui.horizontal(|ui| {
                        ui.label("Font");
                        egui::ComboBox::from_id_source("gpu_text_font")
                            .selected_text(match t.sans_family {
                                1 => "Segoe UI",
                                2 => "Microsoft YaHei",
                                3 => "SimHei",
                                4 => "SimSun",
                                _ => "Auto",
                            })
                            .show_ui(ui, |ui| {
                                changed |=
                                    ui.selectable_value(&mut t.sans_family, 0, "Auto").changed();
                                changed |= ui
                                    .selectable_value(&mut t.sans_family, 1, "Segoe UI")
                                    .changed();
                                changed |= ui
                                    .selectable_value(&mut t.sans_family, 2, "Microsoft YaHei")
                                    .changed();
                                changed |= ui
                                    .selectable_value(&mut t.sans_family, 3, "SimHei")
                                    .changed();
                                changed |= ui
                                    .selectable_value(&mut t.sans_family, 4, "SimSun")
                                    .changed();
                            });
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut t.content_weight, 0.6..=2.0)
                                    .text("weight(细)"),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut t.grayscale_enhanced_contrast, 0.0..=2.5)
                                    .text("contrast"),
                            )
                            .changed();
                        changed |= ui
                            .add(egui::Slider::new(&mut t.gamma, 1.0..=2.2).text("gamma"))
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut t.upload_budget, 0..=512)
                                    .text("upload budget/frame"),
                            )
                            .changed();
                        ui.weak("展开大量 UI 时卡顿就调小；0=暂停上传(字会延迟补齐)");
                    });

                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(egui::Slider::new(&mut t.scale, 0.5..=1.5).text("scale"))
                            .changed();
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut t.line_height_factor, 1.0..=2.0)
                                    .text("line height"),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut t.y_offset_points, -2.0..=2.0)
                                    .text("y offset"),
                            )
                            .changed();
                    });

                    ui.horizontal(|ui| {
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut t.scissor_pad_y, 0..=16)
                                    .text("scissor pad y(px)"),
                            )
                            .changed();
                        changed |= ui.checkbox(&mut t.row_bounds_clamp, "row clamp").changed();
                        ui.checkbox(&mut t.load_system_fonts, "load system fonts (restart)");
                    });

                    ui.horizontal(|ui| {
                        if ui.button("Preset: OS-like (thinner)").clicked() {
                            t.content_weight = 1.25;
                            t.grayscale_enhanced_contrast = 1.0;
                            t.gamma = 1.8;
                            changed = true;
                        }
                        if ui.button("Reset").clicked() {
                            t = egui_wgpu::sdf::GpuTextTuning {
                                scale: 1.0,
                                line_height_factor: 1.25,
                                y_offset_points: 0.0,
                                gamma: 1.8,
                                grayscale_enhanced_contrast: 1.0,
                                content_weight: 1.15,
                                sans_family: 0,
                                upload_budget: 96,
                                scissor_pad_y: 4,
                                row_bounds_clamp: true,
                                load_system_fonts: false,
                            };
                            changed = true;
                        }
                    });

                    if changed {
                        egui_wgpu::sdf::gpu_text_tuning_set(t);
                        ui.ctx().request_repaint();
                    }
                });

            egui::CollapsingHeader::new("Node Graph Text LOD (Live)")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label("控制节点标题在 zoom out 时何时开始淡出/何时隐藏（原来写死 5.0）。");
                    let hide_id = "node_editor.nodes.typography.title_lod_hide_px".to_string();
                    let fade_id = "node_editor.nodes.typography.title_lod_fade_px".to_string();
                    let mut hide = stores
                        .user
                        .get(&hide_id)
                        .cloned()
                        .unwrap_or(SettingValue::F32(5.0));
                    let mut fade = stores
                        .user
                        .get(&fade_id)
                        .cloned()
                        .unwrap_or(SettingValue::F32(4.0));
                    let mut ch = false;
                    if let SettingValue::F32(v) = &mut hide {
                        ch |= ui
                            .add(egui::Slider::new(v, 0.0..=32.0).text("hide px"))
                            .changed();
                    }
                    if let SettingValue::F32(v) = &mut fade {
                        ch |= ui
                            .add(egui::Slider::new(v, 0.1..=32.0).text("fade range px"))
                            .changed();
                    }
                    if ch {
                        stores.user.set(hide_id, hide);
                        stores.user.set(fade_id, fade);
                        ui.ctx().request_repaint();
                    }
                });
        });

        egui::SidePanel::left("settings_left")
            .resizable(true)
            .default_width(280.0)
            .show_inside(ui, |ui| {
                // PERF: rebuilding this tree every frame makes collapsing/expanding (animated) headers hitch badly.
                // Only rebuild when the filter changes.
                if self.cached_tree.is_empty() || self.cached_filter != self.filter {
                    self.rebuild_tree_cache(reg);
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        for (a, bmap) in self.cached_tree.iter() {
                            egui::CollapsingHeader::new(a)
                                .default_open(true)
                                .show(ui, |ui| {
                                    for (b, ids) in bmap.iter() {
                                        egui::CollapsingHeader::new(b).default_open(true).show(
                                            ui,
                                            |ui| {
                                                for id in ids {
                                                    let Some(meta) = reg.get(id) else {
                                                        continue;
                                                    };
                                                    let sel = self.selected.as_deref()
                                                        == Some(id.as_str());
                                                    if ui
                                                        .selectable_label(sel, &meta.label)
                                                        .clicked()
                                                    {
                                                        self.selected = Some(id.clone());
                                                    }
                                                }
                                            },
                                        );
                                    }
                                });
                        }
                    });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let Some(id) = self.selected.clone() else {
                ui.label("Select a setting on the left.");
                return;
            };
            let Some(meta) = reg.get(&id) else {
                ui.label("Missing setting meta.");
                return;
            };

            let pv = stores.project.get(&id).cloned();
            let uv = stores.user.get(&id).cloned();
            let (src, resolved) = SettingsMerge::resolve(meta, pv.as_ref(), uv.as_ref());
            let overridden =
                matches!(meta.scope, SettingScope::Both) && pv.is_some() && uv.is_some();

            ui.heading(&meta.label);
            ui.label(format!("Path: {}", meta.path));
            ui.label(format!("Id: {}", meta.id));
            if !meta.help.is_empty() {
                ui.label(&meta.help);
            }
            ui.separator();
            ui.label(format!(
                "Scope: {:?} · Effective Source: {}",
                meta.scope, src
            ));
            if overridden {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Overridden by Project Settings (read-only).",
                );
            }

            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.strong("Effective");
                    ui.label(Self::value_label(&resolved));
                })
            });
            ui.separator();

            let mut changed_any = false;
            ui.strong("Editor Preferences (User)");
            if !matches!(meta.scope, SettingScope::User | SettingScope::Both) {
                ui.weak("Not applicable.");
                return;
            }
            if let Some(v) = &uv {
                ui.label(format!("Current: {}", Self::value_label(v)));
            } else {
                ui.weak("Current: <default>");
            }
            let mut edit = uv.clone().unwrap_or_else(|| meta.default.clone());
            let mut edit_enabled = true;
            if matches!(meta.scope, SettingScope::Both) && pv.is_some() {
                edit_enabled = false;
            }
            ui.add_enabled_ui(edit_enabled, |ui| {
                if ui.button("Reset").clicked() {
                    stores.user.remove(&id);
                    changed_any = true;
                }
                if Self::draw_value_editor(ui, &mut edit, meta.min, meta.max, meta.step) {
                    stores.user.set(id.clone(), edit);
                    changed_any = true;
                }
            });

            if changed_any {
                Self::apply_ui_settings(meta, stores, reg, cx.ui_settings);
            }
        });

        if restore_style {
            ui.ctx().set_style(old_style);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
