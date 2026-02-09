use bevy::prelude::*;
use bevy::ecs::message::MessageWriter;
use bevy_egui::{egui, EguiContexts};

use crate::{
    camera::ViewportInteractionState,
    hud,
    icons::ViewportIcons,
    layout::ViewportLayout,
    viewport_options::{CameraRotateEvent, DisplayMode, DisplayOptions, SetCameraViewEvent},
    ViewportRenderState,
};

#[derive(Resource, Default)]
pub struct ViewportUiState {
    pub show_gizmos: bool,
}

pub fn viewport_ui_system(
    mut egui_contexts: EguiContexts,
    mut display_options: ResMut<DisplayOptions>,
    mut viewport_layout: ResMut<ViewportLayout>,
    mut viewport_render_state: ResMut<ViewportRenderState>,
    mut interaction_state: ResMut<ViewportInteractionState>,
    mut ui_state: ResMut<ViewportUiState>,
    mut set_camera_view_writer: MessageWriter<SetCameraViewEvent>,
    mut camera_rotate_writer: MessageWriter<CameraRotateEvent>,
) {
    let ctx = egui_contexts.ctx_mut();
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
        let side_icon_button =
            |ui: &mut egui::Ui, id: &'static str, icon: egui::ImageSource<'static>, selected: bool| -> egui::Response {
                let size = 20.0;
                let button_size = egui::vec2(size, size);
                let (_, rect) = ui.allocate_space(button_size);
                let response = ui.interact(rect, ui.make_persistent_id(id), egui::Sense::click());
                response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, ""));
                if ui.is_rect_visible(rect) {
                    let visuals = ui.style().interact_selectable(&response, selected);
                    if selected || response.hovered() {
                        ui.painter().rect(rect.expand(visuals.expansion), visuals.rounding(), visuals.bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
                    }
                    let image_rect = egui::Rect::from_center_size(rect.center(), button_size * 0.8);
                    egui::Image::new(icon).tint(visuals.text_color()).paint_at(ui, image_rect);
                }
                response
            };
        let header_icon_button = side_icon_button;

        egui::TopBottomPanel::top("viewport_3d_header")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)).fill(ui.style().visuals.panel_fill.linear_multiply(0.8)))
            .resizable(false)
            .min_height(32.0)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("3D Viewport").strong().color(egui::Color32::from_white_alpha(100)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.separator();
                        if header_icon_button(ui, "shaded_wire", ViewportIcons::SHADED_WIRE, display_options.final_geometry_display_mode == DisplayMode::ShadedAndWireframe)
                            .on_hover_text("Shaded + Wireframe")
                            .clicked()
                        {
                            display_options.final_geometry_display_mode = DisplayMode::ShadedAndWireframe;
                        }
                        if header_icon_button(ui, "wireframe", ViewportIcons::WIREFRAME, display_options.final_geometry_display_mode == DisplayMode::Wireframe)
                            .on_hover_text("Wireframe")
                            .clicked()
                        {
                            display_options.final_geometry_display_mode = DisplayMode::Wireframe;
                        }
                        if header_icon_button(ui, "shaded", ViewportIcons::SHADED, display_options.final_geometry_display_mode == DisplayMode::Shaded)
                            .on_hover_text("Shaded")
                            .clicked()
                        {
                            display_options.final_geometry_display_mode = DisplayMode::Shaded;
                        }
                        ui.separator();
                        ui.toggle_value(&mut display_options.wireframe_ghost_mode, "Ghost").on_hover_text("Ghost Wireframe Mode: Back-facing wires are faint");
                        #[cfg(feature = "virtual_geometry_meshlet")]
                        ui.toggle_value(&mut display_options.meshlet_virtual_geometry, "Meshlets").on_hover_text("Virtual geometry (Nanite-like): GPU-driven meshlet culling; requires Vulkan/Metal and disables MSAA");
                        ui.toggle_value(&mut display_options.turntable.enabled, "Turntable").on_hover_text("Turntable mode: auto-frame and rotate (disables manual camera controls)");
                        ui.separator();
                    });
                });
            });

        egui::SidePanel::left("handle_controls_panel")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 8)))
            .resizable(false)
            .width_range(if display_options.is_handle_controls_collapsed { 12.0..=12.0 } else { 32.0..=32.0 })
            .show_inside(ui, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    if display_options.is_handle_controls_collapsed {
                        if side_icon_button(ui, "expand_controls", ViewportIcons::EXPAND, false).on_hover_text("Show Controls").clicked() {
                            display_options.is_handle_controls_collapsed = false;
                        }
                    } else {
                        if side_icon_button(ui, "collapse_controls", ViewportIcons::COLLAPSE, false).on_hover_text("Hide Controls").clicked() {
                            display_options.is_handle_controls_collapsed = true;
                        }
                        ui.separator();
                        ui.add(egui::DragValue::new(&mut display_options.camera_speed).speed(0.1).clamp_range(0.1..=100.0)).on_hover_text("Camera Speed (Drag)");
                        ui.add_enabled_ui(display_options.turntable.enabled, |ui| {
                            ui.add(egui::DragValue::new(&mut display_options.turntable.speed_deg_per_sec).speed(1.0).clamp_range(0.0..=360.0)).on_hover_text("Turntable Speed (deg/sec)");
                        });
                        ui.separator();
                        if side_icon_button(ui, "gizmo", ViewportIcons::GIZMO, ui_state.show_gizmos).on_hover_text("Show Transform Gizmo").clicked() {
                            ui_state.show_gizmos = !ui_state.show_gizmos;
                        }
                    }
                });
            });

        egui::SidePanel::right("display_options_panel")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 8)))
            .resizable(false)
            .width_range(if display_options.is_options_collapsed { 12.0..=12.0 } else { 32.0..=32.0 })
            .show_inside(ui, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    if display_options.is_options_collapsed {
                        if side_icon_button(ui, "show_options", ViewportIcons::COLLAPSE, false).on_hover_text("Show Options").clicked() {
                            display_options.is_options_collapsed = false;
                        }
                    } else {
                        if side_icon_button(ui, "hide_options", ViewportIcons::EXPAND, false).on_hover_text("Hide Options").clicked() {
                            display_options.is_options_collapsed = true;
                        }
                        ui.separator();
                        ui.vertical(|ui| {
                            if side_icon_button(ui, "grid", ViewportIcons::GRID, display_options.grid.show).on_hover_text("Show Grid").clicked() {
                                display_options.grid.show = !display_options.grid.show;
                            }
                            ui.separator();
                            if side_icon_button(ui, "points", ViewportIcons::POINTS, display_options.overlays.show_points).on_hover_text("Show Points").clicked() {
                                display_options.overlays.show_points = !display_options.overlays.show_points;
                            }
                            if side_icon_button(ui, "point_nums", ViewportIcons::POINT_NUMS, display_options.overlays.show_point_numbers).on_hover_text("Show Point Numbers").clicked() {
                                display_options.overlays.show_point_numbers = !display_options.overlays.show_point_numbers;
                            }
                            if side_icon_button(ui, "vert_nums", ViewportIcons::VERT_NUMS, display_options.overlays.show_vertex_numbers).on_hover_text("Show Vertex Numbers").clicked() {
                                display_options.overlays.show_vertex_numbers = !display_options.overlays.show_vertex_numbers;
                            }
                            if side_icon_button(ui, "vert_norms", ViewportIcons::VERT_NORMS, display_options.overlays.show_vertex_normals).on_hover_text("Show Vertex Normals").clicked() {
                                display_options.overlays.show_vertex_normals = !display_options.overlays.show_vertex_normals;
                            }
                            if side_icon_button(ui, "prim_nums", ViewportIcons::PRIM_NUMS, display_options.overlays.show_primitive_numbers).on_hover_text("Show Primitive Numbers").clicked() {
                                display_options.overlays.show_primitive_numbers = !display_options.overlays.show_primitive_numbers;
                            }
                            if side_icon_button(ui, "prim_norms", ViewportIcons::PRIM_NORMS, display_options.overlays.show_primitive_normals).on_hover_text("Show Primitive Normals").clicked() {
                                display_options.overlays.show_primitive_normals = !display_options.overlays.show_primitive_normals;
                            }
                            ui.separator();
                        });
                    }
                });
            });

        let available_size = ui.available_size();
        let (rect, response) = ui.allocate_exact_size(available_size, egui::Sense::click_and_drag());

        viewport_layout.window_entity = None;
        if viewport_layout.logical_rect != Some(rect) { viewport_layout.logical_rect = Some(rect); }

        {
            let mut s = viewport_render_state.0.lock().unwrap();
            s.viewport_size = available_size;
        }

        let was_r = interaction_state.is_right_button_dragged;
        let was_m = interaction_state.is_middle_button_dragged;
        let was_a = interaction_state.is_alt_left_button_dragged;
        let hovered = response.hovered();
        interaction_state.is_hovered = hovered;
        let right_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Secondary));
        let middle_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle));
        let alt_left_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary) && i.modifiers.alt);
        interaction_state.is_right_button_dragged = right_down && (hovered || was_r);
        interaction_state.is_middle_button_dragged = middle_down && (hovered || was_m);
        interaction_state.is_alt_left_button_dragged = alt_left_down && (hovered || was_a);

        if let Some(rect) = viewport_layout.logical_rect {
            hud::draw_viewport_hud(ui, rect, &mut *display_options, &mut set_camera_view_writer, &mut camera_rotate_writer);
        }
    });
}

