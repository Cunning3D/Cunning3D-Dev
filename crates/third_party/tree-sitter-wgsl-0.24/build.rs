use std::path::Path;

fn main() {
    let src = Path::new("..").join("tree-sitter-wgsl-src2").join("src");
    cc::Build::new()
        .include(&src)
        .file(src.join("parser.c"))
        .file(src.join("scanner.c"))
        .flag_if_supported("-std=c11")
        .compile("tree-sitter-wgsl");
}

