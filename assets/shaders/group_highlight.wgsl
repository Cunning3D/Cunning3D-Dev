#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::mesh_functions

struct Vertex {
    @location(0) position: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
};

@vertex
fn vertex(vertex: Vertex, @builtin(instance_index) instance_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let model = mesh_functions::get_world_from_local(instance_index);
    out.clip_position = mesh_functions::mesh_position_local_to_clip(model, vec4<f32>(vertex.position, 1.0));
    return out;
}

@fragment
fn fragment() -> @location(0) vec4<f32> {
    // Debug: Solid Red/Orange, ignore exposure for now to ensure visibility
    return vec4<f32>(1.0, 0.5, 0.0, 0.05); 
}
