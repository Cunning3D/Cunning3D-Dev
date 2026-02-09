use std::{env, fs, path::Path};
use cunning_core::algorithms::algorithms_runtime::unity_spline::editor::harness::SplineContainerSnapshot;
use cunning_core::spline_snapshot_fbs::{encode_snapshot_fbs, encode_snapshot_fbs_zstd};

fn main() {
    let in_path = env::args().nth(1).unwrap_or_else(|| { eprintln!("Usage: spline_snapshot_pack <snapshot.json>"); std::process::exit(2) });
    let txt = fs::read_to_string(&in_path).unwrap_or_else(|e| { eprintln!("Failed to read {in_path}: {e}"); std::process::exit(2) });
    let snap: SplineContainerSnapshot = serde_json::from_str(&txt).unwrap_or_else(|e| { eprintln!("Invalid snapshot JSON: {e}"); std::process::exit(2) });
    let base = Path::new(&in_path);
    let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("spline_snapshot");
    let dir = base.parent().unwrap_or_else(|| Path::new("."));
    let out_fbs = dir.join(format!("{stem}.fbs.bin"));
    let out_zst = dir.join(format!("{stem}.fbs.zst"));
    fs::write(&out_fbs, encode_snapshot_fbs(&snap)).unwrap_or_else(|e| { eprintln!("Write failed {:?}: {e}", out_fbs); std::process::exit(1) });
    fs::write(&out_zst, encode_snapshot_fbs_zstd(&snap, 3)).unwrap_or_else(|e| { eprintln!("Write failed {:?}: {e}", out_zst); std::process::exit(1) });
    println!("Wrote:\n- {}\n- {}", out_fbs.display(), out_zst.display());
}

