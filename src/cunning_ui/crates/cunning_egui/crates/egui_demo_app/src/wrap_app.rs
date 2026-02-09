use egui_demo_lib::is_mobile;

#[cfg(feature = "glow")]
use eframe::glow;

#[cfg(target_arch = "wasm32")]
use core::any::Any;

#[derive(Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
struct EasyMarkApp {
    editor: egui_demo_lib::easy_mark::EasyMarkEditor,
}

impl eframe::App for EasyMarkApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.editor.panels(ctx);
    }
}

// ----------------------------------------------------------------------------

#[derive(Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct DemoApp {
    demo_windows: egui_demo_lib::DemoWindows,
}

impl eframe::App for DemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.demo_windows.ui(ctx);
    }
}

// ----------------------------------------------------------------------------

#[derive(Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct FractalClockApp {
    fractal_clock: crate::apps::FractalClock,
}

impl eframe::App for FractalClockApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::dark_canvas(&ctx.style()))
            .show(ctx, |ui| {
                self.fractal_clock
                    .ui(ui, Some(crate::seconds_since_midnight()));
            });
    }
}

// ----------------------------------------------------------------------------

#[derive(Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct ColorTestApp {
    color_test: egui_demo_lib::ColorTest,
}

impl eframe::App for ColorTestApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if frame.is_web() {
                ui.label(
                    "NOTE: Some old browsers stuck on WebGL1 without sRGB support will not pass the color test.",
                );
                ui.separator();
            }
            egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                self.color_test.ui(ui);
            });
        });
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
enum Anchor {
    Demo,
    EasyMarkEditor,
    #[cfg(feature = "http")]
    Http,
    #[cfg(feature = "image_viewer")]
    ImageViewer,
    Clock,
    #[cfg(any(feature = "glow", feature = "wgpu"))]
    Custom3d,
    Colors,
}

impl Anchor {
    #[cfg(target_arch = "wasm32")]
    fn all() -> Vec<Self> {
        vec![
            Self::Demo,
            Self::EasyMarkEditor,
            #[cfg(feature = "http")]
            Self::Http,
            Self::Clock,
            #[cfg(any(feature = "glow", feature = "wgpu"))]
            Self::Custom3d,
            Self::Colors,
        ]
    }
}

impl std::fmt::Display for Anchor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl From<Anchor> for egui::WidgetText {
    fn from(value: Anchor) -> Self {
        Self::RichText(egui::RichText::new(value.to_string()))
    }
}

impl Default for Anchor {
    fn default() -> Self {
        Self::Demo
    }
}

// ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
#[must_use]
enum Command {
    Nothing,
    ResetEverything,
}

// ----------------------------------------------------------------------------

/// The state that we persist (serialize).
#[derive(Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct State {
    demo: DemoApp,
    easy_mark_editor: EasyMarkApp,
    #[cfg(feature = "http")]
    http: crate::apps::HttpApp,
    #[cfg(feature = "image_viewer")]
    image_viewer: crate::apps::ImageViewer,
    clock: FractalClockApp,
    color_test: ColorTestApp,

    selected_anchor: Anchor,
    backend_panel: super::backend_panel::BackendPanel,
    cunning_sdf: bool,
    cunning_gpu_text: bool,
    cunning_retained: bool,
}

/// Wraps many demo/test apps into one.
pub struct WrapApp {
    state: State,

    #[cfg(any(feature = "glow", feature = "wgpu"))]
    custom3d: Option<crate::apps::Custom3d>,

    dropped_files: Vec<egui::DroppedFile>,
}

