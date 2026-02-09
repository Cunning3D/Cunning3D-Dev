#import bevy_pbr::{
    pbr_types,
    pbr_functions::alpha_discard,
    pbr_fragment::pbr_input_from_standard_material,
    decal::clustered::apply_decals,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}
#endif

#ifdef VISIBILITY_RANGE_DITHER
#import bevy_pbr::pbr_functions::visibility_range_dither;
#endif

#ifdef MESHLET_MESH_MATERIAL_PASS
#import bevy_pbr::meshlet_visibility_buffer_resolve::resolve_vertex_output
#endif

#ifdef OIT_ENABLED
#import bevy_core_pipeline::oit::oit_draw
#endif // OIT_ENABLED

#ifdef FORWARD_DECAL
#import bevy_pbr::decal::forward::get_forward_decal_info
#endif

// Extension uniforms (match `BackfaceTintExt` bindings).
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> tint_rgba: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<uniform> mul_rgb: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<uniform> voxel_grid_params: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var<uniform> voxel_grid_color: vec4<f32>;

fn voxel_grid_mask(world_pos: vec3<f32>, world_n: vec3<f32>) -> f32 {
    // enabled gate
    if (voxel_grid_params.z <= 0.5 || voxel_grid_params.x <= 0.0) { return 0.0; }
    let vs = voxel_grid_params.x;
    // line width in pixels
    let lw_px = max(voxel_grid_params.y, 0.0);
    let p = world_pos / vs;
    let fp = fract(p);
    let d = min(fp, vec3<f32>(1.0) - fp); // distance to nearest cell boundary per axis in [0..0.5)
    let dp = fwidth(p); // cell-space delta per pixel

    // pick the face plane based on dominant normal axis
    let an = abs(world_n);
    var a: f32 = 1.0;
    var b: f32 = 1.0;
    var da: f32 = 1.0;
    var db: f32 = 1.0;
    if (an.x >= an.y && an.x >= an.z) {
        a = d.y; b = d.z;
        da = dp.y; db = dp.z;
    } else if (an.y >= an.x && an.y >= an.z) {
        a = d.x; b = d.z;
        da = dp.x; db = dp.z;
    } else {
        a = d.x; b = d.y;
        da = dp.x; db = dp.y;
    }
    let aa_a = max(da, 1e-6);
    let aa_b = max(db, 1e-6);
    // cap line width to avoid "fatter when zooming out": don't allow the line to exceed ~25% of a screen-space cell
    let cell_px = 1.0 / max(max(da, db), 1e-6);
    let lw_eff = min(lw_px, max(0.0, cell_px * 0.25));
    let t_a = lw_eff * aa_a;
    let t_b = lw_eff * aa_b;
    let m_a = 1.0 - smoothstep(t_a, t_a + aa_a, a);
    let m_b = 1.0 - smoothstep(t_b, t_b + aa_b, b);
    // fade out when grid frequency exceeds screen resolution (cell < ~2px)
    let fade = clamp((cell_px - 2.0) / 6.0, 0.0, 1.0);
    return max(m_a, m_b) * fade;
}

@fragment
fn fragment(
#ifdef MESHLET_MESH_MATERIAL_PASS
    @builtin(position) frag_coord: vec4<f32>,
#else
    vertex_output: VertexOutput,
    @builtin(front_facing) is_front: bool,
#endif
) -> FragmentOutput {
#ifdef MESHLET_MESH_MATERIAL_PASS
    let vertex_output = resolve_vertex_output(frag_coord);
    let is_front = true;
#endif

    var in = vertex_output;

#ifdef VISIBILITY_RANGE_DITHER
    visibility_range_dither(in.position, in.visibility_range_dither);
#endif

#ifdef FORWARD_DECAL
    let forward_decal_info = get_forward_decal_info(in);
    in.world_position = forward_decal_info.world_position;
    in.uv = forward_decal_info.uv;
#endif

    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // Backface tint (deferred: modify input, forward: modify output too).
    if (!is_front) {
        let s = clamp(tint_rgba.w, 0.0, 1.0);
        let rgb = mix(pbr_input.material.base_color.rgb, tint_rgba.rgb, s) * mul_rgb.rgb;
        pbr_input.material.base_color = vec4<f32>(rgb, pbr_input.material.base_color.a);
    }

    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    apply_decals(&pbr_input);

#ifdef PREPASS_PIPELINE
    // Voxel grid overlay (deferred path): mix into base_color so dark grid colors work (black lines).
    let gm = voxel_grid_mask(in.world_position.xyz, pbr_input.world_normal);
    if (gm > 0.0) {
        let a = clamp(voxel_grid_color.a, 0.0, 1.0);
        pbr_input.material.base_color =
            vec4<f32>(mix(pbr_input.material.base_color.rgb, voxel_grid_color.rgb, a), pbr_input.material.base_color.a);
    }
    return deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    if (!is_front) {
        let s = clamp(tint_rgba.w, 0.0, 1.0);
        let rgb = mix(out.color.rgb, tint_rgba.rgb, s) * mul_rgb.rgb;
        out.color = vec4<f32>(rgb, out.color.a);
    }
    // Voxel grid overlay (forward path): mix so dark grid colors work (black lines).
    let gm = voxel_grid_mask(in.world_position.xyz, pbr_input.world_normal);
    if (gm > 0.0) {
        let a = clamp(voxel_grid_color.a, 0.0, 1.0);
        out.color = vec4<f32>(mix(out.color.rgb, voxel_grid_color.rgb, a), out.color.a);
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#ifdef OIT_ENABLED
    let alpha_mode = pbr_input.material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS;
    if alpha_mode != pbr_types::STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE {
        oit_draw(in.position, out.color);
        discard;
    }
#endif // OIT_ENABLED
#ifdef FORWARD_DECAL
    out.color.a = min(forward_decal_info.alpha, out.color.a);
#endif
    return out;
#endif
}

