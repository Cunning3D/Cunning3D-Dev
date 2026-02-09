#import bevy_pbr::mesh_view_bindings::view

struct NormalUniform {
    model: mat4x4<f32>,
    color: vec4<f32>,
};

@group(2) @binding(0) var<uniform> normal_uniform: NormalUniform;

struct VertexInput {
    @location(0) position: vec2<f32>, // x: Width Offset (-0.5 to 0.5), y: Length Factor (0.0 to 1.0)
};

struct InstanceInput {
    @location(1) instance_pos: vec3<f32>,    // From Mesh Position (Loc 1)
    @location(2) instance_normal: vec3<f32>, // From Mesh Normal (Loc 2)
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vertex(
    vertex: VertexInput,
    instance: InstanceInput,
) -> VertexOutput {
    var out: VertexOutput;
    
    let model = normal_uniform.model;
    
    // Adjustable parameters
    let normal_length = 0.12; 
    let base_width = 0.002; // Width at the base of the normal (Thinner, like a line)
    
    // World Space Calculation
    let root_local = instance.instance_pos;
    let normal_local = normalize(instance.instance_normal);
    let tip_local = root_local + normal_local * normal_length;
    
    // We need to do billboarding in World Space (or View Space)
    // Let's transform to World Space first
    let root_world = (model * vec4<f32>(root_local, 1.0)).xyz;
    let tip_world = (model * vec4<f32>(tip_local, 1.0)).xyz;
    let normal_world = normalize(tip_world - root_world);
    
    // Calculate View Vector (Camera Position is needed for perfect billboarding)
    // Bevy's `view.world_position` gives camera position
    let camera_pos = view.world_position;
    let view_vec = normalize(camera_pos - root_world);
    
    // Calculate Right Vector (perpendicular to Normal and View)
    // This ensures the triangle width always faces the camera
    var right_vec = cross(view_vec, normal_world);
    
    // Handle degenerate case (view aligned with normal)
    if (length(right_vec) < 0.001) {
        right_vec = vec3<f32>(1.0, 0.0, 0.0); // Arbitrary fallback
    } else {
        right_vec = normalize(right_vec);
    }
    
    // Calculate Final Position
    // vertex.position.y is Length Factor (0=Root, 1=Tip)
    // vertex.position.x is Width Offset (-0.5 to 0.5)
    
    var final_world_pos = vec3<f32>(0.0);
    
    if (vertex.position.y > 0.9) {
        // Tip
        final_world_pos = tip_world;
    } else {
        // Base
        // Offset along the calculated Right Vector
        let offset = right_vec * vertex.position.x * base_width;
        final_world_pos = root_world + offset;
    }
    
    out.clip_position = view.clip_from_world * vec4<f32>(final_world_pos, 1.0);
    
    // Color from Uniform
    out.color = normal_uniform.color;
    
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
