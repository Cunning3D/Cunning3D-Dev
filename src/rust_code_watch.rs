//! Rust source change watcher for UE-like Live Coding prompt.

use bevy::prelude::*;
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Resource, Default)]
pub struct HotUpdatePrompt {
    pub open: bool,
    pub seen_count: usize,
    /// When set, the prompt will fade out and then close (seconds from egui `ctx.input().time`).
    pub fade_out_started_at: Option<f64>,
}

#[derive(Resource, Default, Clone)]
pub struct RustCodeChanges {
    inner: Arc<Mutex<RustCodeChangesInner>>,
}

#[derive(Default)]
struct RustCodeChangesInner {
    pending: Vec<PathBuf>,
    seen: std::collections::HashSet<PathBuf>,
}

impl RustCodeChanges {
    pub fn push(&self, p: PathBuf) {
        if let Ok(mut s) = self.inner.lock() {
            if s.seen.insert(p.clone()) {
                s.pending.push(p);
            }
        }
    }
    pub fn list(&self) -> Vec<PathBuf> {
        self.inner.lock().ok().map(|s| s.pending.clone()).unwrap_or_default()
    }
    pub fn len(&self) -> usize {
        self.inner.lock().ok().map(|s| s.pending.len()).unwrap_or(0)
    }
    pub fn clear(&self) {
        if let Ok(mut s) = self.inner.lock() {
            s.pending.clear();
            s.seen.clear();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateDecision {
    LiveReloadRuntimeModule,
    HotRestart,
    None,
}

pub fn decide_update(paths: &[PathBuf]) -> UpdateDecision {
    if paths.is_empty() {
        return UpdateDecision::None;
    }
    let mut any_host = false;
    let mut any_runtime = false;
    for p in paths {
        let s = p.to_string_lossy().replace('\\', "/");
        if s.contains("/runtime/") {
            any_runtime = true;
        } else {
            any_host = true;
        }
    }
    if any_host {
        UpdateDecision::HotRestart // prefer B when any host code changed
    } else if any_runtime {
        UpdateDecision::LiveReloadRuntimeModule
    } else {
        UpdateDecision::HotRestart
    }
}

#[derive(Resource)]
struct WatchThread {
    _watcher: Arc<Mutex<Option<RecommendedWatcher>>>,
}

pub struct RustCodeWatchPlugin;

impl Plugin for RustCodeWatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RustCodeChanges>()
            .init_resource::<HotUpdatePrompt>()
            .add_systems(Startup, start_rust_watch_thread_system);
    }
}

fn start_rust_watch_thread_system(mut commands: Commands, changes: Res<RustCodeChanges>) {
    let changes = changes.clone();
    let watcher = RecommendedWatcher::new(
        move |res: Result<notify::Event, notify::Error>| {
            let Ok(ev) = res else { return; };
            match ev.kind {
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {}
                _ => return,
            }
            for p in ev.paths {
                if is_rust_source(&p) {
                    changes.push(p.to_path_buf());
                }
            }
        },
        Config::default(),
    )
    .ok();
    let holder = Arc::new(Mutex::new(watcher));
    if let Ok(mut w) = holder.lock() {
        if let Some(w) = w.as_mut() {
            for dir in watch_dirs() {
                let _ = w.watch(&dir, RecursiveMode::Recursive);
            }
        }
    }
    commands.insert_resource(WatchThread { _watcher: holder });
}

fn watch_dirs() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    vec![
        root.join("src"),
        root.join("crates"),
        root.join("runtime"),
    ]
}

#[inline]
fn is_rust_source(p: &Path) -> bool {
    p.extension()
        .and_then(|x| x.to_str())
        .map(|x| x.eq_ignore_ascii_case("rs"))
        .unwrap_or(false)
}

