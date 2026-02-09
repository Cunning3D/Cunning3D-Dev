//! Profile-specific vmesh generators (SquareOut, SquareIn, Miter, etc.).
pub mod miter;
pub mod square_in;
pub mod square_out;
pub use super::super::structures::MiterType; // Re-export for convenience
