//! Rust plugin build utilities (bundled toolchain preferred).

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;

#[inline]
fn is_exe(p: &Path) -> bool {
    p.exists() && p.is_file()
}

pub fn find_bundled_cargo() -> Option<PathBuf> {
    let cands = [
        PathBuf::from("assets/toolchains/rust/windows-x86_64/bin/cargo.exe"),
        PathBuf::from("assets/toolchains/rust/windows/bin/cargo.exe"),
        PathBuf::from("assets/toolchains/rust/bin/cargo.exe"),
    ];
    cands.into_iter().find(|p| is_exe(p))
}

pub fn find_cargo() -> Option<PathBuf> {
    find_bundled_cargo().or_else(|| Some(PathBuf::from("cargo")))
}

pub fn run_cargo(cwd: &Path, args: &[&str]) -> std::io::Result<(PathBuf, Output)> {
    let cargo = find_cargo().unwrap_or_else(|| PathBuf::from("cargo"));
    // Avoid `Command::output()` to enable future streaming/cancellation.
    let mut child = Command::new(&cargo)
        .current_dir(cwd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Read stdout/stderr concurrently to avoid pipe deadlocks on large output.
    let mut out_stdout = child.stdout.take();
    let mut out_stderr = child.stderr.take();
    let t_out = thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        if let Some(mut s) = out_stdout.take() {
            use std::io::Read;
            s.read_to_end(&mut buf)?;
        }
        Ok(buf)
    });
    let t_err = thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        if let Some(mut s) = out_stderr.take() {
            use std::io::Read;
            s.read_to_end(&mut buf)?;
        }
        Ok(buf)
    });

    let status = child.wait()?;
    let stdout = t_out
        .join()
        .unwrap_or_else(|_| Ok(Vec::new()))
        .unwrap_or_default();
    let stderr = t_err
        .join()
        .unwrap_or_else(|_| Ok(Vec::new()))
        .unwrap_or_default();
    let out = Output {
        status,
        stdout,
        stderr,
    };
    Ok((cargo, out))
}

fn crate_name_from_toml(crate_dir: &Path) -> Option<String> {
    let s = std::fs::read_to_string(crate_dir.join("Cargo.toml")).ok()?;
    for line in s.lines() {
        let line = line.trim();
        if line.starts_with("name") {
            let rhs = line.splitn(2, '=').nth(1)?.trim();
            let rhs = rhs.trim_matches('"').trim_matches('\'').trim();
            if !rhs.is_empty() {
                return Some(rhs.to_string());
            }
        }
    }
    None
}

/// Structured compiler error from rustc JSON output
#[derive(Debug, Clone, Default)]
pub struct CompilerError {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub code: String,
    pub message: String,
    pub level: String,    // "error", "warning"
    pub rendered: String, // Full rendered text with context
}

#[derive(Deserialize)]
struct CargoMessage {
    reason: String,
    #[serde(default)]
    message: Option<RustcMessage>,
}

#[derive(Deserialize)]
struct RustcMessage {
    #[serde(default)]
    code: Option<RustcCode>,
    #[serde(default)]
    level: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    spans: Vec<RustcSpan>,
    #[serde(default)]
    rendered: Option<String>,
}

#[derive(Deserialize)]
struct RustcCode {
    code: String,
}

#[derive(Deserialize)]
struct RustcSpan {
    #[serde(default)]
    file_name: String,
    #[serde(default)]
    line_start: u32,
    #[serde(default)]
    column_start: u32,
    #[serde(default)]
    is_primary: bool,
}

/// Parse a single cargo `--message-format=json` line into a structured compiler error (only errors).
pub fn parse_compiler_error_from_cargo_json_line(line: &str) -> Option<CompilerError> {
    let msg = serde_json::from_str::<CargoMessage>(line).ok()?;
    if msg.reason != "compiler-message" {
        return None;
    }
    let m = msg.message?;
    if m.level != "error" {
        return None;
    }
    let span = m
        .spans
        .iter()
        .find(|s| s.is_primary)
        .or_else(|| m.spans.first());
    Some(CompilerError {
        file: span.map(|s| s.file_name.clone()).unwrap_or_default(),
        line: span.map(|s| s.line_start).unwrap_or(0),
        column: span.map(|s| s.column_start).unwrap_or(0),
        code: m.code.map(|c| c.code).unwrap_or_default(),
        message: m.message,
        level: m.level,
        rendered: m.rendered.unwrap_or_default(),
    })
}

