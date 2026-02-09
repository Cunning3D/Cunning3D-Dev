//! Knowledge tools backed by an encrypted knowledge pack (assets/knowledge.pack).
//! UI should show excerpts only; full content is returned via ToolOutput.llm_text.

use super::definitions::{Tool, ToolContext, ToolDefinition, ToolError, ToolLog, ToolLogLevel, ToolOutput};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::XChaCha20Poly1305;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs, path::{Path, PathBuf}, sync::OnceLock};

fn pack_path() -> PathBuf { crate::runtime_paths::assets_dir().join("knowledge.pack") }
fn knowledge_dir() -> PathBuf { crate::runtime_paths::assets_dir().join("knowledge") }
const MAGIC: &[u8; 8] = b"C3DKNOW\0";
const VERSION: u32 = 1;

fn key_from_env() -> Result<[u8; 32], ToolError> {
    let s = std::env::var("CUNNING_KNOWLEDGE_KEY").map_err(|_| ToolError("Missing env var: CUNNING_KNOWLEDGE_KEY".into()))?;
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    Ok(h.finalize().into())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct IndexEntry {
    path: String,
    offset: u32,
    len: u32,
    sha256_hex: String,
}

#[derive(Clone, Debug, Default)]
struct KnowledgeStore {
    blob: Vec<u8>,
    index: Vec<IndexEntry>,
    by_path: HashMap<String, usize>,
}

impl KnowledgeStore {
    fn load() -> Result<Self, ToolError> {
        let pack = pack_path();
        if pack.exists() {
            return Self::load_pack(&pack);
        }
        // Dev fallback: load plaintext files directly (no encryption).
        Self::load_dir(&knowledge_dir())
    }

    fn load_pack(path: &Path) -> Result<Self, ToolError> {
        let bytes = fs::read(path).map_err(|e| ToolError(format!("Failed to read knowledge pack: {e}")))?;
        if bytes.len() < 8 + 4 + 24 + 4 {
            return Err(ToolError("Invalid knowledge pack: too small".into()));
        }
        if &bytes[..8] != MAGIC {
            return Err(ToolError("Invalid knowledge pack: bad magic".into()));
        }
        let ver = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if ver != VERSION {
            return Err(ToolError(format!("Unsupported knowledge pack version: {ver}")));
        }
        let nonce: [u8; 24] = bytes[12..36].try_into().unwrap();
        let idx_len = u32::from_le_bytes(bytes[36..40].try_into().unwrap()) as usize;
        if bytes.len() < 40 + idx_len {
            return Err(ToolError("Invalid knowledge pack: truncated index".into()));
        }
        let ct = &bytes[40..40 + idx_len];
        let key = key_from_env()?;
        let cipher = XChaCha20Poly1305::new((&key).into());
        let idx_json = cipher.decrypt((&nonce).into(), ct).map_err(|_| ToolError("Failed to decrypt knowledge index".into()))?;
        let index: Vec<IndexEntry> = serde_json::from_slice(&idx_json).map_err(|e| ToolError(format!("Invalid knowledge index JSON: {e}")))?;
        let blob = bytes[40 + idx_len..].to_vec();
        let by_path = index.iter().enumerate().map(|(i, e)| (e.path.clone(), i)).collect();
        Ok(Self { blob, index, by_path })
    }

    fn load_dir(root: &Path) -> Result<Self, ToolError> {
        if !root.exists() {
            return Err(ToolError(format!("Knowledge dir not found: {}", root.display())));
        }
        let mut store = KnowledgeStore::default();
        for entry in walkdir::WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
            let data = fs::read(path).unwrap_or_default();
            let offset = store.blob.len() as u32;
            let len = data.len() as u32;
            store.blob.extend_from_slice(&data);
            let mut h = Sha256::new();
            h.update(&data);
            let sha256_hex = format!("{:x}", h.finalize());
            let idx = store.index.len();
            store.index.push(IndexEntry { path: rel.clone(), offset, len, sha256_hex });
            store.by_path.insert(rel, idx);
        }
        Ok(store)
    }

    fn list(&self) -> Vec<String> {
        let mut v = self.index.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
        v.sort();
        v
    }

    fn read(&self, rel_path: &str) -> Result<Vec<u8>, ToolError> {
        let rel = rel_path.trim().replace('\\', "/");
        let Some(&ix) = self.by_path.get(&rel) else {
            return Err(ToolError(format!("Knowledge path not found: {rel}")));
        };
        let e = &self.index[ix];
        let start = e.offset as usize;
        let end = start + e.len as usize;
        if end > self.blob.len() {
            return Err(ToolError("Knowledge pack corrupted: entry out of bounds".into()));
        }
        Ok(self.blob[start..end].to_vec())
    }

    fn search(&self, q: &str, limit: usize, prefix: Option<&str>) -> Vec<(String, String)> {
        let q = q.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let prefix = prefix.map(|p| p.trim().trim_matches('/').to_lowercase());
        let mut hits = Vec::new();
        for e in &self.index {
            if hits.len() >= limit {
                break;
            }
            if let Some(p) = &prefix {
                if !e.path.to_lowercase().starts_with(p) {
                    continue;
                }
            }
            if !e.path.to_lowercase().contains(&q) {
                continue;
            }
            hits.push((e.path.clone(), String::new()));
        }
        // If path hits are too few, do content substring scan (bounded).
        if hits.len() < limit {
            for e in &self.index {
                if hits.len() >= limit {
                    break;
                }
                if let Some(p) = &prefix {
                    if !e.path.to_lowercase().starts_with(p) {
                        continue;
                    }
                }
                if hits.iter().any(|(p, _)| p == &e.path) {
                    continue;
                }
                let Ok(bytes) = self.read(&e.path) else { continue; };
                let Ok(text) = String::from_utf8(bytes) else { continue; };
                let tl = text.to_lowercase();
                if let Some(pos) = tl.find(&q) {
                    let start = pos.saturating_sub(80);
                    let end = (pos + q.len() + 120).min(tl.len());
                    let snippet = text.chars().skip(start).take(end - start).collect::<String>();
                    hits.push((e.path.clone(), snippet));
                }
            }
        }
        hits
    }
}

