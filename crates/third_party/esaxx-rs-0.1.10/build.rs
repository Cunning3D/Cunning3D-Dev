#[cfg(feature = "cpp")]
#[cfg(not(target_os = "macos"))]
fn main() {
    let mut b = cc::Build::new();
    // Force MSVC runtime + iterator debug to match Rust (dynamic CRT, no debug iterators).
    if std::env::var("CARGO_CFG_TARGET_ENV").ok().as_deref() == Some("msvc") {
        b.flag("/MD").flag("/D_ITERATOR_DEBUG_LEVEL=0").flag("/D_HAS_ITERATOR_DEBUGGING=0").flag("/EHsc");
    }
    b
        .cpp(true)
        .flag("-std=c++11")
        .static_crt(false)
        .file("src/esaxx.cpp")
        .include("src")
        .compile("esaxx");
}

#[cfg(feature = "cpp")]
#[cfg(target_os = "macos")]
fn main() {
    cc::Build::new()
        .cpp(true)
        .flag("-std=c++11")
        .flag("-stdlib=libc++")
        .static_crt(false)
        .file("src/esaxx.cpp")
        .include("src")
        .compile("esaxx");
}

#[cfg(not(feature = "cpp"))]
fn main() {}
