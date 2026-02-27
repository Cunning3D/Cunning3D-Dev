#import bevy_pbr::mesh_view_bindings::view

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_pos = vec4<f32>(vertex.uv.x, vertex.uv.y, 0.0, 1.0);
    out.clip_position = view.clip_from_world * world_pos;
    out.uv = vertex.uv;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let e = max(view.exposure, 0.0001);
    let uv = in.uv;

    // UV checker + grid lines (debug-friendly and doesn't depend on vertex colors).
    let scale = 10.0;
    let cx = i32(floor(uv.x * scale));
    let cy = i32(floor(uv.y * scale));
    let checker = f32((cx + cy) & 1);
    var col = mix(vec3<f32>(0.85), vec3<f32>(0.15), checker);

    let f = fract(uv * scale);
    let d = min(min(f.x, 1.0 - f.x), min(f.y, 1.0 - f.y));
    let line = 1.0 - smoothstep(0.0, 0.02, d);
    col = mix(col, vec3<f32>(0.0), line);

    return vec4<f32>(col / e, 1.0);
}
