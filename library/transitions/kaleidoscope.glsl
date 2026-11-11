// @name Kaleidoscope
// @desc The picture folds into a turning kaleidoscope at the midpoint and unfolds onto the incoming picture.
// @tags kaleidoscope, mirror, symmetry, psychedelic, trippy
// @param segments 6 2 16
// @param turns 0.5 -2 2
// @param zoom 1.25 0.5 3
// @param glow 0.3 0 1

vec2 ks_mirror(vec2 u) {
    vec2 m = mod(u, 2.0);
    return 1.0 - abs(1.0 - m);
}

// kaleidoscope coordinate for the aspect-corrected offset d from the centre
vec2 ks_fold(vec2 d, float seg, float rot, float zm) {
    float r = length(d);
    float a = atan(d.y, d.x) + rot;
    float w = 6.28318531 / seg;
    a = mod(a, w);
    a = abs(a - 0.5 * w);            // mirror every wedge so the edges meet cleanly
    a += -1.9 - 0.25 * w;            // the wedge looks down and across the frame, where the content is
    return vec2(cos(a), sin(a)) * r * zm;
}

vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);

    float s = progress;
    float seg = max(floor(segments + 0.5), 2.0);
    vec2 asp = vec2(ratio, 1.0);
    vec2 d = (uv - 0.5) * asp;

    // how folded: 0 at the ends, 1 through the middle
    float k = smoothstep(0.02, 0.4, s) * (1.0 - smoothstep(0.6, 0.98, s));
    float rot = turns * 6.28318531 * (s - 0.5);
    float zm = mix(1.0, zoom, k);

    // the wedge is filled from a spot slightly off-centre so the pattern has content
    vec2 kd = ks_fold(d, seg, rot, zm) + vec2(0.06, 0.1);
    vec2 kuv = ks_mirror(kd / asp + 0.5);
    // fold in: blend the straight picture toward the kaleidoscope coordinates
    vec2 suv = ks_mirror(mix(uv, kuv, k));

    float m = smoothstep(0.4, 0.6, s);
    vec4 col = mix(getFromColor(suv), getToColor(suv), m);

    // a soft bloom at the hub and faint bright seams where the mirrors meet
    float a = mod(atan(d.y, d.x) + rot, 6.28318531 / seg);
    float seam = 1.0 - smoothstep(0.0, 0.02 + 1.5 / (u_res.y * max(length(d), 0.02)), min(a, 6.28318531 / seg - a));
    float hub = exp(-dot(d, d) * 18.0);
    col.rgb += (vec3(1.0, 0.95, 0.85) * hub * 0.5 + vec3(1.0) * seam * 0.12) * glow * k * k;
    return clamp(col, 0.0, 1.0);
}
