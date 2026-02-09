use crate::cunning_core::traits::pane_interface::PaneTab;
use bevy::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

// The factory function pointer
pub type PaneFactory = fn() -> Box<dyn PaneTab>;

// The struct we submit to inventory
pub struct TabDescriptor {
    pub name: &'static str,
    pub factory: PaneFactory,
}

// Tell inventory to collect these
inventory::collect!(TabDescriptor);

#[derive(Resource, Default, Clone)]
pub struct TabRegistry {
    // We keep a runtime map for fast lookups
    factories: Arc<RwLock<HashMap<String, PaneFactory>>>,
}

impl TabRegistry {
    /// Call this once at startup to gather all distributed registrations
    pub fn scan_and_load(&self) {
        let mut map = self.factories.write().unwrap();

        // Iterate over all submitted descriptors
        for descriptor in inventory::iter::<TabDescriptor> {
            info!("TabRegistry: Auto-discovered pane '{}'", descriptor.name);
            map.insert(descriptor.name.to_string(), descriptor.factory);
        }
    }

    pub fn create(&self, name: &str) -> Option<Box<dyn PaneTab>> {
        let map = self.factories.read().unwrap();
        map.get(name).map(|factory| factory())
    }

    pub fn list_names(&self) -> Vec<String> {
        let map = self.factories.read().unwrap();
        map.keys().cloned().collect()
    }
}

// Helper macro to make registration easier for the user
#[macro_export]
macro_rules! register_pane {
    ($name:expr, $type:ty) => {
        inventory::submit! {
            $crate::cunning_core::registries::tab_registry::TabDescriptor {
                name: $name,
                factory: || Box::new(<$type>::default()),
            }
        }
    };
}