static STORE: OnceLock<KnowledgeStore> = OnceLock::new();
fn store() -> Result<&'static KnowledgeStore, ToolError> {
    if let Some(s) = STORE.get() {
        return Ok(s);
    }
    let s = KnowledgeStore::load()?;
    let _ = STORE.set(s);
    STORE
        .get()
        .ok_or_else(|| ToolError("Knowledge store init failed".into()))
}

fn ui_excerpt(s: &str, max: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= max { t.to_string() } else { t.chars().take(max).collect::<String>() + "…" }
}

// -----------------------------------------------------------------------------
// search_knowledge
// -----------------------------------------------------------------------------

#[derive(Deserialize)]
struct SearchKnowledgeArgs {
    query: String,
    #[serde(default = "d20")]
    limit: usize,
    #[serde(default)]
    path_prefix: Option<String>,
}
fn d20() -> usize { 20 }

pub struct SearchKnowledgeTool;
impl Tool for SearchKnowledgeTool {
    fn name(&self) -> &str { "search_knowledge" }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Search encrypted knowledge pack (assets/knowledge). Returns matching paths and short snippets.".to_string(),
            parameters: json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","default":20},"path_prefix":{"type":"string","description":"Optional prefix like 'spec/nodes' or 'internal_nodes'"}}, "required":["query"]}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        self.execute_with_context(args, &ToolContext::default())
    }
    fn execute_with_context(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let a: SearchKnowledgeArgs = serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid args: {e}")))?;
        if ctx.is_cancelled() { return Err(ToolError("Cancelled".into())); }
        let s = store()?;
        let hits = s.search(&a.query, a.limit.min(200).max(1), a.path_prefix.as_deref());
        let items: Vec<Value> = hits.iter().map(|(p, snip)| json!({"path": p, "snippet": ui_excerpt(snip, 160)})).collect();
        let raw = serde_json::to_string_pretty(&json!({"count": items.len(), "items": items})).unwrap_or_else(|_| "{}".into());
        Ok(ToolOutput::with_summary(
            format!("Found {} knowledge hits.", hits.len()),
            raw,
        ))
    }
}

