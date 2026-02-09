#import bevy_pbr::mesh_view_bindings::view

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(5) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    // Use UV coordinates as world position (Vertex Displacement)
    let world_pos = vec4<f32>(vertex.uv.x, vertex.uv.y, 0.0, 1.0);
    // Ignore model matrix, use ViewProjection directly
    out.clip_position = view.clip_from_world * world_pos;
    out.color = vertex.color;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Return vertex color (brightness boosted slightly for visibility if needed, or raw)
    return in.color;
}
