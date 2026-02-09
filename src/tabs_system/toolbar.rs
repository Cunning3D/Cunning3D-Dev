//! `toolbar.rs` - A dedicated UI component for the fixed toolbar.

use bevy_egui::egui;
use egui_dock::{DockArea, TabViewer};
use egui_wgpu::sdf::GpuTextUniform;

use crate::{
    gpu_text,
    ui::{
        create_attribute_promote_node, create_cube_node, create_merge_node, create_sphere_node,
        create_transform_node, ShelfCommand, ShelfTab,
    },
    NodeGraphResource, UiState,
};

#[derive(Default)]
pub struct Toolbar;

/// Helper for drawing Houdini-style "Ghost Buttons"
fn shelf_tool_btn(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let size = egui::vec2(54.0, 54.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

    // Hover/Active feedback
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect, 4.0, egui::Color32::from_white_alpha(25));
        ui.painter().rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(50)),
            egui::StrokeKind::Inside,
        );
    }

    // Icon (Mock) - Draw first letter big or use specific icon logic
    let center_top = rect.center() - egui::vec2(0.0, 10.0);
    // Change icon to String to avoid borrow checker issues with temporary values
    let icon = match label {
        "Cube" | "Box" => "🧊".to_string(),
        "Sphere" => "⚪".to_string(),
        "Transform" => "🔄".to_string(),
        "Merge" => "🔗".to_string(),
        "Grid" => "▦".to_string(),
        "Torus" => "🍩".to_string(),
        "Bend" => "↪️".to_string(),
        "Twist" => "🌪️".to_string(),
        "Bone" => "🦴".to_string(),
        _ => label
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".to_string()),
    };

    {
        let painter = ui.painter();
        let pos = center_top;
        let anchor = egui::Align2::CENTER_CENTER;
        let font_px = 24.0;
        let color = egui::Color32::WHITE;
        let text = icon.clone();
        let galley = ui.fonts_mut(|f| {
            f.layout_no_wrap(text.clone(), egui::FontId::proportional(font_px), color)
        });
        let r = anchor.anchor_size(pos, galley.size());
        let frame_id = (ui.input(|i| i.time) * 1000.0) as u64;
        gpu_text::paint(
            painter,
            GpuTextUniform {
                text,
                pos: r.min,
                color,
                font_px,
                bounds: r.size(),
                family: 0,
            },
            frame_id,
        );
    }

    // Label
    let center_bottom = rect.center() + egui::vec2(0.0, 14.0);
    {
        let painter = ui.painter();
        let pos = center_bottom;
        let anchor = egui::Align2::CENTER_CENTER;
        let font_px = 10.0;
        let color = if response.hovered() {
            egui::Color32::WHITE
        } else {
            egui::Color32::LIGHT_GRAY
        };
        let text = label.to_string();
        let galley = ui.fonts_mut(|f| {
            f.layout_no_wrap(text.clone(), egui::FontId::proportional(font_px), color)
        });
        let r = anchor.anchor_size(pos, galley.size());
        let frame_id = (ui.input(|i| i.time) * 1000.0) as u64;
        gpu_text::paint(
            painter,
            GpuTextUniform {
                text,
                pos: r.min,
                color,
                font_px,
                bounds: r.size(),
                family: 0,
            },
            frame_id,
        );
    }

    response
}

struct ShelfTabViewer<'a> {
    ui_state: &'a mut UiState,
    node_graph_res: &'a mut NodeGraphResource,
    node_editor_settings: &'a crate::node_editor_settings::NodeEditorSettings,
}

