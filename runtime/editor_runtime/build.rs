use std::{env, fs, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    fs::write(out.join("build_id.rs"), format!("pub const BUILD_ID: u64 = {ts}u64;")).unwrap();
}

