// @name Cartoon
// @desc Cel-shaded cartoon look: smooth flat colours in a few bright tones with bold ink outlines.
// @tags cartoon, cel, toon, comic, outline, anime, posterize
// @param levels 4 2 8
// @param outline 1.0 0 2
// @param saturation 1.3 0 2
// @param smoothing 1.0 0 2

vec4 effect(vec4 src, vec2 uv) {
    vec2 px = 1.0 / u_res;
    float s = max(u_scale, 0.05);

    // edge-preserving smoothing: 24 taps on a golden-angle spiral, weighted by colour similarity
    float rad = max(6.0 * smoothing * s, 0.5);
    vec3 acc = src.rgb;
    float wsum = 1.0;
    for (int i = 0; i < 24; i++) {
        float fi = float(i) + 0.5;
        float r = rad * sqrt(fi / 24.0);
        float a = fi * 2.39996;
        vec3 c = texture(tex, uv + vec2(cos(a), sin(a)) * r * px).rgb;
        vec3 d = c - src.rgb;
        float w = exp(-dot(d, d) * 40.0);
        acc += c * w;
        wsum += w;
    }
    vec3 col = acc / wsum;

    // cel bands on lightness with a short soft step so the bands do not alias
    vec3 hsl = rgb2hsl(col);
    float n = max(floor(levels + 0.5), 2.0);
    float x = hsl.z * n;
    float f = fract(x);
    float w = max(fwidth(x), 0.02) * 1.5;
    float q = (floor(x) + 0.42 + smoothstep(1.0 - w, 1.0, f)) / n;
    q = mix(q, hsl.z, 0.25);
    hsl.z = clamp(q, 0.03, 0.97);
    hsl.y = clamp(hsl.y * saturation, 0.0, 1.0);
    col = hsl2rgb(hsl);

    // ink outlines: Sobel on colour, about 2 px wide at 1080p
    vec2 o = px * max(1.6 * s * 2.0, 0.85);
    vec3 tl = texture(tex, uv + o * vec2(-1.0, -1.0)).rgb;
    vec3 tc = texture(tex, uv + o * vec2(0.0, -1.0)).rgb;
    vec3 tr = texture(tex, uv + o * vec2(1.0, -1.0)).rgb;
    vec3 ml = texture(tex, uv + o * vec2(-1.0, 0.0)).rgb;
    vec3 mr = texture(tex, uv + o * vec2(1.0, 0.0)).rgb;
    vec3 bl = texture(tex, uv + o * vec2(-1.0, 1.0)).rgb;
    vec3 bc = texture(tex, uv + o * vec2(0.0, 1.0)).rgb;
    vec3 br = texture(tex, uv + o * vec2(1.0, 1.0)).rgb;
    vec3 gx = (tr + 2.0 * mr + br) - (tl + 2.0 * ml + bl);
    vec3 gy = (bl + 2.0 * bc + br) - (tl + 2.0 * tc + tr);
    float edge = sqrt(dot(gx, gx) + dot(gy, gy));
    float ink = smoothstep(0.25, 0.7, edge) * clamp(outline, 0.0, 1.0);
    ink = clamp(ink * max(outline, 1.0), 0.0, 1.0);

    vec3 inkc = col * 0.15 + vec3(0.02, 0.02, 0.04);
    col = mix(col, inkc, ink);
    return vec4(col, src.a);
}
