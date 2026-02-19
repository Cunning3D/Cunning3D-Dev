//! Seamless hot restart (snapshot + restore + handoff).

use crate::camera::CameraController;
use crate::console::ConsoleLog;
use crate::launcher::plugin::AppState;
use crate::tabs_system::{EditorTab, TabViewer};
use crate::ui::{FloatingTabRegistry, LayoutMode, MobileTab, NodeEditorState, UiState};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::egui;
use egui_dock::DockState;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, OnceLock};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Resource, Default, Clone)]
pub struct HotRestartSettings {
    pub restore_path: Option<PathBuf>,
    pub handoff_port: Option<u16>,
    pub handoff_token: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum MainTabId {
    Viewport3D,
    NodeProperties,
    GeometrySpreadsheet,
    Console,
    Codex,
    NodeEditor,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ShelfTabId {
    Create,
    Modify,
    Model,
    Polygon,
    Deform,
    Texture,
    Rigging,
    Empty,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LayoutModeId {
    Desktop,
    Tablet,
    Phone,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MobileTabId {
    Viewport,
    NodeGraph,
    Properties,
    Console,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CameraSnapshot {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub ctrl_enabled: bool,
    pub ctrl_sensitivity: f32,
    pub ctrl_speed: f32,
    pub ctrl_pivot: [f32; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WindowSnapshot {
    pub title: String,
    pub pos: Option<[i32; 2]>,
    pub size: Option<[u32; 2]>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HotRestartSnapshot {
    pub project: crate::project::ProjectFile,
    pub layout_mode: LayoutModeId,
    pub mobile_tab: MobileTabId,
    pub main_dock: DockState<MainTabId>,
    pub shelf_dock: DockState<ShelfTabId>,
    pub selected_nodes: Vec<crate::nodes::NodeId>,
    pub last_selected_node_id: Option<crate::nodes::NodeId>,
    pub node_editor_pan: [f32; 2],
    pub node_editor_zoom: f32,
    pub cda_path: Vec<crate::nodes::NodeId>,
    pub camera: Option<CameraSnapshot>,
    pub floating_windows: Vec<WindowSnapshot>,
}

pub struct HotRestartPlugin;

impl Plugin for HotRestartPlugin {
    fn build(&self, app: &mut App) {
        let (tx, rx) = crossbeam_channel::unbounded::<RequestHotRestart>();
        let _ = GLOBAL_HOT_RESTART_TX.set(tx);
        app.init_resource::<HotRestartSettings>()
            .init_resource::<HotRestartRuntime>()
            .insert_resource(HotRestartQueue { rx })
            .add_message::<RequestHotRestart>()
            .add_systems(
                OnEnter(AppState::Editor),
                (apply_restore_if_requested_system, send_handoff_ready_if_requested_system),
            )
            .add_systems(Update, handle_hot_restart_requests_system.run_if(in_state(AppState::Editor)))
            .add_systems(Update, poll_hot_restart_build_spawn_system.run_if(in_state(AppState::Editor)))
            .add_systems(Update, poll_handoff_system.run_if(in_state(AppState::Editor)));
    }
}

static GLOBAL_HOT_RESTART_TX: OnceLock<crossbeam_channel::Sender<RequestHotRestart>> = OnceLock::new();

/// Request a seamless hot restart from non-ECS contexts (egui tabs).
pub fn request_hot_restart(build: bool) -> Result<(), String> {
    let Some(tx) = GLOBAL_HOT_RESTART_TX.get() else { return Err("HotRestart queue not initialized".into()); };
    tx.send(RequestHotRestart { build }).map_err(|_| "HotRestart send failed".into())
}

#[derive(Resource, Default)]
struct HotRestartRuntime {
    waiting: Option<HandoffWait>,
    build_spawn: Option<crossbeam_channel::Receiver<Result<(), String>>>,
    build_log: Option<crossbeam_channel::Receiver<BuildLogLine>>,
    build_log_accum: String,
}

#[derive(Clone, Debug)]
struct BuildLogLine {
    is_stderr: bool,
    line: String,
}

#[derive(Resource)]
struct HotRestartQueue {
    rx: crossbeam_channel::Receiver<RequestHotRestart>,
}

struct HandoffWait {
    token: String,
    rx: crossbeam_channel::Receiver<String>,
}

#[derive(Message, Clone)]
pub struct RequestHotRestart {
    pub build: bool,
}

fn layout_mode_to_id(m: LayoutMode) -> LayoutModeId {
    match m {
        LayoutMode::Desktop => LayoutModeId::Desktop,
        LayoutMode::Tablet => LayoutModeId::Tablet,
        LayoutMode::Phone => LayoutModeId::Phone,
    }
}

fn mobile_tab_to_id(t: MobileTab) -> MobileTabId {
    match t {
        MobileTab::Viewport => MobileTabId::Viewport,
        MobileTab::NodeGraph => MobileTabId::NodeGraph,
        MobileTab::Properties => MobileTabId::Properties,
        MobileTab::Console => MobileTabId::Console,
    }
}

fn main_tab_id_from_title(title: &str) -> Option<MainTabId> {
    match title {
        "3D Viewport" => Some(MainTabId::Viewport3D),
        "Properties" => Some(MainTabId::NodeProperties),
        "Geometry Spreadsheet" => Some(MainTabId::GeometrySpreadsheet),
        "Console" => Some(MainTabId::Console),
        "Codex" => Some(MainTabId::Codex),
        "Node Graph" => Some(MainTabId::NodeEditor),
        _ => None,
    }
}

fn shelf_tab_to_id(t: &crate::ui::ShelfTab) -> ShelfTabId {
    use crate::ui::ShelfTab as T;
    match t {
        T::Create => ShelfTabId::Create,
        T::Modify => ShelfTabId::Modify,
        T::Model => ShelfTabId::Model,
        T::Polygon => ShelfTabId::Polygon,
        T::Deform => ShelfTabId::Deform,
        T::Texture => ShelfTabId::Texture,
        T::Rigging => ShelfTabId::Rigging,
        T::Empty => ShelfTabId::Empty,
    }
}

fn shelf_tab_from_id(t: ShelfTabId) -> crate::ui::ShelfTab {
    use crate::ui::ShelfTab as T;
    match t {
        ShelfTabId::Create => T::Create,
        ShelfTabId::Modify => T::Modify,
        ShelfTabId::Model => T::Model,
        ShelfTabId::Polygon => T::Polygon,
        ShelfTabId::Deform => T::Deform,
        ShelfTabId::Texture => T::Texture,
        ShelfTabId::Rigging => T::Rigging,
        ShelfTabId::Empty => T::Empty,
    }
}

fn build_tab(id: MainTabId) -> Box<dyn EditorTab> {
    match id {
        MainTabId::Viewport3D => Box::new(crate::tabs_system::Viewport3DTab::default()),
        MainTabId::NodeProperties => Box::new(crate::tabs_system::NodePropertiesTab::default()),
        MainTabId::GeometrySpreadsheet => Box::new(crate::tabs_system::GeometrySpreadsheetTab::default()),
        MainTabId::Console => Box::new(crate::tabs_system::ConsoleTab::default()),
        MainTabId::Codex => Box::new(crate::tabs_system::CodexTab::default()),
        MainTabId::NodeEditor => Box::new(crate::tabs_system::node_editor::NodeEditorTab::default()),
    }
}

pub fn default_snapshot_path() -> PathBuf {
    std::env::temp_dir().join("cunning3d_hot_restart_snapshot.json")
}

fn default_hot_restart_target_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target_hot_restart")
}

pub fn save_snapshot_atomic(path: &Path, s: &HotRestartSnapshot) -> Result<(), String> {
    let json = serde_json::to_string_pretty(s).map_err(|e| format!("serialize snapshot failed: {e}"))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
    let tmp = parent.join(format!(
        ".{}.tmp.{}",
        path.file_name().and_then(|x| x.to_str()).unwrap_or("snapshot.json"),
        std::process::id()
    ));
    std::fs::write(&tmp, json.as_bytes()).map_err(|e| format!("write tmp failed: {e}"))?;
    let _ = std::fs::remove_file(path);
    std::fs::rename(&tmp, path).map_err(|e| format!("rename failed: {e}"))?;
    Ok(())
}

pub fn load_snapshot(path: &Path) -> Result<HotRestartSnapshot, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("read snapshot failed: {e}"))?;
    serde_json::from_str(&json).map_err(|e| format!("deserialize snapshot failed: {e}"))
}

pub fn capture_snapshot_system(
    mut out: MessageWriter<HotRestartSnapshotReady>,
    snapshot: Res<crate::nodes::graph_model::NodeGraphSnapshotRes>,
    node_editor: Res<NodeEditorState>,
    ui: Res<UiState>,
    tab_viewer: Res<TabViewer>,
    cam_q: Query<(&Transform, &CameraController), With<crate::MainCamera>>,
    float_reg: Res<FloatingTabRegistry>,
    windows: Query<&Window>,
) {
    out.write(HotRestartSnapshotReady(capture_snapshot(
        &snapshot,
        &node_editor,
        &ui,
        &tab_viewer,
        &cam_q,
        &float_reg,
        &windows,
    )));
}

fn capture_snapshot(
    snapshot: &crate::nodes::graph_model::NodeGraphSnapshotRes,
    node_editor: &NodeEditorState,
    ui: &UiState,
    tab_viewer: &TabViewer,
    cam_q: &Query<(&Transform, &CameraController), With<crate::MainCamera>>,
    float_reg: &FloatingTabRegistry,
    windows: &Query<&Window>,
) -> HotRestartSnapshot {
    let view = crate::project::UiStateData { pan: Vec2::new(node_editor.pan.x, node_editor.pan.y), zoom: node_editor.zoom };
    let graph = crate::project::GraphData {
        nodes: snapshot.0.nodes.clone(),
        connections: snapshot.0.connections.clone(),
        sticky_notes: snapshot.0.sticky_notes.clone(),
        sticky_note_draw_order: snapshot.0.sticky_note_draw_order.clone(),
        network_boxes: snapshot.0.network_boxes.clone(),
        network_box_draw_order: snapshot.0.network_box_draw_order.clone(),
        promote_notes: snapshot.0.promote_notes.clone(),
        promote_note_draw_order: snapshot.0.promote_note_draw_order.clone(),
        display_node: snapshot.0.display_node,
        ui_state: view,
    };
    let project = crate::project::ProjectFile {
        header: crate::project::ProjectHeader {
            version: "hot_restart".into(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            uuid: uuid::Uuid::new_v4(),
            author: "hot".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
        graph,
    cda_defs: Default::default(),
    };

    let main_dock = tab_viewer.dock_state.filter_map_tabs(|t| {
        let s = t.title().text().to_string();
        main_tab_id_from_title(&s)
    });
    let shelf_dock = ui.shelf_dock_state.filter_map_tabs(|t| Some(shelf_tab_to_id(t)));

    let camera = cam_q.single().ok().map(|(t, c)| CameraSnapshot {
        translation: [t.translation.x, t.translation.y, t.translation.z],
        rotation: [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w],
        ctrl_enabled: c.enabled,
        ctrl_sensitivity: c.sensitivity,
        ctrl_speed: c.speed,
        ctrl_pivot: [c.pivot.x, c.pivot.y, c.pivot.z],
    });

    let mut floating_windows = Vec::new();
    for (e, entry) in float_reg.floating_windows.iter() {
        if let Ok(w) = windows.get(*e) {
            floating_windows.push(WindowSnapshot {
                title: entry.title.clone(),
                pos: match w.position { bevy::window::WindowPosition::At(p) => Some([p.x, p.y]), _ => None },
                size: Some([w.physical_width(), w.physical_height()]),
            });
        }
    }

    let selected_nodes: Vec<_> = ui.selected_nodes.iter().cloned().collect();
    HotRestartSnapshot {
        project,
        layout_mode: layout_mode_to_id(ui.layout_mode),
        mobile_tab: mobile_tab_to_id(ui.mobile_active_tab),
        main_dock,
        shelf_dock,
        selected_nodes,
        last_selected_node_id: ui.last_selected_node_id,
        node_editor_pan: [node_editor.pan.x, node_editor.pan.y],
        node_editor_zoom: node_editor.zoom,
        cda_path: node_editor.cda_path.clone(),
        camera,
        floating_windows,
    }
}

fn start_handoff_listener() -> Result<(u16, String, crossbeam_channel::Receiver<String>), String> {
    let token = uuid::Uuid::new_v4().to_string();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("bind handoff listener failed: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("get local addr failed: {e}"))?
        .port();
    let (tx, rx) = crossbeam_channel::unbounded::<String>();
    std::thread::spawn(move || {
        let Ok((mut sock, _)) = listener.accept() else { return; };
        let mut buf = [0u8; 1024];
        let n = sock.read(&mut buf).unwrap_or(0);
        let msg = String::from_utf8_lossy(&buf[..n]).trim().to_string();
        let _ = tx.send(msg);
    });
    Ok((port, token, rx))
}

fn built_exe_path(target_dir: &Path) -> PathBuf {
    let exe = if cfg!(target_os = "windows") { "cunning3d.exe" } else { "cunning3d" };
    target_dir.join("debug").join(exe)
}

fn build_new_exe(target_dir: &Path, console: Option<&ConsoleLog>) -> Result<PathBuf, String> {
    let cargo = crate::cunning_core::plugin_system::rust_build::find_cargo()
        .unwrap_or_else(|| PathBuf::from("cargo"));
    if let Some(c) = console { c.info(format!("Hot Restart: cargo build (target_dir={})", target_dir.display())); }
    let mut cmd = Command::new(cargo);
    cmd.current_dir(std::env::current_dir().map_err(|e| format!("cwd failed: {e}"))?)
        .arg("build")
        .env("CARGO_TARGET_DIR", target_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("spawn cargo failed: {e}"))?;
    let out = child.wait_with_output().map_err(|e| format!("wait cargo failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo build failed (code={:?})\n{}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        ));
    }
    let exe = built_exe_path(target_dir);
    if !exe.exists() {
        return Err(format!("build succeeded but exe missing: {}", exe.display()));
    }
    Ok(exe)
}

fn spawn_new_process(exe: &Path, restore: &Path, port: u16, token: &str) -> Result<(), String> {
    let mut cmd = Command::new(exe);
    cmd.arg("--restore-state")
        .arg(restore)
        .arg("--handoff-port")
        .arg(port.to_string())
        .arg("--handoff-token")
        .arg(token)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn().map_err(|e| format!("spawn new process failed: {e}"))?;
    Ok(())
}

fn poll_handoff_system(
    mut rt: ResMut<HotRestartRuntime>,
    mut exit: MessageWriter<bevy::app::AppExit>,
    console: Option<Res<ConsoleLog>>,
) {
    let Some(w) = rt.as_mut().waiting.as_mut() else { return; };
    match w.rx.try_recv() {
        Ok(msg) => {
            if msg == w.token {
                if let Some(c) = console.as_deref() { c.info("Hot Restart: new process ready, exiting old process."); }
                exit.write(bevy::app::AppExit::Success);
            } else if let Some(c) = console.as_deref() {
                c.warning(format!("Hot Restart: handoff token mismatch (got='{msg}')"));
            }
        }
        Err(crossbeam_channel::TryRecvError::Empty) => {}
        Err(crossbeam_channel::TryRecvError::Disconnected) => {
            if let Some(c) = console.as_deref() { c.warning("Hot Restart: handoff channel disconnected."); }
            rt.as_mut().waiting = None;
        }
    }
}

fn poll_hot_restart_build_spawn_system(
    mut rt: ResMut<HotRestartRuntime>,
    console: Option<Res<ConsoleLog>>,
    hot_log: Option<Res<crate::tabs_system::pane::hot_reload::HotReloadLog>>,
    time: Res<Time>,
) {
    let t = time.elapsed_secs();
    let mut flush_build_lines = |rt: &mut HotRestartRuntime| {
        let lines: Vec<BuildLogLine> = rt
            .build_log
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for l in lines {
            if l.line.trim().is_empty() {
                continue;
            }
            let msg = format!("[cargo] {}", l.line);
            if l.is_stderr {
                if let Some(c) = console.as_deref() {
                    c.warning(msg.clone());
                }
                if let Some(hl) = hot_log.as_deref() {
                    hl.warn(msg, t);
                }
            } else {
                if let Some(c) = console.as_deref() {
                    c.info(msg.clone());
                }
                if let Some(hl) = hot_log.as_deref() {
                    hl.info(msg, t);
                }
            }
            rt.build_log_accum.push_str(&l.line);
            rt.build_log_accum.push('\n');
        }
    };
    flush_build_lines(rt.as_mut());
    let Some(rx) = rt.as_mut().build_spawn.as_mut() else { return; };
    match rx.try_recv() {
        Ok(Ok(())) => {
            // Drain any late lines that arrived right before completion.
            flush_build_lines(rt.as_mut());
            rt.as_mut().build_spawn = None;
            rt.as_mut().build_log = None;
            rt.as_mut().build_log_accum.clear();
            if let Some(c) = console.as_deref() { c.info("Hot Restart: spawned new process, waiting for ready signal..."); }
            if let Some(hl) = hot_log.as_deref() { hl.info("Hot Restart: waiting for ready signal...", t); }
        }
        Ok(Err(e)) => {
            // Drain any late lines that arrived right before failure.
            flush_build_lines(rt.as_mut());
            let detail_block = if !rt.as_mut().build_log_accum.trim().is_empty() {
                Some(rt.as_mut().build_log_accum.clone())
            } else {
                None
            };
            rt.as_mut().build_spawn = None;
            rt.as_mut().build_log = None;
            rt.as_mut().waiting = None;
            if let Some(c) = console.as_deref() { c.error(format!("Hot Restart failed: {e}")); }
            if let Some(hl) = hot_log.as_deref() { hl.error(format!("Hot Restart failed: {e}"), t); }
            if let Some(details) = detail_block {
                if let Some(c) = console.as_deref() {
                    c.error("Hot Restart build log (full):");
                    c.error(details.clone());
                }
                if let Some(hl) = hot_log.as_deref() {
                    hl.error("Hot Restart build log (full):", t);
                    hl.error(details, t);
                }
            }
            rt.as_mut().build_log_accum.clear();
        }
        Err(crossbeam_channel::TryRecvError::Empty) => {}
        Err(crossbeam_channel::TryRecvError::Disconnected) => {
            rt.as_mut().build_spawn = None;
            rt.as_mut().build_log = None;
            rt.as_mut().build_log_accum.clear();
        }
    }
}

fn handle_hot_restart_requests_system(
    mut req: MessageReader<RequestHotRestart>,
    mut rt: ResMut<HotRestartRuntime>,
    q: Res<HotRestartQueue>,
    build_settings: Res<crate::build_settings::BuildSettings>,
    snapshot: Res<crate::nodes::graph_model::NodeGraphSnapshotRes>,
    node_editor: Res<NodeEditorState>,
    ui: Res<UiState>,
    tab_viewer: Res<TabViewer>,
    cam_q: Query<(&Transform, &CameraController), With<crate::MainCamera>>,
    float_reg: Res<FloatingTabRegistry>,
    windows: Query<&Window>,
    console: Option<Res<ConsoleLog>>,
    hot_log: Option<Res<crate::tabs_system::pane::hot_reload::HotReloadLog>>,
    time: Res<Time>,
) {
    if req.is_empty() && q.rx.is_empty() { return; }
    if rt.waiting.is_some() || rt.build_spawn.is_some() {
        return;
    }
    // Coalesce requests: last wins.
    let mut build = true;
    for r in req.read() {
        build = r.build;
    }
    for r in q.rx.try_iter() {
        build = r.build;
    }
    let Some(c) = console.as_deref() else { return; };
    let t = time.elapsed_secs();
    if let Some(hl) = hot_log.as_deref() { hl.warn("Hot Restart requested.", t); }
    // Capture snapshot now (fast, in-memory). Write to temp snapshot.
    let snap = capture_snapshot(&snapshot, &node_editor, &ui, &tab_viewer, &cam_q, &float_reg, &windows);
    let snap_path = default_snapshot_path();
    if let Err(e) = save_snapshot_atomic(&snap_path, &snap) {
        c.error(format!("Hot Restart: write snapshot failed: {e}"));
        if let Some(hl) = hot_log.as_deref() { hl.error(format!("Hot Restart snapshot write failed: {e}"), t); }
        return;
    }

    let Ok((port, token, rx)) = start_handoff_listener() else {
        c.error("Hot Restart: failed to start handoff listener.");
        if let Some(hl) = hot_log.as_deref() { hl.error("Hot Restart failed: handoff listener.", t); }
        return;
    };
    rt.as_mut().waiting = Some(HandoffWait { token: token.clone(), rx });

    // Build + spawn in background thread to avoid freezing the main thread.
    let (tx, rx2) = crossbeam_channel::unbounded::<Result<(), String>>();
    let (tx_log, rx_log) = crossbeam_channel::unbounded::<BuildLogLine>();
    rt.as_mut().build_log_accum.clear();
    rt.as_mut().build_spawn = Some(rx2);
    rt.as_mut().build_log = Some(rx_log);
    let snap_path2 = snap_path.clone();
    let token2 = token.clone();
    let target_dir = build_settings.cargo_target_dir_hot_restart();
    std::thread::spawn(move || {
        let exe = if build {
            build_new_exe_streaming(&target_dir, &tx_log)
        } else {
            std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))
        };
        let r = match exe {
            Ok(exe) => spawn_new_process(&exe, &snap_path2, port, &token2).map_err(|e| format!("spawn new process failed: {e}")),
            Err(e) => Err(e),
        };
        let _ = tx.send(r.map(|_| ()));
    });
    c.info("Hot Restart: building/spawning in background...");
    if let Some(hl) = hot_log.as_deref() { hl.info("Hot Restart: building/spawning in background...", t); }
}

fn build_new_exe_streaming(
    target_dir: &Path,
    tx: &crossbeam_channel::Sender<BuildLogLine>,
) -> Result<PathBuf, String> {
    let cargo = crate::cunning_core::plugin_system::rust_build::find_cargo()
        .unwrap_or_else(|| PathBuf::from("cargo"));
    let _ = tx.send(BuildLogLine {
        is_stderr: false,
        line: format!(
            "cmd: cargo build --bin cunning3d --color never --message-format short (target_dir={})",
            target_dir.display()
        ),
    });
    let mut cmd = Command::new(cargo);
    cmd.current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
        .arg("build")
        .arg("--bin")
        .arg("cunning3d")
        .arg("--color")
        .arg("never")
        .arg("--message-format")
        .arg("short")
        .env("CARGO_TARGET_DIR", target_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("spawn cargo failed: {e}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let raw_log = Arc::new(Mutex::new(String::new()));
    let tx2 = tx.clone();
    let raw2 = raw_log.clone();
    let t_out = std::thread::spawn(move || {
        let Some(s) = stdout else { return; };
        let mut r = BufReader::new(s);
        let mut line = Vec::<u8>::new();
        loop {
            line.clear();
            match r.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let decoded = String::from_utf8_lossy(&line).to_string();
                    let l = decoded.trim_end_matches(&['\r', '\n'][..]).to_string();
                    let _ = tx2.send(BuildLogLine { is_stderr: false, line: l });
                    if let Ok(mut raw) = raw2.lock() {
                        raw.push_str(decoded.as_str());
                    }
                }
                Err(_) => break,
            }
        }
    });
    let tx3 = tx.clone();
    let raw3 = raw_log.clone();
    let t_err = std::thread::spawn(move || {
        let Some(s) = stderr else { return; };
        let mut r = BufReader::new(s);
        let mut line = Vec::<u8>::new();
        loop {
            line.clear();
            match r.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let decoded = String::from_utf8_lossy(&line).to_string();
                    let l = decoded.trim_end_matches(&['\r', '\n'][..]).to_string();
                    let _ = tx3.send(BuildLogLine { is_stderr: true, line: l });
                    if let Ok(mut raw) = raw3.lock() {
                        raw.push_str(decoded.as_str());
                    }
                }
                Err(_) => break,
            }
        }
    });
    let status = child.wait().map_err(|e| format!("wait cargo failed: {e}"))?;
    let _ = t_out.join();
    let _ = t_err.join();
    if !status.success() {
        let mut raw = raw_log.lock().ok().map(|s| s.clone()).unwrap_or_default();
        if raw.trim().is_empty() {
            raw = capture_build_output_fallback(target_dir);
        }
        if raw.trim().is_empty() {
            return Err(format!(
                "cargo build failed (code={:?}) (no stdout/stderr captured)",
                status.code()
            ));
        }
        return Err(format!(
            "cargo build failed (code={:?})\n{}",
            status.code(),
            raw
        ));
    }
    let exe = built_exe_path(target_dir);
    if !exe.exists() {
        return Err(format!("build succeeded but exe missing: {}", exe.display()));
    }
    Ok(exe)
}

fn capture_build_output_fallback(target_dir: &Path) -> String {
    let cargo = crate::cunning_core::plugin_system::rust_build::find_cargo()
        .unwrap_or_else(|| PathBuf::from("cargo"));
    let out = Command::new(cargo)
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
        .arg("build")
        .arg("--bin")
        .arg("cunning3d")
        .arg("--color")
        .arg("never")
        .arg("--message-format")
        .arg("short")
        .env("CARGO_TARGET_DIR", target_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let Ok(out) = out else {
        return String::new();
    };
    let mut raw = String::from_utf8_lossy(&out.stdout).to_string();
    if !raw.is_empty() && !raw.ends_with('\n') {
        raw.push('\n');
    }
    raw.push_str(String::from_utf8_lossy(&out.stderr).as_ref());
    raw
}

#[derive(Message, Clone)]
pub struct HotRestartSnapshotReady(pub HotRestartSnapshot);

fn apply_restore_if_requested_system(
    settings: Res<HotRestartSettings>,
    mut node_graph_res: ResMut<crate::NodeGraphResource>,
    mut node_editor_state: ResMut<NodeEditorState>,
    mut ui: ResMut<UiState>,
    mut tab_viewer: ResMut<TabViewer>,
    mut cam_q: Query<(&mut Transform, &mut CameraController), With<crate::MainCamera>>,
    console: Option<Res<ConsoleLog>>,
) {
    let Some(path) = settings.restore_path.as_deref() else { return; };
    let Ok(snap) = load_snapshot(path) else {
        if let Some(c) = console.as_deref() { c.error("Hot Restart restore failed: snapshot read error"); }
        return;
    };
    node_graph_res.0 = crate::nodes::NodeGraph::from(snap.project.graph.clone());
    node_editor_state.pan = egui::vec2(snap.node_editor_pan[0], snap.node_editor_pan[1]);
    node_editor_state.zoom = snap.node_editor_zoom;
    node_editor_state.target_pan = node_editor_state.pan;
    node_editor_state.target_zoom = node_editor_state.zoom;
    node_editor_state.cda_path = snap.cda_path.clone();

    ui.layout_mode = match snap.layout_mode { LayoutModeId::Desktop => LayoutMode::Desktop, LayoutModeId::Tablet => LayoutMode::Tablet, LayoutModeId::Phone => LayoutMode::Phone };
    ui.mobile_active_tab = match snap.mobile_tab { MobileTabId::Viewport => MobileTab::Viewport, MobileTabId::NodeGraph => MobileTab::NodeGraph, MobileTabId::Properties => MobileTab::Properties, MobileTabId::Console => MobileTab::Console };
    ui.selected_nodes = snap.selected_nodes.iter().cloned().collect::<HashSet<_>>();
    ui.last_selected_node_id = snap.last_selected_node_id;

    tab_viewer.dock_state = snap.main_dock.filter_map_tabs(|id| Some(build_tab(*id)));
    ui.shelf_dock_state = snap.shelf_dock.filter_map_tabs(|id| Some(shelf_tab_from_id(*id)));

    if let (Some(cam), Ok((mut t, mut c))) = (snap.camera, cam_q.single_mut()) {
        t.translation = Vec3::new(cam.translation[0], cam.translation[1], cam.translation[2]);
        t.rotation = Quat::from_xyzw(cam.rotation[0], cam.rotation[1], cam.rotation[2], cam.rotation[3]);
        c.enabled = cam.ctrl_enabled;
        c.sensitivity = cam.ctrl_sensitivity;
        c.speed = cam.ctrl_speed;
        c.pivot = Vec3::new(cam.ctrl_pivot[0], cam.ctrl_pivot[1], cam.ctrl_pivot[2]);
    }
    if let Some(c) = console.as_deref() { c.info("Hot Restart restore applied."); }
}

fn send_handoff_ready_if_requested_system(
    settings: Res<HotRestartSettings>,
    time: Res<Time>,
    mut fired: Local<bool>,
) {
    let (Some(port), Some(token)) = (settings.handoff_port, settings.handoff_token.as_deref()) else { return; };
    if *fired { return; }
    // Delay a bit to ensure first UI frame has run.
    if time.elapsed_secs() < 0.25 { return; }
    *fired = true;
    let msg = token.to_string();
    std::thread::spawn(move || {
        if let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) {
            let _ = s.write_all(msg.as_bytes());
        }
    });
}

