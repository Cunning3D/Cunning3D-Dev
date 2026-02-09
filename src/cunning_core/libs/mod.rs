//! Legacy module aliases (thin re-exports) after repo restructuring.

pub mod algorithms { pub use crate::cunning_core::core::algorithms::*; }
pub mod geometry { pub use crate::cunning_core::core::geometry::*; }
pub mod ai_service { pub use crate::cunning_core::ai_service::*; }
pub mod voice { pub use crate::voice::*; }
pub mod codex_integration { pub const DEFAULT_CODEX_MCP_URL: &str = "http://127.0.0.1:3000"; }
