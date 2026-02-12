use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_markdown() -> *const ();
    fn tree_sitter_markdown_inline() -> *const ();
}

pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_markdown) };
pub const LANGUAGE_INLINE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_markdown_inline) };

pub const NODE_TYPES: &str = include_str!("../../tree-sitter-markdown-src/tree-sitter-markdown/src/node-types.json");
pub const NODE_TYPES_INLINE: &str =
    include_str!("../../tree-sitter-markdown-src/tree-sitter-markdown-inline/src/node-types.json");

pub const HIGHLIGHTS_QUERY: &str =
    include_str!("../../tree-sitter-markdown-src/tree-sitter-markdown/queries/highlights.scm");
pub const INJECTIONS_QUERY: &str =
    include_str!("../../tree-sitter-markdown-src/tree-sitter-markdown/queries/injections.scm");
pub const HIGHLIGHTS_QUERY_INLINE: &str =
    include_str!("../../tree-sitter-markdown-src/tree-sitter-markdown-inline/queries/highlights.scm");
pub const INJECTIONS_QUERY_INLINE: &str =
    include_str!("../../tree-sitter-markdown-src/tree-sitter-markdown-inline/queries/injections.scm");

