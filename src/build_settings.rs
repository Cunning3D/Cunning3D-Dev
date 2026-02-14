//! Build/hot-update settings (single source of truth for Cargo target dirs).

use bevy::prelude::*;
use std::path::{Path, PathBuf};

use crate::settings::{SettingMeta, SettingScope, SettingValue, SettingsMerge, SettingsRegistry, SettingsStores};

#[derive(Resource, Clone, PartialEq)]
pub struct BuildSettings {
    pub cargo_target_root: PathBuf,
}

impl Default for BuildSettings {
    fn default() -> Self {
        let root = if cfg!(target_os = "windows") {
            PathBuf::from(r"F:\cargo-target2")
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target")
        };
        Self { cargo_target_root: root }
    }
}

impl BuildSettings {
    #[inline]
    pub fn cargo_target_dir_main(&self) -> PathBuf { self.cargo_target_root.clone() }
    #[inline]
    pub fn cargo_target_dir_plugins(&self) -> PathBuf { self.cargo_target_root.join("plugins") }
    #[inline]
    pub fn cargo_target_dir_runtime_modules(&self) -> PathBuf { self.cargo_target_root.join("runtime_modules") }
    #[inline]
    pub fn cargo_target_dir_hot_restart(&self) -> PathBuf { self.cargo_target_root.join("hot_restart") }
}

pub fn apply_from_settings(reg: &SettingsRegistry, stores: &SettingsStores, s: &mut BuildSettings) {
    let d = BuildSettings::default();
    let get = |id: &str| {
        reg.get(id).map(|m| {
            let (src, v) = SettingsMerge::resolve(m, stores.project.get(id), stores.user.get(id));
            (src, v)
        })
    };
    let mut next = s.clone();
    if let Some((_src, SettingValue::String(v))) = get("build.cargo_target_root") {
        let p = PathBuf::from(v.trim());
        next.cargo_target_root = if v.trim().is_empty() { d.cargo_target_root } else { p };
    }
    if *s != next { *s = next; }
    // Best-effort: keep terminal `cargo run/build` stable too.
    let _ = ensure_cargo_target_layout(&stores.project_root, s);
}

pub fn sync_from_settings_stores(
    reg: Res<SettingsRegistry>,
    stores: Res<SettingsStores>,
    mut s: ResMut<BuildSettings>,
) {
    if !(reg.is_changed() || stores.is_changed() || s.is_added()) {
        return;
    }
    let mut next = (*s).clone();
    apply_from_settings(&*reg, &*stores, &mut next);
    if next != *s {
        *s = next;
    }
}

fn write_cargo_config_target_dir(project_root: &Path, target_dir: &Path) -> std::io::Result<()> {
    let dir = project_root.join(".cargo");
    std::fs::create_dir_all(&dir)?;
    // Use forward slashes to avoid TOML escaping issues on Windows.
    let td = target_dir.to_string_lossy().replace('\\', "/");
    let txt = format!(
        r#"[build]
target-dir = "{td}"
"#
    );
    std::fs::write(dir.join("config.toml"), txt)
}

fn ensure_cargo_target_layout(project_root: &Path, s: &BuildSettings) -> std::io::Result<()> {
    let dir = project_root.join(".cargo").join("config.toml");
    let want = s.cargo_target_dir_main().to_string_lossy().replace('\\', "/");
    let want_txt = format!("[build]\ntarget-dir = \"{want}\"\n");
    let cur = std::fs::read_to_string(&dir).unwrap_or_default();
    if cur != want_txt {
        let _ = write_cargo_config_target_dir(project_root, &s.cargo_target_dir_main());
    }
    let _ = std::fs::create_dir_all(s.cargo_target_dir_main());
    let _ = std::fs::create_dir_all(s.cargo_target_dir_plugins());
    let _ = std::fs::create_dir_all(s.cargo_target_dir_runtime_modules());
    let _ = std::fs::create_dir_all(s.cargo_target_dir_hot_restart());
    Ok(())
}

fn register_build_settings(reg: &mut SettingsRegistry) {
    let d = BuildSettings::default();
    reg.upsert(SettingMeta {
        id: "build.cargo_target_root".into(),
        path: "General/Build".into(),
        label: "Cargo Target Root".into(),
        help: "All cargo builds for hot restart/runtime/modules/plugins use this root (subfolders are auto-derived).".into(),
        scope: SettingScope::Both,
        default: SettingValue::String(d.cargo_target_root.display().to_string()),
        min: None,
        max: None,
        step: None,
        keywords: vec![
            "cargo".into(),
            "target".into(),
            "dir".into(),
            "disk".into(),
            "hot".into(),
            "restart".into(),
            "reload".into(),
        ],
    });
}

crate::register_settings_provider!("build", register_build_settings);

