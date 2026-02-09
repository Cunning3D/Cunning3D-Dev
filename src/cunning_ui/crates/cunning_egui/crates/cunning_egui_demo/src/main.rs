use eframe::egui;
use std::sync::Arc;

fn main() -> eframe::Result<()> {
    env_logger::init();
    
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_title("Cunning Egui Demo (Standardized)"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    
    eframe::run_native(
        "cunning_egui_demo",
        options,
        Box::new(|cc| {
            // 确保 context 配置一致
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            cc.egui_ctx.set_text_renderer(Arc::new(|ctx, pos, galley, color, clip_rect| {
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
            Box::new(DemoApp::default())
        }),
    )
}

struct DemoApp {
    sdf_enabled: bool,
    gpu_text_enabled: bool,
}

impl Default for DemoApp {
    fn default() -> Self {
        Self {
            sdf_enabled: true,
            gpu_text_enabled: true,
        }
    }
}

impl eframe::App for DemoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Cunning Egui Demo (Clean Environment)");
            ui.separator();
            
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.sdf_enabled, "Enable SDF Buttons");
                ui.checkbox(&mut self.gpu_text_enabled, "Enable GPU Text");
            });
            
            ui.add_space(20.0);
            
            ui.label("Standard Egui Label");
            
            ui.add_space(10.0);
            
            // Custom GPU Text Label
            if self.gpu_text_enabled {
                gpu_label(ui, "GPU Text Label (中文测试)", 20.0, egui::Color32::GREEN);
            } else {
                ui.label(egui::RichText::new("Fallback Egui Label (中文测试)").size(20.0).color(egui::Color32::GREEN));
            }

            ui.add_space(20.0);

            // SDF Buttons
            ui.horizontal(|ui| {
                if self.sdf_enabled {
                    if sdf_button(ui, "SDF Button 1", false, self.gpu_text_enabled).clicked() {
                        println!("Clicked 1");
                    }
                    if sdf_button(ui, "SDF Button 2 (Selected)", true, self.gpu_text_enabled).clicked() {
                        println!("Clicked 2");
                    }
                } else {
                    ui.button("Standard Button 1");
                    ui.button("Standard Button 2");
                }
            });
            
            ui.add_space(20.0);
            
            #[cfg(feature = "wgpu")]
            {
                let stats = egui_wgpu::sdf::gpu_text_last_stats();
                ui.monospace(format!("GPU Text Stats: Texts={} DrawCalls={} Verts={}", stats.texts, stats.drawcalls, stats.verts));
            }
        });
    }
}

// --- Minimal CUI Implementation ---

fn gpu_label(ui: &mut egui::Ui, text: &str, font_px: f32, color: egui::Color32) {
    let font = egui::FontId::proportional(font_px);
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, color));
    let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    
    // Using Clip Rect as the callback rect, matching main app behavior
    let cb = egui_wgpu::sdf::create_gpu_text_callback(
        ui.painter().clip_rect(),
        egui_wgpu::sdf::GpuTextUniform {
            text: text.to_owned(),
            pos: rect.min,
            color,
            font_px,
            bounds: galley.rect.size(),
            family: 0,
        },
        ui.ctx().frame_nr(),
    );
    ui.painter().add(egui::Shape::Callback(cb));
}

fn sdf_button(ui: &mut egui::Ui, text: &str, selected: bool, use_gpu_text: bool) -> egui::Response {
    let font_px = 14.0;
    let pad = egui::vec2(12.0, 8.0);
    let font = egui::FontId::proportional(font_px);
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE));
    let size = galley.size() + pad * 2.0;
    
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        
        // Draw SDF Rect
        let r = visuals.rounding().nw as f32;
        let u = egui_wgpu::sdf::SdfRectUniform {
            center: [rect.center().x, rect.center().y],
            half_size: [rect.width() * 0.5, rect.height() * 0.5],
            corner_radii: [r, r, r, r],
            fill_color: egui::Rgba::from(visuals.bg_fill).to_array(),
            shadow_color: [0.0; 4],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0; 2],
            border_width: visuals.bg_stroke.width,
            _pad2: [0.0; 3],
            border_color: egui::Rgba::from(visuals.bg_stroke.color).to_array(),
            screen_size: [ui.ctx().screen_rect().width(), ui.ctx().screen_rect().height()],
            _pad3: [0.0; 2],
        };
        // Use painter's clip_rect to allow SDF shadows/borders to bleed outside the logical rect
        let cb = egui_wgpu::sdf::create_sdf_rect_callback(ui.painter().clip_rect(), u, ui.ctx().cumulative_frame_nr());
        ui.painter().add(egui::Shape::Callback(cb));

        // Draw Text
        let text_pos = rect.min + pad;
        let text_color = visuals.text_color();
        
        if use_gpu_text {
             let cb = egui_wgpu::sdf::create_gpu_text_callback(
                ui.painter().clip_rect(),
                egui_wgpu::sdf::GpuTextUniform {
                    text: text.to_owned(),
                    pos: text_pos,
                    color: text_color,
                    font_px,
                    bounds: rect.size(),
                    family: 0,
                },
                ui.ctx().frame_nr(),
            );
            ui.painter().add(egui::Shape::Callback(cb));
        } else {
            ui.painter().text(text_pos, egui::Align2::LEFT_TOP, text, font, text_color);
        }
    }
    
    response
}


