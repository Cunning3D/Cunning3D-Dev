#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types,
}

struct SdfUniform {
    model: mat4x4<f32>,
    color: vec4<f32>,
    // x=mode (0/1), y=roughness, z=rim_strength, w=cavity_strength
    params0: vec4<f32>,
    // x=rim_power, y=clay_spec, z/w reserved
    params1: vec4<f32>,
};

@group(2) @binding(0) var<uniform> u: SdfUniform;

struct VertexInput {
    @location(0) pos: vec4<f32>,
    @location(1) normal: vec4<f32>,
};

@vertex
fn vertex(v: VertexInput) -> VertexOutput {
    var o: VertexOutput;

    let world_pos = u.model * vec4<f32>(v.pos.xyz, 1.0);
    o.position = view.clip_from_world * world_pos;
    o.world_position = world_pos;

    // NOTE: for non-uniform scale, this should be inverse-transpose; good enough for now.
    o.world_normal = normalize((u.model * vec4<f32>(v.normal.xyz, 0.0)).xyz);

#ifdef VERTEX_UVS_A
    o.uv = vec2<f32>(0.0);
#endif
#ifdef VERTEX_UVS_B
    o.uv_b = vec2<f32>(0.0);
#endif
#ifdef VERTEX_TANGENTS
    o.world_tangent = vec4<f32>(0.0, 0.0, 0.0, 1.0);
#endif
#ifdef VERTEX_COLORS
    o.color = u.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    o.instance_index = 0u;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    o.visibility_range_dither = 0;
#endif

    return o;
}

fn vdb_cavity_from_normal(n: vec3<f32>) -> f32 {
    // Cheap "cavity"/curvature proxy: higher when the normal varies rapidly across pixels.
    return clamp(length(fwidth(n)) * 2.0, 0.0, 1.0);
}

fn clay_shade(
    n_ws: vec3<f32>,
    base_rgb: vec3<f32>,
    roughness: f32,
    spec_boost: f32,
) -> vec3<f32> {
    // Camera-stable shading: operate in view-space so it behaves like a MatCap/clay look.
    let n_vs = normalize((view.view_from_world * vec4<f32>(n_ws, 0.0)).xyz);

    // A "studio" headlight that always follows the camera.
    let l_vs = normalize(vec3<f32>(0.15, 0.55, 1.00));
    let ndl = dot(n_vs, l_vs);
    let wrap = 0.35;
    let diff = clamp((ndl + wrap) / (1.0 + wrap), 0.0, 1.0);

    let v_vs = vec3<f32>(0.0, 0.0, 1.0);
    let h_vs = normalize(l_vs + v_vs);
    let ndh = max(dot(n_vs, h_vs), 0.0);
    let shininess = mix(220.0, 18.0, clamp(roughness, 0.0, 1.0));
    let spec = pow(ndh, shininess) * (0.04 + 0.22 * (1.0 - roughness)) * spec_boost;

    // Subtle vertical studio gradient (brighter from above).
    let studio = mix(0.90, 1.07, clamp(n_vs.y * 0.5 + 0.5, 0.0, 1.0));
    let shadow_tint = vec3<f32>(1.0, 0.985, 0.97);
    let ambient = 0.22;

    return (base_rgb * shadow_tint) * (ambient + (1.0 - ambient) * diff) * studio + vec3<f32>(spec);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr = pbr_types::pbr_input_new();
    pbr.frag_coord = in.position;
    pbr.world_position = in.world_position;

    var n = normalize(in.world_normal);
    if (!is_front) { n = -n; }

    pbr.world_normal = n;
    pbr.N = n;
    pbr.V = normalize(view.world_position - in.world_position.xyz);
    pbr.is_orthographic = (view.clip_from_view[3][3] == 1.0);

    pbr.material = pbr_types::standard_material_new();
    let mode = u.params0.x;
    let roughness = clamp(u.params0.y, 0.0, 1.0);
    let rim_strength = clamp(u.params0.z, 0.0, 1.0);
    let cavity_strength = clamp(u.params0.w, 0.0, 1.0);
    let rim_power = max(u.params1.x, 0.0);
    let clay_spec = max(u.params1.y, 0.0);

    pbr.material.base_color = u.color;
    pbr.material.metallic = 0.0;
    pbr.material.perceptual_roughness = roughness;
    pbr.material.reflectance = vec3<f32>(0.45);
    pbr.material.flags =
        pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE |
        pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT |
        pbr_types::STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;

    var out: FragmentOutput;
    if (mode < 0.5) {
        out.color = apply_pbr_lighting(pbr);
    } else {
        out.color = vec4<f32>(clay_shade(n, u.color.rgb, roughness, clay_spec), u.color.a);
    }

    // "VDB-ish" shading accents: subtle rim + cavity (still physically plausible-ish).
    let ndv = clamp(dot(n, pbr.V), 0.0, 1.0);
    let rim = pow(1.0 - ndv, max(rim_power, 3.0));
    let cavity = vdb_cavity_from_normal(n);

    var rgb = out.color.rgb;
    rgb *= 1.0 - cavity_strength * cavity;
    rgb += rim * vec3<f32>(0.08 * rim_strength);
    out.color = vec4<f32>(rgb, out.color.a);

    out.color = main_pass_post_lighting_processing(pbr, out.color);
    return out;
}
