//! Nano-to-3D common helpers: Gemini atlas -> points -> voxel/volume.

use base64::Engine;
use bevy::prelude::*;
use image::{GenericImageView, ImageEncoder};
use reqwest::blocking::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::volume::{Chunk, VoxelGrid, CHUNK_SIZE};

pub(crate) const ATTR_REF_IMAGE_PATH: &str = "__ai_reference_image_path";

#[derive(Clone, Copy, Debug)]
pub(crate) enum ViewFace { Front, Back, Left, Right, Top, Bottom }

pub(crate) const ATLAS_FACES: [ViewFace; 6] = [
    ViewFace::Front, ViewFace::Back, ViewFace::Left,
    ViewFace::Right, ViewFace::Top, ViewFace::Bottom,
];

#[derive(Clone, Copy, Debug)]
pub(crate) struct ViewDef {
    pub az: f32, // Radians. 0 = +Z (Front).
    pub el: f32, // Radians. +PI/2 = +Y (Top).
}

pub(crate) fn get_views_24() -> Vec<ViewDef> {
    let mut v = Vec::new();
    // 3 Rows of 8 columns.
    // Row 1: Elevation 0 (Equator).
    for i in 0..8 {
        let az = (i as f32 / 8.0) * std::f32::consts::TAU;
        v.push(ViewDef { az, el: 0.0 });
    }
    // Row 2: Elevation +45 deg.
    for i in 0..8 {
        let az = (i as f32 / 8.0) * std::f32::consts::TAU;
        v.push(ViewDef { az, el: 45.0f32.to_radians() });
    }
    // Row 3: Elevation -45 deg.
    for i in 0..8 {
        let az = (i as f32 / 8.0) * std::f32::consts::TAU;
        v.push(ViewDef { az, el: -45.0f32.to_radians() });
    }
    v
}

#[inline]
pub(crate) fn load_gemini_key() -> String {
    let k = crate::cunning_core::ai_service::gemini::api_key::read_gemini_api_key_env();
    if !k.trim().is_empty() { return k; }
    let raw = std::fs::read_to_string(crate::runtime_paths::ai_providers_path()).unwrap_or_default();
    let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    v.get("gemini").and_then(|g| g.get("api_key")).and_then(|x| x.as_str()).unwrap_or("").trim().to_string()
}

#[inline]
pub(crate) fn load_gemini_model_image() -> String {
    if let Ok(m) = std::env::var("CUNNING_GEMINI_MODEL_IMAGE") { if !m.trim().is_empty() { return m; } }
    if let Ok(m) = std::env::var("CUNNING_GEMINI_IMAGE_MODEL") { if !m.trim().is_empty() { return m; } }
    let raw = std::fs::read_to_string(crate::runtime_paths::ai_providers_path()).unwrap_or_default();
    let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    v.get("gemini").and_then(|g| g.get("model_image").or_else(|| g.get("image_model"))).and_then(|x| x.as_str()).unwrap_or("gemini-3-pro-image-preview").trim().to_string()
}

#[derive(Clone, Debug)]
pub(crate) struct ImgIn { pub mime: String, pub bytes: Vec<u8> }

#[derive(Clone, Debug)]
pub(crate) struct ImgOut { pub mime: String, pub bytes: Vec<u8> }

#[inline]
fn image_size_for_px(px: u32) -> &'static str { if px <= 1024 { "1K" } else if px <= 2048 { "2K" } else { "4K" } }

#[inline]
fn ext_for_mime(mime: &str) -> &'static str {
    match mime.trim() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        _ => "png",
    }
}

pub(crate) fn load_image_path(p: &str) -> Result<ImgIn, String> {
    let s = p.trim();
    if s.is_empty() { return Err("Empty image path.".to_string()); }
    let raw = PathBuf::from(s);
    let abs = if raw.is_absolute() {
        raw
    } else {
        let rel = s.replace('\\', "/").trim_start_matches("assets/").trim_start_matches("./").to_string();
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let a0 = cwd.join("assets").join(&rel);
        if a0.exists() { a0 } else { cwd.join(&rel) }
    };
    let bytes = std::fs::read(&abs).map_err(|e| format!("read {}: {}", abs.display(), e))?;
    let mime = if abs.extension().and_then(|e| e.to_str()).unwrap_or("").eq_ignore_ascii_case("webp") {
        "image/webp"
    } else if abs.extension().and_then(|e| e.to_str()).unwrap_or("").eq_ignore_ascii_case("jpg") || abs.extension().and_then(|e| e.to_str()).unwrap_or("").eq_ignore_ascii_case("jpeg") {
        "image/jpeg"
    } else {
        "image/png"
    };
    Ok(ImgIn { mime: mime.to_string(), bytes })
}

