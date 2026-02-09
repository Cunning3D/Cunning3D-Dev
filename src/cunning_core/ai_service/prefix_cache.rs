//! Stable prefix cache (string/JSON) for cloud models (not KV cache).

use super::context_config as cfg;
use crate::tabs_registry::ai_workspace::tools::ToolDefinition;
use serde_json::Value;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

static DOC_CACHE: OnceLock<Option<String>> = OnceLock::new();
static TOOLS_CACHE: OnceLock<Mutex<HashMap<u64, Value>>> = OnceLock::new();

pub struct StablePrefixCache;

impl StablePrefixCache {
    fn gemini_schema_from_json_schema(v: &Value) -> Value {
        let Some(obj) = v.as_object() else { return serde_json::json!({ "type": "OBJECT" }); };
        let t = obj.get("type").and_then(|x| x.as_str()).unwrap_or("object").to_ascii_lowercase();
        let type_enum = match t.as_str() {
            "object" => "OBJECT",
            "string" => "STRING",
            "number" => "NUMBER",
            "integer" => "INTEGER",
            "boolean" => "BOOLEAN",
            "array" => "ARRAY",
            _ => "OBJECT",
        };
        let mut out = serde_json::Map::new();
        out.insert("type".to_string(), Value::String(type_enum.to_string()));
        if let Some(desc) = obj.get("description").and_then(|x| x.as_str()) {
            out.insert("description".to_string(), Value::String(desc.to_string()));
        }
        if let Some(req) = obj.get("required").and_then(|x| x.as_array()) {
            out.insert("required".to_string(), Value::Array(req.clone()));
        }
        if type_enum == "OBJECT" {
            if let Some(props) = obj.get("properties").and_then(|x| x.as_object()) {
                let mut m = serde_json::Map::new();
                for (k, pv) in props {
                    m.insert(k.clone(), Self::gemini_schema_from_json_schema(pv));
                }
                out.insert("properties".to_string(), Value::Object(m));
            }
        }
        if type_enum == "ARRAY" {
            let items = obj
                .get("items")
                .map(Self::gemini_schema_from_json_schema)
                .unwrap_or_else(|| serde_json::json!({ "type": "OBJECT" }));
            out.insert("items".to_string(), items);
        }
        Value::Object(out)
    }

    pub fn project_doc() -> Option<String> {
        DOC_CACHE
            .get_or_init(|| {
                for p in cfg::PROJECT_DOC_FILES {
                    let path = std::path::Path::new(p);
                    if let Ok(bytes) = std::fs::read(path) {
                        let slice = &bytes[..bytes.len().min(cfg::PROJECT_DOC_MAX_BYTES)];
                        if let Ok(s) = std::str::from_utf8(slice) {
                            let t = s.trim();
                            if !t.is_empty() {
                                return Some(t.to_string());
                            }
                        }
                    }
                }
                None
            })
            .clone()
    }

    pub fn tool_declarations(mut tools: Vec<ToolDefinition>) -> Value {
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for t in &tools {
            t.name.hash(&mut hasher);
            t.description.hash(&mut hasher);
            let s = serde_json::to_string(&t.parameters).unwrap_or_default();
            s.hash(&mut hasher);
        }
        let key = hasher.finish();
        let map = TOOLS_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(mut m) = map.lock() {
            if let Some(v) = m.get(&key) {
                return v.clone();
            }
            let funcs: Vec<Value> = tools
                .into_iter()
                .map(|t| {
                    let params = Self::gemini_schema_from_json_schema(&t.parameters);
                    serde_json::json!({ "name": t.name, "description": t.description, "parameters": params })
                })
                .collect();
            let v = serde_json::json!([{ "function_declarations": funcs }]);
            if m.len() > 8 {
                m.clear();
            }
            m.insert(key, v.clone());
            return v;
        }
        serde_json::json!([])
    }
}