/// Fast check using cargo check --message-format=json (no codegen, faster feedback)
pub fn check_fast(crate_dir: &Path) -> Result<Vec<CompilerError>, String> {
    let args = ["check", "--message-format=json"];
    let (_cargo, out) =
        run_cargo(crate_dir, &args).map_err(|e| format!("cargo check failed: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut errors = Vec::new();
    for line in stdout.lines() {
        if let Ok(msg) = serde_json::from_str::<CargoMessage>(line) {
            if msg.reason == "compiler-message" {
                if let Some(m) = msg.message {
                    if m.level == "error" {
                        let span = m
                            .spans
                            .iter()
                            .find(|s| s.is_primary)
                            .or_else(|| m.spans.first());
                        errors.push(CompilerError {
                            file: span.map(|s| s.file_name.clone()).unwrap_or_default(),
                            line: span.map(|s| s.line_start).unwrap_or(0),
                            column: span.map(|s| s.column_start).unwrap_or(0),
                            code: m.code.map(|c| c.code).unwrap_or_default(),
                            message: m.message.clone(),
                            level: m.level.clone(),
                            rendered: m.rendered.unwrap_or_default(),
                        });
                    }
                }
            }
        }
    }
    if out.status.success() {
        Ok(errors)
    } else {
        Ok(errors)
    }
}

/// Build result with structured errors
pub struct BuildResult {
    pub success: bool,
    pub cargo_path: PathBuf,
    pub dll_path: Option<PathBuf>,
    pub raw_log: String,
    pub errors: Vec<CompilerError>,
}

pub fn build_cdylib_structured(
    crate_dir: &Path,
    release: bool,
    offline: bool,
    locked: bool,
) -> BuildResult {
    // First do a fast check to get structured errors
    let check_errors = check_fast(crate_dir).unwrap_or_default();
    if !check_errors.is_empty() {
        return BuildResult {
            success: false,
            cargo_path: find_cargo().unwrap_or_else(|| PathBuf::from("cargo")),
            dll_path: None,
            raw_log: check_errors
                .iter()
                .map(|e| e.rendered.clone())
                .collect::<Vec<_>>()
                .join("\n"),
            errors: check_errors,
        };
    }
    // Check passed, now do full build
    match build_cdylib(crate_dir, release, offline, locked) {
        Ok((cargo, dll, log)) => BuildResult {
            success: true,
            cargo_path: cargo,
            dll_path: Some(dll),
            raw_log: log,
            errors: Vec::new(),
        },
        Err(log) => {
            // Parse errors from raw log as fallback
            let errors = parse_errors_from_log(&log);
            BuildResult {
                success: false,
                cargo_path: find_cargo().unwrap_or_else(|| PathBuf::from("cargo")),
                dll_path: None,
                raw_log: log,
                errors,
            }
        }
    }
}

fn parse_errors_from_log(log: &str) -> Vec<CompilerError> {
    let mut errors = Vec::new();
    let mut last_err: Option<CompilerError> = None;
    for line in log.lines() {
        let l = line.trim();
        if l.starts_with("error[") || l.starts_with("error:") {
            if let Some(e) = last_err.take() {
                errors.push(e);
            }
            let code = l
                .find('[')
                .and_then(|i| {
                    l[i + 1..]
                        .find(']')
                        .map(|j| l[i + 1..i + 1 + j].to_string())
                })
                .unwrap_or_default();
            let msg = l
                .splitn(2, "]: ")
                .nth(1)
                .or_else(|| l.splitn(2, ": ").nth(1))
                .unwrap_or(l)
                .to_string();
            last_err = Some(CompilerError {
                code,
                message: msg,
                level: "error".into(),
                ..Default::default()
            });
        } else if l.starts_with("-->") {
            if let Some(e) = last_err.as_mut() {
                let loc = l
                    .trim_start_matches("--> ")
                    .trim_start_matches("-->")
                    .trim();
                let parts: Vec<&str> = loc.rsplitn(3, ':').collect();
                if parts.len() >= 3 {
                    e.file = parts[2].to_string();
                    e.line = parts[1].parse().unwrap_or(0);
                    e.column = parts[0].parse().unwrap_or(0);
                }
            }
        }
    }
    if let Some(e) = last_err {
        errors.push(e);
    }
    errors
}

pub fn build_cdylib(
    crate_dir: &Path,
    release: bool,
    offline: bool,
    locked: bool,
) -> Result<(PathBuf, PathBuf, String), String> {
    let mut args: Vec<&str> = vec!["build"];
    if release {
        args.push("--release");
    }
    if offline {
        args.push("--offline");
    }
    if locked {
        args.push("--locked");
    }
    let (cargo_path, out) =
        run_cargo(crate_dir, &args).map_err(|e| format!("Failed to invoke cargo: {e}"))?;
    let mut log = String::new();
    log.push_str(&String::from_utf8_lossy(&out.stdout));
    log.push_str(&String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        return Err(log);
    }
    let mode = if release { "release" } else { "debug" };
    let name = crate_name_from_toml(crate_dir)
        .ok_or_else(|| "Failed to read crate name from Cargo.toml".to_string())?;
    let dll = crate_dir
        .join("target")
        .join(mode)
        .join(format!("{name}.dll"));
    if !dll.exists() {
        return Err(format!(
            "Build OK but missing output DLL: {}\n{}",
            dll.display(),
            log
        ));
    }
    Ok((cargo_path, dll, log))
}

pub fn copy_versioned(dll_path: &Path, out_dir: &Path, base_name: &str) -> Result<PathBuf, String> {
    let _ = std::fs::create_dir_all(out_dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let out = out_dir.join(format!("{base_name}_{ts}.dll"));
    std::fs::copy(dll_path, &out).map_err(|e| format!("Copy failed: {e}"))?;
    Ok(out)
}