pub(crate) fn gemini_generate_image(
    timeout_s: i32,
    system: &str,
    prompt: &str,
    images: &[ImgIn],
    out_w: u32,
    out_h: u32,
) -> Result<ImgOut, String> {
    let api_key = load_gemini_key();
    if api_key.trim().is_empty() { return Err("Gemini API key missing (GEMINI_API_KEY or settings/ai/providers.json).".to_string()); }
    let model = load_gemini_model_image();
    let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent");
    let parts = {
        let mut v = vec![serde_json::json!({ "text": format!("{system}\n\n{prompt}") })];
        for img in images {
            v.push(serde_json::json!({ "inlineData": { "mimeType": img.mime, "data": base64::engine::general_purpose::STANDARD.encode(&img.bytes) } }));
        }
        v
    };
    let body = serde_json::json!({
        "contents": [{ "role": "user", "parts": parts }],
        "generationConfig": {
            "responseModalities": ["TEXT", "IMAGE"],
            "imageConfig": {
                "aspectRatio": "3:2",
                "imageSize": image_size_for_px(out_w.max(out_h).max(1))
            }
        }
    });
    let mut b = Client::builder().connect_timeout(Duration::from_secs(10));
    if timeout_s > 0 { b = b.timeout(Duration::from_secs(timeout_s as u64)); }
    let client = b.build().map_err(|e| e.to_string())?;
    let resp = client.post(&url).header("x-goog-api-key", api_key).json(&body).send().map_err(|e| e.to_string())?;
    let status = resp.status();
    let txt = resp.text().unwrap_or_default();
    if !status.is_success() { return Err(format!("HTTP {} (model={}): {}", status.as_u16(), model, txt)); }
    let v: Value = serde_json::from_str(&txt).map_err(|e| format!("JSON: {e}"))?;
    let parts = v.get("candidates").and_then(|c| c.get(0)).and_then(|c| c.get("content")).and_then(|c| c.get("parts")).and_then(|p| p.as_array()).cloned().unwrap_or_default();
    for p in parts {
        let id = p.get("inlineData").or_else(|| p.get("inline_data")).and_then(|x| x.as_object());
        let Some(id) = id else { continue; };
        let mime = id.get("mimeType").or_else(|| id.get("mime_type")).and_then(|m| m.as_str()).unwrap_or("");
        let data = id.get("data").and_then(|d| d.as_str()).unwrap_or("");
        if !mime.starts_with("image/") || data.trim().is_empty() { continue; }
        let bytes = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).map_err(|e| e.to_string())?;
        return Ok(ImgOut { mime: mime.to_string(), bytes });
    }
    Err("Gemini: no image inlineData returned".to_string())
}

pub(crate) fn save_img_under_assets(subdir: &str, name_no_ext: &str, img: &ImgOut) -> Result<String, String> {
    let ext = ext_for_mime(img.mime.as_str());
    let file_name = format!("{name_no_ext}.{ext}");
    let assets = std::env::current_dir().map_err(|e| e.to_string())?.join("assets");
    let abs = assets.join(subdir.trim_matches('/')).join(&file_name);
    if let Some(p) = abs.parent() { let _ = std::fs::create_dir_all(p); }
    std::fs::write(&abs, &img.bytes).map_err(|e| e.to_string())?;
    Ok(format!("{}/{}", subdir.trim_matches('/'), file_name).replace('\\', "/"))
}

pub(crate) fn decode_rgba_resized(bytes: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    image::load_from_memory(bytes)
        .map_err(|e| e.to_string())
        .map(|i| i.to_rgba8())
        .map(|img| image::imageops::resize(&img, w, h, image::imageops::FilterType::Lanczos3).into_raw())
        .map_err(|e| e.to_string())
}

#[inline]
fn tile_xy(i: usize, cols: u32) -> (u32, u32) { ((i as u32) % cols, (i as u32) / cols) }

