//! File importer module - supports multiple 3D formats
pub mod fbx;
pub mod gltf_loader;
pub mod obj;
pub mod ply;
pub mod stl;
pub mod vox;

use crate::libs::geometry::mesh::Geometry;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

/// File metadata (quick preview)
#[derive(Debug, Clone, Default)]
pub struct FileMetadata {
    pub point_count: usize,
    pub primitive_count: usize,
    pub has_uv: bool,
    pub has_normals: bool,
    pub has_colors: bool,
}

/// Importer Trait
pub trait FileImporter: Send + Sync {
    fn extensions(&self) -> &[&str];
    fn import(&self, path: &Path) -> Result<Geometry, String>;
    fn peek(&self, _path: &Path) -> Option<FileMetadata> {
        None
    }
}

/// Global importer registry
static REGISTRY: OnceLock<ImporterRegistry> = OnceLock::new();

pub fn get_registry() -> &'static ImporterRegistry {
    REGISTRY.get_or_init(ImporterRegistry::new)
}

pub struct ImporterRegistry {
    importers: Vec<Box<dyn FileImporter>>,
    extension_map: HashMap<String, usize>,
}

impl ImporterRegistry {
    pub fn new() -> Self {
        let mut reg = Self {
            importers: Vec::new(),
            extension_map: HashMap::new(),
        };
        reg.register(Box::new(fbx::FbxImporter));
        reg.register(Box::new(obj::ObjImporter));
        reg.register(Box::new(gltf_loader::GltfImporter));
        reg.register(Box::new(stl::StlImporter));
        reg.register(Box::new(ply::PlyImporter));
        reg.register(Box::new(vox::VoxImporter));
        reg
    }

    fn register(&mut self, importer: Box<dyn FileImporter>) {
        let idx = self.importers.len();
        for ext in importer.extensions() {
            self.extension_map.insert(ext.to_lowercase(), idx);
        }
        self.importers.push(importer);
    }

    pub fn import(&self, path: &Path) -> Result<Geometry, String> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let idx = self
            .extension_map
            .get(&ext)
            .ok_or_else(|| format!("Unsupported format: {}", ext))?;
        self.importers[*idx].import(path)
    }

    pub fn supported_extensions(&self) -> Vec<&str> {
        self.importers
            .iter()
            .flat_map(|i| i.extensions().iter().copied())
            .collect()
    }

    pub fn peek(&self, path: &Path) -> Option<FileMetadata> {
        let ext = path.extension().and_then(|e| e.to_str())?.to_lowercase();
        let idx = self.extension_map.get(&ext)?;
        self.importers[*idx].peek(path)
    }
}
