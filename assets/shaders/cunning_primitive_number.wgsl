#import bevy_pbr::mesh_view_bindings::view

struct NumberUniform {
    model: mat4x4<f32>,
    color: vec4<f32>,
    font_texture_size: vec2<f32>, // Size of the atlas (e.g., 256x256)
    glyph_size: vec2<f32>,        // Size of one digit (e.g., 32x64)
};

@group(1) @binding(0) var<uniform> number_uniform: NumberUniform;
@group(1) @binding(1) var font_texture: texture_2d<f32>;
@group(1) @binding(2) var font_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>, // Quad Vertex Position (0..1, 0..1)
};

struct InstanceInput {
    @location(1) world_position: vec3<f32>, // Center of the number
    @location(2) value: u32,                // The integer value to display
    @location(3) count_and_scale: vec2<f32>,// x: digit count, y: scale
    @location(4) color: vec4<f32>,          // Instance color
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

// Returns the digit at specific 0-based index (0=ones, 1=tens, etc.)
fn get_digit(value: u32, index: u32) -> u32 {
    var v = value;
    for (var i = 0u; i < index; i = i + 1u) {
        v = v / 10u;
    }
    return v % 10u;
}

@vertex
fn vertex(
    vertex: VertexInput,
    instance: InstanceInput,
    @builtin(vertex_index) vertex_idx: u32
) -> VertexOutput {
    var out: VertexOutput;

    let digit_count = u32(instance.count_and_scale.x);
    let scale = instance.count_and_scale.y;
    
    // Calculate which digit we are drawing based on vertex_index
    // We draw digits from left (most significant) to right (least significant)
    // But logically "index 0" is usually ones place. 
    // Let's assume the Draw Call is instanced per NUMBER, but we expanded geometry manually?
    // No, for "Optimal" approach without Geometry Shader, we use Instancing per Number, 
    // but we need to draw multiple quads per instance. 
    // Bevy's standard pipeline is hard to do "1 Instance = N Quads" without custom draw indirect.
    // 
    // SIMPLIFIED OPTIMAL APPROACH for Bevy Standard Instancing:
    // We assume the Mesh (Vertex Buffer 0) contains pre-built quads for MAX_DIGITS (e.g. 6 quads = 24 verts).
    // vertex.position.z can store the "digit index" (0..5).
    
    // Let's assume vertex.position.z is the digit index (0 = Leftmost/Highest, or Rightmost? Let's say 0 is Leftmost).
    let current_digit_idx = u32(vertex.position.z);
    
    // If this digit is beyond the number's actual count, cull it.
    if (current_digit_idx >= digit_count) {
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 0.0); // NaN or Zero scale to cull
        return out;
    }

    // Extract the actual digit value (0-9)
    // If digit_count is 3 (e.g. "123"), idx 0 is '1' (hundreds), idx 1 is '2', idx 2 is '3'.
    // Power of 10 index = digit_count - 1 - current_digit_idx
    let power_idx = digit_count - 1u - current_digit_idx;
    let digit_val = get_digit(instance.value, power_idx);

    // Billboarding Logic
    let world_center = instance.world_position;
    let camera_position = view.world_position;
    let view_vec = normalize(camera_position - world_center);
    let up_vec = vec3<f32>(0.0, 1.0, 0.0); // Or view.view[1].xyz for camera-aligned up
    let right_vec = normalize(cross(up_vec, view_vec));
    let billboard_up = cross(view_vec, right_vec);

    // Layout Offset (Left to Right)
    // Center the whole number block
    let total_width_px = f32(digit_count) * number_uniform.glyph_size.x * scale;
    let start_offset_px = -total_width_px * 0.5;
    let digit_offset_px = f32(current_digit_idx) * number_uniform.glyph_size.x * scale;
    
    // Quad Vertex Offset (vertex.position.xy is 0..1)
    let quad_w = number_uniform.glyph_size.x * scale;
    let quad_h = number_uniform.glyph_size.y * scale;
    
    let local_x = (vertex.position.x - 0.5) * quad_w + start_offset_px + digit_offset_px;
    let local_y = (vertex.position.y - 0.5) * quad_h;

    let world_pos = world_center 
        + right_vec * local_x 
        + billboard_up * local_y;
    // Move a tiny bit toward the camera to avoid z-fighting / reverse-z precision issues.
    // In Reverse-Z (Near=1, Far=0), larger Z is closer.
    let world_pos_biased = world_pos + view_vec * (0.005 * scale); // Move in world space first

    // Apply View Projection
    var clip_pos = view.clip_from_world * vec4<f32>(world_pos_biased, 1.0);
    
    // Depth Bias (Move slightly closer to camera to sit on top of mesh)
    // clip_pos.z = clip_pos.z + 0.00001 * clip_pos.w; // [FIX] Add to Z for Reverse-Z

    out.clip_position = clip_pos;

    // UV Calculation (Atlas assumes 0-9 horizontally or grid)
    // Simple horizontal strip: 0..9
    let uv_stride = 1.0 / 10.0;
    let uv_start = f32(digit_val) * uv_stride;
    out.uv = vec2<f32>(uv_start + vertex.position.x * uv_stride, 1.0 - vertex.position.y); // Flip Y if needed
    
    out.color = instance.color;

    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample SDF or Texture
    let alpha = textureSample(font_texture, font_sampler, in.uv).r;
    
    // [DEBUG] Bypass texture, draw RED BOX with border
    let border = 0.1;
    if (in.uv.x < border || in.uv.x > 1.0 - border || in.uv.y < border || in.uv.y > 1.0 - border) {
        return vec4<f32>(1.0, 1.0, 0.0, 1.0); // Yellow Border
    }
    
    // Simple Alpha Test/SDF Threshold
    let threshold = 0.5;
    let smooth_width = fwidth(alpha);
    let opacity = smoothstep(threshold - smooth_width, threshold + smooth_width, alpha);
    
    if (opacity < 0.1) {
        // [DEBUG] Don't discard, draw semi-transparent red for background
        return vec4<f32>(1.0, 0.0, 0.0, 0.5); 
    }

    return vec4<f32>(in.color.rgb, 1.0);
}