pub(crate) fn points_from_depth_atlas(
    atlas_rgba: &[u8],
    tile_res: u32,
    world_size: f32,
    sample_step: u32,
    depth_min: f32,
) -> Vec<Vec3> {
    let s = (sample_step.max(1)) as i32;
    // Detect layout based on image size.
    // 6 views: 3x2. 24 views: 8x3.
    let total_pixels = atlas_rgba.len() / 4;
    let w_est = (total_pixels as f32 * 1.5).sqrt() as u32; // 3/2 aspect
    let w_est_24 = (total_pixels as f32 * 8.0 / 3.0).sqrt() as u32; // 8/3 aspect
    
    let (cols, rows, is_24) = if (w_est_24 as f32 / tile_res as f32 - 8.0).abs() < 1.0 {
        (8, 3, true)
    } else {
        (3, 2, false)
    };

    let w0 = (tile_res * cols) as i32;
    let h0 = (tile_res * rows) as i32;
    let idx = |x: i32, y: i32| -> usize { ((y as usize * w0 as usize + x as usize) * 4) };
    let mut out = Vec::new();
    let hs = world_size.max(1e-6) * 0.5;
    // Margin to avoid sampling tile borders (which Gemini often draws as white frames).
    // 5% margin on each side.
    let margin = (tile_res as f32 * 0.05) as i32;
    
    // Map pixel 0..res to -hs..hs.
    // Image Y is down (0 at top), World Y is up. So pixel Y needs inversion.
    let pix_to_u = |u: i32| -> f32 { ((u as f32 + 0.5) / (tile_res as f32) - 0.5) * (hs * 2.0) };
    let pix_to_v = |v: i32| -> f32 { -((v as f32 + 0.5) / (tile_res as f32) - 0.5) * (hs * 2.0) };

    let views_24 = if is_24 { get_views_24() } else { Vec::new() };

    let num_tiles = if is_24 { 24 } else { 6 };

    for ti in 0..num_tiles {
        let (tx, ty) = tile_xy(ti, cols);
        let ox = (tx * tile_res) as i32;
        let oy = (ty * tile_res) as i32;
        for py in (margin..(tile_res as i32 - margin)).step_by(s as usize) {
            for px in (margin..(tile_res as i32 - margin)).step_by(s as usize) {
                let x = ox + px;
                let y = oy + py;
                if x < 0 || y < 0 || x >= w0 || y >= h0 { continue; }
                let i = idx(x, y);
                if i >= atlas_rgba.len() { continue; }
                let g = atlas_rgba[i] as f32 / 255.0;
                if g <= depth_min { continue; }
                let d = g.clamp(0.0, 1.0);
                
                // u: horizontal on screen (Right)
                // v: vertical on screen (Up)
                let u = pix_to_u(px);
                let v = pix_to_v(py);
                
                // Depth projection: White(1.0) = Near (Surface), Black(0.0) = Far (Center).
                // We assume the object is centered.
                // d=1.0 -> Surface at bounding box face (+hs).
                // d=0.0 -> Center (0).
                // So depth coordinate = d * hs.
                let depth_offset = d * hs; 

                let p = if is_24 {
                    let view = views_24[ti];
                    // Rotate (u, v, depth_offset) by view rotation.
                    // View rotation: Azimuth around Y, Elevation around X.
                    // Local: +Z is "Towards Camera" (Normal).
                    // Wait, in 6-view code:
                    // Front (+Z face): p = (u, v, depth). So +Z is normal.
                    // So we construct point in local frame where +Z is normal.
                    let local = Vec3::new(u, v, depth_offset);
                    let rot = Quat::from_rotation_y(view.az) * Quat::from_rotation_x(-view.el);
                    rot * local
                } else {
                    let face = ATLAS_FACES[ti];
                    match face {
                        // Front (+Z): Screen X = World X, Screen Y = World Y.
                        ViewFace::Front => Vec3::new(u, v, depth_offset),
                        
                        // Back (-Z): Screen X = World -X (Back view looks at -Z, so Right on screen is Left in World).
                        ViewFace::Back => Vec3::new(-u, v, -depth_offset),
                        
                        // Left (-X): Looking at +X. Screen X = World -Z. Screen Y = World Y.
                        ViewFace::Left => Vec3::new(-depth_offset, v, -u),
                        
                        // Right (+X): Looking at -X. Screen X = World +Z. Screen Y = World Y.
                        ViewFace::Right => Vec3::new(depth_offset, v, u),
                        
                        // Top (+Y): Looking at -Y. Screen X = World X. Screen Y = World -Z.
                        ViewFace::Top => Vec3::new(u, depth_offset, -v),
                        
                        // Bottom (-Y): Looking at +Y. Screen X = World X. Screen Y = World +Z.
                        ViewFace::Bottom => Vec3::new(u, -depth_offset, v),
                    }
                };
                out.push(p);
            }
        }
    }
    out
}

