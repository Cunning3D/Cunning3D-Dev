use std::path::Path;

fn main() {
    let root = Path::new("..").join("tree-sitter-markdown-src");
    let md = root.join("tree-sitter-markdown").join("src");
    let md_inline = root.join("tree-sitter-markdown-inline").join("src");

    cc::Build::new()
        .include(&md)
        .file(md.join("parser.c"))
        .file(md.join("scanner.c"))
        .include(&md_inline)
        .file(md_inline.join("parser.c"))
        .file(md_inline.join("scanner.c"))
        .flag_if_supported("-std=c11")
        .compile("tree-sitter-markdown");
}

