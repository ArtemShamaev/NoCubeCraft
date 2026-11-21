import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    mesh_view_bindings::{globals, view},
};

// Sum of travelling waves; returns the slope (d/dx, d/dz) of the surface.
fn wave_slope(p: vec2<f32>, t: f32) -> vec2<f32> {
    var slope = vec2(0.0);
    let dirs = array<vec2<f32>, 4>(vec2(0.8, 0.6), vec2(-0.5, 0.86), vec2(0.97, -0.24), vec2(-0.7, -0.71));
    let lengths = array<f32, 4>(7.0, 4.3, 2.1, 1.3);
    let heights = array<f32, 4>(0.06, 0.04, 0.018, 0.01);
    for (var i = 0; i < 4; i++) {
        let k = 6.2831 / lengths[i];
        let speed = sqrt(9.8 / k);
        let phase = k * dot(dirs[i], p) - speed * k * t;
        slope += dirs[i] * heights[i] * k * cos(phase);
    }
    return slope;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let p = in.world_position.xyz;
    let slope = wave_slope(p.xz, globals.time);
    var n = normalize(vec3(-slope.x, 1.0, -slope.y));
    if !is_front {
        n = -n;
    }
    pbr_input.N = n;

    // Looking straight down you see into the water; at a grazing angle it mirrors the sky.
    let to_eye = normalize(view.world_position - p);
    let fresnel = pow(1.0 - clamp(abs(dot(n, to_eye)), 0.0, 1.0), 4.0);
    let deep = vec3(0.03, 0.17, 0.24);
    let shallow = vec3(0.1, 0.36, 0.42);
    pbr_input.material.base_color = vec4(mix(shallow, deep, fresnel), mix(0.55, 0.92, fresnel));

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
