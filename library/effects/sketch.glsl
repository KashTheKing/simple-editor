// @name Pencil Sketch
// @desc Redraws the picture as a graphite pencil sketch: dark outlines, hatching in the shadows and paper grain.
// @tags sketch, pencil, drawing, artistic, hatching, paper
// @param lines 1.0 0 2
// @param hatching 0.8 0 1.5
// @param color_amount 0.0 0 1
// @param paper 0.5 0 1

float sk_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash12(i), hash12(i + vec2(1.0, 0.0)), f.x),
               mix(hash12(i + vec2(0.0, 1.0)), hash12(i + vec2(1.0, 1.0)), f.x), f.y);
}

// one set of pencil strokes along direction a, broken up so it reads as hand drawn
float sk_hatch(vec2 p, float a, float spacing, float seed) {
    vec2 d = vec2(cos(a), sin(a));
    float along = dot(p, d);
    float across = dot(p, vec2(-d.y, d.x));
    float wob = sk_noise(vec2(along * 0.02, across * 0.05) + seed) * 2.0;
    float row = across / spacing + wob;
    float line = 1.0 - smoothstep(0.0, 0.32, abs(fract(row) - 0.5));
    float breaks = smoothstep(0.25, 0.6, sk_noise(vec2(along * 0.03, floor(row) * 3.1 + seed)));
    return line * breaks;
}

vec4 effect(vec4 src, vec2 uv) {
    vec2 px = 1.0 / u_res;
    float s = max(u_scale, 0.05);
    vec2 o = px * max(1.4 * s * 2.0, 0.9);

    // Sobel on a slightly wide footprint so lines are pencil-thick at 1080p
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
    float edge = sqrt(dot(gx, gx) + dot(gy, gy)) * 0.6;

    // a second, wider Sobel pass for the soft construction lines
    vec2 o2 = o * 2.5;
    float a1 = luma(texture(tex, uv + o2 * vec2(-1.0, 0.0)).rgb);
    float a2 = luma(texture(tex, uv + o2 * vec2(1.0, 0.0)).rgb);
    float a3 = luma(texture(tex, uv + o2 * vec2(0.0, -1.0)).rgb);
    float a4 = luma(texture(tex, uv + o2 * vec2(0.0, 1.0)).rgb);
    float edge2 = length(vec2(a2 - a1, a4 - a3));

    float l = luma(src.rgb);
    // pencil coordinates in 1080p-sized pixels
    vec2 p = uv * u_res / s;

    float ink = smoothstep(0.12, 0.55, edge) * 0.9 + smoothstep(0.08, 0.4, edge2) * 0.25;
    ink *= lines;
    ink *= 0.75 + 0.25 * sk_noise(p * 0.15);

    // hatching: lighter tones get one direction, darker tones add the cross strokes
    float tone = 1.0 - l;
    float h = 0.0;
    h += sk_hatch(p, 0.785, 7.0, 1.0) * smoothstep(0.25, 0.45, tone) * 0.55;
    h += sk_hatch(p, -0.785, 7.0, 5.0) * smoothstep(0.45, 0.65, tone) * 0.6;
    h += sk_hatch(p, 0.2, 5.0, 9.0) * smoothstep(0.65, 0.85, tone) * 0.7;
    h *= hatching;

    // paper: warm white with a fine tooth that catches the graphite
    float tooth = sk_noise(p * 0.6) * 0.6 + sk_noise(p * 0.18) * 0.4;
    vec3 paperc = vec3(0.97, 0.955, 0.92) - (tooth - 0.5) * 0.06 * paper;
    float graphite = clamp(ink + h * (0.75 + 0.5 * tooth * paper), 0.0, 1.0);
    // a soft tonal wash so large dark areas are not pure white
    float wash = smoothstep(0.35, 1.0, tone) * 0.18;
    vec3 lead = vec3(0.16, 0.16, 0.18);
    vec3 col = mix(paperc * (1.0 - wash), lead, graphite);

    // colour pencil: tint the paper and strokes with the original colour
    vec3 tinted = col * mix(vec3(1.0), src.rgb / max(l, 0.2), 0.85);
    col = mix(col, tinted * 0.5 + src.rgb * 0.5 * (1.0 - graphite * 0.6), color_amount);
    return vec4(clamp(col, 0.0, 1.0), src.a);
}
