//! Build + copy runtime modules into `runtime_modules/`.

use crate::app_jobs::{JobContext, JobError, JobOutput, JobPool, JobRunnable};
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct CompileRuntimeModuleRequest {
    pub module_name: String,
    pub crate_dir: PathBuf,
    pub release: bool,
    pub hot_load: bool,
    /// Cargo target dir to use for this build (from Settings → General/Build).
    pub target_dir: Option<PathBuf>,
}

impl CompileRuntimeModuleRequest {
    pub fn editor_runtime() -> Self {
        Self {
            module_name: "editor_runtime".into(),
            crate_dir: PathBuf::from("runtime").join("editor_runtime"),
            release: false,
            hot_load: true,
            target_dir: None,
        }
    }
}

#[derive(Debug)]
pub struct CompileRuntimeModuleOutput {
    pub module_name: String,
    pub built_dll: Option<PathBuf>,
    pub copied_dll: Option<PathBuf>,
    pub success: bool,
    pub raw_log: String,
}

#[derive(Message, Clone)]
pub struct CompileRuntimeModuleRequestMsg(pub CompileRuntimeModuleRequest);

pub struct RuntimeModuleBuildJobsPlugin;

static GLOBAL_COMPILE_RT_TX: OnceLock<crossbeam_channel::Sender<CompileRuntimeModuleRequest>> = OnceLock::new();

#[derive(Resource)]
struct CompileRuntimeModuleQueue {
    rx: crossbeam_channel::Receiver<CompileRuntimeModuleRequest>,
}

/// Enqueue a runtime module build from non-ECS callers (egui tabs).
pub fn request_compile_runtime_module(req: CompileRuntimeModuleRequest) -> Result<(), String> {
    let Some(tx) = GLOBAL_COMPILE_RT_TX.get() else { return Err("Runtime module queue not initialized".into()); };
    tx.send(req).map_err(|_| "Runtime module send failed".into())
}

impl Plugin for RuntimeModuleBuildJobsPlugin {
    fn build(&self, app: &mut App) {
        let (tx, rx) = crossbeam_channel::unbounded::<CompileRuntimeModuleRequest>();
        let _ = GLOBAL_COMPILE_RT_TX.set(tx);
        app.insert_resource(CompileRuntimeModuleQueue { rx });
        app.add_systems(
            Update,
            (
                enqueue_compile_runtime_jobs_from_queue_system,
                compile_runtime_module_requests_to_jobs_system,
                apply_completed_runtime_module_jobs_system,
            ),
        );
    }
}

fn enqueue_compile_runtime_jobs_from_queue_system(
    q: Res<CompileRuntimeModuleQueue>,
    build_settings: Res<crate::build_settings::BuildSettings>,
    mut jobs: ResMut<crate::app_jobs::AppJobs>,
) {
    for mut r in q.rx.try_iter() {
        if r.target_dir.is_none() {
            r.target_dir = Some(build_settings.cargo_target_dir_runtime_modules());
        }
        jobs.enqueue(Box::new(CompileRuntimeModuleJob { req: r }));
    }
}

fn compile_runtime_module_requests_to_jobs_system(
    mut req: MessageReader<CompileRuntimeModuleRequestMsg>,
    build_settings: Res<crate::build_settings::BuildSettings>,
    mut jobs: ResMut<crate::app_jobs::AppJobs>,
) {
    for r in req.read() {
        let mut r = r.0.clone();
        if r.target_dir.is_none() {
            r.target_dir = Some(build_settings.cargo_target_dir_runtime_modules());
        }
        jobs.enqueue(Box::new(CompileRuntimeModuleJob { req: r }));
    }
}

fn apply_completed_runtime_module_jobs_system(
    mut jobs: ResMut<crate::app_jobs::AppJobs>,
    mut state: ResMut<crate::runtime_module::RuntimeModuleState>,
    time: Res<Time>,
    log: Res<crate::runtime_module::RuntimeModuleLog>,
) {
    let mut done: Vec<crate::app_jobs::JobId> = Vec::new();
    while let Some(id) = jobs.completed_queue().pop_front() { done.push(id); }
    for id in done {
        let Some(out) = jobs.take_output(id) else { continue };
        let Ok(o) = out.downcast::<CompileRuntimeModuleOutput>() else { continue };
        let t = time.elapsed_secs();
        if o.success {
            log.push(format!("Runtime module build OK: {} ({})", o.module_name, o.copied_dll.as_ref().map(|p| p.display().to_string()).unwrap_or_default()));
            if let Some(p) = o.copied_dll.as_deref() {
                if let Ok(m) = crate::runtime_module::load_runtime_module(p) {
                    state.last_loaded_path = Some(p.to_path_buf());
                    state.module = Some(m);
                    log.push(format!("Runtime module hot-loaded: {}", o.module_name));
                    let _ = t;
                } else {
                    log.push("Runtime module load failed.".to_string());
                }
            }
        } else {
            log.push(format!("Runtime module build FAILED: {} (see job log)", o.module_name));
            let _ = t;
        }
    }
}

