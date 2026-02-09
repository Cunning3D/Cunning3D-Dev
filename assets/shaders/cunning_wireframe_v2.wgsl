#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::mesh_functions
#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils
#endif

struct Vertex { @location(0) position: vec3<f32>, }
struct VertexOutput { @builtin(position) clip_position: vec4<f32>, }

@vertex
fn vertex(vertex: Vertex, @builtin(instance_index) instance_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let model = mesh_functions::get_world_from_local(instance_index);
    out.clip_position = mesh_functions::mesh_position_local_to_clip(model, vec4<f32>(vertex.position, 1.0));
    return out;
}

@fragment
fn fragment(
#ifdef MULTISAMPLED
    @builtin(sample_index) sample_index: u32,
#endif
    in: VertexOutput
) -> @location(0) vec4<f32> {
#ifndef MULTISAMPLED
    let sample_index = 0u;
#endif

    let e = max(view.exposure, 0.0001);
    var a = 0.75;
    var rgb = vec3<f32>(0.0);

#ifdef GHOST_MODE
#ifdef DEPTH_PREPASS
    // DEBUG: if Ghost is really active, make it obviously green
    rgb = vec3<f32>(0.0, 1.0, 0.0);
    // Reverse-Z: 1.0 = near, 0.0 = far. Larger value = closer to camera.
    let geometry_depth = prepass_utils::prepass_depth(in.clip_position, sample_index);
    // Match prepass depth space (NDC z): use z/w to avoid shimmer while moving.
    let wire_depth = in.clip_position.z / max(in.clip_position.w, 0.0000001);
    // If geometry (from prepass) is closer than wire (or equal), wire is occluded -> ghost it
    if geometry_depth >= wire_depth {
        a = 0.1;
    } else {
        a = 1.0;
    }
#else
    return vec4<f32>(1.0, 0.0, 0.0, 1.0); // RED DEBUG: DEPTH_PREPASS MISSING
#endif
#endif // GHOST_MODE

    return vec4<f32>(rgb / e, a);
}
