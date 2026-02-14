//! Hot-Reload window pane: UE-like popup with live log, progress, and plugin rebuild status.

use crate::cunning_core::plugin_system::{request_compile_rust_plugin, CompileRustPluginRequest};
use crate::console::LogLevel;
use crate::runtime_module;
use crate::runtime_module::build_jobs as rt_build;
use crate::rust_code_watch::{decide_update, UpdateDecision};
use crate::tabs_system::{EditorTab, EditorTabContext};
use bevy::asset::AssetEvent;
use bevy::prelude::*;
use bevy_egui::egui;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// HotReloadLog: dedicated log for hot-reload operations
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct HotReloadEntry {
    pub level: LogLevel,
    pub message: String,
    pub elapsed_s: f32,
}

#[derive(Resource, Clone)]
pub struct HotReloadLog {
    entries: Arc<Mutex<Vec<HotReloadEntry>>>,
    rev: Arc<AtomicU64>,
}

impl Default for HotReloadLog {
    fn default() -> Self { Self { entries: Arc::new(Mutex::new(Vec::new())), rev: Arc::new(AtomicU64::new(1)) } }
}

impl HotReloadLog {
    pub fn revision(&self) -> u64 { self.rev.load(Ordering::Relaxed) }
    pub fn push(&self, level: LogLevel, msg: impl Into<String>, elapsed_s: f32) {
        if let Ok(mut e) = self.entries.lock() {
            e.push(HotReloadEntry { level, message: msg.into(), elapsed_s });
            let len = e.len(); if len > 2000 { e.drain(0..len - 2000); }
        }
        self.rev.fetch_add(1, Ordering::Relaxed);
    }
    pub fn info(&self, m: impl Into<String>, t: f32) { self.push(LogLevel::Info, m, t); }
    pub fn warn(&self, m: impl Into<String>, t: f32) { self.push(LogLevel::Warning, m, t); }
    pub fn error(&self, m: impl Into<String>, t: f32) { self.push(LogLevel::Error, m, t); }
    pub fn entries(&self) -> Vec<HotReloadEntry> { self.entries.lock().ok().map(|e| e.clone()).unwrap_or_default() }
    pub fn clear(&self) { if let Ok(mut e) = self.entries.lock() { e.clear(); } self.rev.fetch_add(1, Ordering::Relaxed); }
}

// ---------------------------------------------------------------------------
// Snapshot of active compile jobs (synced from AppJobs by a Bevy system)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct CompileJobSnapshot {
    pub plugin: String,
    pub title: String,
    pub fraction: f32,
    pub message: String,
    pub state: &'static str,
    pub log_tail: Vec<String>,
}

#[derive(Resource, Clone, Default)]
pub struct HotReloadJobsSnapshot {
    inner: Arc<Mutex<Vec<CompileJobSnapshot>>>,
}

impl HotReloadJobsSnapshot {
    pub fn update(&self, jobs: Vec<CompileJobSnapshot>) { if let Ok(mut v) = self.inner.lock() { *v = jobs; } }
    pub fn get(&self) -> Vec<CompileJobSnapshot> { self.inner.lock().ok().map(|v| v.clone()).unwrap_or_default() }
}