impl WrapApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // This gives us image support:
        egui_extras::install_image_loaders(&cc.egui_ctx);

        // Inject Chinese system fonts for native egui text support
        #[cfg(target_os = "windows")]
        {
            let mut fonts = egui::FontDefinitions::default();
            let sys_root = std::env::var("SYSTEMROOT").unwrap_or_else(|_| "C:\\Windows".to_owned());
            let font_paths = [
                format!("{}\\Fonts\\msyh.ttc", sys_root),
                format!("{}\\Fonts\\msyh.ttf", sys_root),
                format!("{}\\Fonts\\simhei.ttf", sys_root),
            ];
            for path in font_paths {
                if let Ok(data) = std::fs::read(&path) {
                    fonts.font_data.insert("Microsoft YaHei".to_owned(), egui::FontData::from_owned(data));
                    if let Some(vec) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                        vec.insert(0, "Microsoft YaHei".to_owned());
                    }
                    if let Some(vec) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                        vec.push("Microsoft YaHei".to_owned());
                    }
                    break;
                }
            }
            cc.egui_ctx.set_fonts(fonts);
        }

        #[cfg(feature = "wgpu")]
        cc.egui_ctx.set_text_renderer(std::sync::Arc::new(|ctx, pos, galley, color, clip_rect| {
            // Cunning: Iterate rows to respect egui's layout (wrapping, alignment) perfectly.
            let mut shapes = Vec::with_capacity(galley.rows.len());
            let font_px = galley.job.sections.first().map(|s| s.format.font_id.size).unwrap_or(12.0);
            let family = if matches!(galley.job.sections.first().map(|s| &s.format.font_id.family), Some(egui::FontFamily::Monospace)) { 1 } else { 0 };
            for row in &galley.rows {
                let row_text: String = row.glyphs.iter().map(|g| g.chr).collect();
                if row_text.trim().is_empty() { continue; }
                let row_pos = pos + row.rect.min.to_vec2();
                shapes.push(egui::Shape::Callback(egui_wgpu::sdf::create_gpu_text_callback(
                    clip_rect,
                    egui_wgpu::sdf::GpuTextUniform {
                        text: row_text,
                        pos: row_pos,
                        color,
                        font_px,
                        bounds: row.rect.size(),
                        family,
                    },
                    ctx.frame_nr(),
                )));
            }
            Some(shapes)
        }));

        #[allow(unused_mut)]
        let mut slf = Self {
            state: State::default(),

            #[cfg(any(feature = "glow", feature = "wgpu"))]
            custom3d: crate::apps::Custom3d::new(cc),

            dropped_files: Default::default(),
        };

        #[cfg(feature = "persistence")]
        if let Some(storage) = cc.storage {
            if let Some(state) = eframe::get_value(storage, eframe::APP_KEY) {
                slf.state = state;
            }
        }

        #[cfg(feature = "wgpu")]
        if !slf.state.cunning_sdf && !slf.state.cunning_gpu_text && !slf.state.cunning_retained {
            slf.state.cunning_sdf = true;
            slf.state.cunning_gpu_text = true;
            slf.state.cunning_retained = true;
        }

        slf
    }

    fn apps_iter_mut(&mut self) -> impl Iterator<Item = (&str, Anchor, &mut dyn eframe::App)> {
        let mut vec = vec![
            (
                "✨ Demos",
                Anchor::Demo,
                &mut self.state.demo as &mut dyn eframe::App,
            ),
            (
                "🖹 EasyMark editor",
                Anchor::EasyMarkEditor,
                &mut self.state.easy_mark_editor as &mut dyn eframe::App,
            ),
            #[cfg(feature = "http")]
            (
                "⬇ HTTP",
                Anchor::Http,
                &mut self.state.http as &mut dyn eframe::App,
            ),
            (
                "🕑 Fractal Clock",
                Anchor::Clock,
                &mut self.state.clock as &mut dyn eframe::App,
            ),
            #[cfg(feature = "image_viewer")]
            (
                "🖼 Image Viewer",
                Anchor::ImageViewer,
                &mut self.state.image_viewer as &mut dyn eframe::App,
            ),
        ];

        #[cfg(any(feature = "glow", feature = "wgpu"))]
        if let Some(custom3d) = &mut self.custom3d {
            vec.push((
                "🔺 3D painting",
                Anchor::Custom3d,
                custom3d as &mut dyn eframe::App,
            ));
        }

        vec.push((
            "🎨 Color test",
            Anchor::Colors,
            &mut self.state.color_test as &mut dyn eframe::App,
        ));

        vec.into_iter()
    }
}

