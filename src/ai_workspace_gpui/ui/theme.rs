//! Theme colors for AI Workspace GPUI (dark theme)

use gpui::{Hsla, rgb};

/// Theme color palette
pub struct ThemeColors;
impl ThemeColors {
    pub fn bg_primary() -> Hsla { rgb(0x111111).into() }
    pub fn bg_secondary() -> Hsla { rgb(0x1e1e1e).into() }
    pub fn bg_elevated() -> Hsla { rgb(0x2b2b2b).into() }
    pub fn bg_hover() -> Hsla { rgb(0x2f2f2f).into() }
    pub fn bg_active() -> Hsla { rgb(0x3a3a3a).into() }
    pub fn bg_selected() -> Hsla { rgb(0x264f78).into() }
    pub fn border() -> Hsla { rgb(0x2a2a2a).into() }
    pub fn border_focus() -> Hsla { rgb(0x007acc).into() }
    pub fn text_primary() -> Hsla { rgb(0xffffff).into() }
    pub fn text_secondary() -> Hsla { rgb(0xa0a0a0).into() }
    pub fn text_muted() -> Hsla { rgb(0x777777).into() }
    pub fn text_accent() -> Hsla { rgb(0x007acc).into() }
    pub fn text_success() -> Hsla { rgb(0x4ec9b0).into() }
    pub fn text_warning() -> Hsla { rgb(0xdcdcaa).into() }
    pub fn text_error() -> Hsla { rgb(0xf14c4c).into() }
    pub fn btn_success() -> Hsla { rgb(0x1b3a1b).into() }
    pub fn btn_danger() -> Hsla { rgb(0x3a1b1b).into() }
    // Diff colors
    pub fn diff_added_bg() -> Hsla { rgb(0x1b3a1b).into() }
    pub fn diff_added_text() -> Hsla { rgb(0x4ec9b0).into() }
    pub fn diff_removed_bg() -> Hsla { rgb(0x3a1b1b).into() }
    pub fn diff_removed_text() -> Hsla { rgb(0xf14c4c).into() }
}
