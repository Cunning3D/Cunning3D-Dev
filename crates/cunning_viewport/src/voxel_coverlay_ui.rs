//! Shared VoxelEdit coverlay UI (desktop + WASM player).
use bevy::prelude::IVec3;
use bevy::prelude::Resource;
use bevy_egui::egui;
use cunning_overlay_widgets::overlay_widgets as ow;
use cunning_kernel::algorithms::algorithms_editor::voxel::DiscreteVoxelOp;

use crate::{coverlay_dock::CoverlayPanelKey, viewport_options::DisplayOptions};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoxelToolMode { Add, Select, Move, Paint, Extrude }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoxelAddType { Point, Line, Region, Extrude, Clay, Smooth, Clone }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoxelSelectType { Point, Line, Region, Face, Rect, Color }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoxelPaintType { Point, Line, Region, Face, ColorPick, PromptStamp }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoxelBrushShape { Sphere, Cube, Cylinder, Cross, CrossWall, Diamond }

#[derive(Resource, Clone, Debug)]
pub struct VoxelToolState {
    pub mode: VoxelToolMode,
    pub add_type: VoxelAddType,
    pub select_type: VoxelSelectType,
    pub paint_type: VoxelPaintType,
    pub shape: VoxelBrushShape,
    pub clone_overwrite: bool,
    pub brush_radius: f32,
    pub palette_index: u8,
    pub sym_x: bool,
    pub sym_y: bool,
    pub sym_z: bool,
    pub spacing: f32,
}

