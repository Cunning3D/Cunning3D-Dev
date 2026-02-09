use crate::settings::types::{SettingId, SettingMeta};
use bevy::prelude::*;
use std::collections::HashMap;

pub type RegisterFn = fn(&mut SettingsRegistry);

pub struct SettingsDescriptor {
    pub name: &'static str,
    pub register: RegisterFn,
}

inventory::collect!(SettingsDescriptor);

#[macro_export]
macro_rules! register_settings_provider {
    ($name:expr, $register:path) => {
        inventory::submit! {
            $crate::settings::registry::SettingsDescriptor {
                name: $name,
                register: $register,
            }
        }
    };
}

#[derive(Resource, Default, Clone)]
pub struct SettingsRegistry {
    metas: HashMap<SettingId, SettingMeta>,
    order: Vec<SettingId>,
}

impl SettingsRegistry {
    pub fn scan_and_load(&mut self) {
        for d in inventory::iter::<SettingsDescriptor> {
            info!("SettingsRegistry: provider '{}'", d.name);
            (d.register)(self);
        }
    }

    pub fn upsert(&mut self, meta: SettingMeta) {
        let id = meta.id.clone();
        if !self.metas.contains_key(&id) {
            self.order.push(id.clone());
        }
        self.metas.insert(id, meta);
    }

    pub fn get(&self, id: &str) -> Option<&SettingMeta> {
        self.metas.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SettingMeta> {
        self.order.iter().filter_map(|k| self.metas.get(k))
    }
}
