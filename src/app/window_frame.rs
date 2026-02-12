//! Window frame tuning constants (NetEase-like safe insets).

/// Content/UI safe inset (logical px) from window edges.
pub const WINDOW_SAFE_INSET_LP: f32 = 10.0;
/// Rounded corner radius baseline (logical px), scaled by DPI for region clipping.
pub const WINDOW_CORNER_RADIUS_LP: f32 = 14.0;
/// Window background (used for rounded-edge safe area).
pub const WINDOW_BG_SRGB: [f32; 3] = [0.10, 0.10, 0.10];
/// Unified UI surface background (Topbar/Shelf/Egui panels) in sRGBA.
pub const WINDOW_UI_SURFACE_BG_SRGBA: [f32; 4] = [0.18, 0.18, 0.20, 0.98];
/// Desktop topbar height (logical px).
pub const WINDOW_TOPBAR_H_LP: f32 = 30.0;
/// Desktop shelf bar height (logical px).
pub const WINDOW_SHELF_H_LP: f32 = 80.0;
/// Desktop window chrome button width (logical px).
pub const WINDOW_CHROME_BTN_W_LP: f32 = 22.0;
/// Desktop window chrome button height (logical px).
pub const WINDOW_CHROME_BTN_H_LP: f32 = 20.0;
/// Font family used for window chrome icons (min/max/close) to avoid missing-glyph tofu/garbage.
pub const WINDOW_CHROME_ICON_FONT_FAMILY: &str = "Segoe MDL2 Assets";
/// Window chrome glyphs (Segoe MDL2 Assets).
pub const WINDOW_CHROME_GLYPH_MIN: &str = "\u{E921}"; // ChromeMinimize
pub const WINDOW_CHROME_GLYPH_MAX: &str = "\u{E922}"; // ChromeMaximize
pub const WINDOW_CHROME_GLYPH_CLOSE: &str = "\u{E8BB}"; // ChromeClose

