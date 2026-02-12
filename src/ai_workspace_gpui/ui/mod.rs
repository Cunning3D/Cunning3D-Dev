//! Lightweight UI components inspired by Zed's component architecture.
//! Provides Builder pattern APIs without requiring ui_macros or complex dependencies.

mod button;
mod label;
mod list_item;
mod layout;
mod theme;
pub mod text_input;
pub mod context_menu;
mod tooltip;
mod scrollbar;
pub mod mentions;
pub mod popover;
pub mod picker;
mod markdown;
mod resizable;

pub use button::*;
pub use label::*;
pub use list_item::*;
pub use layout::*;
pub use theme::*;
pub use text_input::*;
pub use context_menu::*;
pub use tooltip::*;
pub use scrollbar::*;
pub use mentions::*;
pub use popover::*;
pub use picker::*;
pub use markdown::*;
pub use resizable::*;

use gpui::{Pixels, px};

pub struct UiMetrics;
impl UiMetrics {
    pub const FONT_XSMALL: f32 = 10.0;
    pub const FONT_SMALL: f32 = 11.0;
    pub const FONT_DEFAULT: f32 = 12.0;
    pub const FONT_LARGE: f32 = 14.0;

    pub const BUTTON_TEXT: f32 = 12.0;
    pub const BUTTON_ICON_TEXT: f32 = 13.0;
    pub const BUTTON_ICON_MIN: f32 = 22.0;

    pub const TOOL_REPLAY_STEP_MS: u64 = 16;
    pub const TOOL_REPLAY_SWEEP_MS: u64 = 140;
}

/// Dynamic spacing values (simplified from Zed's DynamicSpacing)
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Spacing { Base02, Base04, Base06, Base08, Base12, Base16, Base20, Base24 }
impl Spacing {
    pub fn px(self) -> Pixels {
        match self { Self::Base02 => px(2.0), Self::Base04 => px(4.0), Self::Base06 => px(6.0), Self::Base08 => px(8.0), Self::Base12 => px(12.0), Self::Base16 => px(16.0), Self::Base20 => px(20.0), Self::Base24 => px(24.0) }
    }
}
