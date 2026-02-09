//! Layout helpers (h_flex, v_flex) inspired by Zed

use gpui::{Div, Styled, div, px, Pixels};

/// Horizontal flex container
pub fn h_flex() -> Div { div().flex().flex_row().items_center() }

/// Vertical flex container
pub fn v_flex() -> Div { div().flex().flex_col() }

/// Horizontal flex with gap
pub fn h_group(gap: Pixels) -> Div { h_flex().gap(gap) }

/// Vertical flex with gap
pub fn v_group(gap: Pixels) -> Div { v_flex().gap(gap) }