impl<'a> TabViewer for ShelfTabViewer<'a> {
    type Tab = ShelfTab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            ShelfTab::Empty => "New Set".into(),
            _ => format!("{:?}", tab).into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        ui.horizontal(|ui| {
            ui.style_mut().spacing.item_spacing = egui::vec2(2.0, 0.0);
            match tab {
                ShelfTab::Empty => {
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new("Click + to add tools").weak().italics());
                    });
                }
                ShelfTab::Create => {
                    if shelf_tool_btn(ui, "Cube").clicked() {
                        create_cube_node(
                            self.ui_state,
                            self.node_graph_res,
                            self.node_editor_settings,
                            egui::Pos2::ZERO,
                        );
                    }
                    if shelf_tool_btn(ui, "Sphere").clicked() {
                        create_sphere_node(
                            self.ui_state,
                            self.node_graph_res,
                            self.node_editor_settings,
                            egui::Pos2::ZERO,
                        );
                    }
                    if shelf_tool_btn(ui, "Transform").clicked() {
                        create_transform_node(
                            self.ui_state,
                            self.node_graph_res,
                            self.node_editor_settings,
                            egui::Pos2::ZERO,
                        );
                    }
                    if shelf_tool_btn(ui, "Promote").clicked() {
                        // Attribute Promote
                        create_attribute_promote_node(
                            self.ui_state,
                            self.node_graph_res,
                            self.node_editor_settings,
                            egui::Pos2::ZERO,
                        );
                    }
                    if shelf_tool_btn(ui, "Merge").clicked() {
                        create_merge_node(
                            self.ui_state,
                            self.node_graph_res,
                            self.node_editor_settings,
                            egui::Pos2::ZERO,
                        );
                    }

                    // Mock Buttons
                    let _ = shelf_tool_btn(ui, "Grid");
                    let _ = shelf_tool_btn(ui, "Torus");
                    let _ = shelf_tool_btn(ui, "Tube");
                }
                ShelfTab::Modify => {
                    let _ = shelf_tool_btn(ui, "Edit");
                    let _ = shelf_tool_btn(ui, "Clip");
                    let _ = shelf_tool_btn(ui, "Extrude"); // PolyExtrude
                    let _ = shelf_tool_btn(ui, "Facet");
                }
                ShelfTab::Deform => {
                    let _ = shelf_tool_btn(ui, "Bend");
                    let _ = shelf_tool_btn(ui, "Twist");
                    let _ = shelf_tool_btn(ui, "Noise");
                }
                ShelfTab::Rigging => {
                    let _ = shelf_tool_btn(ui, "Bone");
                    let _ = shelf_tool_btn(ui, "IK");
                    let _ = shelf_tool_btn(ui, "Weight");
                }
                _ => {
                    ui.label("Work in progress");
                }
            }
        });
    }

    fn closeable(&mut self, _tab: &mut Self::Tab) -> bool {
        false
    }

    fn allowed_in_windows(&self, _tab: &mut Self::Tab) -> bool {
        false
    }

    fn add_popup(
        &mut self,
        ui: &mut egui::Ui,
        surface: egui_dock::SurfaceIndex,
        node: egui_dock::NodeIndex,
    ) {
        ui.set_min_width(150.0);
        ui.style_mut().visuals.button_frame = false;

        // 1. New Shelf Set Action
        if ui.button("➕ New Shelf Set").clicked() {
            self.ui_state
                .shelf_command_queue
                .push(ShelfCommand::NewSet(surface, node));
            ui.close_menu();
        }

        ui.separator();
        ui.label(egui::RichText::new("Shelf Tabs").strong());

        let all_tabs = vec![
            ShelfTab::Create,
            ShelfTab::Modify,
            ShelfTab::Model,
            ShelfTab::Polygon,
            ShelfTab::Deform,
            ShelfTab::Texture,
            ShelfTab::Rigging,
        ];

        // Check which tabs are present in this node using our cache
        let present_tabs = self.ui_state.shelf_tab_cache.get(&(surface, node));

        for tab in all_tabs {
            let is_present = present_tabs.map(|s| s.contains(&tab)).unwrap_or(false);
            let mut checked = is_present;

            // Use checkbox for toggling
            if ui.checkbox(&mut checked, format!("{:?}", tab)).clicked() {
                self.ui_state
                    .shelf_command_queue
                    .push(ShelfCommand::Toggle(tab, surface, node));
                // Keep menu open for multiple selections
            }
        }
    }
}

