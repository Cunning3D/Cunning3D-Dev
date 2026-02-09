use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use bevy::prelude::Resource;
use once_cell::sync::OnceCell;
use uuid::Uuid;

use crate::console::global_console;
use crate::cunning_core::cda::{CDAAsset, CDAError};
use crate::libs::geometry::geo_ref::GeometryRef;
use crate::mesh::Geometry;
use crate::nodes::parameter::ParameterValue;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Hash)]
pub struct CdaAssetRef {
    pub uuid: Uuid,
    #[serde(default)]
    pub path: String, // empty => unsaved/in-memory
}

#[derive(Default)]
struct Inner {
    defs: HashMap<Uuid, CDAAsset>,
    paths: HashMap<Uuid, String>,
}

#[derive(Default, Resource, Clone)]
pub struct CdaLibrary(Arc<Mutex<Inner>>);

static GLOBAL_CDA_LIB: OnceCell<CdaLibrary> = OnceCell::new();
pub fn global_cda_library() -> Option<&'static CdaLibrary> {
    GLOBAL_CDA_LIB.get()
}
pub fn init_global_cda_library(lib: bevy::prelude::Res<CdaLibrary>) {
    let _ = GLOBAL_CDA_LIB.set(lib.clone());
}

impl CdaLibrary {
    pub fn get(&self, id: Uuid) -> Option<CDAAsset> {
        self.0.lock().ok().and_then(|g| g.defs.get(&id).cloned())
    }
    pub fn put(&self, a: CDAAsset) {
        if let Ok(mut g) = self.0.lock() {
            g.defs.insert(a.id, a);
        }
    }
    pub fn put_with_path(&self, a: CDAAsset, path: String) {
        if let Ok(mut g) = self.0.lock() {
            g.paths.insert(a.id, path);
            g.defs.insert(a.id, a);
        }
    }
    pub fn path_for(&self, id: Uuid) -> Option<String> {
        self.0.lock().ok().and_then(|g| g.paths.get(&id).cloned())
    }
    pub fn list_defs(&self) -> Vec<(Uuid, String)> {
        let Ok(g) = self.0.lock() else {
            return Vec::new();
        };
        let mut v: Vec<(Uuid, String)> = g.defs.values().map(|a| (a.id, a.name.clone())).collect();
        v.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        v
    }

    pub fn def_guard(&self, id: Uuid) -> Option<CdaDefGuard<'_>> {
        let g = self.0.lock().ok()?;
        if !g.defs.contains_key(&id) {
            return None;
        }
        Some(CdaDefGuard { g, id })
    }

    pub fn ensure_loaded(&self, r: &CdaAssetRef) -> Result<(), CDAError> {
        if self
            .0
            .lock()
            .ok()
            .map(|g| g.defs.contains_key(&r.uuid))
            .unwrap_or(false)
        {
            return Ok(());
        }
        if r.path.is_empty() {
            return Err(CDAError::MissingChunk);
        }
        let a = CDAAsset::load_dcc(&r.path)?;
        if a.id != r.uuid && global_console().is_some() {
            if let Some(c) = global_console() {
                c.warning(format!(
                    "CDA library: uuid mismatch for {} (ref={}, file={})",
                    r.path, r.uuid, a.id
                ));
            }
        }
        self.put_with_path(a, r.path.clone());
        Ok(())
    }

    pub fn insert_in_memory(&self, a: CDAAsset) -> CdaAssetRef {
        let id = a.id;
        self.put(a);
        CdaAssetRef {
            uuid: id,
            path: String::new(),
        }
    }

    pub fn cook(
        &self,
        instance_node_id: Option<Uuid>,
        r: &CdaAssetRef,
        overrides: &HashMap<String, ParameterValue>,
        inputs: &[Arc<dyn GeometryRef>],
        registry: &crate::cunning_core::registries::node_registry::NodeRegistry,
    ) -> Vec<Arc<Geometry>> {
        let _ = instance_node_id;
        if self.ensure_loaded(r).is_err() {
            return vec![Arc::new(Geometry::new())];
        }
        let (asset, out_len) = match self.0.lock().ok().and_then(|g| {
            g.defs
                .get(&r.uuid)
                .cloned()
                .map(|a| (a.clone(), a.outputs.len().max(1)))
        }) {
            Some(v) => v,
            None => return vec![Arc::new(Geometry::new())],
        };
        let mut channel_values: HashMap<String, Vec<f64>> = HashMap::new();
        for (k, v) in overrides {
            let ch = crate::cunning_core::cda::utils::value_to_channels(v);
            if !ch.is_empty() {
                channel_values.insert(k.clone(), ch);
            }
        }
        let mats: Vec<Arc<Geometry>> = inputs.iter().map(|g| Arc::new(g.materialize())).collect();
        let outs = asset.evaluate_outputs_cached(&channel_values, mats.as_slice(), registry);
        if outs.is_empty() {
            vec![Arc::new(Geometry::new()); out_len]
        } else {
            outs
        }
    }

    pub fn with_defs_mut<R>(&self, f: impl FnOnce(&mut HashMap<Uuid, CDAAsset>) -> R) -> Option<R> {
        let mut g = self.0.lock().ok()?;
        Some(f(&mut g.defs))
    }
}

pub struct CdaDefGuard<'a> {
    g: MutexGuard<'a, Inner>,
    id: Uuid,
}

impl<'a> CdaDefGuard<'a> {
    pub fn asset(&self) -> &CDAAsset {
        self.g.defs.get(&self.id).expect("CDAAsset missing")
    }
    pub fn asset_mut(&mut self) -> &mut CDAAsset {
        self.g.defs.get_mut(&self.id).expect("CDAAsset missing")
    }
    pub fn graph(&self) -> &crate::nodes::structs::NodeGraph {
        &self.asset().inner_graph
    }
    pub fn graph_mut(&mut self) -> &mut crate::nodes::structs::NodeGraph {
        &mut self.asset_mut().inner_graph
    }
}
