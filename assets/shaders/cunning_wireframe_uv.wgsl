#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::mesh_functions

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
};

@vertex
fn vertex(vertex: Vertex, @builtin(instance_index) instance_index: u32) -> VertexOutput {
    var out: VertexOutput;
    // Vertex Displacement for Wireframe in UV Mode
    // Use UV as position (Z=0)
    let world_pos = vec4<f32>(vertex.uv.x, vertex.uv.y, 0.0, 1.0);
    out.clip_position = view.clip_from_world * world_pos;
    return out;
}

@fragment
fn fragment() -> @location(0) vec4<f32> {
    // Houdini-like: black ink + stable brightness under auto-exposure
    let e = max(view.exposure, 0.0001);
    let a = 0.75;
    return vec4<f32>(vec3<f32>(0.0) / e, a);
}
