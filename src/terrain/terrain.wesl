import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
};

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var color_tex: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var tex_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var normal_tex: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var rough_tex: texture_2d_array<f32>;

// Metres covered by one repeat of a texture.
const TEXTURE_SPAN: f32 = 2.0;
const DIRT_LAYER: i32 = 1;
const COAL: vec3<f32> = vec3(0.012, 0.012, 0.014);
const DIAMOND: vec3<f32> = vec3(0.25, 0.85, 0.9);
const IRON: vec3<f32> = vec3(0.45, 0.20, 0.09);

fn hash(p: vec3<i32>) -> f32 {
    var h = (bitcast<u32>(p.x) * 0x8da6b343u) ^ (bitcast<u32>(p.y) * 0xd8163841u) ^ (bitcast<u32>(p.z) * 0xcb1ab31fu);
    h = h * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h) / 4294967295.0;
}

// Value noise in [0, 1] with its analytic gradient: returns (value, d/dx, d/dy, d/dz).
fn noise_d(p: vec3<f32>) -> vec4<f32> {
    let i = vec3<i32>(floor(p));
    let w = fract(p);
    let u = w * w * (3.0 - 2.0 * w);
    let du = 6.0 * w * (1.0 - w);

    let a = hash(i);
    let b = hash(i + vec3(1, 0, 0));
    let c = hash(i + vec3(0, 1, 0));
    let d = hash(i + vec3(1, 1, 0));
    let e = hash(i + vec3(0, 0, 1));
    let f = hash(i + vec3(1, 0, 1));
    let g = hash(i + vec3(0, 1, 1));
    let h = hash(i + vec3(1, 1, 1));

    let k1 = b - a;
    let k2 = c - a;
    let k3 = e - a;
    let k4 = a - b - c + d;
    let k5 = a - c - e + g;
    let k6 = a - b - e + f;
    let k7 = -a + b + c - d + e - f - g + h;

    let value = a + k1 * u.x + k2 * u.y + k3 * u.z + k4 * u.x * u.y + k5 * u.y * u.z + k6 * u.z * u.x + k7 * u.x * u.y * u.z;
    let gradient = du * vec3(
        k1 + k4 * u.y + k6 * u.z + k7 * u.y * u.z,
        k2 + k5 * u.z + k4 * u.x + k7 * u.z * u.x,
        k3 + k6 * u.x + k5 * u.y + k7 * u.x * u.y,
    );
    return vec4(value, gradient);
}

fn noise(p: vec3<f32>) -> f32 {
    return noise_d(p).x;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let p = in.world_position.xyz;
    // The vertex normal is smoothed over a metre; the face's own flat normal comes from
    // how the position changes across the screen, turned to agree with the smooth one.
    let smoothed = normalize(in.world_normal);
    var n = normalize(cross(dpdx(p), dpdy(p)));
    if dot(n, smoothed) < 0.0 {
        n = -n;
    }
    let code = in.uv.y;
    // Skirts hide cracks between levels of detail; they should look like the ground nearby.
    if code < -0.5 {
        n = smoothed;
    }

    let q = p * 4.0;
    let pixels_per_cell = 1.0 / max(length(fwidth(q)), 1e-4);
    // Close range shows each block; far away the ground is lit by the smooth normal.
    let near = smoothstep(3.0, 10.0, pixels_per_cell);
    let far = 1.0 - smoothstep(4.0, 26.0, pixels_per_cell);
    let base_n = normalize(mix(n, smoothed, far));

    // Project the texture along the face's main axis; walls keep their texture upright.
    let a = abs(n);
    var uv: vec2<f32>;
    var tangent: vec3<f32>;
    var bitangent: vec3<f32>;
    if a.y >= a.x && a.y >= a.z {
        uv = p.xz;
        tangent = vec3(1.0, 0.0, 0.0);
        bitangent = vec3(0.0, 0.0, -1.0);
    } else if a.x >= a.z {
        uv = vec2(p.z, -p.y);
        tangent = vec3(0.0, 0.0, 1.0);
        bitangent = vec3(0.0, 1.0, 0.0);
    } else {
        uv = vec2(p.x, -p.y);
        tangent = vec3(1.0, 0.0, 0.0);
        bitangent = vec3(0.0, 1.0, 0.0);
    }
    uv = uv / TEXTURE_SPAN;
    var layer = i32(in.uv.x + 0.5);
    // Turf only on top; its sides show the soil underneath.
    if layer == 0 && n.y < 0.45 {
        layer = DIRT_LAYER;
    }

    let albedo = textureSample(color_tex, tex_sampler, uv, layer).rgb;
    let tex_n = textureSample(normal_tex, tex_sampler, uv, layer).xyz * 2.0 - 1.0;
    let rough_ao = textureSample(rough_tex, tex_sampler, uv, layer).rg;

    // A hint of which block is which, and broad patches so big areas don't repeat visibly.
    let id = vec3<i32>(floor(q - n * 0.5));
    let tint = hash(id);
    let patches = noise(p * 0.11);
    let ao = mix(in.color.a, 1.0, far * 0.85);
    var color = albedo * in.color.rgb * ao * (0.86 + 0.24 * patches) * mix(1.0, 0.93 + 0.14 * tint, near);

    if code > 0.5 {
        let fleck = step(0.6, noise(p * 6.0));
        let ore = select(select(COAL, IRON, code > 1.5), DIAMOND, code > 2.5);
        color = mix(color, ore, fleck * 0.85);
    }
    pbr_input.material.base_color = vec4(color, 1.0);
    pbr_input.material.perceptual_roughness = mix(rough_ao.r, 0.9, far);
    pbr_input.diffuse_occlusion = vec3(mix(rough_ao.g, 1.0, far));

    // Bevelled cell edges: near an edge the surface turns towards it, catching the light.
    let f = fract(q) - 0.5;
    let w = 0.5 - 0.09;
    let bevel_t = smoothstep(w, 0.5, abs(dot(f, tangent))) * sign(dot(f, tangent));
    let bevel_b = smoothstep(w, 0.5, abs(dot(f, abs(bitangent)))) * sign(dot(f, abs(bitangent)));
    let bumped = tangent * (tex_n.x + bevel_t * 0.7 * near) + bitangent * tex_n.y + abs(bitangent) * bevel_b * 0.7 * near;
    let detail = smoothstep(1.0, 6.0, pixels_per_cell);
    pbr_input.N = normalize(base_n + bumped * detail * 0.9);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
