#import bevy_pbr::mesh_view_bindings::view

struct PointUniform {
    model: mat4x4<f32>,
};

@group(2) @binding(0) var<uniform> point_uniform: PointUniform;

struct VertexInput {
    @location(0) position: vec3<f32>, // Quad position (-1..1)
    @location(1) uv: vec2<f32>,       // Quad UV
};

struct InstanceInput {
    @location(2) instance_pos: vec3<f32>, // From Main Mesh
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vertex(
    vertex: VertexInput,
    instance: InstanceInput,
) -> VertexOutput {
    var out: VertexOutput;
    
    // Use our custom Uniform for the Entity Transform
    let model = point_uniform.model;
    
    // Calculate World Position of the Point (Instance Center)
    // model maps Local -> World. instance_pos is in Local Space.
    let world_center = model * vec4<f32>(instance.instance_pos, 1.0);
    
    // Use combined ViewProj matrix for consistency
    var clip_center = view.clip_from_world * world_center;
    
    // Screen Space Sizing (NDC)
    // Debug: Smaller size to verify center alignment
    let point_radius_px = 3.3; 
    let screen_height = view.viewport.w;
    let screen_width = view.viewport.z;
    
    // NDC Height is 2.0 (-1 to 1).
    let scale_y = (point_radius_px / screen_height) * 2.0 * clip_center.w;
    let scale_x = (point_radius_px / screen_width) * 2.0 * clip_center.w;
    
    // Debug: Hardcoded scale for WebGL - REMOVED
    // let scale_y = 0.02 * clip_center.w;
    // let scale_x = 0.02 * clip_center.w;
    
    // Apply Offset in Clip Space directly
    let offset_clip = vec4<f32>(
        vertex.position.x * scale_x,
        vertex.position.y * scale_y,
        0.0, 
        0.0
    );
    
    out.clip_position = clip_center + offset_clip;
    
    out.uv = vertex.uv;
    // Deep Blue (User requested match)
    out.color = vec4<f32>(0.0, 0.3, 0.7, 1.0); 
    
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Optional: Circular points
    let dist = length(in.uv - vec2<f32>(0.5));
    if (dist > 0.5) {
        discard;
    }
    
    return in.color;
}
