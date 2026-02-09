use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use chacha20poly1305::aead::KeyInit;

const KNOWLEDGE_DIR: &str = "assets/knowledge";
const KNOWLEDGE_PACK: &str = "assets/knowledge.pack";
const KNOWLEDGE_MAGIC: &[u8; 8] = b"C3DKNOW\0";
const KNOWLEDGE_VERSION: u32 = 1;

fn is_enabled(feature: &str) -> bool {
    env::var_os(format!("CARGO_FEATURE_{}", feature)).is_some()
}

fn env_is_1(name: &str) -> bool {
    env::var(name).ok().as_deref() == Some("1")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn derive_key() -> Option<[u8; 32]> {
    use sha2::Digest;
    let s = env::var("CUNNING_KNOWLEDGE_KEY").ok()?;
    let mut h = sha2::Sha256::new();
    h.update(s.as_bytes());
    Some(h.finalize().into())
}

fn collect_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(root) else { return; };
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.is_dir() {
            collect_files(&p, out);
        } else if p.is_file() {
            out.push(p);
        }
    }
}

fn newest_mtime(paths: &[PathBuf]) -> Option<std::time::SystemTime> {
    paths
        .iter()
        .filter_map(|p| fs::metadata(p).ok().and_then(|m| m.modified().ok()))
        .max()
}

fn write_knowledge_pack(manifest_dir: &Path) {
    if !env_is_1("CUNNING_BUILD_KNOWLEDGE_PACK") {
        return;
    }
    let Some(key) = derive_key() else {
        println!("cargo:warning=knowledge.pack not built (missing CUNNING_KNOWLEDGE_KEY)");
        return;
    };

    let input_root = manifest_dir.join(KNOWLEDGE_DIR);
    let out_path = manifest_dir.join(KNOWLEDGE_PACK);
    println!("cargo:rerun-if-changed={}", input_root.display());
    if !input_root.exists() {
        println!("cargo:warning=knowledge.pack not built (missing {})", input_root.display());
        return;
    }

    let mut files = Vec::new();
    collect_files(&input_root, &mut files);
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
    }

    let src_mtime = newest_mtime(&files);
    let out_mtime = fs::metadata(&out_path).ok().and_then(|m| m.modified().ok());
    if let (Some(src), Some(out)) = (src_mtime, out_mtime) {
        if out >= src {
            return;
        }
    }

    let mut rows: Vec<(String, Vec<u8>)> = files
        .into_iter()
        .filter_map(|p| {
            let rel = p.strip_prefix(&input_root).ok()?.to_string_lossy().replace('\\', "/");
            let data = fs::read(&p).ok()?;
            Some((rel, data))
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let mut blob = Vec::new();
    let mut index = Vec::new();
    for (path, data) in rows {
        let offset = blob.len() as u32;
        let len = data.len() as u32;
        let hash = sha256_hex(&data);
        blob.extend_from_slice(&data);
        index.push(serde_json::json!({"path": path, "offset": offset, "len": len, "sha256_hex": hash}));
    }

    let idx_json = serde_json::to_vec(&index).expect("serialize knowledge index");
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut nonce).expect("nonce rng");
    let cipher = chacha20poly1305::XChaCha20Poly1305::new((&key).into());
    let ct = chacha20poly1305::aead::Aead::encrypt(&cipher, (&nonce).into(), idx_json.as_ref())
        .expect("encrypt knowledge index");

    let mut out = Vec::new();
    out.extend_from_slice(KNOWLEDGE_MAGIC);
    out.extend_from_slice(&KNOWLEDGE_VERSION.to_le_bytes());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&(ct.len() as u32).to_le_bytes());
    out.extend_from_slice(&ct);
    out.extend_from_slice(&blob);
    if let Some(parent) = out_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&out_path, out).expect("write knowledge.pack");
    println!("cargo:warning=built {}", out_path.display());
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(target_os = "windows")]
    embed_windows_manifest();

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into()));
    write_knowledge_pack(&manifest_dir);

    if !is_enabled("CODEX_BUNDLED") {
        return;
    }

    let codex_root = manifest_dir
        .join("src")
        .join("libs")
        .join("3rd")
        .join("codex")
        .join("codex-rs");
    let codex_manifest = codex_root.join("Cargo.toml");
    let target_dir = manifest_dir.join("target").join("codex_rs");

    let exe_name = if cfg!(windows) { "codex.exe" } else { "codex" };
    let built_exe = target_dir.join("release").join(exe_name);

    let out_tools = manifest_dir.join("Ltools");
    let bundled_exe = out_tools.join(exe_name);

    if !built_exe.exists() {
        let st = Command::new("cargo")
            .current_dir(&codex_root)
            .args([
                "build",
                "--release",
                "--manifest-path",
                codex_manifest.to_string_lossy().as_ref(),
                "--target-dir",
                target_dir.to_string_lossy().as_ref(),
                "-p",
                "codex-cli",
            ])
            .status();
        match st {
            Ok(s) if s.success() => {}
            Ok(s) => panic!("building codex failed: {}", s),
            Err(e) => panic!("failed to spawn cargo to build codex: {}", e),
        }
    }

    let _ = fs::create_dir_all(&out_tools);
    fs::copy(&built_exe, &bundled_exe).expect("failed to copy bundled codex");
    println!(
        "cargo:rustc-env=CUNNING_CODEX_BIN={}",
        bundled_exe.to_string_lossy()
    );
}

#[cfg(target_os = "windows")]
fn embed_windows_manifest() {
    let manifest = Path::new("resources/windows/cunning3d.manifest.xml");
    let rc_file = Path::new("resources/windows/cunning3d.rc");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed={}", rc_file.display());
    embed_resource::compile(rc_file, embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