impl Toolbar {
    /// Renders the fixed toolbar below the main menu bar.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        ui_state: &mut UiState,
        node_graph_res: &mut NodeGraphResource,
        node_editor_settings: &crate::node_editor_settings::NodeEditorSettings,
    ) {
        // Update Tab Cache BEFORE swapping state
        // This allows add_popup to know what's inside each node
        ui_state.shelf_tab_cache.clear();
        // Iterate over all surfaces and nodes (simplified for main surface, but good to be generic)
        for (surface_idx, surface) in ui_state.shelf_dock_state.iter_surfaces().enumerate() {
            let surface_index = egui_dock::SurfaceIndex(surface_idx);
            for (node_idx, node) in surface.iter_nodes().enumerate() {
                let node_index = egui_dock::NodeIndex(node_idx);
                if let Some(tabs) = node.tabs() {
                    let tab_set: std::collections::HashSet<ShelfTab> =
                        tabs.iter().cloned().collect();
                    ui_state
                        .shelf_tab_cache
                        .insert((surface_index, node_index), tab_set);
                }
            }
        }

        // Temporarily take ownership of shelf_dock_state to avoid borrow conflicts
        // when passing ui_state to ShelfTabViewer.
        let mut shelf_dock_state = egui_dock::DockState::new(vec![]);
        std::mem::swap(&mut ui_state.shelf_dock_state, &mut shelf_dock_state);

        let desired_height = 80.0; // Increase height for larger icon buttons
        egui::TopBottomPanel::top("main_toolbar")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(0)))
            .height_range(desired_height..=desired_height)
            .show(ctx, |ui| {
                let key = (shelf_dock_state.iter_all_tabs().count() as u64)
                    ^ ((ui.max_rect().width().to_bits() as u64) << 1)
                    ^ ((ui_state.shelf_command_queue.len() as u64) << 2);
                ui.push_id(("main_toolbar", key), |ui| {
                    // --- Houdini Gradient Background ---
                    let rect = ui.max_rect();
                    let painter = ui.painter();

                    let color_top = egui::Color32::from_gray(45);
                    let color_bottom = egui::Color32::from_gray(75);

                    use egui::epaint::{Mesh, Vertex};
                    let mut mesh = Mesh::default();

                    mesh.vertices.push(Vertex {
                        pos: rect.left_top(),
                        uv: egui::Pos2::ZERO,
                        color: color_top,
                    });
                    mesh.vertices.push(Vertex {
                        pos: rect.right_top(),
                        uv: egui::Pos2::ZERO,
                        color: color_top,
                    });
                    mesh.vertices.push(Vertex {
                        pos: rect.right_bottom(),
                        uv: egui::Pos2::ZERO,
                        color: color_bottom,
                    });
                    mesh.vertices.push(Vertex {
                        pos: rect.left_bottom(),
                        uv: egui::Pos2::ZERO,
                        color: color_bottom,
                    });

                    mesh.add_triangle(0, 1, 2);
                    mesh.add_triangle(0, 2, 3);

                    painter.add(mesh);

                    // --- Shelf Dock Area ---
                    let mut tab_viewer = ShelfTabViewer {
                        ui_state,
                        node_graph_res,
                        node_editor_settings,
                    };

                    let mut style = egui_dock::Style::from_egui(ui.style().as_ref());
                    style.separator.width = 4.0; // Slightly wider separator for split view
                    style.tab_bar.height = 24.0;

                    // Style Tweaks
                    style.tab.active.rounding = egui::CornerRadius::ZERO;
                    style.tab.inactive.rounding = egui::CornerRadius::ZERO;
                    style.tab.focused.rounding = egui::CornerRadius::ZERO;
                    style.tab.hovered.rounding = egui::CornerRadius::ZERO;
                    style.tab.tab_body.rounding = egui::CornerRadius::ZERO;

                    style.tab_bar.bg_fill = egui::Color32::TRANSPARENT;

                    // Make tabs transparent to let our leaf.rs custom painting take over
                    style.tab.active.bg_fill = egui::Color32::TRANSPARENT;
                    style.tab.inactive.bg_fill = egui::Color32::TRANSPARENT;
                    style.tab.active.text_color = egui::Color32::WHITE; // Bright text for active
                    style.tab.inactive.text_color = egui::Color32::from_gray(160); // Dim text for inactive

                    DockArea::new(&mut shelf_dock_state)
                        .id(egui::Id::new("shelf_dock_area")) // Unique ID for Shelf
                        .style(style)
                        .show_add_buttons(true)
                        .show_add_popup(true)
                        .show_inside(ui, &mut tab_viewer);
                });
            });

        // Swap back the dock state
        std::mem::swap(&mut ui_state.shelf_dock_state, &mut shelf_dock_state);

        // Process Command Queue
        let commands: Vec<_> = ui_state.shelf_command_queue.drain(..).collect();
        for cmd in commands {
            match cmd {
                ShelfCommand::Add(tab, surface, node) => {
                    if let egui_dock::Node::Leaf { tabs, .. } =
                        &mut ui_state.shelf_dock_state[surface][node]
                    {
                        tabs.push(tab);
                    }
                }
                ShelfCommand::Remove(surface, node, index) => {
                    ui_state.shelf_dock_state.remove_tab((surface, node, index));
                }
                ShelfCommand::Toggle(tab, surface, node) => {
                    if let egui_dock::Node::Leaf { tabs, .. } =
                        &mut ui_state.shelf_dock_state[surface][node]
                    {
                        if let Some(idx) = tabs.iter().position(|t| *t == tab) {
                            tabs.remove(idx);
                        } else {
                            tabs.push(tab);
                        }
                    }
                }
                ShelfCommand::NewSet(surface, node) => {
                    // Split the current node to the right to create a new "Shelf Set"
                    // We add a default tab (e.g., Create) so the new set isn't empty/invisible
                    let _ = ui_state.shelf_dock_state[surface].split_right(
                        node,
                        0.5,
                        vec![ShelfTab::Create],
                    );
                }
            }
        }
    }
}
