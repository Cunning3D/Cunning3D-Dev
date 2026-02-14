use bevy::prelude::{Entity, Resource};
use bevy_egui::{
    egui::{self, Sense},
    EguiContexts,
};

use crate::{
    tabs_system::EditorTabContext,
    viewport_options::{DisplayMode, OpenNaiveWindowEvent},
};
use crate::coverlay_bevy_ui::{coverlay_collect_panels, CoverlayDockTab};
use cunning_viewport::coverlay_dock::{
    apply_palette_ratio_once, build_default_viewport_dock_from_preset, clamp_viewport_dock_fractions,
    coverlay_strip_runtime, preset_palette_ratio, CoverlayDockPanel, CoverlayPanelKey,
    ViewportDockTab, VIEWPORT_DOCK_LAYOUT_KEY, VIEWPORT_DOCK_PRESET_KEY, VIEWPORT_KEEP_X,
    VIEWPORT_KEEP_Y,
};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer as DockTabViewer};
use egui_wgpu::sdf::{create_gpu_text_callback, GpuTextUniform};

use super::EditorTab;

pub mod camera_sync;
pub mod gizmo_systems;
pub mod grid; // [NEW] Add grid module
pub mod group_highlight;
pub mod hud;
pub mod icons; // [NEW] Add icons module // [NEW] Add group highlight module

use icons::ViewportIcons; // [NEW] Import icons

use crate::viewport_options::{ViewportViewMode, ViewportLightingMode};
use bevy::prelude::*;

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct UvBoundaryGizmos;

