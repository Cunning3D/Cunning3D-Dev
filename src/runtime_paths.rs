//! Runtime install/asset paths (single-source of truth).

use std::path::PathBuf;

/// Returns the folder containing the running executable (best-effort).
pub fn install_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Returns the absolute `assets/` directory under the install folder.
pub fn assets_dir() -> PathBuf {
    if let Ok(p) = std::env::var("CUNNING3D_ASSETS_DIR") {
        let p = PathBuf::from(p);
        if p.exists() { return p; }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let p = cwd.join("assets");
        if p.exists() { return p; }
    }
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
    if p.exists() { return p; }
    install_dir().join("assets")
}

