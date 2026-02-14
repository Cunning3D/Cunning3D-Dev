//! AppJobs integration for Rust plugin build (no-hitch).
//!
//! - Runs `cargo check` + `cargo build` in background (IoTaskPool)
//! - Streams stdout/stderr incrementally into JobLog
//! - Supports cancellation by killing the child process
//! - On success, copies a versioned DLL and triggers hot-load on main thread

use crate::app_jobs::{JobContext, JobError, JobOutput, JobPool, JobRunnable};
use crate::console::ConsoleLog;
use crate::cunning_core::plugin_system::{rust_build, PluginSystem};
use crate::cunning_core::registries::node_registry::NodeRegistry;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use once_cell::sync::OnceCell;

static GLOBAL_COMPILE_REQ_TX: OnceCell<crossbeam_channel::Sender<CompileRustPluginRequest>> =
    OnceCell::new();

/// Request a plugin compile from non-Bevy contexts (tools/agents).
/// Returns Err if the app queue is not initialized yet.
pub fn request_compile_rust_plugin(req: CompileRustPluginRequest) -> Result<(), String> {
    let Some(tx) = GLOBAL_COMPILE_REQ_TX.get() else {
        return Err("Compile queue not initialized".to_string());
    };
    tx.send(req)
        .map_err(|_| "Compile queue send failed".to_string())
}

#[derive(Message, Clone, Debug)]
pub struct CompileRustPluginRequest {
    pub plugin_name: String,
    pub crate_dir: PathBuf,
    pub release: bool,
    pub offline: bool,
    pub locked: bool,
    /// Whether to hot-load after a successful build.
    pub hot_reload: bool,
    /// Cargo target dir to use for this build (from Settings → General/Build).
    pub target_dir: Option<PathBuf>,
}