impl eframe::App for WrapApp {
    #[cfg(feature = "persistence")]
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.state);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        egui_demo_lib::cui::set_cfg(ctx, egui_demo_lib::cui::Cfg {
            sdf: self.state.cunning_sdf,
            gpu_text: self.state.cunning_gpu_text,
            retained: self.state.cunning_retained,
        });

        #[cfg(target_arch = "wasm32")]
        if let Some(anchor) = frame.info().web_info.location.hash.strip_prefix('#') {
            let anchor = Anchor::all().into_iter().find(|x| x.to_string() == anchor);
            if let Some(v) = anchor {
                self.state.selected_anchor = v;
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F11)) {
            let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
        }

        let mut cmd = Command::Nothing;
        egui::TopBottomPanel::top("wrap_app_top_bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.visuals_mut().button_frame = false;
                self.bar_contents(ui, frame, &mut cmd);
            });
        });

        self.state.backend_panel.update(ctx, frame);

        if !is_mobile(ctx) {
            cmd = self.backend_panel(ctx, frame);
        }

        self.show_selected_app(ctx, frame);

        self.state.backend_panel.end_of_frame(ctx);

        self.ui_file_drag_and_drop(ctx);

        self.run_cmd(ctx, cmd);
    }

    #[cfg(feature = "glow")]
    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        if let Some(custom3d) = &mut self.custom3d {
            custom3d.on_exit(gl);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(&mut *self)
    }
}

impl WrapApp {
    fn backend_panel(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) -> Command {
        // The backend-panel can be toggled on/off.
        // We show a little animation when the user switches it.
        let is_open =
            self.state.backend_panel.open || ctx.memory(|mem| mem.everything_is_visible());

        let mut cmd = Command::Nothing;

        egui::SidePanel::left("backend_panel")
            .resizable(false)
            .show_animated(ctx, is_open, |ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("💻 Backend");
                });

