use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_wgsl() -> *const ();
}

pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_wgsl) };
pub const NODE_TYPES: &str = include_str!("../../tree-sitter-wgsl-src2/src/node-types.json");
pub const HIGHLIGHTS_QUERY: &str = include_str!("../../tree-sitter-wgsl-src2/queries/highlights.scm");

