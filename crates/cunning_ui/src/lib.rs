use egui::Ui;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UiParamUiConfig {
    #[serde(default)] pub visible: bool,
    #[serde(default)] pub enabled: bool,
    #[serde(default)] pub tooltip: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiDropdownItem { pub value: i32, pub label: String }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum UiParamType {
    Float { min: f32, max: f32, #[serde(default)] logarithmic: bool },
    Int { min: i32, max: i32 },
    Bool,
    Toggle,
    Vec2,
    Vec3,
    Vec4,
    Color { #[serde(default)] has_alpha: bool },
    String,
    Angle,
    Dropdown { #[serde(default)] items: Vec<UiDropdownItem> },
    FilePath { #[serde(default)] filters: Vec<String> },
    Button,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiParamDef {
    pub name: String,
    #[serde(default)] pub label: String,
    #[serde(default)] pub group: String,
    #[serde(default)] pub order: i32,
    pub param_type: UiParamType,
    #[serde(default)] pub ui: UiParamUiConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum UiParamValue {
    Float(f32),
    Int(i32),
    Bool(bool),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Color([f32; 4]),
    String(String),
}

impl UiParamValue {
    pub fn default_for(def: &UiParamDef) -> Self {
        match &def.param_type {
            UiParamType::Float { min, .. } => UiParamValue::Float(*min),
            UiParamType::Angle => UiParamValue::Float(0.0),
            UiParamType::Int { min, .. } => UiParamValue::Int(*min),
            UiParamType::Bool | UiParamType::Toggle => UiParamValue::Bool(false),
            UiParamType::Vec2 => UiParamValue::Vec2([0.0, 0.0]),
            UiParamType::Vec3 => UiParamValue::Vec3([0.0, 0.0, 0.0]),
            UiParamType::Vec4 => UiParamValue::Vec4([0.0, 0.0, 0.0, 0.0]),
            UiParamType::Color { has_alpha } => UiParamValue::Color([0.0, 0.0, 0.0, if *has_alpha { 1.0 } else { 1.0 }]),
            UiParamType::String | UiParamType::FilePath { .. } => UiParamValue::String(String::new()),
            UiParamType::Dropdown { items } => UiParamValue::Int(items.first().map(|i| i.value).unwrap_or(0)),
            UiParamType::Button => UiParamValue::Bool(false),
        }
    }
}

pub fn draw_param(ui: &mut Ui, def: &UiParamDef, v: &mut UiParamValue) -> bool {
    if !def.ui.visible { return false; }
    let enabled = def.ui.enabled;
    let label = if def.label.is_empty() { def.name.as_str() } else { def.label.as_str() };
    let mut changed = false;

    let r = ui.add_enabled_ui(enabled, |ui| {
        match (&def.param_type, v) {
            (UiParamType::Float { min, max, logarithmic }, UiParamValue::Float(x)) => {
                let mut w = egui::Slider::new(x, *min..=*max).text(label);
                if *logarithmic { w = w.logarithmic(true); }
                ui.add(w).changed()
            }
            (UiParamType::Angle, UiParamValue::Float(x)) => ui.add(egui::Slider::new(x, -180.0..=180.0).text(label)).changed(),
            (UiParamType::Int { min, max }, UiParamValue::Int(x)) => ui.add(egui::Slider::new(x, *min..=*max).text(label)).changed(),
            (UiParamType::Bool | UiParamType::Toggle, UiParamValue::Bool(x)) => ui.checkbox(x, label).changed(),
            (UiParamType::Vec2, UiParamValue::Vec2(a)) => ui.horizontal(|ui| {
                ui.label(label);
                let c0 = ui.add(egui::DragValue::new(&mut a[0]).speed(0.1)).changed();
                let c1 = ui.add(egui::DragValue::new(&mut a[1]).speed(0.1)).changed();
                c0 || c1
            }).inner,
            (UiParamType::Vec3, UiParamValue::Vec3(a)) => ui.horizontal(|ui| {
                ui.label(label);
                let c0 = ui.add(egui::DragValue::new(&mut a[0]).speed(0.1)).changed();
                let c1 = ui.add(egui::DragValue::new(&mut a[1]).speed(0.1)).changed();
                let c2 = ui.add(egui::DragValue::new(&mut a[2]).speed(0.1)).changed();
                c0 || c1 || c2
            }).inner,
            (UiParamType::Vec4, UiParamValue::Vec4(a)) => ui.horizontal(|ui| {
                ui.label(label);
                let c0 = ui.add(egui::DragValue::new(&mut a[0]).speed(0.1)).changed();
                let c1 = ui.add(egui::DragValue::new(&mut a[1]).speed(0.1)).changed();
                let c2 = ui.add(egui::DragValue::new(&mut a[2]).speed(0.1)).changed();
                let c3 = ui.add(egui::DragValue::new(&mut a[3]).speed(0.1)).changed();
                c0 || c1 || c2 || c3
            }).inner,
            (UiParamType::Color { has_alpha }, UiParamValue::Color(c)) => {
                if *has_alpha {
                    let mut col = egui::Color32::from_rgba_unmultiplied((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, (c[3] * 255.0) as u8);
                    let resp = ui.color_edit_button_srgba(&mut col);
                    if resp.changed() {
                        c[0] = col.r() as f32 / 255.0; c[1] = col.g() as f32 / 255.0; c[2] = col.b() as f32 / 255.0; c[3] = col.a() as f32 / 255.0;
                        true
                    } else { false }
                } else {
                    let mut rgb = [c[0], c[1], c[2]];
                    let resp = ui.color_edit_button_rgb(&mut rgb);
                    if resp.changed() { c[0] = rgb[0]; c[1] = rgb[1]; c[2] = rgb[2]; true } else { false }
                }
            }
            (UiParamType::String | UiParamType::FilePath { .. }, UiParamValue::String(s)) => ui.add(egui::TextEdit::singleline(s).hint_text(label)).changed(),
            (UiParamType::Dropdown { items }, UiParamValue::Int(x)) => {
                let mut changed = false;
                egui::ComboBox::from_id_source((&def.name, "dropdown"))
                    .selected_text(items.iter().find(|i| i.value == *x).map(|i| i.label.as_str()).unwrap_or(label))
                    .show_ui(ui, |ui| {
                        for it in items {
                            if ui.selectable_value(x, it.value, &it.label).changed() { changed = true; }
                        }
                    });
                changed
            }
            (UiParamType::Button, UiParamValue::Bool(b)) => { if ui.button(label).clicked() { *b = true; true } else { false } }
            _ => { ui.label(label); false }
        }
    });
    changed |= r.inner;
    if let Some(t) = &def.ui.tooltip { if r.response.hovered() { r.response.on_hover_text(t); } }
    changed
}

