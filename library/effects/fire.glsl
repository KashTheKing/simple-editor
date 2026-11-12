// @name Fire
// @desc Procedural flames lick up from the bottom of the frame, flickering yellow, orange and red over the picture.
// @tags fire, flames, burn, hot, inferno, glow, animated
// @param height 0.45 0.05 1
// @param intensity 1.0 0 2
// @param speed 1.0 0 3
// @param scale 1.0 0.3 3

float fi_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash12(i), hash12(i + vec2(1.0, 0.0)), f.x),
               mix(hash12(i + vec2(0.0, 1.0)), hash12(i + vec2(1.0, 1.0)), f.x), f.y);
}

float fi_fbm(vec2 p) {
    float v = 0.0;
    float a = 0.5;
    for (int i = 0; i < 5; i++) {
        v += a * fi_noise(p);
        p = p * 2.03 + vec2(1.7, 9.2);
        a *= 0.5;
    }
    return v;
}

vec4 effect(vec4 src, vec2 uv) {
    float aspect = u_res.x / u_res.y;
    // y up from the bottom edge, in frame heights
    vec2 p = vec2(uv.x * aspect, 1.0 - uv.y);
    float t = u_time * speed;
    vec2 q = p * vec2(5.0, 2.4) / scale;

    // domain warp makes the tongues curl; everything scrolls upward
    vec2 w = vec2(fi_fbm(q * 0.8 + vec2(0.0, -t * 1.1)), fi_fbm(q * 0.8 + vec2(5.3, -t * 1.3)));
    float n = fi_fbm(q + (w - 0.5) * 1.6 + vec2(0.0, -t * 2.4));

    // flames thin out with height; a little side-to-side sway in the reach
    float sway = 0.12 * (fi_noise(vec2(p.x * 2.0 / scale, t * 0.6)) - 0.5);
    float h = p.y / max(height * (1.0 + sway), 0.01);
    float flame = smoothstep(0.0, 0.7, n * 1.6 - h * 0.55 - 0.36) * smoothstep(1.25, 0.6, h);
    // a low glow along the bottom that feeds the flames
    float base = exp(-p.y / max(height, 0.01) * 5.0) * 0.22 * (0.4 + n);
    float heat = clamp(flame + base, 0.0, 1.0);

    // black body ramp: deep red, orange, yellow, white only in the hottest cores
    vec3 c = vec3(smoothstep(0.0, 0.45, heat),
                  smoothstep(0.2, 0.95, heat) * 0.8,
                  smoothstep(0.85, 1.0, heat) * 0.5);
    c *= intensity * 1.15;

    // flames are bright but not see-through: darken what is behind a little, then add the light
    float body = clamp(heat * 1.4, 0.0, 1.0) * clamp(intensity, 0.0, 1.0);
    vec3 col = src.rgb * (1.0 - body * 0.55) * mix(vec3(1.0), vec3(1.1, 0.9, 0.75), clamp(base * 3.0, 0.0, 1.0));
    col = 1.0 - (1.0 - col) * (1.0 - clamp(c, 0.0, 1.0));
    return vec4(clamp(col, 0.0, 1.0), src.a);
}
