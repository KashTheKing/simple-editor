// @name Lens Flare
// @desc A sunlit lens flare: glowing core, starburst rays, a chain of coloured ghosts and a rainbow halo.
// @tags flare, lens, sun, light, glow, cinematic, summer
// @param pos_x 0.78 0 1
// @param pos_y 0.2 0 1
// @param intensity 1.0 0 2
// @param rays 0.7 0 1.5
// @param ghosts 1.0 0 1.5
// @param warmth 0.5 0 1

float lf_noise(float x) {
    float i = floor(x);
    float f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    return mix(hash11(i), hash11(i + 1.0), f);
}

// one soft disc ghost with a slightly brighter rim, as real ghosts have
float lf_ghost(vec2 p, vec2 c, float r) {
    float d = length(p - c) / r;
    float disc = 1.0 - smoothstep(0.75, 1.0, d);
    float rim = smoothstep(0.6, 0.95, d) * (1.0 - smoothstep(0.95, 1.05, d));
    return disc * 0.6 + rim * 0.6;
}

vec4 effect(vec4 src, vec2 uv) {
    float aspect = u_res.x / u_res.y;
    vec2 p = (uv - 0.5) * vec2(aspect, 1.0);
    vec2 l = (vec2(pos_x, pos_y) - 0.5) * vec2(aspect, 1.0);
    vec2 dv = p - l;
    float d = length(dv);
    float shimmer = 0.92 + 0.08 * sin(u_time * 2.3) * sin(u_time * 1.37 + 1.0);

    vec3 warm = mix(vec3(0.9, 0.95, 1.0), vec3(1.0, 0.8, 0.55), warmth);

    // core: a hot centre and a wide soft glow
    vec3 col = vec3(0.0);
    col += warm * (exp(-d * 28.0) * 1.2 + exp(-d * 6.0) * 0.35 + 0.012 / (d + 0.03));

    // starburst: many thin rays with noisy lengths, slowly turning
    float a = atan(dv.y, dv.x);
    float turn = u_time * 0.05;
    float rayA = pow(lf_noise(a * 9.549 + turn * 9.549 + 20.0), 3.0);
    float rayB = pow(lf_noise(a * 22.92 - turn * 14.0 + 50.0), 6.0);
    float fall = 1.0 / (1.0 + d * 9.0);
    col += warm * (rayA * 0.55 + rayB * 0.35) * fall * fall * rays * 1.6;

    // ghosts: on the line from the light through the frame centre
    vec3 gh = vec3(0.0);
    gh += vec3(1.0, 0.55, 0.2) * lf_ghost(p, l * 0.55, 0.05) * 0.35;
    gh += vec3(0.4, 1.0, 0.5) * lf_ghost(p, l * 0.25, 0.025) * 0.4;
    gh += vec3(0.5, 0.6, 1.0) * lf_ghost(p, l * -0.15, 0.07) * 0.25;
    gh += vec3(1.0, 0.4, 0.7) * lf_ghost(p, l * -0.4, 0.035) * 0.35;
    gh += vec3(0.3, 0.8, 1.0) * lf_ghost(p, l * -0.7, 0.11) * 0.18;
    gh += vec3(1.0, 0.8, 0.3) * lf_ghost(p, l * -1.05, 0.05) * 0.3;
    gh += vec3(0.6, 0.5, 1.0) * lf_ghost(p, l * -1.3, 0.16) * 0.12;
    col += gh * ghosts;

    // halo: a faint rainbow ring around the light
    float hr = abs(d - 0.38);
    float ring = exp(-hr * hr * 900.0);
    vec3 rainbow = hsl2rgb(vec3(fract(0.02 + (d - 0.38) * 6.0), 0.9, 0.6));
    col += rainbow * ring * 0.12 * ghosts;

    col *= intensity * shimmer;
    vec3 outc = 1.0 - (1.0 - src.rgb) * (1.0 - clamp(col, 0.0, 1.0));
    return vec4(outc, src.a);
}