impl CompileRustPluginRequest {
    pub fn for_extra_node(plugin_name: impl Into<String>) -> Self {
        let plugin_name = plugin_name.into();
        Self {
            crate_dir: PathBuf::from("plugins/extra_node").join(&plugin_name),
            plugin_name,
            release: true,
            offline: false,
            locked: false,
            hot_reload: true,
            target_dir: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompileRustPluginOutput {
    pub plugin_name: String,
    pub crate_dir: PathBuf,
    pub success: bool,
    pub cargo_path: PathBuf,
    pub built_dll: Option<PathBuf>,
    pub copied_dll: Option<PathBuf>,
    pub errors: Vec<rust_build::CompilerError>,
    pub raw_log: String,
    pub hot_reloaded: bool,
}

pub struct PluginBuildJobsPlugin;

impl Plugin for PluginBuildJobsPlugin {
    fn build(&self, app: &mut App) {
        // Global request queue for callers without ECS access.
        let (tx, rx) = crossbeam_channel::unbounded::<CompileRustPluginRequest>();
        let _ = GLOBAL_COMPILE_REQ_TX.set(tx);
        app.insert_resource(CompileRustPluginQueue { rx });

        app.add_systems(
            Update,
            (
                enqueue_compile_plugin_jobs_from_queue_system,
                enqueue_compile_plugin_jobs_system,
                apply_completed_compile_plugin_jobs_system,
            ),
        );
    }
}

#[derive(Resource)]
struct CompileRustPluginQueue {
    rx: crossbeam_channel::Receiver<CompileRustPluginRequest>,
}

fn enqueue_compile_plugin_jobs_from_queue_system(
    q: Res<CompileRustPluginQueue>,
    build_settings: Res<crate::build_settings::BuildSettings>,
    mut jobs: ResMut<crate::app_jobs::AppJobs>,
) {
    for mut r in q.rx.try_iter() {
        if r.target_dir.is_none() {
            r.target_dir = Some(build_settings.cargo_target_dir_plugins());
        }
        jobs.enqueue(Box::new(CompileRustPluginJob { req: r }));
    }
}

fn enqueue_compile_plugin_jobs_system(
    mut req: MessageReader<CompileRustPluginRequest>,
    build_settings: Res<crate::build_settings::BuildSettings>,
    mut jobs: ResMut<crate::app_jobs::AppJobs>,
) {
    for r in req.read() {
        let mut r = r.clone();
        if r.target_dir.is_none() {
            r.target_dir = Some(build_settings.cargo_target_dir_plugins());
        }
        jobs.enqueue(Box::new(CompileRustPluginJob { req: r }));
    }
}

fn apply_completed_compile_plugin_jobs_system(
    mut jobs: ResMut<crate::app_jobs::AppJobs>,
    time: Res<Time>,
    ps: Option<Res<PluginSystem>>,
    reg: Option<Res<NodeRegistry>>,
    console: Option<Res<ConsoleLog>>,
    hot_log: Option<Res<crate::tabs_system::pane::hot_reload::HotReloadLog>>,
) {
    let mut done: Vec<crate::app_jobs::JobId> = Vec::new();
    while let Some(id) = jobs.completed_queue().pop_front() { done.push(id); }
    for id in done {
        let Some(out) = jobs.take_output(id) else { continue };
        let Ok(o) = out.downcast::<CompileRustPluginOutput>() else { continue };
        let t = time.elapsed_secs();
        if o.success {
            let dll = o.copied_dll.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            if let Some(c) = console.as_deref() { c.info(format!("Plugin compiled: {} {}", o.plugin_name, dll)); }
            if let Some(hl) = hot_log.as_deref() { hl.info(format!("Build OK: {} → {}", o.plugin_name, dll), t); }
        } else {
            if let Some(c) = console.as_deref() { c.error(format!("Plugin compile failed: {}", o.plugin_name)); }
            if let Some(hl) = hot_log.as_deref() { hl.error(format!("Build FAILED: {}", o.plugin_name), t); }
            for err in &o.errors {
                if let Some(hl) = hot_log.as_deref() { hl.error(format!("  {}:{}: {}", err.file, err.line, err.message), t); }
            }
        }
        // Hot reload (main thread, best-effort)
        if o.success && o.hot_reloaded {
            if let (Some(ps), Some(reg)) = (ps.as_deref(), reg.as_deref()) {
                ps.scan_plugins_latest("plugins", &*reg);
                if let Some(hl) = hot_log.as_deref() { hl.info(format!("Hot-loaded plugin: {}", o.plugin_name), t); }
            }
        }
    }
}

struct CompileRustPluginJob {
    req: CompileRustPluginRequest,
}

impl JobRunnable for CompileRustPluginJob {
    fn title(&self) -> String {
        format!("编译插件: {}", self.req.plugin_name)
    }
    fn kind(&self) -> &'static str {
        "compile_rust_plugin"
    }
    fn pool(&self) -> JobPool {
        JobPool::Io
    }

    fn start(self: Box<Self>, cx: JobContext) -> bevy::tasks::Task<Result<JobOutput, JobError>> {
        let req = self.req.clone();
        IoTaskPool::get().spawn(async move {
            if cx.cancel.is_cancelled() {
                return Err("Cancelled".into());
            }

            let cargo_path = rust_build::find_cargo().unwrap_or_else(|| PathBuf::from("cargo"));

            let mut raw_log = String::new();
            let mut errors: Vec<rust_build::CompilerError> = Vec::new();

            let _ = cx.progress.send(crate::app_jobs::JobProgress {
                fraction: 0.02,
                message: "cargo check...".into(),
            });

            // 1) cargo check --message-format=json (structured errors)
            {
                let args = ["check", "--message-format=json"];
                let (ok, log, errs) = run_cargo_streaming(
                    &cargo_path,
                    &req.crate_dir,
                    &args,
                    &cx,
                    true, // parse json
                    req.target_dir.as_deref(),
                )?;
                raw_log.push_str(&log);
                errors.extend(errs);
                if !ok {
                    // Even if check failed, we already parsed errors. Return early.
                    return Ok(Box::new(CompileRustPluginOutput {
                        plugin_name: req.plugin_name.clone(),
                        crate_dir: req.crate_dir.clone(),
                        success: false,
                        cargo_path,
                        built_dll: None,
                        copied_dll: None,
                        errors,
                        raw_log,
                        hot_reloaded: false,
                    }) as JobOutput);
                }
            }

            if cx.cancel.is_cancelled() {
                return Err("Cancelled".into());
            }

            let _ = cx.progress.send(crate::app_jobs::JobProgress {
                fraction: 0.18,
                message: "cargo build...".into(),
            });

            // 2) cargo build (stream logs)
            {
                let mut args: Vec<&str> = vec!["build"];
                if req.release {
                    args.push("--release");
                }
                if req.offline {
                    args.push("--offline");
                }
                if req.locked {
                    args.push("--locked");
                }

                let (ok, log, _errs) = run_cargo_streaming(
                    &cargo_path,
                    &req.crate_dir,
                    &args,
                    &cx,
                    false,
                    req.target_dir.as_deref(),
                )?;
                raw_log.push_str(&log);
                if !ok {
                    return Ok(Box::new(CompileRustPluginOutput {
                        plugin_name: req.plugin_name.clone(),
                        crate_dir: req.crate_dir.clone(),
                        success: false,
                        cargo_path,
                        built_dll: None,
                        copied_dll: None,
                        errors,
                        raw_log,
                        hot_reloaded: false,
                    }) as JobOutput);
                }
            }

            if cx.cancel.is_cancelled() {
                return Err("Cancelled".into());
            }

            let _ = cx.progress.send(crate::app_jobs::JobProgress {
                fraction: 0.86,
                message: "复制 DLL...".into(),
            });

            let built = expected_windows_dll_path(&req.crate_dir, req.release, req.target_dir.as_deref())?;
            let copied = rust_build::copy_versioned(
                &built,
                Path::new("plugins"),
                &req.plugin_name,
            )
            .map_err(JobError::from)?;

            let _ = cx.progress.send(crate::app_jobs::JobProgress {
                fraction: 1.0,
                message: "完成".into(),
            });

            Ok(Box::new(CompileRustPluginOutput {
                plugin_name: req.plugin_name.clone(),
                crate_dir: req.crate_dir.clone(),
                success: true,
                cargo_path,
                built_dll: Some(built),
                copied_dll: Some(copied),
                errors,
                raw_log,
                hot_reloaded: req.hot_reload,
            }) as JobOutput)
        })
    }
}

fn expected_windows_dll_path(
    crate_dir: &Path,
    release: bool,
    target_dir: Option<&Path>,
) -> Result<PathBuf, JobError> {
    // Mirrors rust_build::build_cdylib logic.
    let mode = if release { "release" } else { "debug" };
    let name = {
        let s = std::fs::read_to_string(crate_dir.join("Cargo.toml"))
            .map_err(|e| JobError::from(format!("read Cargo.toml failed: {e}")))?;
        let mut found: Option<String> = None;
        for line in s.lines() {
            let line = line.trim();
            if line.starts_with("name") {
                let rhs = line.splitn(2, '=').nth(1).unwrap_or("").trim();
                let rhs = rhs.trim_matches('"').trim_matches('\'').trim();
                if !rhs.is_empty() {
                    found = Some(rhs.to_string());
                    break;
                }
            }
        }
        found.ok_or_else(|| JobError::from("Failed to read crate name from Cargo.toml"))?
    };

    let base = target_dir.map(|p| p.to_path_buf()).unwrap_or_else(|| crate_dir.join("target"));
    let dll = base.join(mode).join(format!("{name}.dll"));
    if !dll.exists() {
        return Err(JobError::from(format!(
            "Build OK but missing output DLL: {}",
            dll.display()
        )));
    }
    Ok(dll)
}

fn run_cargo_streaming(
    cargo_path: &Path,
    cwd: &Path,
    args: &[&str],
    cx: &JobContext,
    parse_json_errors: bool,
    target_dir: Option<&Path>,
) -> Result<(bool, String, Vec<rust_build::CompilerError>), JobError> {
    let mut cmd = Command::new(cargo_path);
    cmd.current_dir(cwd)
        .args(args)
        .env_remove("CARGO_TARGET_DIR")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(td) = target_dir {
        cmd.env("CARGO_TARGET_DIR", td);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| JobError::from(format!("spawn cargo failed: {e}")))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = crossbeam_channel::unbounded::<(bool, String)>(); // (is_stderr, line)

    let t_out = spawn_line_reader(stdout, tx.clone(), false);
    let t_err = spawn_line_reader(stderr, tx.clone(), true);

    let mut raw_log = String::new();
    let mut errors: Vec<rust_build::CompilerError> = Vec::new();

    loop {
        // Drain some output.
        while let Ok((is_err, line)) = rx.try_recv() {
            if is_err {
                let _ = cx.log.send(crate::app_jobs::JobLogLine {
                    level: crate::app_jobs::JobLogLevel::Error,
                    message: line.clone(),
                });
            } else {
                let _ = cx.log.send(crate::app_jobs::JobLogLine {
                    level: crate::app_jobs::JobLogLevel::Info,
                    message: line.clone(),
                });
            }
            raw_log.push_str(&line);
            raw_log.push('\n');

            if parse_json_errors && !is_err {
                if let Some(e) = rust_build::parse_compiler_error_from_cargo_json_line(&line) {
                    errors.push(e);
                }
            }
        }

        if cx.cancel.is_cancelled() {
            let _ = child.kill();
            return Err("Cancelled".into());
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                // Final drain.
                for _ in 0..256 {
                    if let Ok((is_err, line)) = rx.try_recv() {
                        if is_err {
                            let _ = cx.log.send(crate::app_jobs::JobLogLine {
                                level: crate::app_jobs::JobLogLevel::Error,
                                message: line.clone(),
                            });
                        } else {
                            let _ = cx.log.send(crate::app_jobs::JobLogLine {
                                level: crate::app_jobs::JobLogLevel::Info,
                                message: line.clone(),
                            });
                        }
                        raw_log.push_str(&line);
                        raw_log.push('\n');
                        if parse_json_errors && !is_err {
                            if let Some(e) =
                                rust_build::parse_compiler_error_from_cargo_json_line(&line)
                            {
                                errors.push(e);
                            }
                        }
                    } else {
                        break;
                    }
                }
                let _ = t_out.join();
                let _ = t_err.join();
                return Ok((status.success(), raw_log, errors));
            }
            Ok(None) => {
                // Wait a bit for more output / avoid busy spin.
                let _ = rx.recv_timeout(Duration::from_millis(20));
            }
            Err(e) => {
                let _ = t_out.join();
                let _ = t_err.join();
                return Err(JobError::from(format!("cargo wait failed: {e}")));
            }
        }
    }
}

fn spawn_line_reader(
    stream: Option<impl std::io::Read + Send + 'static>,
    tx: crossbeam_channel::Sender<(bool, String)>,
    is_stderr: bool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let Some(s) = stream else { return };
        let mut r = BufReader::new(s);
        let mut line = String::new();
        loop {
            line.clear();
            match r.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let l = line.trim_end_matches(&['\r', '\n'][..]).to_string();
                    let _ = tx.send((is_stderr, l));
                }
                Err(_) => break,
            }
        }
    })
}

