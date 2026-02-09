use std::{env, fs, path::{Path, PathBuf}};

fn first_existing(paths: &[PathBuf]) -> Option<PathBuf> { paths.iter().find(|p| p.exists()).cloned() }

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("wasm32") { return; }
    println!("cargo:rerun-if-env-changed=CUNNING_WASM_SANS_FONT");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dst = out.join("cunning_wasm_sans_font.bin");
    let src = env::var("CUNNING_WASM_SANS_FONT").ok().map(PathBuf::from).or_else(|| {
        #[cfg(windows)]
        {
            let windir = env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_owned());
            let fonts = PathBuf::from(format!("{windir}\\Fonts"));
            let cands = ["msyh.ttc", "msyh.ttf", "simhei.ttf", "simsun.ttc", "segoeui.ttf", "arial.ttf"].into_iter().map(|f| fonts.join(f)).collect::<Vec<_>>();
            first_existing(&cands)
        }
        #[cfg(not(windows))]
        { None }
    }).or_else(|| {
        let fallback = Path::new("..").join("cunning_egui").join("crates").join("epaint_default_fonts").join("fonts").join("Ubuntu-Light.ttf");
        println!("cargo:rerun-if-changed={}", fallback.display());
        if fallback.exists() { Some(fallback) } else { None }
    }).unwrap_or_else(|| panic!("No font found for wasm. Set CUNNING_WASM_SANS_FONT to a .ttf/.ttc path (e.g. C:\\\\Windows\\\\Fonts\\\\msyh.ttc)."));
    println!("cargo:rerun-if-changed={}", src.display());
    fs::create_dir_all(&out).ok();
    fs::copy(&src, &dst).unwrap_or_else(|e| panic!("Failed to copy font {} -> {}: {e}", src.display(), dst.display()));
}