/// Bevy system: sync compile-plugin job progress into the shared snapshot.
pub fn sync_hot_reload_jobs_snapshot_system(
    jobs: Option<Res<crate::app_jobs::AppJobs>>,
    snap: Res<HotReloadJobsSnapshot>,
) {
    let Some(jobs) = jobs.as_deref() else { return; };
    let active: Vec<CompileJobSnapshot> = jobs.jobs().values()
        .filter(|j| j.kind == "compile_rust_plugin")
        .map(|j| {
            let plugin = j
                .title
                .splitn(2, ':')
                .nth(1)
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| j.title.clone());
            let tail: Vec<String> = j
                .log
                .iter()
                .rev()
                .take(20)
                .rev()
                .map(|l| {
                    let p = match l.level {
                        crate::app_jobs::JobLogLevel::Info => "I",
                        crate::app_jobs::JobLogLevel::Warning => "W",
                        crate::app_jobs::JobLogLevel::Error => "E",
                    };
                    format!("[{p}] {}", l.message)
                })
                .collect();
            CompileJobSnapshot {
                plugin,
                title: j.title.clone(),
                fraction: j.progress.fraction,
                message: j.progress.message.clone(),
                state: match j.state {
                    crate::app_jobs::JobState::Running => "running",
                    crate::app_jobs::JobState::Completed => "done",
                    crate::app_jobs::JobState::Failed => "failed",
                    crate::app_jobs::JobState::Cancelled => "cancelled",
                    _ => "queued",
                },
                log_tail: tail,
            }
        })
        .collect();
    snap.update(active);
}