struct CompileRuntimeModuleJob {
    req: CompileRuntimeModuleRequest,
}

impl JobRunnable for CompileRuntimeModuleJob {
    fn title(&self) -> String { format!("Build runtime module: {}", self.req.module_name) }
    fn kind(&self) -> &'static str { "compile_runtime_module" }
    fn pool(&self) -> JobPool { JobPool::Io }
    fn start(self: Box<Self>, cx: JobContext) -> bevy::tasks::Task<Result<JobOutput, JobError>> {
        let req = self.req.clone();
        IoTaskPool::get().spawn(async move {
            let crate_dir = req.crate_dir.clone();
            let args = if req.release { vec!["build", "--release"] } else { vec!["build"] };
            let cargo = crate::cunning_core::plugin_system::rust_build::find_cargo().unwrap_or_else(|| PathBuf::from("cargo"));
            let args_ref: Vec<&str> = args.iter().map(|s| *s).collect();
            let (ok, raw_log) = run_cargo_streaming(&cargo, &crate_dir, &args_ref, &cx, req.target_dir.as_deref()).map_err(JobError::from)?;
            if !ok {
                return Ok(Box::new(CompileRuntimeModuleOutput {
                    module_name: req.module_name,
                    built_dll: None,
                    copied_dll: None,
                    success: false,
                    raw_log,
                }) as JobOutput);
            }
            let built = expected_windows_dll_path(&crate_dir, req.release, req.target_dir.as_deref())?;
            let copied = crate::cunning_core::plugin_system::rust_build::copy_versioned(
                &built,
                &crate::runtime_module::runtime_modules_dir(),
                &req.module_name,
            )
            .map_err(JobError::from)?;
            Ok(Box::new(CompileRuntimeModuleOutput {
                module_name: req.module_name,
                built_dll: Some(built),
                copied_dll: Some(copied),
                success: true,
                raw_log,
            }) as JobOutput)
        })
    }
}

fn expected_windows_dll_path(crate_dir: &Path, release: bool, target_dir: Option<&Path>) -> Result<PathBuf, JobError> {
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
                if !rhs.is_empty() { found = Some(rhs.to_string()); break; }
            }
        }
        found.ok_or_else(|| JobError::from("Failed to read crate name from Cargo.toml"))?
    };
    let base = target_dir.map(|p| p.to_path_buf()).unwrap_or_else(|| crate_dir.join("target"));
    let dll = base.join(mode).join(format!("{name}.dll"));
    if !dll.exists() { return Err(JobError::from(format!("Build OK but missing output DLL: {}", dll.display()))); }
    Ok(dll)
}

fn run_cargo_streaming(
    cargo_path: &Path,
    cwd: &Path,
    args: &[&str],
    cx: &JobContext,
    target_dir: Option<&Path>,
) -> Result<(bool, String), String> {
    let mut cmd = Command::new(cargo_path);
    cmd.current_dir(cwd)
        .args(args)
        .env_remove("CARGO_TARGET_DIR")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(td) = target_dir {
        cmd.env("CARGO_TARGET_DIR", td);
    }
    let mut child = cmd.spawn().map_err(|e| format!("spawn cargo failed: {e}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = crossbeam_channel::unbounded::<(bool, String)>(); // (is_stderr, line)
    let t_out = spawn_line_reader(stdout, tx.clone(), false);
    let t_err = spawn_line_reader(stderr, tx.clone(), true);
    let mut raw_log = String::new();
    loop {
        while let Ok((is_err, line)) = rx.try_recv() {
            let _ = cx.log.send(crate::app_jobs::JobLogLine {
                level: if is_err { crate::app_jobs::JobLogLevel::Error } else { crate::app_jobs::JobLogLevel::Info },
                message: line.clone(),
            });
            raw_log.push_str(&line);
            raw_log.push('\n');
        }
        if cx.cancel.is_cancelled() {
            let _ = child.kill();
            return Err("Cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                for _ in 0..256 {
                    if let Ok((is_err, line)) = rx.try_recv() {
                        let _ = cx.log.send(crate::app_jobs::JobLogLine {
                            level: if is_err { crate::app_jobs::JobLogLevel::Error } else { crate::app_jobs::JobLogLevel::Info },
                            message: line.clone(),
                        });
                        raw_log.push_str(&line);
                        raw_log.push('\n');
                    } else {
                        break;
                    }
                }
                let _ = t_out.join();
                let _ = t_err.join();
                return Ok((status.success(), raw_log));
            }
            Ok(None) => {
                let _ = rx.recv_timeout(Duration::from_millis(20));
            }
            Err(e) => {
                let _ = t_out.join();
                let _ = t_err.join();
                return Err(format!("cargo wait failed: {e}"));
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

