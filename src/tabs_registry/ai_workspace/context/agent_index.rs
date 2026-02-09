use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use walkdir::WalkDir;

/// Index of available AI agents (node descriptions)
pub struct AgentIndex {
    // Map from NodeName to PathBuf of the _agent.toml file
    agents: HashMap<String, PathBuf>,
}

impl AgentIndex {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    /// Build the index by scanning assets/agents and plugins/
    pub fn build_index(&mut self, root_dir: &std::path::Path) {
        // 1. Scan assets/agents
        let assets_agents = root_dir.join("assets").join("agents");
        self.scan_dir(&assets_agents);

        // 2. Scan assets/knowledge (internal nodes, system knowledge)
        let knowledge_dir = root_dir.join("assets").join("knowledge");
        self.scan_dir(&knowledge_dir);

        // 3. Scan plugins
        let plugins = root_dir.join("plugins");
        self.scan_dir(&plugins);
    }

    fn scan_dir(&mut self, dir: &std::path::Path) {
        if !dir.exists() {
            return;
        }

        let walker = WalkDir::new(dir).max_depth(5).into_iter();
        for entry in walker.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                if let Some(fname) = path.file_name().and_then(|n| n.to_str()) {
                    if fname.ends_with("_agent.toml") {
                        // Extract NodeName from NodeName_agent.toml
                        let node_name = fname.trim_end_matches("_agent.toml").to_string();
                        self.agents.insert(node_name, path.to_path_buf());
                    }
                }
            }
        }
    }

    /// Find relevant agent descriptions based on code content
    pub fn find_relevant_agents(&self, code: &str) -> Vec<(String, String)> {
        let mut relevant = Vec::new();

        // Simple keyword matching: if NodeName appears in code, include its description
        // Optimized: only check for keys that are actually in the index
        for (name, path) in &self.agents {
            // Check if 'name' exists in code as a word boundary?
            // For Rhai, nodes are often used as function calls or types.
            // Simple substring check is fast enough for now, maybe too broad but okay for MVP.
            if code.contains(name) {
                if let Ok(content) = fs::read_to_string(path) {
                    relevant.push((name.clone(), content));
                }
            }
        }

        relevant
    }
}