#[inline]
fn div_rem_euclid(a: i32, d: i32) -> (i32, i32) { (a.div_euclid(d), a.rem_euclid(d)) }

pub(crate) fn raster_points_to_voxel_chunks(points: &[Vec3], voxel_size: f32, pi: u8) -> (HashMap<IVec3, Vec<u8>>, HashMap<IVec3, u32>) {
    let vs = voxel_size.max(0.001);
    let cs = CHUNK_SIZE.max(1);
    let csu = cs as usize;
    let cs3 = csu * csu * csu;
    let idx3 = |lx: i32, ly: i32, lz: i32| -> usize { (lz as usize) * csu * csu + (ly as usize) * csu + (lx as usize) };
    let mut chunks: HashMap<IVec3, Vec<u8>> = HashMap::new();
    let mut solid: HashMap<IVec3, u32> = HashMap::new();
    for p in points {
        let x = (p.x / vs).floor() as i32;
        let y = (p.y / vs).floor() as i32;
        let z = (p.z / vs).floor() as i32;
        let (cx, lx) = div_rem_euclid(x, cs);
        let (cy, ly) = div_rem_euclid(y, cs);
        let (cz, lz) = div_rem_euclid(z, cs);
        let ck = IVec3::new(cx, cy, cz);
        let buf = chunks.entry(ck).or_insert_with(|| vec![0u8; cs3]);
        let i = idx3(lx, ly, lz);
        if buf[i] == 0 {
            buf[i] = pi.max(1);
            *solid.entry(ck).or_insert(0) += 1;
        }
    }
    (chunks, solid)
}

pub(crate) fn splat_points_to_vdb(points: &[Vec3], voxel_size: f32, radius_vox: i32) -> VoxelGrid {
    let vs = voxel_size.max(0.001);
    let r = radius_vox.max(1);
    let bg = 1.0f32;
    let mut grid = VoxelGrid::new(vs, bg);
    for p in points {
        let cx = (p.x / vs).floor() as i32;
        let cy = (p.y / vs).floor() as i32;
        let cz = (p.z / vs).floor() as i32;
        for dz in -r..=r {
            for dy in -r..=r {
                for dx in -r..=r {
                    let d = Vec3::new(dx as f32, dy as f32, dz as f32).length();
                    if d > r as f32 + 1e-3 { continue; }
                    let val = (d - r as f32) * vs;
                    let x = cx + dx;
                    let y = cy + dy;
                    let z = cz + dz;
                    let cur = grid.get_voxel(x, y, z);
                    if val < cur {
                        grid.set_voxel(x, y, z, val);
                    }
                }
            }
        }
    }
    // Ensure chunks exist even if only background values were written (safety).
    if grid.chunks.is_empty() && !points.is_empty() {
        let p0 = points[0];
        let x = (p0.x / vs).floor() as i32;
        let y = (p0.y / vs).floor() as i32;
        let z = (p0.z / vs).floor() as i32;
        let ck = IVec3::new(x.div_euclid(CHUNK_SIZE), y.div_euclid(CHUNK_SIZE), z.div_euclid(CHUNK_SIZE));
        grid.chunks.insert(ck, Chunk::new(bg));
    }
    grid
}

#[inline]
fn get_voxel_f(grid: &VoxelGrid, p: Vec3) -> f32 {
    let v = p / grid.voxel_size;
    grid.get_voxel(v.x.floor() as i32, v.y.floor() as i32, v.z.floor() as i32)
}

