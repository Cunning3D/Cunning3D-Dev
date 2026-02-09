use std::{env, fs};
use cunning_core::algorithms::algorithms_runtime::unity_spline::editor::harness::{SplineSession, run_session, snapshot_container, export_json};

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| { eprintln!("Usage: spline_session_dump <session.json>"); std::process::exit(2) });
    let txt = fs::read_to_string(&path).unwrap_or_else(|e| { eprintln!("Failed to read {path}: {e}"); std::process::exit(2) });
    let session: SplineSession = serde_json::from_str(&txt).unwrap_or_else(|e| { eprintln!("Invalid session JSON: {e}"); std::process::exit(2) });
    let out = run_session(&session).unwrap_or_else(|e| { eprintln!("Session failed: {e}"); std::process::exit(1) });
    println!("{}", export_json(&snapshot_container(&out)));
}

