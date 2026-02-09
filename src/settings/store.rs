use crate::settings::types::{SettingId, SettingMeta, SettingScope, SettingValue};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SettingsStore {
    pub values: HashMap<SettingId, SettingValue>,
}

impl SettingsStore {
    pub fn get(&self, id: &str) -> Option<&SettingValue> {
        self.values.get(id)
    }
    pub fn set(&mut self, id: SettingId, v: SettingValue) {
        self.values.insert(id, v);
    }
    pub fn remove(&mut self, id: &str) {
        self.values.remove(id);
    }
}

#[derive(Clone, Debug, Default)]
pub struct SettingsMerge;

impl SettingsMerge {
    pub fn resolve(
        meta: &SettingMeta,
        project: Option<&SettingValue>,
        user: Option<&SettingValue>,
    ) -> (&'static str, SettingValue) {
        match meta.scope {
            SettingScope::Project => (
                "project",
                project.cloned().unwrap_or_else(|| meta.default.clone()),
            ),
            SettingScope::User => (
                "user",
                user.cloned().unwrap_or_else(|| meta.default.clone()),
            ),
            SettingScope::Both => {
                if let Some(v) = project {
                    return ("project", v.clone());
                }
                if let Some(v) = user {
                    return ("user", v.clone());
                }
                ("default", meta.default.clone())
            }
        }
    }
}

#[derive(Resource, Clone, PartialEq)]
pub struct SettingsStores {
    pub user: SettingsStore,
    pub project: SettingsStore,
    pub project_root: PathBuf,
}

impl Default for SettingsStores {
    fn default() -> Self {
        Self {
            user: SettingsStore::default(),
            project: SettingsStore::default(),
            project_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

impl SettingsStores {
    fn user_path() -> PathBuf {
        let base = std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        base.join("Cunning3D").join("settings.ron")
    }

    fn project_path(root: &Path) -> PathBuf {
        root.join(".cunning3d").join("settings.ron")
    }

    pub fn load(&mut self) {
        self.user = load_store(Self::user_path()).unwrap_or_default();
        self.project = load_store(Self::project_path(&self.project_root)).unwrap_or_default();
    }

    pub fn save_user(&self) {
        let _ = save_store(Self::user_path(), &self.user);
    }
    pub fn save_project(&self) {
        let _ = save_store(Self::project_path(&self.project_root), &self.project);
    }
}

fn store_hash(s: &SettingsStore) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut acc = 0u64;
    for (k, v) in &s.values {
        let mut h = DefaultHasher::new();
        k.hash(&mut h);
        match v {
            SettingValue::Bool(x) => x.hash(&mut h),
            SettingValue::I64(x) => x.hash(&mut h),
            SettingValue::F32(x) => x.to_bits().hash(&mut h),
            SettingValue::String(x) => x.hash(&mut h),
            SettingValue::Color32(x) => x.hash(&mut h),
            SettingValue::Vec2(x) => {
                x[0].to_bits().hash(&mut h);
                x[1].to_bits().hash(&mut h);
            }
            SettingValue::Enum(x) => x.hash(&mut h),
        }
        acc ^= h.finish().rotate_left((acc as u32) & 63);
    }
    acc
}

pub fn autosave_settings_stores(
    time: Res<Time>,
    s: Res<SettingsStores>,
    mut last_t: Local<f64>,
    mut last_h: Local<u64>,
) {
    let now = time.elapsed_secs_f64();
    if now - *last_t < 0.5 {
        return;
    }
    let h = store_hash(&s.user) ^ store_hash(&s.project).rotate_left(1);
    if *last_h == h {
        return;
    }
    *last_t = now;
    *last_h = h;
    s.save_user();
    s.save_project();
}

fn load_store(path: PathBuf) -> Option<SettingsStore> {
    let txt = std::fs::read_to_string(path).ok()?;
    ron::from_str(&txt).ok()
}

fn save_store(path: PathBuf, store: &SettingsStore) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let txt = ron::ser::to_string_pretty(store, ron::ser::PrettyConfig::new().depth_limit(8))
        .unwrap_or_else(|_| "(".to_string() + ")");
    std::fs::write(path, txt)
}
