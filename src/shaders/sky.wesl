import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::view;

struct Sky {
    zenith: vec4<f32>,
    horizon: vec4<f32>,
    // xyz: towards the sun; w: daylight 0..1.
    sun: vec4<f32>,
    // rgb: sunlight colour; w: dusk 0..1.
    sun_color: vec4<f32>,
    // x: seconds, for drifting clouds.
    misc: vec4<f32>,
    // rgb: cave fog; w: how far underground the eye is. Any sky glimpsed from a cave, through
    // rock that isn't drawn, is lost in the fog like everything else.
    cave: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> sky: Sky;

fn hash2(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2(123.34, 456.21));
    let r = q + dot(q, q + 45.32);
    return fract(r.x * r.y);
}

fn hash3(p: vec3<f32>) -> f32 {
    return hash2(p.xy + vec2(p.z * 17.13, p.z * 3.71));
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2(1.0, 0.0));
    let c = hash2(i + vec2(0.0, 1.0));
    let d = hash2(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 3; i++) {
        v += a * noise(q);
        q = q * 2.03 + vec2(1.7, 9.2);
        a *= 0.5;
    }
    return v;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let dir = normalize(in.world_position.xyz - view.world_position);
    let up = dir.y;
    let day = sky.sun.w;
    let dusk = sky.sun_color.w;
    let night = 1.0 - day;

    // Horizon to zenith, and a slightly darker band below the horizon.
    var color = mix(sky.horizon.rgb, sky.zenith.rgb, pow(clamp(up, 0.0, 1.0), 0.5));
    color = mix(color, sky.horizon.rgb * 0.7, clamp(-up * 3.0, 0.0, 1.0));

    // Sun: a hard disk plus a wide glow that grows warm at dusk.
    let s = max(dot(dir, sky.sun.xyz), 0.0);
    let disk = smoothstep(0.99955, 0.9998, s);
    let glow = pow(s, 48.0) * 0.45 + pow(s, 6.0) * (0.12 + 0.35 * dusk);
    color += sky.sun_color.rgb * (disk * 3.0 + glow) * clamp(day * 1.5, 0.0, 1.0) * step(-0.05, sky.sun.y + 0.05);

    // Moon opposite the sun, and stars once it is dark.
    let m = max(dot(dir, -sky.sun.xyz), 0.0);
    color += vec3(0.8, 0.85, 1.0) * (smoothstep(0.9993, 0.9996, m) * 0.9 + pow(m, 200.0) * 0.15) * night;
    if night > 0.02 && up > 0.0 {
        let cell = floor(dir * 260.0);
        let h = hash3(cell);
        let twinkle = 0.6 + 0.4 * sin(sky.misc.x * 3.0 + h * 50.0);
        color += vec3(step(0.9975, h) * night * twinkle * smoothstep(0.0, 0.15, up));
    }

    // Clouds on a flat layer high above, drifting with the wind.
    if up > 0.0 {
        let p = dir.xz / (up + 0.08) * 1.4 + vec2(sky.misc.x * 0.006, sky.misc.x * 0.002);
        let c = fbm(p);
        let cover = smoothstep(0.5, 0.78, c) * smoothstep(0.0, 0.25, up);
        let thickness = smoothstep(0.5, 0.95, c);
        let lit = mix(vec3(0.07, 0.08, 0.12), vec3(1.0), day) * mix(vec3(1.0), vec3(1.0, 0.7, 0.5), dusk);
        let cloud = lit * (1.0 - 0.35 * thickness) + sky.sun_color.rgb * pow(s, 8.0) * 0.4 * day;
        color = mix(color, cloud, cover * 0.85);
    }
    return vec4(mix(color, sky.cave.rgb, sky.cave.w), 1.0);
}