/// Bevy system: forward asset hot-reload events into `HotReloadLog`.
pub fn log_asset_hot_reload_system(
    time: Res<Time>,
    asset_server: Res<AssetServer>,
    mut shader_events: MessageReader<AssetEvent<Shader>>,
    mut image_events: MessageReader<AssetEvent<Image>>,
    hot_log: Option<Res<HotReloadLog>>,
) {
    let Some(hl) = hot_log.as_deref() else { return; };
    let t = time.elapsed_secs();
    let mut log_one = |kind: &str, id_untyped: bevy::asset::UntypedAssetId, action: &str| {
        let p = asset_server.get_path(id_untyped).map(|p| format!("{p}")).unwrap_or_else(|| "<unknown>".into());
        hl.info(format!("{action} {kind}: {p}"), t);
    };
    for e in shader_events.read() {
        match *e {
            AssetEvent::Modified { id } => log_one("shader", id.untyped(), "Updated"),
            AssetEvent::Removed { id } => log_one("shader", id.untyped(), "Removed"),
            _ => {}
        }
    }
    for e in image_events.read() {
        match *e {
            AssetEvent::Modified { id } => log_one("image", id.untyped(), "Updated"),
            AssetEvent::Removed { id } => log_one("image", id.untyped(), "Removed"),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// HotReloadTab: EditorTab rendered inside the floating hot-reload window
// ---------------------------------------------------------------------------

pub struct HotReloadTab {
    log: HotReloadLog,
    jobs_snap: HotReloadJobsSnapshot,
    last_rev: u64,
    plugin_name: String,
}

impl HotReloadTab {
    pub fn new(log: HotReloadLog, jobs_snap: HotReloadJobsSnapshot) -> Self {
        Self { log, jobs_snap, last_rev: 0, plugin_name: String::new() }
    }
}

impl EditorTab for HotReloadTab {
    fn title(&self) -> egui::WidgetText { "Hot Reload".into() }
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn is_immediate(&self) -> bool { true }
    fn retained_key(&self, _ui: &egui::Ui, _cx: &EditorTabContext) -> u64 { 0 }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut EditorTabContext) {
        let bg = egui::Color32::from_rgb(22, 22, 26);
        egui::Frame::new().fill(bg).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            // Header
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new("Hot Reload").color(egui::Color32::from_rgb(80, 200, 120)).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Clear").clicked() { self.log.clear(); }
                });
            });
            ui.separator();

            // Rust source changes (UE-like prompt)
            ui.collapsing("Rust Code Changes", |ui| {
                let files = cx.rust_code_changes.list();
                if files.is_empty() {
                    ui.label(egui::RichText::new("No Rust file changes detected.").color(C_DIM));
                    return;
                }
                let decision = decide_update(&files);
                ui.label(egui::RichText::new(format!("Detected {} Rust file change(s).", files.len())).color(C_LABEL));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(110.0).auto_shrink([false, false]).show(ui, |ui| {
                    for p in &files {
                        ui.label(egui::RichText::new(p.display().to_string()).color(C_DIM).monospace());
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Hot Update").clicked() {
                        match decision {
                            UpdateDecision::HotRestart => {
                                self.log.warn("Hot Update requested...", 0.0);
                                let _ = crate::hot_restart::request_hot_restart(true);
                            }
                            UpdateDecision::LiveReloadRuntimeModule => {
                                self.log.info("Hot Update requested...", 0.0);
                                let _ = rt_build::request_compile_runtime_module(rt_build::CompileRuntimeModuleRequest::editor_runtime());
                            }
                            UpdateDecision::None => {}
                        }
                        cx.rust_code_changes.clear();
                    }
                    if ui.button("Ignore").clicked() {
                        cx.rust_code_changes.clear();
                    }
                });
            });
            ui.separator();

            // Runtime module controls (in-process)
            ui.collapsing("Runtime Module (in-process)", |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Build & Hot-Load editor_runtime").clicked() {
                        match rt_build::request_compile_runtime_module(rt_build::CompileRuntimeModuleRequest::editor_runtime()) {
                            Ok(()) => self.log.info("Build requested: runtime/editor_runtime (debug)", 0.0),
                            Err(e) => self.log.error(format!("Build request failed: {e}"), 0.0),
                        }
                    }
                    if let Some(p) = cx.runtime_module_state.last_loaded_path.as_ref() {
                        ui.label(egui::RichText::new(format!("Loaded: {}", p.display())).color(C_DIM));
                    } else {
                        ui.label(egui::RichText::new("Loaded: <none>").color(C_DIM));
                    }
                });

                let cmds = runtime_module::list_commands(cx.runtime_module_state);
                if cmds.is_empty() {
                    ui.label(egui::RichText::new("Commands: <none>").color(C_DIM));
                } else {
                    ui.label(egui::RichText::new("Commands:").color(C_LABEL));
                    ui.horizontal_wrapped(|ui| {
                        for (i, name) in cmds.iter().enumerate() {
                            if ui.button(name).clicked() {
                                let _ = runtime_module::run_command(cx.runtime_module_state, i as u32, cx.runtime_module_log);
                            }
                        }
                    });
                }

                ui.add_space(4.0);
                ui.label(egui::RichText::new("Runtime log:").color(C_LABEL));
                let tail = cx.runtime_module_log.0.lock().ok().map(|v| v.clone()).unwrap_or_default();
                egui::ScrollArea::vertical().max_height(120.0).auto_shrink([false, false]).show(ui, |ui| {
                    for l in tail.iter().rev().take(60).rev() {
                        ui.label(egui::RichText::new(l).color(C_DIM).monospace());
                    }
                });
            });
            ui.separator();

            // Live Coding controls
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Plugin").color(C_LABEL));
                ui.text_edit_singleline(&mut self.plugin_name);
                if ui.button("Build & Hot-Load").clicked() {
                    let p = self.plugin_name.trim();
                    if p.is_empty() {
                        self.log.warn("Missing plugin name. Example: curve_plugin", 0.0);
                    } else {
                        match request_compile_rust_plugin(CompileRustPluginRequest::for_extra_node(p)) {
                            Ok(()) => self.log.info(format!("Build requested: plugins/extra_node/{p} (release)"), 0.0),
                            Err(e) => self.log.error(format!("Build request failed: {e}"), 0.0),
                        }
                    }
                }
                if ui.button("Open plugins/").clicked() {
                    let ok = open_path_in_os_file_manager("plugins");
                    if ok { self.log.info("Opened plugins/ directory.", 0.0); } else { self.log.warn("Failed to open plugins/ directory.", 0.0); }
                }
                if ui.button("Hot Restart (Seamless)").clicked() {
                    self.log.warn("Hot Restart requested: building new app + seamless handoff...", 0.0);
                    let _ = crate::hot_restart::request_hot_restart(true);
                }
            });
            ui.label(egui::RichText::new("Note: editing .rs won't hot-reload until a new plugin DLL is built/copied into plugins/.").color(C_DIM));
            ui.separator();

            // Aggregated compile jobs (by plugin)
            let mut by_plugin: BTreeMap<String, Vec<CompileJobSnapshot>> = BTreeMap::new();
            for j in self.jobs_snap.get() { by_plugin.entry(j.plugin.clone()).or_default().push(j); }
            if !by_plugin.is_empty() {
                for (plugin, jobs) in by_plugin {
                    let (state, frac, msg) = jobs
                        .iter()
                        .find(|j| j.state == "running")
                        .map(|j| (j.state, j.fraction, j.message.clone()))
                        .or_else(|| jobs.first().map(|j| (j.state, j.fraction, j.message.clone())))
                        .unwrap_or(("queued", 0.0, String::new()));
                    egui::CollapsingHeader::new(format!("{plugin}  [{state}]"))
                        .default_open(state == "running")
                        .show(ui, |ui| {
                            if state == "running" {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(egui::RichText::new(format!("{}%  {msg}", (frac * 100.0) as u32)).color(egui::Color32::from_rgb(100, 180, 255)));
                                });
                                ui.add(egui::ProgressBar::new(frac).animate(true));
                            }
                            for j in &jobs {
                                if !j.log_tail.is_empty() {
                                    ui.separator();
                                    ui.label(egui::RichText::new(&j.title).color(C_LABEL));
                                    for l in &j.log_tail {
                                        ui.label(egui::RichText::new(l).color(C_DIM).monospace());
                                    }
                                }
                            }
                        });
                }
                ui.separator();
            }

            // Status panel
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Asset Watcher").color(C_LABEL));
                ui.label(egui::RichText::new("ACTIVE").color(C_OK).strong());
                ui.label(egui::RichText::new("(file_watcher: shaders, textures auto-reload on save)").color(C_DIM));
            });
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Plugin Auto-Scan").color(C_LABEL));
                ui.label(egui::RichText::new("ACTIVE").color(C_OK).strong());
                ui.label(egui::RichText::new("(every 0.75s, plugins/ directory)").color(C_DIM));
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Shortcut").color(C_DIM));
                ui.label(egui::RichText::new("Ctrl+Alt+R").color(egui::Color32::from_rgb(200, 200, 100)).strong());
                ui.label(egui::RichText::new("— force rescan all plugins immediately").color(C_DIM));
            });
            ui.separator();

            // Scrollable log area
            let entries = self.log.entries();
            let new_rev = self.log.revision();
            let changed = new_rev != self.last_rev;
            self.last_rev = new_rev;

            egui::ScrollArea::vertical()
                .max_height(ui.available_height().max(120.0))
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if entries.is_empty() {
                        ui.label(egui::RichText::new("No hot-reload events yet. Press Ctrl+Alt+R or save a shader file.").color(C_DIM).italics());
                    }
                    for e in &entries {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("[{:.2}s]", e.elapsed_s)).color(C_DIM).monospace());
                            let (icon, color) = match e.level {
                                LogLevel::Info => ("INFO", C_OK),
                                LogLevel::Warning => ("WARN", egui::Color32::from_rgb(255, 200, 0)),
                                LogLevel::Error => ("ERR ", egui::Color32::from_rgb(255, 100, 100)),
                                LogLevel::Debug => ("DBG ", egui::Color32::from_rgb(150, 150, 255)),
                            };
                            ui.label(egui::RichText::new(icon).color(color).monospace());
                            ui.label(egui::RichText::new(&e.message).color(egui::Color32::from_rgb(210, 210, 210)).monospace());
                        });
                    }
                    if changed { ui.scroll_to_cursor(Some(egui::Align::BOTTOM)); }
                });
        });
    }
}

const C_OK: egui::Color32 = egui::Color32::from_rgb(80, 200, 120);
const C_LABEL: egui::Color32 = egui::Color32::from_rgb(180, 180, 180);
const C_DIM: egui::Color32 = egui::Color32::from_rgb(120, 120, 120);

fn open_path_in_os_file_manager(path: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer").arg(path).spawn().is_ok()
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn().is_ok()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn().is_ok()
    }
}
