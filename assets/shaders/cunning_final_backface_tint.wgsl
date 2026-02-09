#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
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
}
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var<uniform> tint_rgba: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101)
var<uniform> mul_rgb: vec4<f32>;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // Deferred: only safe to edit PbrInput (lighting happens elsewhere).
    if (!is_front) {
        let s = clamp(tint_rgba.w, 0.0, 1.0);
        let rgb = mix(pbr_input.material.base_color.rgb, tint_rgba.rgb, s) * mul_rgb.rgb;
        pbr_input.material.base_color = vec4<f32>(rgb, pbr_input.material.base_color.a);
    }

    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

#ifdef PREPASS_PIPELINE
    return deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    // Forward: also tint the lit result (preserves your “deep blue backface” look).
    if (!is_front) {
        let s = clamp(tint_rgba.w, 0.0, 1.0);
        let rgb = mix(out.color.rgb, tint_rgba.rgb, s) * mul_rgb.rgb;
        out.color = vec4<f32>(rgb, out.color.a);
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
#endif
}

