/// Due to there being a lot of code to show a dock in a ui every complementing
/// method to ``show`` and ``show_inside`` is put in ``show_extra``.
/// Otherwise ``mod.rs`` would be humongous.
mod show;

// Various components of the `DockArea` which is used when rendering
mod allowed_splits;
mod drag_and_drop;
mod state;
mod tab_removal;

use crate::{dock_state::DockState, NodeIndex, Style, SurfaceIndex, TabIndex};
pub use allowed_splits::AllowedSplits;
use tab_removal::TabRemoval;

use egui::{emath::*, Color32, CornerRadius, Id, Margin, Stroke};

/// Displays a [`DockState`] in `egui`.
pub struct DockArea<'tree, Tab> {
    id: Id,
    dock_state: &'tree mut DockState<Tab>,
    style: Option<Style>,
    show_add_popup: bool,
    show_add_buttons: bool,
    show_close_buttons: bool,
    tab_context_menus: bool,
    draggable_tabs: bool,
    show_tab_name_on_hover: bool,
    show_window_close_buttons: bool,
    show_window_collapse_buttons: bool,
    allowed_splits: AllowedSplits,
    window_bounds: Option<Rect>,

    to_remove: Vec<TabRemoval>,
    to_detach: Vec<(SurfaceIndex, NodeIndex, TabIndex)>,
    new_focused: Option<(SurfaceIndex, NodeIndex)>,
    tab_hover_rect: Option<(Rect, TabIndex)>,
    on_tab_window: Option<
        Box<
            dyn FnMut(
                    &mut DockState<Tab>,
                    (SurfaceIndex, NodeIndex, TabIndex),
                    Rect,
                ) + 'tree,
        >,
    >,
}

// Builder
impl<'tree, Tab> DockArea<'tree, Tab> {
    /// Creates a new [`DockArea`] from the provided [`DockState`].
    #[inline(always)]
    pub fn new(tree: &'tree mut DockState<Tab>) -> DockArea<'tree, Tab> {
        Self {
            id: Id::new("egui_dock::DockArea"),
            dock_state: tree,
            style: None,
            show_add_popup: false,
            show_add_buttons: false,
            show_close_buttons: true,
            tab_context_menus: true,
            draggable_tabs: true,
            show_tab_name_on_hover: false,
            allowed_splits: AllowedSplits::default(),
            to_remove: Vec::new(),
            to_detach: Vec::new(),
            new_focused: None,
            tab_hover_rect: None,
            window_bounds: None,
            show_window_close_buttons: true,
            show_window_collapse_buttons: true,
            on_tab_window: None,
        }
    }

    /// Sets the [`DockArea`] ID. Useful if you have more than one [`DockArea`].
    #[inline(always)]
    pub fn id(mut self, id: Id) -> Self {
        self.id = id;
        self
    }

    /// Sets the look and feel of the [`DockArea`].
    #[inline(always)]
    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }

    /// Shows or hides the add button popup.
    /// By default it's `false`.
    pub fn show_add_popup(mut self, show_add_popup: bool) -> Self {
        self.show_add_popup = show_add_popup;
        self
    }

    /// Shows or hides the tab add buttons.
    /// By default it's `false`.
    pub fn show_add_buttons(mut self, show_add_buttons: bool) -> Self {
        self.show_add_buttons = show_add_buttons;
        self
    }

    /// Shows or hides the tab close buttons.
    /// By default it's `true`.
    pub fn show_close_buttons(mut self, show_close_buttons: bool) -> Self {
        self.show_close_buttons = show_close_buttons;
        self
    }

    /// Whether tabs show a context menu when right-clicked.
    /// By default it's `true`.
    pub fn tab_context_menus(mut self, tab_context_menus: bool) -> Self {
        self.tab_context_menus = tab_context_menus;
        self
    }

    /// Whether tabs can be dragged between nodes and reordered on the tab bar.
    /// By default it's `true`.
    pub fn draggable_tabs(mut self, draggable_tabs: bool) -> Self {
        self.draggable_tabs = draggable_tabs;
        self
    }

    /// Whether tabs show their name when hovered over them.
    /// By default it's `false`.
    pub fn show_tab_name_on_hover(mut self, show_tab_name_on_hover: bool) -> Self {
        self.show_tab_name_on_hover = show_tab_name_on_hover;
        self
    }

    /// What directions can a node be split in: left-right, top-bottom, all, or none.
    /// By default it's all.
    pub fn allowed_splits(mut self, allowed_splits: AllowedSplits) -> Self {
        self.allowed_splits = allowed_splits;
        self
    }

    /// The bounds for any windows inside the [`DockArea`]. Defaults to the screen rect.
    /// By default it's set to [`egui::Context::screen_rect`].
    #[inline(always)]
    pub fn window_bounds(mut self, bounds: Rect) -> Self {
        self.window_bounds = Some(bounds);
        self
    }

    /// Enables or disables the close button on windows.
    /// By default it's `true`.
    #[inline(always)]
    pub fn show_window_close_buttons(mut self, show_window_close_buttons: bool) -> Self {
        self.show_window_close_buttons = show_window_close_buttons;
        self
    }

