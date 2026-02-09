use serde::{Deserialize, Serialize};

pub type SettingId = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettingScope {
    User,
    Project,
    Both,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v")]
pub enum SettingValue {
    Bool(bool),
    I64(i64),
    F32(f32),
    String(String),
    Color32([u8; 4]),
    Vec2([f32; 2]),
    Enum(String),
}

#[derive(Clone, Debug)]
pub struct SettingMeta {
    pub id: SettingId, // stable key e.g. "ui.appearance.rounding"
    pub path: String,  // tree path e.g. "UI/Appearance"
    pub label: String, // display name
    pub help: String,  // one-liner
    pub scope: SettingScope,
    pub default: SettingValue,
    pub min: Option<f32>, // numeric constraints (optional)
    pub max: Option<f32>,
    pub step: Option<f32>,
    pub keywords: Vec<String>,
}

impl SettingMeta {
    pub fn key_path(&self) -> (&str, &str) {
        (&self.path, &self.id)
    }
}
