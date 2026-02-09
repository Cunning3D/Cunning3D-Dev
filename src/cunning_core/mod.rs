pub mod cda;
pub mod command;
pub mod core { pub mod algorithms { pub use cunning_kernel::algorithms::*; } pub mod geometry { pub use cunning_kernel::geometry::*; } }
pub mod graph;
pub mod input;
pub mod plugin_system;
pub mod profiling;
pub mod registries;
pub mod scripting;
pub mod traits;
pub mod ui;
pub mod ai_service;

// Prelude 方便其他模块引用核心功能
pub mod prelude {
    pub use super::traits::node_interface::{NodeInteraction, NodeOp};
    pub use super::traits::pane_interface::PaneTab;
    // 稍后在 input/mod.rs 中导出具体类型后取消注释
    // pub use super::input::{PcHandler, TouchHandler};
}
