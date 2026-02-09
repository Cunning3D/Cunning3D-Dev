//! Back-compat facade for legacy paths (`crate::libs::*`).
pub mod geometry { pub use cunning_kernel::geometry::*; }
pub mod algorithms { pub use cunning_kernel::algorithms::*; }
pub mod ai_service { pub use crate::cunning_core::ai_service::*; }
pub mod voice { pub use crate::voice::service::*; }
pub mod codex_integration { pub use crate::cunning_core::libs::codex_integration::*; }