pub(crate) fn render_vdb_to_atlas(
    grid: &VoxelGrid,
    tile_res: u32,
    world_size: f32,
    use_24_views: bool,
) -> Result<Vec<u8>, String> {
    let cols = if use_24_views { 8 } else { 3 };
    let rows = if use_24_views { 3 } else { 2 };
    let w = tile_res * cols;
    let h = tile_res * rows;
    let mut img = image::RgbaImage::new(w, h);
    let hs = world_size * 0.5;
    let step_size = grid.voxel_size * 0.5;
    let max_dist = world_size * 2.0;

    let views_24 = if use_24_views { get_views_24() } else { Vec::new() };
    let num_tiles = if use_24_views { 24 } else { 6 };

    for ti in 0..num_tiles {
        let (tx, ty) = tile_xy(ti, cols);
        let ox = tx * tile_res;
        let oy = ty * tile_res;
        
        // Precompute view basis for 24 views
        let (ro_base, rd_base, u_axis, v_axis) = if use_24_views {
            let view = views_24[ti];
            let rot = Quat::from_rotation_y(view.az) * Quat::from_rotation_x(-view.el);
            // Camera at +Z in local frame, looking at -Z.
            // World pos = Rot * (0, 0, max_dist).
            // Ray dir = Rot * (0, 0, -1).
            // U axis = Rot * (1, 0, 0).
            // V axis = Rot * (0, 1, 0).
            (
                rot * Vec3::new(0.0, 0.0, max_dist),
                rot * Vec3::new(0.0, 0.0, -1.0),
                rot * Vec3::X,
                rot * Vec3::Y
            )
        } else {
            (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO) // Unused
        };

        for py in 0..tile_res {
            for px in 0..tile_res {
                // u, v in [-hs, hs]
                let u = ((px as f32 + 0.5) / tile_res as f32 - 0.5) * 2.0 * hs;
                let v = -((py as f32 + 0.5) / tile_res as f32 - 0.5) * 2.0 * hs; // Y-flip
                
                let (ro, rd) = if use_24_views {
                    // Orthographic ray: Origin = Base + u*Right + v*Up. Direction = Forward.
                    (ro_base + u_axis * u + v_axis * v, rd_base)
                } else {
                    let face = ATLAS_FACES[ti];
                    match face {
                        ViewFace::Front => (Vec3::new(u, v, max_dist), Vec3::new(0.0, 0.0, -1.0)),
                        ViewFace::Back => (Vec3::new(-u, v, -max_dist), Vec3::new(0.0, 0.0, 1.0)),
                        ViewFace::Left => (Vec3::new(-max_dist, v, -u), Vec3::new(1.0, 0.0, 0.0)),
                        ViewFace::Right => (Vec3::new(max_dist, v, u), Vec3::new(-1.0, 0.0, 0.0)),
                        ViewFace::Top => (Vec3::new(u, max_dist, -v), Vec3::new(0.0, -1.0, 0.0)),
                        ViewFace::Bottom => (Vec3::new(u, -max_dist, v), Vec3::new(0.0, 1.0, 0.0)),
                    }
                };
                
                let mut t = max_dist - hs - grid.voxel_size;
                if t < 0.0 { t = 0.0; }
                let limit = max_dist + hs;
                let mut hit = false;
                let mut hit_pos = Vec3::ZERO;
                
                while t < limit {
                    let p = ro + rd * t;
                    let val = get_voxel_f(grid, p);
                    if val <= 0.0 {
                        hit = true;
                        hit_pos = p;
                        break;
                    }
                    t += step_size;
                }
                
                if hit {
                    let e = grid.voxel_size;
                    let n = Vec3::new(
                        get_voxel_f(grid, hit_pos + Vec3::X * e) - get_voxel_f(grid, hit_pos - Vec3::X * e),
                        get_voxel_f(grid, hit_pos + Vec3::Y * e) - get_voxel_f(grid, hit_pos - Vec3::Y * e),
                        get_voxel_f(grid, hit_pos + Vec3::Z * e) - get_voxel_f(grid, hit_pos - Vec3::Z * e),
                    ).normalize_or_zero();
                    
                    // Transform normal to camera space for "Normal Map" look (optional, but raw world normal is also fine for AI).
                    // Let's output World Normal mapped to 0..1.
                    let r = ((n.x * 0.5 + 0.5) * 255.0) as u8;
                    let g = ((n.y * 0.5 + 0.5) * 255.0) as u8;
                    let b = ((n.z * 0.5 + 0.5) * 255.0) as u8;
                    img.put_pixel(ox + px, oy + py, image::Rgba([r, g, b, 255]));
                } else {
                    img.put_pixel(ox + px, oy + py, image::Rgba([0, 0, 0, 255]));
                }
            }
        }
    }
    
    let mut bytes: Vec<u8> = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes).write_image(
        &img, w, h, image::ColorType::Rgba8.into()
    ).map_err(|e| e.to_string())?;
    Ok(bytes)
}

