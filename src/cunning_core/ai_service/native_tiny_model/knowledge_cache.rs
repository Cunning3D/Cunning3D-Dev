use bevy::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use walkdir::WalkDir;

/// Global access to node knowledge, populated at startup.
/// Used by both Bevy systems (via Resource) and non-Bevy contexts (via static).
pub static GLOBAL_KNOWLEDGE_CACHE: OnceLock<KnowledgeCache> = OnceLock::new();
/// Cached full-knowledge blob for tiny-model prompts (built once at startup).
pub static GLOBAL_KNOWLEDGE_PROMPT_BLOB: OnceLock<String> = OnceLock::new();

#[derive(Resource, Clone, Default)]
pub struct KnowledgeCache {
    /// Map of "Node Name" -> Knowledge
    pub nodes: Arc<HashMap<String, NodeKnowledge>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeKnowledge {
    pub name: String,
    pub category: String,
    pub description: String,
    pub io: NodeIoKnowledge,
    #[serde(default)]
    pub parameters: HashMap<String, String>,
    pub usage: Option<NodeUsageKnowledge>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeIoKnowledge {
    pub input_type: String,
    pub output_type: String,
    #[serde(default)]
    pub inputs: Vec<NodePortKnowledge>,
    #[serde(default)]
    pub outputs: Vec<NodePortKnowledge>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodePortKnowledge {
    pub index: usize,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeUsageKnowledge {
    pub text: String,
}

impl KnowledgeCache {
    pub fn load_from_dir<P: AsRef<Path>>(dir: P) -> Self {
        let mut map = HashMap::new();
        let dir = dir.as_ref();

        if !dir.exists() {
            warn!("Knowledge directory not found: {:?}", dir);
            return Self::default();
        }

        info!("Loading AI Knowledge from: {:?}", dir);

        for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            if entry.path().extension().map_or(false, |ext| ext == "toml") {
                match std::fs::read_to_string(entry.path()) {
                    Ok(content) => match toml::from_str::<NodeKnowledge>(&content) {
                        Ok(knowledge) => {
                            map.insert(knowledge.name.clone(), knowledge);
                        }
                        Err(e) => {
                            warn!("Failed to parse knowledge file {:?}: {}", entry.path(), e);
                        }
                    },
                    Err(e) => {
                        warn!("Failed to read knowledge file {:?}: {}", entry.path(), e);
                    }
                }
            }
        }

        info!("Loaded {} knowledge entries.", map.len());
        Self {
            nodes: Arc::new(map),
        }
    }

    pub fn get(&self, name: &str) -> Option<&NodeKnowledge> {
        self.nodes.get(name)
    }

    pub fn build_prompt_blob(&self) -> String {
        let mut keys: Vec<&String> = self.nodes.keys().collect();
        keys.sort();
        let mut out = String::new();
        out.push_str("[Knowledge.AllowedNodes]\n");
        out.push_str(
            &keys
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push_str("\n\n[Knowledge.Nodes]\n");
        for k in keys {
            if let Some(n) = self.nodes.get(k) {
                let ins =
                    n.io.inputs
                        .iter()
                        .map(|p| format!("{}:{}", p.name, p.description))
                        .collect::<Vec<_>>()
                        .join(" | ");
                let outs =
                    n.io.outputs
                        .iter()
                        .map(|p| format!("{}:{}", p.name, p.description))
                        .collect::<Vec<_>>()
                        .join(" | ");
                out.push_str(&format!(
                    "- {} | {} | in_type={} out_type={} | in[{}] | out[{}]\n",
                    n.name, n.description, n.io.input_type, n.io.output_type, ins, outs
                ));
            }
        }
        out
    }

    /// Build a compact prompt blob filtered by categories (for reduced token usage)
    pub fn build_filtered_prompt(&self, categories: &[&str], start_node: Option<&str>) -> String {
        let mut out = String::new();
        let mut relevant: Vec<&NodeKnowledge> = self.nodes.values()
            .filter(|n| categories.iter().any(|c| n.category.eq_ignore_ascii_case(c))
                || start_node.map_or(false, |s| n.name == s))
            .collect();
        relevant.sort_by(|a, b| a.name.cmp(&b.name));
        out.push_str("[AllowedNodes]\n");
        out.push_str(&relevant.iter().map(|n| n.name.as_str()).collect::<Vec<_>>().join(","));
        out.push_str("\n\n[Nodes]\n");
        for n in &relevant {
            out.push_str(&format!("{}|{}|{}\n", n.name, n.category, n.description));
        }
        out
    }

    /// Get category of a node
    pub fn get_category(&self, name: &str) -> Option<&str> {
        self.nodes.get(name).map(|n| n.category.as_str())
    }

    /// Get related categories for a given category
    pub fn related_categories(cat: &str) -> Vec<&'static str> {
        match cat.to_lowercase().as_str() {
            "geometry" | "sop" => vec!["Geometry", "SOP", "Transform", "Modify", "Utility"],
            "transform" => vec!["Transform", "Geometry", "SOP"],
            "modify" => vec!["Modify", "Geometry", "SOP", "Transform"],
            "primitive" | "create" => vec!["Primitive", "Create", "Geometry", "SOP"],
            "utility" => vec!["Utility", "Geometry", "SOP", "Modify"],
            "sdf" => vec!["SDF", "Geometry", "Boolean"],
            "boolean" => vec!["Boolean", "SDF", "Geometry"],
            "attribute" => vec!["Attribute", "Geometry", "SOP", "Modify"],
            "curve" => vec!["Curve", "Geometry", "SOP"],
            "particle" | "points" => vec!["Particle", "Points", "Geometry", "SOP"],
            _ => vec!["Geometry", "SOP", "Utility"],
        }
    }
}

pub struct KnowledgeCachePlugin;

impl Plugin for KnowledgeCachePlugin {
    fn build(&self, app: &mut App) {
        // Load on startup
        let cache = KnowledgeCache::load_from_dir(crate::runtime_paths::assets_dir().join("knowledge"));
        let _ = GLOBAL_KNOWLEDGE_PROMPT_BLOB.set(cache.build_prompt_blob());

        // Set global static
        if GLOBAL_KNOWLEDGE_CACHE.set(cache.clone()).is_err() {
            warn!("GLOBAL_KNOWLEDGE_CACHE was already initialized!");
        }

        // Add as resource
        app.insert_resource(cache);
    }
}
