//! IDE subsystem: Worktree, Document, Search (Zed-isomorphic architecture)

pub mod worktree;
pub mod document;
pub mod text_core;
pub mod display_map;
pub mod syntax;
pub mod lsp;

pub use worktree::*;
pub use document::*;
pub use text_core::*;
pub use display_map::*;
pub use syntax::*;
pub use lsp::*;