    /// Enables or disables the collapsing header  on windows.
    /// By default it's `true`.
    #[inline(always)]
    pub fn show_window_collapse_buttons(mut self, show_window_collapse_buttons: bool) -> Self {
        self.show_window_collapse_buttons = show_window_collapse_buttons;
        self
    }

    /// Sets a callback that is invoked when a tab is dropped into a window destination.
    /// If this is set, the callback is responsible for handling the window semantics.
    /// If it is not set, the dock state will fall back to its built-in window behavior.
    pub fn on_tab_window(
        mut self,
        callback: impl FnMut(
                &mut DockState<Tab>,
                (SurfaceIndex, NodeIndex, TabIndex),
                Rect,
            ) + 'tree,
    ) -> Self {
        self.on_tab_window = Some(Box::new(callback));
        self
    }
}

impl<'tree, Tab> std::fmt::Debug for DockArea<'tree, Tab> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockArea").finish_non_exhaustive()
    }
}

/// Coverlay-specific dock widget.
///
/// This is intentionally a distinct widget from [`DockArea`], so the "Coverlay dock"
/// concept can be translated to other runtimes (e.g. Unity/Unreal) without reusing
/// the generic editor dock styling.
pub struct CoverlayDockArea<'tree, Tab> {
    area: DockArea<'tree, Tab>,
}

impl<'tree, Tab> CoverlayDockArea<'tree, Tab> {
    #[inline(always)]
    pub fn new(state: &'tree mut DockState<Tab>) -> Self { Self { area: DockArea::new(state) } }

    #[inline(always)]
    pub fn id(mut self, id: Id) -> Self { self.area = self.area.id(id); self }

    #[inline(always)]
    pub fn allowed_splits(mut self, allowed_splits: AllowedSplits) -> Self {
        self.area = self.area.allowed_splits(allowed_splits);
        self
    }

    /// Show coverlay dock inside an existing [`egui::Ui`].
    #[inline]
    pub fn show_inside<V: crate::widgets::tab_viewer::TabViewer<Tab = Tab>>(
        mut self,
        ui: &mut egui::Ui,
        tab_viewer: &mut V,
    ) {
        let mut style = Style::from_egui(ui.style().as_ref());
        style.dock_area_padding = Some(Margin::same(0));
        style.main_surface_border_stroke = Stroke::NONE;
        style.main_surface_border_rounding = CornerRadius::ZERO;

        // Coverlay dock: MagicaVoxel-like "in-screen panels" with tab-buttons (centered labels).
        // - Multiple tabs: show a compact tab strip.
        // - Single tab: hide the strip entirely.
        style.tab_bar.height = 24.0;
        style.tab_bar.bg_fill = Color32::TRANSPARENT;
        style.tab_bar.hline_color = Color32::TRANSPARENT;
        style.tab_bar.rounding = CornerRadius::ZERO;
        style.tab_bar.show_scroll_bar_on_overflow = false;
        style.tab_bar.fill_tab_bar = true;
        style.tab_bar.hide_single_tab = true;

        style.buttons.close_tab_bg_fill = Color32::TRANSPARENT;
        style.buttons.add_tab_bg_fill = Color32::TRANSPARENT;

        // Subtle split separators only.
        style.separator.color_idle = Color32::from_white_alpha(20);
        style.separator.color_hovered = Color32::from_white_alpha(40);
        style.separator.color_dragged = Color32::from_white_alpha(70);
        // Allow very thin coverlay side strips (e.g. MagicaVoxel palette).
        // `separator.extra` is the minimum size (in points) each side can shrink to.
        style.separator.extra = 48.0;

        // No tab body padding/borders; content is the panel.
        style.tab.hline_below_active_tab_name = false;
        style.tab.tab_body.inner_margin = Margin::same(0);
        style.tab.tab_body.stroke = Stroke::NONE;
        style.tab.tab_body.rounding = CornerRadius::ZERO;
        style.tab.tab_body.bg_fill = Color32::TRANSPARENT;

        // Tab button visuals.
        let active_bg = Color32::from_black_alpha(90);
        let inactive_bg = Color32::from_black_alpha(50);
        let hovered_bg = Color32::from_black_alpha(70);
        let text_on = Color32::from_gray(240);
        let text_off = Color32::from_gray(185);
        let rounding = CornerRadius::same(4);

        style.tab.active.bg_fill = active_bg;
        style.tab.active.text_color = text_on;
        style.tab.active.outline_color = Color32::TRANSPARENT;
        style.tab.active.rounding = rounding;

        style.tab.inactive.bg_fill = inactive_bg;
        style.tab.inactive.text_color = text_off;
        style.tab.inactive.outline_color = Color32::TRANSPARENT;
        style.tab.inactive.rounding = rounding;

        style.tab.hovered.bg_fill = hovered_bg;
        style.tab.hovered.text_color = text_on;
        style.tab.hovered.outline_color = Color32::TRANSPARENT;
        style.tab.hovered.rounding = rounding;

        style.tab.focused = style.tab.active.clone();
        style.tab.active_with_kb_focus = style.tab.active.clone();
        style.tab.inactive_with_kb_focus = style.tab.inactive.clone();
        style.tab.focused_with_kb_focus = style.tab.active.clone();

        self.area
            .style(style)
            .show_tab_name_on_hover(false)
            .show_add_buttons(false)
            .show_add_popup(false)
            .show_inside(ui, tab_viewer);
    }
}
