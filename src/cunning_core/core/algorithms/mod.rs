pub mod algorithms_runtime;
pub mod algorithms_editor;
pub mod algorithms_dcc;
pub mod merge;
pub mod transform;

// Back-compat re-export (many call sites expect `crate::libs::algorithms::boolean`)
pub mod boolean { pub use super::algorithms_editor::boolean::*; }

pub mod mc { pub use super::algorithms_runtime::mc::*; }
