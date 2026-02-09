#![deny(missing_docs)]
//! Patched bindgen_cuda (0.1.7): force MSVC host compilation to use /MD and disable debug iterators.

use rayon::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Error messages
#[derive(Debug)]
pub enum Error {}

/// Core builder to setup the bindings options
#[derive(Debug)]
pub struct Builder {
    cuda_root: Option<PathBuf>,
    kernel_paths: Vec<PathBuf>,
    watch: Vec<PathBuf>,
    include_paths: Vec<PathBuf>,
    compute_cap: Option<usize>,
    out_dir: PathBuf,
    extra_args: Vec<&'static str>,
}

impl Default for Builder {
    fn default() -> Self {
        let num_cpus = std::env::var("RAYON_NUM_THREADS").map_or_else(|_| num_cpus::get_physical(), |s| usize::from_str(&s).expect("RAYON_NUM_THREADS is not set to a valid integer"));
        rayon::ThreadPoolBuilder::new().num_threads(num_cpus).build_global().expect("build rayon global threadpool");
        let out_dir = std::env::var("OUT_DIR").expect("Expected OUT_DIR environment variable").into();
        let cuda_root = cuda_include_dir();
        let kernel_paths = default_kernels().unwrap_or_default();
        let include_paths = default_include().unwrap_or_default();
        Self { cuda_root, kernel_paths, watch: vec![], include_paths, extra_args: vec![], compute_cap: compute_cap().ok(), out_dir }
    }
}

/// Helper struct to create a rust file when buildings PTX files.
pub struct Bindings { write: bool, paths: Vec<PathBuf> }

fn default_kernels() -> Option<Vec<PathBuf>> { Some(glob::glob("src/**/*.cu").ok()?.map(|p| p.expect("Invalid path")).collect()) }
fn default_include() -> Option<Vec<PathBuf>> { Some(glob::glob("src/**/*.cuh").ok()?.map(|p| p.expect("Invalid path")).collect()) }

fn is_msvc() -> bool { std::env::var("TARGET").ok().map(|t| t.contains("msvc")).unwrap_or(false) }
fn add_msvc_host_flags(cmd: &mut std::process::Command) {
    if !is_msvc() { return; }
    // Match Rust (dynamic CRT) and avoid MSVC iterator debug level mismatches.
    // Important: push /MD last so it wins if the toolchain injects /MT by default.
    cmd.arg("-Xcompiler=/D_ITERATOR_DEBUG_LEVEL=0");
    cmd.arg("-Xcompiler=/D_HAS_ITERATOR_DEBUGGING=0");
    cmd.arg("-Xcompiler=/MD");
}

impl Builder {
    /// Setup the kernel paths. All path must be set at once and be valid files.
    pub fn kernel_paths<P: Into<PathBuf>>(mut self, paths: Vec<P>) -> Self {
        let paths: Vec<_> = paths.into_iter().map(|p| p.into()).collect();
        let inexistent: Vec<_> = paths.iter().filter(|f| !f.exists()).collect();
        if !inexistent.is_empty() { panic!("Kernels paths do not exist {inexistent:?}"); }
        self.kernel_paths = paths;
        self
    }

    /// Setup the paths that the lib depend on but does not need to build
    pub fn watch<T, P>(mut self, paths: T) -> Self where T: IntoIterator<Item = P>, P: Into<PathBuf> {
        let paths: Vec<_> = paths.into_iter().map(|p| p.into()).collect();
        let inexistent: Vec<_> = paths.iter().filter(|f| !f.exists()).collect();
        if !inexistent.is_empty() { panic!("Kernels paths do not exist {inexistent:?}"); }
        self.watch = paths;
        self
    }