                ui.separator();
                self.backend_panel_contents(ui, frame, &mut cmd);
            });

        cmd
    }

    fn run_cmd(&mut self, ctx: &egui::Context, cmd: Command) {
        match cmd {
            Command::Nothing => {}
            Command::ResetEverything => {
                self.state = Default::default();
                ctx.memory_mut(|mem| *mem = Default::default());
            }
        }
    }

    fn backend_panel_contents(
        &mut self,
        ui: &mut egui::Ui,
        frame: &mut eframe::Frame,
        cmd: &mut Command,
    ) {
        self.state.backend_panel.ui(ui, frame);

        ui.separator();

        ui.horizontal(|ui| {
            if ui
                .button("Reset egui")
                .on_hover_text("Forget scroll, positions, sizes etc")
                .clicked()
            {
                ui.ctx().memory_mut(|mem| *mem = Default::default());
                ui.close_menu();
            }

            if ui.button("Reset everything").clicked() {
                *cmd = Command::ResetEverything;
                ui.close_menu();
            }
        });
    }

    fn show_selected_app(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let selected_anchor = self.state.selected_anchor;
        for (_name, anchor, app) in self.apps_iter_mut() {
            if anchor == selected_anchor || ctx.memory(|mem| mem.everything_is_visible()) {
                app.update(ctx, frame);
            }
        }
    }

    fn bar_contents(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame, cmd: &mut Command) {
        let cfg = egui_demo_lib::cui::Cfg { sdf: self.state.cunning_sdf, gpu_text: self.state.cunning_gpu_text, retained: self.state.cunning_retained };
        if cfg.retained {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            self.state.selected_anchor.hash(&mut h);
            self.state.backend_panel.open.hash(&mut h);
            self.state.cunning_sdf.hash(&mut h);
            self.state.cunning_gpu_text.hash(&mut h);
            ui.input(|i| i.pointer.hover_pos().map(|p| (p.x.to_bits(), p.y.to_bits()))).hash(&mut h);
            ui.input(|i| i.pointer.any_down()).hash(&mut h);
            let k = h.finish();
            ui.push_id(("cunning_topbar", k), |ui| self.bar_contents_inner(ui, frame, cmd));
            return;
        }
        self.bar_contents_inner(ui, frame, cmd);
    }

    fn bar_contents_inner(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame, cmd: &mut Command) {
        use egui_demo_lib::cui;
        egui::widgets::global_dark_light_mode_switch(ui);

        ui.separator();

        if is_mobile(ui.ctx()) {
            ui.menu_button("💻 Backend", |ui| {
                ui.set_style(ui.ctx().style()); // ignore the "menu" style set by `menu_button`.
                self.backend_panel_contents(ui, frame, cmd);
            });
        } else {
            cui::toggle(ui, &mut self.state.backend_panel.open, "💻 Backend");
        }

        ui.separator();

        let mut selected_anchor = self.state.selected_anchor;
        for (name, anchor, _app) in self.apps_iter_mut() {
            if cui::sdf_button(ui, name, selected_anchor == anchor).clicked() {
                selected_anchor = anchor;
                if frame.is_web() {
                    ui.ctx()
                        .open_url(egui::OpenUrl::same_tab(format!("#{anchor}")));
                }
            }
        }
        self.state.selected_anchor = selected_anchor;

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // CunningUI verification toggles (keep English so it's obvious what they do)
            cui::toggle(ui, &mut self.state.cunning_retained, "Retained");
            cui::toggle(ui, &mut self.state.cunning_gpu_text, "GPU Text");
            cui::toggle(ui, &mut self.state.cunning_sdf, "SDF UI");

            // Make it obvious whether GPU-text callbacks are actually being used.
            #[cfg(feature = "wgpu")]
            {
                let s = egui_wgpu::sdf::gpu_text_last_stats();
                ui.label(egui::RichText::new(format!("GT texts={} dc={}", s.texts, s.drawcalls)).monospace());
                if self.state.cunning_gpu_text {
                    // Big sample: should look clearly different vs normal egui text if something is wrong.
                    cui::gpu_label(ui, "中文测试 GPU Text", 18.0, ui.visuals().text_color());
                } else {
                    ui.label(egui::RichText::new("中文测试 (egui text)").size(18.0));
                }
            }
            if false {
                // TODO(emilk): fix the overlap on small screens
                if clock_button(ui, crate::seconds_since_midnight()).clicked() {
                    self.state.selected_anchor = Anchor::Clock;
                    if frame.is_web() {
                        ui.ctx().open_url(egui::OpenUrl::same_tab("#clock"));
                    }
                }
            }

            egui::warn_if_debug_build(ui);
        });
    }

    fn ui_file_drag_and_drop(&mut self, ctx: &egui::Context) {
        use egui::*;
        use std::fmt::Write as _;

        // Preview hovering files:
        if !ctx.input(|i| i.raw.hovered_files.is_empty()) {
            let text = ctx.input(|i| {
                let mut text = "Dropping files:\n".to_owned();
                for file in &i.raw.hovered_files {
                    if let Some(path) = &file.path {
                        write!(text, "\n{}", path.display()).ok();
                    } else if !file.mime.is_empty() {
                        write!(text, "\n{}", file.mime).ok();
                    } else {
                        text += "\n???";
                    }
                }
                text
            });

            let painter =
                ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("file_drop_target")));

            let screen_rect = ctx.screen_rect();
            painter.rect_filled(screen_rect, 0.0, Color32::from_black_alpha(192));
            painter.text(
                screen_rect.center(),
                Align2::CENTER_CENTER,
                text,
                TextStyle::Heading.resolve(&ctx.style()),
                Color32::WHITE,
            );
        }

        // Collect dropped files:
        ctx.input(|i| {
            if !i.raw.dropped_files.is_empty() {
                self.dropped_files = i.raw.dropped_files.clone();
            }
        });

        // Show dropped files (if any):
        if !self.dropped_files.is_empty() {
            let mut open = true;
            egui::Window::new("Dropped files")
                .open(&mut open)
                .show(ctx, |ui| {
                    for file in &self.dropped_files {
                        let mut info = if let Some(path) = &file.path {
                            path.display().to_string()
                        } else if !file.name.is_empty() {
                            file.name.clone()
                        } else {
                            "???".to_owned()
                        };

                        let mut additional_info = vec![];
                        if !file.mime.is_empty() {
                            additional_info.push(format!("type: {}", file.mime));
                        }
                        if let Some(bytes) = &file.bytes {
                            additional_info.push(format!("{} bytes", bytes.len()));
                        }
                        if !additional_info.is_empty() {
                            info += &format!(" ({})", additional_info.join(", "));
                        }

                        ui.label(info);
                    }
                });
            if !open {
                self.dropped_files.clear();
            }
        }
    }
}

fn clock_button(ui: &mut egui::Ui, seconds_since_midnight: f64) -> egui::Response {
    let time = seconds_since_midnight;
    let time = format!(
        "{:02}:{:02}:{:02}.{:02}",
        (time % (24.0 * 60.0 * 60.0) / 3600.0).floor(),
        (time % (60.0 * 60.0) / 60.0).floor(),
        (time % 60.0).floor(),
        (time % 1.0 * 100.0).floor()
    );

    ui.button(egui::RichText::new(time).monospace())
}
