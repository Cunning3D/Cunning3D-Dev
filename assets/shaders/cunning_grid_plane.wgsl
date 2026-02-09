#import bevy_pbr::forward_io::VertexOutput

// Material uniform (set from Rust via AsBindGroup)
// Packed to keep alignment simple.
struct GridU {
    center: vec2<f32>,
    base_step: f32,
    next_step: f32,
    blend: f32,
    minor_alpha: f32,
    major_alpha: f32,
    axis_alpha: f32,
    _pad0: f32,
    minor_rgb: vec3<f32>,
    _pad1: f32,
    major_rgb: vec3<f32>,
    _pad2: f32,
    axis_rgb: vec3<f32>,
    _pad3: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> u: GridU;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var font_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var font_sampler: sampler;

fn smoothstep01(x: f32) -> f32 {
    let t = clamp(x, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Robust pixel-coverage anti-aliasing.
// If thickness < 1 pixel (aa), we keep width at 1 pixel but fade alpha.
fn pixel_coverage(d: f32, thickness: f32) -> f32 {
    let aa = fwidth(d);
    let w = max(thickness, aa);
    let alpha = min(1.0, thickness / max(aa, 1e-6));
    return alpha * (1.0 - smoothstep(w - aa, w + aa, d));
}

// Sample the font texture
fn sample_digit(p: vec2<f32>, d: i32) -> f32 {
    // p is [-0.5..0.5]. Remap to [0..1]
    // If outside, return 0
    if (abs(p.x) > 0.5 || abs(p.y) > 0.5) { return 0.0; }
    
    // UV within the cell
    let uv = p + 0.5; // [0..1]
    
    // Map char 'd' to cell index
    // 0-9: digits
    // 10: -
    // 11: .
    var idx = d;
    if (d < 0) { idx = 10; } // Assuming caller passes -1 for minus? Or specific ID.
    
    // Grid: 4 cols, 3 rows.
    let col = f32(idx % 4);
    let row = f32(idx / 4);
    
    // UV scale = 1/4, 1/3
    let cell_w = 0.25;
    let cell_h = 0.333333;
    
    // Final UV
    let final_uv = vec2<f32>(
        (col + uv.x) * cell_w,
        (row + uv.y) * cell_h // Y is usually down in texture, Bevy/WGPU 0,0 top-left? 
        // Font generation code put Y=0 at top. WGPU UV (0,0) is top-left. So this matches.
    );
    
    return textureSample(font_texture, font_sampler, final_uv).r;
}

fn label_alpha_at(p: vec2<f32>, value: i32, color_sel: f32) -> vec4<f32> {
    // color_sel: 0 = X axis (red), 1 = Z axis (blue)
    // Layout: optional '-' + up to 4 digits (right-aligned)
    var v = value;
    var sign = 0;
    if (v < 0) { sign = 1; v = -v; }
    if (v > 9999) { return vec4<f32>(0.0); }

    let d0 = v % 10;
    let d1 = (v / 10) % 10;
    let d2 = (v / 100) % 10;
    let d3 = (v / 1000) % 10;
    let has3 = v >= 1000;
    let has2 = v >= 100;
    let has1 = v >= 10;

    // local label space: each glyph in 1x1 box, p in meters -> we map in caller
    // Here p is already glyph-local.
    var a: f32 = 0.0;
    // Sample texture instead of SDF
    a = max(a, sample_digit(p - vec2<f32>( 1.5, 0.0), d0));
    if (has1) { a = max(a, sample_digit(p - vec2<f32>( 0.5, 0.0), d1)); }
    if (has2) { a = max(a, sample_digit(p - vec2<f32>(-0.5, 0.0), d2)); }
    if (has3) { a = max(a, sample_digit(p - vec2<f32>(-1.5, 0.0), d3)); }
    // Pass -1 for minus sign
    if (sign == 1) { a = max(a, sample_digit(p - vec2<f32>(-2.5, 0.0), -1)); }

    // Colors similar to Houdini screenshot
    let rgb = select(vec3<f32>(0.86, 0.45, 0.45), vec3<f32>(0.47, 0.67, 0.95), color_sel > 0.5);
    // Alpha boost for readability since texture might be soft
    let alpha = smoothstep(0.1, 0.6, a) * 0.95; 
    return vec4<f32>(rgb, alpha);
}

// Anti-aliased grid line mask in [0..1], 1 = line
fn grid_mask(p: vec2<f32>, step: f32, thickness: f32) -> f32 {
    let s = max(step, 1e-6);
    let coord = p / s;
    let cell = abs(fract(coord - 0.5) - 0.5);
    let d = min(cell.x, cell.y);
    return pixel_coverage(d, thickness);
}

fn axis_mask(v: f32, step: f32, thickness: f32) -> f32 {
    let s = max(step, 1e-6);
    let d = abs(v) / s;
    return pixel_coverage(d, thickness);
}

// Alpha blend label over grid (avoid hard replace / popping)
fn blend_over(dst: vec4<f32>, src: vec4<f32>) -> vec4<f32> {
    let a = clamp(src.a, 0.0, 1.0);
    let rgb = mix(dst.rgb, src.rgb, a);
    return vec4<f32>(rgb, max(dst.a, a));
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let wp = mesh.world_position.xyz;
    let p = wp.xz - u.center;

    // Two-level blend (Houdini style): A=(base/5, base), B=(next/5, next)
    let base = max(u.base_step, 1e-6);
    let next = max(u.next_step, base * 1.001);
    let t = smoothstep01(u.blend);

    let a_minor_step = base / 5.0;
    let a_major_step = base;
    let b_minor_step = next / 5.0; // == base
    let b_major_step = next;


    // Thickness in cell-space: smaller = thinner
    // Houdini style: very thin, precise lines
    let minor_th = 0.001;
    let major_th = 0.0015;
    let axis_th = 0.002;

    let a_minor = grid_mask(p, a_minor_step, minor_th) * u.minor_alpha;
    let a_major = grid_mask(p, a_major_step, major_th) * u.major_alpha;
    let b_minor = grid_mask(p, b_minor_step, minor_th) * u.minor_alpha;
    let b_major = grid_mask(p, b_major_step, major_th) * u.major_alpha;

    let minor_a = mix(a_minor, b_minor, t);
    let major_a = mix(a_major, b_major, t);

    // Axes: subtle highlight, monochrome like Houdini grid lines
    let ax = axis_mask(wp.x - u.center.x, base, axis_th);
    let az = axis_mask(wp.z - u.center.y, base, axis_th);
    let axis_a = max(ax, az) * u.axis_alpha;

    // Distance fade disabled for debugging visibility (we'll re-enable + tune once confirmed visible).
    let fade_minor = 1.0;
    let fade_major = 1.0;

    let c_minor = vec4<f32>(u.minor_rgb, minor_a * fade_minor);
    let c_major = vec4<f32>(u.major_rgb, major_a * fade_major);
    let c_axis  = vec4<f32>(u.axis_rgb,  axis_a  * fade_major);

    // Porter-Duff style: just pick max alpha (grid look), but keep color consistent
    var out = c_minor;
    if (c_major.a > out.a) { out = c_major; }
    if (c_axis.a > out.a) { out = c_axis; }

    // --- Axis labels "printed" on the grid plane (texture-like) ---
    if (u._pad0 > 0.5) {
        // Label spacing: show a number every 5 minor cells (Houdini-like).
        // Here: minor_step = base/5, so 5 minor cells == base.
        // User request: 5 cells per number => every 5 * base.
        let step = base;
        let label_step = step * 5.0;
        let band = step * 0.18; // how far from axis line
        // Each label glyph cell size in world meters (relative to step)
        // Slightly larger labels for readability (texture atlas is now higher-res)
        let glyph_m = step * 0.24;
        let inv = 1.0 / max(glyph_m, 1e-6);

        // X-axis labels (along Z == center.z)
        if (abs(p.y) < band) {
            let k = i32(round(p.x / label_step));
            let n = k * 5;
            if (abs(f32(n)) <= 9999.0 && n != 0) {
                // Only draw near the tick center (prevents repeated "11111" across the band)
                let dx = p.x - f32(k) * label_step;
                if (abs(dx) < glyph_m * 0.9) {
                    let local = vec2<f32>(dx * inv, p.y * inv);
                // local glyph space: center at 0, scale to [-0.5..0.5]
                let gl = local * 0.5;
                // Only draw near the label center
                if (abs(gl.x) < 3.5 && abs(gl.y) < 0.8) {
                    let lab = label_alpha_at(gl, n, 0.0);
                    out = blend_over(out, lab);
                }
                }
            }
        }

        // Z-axis labels (along X == center.x)
        if (abs(p.x) < band) {
            let k = i32(round(p.y / label_step));
            let n = k * 5;
            if (n != 0 && abs(f32(n)) <= 9999.0) {
                let dy = p.y - f32(k) * label_step;
                if (abs(dy) < glyph_m * 0.9) {
                    // Swap coords so digits align with Z axis
                    let local = vec2<f32>(dy * inv, p.x * inv);
                let gl = local * 0.5;
                if (abs(gl.x) < 3.5 && abs(gl.y) < 0.8) {
                    let lab = label_alpha_at(gl, n, 1.0);
                    out = blend_over(out, lab);
                }
                }
            }
        }
    }
    return out;
}


