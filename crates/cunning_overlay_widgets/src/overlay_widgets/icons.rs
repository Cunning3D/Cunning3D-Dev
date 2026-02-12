use bevy_egui::egui::{self, Context, Rect, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

// Embed the icon sheet directly into the binary for zero-latency loading.
// Path is relative to this file.
const ICON_ATLAS_BYTES: &[u8] = include_bytes!("../../../../assets/ui/icon_sheet_v1.png");

// Global handle storage keyed by egui TextureManager identity.
// IMPORTANT: `TextureHandle` is tied to a specific `egui::Context` (TextureManager).
// With egui-dock "detached OS windows" we can have multiple contexts, so a single global handle
// will crash when used across contexts. We cache per TextureManager.
static ICON_TEXTURES: OnceLock<Mutex<HashMap<usize, TextureHandle>>> = OnceLock::new();

fn get_icon_storage() -> &'static Mutex<HashMap<usize, TextureHandle>> {
    ICON_TEXTURES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Ensures the icon atlas is loaded into the current egui Context.
/// Returns the TextureHandle.
pub fn get_icon_texture(ctx: &Context) -> TextureHandle {
    let storage = get_icon_storage();
    let tm: Arc<_> = ctx.tex_manager();
    let key = Arc::as_ptr(&tm) as usize;
    if let Some(h) = storage.lock().unwrap().get(&key) { return h.clone(); }

    // Load the image (and downscale if egui max texture side is smaller).
    let mut image = image::load_from_memory(ICON_ATLAS_BYTES).expect("Failed to load embedded icon atlas");
    let max_side = ctx.input(|i| i.max_texture_side) as u32;
    let (w, h) = (image.width(), image.height());
    if w > max_side || h > max_side {
        let s = (max_side as f32) / (w.max(h) as f32);
        let nw = ((w as f32) * s).round().clamp(1.0, max_side as f32) as u32;
        let nh = ((h as f32) * s).round().clamp(1.0, max_side as f32) as u32;
        image = image.resize_exact(nw, nh, image::imageops::FilterType::Lanczos3);
    }
    let size = [image.width() as usize, image.height() as usize];
    let image_buffer = image.to_rgba8();
    let pixels = image_buffer.as_flat_samples();
    
    let color_image = egui::ColorImage::from_rgba_unmultiplied(
        size,
        pixels.as_slice(),
    );

    // Upload to GPU
    // "Retained" means egui keeps it as long as we hold the handle.
    let handle = ctx.load_texture(
        "overlay_icons",
        color_image,
        TextureOptions::LINEAR, // Linear filtering for smooth scaling
    );

    storage.lock().unwrap().insert(key, handle.clone());
    handle
}

/// Returns the UV rectangle for a named icon.
/// Atlas is 8 columns x 2 rows.
pub fn get_icon_uv(name: &str) -> Rect {
    let (col, row) = match name {
        // Row 1
        "plus" | "add" => (0, 0),
        "select" | "cursor" | "square" => (1, 0),
        "move" | "arrows" => (2, 0),
        "brush" | "paint" => (3, 0),
        "extrude" | "up" => (4, 0),
        "undo" => (5, 0),
        "redo" => (6, 0),
        "delete" | "trash" | "remove" => (7, 0),
        
        // Row 2
        "trim" | "scissors" | "cut" => (0, 1),
        "picker" | "eye" | "color" => (1, 1),
        "region" | "grid" => (2, 1),
        "sphere" | "circle" => (3, 1),
        "cube" | "box" => (4, 1),
        "cylinder" => (5, 1),
        "cross" | "x" => (6, 1),
        "stamp" | "sparkles" | "magic" => (7, 1),

        // Fallback for unknown icons (show a generic 'select' or 'cross')
        _ => (6, 1), 
    };

    let cols = 8.0;
    let rows = 2.0;
    
    let u_step = 1.0 / cols;
    let v_step = 1.0 / rows;

    let u0 = col as f32 * u_step;
    let v0 = row as f32 * v_step;
    
    Rect::from_min_max(
        egui::pos2(u0, v0),
        egui::pos2(u0 + u_step, v0 + v_step),
    )
}