// -----------------------------------------------------------------------------
// read_knowledge
// -----------------------------------------------------------------------------

#[derive(Deserialize)]
struct ReadKnowledgeArgs {
    path: String,
    #[serde(default = "d8000")]
    max_chars: usize,
}
fn d8000() -> usize { 8000 }

pub struct ReadKnowledgeTool;
impl Tool for ReadKnowledgeTool {
    fn name(&self) -> &str { "read_knowledge" }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Read a knowledge document by relative path under assets/knowledge. Full content goes to model; UI shows excerpt only.".to_string(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"e.g. 'internal_nodes/CreateCube_agent.toml' or 'spec/nodes/CreateCube.spec.toml'"},"max_chars":{"type":"integer","default":8000}}, "required":["path"]}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        self.execute_with_context(args, &ToolContext::default())
    }
    fn execute_with_context(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let a: ReadKnowledgeArgs = serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid args: {e}")))?;
        if ctx.is_cancelled() { return Err(ToolError("Cancelled".into())); }
        let s = store()?;
        let bytes = s.read(&a.path)?;
        let text = String::from_utf8(bytes).map_err(|_| ToolError("Knowledge content is not valid UTF-8".into()))?;
        let full = if text.chars().count() > a.max_chars { text.chars().take(a.max_chars).collect::<String>() } else { text.clone() };
        let raw = json!({"path": a.path, "excerpt": ui_excerpt(&full, 260), "chars": full.chars().count()});
        Ok(ToolOutput::with_summary(full, serde_json::to_string_pretty(&raw).unwrap_or_else(|_| "{}".into())))
    }
}

// -----------------------------------------------------------------------------
// build_knowledge_pack (optional utility)
// -----------------------------------------------------------------------------

#[derive(Deserialize)]
struct BuildPackArgs {
    #[serde(default)]
    input_dir: Option<String>,
    #[serde(default)]
    output_path: Option<String>,
}

pub struct BuildKnowledgePackTool;
impl Tool for BuildKnowledgePackTool {
    fn name(&self) -> &str { "build_knowledge_pack" }
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: "Build assets/knowledge.pack from assets/knowledge directory (requires CUNNING_KNOWLEDGE_KEY).".to_string(),
            parameters: json!({"type":"object","properties":{"input_dir":{"type":"string"},"output_path":{"type":"string"}}}),
        }
    }
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError> {
        let a: BuildPackArgs = serde_json::from_value(args).map_err(|e| ToolError(format!("Invalid args: {e}")))?;
        let input = a.input_dir.map(PathBuf::from).unwrap_or_else(knowledge_dir);
        let output = a.output_path.map(PathBuf::from).unwrap_or_else(pack_path);
        let store = KnowledgeStore::load_dir(&input)?;
        let index_json = serde_json::to_vec(&store.index).map_err(|e| ToolError(format!("Serialize index failed: {e}")))?;
        let key = key_from_env()?;
        let cipher = XChaCha20Poly1305::new((&key).into());
        let mut nonce = [0u8; 24];
        getrandom::fill(&mut nonce).map_err(|e| ToolError(format!("Nonce RNG failed: {e}")))?;
        let ct = cipher.encrypt((&nonce).into(), index_json.as_ref()).map_err(|_| ToolError("Encrypt index failed".into()))?;
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&(ct.len() as u32).to_le_bytes());
        out.extend_from_slice(&ct);
        out.extend_from_slice(&store.blob);
        let out_path = output;
        if let Some(parent) = out_path.parent() { let _ = fs::create_dir_all(parent); }
        fs::write(&out_path, out).map_err(|e| ToolError(format!("Write pack failed: {e}")))?;
        Ok(ToolOutput::new(
            format!("Wrote knowledge pack: {}", out_path.display()),
            vec![ToolLog { message: "Built knowledge pack".into(), level: ToolLogLevel::Success }],
        ))
    }
}