impl Default for VoxelToolState {
    fn default() -> Self {
        Self {
            mode: VoxelToolMode::Paint,
            add_type: VoxelAddType::Point,
            select_type: VoxelSelectType::Point,
            paint_type: VoxelPaintType::Point,
            shape: VoxelBrushShape::Sphere,
            clone_overwrite: true,
            brush_radius: 0.35,
            palette_index: 1,
            sym_x: false,
            sym_y: false,
            sym_z: false,
            spacing: 0.1,
        }
    }
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct VoxelHudInfo {
    pub has_hit: bool,
    pub cell: IVec3,
    pub normal: IVec3,
    pub voxel_size: f32,
    pub has_bounds: bool,
    pub bounds_min: IVec3,
    pub bounds_max: IVec3,
    pub distance: f32,
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct VoxelOverlaySettings { pub show_volume_grid: bool, pub show_voxel_grid: bool, pub show_coordinates: bool, pub show_distance: bool, pub show_volume_bound: bool }
impl Default for VoxelOverlaySettings { fn default() -> Self { Self { show_volume_grid: false, show_voxel_grid: true, show_coordinates: false, show_distance: false, show_volume_bound: false } } }

#[derive(Clone, Debug)]
pub struct VoxelOpsPanelState { pub tab: u8, pub perlin_min: IVec3, pub perlin_max: IVec3, pub perlin_scale: f32, pub perlin_threshold: f32, pub perlin_seed: u32, pub dup_delta: IVec3 }
impl Default for VoxelOpsPanelState {
    fn default() -> Self {
        Self { tab: 0, perlin_min: IVec3::ZERO, perlin_max: IVec3::new(32, 32, 32), perlin_scale: 0.08, perlin_threshold: 0.1, perlin_seed: 1, dup_delta: IVec3::new(1, 0, 0) }
    }
}

pub trait VoxelToolsBackend {
    fn selection_cells(&self) -> &[IVec3];
    fn undo(&mut self);
    fn redo(&mut self);
    fn push_op(&mut self, op: DiscreteVoxelOp);
}

#[inline]
fn palette_color32(i: u8) -> egui::Color32 {
    if i == 0 { return egui::Color32::TRANSPARENT; }
    let h = (i as f32 * 0.618_033_988_75) % 1.0;
    let (s, v) = (0.65, 0.90);
    let h6 = h * 6.0;
    let c = v * s;
    let x = c * (1.0 - ((h6 % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h6 as i32 { 0 => (c, x, 0.0), 1 => (x, c, 0.0), 2 => (0.0, c, x), 3 => (0.0, x, c), 4 => (x, 0.0, c), _ => (c, 0.0, x) };
    egui::Color32::from_rgb(((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8)
}

pub fn draw_voxel_palette_panel(ui: &mut egui::Ui, st: &mut VoxelToolState) {
    ow::panel_frame(ui, |ui| {
        let col = palette_color32(st.palette_index);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            ui.horizontal(|ui| {
                ow::color_preview(ui, col, 28.0);
                ow::label_primary(ui, "Palette");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ow::badge(ui, format!("{}", st.palette_index).as_str(), egui::Color32::from_black_alpha(90));
                });
            });
            let bottom_h = 92.0;
            let strip_h = (ui.available_height() - bottom_h).max(40.0);
            ui.allocate_ui(egui::vec2(ui.available_width(), strip_h), |ui| {
                ow::palette_grid_fill(ui, &mut st.palette_index, palette_color32);
            });
            ow::hsep(ui);
            ow::hsv_palette_bar(ui, "palette_hsv", &mut st.palette_index, palette_color32);
        });
    });
}

pub fn draw_voxel_tools_panel(
    ui: &mut egui::Ui,
    st: &mut VoxelToolState,
    ops: &mut VoxelOpsPanelState,
    key: CoverlayPanelKey,
    backend: &mut dyn VoxelToolsBackend,
    ov: &mut VoxelOverlaySettings,
    hud: &VoxelHudInfo,
    display_options: &mut DisplayOptions,
) {
    let _ = key;
    ow::panel_frame(ui, |ui| {
        ui.horizontal(|ui| {
            ow::toolbar(ui, false, |ui| {
                ow::icon_select(ui, &mut st.mode, VoxelToolMode::Add, "add", "Tool: Add (1)");
                ow::icon_select(ui, &mut st.mode, VoxelToolMode::Select, "select", "Tool: Select (2)");
                ow::icon_select(ui, &mut st.mode, VoxelToolMode::Move, "move", "Tool: Move (3)");
                ow::icon_select(ui, &mut st.mode, VoxelToolMode::Paint, "brush", "Tool: Paint (4)");
                ow::icon_select(ui, &mut st.mode, VoxelToolMode::Extrude, "extrude", "Tool: Extrude (5)");
            });
        });
        ow::hsep(ui);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ow::group(ui, "Actions", true, |ui| {
                ow::toolbar(ui, false, |ui| {
                    if ow::icon_button(ui, "undo", false).on_hover_text("Undo (Z)").clicked() { backend.undo(); }
                    if ow::icon_button(ui, "redo", false).on_hover_text("Redo (Y)").clicked() { backend.redo(); }
                    if ow::icon_button(ui, "delete", false).on_hover_text("Clear all voxels").clicked() { backend.push_op(DiscreteVoxelOp::ClearAll); }
                    if ow::icon_button(ui, "trim", false).on_hover_text("Trim bounds to origin").clicked() { backend.push_op(DiscreteVoxelOp::TrimToOrigin); }
                });
            });

            ow::group(ui, "Mode", true, |ui| {
                match st.mode {
                    VoxelToolMode::Add => {
                        ow::toolbar(ui, false, |ui| {
                            ow::icon_select(ui, &mut st.add_type, VoxelAddType::Point, "cube", "Add: Point");
                            ow::icon_select(ui, &mut st.add_type, VoxelAddType::Line, "region", "Add: Line");
                            ow::icon_select(ui, &mut st.add_type, VoxelAddType::Region, "region", "Add: Region");
                            ow::icon_select(ui, &mut st.add_type, VoxelAddType::Extrude, "extrude", "Add: Extrude");
                            ow::icon_select(ui, &mut st.add_type, VoxelAddType::Clay, "sphere", "Add: Clay");
                            ow::icon_select(ui, &mut st.add_type, VoxelAddType::Smooth, "stamp", "Add: Smooth");
                            ow::icon_select(ui, &mut st.add_type, VoxelAddType::Clone, "select", "Add: Clone");
                        });
                        ow::toggle_button(ui, &mut st.clone_overwrite, "Overwrite");
                    }
                    VoxelToolMode::Select => {
                        ow::toolbar(ui, false, |ui| {
                            ow::icon_select(ui, &mut st.select_type, VoxelSelectType::Point, "cube", "Select: Point");
                            ow::icon_select(ui, &mut st.select_type, VoxelSelectType::Line, "region", "Select: Line");
                            ow::icon_select(ui, &mut st.select_type, VoxelSelectType::Region, "region", "Select: Region");
                            ow::icon_select(ui, &mut st.select_type, VoxelSelectType::Face, "cube", "Select: Face");
                            ow::icon_select(ui, &mut st.select_type, VoxelSelectType::Rect, "select", "Select: Rect");
                            ow::icon_select(ui, &mut st.select_type, VoxelSelectType::Color, "picker", "Select: Color");
                        });
                    }
                    VoxelToolMode::Paint => {
                        ow::toolbar(ui, false, |ui| {
                            ow::icon_select(ui, &mut st.paint_type, VoxelPaintType::Point, "cube", "Paint: Point");
                            ow::icon_select(ui, &mut st.paint_type, VoxelPaintType::Line, "region", "Paint: Line");
                            ow::icon_select(ui, &mut st.paint_type, VoxelPaintType::Region, "region", "Paint: Region");
                            ow::icon_select(ui, &mut st.paint_type, VoxelPaintType::Face, "cube", "Paint: Face");
                            ow::icon_select(ui, &mut st.paint_type, VoxelPaintType::ColorPick, "picker", "Paint: Pick");
                            ow::icon_select(ui, &mut st.paint_type, VoxelPaintType::PromptStamp, "stamp", "Paint: AI Stamp");
                        });
                    }
                    _ => {}
                }
            });

            ow::group(ui, "Brush", true, |ui| {
                ow::toolbar(ui, false, |ui| {
                    ow::icon_select(ui, &mut st.shape, VoxelBrushShape::Sphere, "sphere", "Shape: Sphere");
                    ow::icon_select(ui, &mut st.shape, VoxelBrushShape::Cube, "cube", "Shape: Cube");
                    ow::icon_select(ui, &mut st.shape, VoxelBrushShape::Cylinder, "cylinder", "Shape: Cylinder");
                    ow::icon_select(ui, &mut st.shape, VoxelBrushShape::Cross, "cross", "Shape: Cross");
                    ow::icon_select(ui, &mut st.shape, VoxelBrushShape::CrossWall, "cross", "Shape: Cross Wall");
                    ow::icon_select(ui, &mut st.shape, VoxelBrushShape::Diamond, "sphere", "Shape: Diamond");
                });
                ow::axis_toggle(ui, &mut st.sym_x, &mut st.sym_y, &mut st.sym_z);
                ow::styled_slider(ui, &mut st.brush_radius, 0.05..=5.0, "Radius");
            });

            ow::group(ui, "Generate", true, |ui| {
                ow::segmented_tabs(ui, ("voxel_ops_tabs", key), &mut ops.tab, &["Perlin", "Duplicate"]);
                ow::hsep(ui);
                match ops.tab {
                    0 => {
                        egui::Grid::new(ui.make_persistent_id(("perlin_grid", key))).num_columns(4).spacing(egui::vec2(6.0, 4.0)).show(ui, |ui| {
                            ui.label("Min"); ui.add(egui::DragValue::new(&mut ops.perlin_min.x)); ui.add(egui::DragValue::new(&mut ops.perlin_min.y)); ui.add(egui::DragValue::new(&mut ops.perlin_min.z)); ui.end_row();
                            ui.label("Max"); ui.add(egui::DragValue::new(&mut ops.perlin_max.x)); ui.add(egui::DragValue::new(&mut ops.perlin_max.y)); ui.add(egui::DragValue::new(&mut ops.perlin_max.z)); ui.end_row();
                            ui.label("Scale"); ui.add(egui::DragValue::new(&mut ops.perlin_scale).speed(0.01)); ui.label("Thr"); ui.add(egui::DragValue::new(&mut ops.perlin_threshold).speed(0.01)); ui.end_row();
                            ui.label("Seed"); ui.add(egui::DragValue::new(&mut ops.perlin_seed)); ui.label("Pal"); ui.add(egui::DragValue::new(&mut st.palette_index).range(1..=255)); ui.end_row();
                        });
                        if ow::action_button(ui, "Apply").on_hover_text("Apply Perlin").clicked() {
                            let mn = ops.perlin_min.min(ops.perlin_max);
                            let mx = ops.perlin_min.max(ops.perlin_max);
                            backend.push_op(DiscreteVoxelOp::PerlinFill { min: mn, max: mx, scale: ops.perlin_scale, threshold: ops.perlin_threshold, palette_index: st.palette_index, seed: ops.perlin_seed });
                        }
                    }
                    _ => {
                        ui.horizontal(|ui| { ui.label("Delta"); ui.add(egui::DragValue::new(&mut ops.dup_delta.x)); ui.add(egui::DragValue::new(&mut ops.dup_delta.y)); ui.add(egui::DragValue::new(&mut ops.dup_delta.z)); });
                        ow::toggle_button(ui, &mut st.clone_overwrite, "Overwrite");
                        let cells: Vec<IVec3> = backend.selection_cells().to_vec();
                        let has = !cells.is_empty();
                        if ow::action_button(ui, "Duplicate").clicked() && has {
                            backend.push_op(DiscreteVoxelOp::CloneSelected { cells, delta: ops.dup_delta, overwrite: st.clone_overwrite });
                        }
                        if !has { ui.label("Tip: Select cells (Select tool) to duplicate."); }
                    }
                }
            });

            ow::group(ui, "Overlays", false, |ui| {
                ow::toolbar(ui, false, |ui| {
                    ow::toggle_button(ui, &mut ov.show_volume_grid, "Volume");
                    ow::toggle_button(ui, &mut ov.show_voxel_grid, "Voxel");
                    ow::toggle_button(ui, &mut ov.show_volume_bound, "Bounds");
                });
                ow::styled_slider(ui, &mut display_options.overlays.voxel_grid_line_px, 0.0..=4.0, "Grid px");
                ow::toolbar(ui, false, |ui| {
                    ow::toggle_button(ui, &mut ov.show_coordinates, "Coord");
                    ow::toggle_button(ui, &mut ov.show_distance, "Dist");
                });
            });

            ow::group(ui, "Raycast", false, |ui| {
                ow::info_row(ui, "Hit", if hud.has_hit { "Yes" } else { "No" });
                ow::info_row(ui, "Cell", &format!("{:?}", hud.cell));
                ow::info_row(ui, "Normal", &format!("{:?}", hud.normal));
                if ov.show_distance { ow::info_row(ui, "Distance", &format!("{:.3}", hud.distance)); }
                if ov.show_volume_bound && hud.has_bounds {
                    ow::info_row(ui, "Min", &format!("{:?}", hud.bounds_min));
                    ow::info_row(ui, "Max", &format!("{:?}", hud.bounds_max));
                }
            });
        });
    });
}

