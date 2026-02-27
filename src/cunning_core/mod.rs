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
pub mod libs;

// Prelude: convenient re-exports for other modules
pub mod prelude {
    pub use super::traits::node_interface::{NodeInteraction, NodeOp};
    pub use super::traits::pane_interface::PaneTab;
    // Uncomment after exporting concrete types in input/mod.rs
    // pub use super::input::{PcHandler, TouchHandler};
}