pub fn draw_uv_grid_system(
    display_options: Res<crate::viewport_options::DisplayOptions>,
    mut gizmos: Gizmos<UvBoundaryGizmos>,
    camera_query: Query<(&Camera, &GlobalTransform), With<crate::MainCamera>>,
    viewport_layout: Res<ViewportLayout>,
    mut egui_contexts: EguiContexts,
    node_graph_res: Res<crate::nodes::NodeGraphResource>,
    ui_state: Res<crate::ui::UiState>,
) {
    if !matches!(display_options.view_mode, ViewportViewMode::UV | ViewportViewMode::NodeImage) {
        return;
    }

    let Ok((camera, transform)) = camera_query.single() else {
        return;
    };

    // Calculate visible UV bounds
    let ndc_to_world = transform.to_matrix() * camera.clip_from_view().inverse();
    let points = [
        ndc_to_world.project_point3(Vec3::new(-1.0, -1.0, 0.0)),
        ndc_to_world.project_point3(Vec3::new(1.0, 1.0, 0.0)),
    ];

    let min_x = points[0].x.min(points[1].x).floor() as i32;
    let max_x = points[0].x.max(points[1].x).ceil() as i32;
    let min_y = points[0].y.min(points[1].y).floor() as i32;
    let max_y = points[0].y.max(points[1].y).ceil() as i32;

    // Expand slightly to fill screen, but protect against overflow
    let start_x = min_x.saturating_sub(1).max(-1000);
    let end_x = max_x.saturating_add(1).min(2000);
    let start_y = min_y.saturating_sub(1).max(-1000);
    let end_y = max_y.saturating_add(1).min(2000);

    let z_offset = -0.01;
    let grid_color = Color::srgba(0.3, 0.3, 0.3, 0.3);
    let tile_border_color = Color::srgba(0.6, 0.6, 0.6, 0.5);

    // 1. Draw Tile Borders (Integers)
    for x in start_x..=end_x {
        let color = if x == 0 {
            Color::srgb(0.0, 1.0, 0.0)
        } else {
            tile_border_color
        }; // Y-axis is Green (V)
        gizmos.line(
            Vec3::new(x as f32, start_y as f32, z_offset),
            Vec3::new(x as f32, end_y as f32, z_offset),
            color,
        );
    }
    for y in start_y..=end_y {
        let color = if y == 0 {
            Color::srgb(1.0, 0.0, 0.0)
        } else {
            tile_border_color
        }; // X-axis is Red (U)
        gizmos.line(
            Vec3::new(start_x as f32, y as f32, z_offset),
            Vec3::new(end_x as f32, y as f32, z_offset),
            color,
        );
    }

    // 2. Draw Sub-grid (0.1) if not too zoomed out
    let zoom_level = (max_x.saturating_sub(min_x)).max(max_y.saturating_sub(min_y));
    if zoom_level < 8 {
        for x in start_x..end_x {
            for i in 1..10 {
                let t = x as f32 + i as f32 * 0.1;
                gizmos.line(
                    Vec3::new(t, start_y as f32, z_offset),
                    Vec3::new(t, end_y as f32, z_offset),
                    grid_color,
                );
            }
        }
        for y in start_y..end_y {
            for i in 1..10 {
                let t = y as f32 + i as f32 * 0.1;
                gizmos.line(
                    Vec3::new(start_x as f32, t, z_offset),
                    Vec3::new(end_x as f32, t, z_offset),
                    grid_color,
                );
            }
        }
    }

    // 3. Draw UDIM Labels
    if let Some(rect) = viewport_layout.logical_rect {
        // Find correct Egui Context
        // Assuming single window or primary for now, as full multi-window EguiContexts support is complex here
        // without knowing exactly which window entity corresponds to the context.
        // But we have viewport_layout.window_entity!

        let ctx = if let Some(window_entity) = viewport_layout.window_entity {
            // Use try_ctx_for_window_mut to avoid panic if context is not initialized (e.g. during startup or window close)
            if let Some(ctx) = egui_contexts.try_ctx_for_window_mut(window_entity) {
                ctx
            } else {
                return;
            }
        } else {
            egui_contexts.ctx_mut()
        };
        // Ensure per-window egui contexts have image loaders (URI/file) installed.
        egui_extras::install_image_loaders(ctx);

        let mut painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("udim_labels"),
        ));
        painter.set_clip_rect(rect);

        // Check if camera viewport is set (it should be)
        if camera.viewport.is_some() {
            // Camera viewport is in physical pixels usually, but logical_viewport_rect is available too
            // camera.logical_viewport_rect() returns Rect<f32>
            // But we rely on `world_to_viewport` which handles projection to the specific viewport.

            let frame_id = (ctx.input(|i| i.time) * 1000.0) as u64;

            // 0..1 tile image overlay for selected node (if it has an image path).
            {
                let selected = ui_state
                    .last_selected_node_id
                    .or_else(|| ui_state.selected_nodes.iter().next().copied());
                let rel = selected.and_then(|id| {
                    let g = &node_graph_res.0;
                    g.nodes.get(&id).and_then(|n| {
                        let get_s = |name: &str| {
                            n.parameters.iter().find(|p| p.name == name).and_then(|p| {
                                if let crate::nodes::parameter::ParameterValue::String(s) = &p.value {
                                    Some(s.trim().to_string())
                                } else {
                                    None
                                }
                            })
                            .filter(|s| !s.is_empty())
                        };
                        get_s(crate::nodes::ai_texture::PARAM_IMAGE_PATH).or_else(|| {
                            let is_baker = matches!(&n.node_type, crate::nodes::NodeType::Generic(s) if s == crate::nodes::ai_texture::NODE_NANO_HEXPLANAR_BAKER);
                            if is_baker {
                                get_s(crate::nodes::ai_texture::PARAM_BASE_UV_FIXED)
                                    .or_else(|| get_s(crate::nodes::ai_texture::PARAM_BASE_UV))
                            } else {
                                None
                            }
                        })
                    })
                    .or_else(|| {
                        g.geometry_cache
                            .get(&id)
                            .and_then(|geo| {
                                geo.get_detail_attribute(crate::nodes::ai_texture::ATTR_HEIGHTMAP_PATH)
                                    .and_then(|a| a.as_slice::<String>())
                                    .and_then(|v| v.first())
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                            })
                    })
                });

                let uri = rel
                    .and_then(|rel| {
                        let rel = rel.trim().to_string();
                        let cwd = std::env::current_dir().ok()?;
                        let abs = std::path::PathBuf::from(&rel);
                        let abs = if abs.is_absolute() { abs } else { cwd.join("assets").join(&rel) };
                        Some((rel, abs))
                    })
                    .and_then(|(rel, abs)| {
                        let exists = abs.exists();
                        if !exists {
                            warn!("NodeImage: missing image file. rel='{}' abs='{}'", rel, abs.display());
                            return None;
                        }
                        let abs = abs.canonicalize().unwrap_or(abs);
                        let mut p = abs.to_string_lossy().to_string();
                        if let Some(s) = p.strip_prefix(r"\\?\") { p = s.to_string(); }
                        p = p.replace('\\', "/");
                        if let Some(s) = p.strip_prefix("//?/") { p = s.to_string(); }
                        Some(format!("file:///{}", p))
                    });

                if let Some(uri) = uri {
                    let bl = Vec3::new(0.0, 0.0, 0.0);
                    let tr = Vec3::new(1.0, 1.0, 0.0);
                    if let (Ok(v0), Ok(v1)) = (camera.world_to_viewport(transform, bl), camera.world_to_viewport(transform, tr)) {
                        if let Some(logical_size) = camera.logical_target_size() {
                            let p0 = egui::pos2(v0.x, logical_size.y - v0.y);
                            let p1 = egui::pos2(v1.x, logical_size.y - v1.y);
                            let r = egui::Rect::from_min_max(
                                egui::pos2(p0.x.min(p1.x), p0.y.min(p1.y)),
                                egui::pos2(p0.x.max(p1.x), p0.y.max(p1.y)),
                            );
                            let r = r.intersect(rect);
                            if r.width() > 2.0 && r.height() > 2.0 {
                                // Paint via a tiny overlay UI so Image loaders work.
                                egui::Area::new(egui::Id::new("c3d_selected_node_image_overlay"))
                                    .order(egui::Order::Foreground)
                                    .fixed_pos(r.left_top())
                                    .show(ctx, |ui| {
                                        ui.set_min_size(r.size());
                                        ui.set_max_size(r.size());
                                        let rr = ui.max_rect().intersect(rect);
                                        if rr.width() <= 2.0 || rr.height() <= 2.0 {
                                            return;
                                        }
                                        ui.painter().rect_filled(
                                            rr,
                                            egui::CornerRadius::same(2),
                                            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 140),
                                        );
                                        egui::Image::new(uri)
                                            .fit_to_exact_size(rr.size())
                                            .paint_at(ui, rr);
                                        ui.painter().add(create_gpu_text_callback(
                                            ui.painter().clip_rect(),
                                            GpuTextUniform {
                                                text: "Selected Node Image".to_string(),
                                                pos: (rr.left_top() + egui::vec2(6.0, 6.0)),
                                                color: egui::Color32::from_white_alpha(180),
                                                font_px: 14.0,
                                                bounds: egui::vec2(rr.width().max(1.0), 20.0),
                                                family: 0,
                                            },
                                            frame_id,
                                        ));
                                    });
                            }
                        }
                    }
                }
            }

            // Iterate visible tiles for labels
            for x in start_x..end_x {
                for y in start_y..end_y {
                    if x >= 0 && x <= 9 && y >= 0 {
                        let udim = 1001 + x + (y * 10);
                        let world_pos = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, 0.0);

                        if let Ok(viewport_pos) = camera.world_to_viewport(transform, world_pos) {
                            // Bevy Viewport (0,0 bottom-left) -> Egui (0,0 top-left)
                            // However, if the viewport is offset (e.g. in a window), viewport_pos is relative to the window.
                            // And egui coordinates are also relative to the window.
                            // But Bevy Y is up, Egui Y is down.

                            if let Some(logical_size) = camera.logical_target_size() {
                                let egui_pos =
                                    egui::pos2(viewport_pos.x, logical_size.y - viewport_pos.y);

                                // Draw only if inside rect
                                if rect.contains(egui_pos) {
                                    let anchor = egui::Align2::CENTER_CENTER;
                                    let font_px = 20.0;
                                    let color = egui::Color32::from_white_alpha(128);
                                    let text = udim.to_string();
                                    let galley = painter.ctx().fonts_mut(|f| {
                                        f.layout_no_wrap(
                                            text.clone(),
                                            egui::FontId::proportional(font_px),
                                            color,
                                        )
                                    });
                                    let r = anchor.anchor_size(egui_pos, galley.size());
                                    painter.add(create_gpu_text_callback(
                                        painter.clip_rect(),
                                        GpuTextUniform {
                                            text,
                                            pos: r.min,
                                            color,
                                            font_px,
                                            bounds: r.size(),
                                            family: 0,
                                        },
                                        frame_id,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Resource, Default, Clone)]
pub struct ViewportLayout {
    pub window_entity: Option<Entity>,
    pub logical_rect: Option<egui::Rect>,
}

pub struct Viewport3DTab {
    pub viewport_size: Option<egui::Vec2>,
    pub viewport_rect: Option<egui::Rect>,
    pub show_gizmos: bool,
    coverlay_dock: CoverlayDockTab,
    dock_state: DockState<ViewportDockTab>,
    dock_sig: u64,
    dock_owner: Option<crate::nodes::NodeId>,
}

impl Default for Viewport3DTab {
    fn default() -> Self {
        let mut dock_state = DockState::new(vec![ViewportDockTab::Viewport]);
        Self {
            viewport_size: None,
            viewport_rect: None,
            show_gizmos: true,
            coverlay_dock: CoverlayDockTab::default(),
            dock_state,
            dock_sig: 0,
            dock_owner: None,
        }
    }
}

#[inline]
#[allow(dead_code)]
fn mix_rect_u64(mut h: u64, r: egui::Rect) -> u64 {
    h ^= r.min.x.to_bits() as u64;
    h = h.rotate_left(11) ^ r.min.y.to_bits() as u64;
    h = h.rotate_left(11) ^ r.max.x.to_bits() as u64;
    h = h.rotate_left(11) ^ r.max.y.to_bits() as u64;
    h
}

impl EditorTab for Viewport3DTab {
    fn title(&self) -> egui::WidgetText {
        "3D Viewport".into()
    }

    // Interactive viewport chrome (edge bars) must be immediate-mode, otherwise retained caching
    // can cause hover/click to "miss" due to stale widget snapshots.
    fn is_immediate(&self) -> bool {
        true
    }

    fn ui(&mut self, ui: &mut egui::Ui, context: &mut EditorTabContext) {
        context.coverlay_wants_input.0 = false;

        // Render viewport outer chrome (header + sidebars) FIRST, OUTSIDE coverlay dock area.
        self.render_viewport_chrome(ui, context);

        // Sync desired tabs for current selection: Viewport + each coverlay panel as its own dock tab.
        let (owner, panels) = coverlay_collect_panels(&*context.ui_state, &context.node_graph_res.0)
            .unwrap_or((context.ui_state.last_selected_node_id.unwrap_or_default(), Vec::new()));
        let has = !panels.is_empty();
        let owner_opt = has.then_some(owner);
        if self.dock_owner != owner_opt { self.dock_owner = owner_opt; self.dock_sig = 0; }
        let mut desired: Vec<ViewportDockTab> = vec![ViewportDockTab::Viewport];
        for p in panels { desired.push(ViewportDockTab::Coverlay(p)); }
        let sig = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            self.dock_owner.hash(&mut h);
            desired.len().hash(&mut h);
            for t in &desired { t.hash(&mut h); }
            h.finish()
        };
        if self.dock_sig != sig {
            self.dock_sig = sig;
            if let Some(owner) = self.dock_owner {
                let saved = context.node_graph_res.0.nodes.get(&owner)
                    .and_then(|n| n.parameters.iter().find(|p| p.name == VIEWPORT_DOCK_LAYOUT_KEY))
                    .and_then(|p| if let crate::nodes::parameter::ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None });
                let preset = context.node_graph_res.0.nodes.get(&owner)
                    .and_then(|n| n.parameters.iter().find(|p| p.name == VIEWPORT_DOCK_PRESET_KEY))
                    .and_then(|p| if let crate::nodes::parameter::ParameterValue::String(s) = &p.value { Some(s.clone()) } else { None });
                let pal_ratio = preset_palette_ratio(preset.as_deref());
                let mut map: std::collections::HashMap<CoverlayPanelKey, CoverlayDockPanel> = std::collections::HashMap::new();
                for t in desired.iter() { if let ViewportDockTab::Coverlay(p) = t { map.insert(p.key, p.clone()); } }
                let loaded = saved.as_deref()
                    .and_then(|s| serde_json::from_str::<DockState<ViewportDockTab>>(s).ok())
                    .map(|st| st.filter_map_tabs(|t| match t { ViewportDockTab::Viewport => Some(ViewportDockTab::Viewport), ViewportDockTab::Coverlay(p) => map.get(&p.key).cloned().map(ViewportDockTab::Coverlay) }))
                    .filter(|st| st.iter_all_tabs().next().is_some());
                self.dock_state = loaded.unwrap_or_else(|| {
                    let mut panels: Vec<CoverlayDockPanel> = map.values().cloned().collect();
                    panels.sort_by(|a, b| a.title.cmp(&b.title));
                    build_default_viewport_dock_from_preset(preset, panels)
                });
                if let Some(r) = pal_ratio { apply_palette_ratio_once(&mut self.dock_state, r); }
                clamp_viewport_dock_fractions(&mut self.dock_state, VIEWPORT_KEEP_X, VIEWPORT_KEEP_Y);
            } else {
                self.dock_state = DockState::new(vec![ViewportDockTab::Viewport]);
            }
        }

        struct Viewer<'a, 'b> { tab: &'a mut Viewport3DTab, cx: &'a mut EditorTabContext<'b> }
        impl<'a, 'b> DockTabViewer for Viewer<'a, 'b> {
            type Tab = ViewportDockTab;
            fn title(&mut self, t: &mut Self::Tab) -> egui::WidgetText {
                match t { ViewportDockTab::Viewport => "".into(), ViewportDockTab::Coverlay(p) => p.title.clone().into() }
            }
            fn ui(&mut self, ui: &mut egui::Ui, t: &mut Self::Tab) {
                match t {
                    ViewportDockTab::Viewport => self.tab.viewport_canvas_ui(ui, self.cx),
                    ViewportDockTab::Coverlay(p) => {
                        let r = ui.max_rect();
                        if ui.ctx().pointer_hover_pos().is_some_and(|pt| r.contains(pt)) || ui.ctx().wants_keyboard_input() { self.cx.coverlay_wants_input.0 = true; }
                        self.tab.coverlay_dock.draw_panel(ui, self.cx, p);
                    }
                }
            }
            fn allowed_in_windows(&self, _tab: &mut Self::Tab) -> bool { false }
            fn closeable(&mut self, _tab: &mut Self::Tab) -> bool { false }
            fn clear_background(&self, t: &Self::Tab) -> bool { !matches!(t, ViewportDockTab::Viewport) }
        }

        let mut dock_state = std::mem::replace(&mut self.dock_state, DockState::new(vec![ViewportDockTab::Viewport]));
        { let mut viewer = Viewer { tab: self, cx: context }; egui_dock::CoverlayDockArea::new(&mut dock_state).id(egui::Id::new("viewport_coverlay_inner_dock")).show_inside(ui, &mut viewer); }
        self.dock_state = dock_state;

        // Persist dock layout per owner node.
        if let Some(owner) = self.dock_owner {
            if !ui.ctx().input(|i| i.pointer.any_down()) {
                let st = coverlay_strip_runtime(self.dock_state.clone());
                if let Ok(json) = serde_json::to_string(&st) {
                    let g = &mut context.node_graph_res.0;
                    if let Some(n) = g.nodes.get_mut(&owner) {
                        if let Some(p) = n.parameters.iter_mut().find(|p| p.name == VIEWPORT_DOCK_LAYOUT_KEY) {
                            if let crate::nodes::parameter::ParameterValue::String(cur) = &p.value { if *cur == json { return; } }
                            p.value = crate::nodes::parameter::ParameterValue::String(json);
                        } else {
                            n.parameters.push(crate::nodes::parameter::Parameter::new(
                                VIEWPORT_DOCK_LAYOUT_KEY,
                                VIEWPORT_DOCK_LAYOUT_KEY,
                                "Internal",
                                crate::nodes::parameter::ParameterValue::String(json),
                                crate::nodes::parameter::ParameterUIType::Code,
                            ));
                        }
                    }
                }
            }
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Viewport3DTab {
    fn side_icon_button(ui: &mut egui::Ui, id: &'static str, icon: egui::ImageSource<'static>, selected: bool) -> egui::Response {
        let size = 20.0;
        let button_size = egui::vec2(size, size);
        let (_, rect) = ui.allocate_space(button_size);
        let response = ui.interact(rect, ui.make_persistent_id(id), egui::Sense::click());
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, ""));
        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact_selectable(&response, selected);
            if selected || response.hovered() { ui.painter().rect(rect.expand(visuals.expansion), visuals.corner_radius, visuals.bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside); }
            egui::Image::new(icon).tint(visuals.text_color()).paint_at(ui, egui::Rect::from_center_size(rect.center(), button_size * 0.8));
        }
        response
    }

    fn render_viewport_chrome(&mut self, ui: &mut egui::Ui, context: &mut EditorTabContext) {
        let display_options = &mut context.display_options;
        egui::TopBottomPanel::top("viewport_3d_header")
            .frame(
                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .fill(ui.style().visuals.panel_fill.linear_multiply(0.8)),
            )
            .resizable(false)
            .min_height(32.0)
            .show_inside(ui, |ui| {
                // NOTE: header is interactive; don't wrap in retained cache (otherwise clicks only apply on rebuild frames).
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("3D Viewport")
                            .strong()
                            .color(egui::Color32::from_white_alpha(100)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("New Window").clicked() { context.open_naive_window_writer.write(OpenNaiveWindowEvent); }
                        ui.separator();
                        if Self::side_icon_button(ui, "shaded_wire", ViewportIcons::SHADED_WIRE, display_options.final_geometry_display_mode == DisplayMode::ShadedAndWireframe).on_hover_text("Shaded + Wireframe").clicked() { display_options.final_geometry_display_mode = DisplayMode::ShadedAndWireframe; }
                        if Self::side_icon_button(ui, "wireframe", ViewportIcons::WIREFRAME, display_options.final_geometry_display_mode == DisplayMode::Wireframe).on_hover_text("Wireframe").clicked() { display_options.final_geometry_display_mode = DisplayMode::Wireframe; }
                        if Self::side_icon_button(ui, "shaded", ViewportIcons::SHADED, display_options.final_geometry_display_mode == DisplayMode::Shaded).on_hover_text("Shaded").clicked() { display_options.final_geometry_display_mode = DisplayMode::Shaded; }
                        ui.separator();
                        egui::ComboBox::from_id_salt("lighting_mode")
                            .selected_text(match display_options.lighting_mode { ViewportLightingMode::HeadlightOnly => "Headlight", ViewportLightingMode::FullLighting => "Full", ViewportLightingMode::FullLightingWithShadow => "Full+Shadow" })
                            .width(80.0).show_ui(ui, |ui| {
                                ui.selectable_value(&mut display_options.lighting_mode, ViewportLightingMode::HeadlightOnly, "Headlight Only");
                                ui.selectable_value(&mut display_options.lighting_mode, ViewportLightingMode::FullLighting, "Full Lighting");
                                ui.selectable_value(&mut display_options.lighting_mode, ViewportLightingMode::FullLightingWithShadow, "Full + Shadow");
                            });
                        ui.separator();
                        ui.toggle_value(&mut display_options.wireframe_ghost_mode, "Ghost").on_hover_text("Ghost Wireframe");
                        ui.toggle_value(&mut display_options.turntable.enabled, "Turntable").on_hover_text("Auto-frame + rotate (disables manual camera controls)");
                        ui.add_enabled_ui(display_options.turntable.enabled, |ui| {
                            ui.add(egui::DragValue::new(&mut display_options.turntable.speed_deg_per_sec).speed(1.0).range(0.0..=360.0)).on_hover_text("Turntable speed (deg/sec)");
                        });
                        ui.separator();
                    });
                });
            });

        egui::SidePanel::left("handle_controls_panel")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 8)))
            .resizable(false)
            .width_range(if display_options.is_handle_controls_collapsed {
                12.0..=12.0
            } else {
                32.0..=32.0
            })
            .show_inside(ui, |ui| {
                // NOTE: panel is interactive; don't wrap in retained cache.
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    if display_options.is_handle_controls_collapsed {
                        if Self::side_icon_button(ui, "expand_controls", ViewportIcons::EXPAND, false)
                            .on_hover_text("Show Controls")
                            .clicked()
                        {
                            display_options.is_handle_controls_collapsed = false;
                        }
                    } else {
                        if Self::side_icon_button(ui, "collapse_controls", ViewportIcons::COLLAPSE, false)
                            .on_hover_text("Hide Controls")
                            .clicked()
                        {
                            display_options.is_handle_controls_collapsed = true;
                        }
                        ui.separator();
                        ui.add(
                            egui::DragValue::new(&mut display_options.camera_speed)
                                .speed(0.1)
                                .range(0.1..=100.0),
                        )
                        .on_hover_text("Camera Speed (Drag)");
                        ui.separator();
                        if Self::side_icon_button(ui, "gizmo", ViewportIcons::GIZMO, self.show_gizmos)
                            .on_hover_text("Show Transform Gizmo")
                            .clicked()
                        {
                            self.show_gizmos = !self.show_gizmos;
                        }
                    }
                });
            });

        egui::SidePanel::right("display_options_panel")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 8)))
            .resizable(false)
            .width_range(if display_options.is_options_collapsed { 12.0..=12.0 } else { 32.0..=32.0 })
            .show_inside(ui, |ui| {
                // NOTE: panel is interactive; don't wrap in retained cache.
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    if display_options.is_options_collapsed {
                        if Self::side_icon_button(ui, "show_options", ViewportIcons::COLLAPSE, false).on_hover_text("Show Options").clicked() {
                            display_options.is_options_collapsed = false;
                        }
                    } else {
                        if Self::side_icon_button(ui, "hide_options", ViewportIcons::EXPAND, false).on_hover_text("Hide Options").clicked() {
                            display_options.is_options_collapsed = true;
                        }
                        ui.separator();
                        
                        // [NEW] Group Visualizer Section
                        ui.collapsing("Group Viz", |ui| {
                            ui.checkbox(&mut display_options.overlays.highlight_active_group, "Highlight Active Node Group")
                                .on_hover_text("Automatically highlight the group defined by the currently displayed node");
                            
                            // Point Groups
                            ui.add_enabled_ui(!display_options.overlays.highlight_active_group, |ui| {
                                egui::ComboBox::from_id_salt("point_group_viz")
                                    .selected_text(display_options.overlays.point_group_viz.as_deref().unwrap_or("None"))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut display_options.overlays.point_group_viz, None, "None");
                                        ui.selectable_value(&mut display_options.overlays.point_group_viz, Some("group1".to_string()), "group1");
                                        ui.selectable_value(&mut display_options.overlays.point_group_viz, Some("group2".to_string()), "group2");
                                    });
                                    
                                // Edge Groups
                                egui::ComboBox::from_id_salt("edge_group_viz")
                                    .selected_text(display_options.overlays.edge_group_viz.as_deref().unwrap_or("None"))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut display_options.overlays.edge_group_viz, None, "None");
                                        ui.selectable_value(&mut display_options.overlays.edge_group_viz, Some("edge_group1".to_string()), "edge_group1");
                                    });

                                // Vertex Groups
                                egui::ComboBox::from_id_salt("vert_group_viz")
                                    .selected_text(display_options.overlays.vertex_group_viz.as_deref().unwrap_or("None"))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut display_options.overlays.vertex_group_viz, None, "None");
                                        ui.selectable_value(&mut display_options.overlays.vertex_group_viz, Some("vert_group1".to_string()), "vert_group1");
                                    });
                            });
                        });
                        
                        ui.vertical(|ui| {
                            if Self::side_icon_button(ui, "grid", ViewportIcons::GRID, display_options.grid.show).on_hover_text("Show Grid").clicked() {
                                display_options.grid.show = !display_options.grid.show;
                            }
                            ui.separator();
                            if Self::side_icon_button(ui, "points", ViewportIcons::POINTS, display_options.overlays.show_points).on_hover_text("Show Points").clicked() {
                                display_options.overlays.show_points = !display_options.overlays.show_points;
                            }
                            if Self::side_icon_button(ui, "point_nums", ViewportIcons::POINT_NUMS, display_options.overlays.show_point_numbers).on_hover_text("Show Point Numbers").clicked() {
                                display_options.overlays.show_point_numbers = !display_options.overlays.show_point_numbers;
                            }
                            if Self::side_icon_button(ui, "vert_nums", ViewportIcons::VERT_NUMS, display_options.overlays.show_vertex_numbers).on_hover_text("Show Vertex Numbers").clicked() {
                                display_options.overlays.show_vertex_numbers = !display_options.overlays.show_vertex_numbers;
                            }
                            if Self::side_icon_button(ui, "vert_norms", ViewportIcons::VERT_NORMS, display_options.overlays.show_vertex_normals).on_hover_text("Show Vertex Normals").clicked() {
                                display_options.overlays.show_vertex_normals = !display_options.overlays.show_vertex_normals;
                            }
                            if Self::side_icon_button(ui, "prim_nums", ViewportIcons::PRIM_NUMS, display_options.overlays.show_primitive_numbers).on_hover_text("Show Primitive Numbers").clicked() {
                                display_options.overlays.show_primitive_numbers = !display_options.overlays.show_primitive_numbers;
                            }
                            if Self::side_icon_button(ui, "prim_norms", ViewportIcons::PRIM_NORMS, display_options.overlays.show_primitive_normals).on_hover_text("Show Primitive Normals").clicked() {
                                display_options.overlays.show_primitive_normals = !display_options.overlays.show_primitive_normals;
                            }
                            ui.separator();
                        });
                    }
                });
            });
    }

    fn viewport_canvas_ui(&mut self, ui: &mut egui::Ui, context: &mut EditorTabContext) {
        let available_size = ui.available_size();
        self.viewport_size = Some(available_size);
        let (rect, response) = ui.allocate_exact_size(available_size, Sense::click_and_drag());
        self.viewport_rect = Some(rect);
        if context.viewport_layout.window_entity != Some(context.window_entity) { context.viewport_layout.window_entity = Some(context.window_entity); }
        if context.viewport_layout.logical_rect != Some(rect) { context.viewport_layout.logical_rect = Some(rect); }
        let interaction_state = &mut context.viewport_interaction_state;
        let (was_r, was_m, was_a) = (interaction_state.is_right_button_dragged, interaction_state.is_middle_button_dragged, interaction_state.is_alt_left_button_dragged);
        let hovered = response.hovered();
        interaction_state.is_hovered = hovered;
        let right_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Secondary));
        let middle_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle));
        let alt_left_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary) && i.modifiers.alt);
        interaction_state.is_right_button_dragged = right_down && (hovered || was_r);
        interaction_state.is_middle_button_dragged = middle_down && (hovered || was_m);
        interaction_state.is_alt_left_button_dragged = alt_left_down && (hovered || was_a);

        // Selected node image view mode: draw image overlay into the viewport rect (HUD stays on top).
        if matches!(context.display_options.view_mode, ViewportViewMode::NodeImage) {
            let selected = context
                .ui_state
                .last_selected_node_id
                .or_else(|| context.ui_state.selected_nodes.iter().next().copied());
            let rel = selected
                .and_then(|id| {
                    let g = &context.node_graph_res.0;
                    // 1) Prefer node parameter (Nano Heightmap).
                    g.nodes.get(&id).and_then(|n| {
                        n.parameters
                            .iter()
                            .find(|p| p.name == crate::nodes::ai_texture::PARAM_IMAGE_PATH)
                            .and_then(|p| if let crate::nodes::parameter::ParameterValue::String(s) = &p.value { Some(s.trim().to_string()) } else { None })
                            .filter(|s| !s.is_empty())
                    })
                    // 2) Fallback to cooked geometry cache detail attr.
                    .or_else(|| {
                        g.geometry_cache
                            .get(&id)
                            .and_then(|geo| {
                                geo.get_detail_attribute(crate::nodes::ai_texture::ATTR_HEIGHTMAP_PATH)
                                    .and_then(|a| a.as_slice::<String>())
                                    .and_then(|v| v.first())
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                            })
                    })
                });

            let uri = rel
                .and_then(|rel| std::env::current_dir().ok().map(|cwd| cwd.join("assets").join(rel)))
                .and_then(|abs| abs.canonicalize().ok())
                .map(|abs| {
                    let p = abs.to_string_lossy().replace('\\', "/");
                    if p.starts_with('/') { format!("file://{}", p) } else { format!("file:///{}", p) }
                });

            ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, egui::Color32::from_rgb(10, 10, 10));
            if let Some(uri) = uri {
                egui::Image::new(uri).fit_to_exact_size(rect.size()).paint_at(ui, rect);
            } else {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "No image found on selected node.",
                    egui::FontId::proportional(14.0),
                    egui::Color32::from_gray(160),
                );
            }
        }

        if let Some(rect) = self.viewport_rect { hud::draw_hud(ui, &mut *context, rect); }
    }
}
