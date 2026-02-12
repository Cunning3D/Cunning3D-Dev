//! Runtime install/asset paths (single-source of truth).

use std::path::PathBuf;

/// Returns the per-user config root (best-effort, writable).
pub fn user_config_dir() -> PathBuf {
    if let Ok(p) = std::env::var("APPDATA") {
        return PathBuf::from(p).join("Cunning3D");
    }
    if let Ok(p) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(p).join("cunning3d");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("cunning3d");
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Returns the per-user AI settings folder.
pub fn ai_settings_dir() -> PathBuf {
    user_config_dir().join("ai")
}

/// Returns the providers settings JSON path.
pub fn ai_providers_path() -> PathBuf {
    ai_settings_dir().join("providers.json")
}

/// Returns the sessions JSON path.
pub fn ai_sessions_path() -> PathBuf {
    ai_settings_dir().join("sessions.json")
}

/// Returns the AI Workspace state JSON path (UI/IDE restore).
pub fn ai_workspace_state_path() -> PathBuf {
    ai_settings_dir().join("workspace_state.json")
}

/// Returns the tool permissions JSON path.
pub fn ai_tool_permissions_path() -> PathBuf {
    ai_settings_dir().join("tool_permissions.json")
}

/// Returns the voice assistant settings JSON path.
pub fn ai_voice_assistant_path() -> PathBuf {
    ai_settings_dir().join("voice_assistant.json")
}

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