    /// Setup the include files list.
    pub fn include_paths<P: Into<PathBuf>>(mut self, paths: Vec<P>) -> Self { self.include_paths = paths.into_iter().map(|p| p.into()).collect(); self }
    /// Sets kernels with a glob.
    pub fn kernel_paths_glob(mut self, g: &str) -> Self { self.kernel_paths = glob::glob(g).expect("Invalid glob").map(|p| p.expect("Invalid path")).collect(); self }
    /// Sets include files with a glob.
    pub fn include_paths_glob(mut self, g: &str) -> Self { self.include_paths = glob::glob(g).expect("Invalid glob").map(|p| p.expect("Invalid path")).collect(); self }
    /// Modifies output directory (default OUT_DIR).
    pub fn out_dir<P: Into<PathBuf>>(mut self, out_dir: P) -> Self { self.out_dir = out_dir.into(); self }
    /// Extra nvcc args.
    pub fn arg(mut self, arg: &'static str) -> Self { self.extra_args.push(arg); self }
    /// Forces cuda root.
    pub fn cuda_root<P: Into<PathBuf>>(&mut self, path: P) { self.cuda_root = Some(path.into()); }

    /// Build a static library in OUT_DIR and emit link directives.
    pub fn build_lib<P: Into<PathBuf>>(&self, out_file: P) {
        let out_file = out_file.into();
        let compute_cap = self.compute_cap.expect("Failed to get compute_cap");
        let out_dir = self.out_dir.clone();
        for p in &self.watch { println!("cargo:rerun-if-changed={}", p.display()); }
        let cu_files: Vec<_> = self.kernel_paths.iter().map(|f| {
            let mut s = DefaultHasher::new(); f.display().to_string().hash(&mut s); let hash = s.finish();
            let mut obj = out_dir.join(format!("{}-{:x}", f.file_stem().expect("filename").to_string_lossy(), hash)); obj.set_extension("o"); (f, obj)
        }).collect();
        let out_modified = out_file.metadata().and_then(|m| m.modified()).ok();
        let should_compile = out_modified.map(|m| {
            self.kernel_paths.iter().any(|e| e.metadata().unwrap().modified().unwrap().duration_since(m).is_ok()) ||
            self.watch.iter().any(|e| e.metadata().unwrap().modified().unwrap().duration_since(m).is_ok())
        }).unwrap_or(true);
        let ccbin_env = std::env::var("NVCC_CCBIN").ok();
        let nvcc = if Path::new("/usr/local/cuda/bin/nvcc").exists() { "/usr/local/cuda/bin/nvcc" } else { "nvcc" };
        if should_compile {
            cu_files.par_iter().for_each(|(cu, obj)| {
                let mut cmd = std::process::Command::new(nvcc);
                cmd.arg(format!("--gpu-architecture=sm_{compute_cap}")).arg("-c").args(["-o", obj.to_str().unwrap()]).args(["--default-stream", "per-thread"]).args(&self.extra_args);
                if let Some(ccbin) = &ccbin_env { cmd.arg("-allow-unsupported-compiler").args(["-ccbin", ccbin]); }
                add_msvc_host_flags(&mut cmd);
                cmd.arg(cu);
                let out = cmd.spawn().expect("spawn nvcc").wait_with_output().expect("nvcc output");
                if !out.status.success() { panic!("nvcc error compiling: {:?}\n\n# stdout\n{}\n\n# stderr\n{}", &cmd, String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)); }
            });
            let obj_files = cu_files.iter().map(|c| c.1.clone()).collect::<Vec<_>>();
            let mut cmd = std::process::Command::new(nvcc);
            cmd.arg("--lib").args(["-o", out_file.to_str().unwrap()]).args(obj_files);
            if let Some(ccbin) = &ccbin_env { cmd.arg("-allow-unsupported-compiler").args(["-ccbin", ccbin]); }
            let out = cmd.spawn().expect("spawn nvcc").wait_with_output().expect("nvcc output");
            if !out.status.success() { panic!("nvcc error linking: {:?}\n\n# stdout\n{}\n\n# stderr\n{}", &cmd, String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)); }

            // MSVC: crates often name archives `libfoo.a`, but rustc expects `foo.lib`.
            if is_msvc() {
                let name = out_file.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name.starts_with("lib") && name.ends_with(".a") {
                    let lib_name = format!("{}.lib", &name[3..name.len() - 2]);
                    let dst = out_dir.join(lib_name);
                    let _ = std::fs::copy(&out_file, dst);
                }
            }
        }
    }

    /// Build PTX for kernels.
    pub fn build_ptx(&self) -> Result<Bindings, Error> {
        let mut cuda_inc = PathBuf::from("/usr/local/cuda/include");
        if let Some(root) = &self.cuda_root { cuda_inc = root.join("include"); println!("cargo:rustc-env=CUDA_INCLUDE_DIR={}", cuda_inc.display()); }
        let compute_cap = self.compute_cap.expect("Could not find compute_cap");
        let out_dir = self.out_dir.clone();
        let mut include_paths = self.include_paths.clone();
        for p in &mut include_paths {
            println!("cargo:rerun-if-changed={}", p.display());
            let dst = out_dir.join(p.file_name().expect("filename"));
            std::fs::copy(p.clone(), dst).expect("copy include headers");
            p.pop();
        }
        include_paths.sort(); include_paths.dedup();
        let mut include_opts: Vec<String> = include_paths.into_iter().map(|s| "-I".to_string() + &s.into_os_string().into_string().expect("include string")).collect();
        include_opts.push(format!("-I{}", cuda_inc.display()));
        let ccbin_env = std::env::var("NVCC_CCBIN").ok();
        let nvcc = if Path::new("/usr/local/cuda/bin/nvcc").exists() { "/usr/local/cuda/bin/nvcc" } else { "nvcc" };
        println!("cargo:rerun-if-env-changed=NVCC_CCBIN");
        for p in &self.watch { println!("cargo:rerun-if-changed={}", p.display()); }
        let children = self.kernel_paths.par_iter().flat_map(|p| {
            println!("cargo:rerun-if-changed={}", p.display());
            let mut out = p.clone(); out.set_extension("ptx");
            let out_file = Path::new(&out_dir).join("out").with_file_name(out.file_name().unwrap());
            let ignore = out_file.metadata().ok().and_then(|m| m.modified().ok()).zip(p.metadata().ok().and_then(|m| m.modified().ok())).map(|(o,i)| o.duration_since(i).is_ok()).unwrap_or(false);
            if ignore { None } else {
                let mut cmd = std::process::Command::new(nvcc);
                cmd.arg(format!("--gpu-architecture=sm_{compute_cap}")).arg("--ptx").args(["--default-stream","per-thread"]).args(["--output-directory",&out_dir.display().to_string()]).args(&self.extra_args).args(&include_opts);
                if let Some(ccbin) = &ccbin_env { cmd.arg("-allow-unsupported-compiler").args(["-ccbin", ccbin]); }
                add_msvc_host_flags(&mut cmd);
                cmd.arg(p);
                Some((p.clone(), format!("{cmd:?}"), cmd.spawn().expect("spawn nvcc").wait_with_output()))
            }
        }).collect::<Vec<_>>();
        let ptx_paths: Vec<PathBuf> = glob::glob(&format!("{}/**/*.ptx", out_dir.display())).expect("glob").map(|p| p.expect("ptx path")).collect();
        let write = !children.is_empty() || self.kernel_paths.len() < ptx_paths.len();
        for (k, cli, child) in children {
            let out = child.expect("nvcc run");
            assert!(out.status.success(), "nvcc error while compiling {k:?}:\n\n# CLI {cli}\n\n# stdout\n{}\n\n# stderr\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        }
        Ok(Bindings { write, paths: self.kernel_paths.clone() })
    }
}

impl Bindings {
    /// Writes helper rust file including PTX sources.
    pub fn write<P: AsRef<Path>>(&self, out: P) -> Result<(), Error> {
        if self.write {
            let mut f = std::fs::File::create(out).expect("create file");
            for k in &self.paths {
                let name = k.file_stem().unwrap().to_str().unwrap();
                f.write_all(format!(r#"pub const {}: &str = include_str!(concat!(env!("OUT_DIR"), "/{}.ptx"));"#, name.to_uppercase().replace('.', "_"), name).as_bytes()).unwrap();
                f.write_all(&[b'\n']).unwrap();
            }
        }
        Ok(())
    }
}

fn cuda_include_dir() -> Option<PathBuf> {
    let env_vars = ["CUDA_PATH","CUDA_ROOT","CUDA_TOOLKIT_ROOT_DIR","CUDNN_LIB"];
    let env_vars = env_vars.into_iter().map(std::env::var).filter_map(Result::ok).map(Into::<PathBuf>::into);
    let roots = ["/usr","/usr/local/cuda","/opt/cuda","/usr/lib/cuda","C:/Program Files/NVIDIA GPU Computing Toolkit","C:/CUDA"];
    let roots = roots.into_iter().map(Into::<PathBuf>::into);
    env_vars.chain(roots).find_map(|root| {
        let include = root.join("include");
        if include.join("cuda.h").exists() { Some(root) } else { None }
    })
}

fn compute_cap() -> Result<usize, ()> {
    // Minimal: allow override; otherwise fall back to 80.
    std::env::var("CUDA_COMPUTE_CAP").ok().and_then(|s| s.parse().ok()).ok_or(()).or(Ok(80))
}

