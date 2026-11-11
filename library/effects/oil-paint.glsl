// @name Oil Paint
// @desc Turns the picture into thick oil paint: flat dabs of colour with crisp edges, brush bristles and canvas.
// @tags painting, oil, artistic, kuwahara, canvas, painterly
// @param radius 10 2 24
// @param sharpness 6 1 12
// @param brush 0.6 0 1.5
// @param canvas 0.4 0 1

float op_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// brush-stroke height field: overlapping dabs, each with its own direction and bristle streaks
float op_strokes(vec2 p, float size) {
    vec2 g = p / size;
    vec2 base = floor(g);
    float h = 0.0;
    float ws = 0.0;
    for (int j = 0; j < 4; j++) {
        vec2 cell = base + vec2(float(j - (j / 2) * 2), float(j / 2));
        vec2 ctr = cell + 0.5 * vec2(hash12(cell * 1.7), hash12(cell * 2.3)) + 0.25;
        float a = hash12(cell + 11.0) * 3.14159;
        vec2 d = (g - ctr) * size;
        vec2 q = vec2(cos(a) * d.x + sin(a) * d.y, -sin(a) * d.x + cos(a) * d.y);
        float bristle = op_noise(vec2(q.x * 0.035, q.y * 0.16) + cell * 7.0);
        float w = exp(-dot(g - ctr, g - ctr) * 3.0) + 1e-4;
        h += bristle * w;
        ws += w;
    }
    return h / ws;
}

vec4 effect(vec4 src, vec2 uv) {
    vec2 px = 1.0 / u_res;
    float r = max(radius * u_scale, 0.75);
    // 8 sectors x 8 taps: generalised Kuwahara, each sector weighted by how flat it is
    vec3 accum = vec3(0.0);
    float wsum = 0.0;
    for (int k = 0; k < 8; k++) {
        vec3 m = src.rgb;
        vec3 m2 = src.rgb * src.rgb;
        for (int j = 0; j < 8; j++) {
            float ring = float(j / 2);
            float side = float(j - (j / 2) * 2);
            float rr = r * sqrt((ring + 0.6) / 4.0);
            float a = (float(k) + 0.25 + 0.5 * side + 0.15 * (ring - 1.5)) * 0.7853982;
            vec3 c = texture(tex, uv + vec2(cos(a), sin(a)) * rr * px).rgb;
            m += c;
            m2 += c * c;
        }
        m /= 9.0;
        vec3 v = abs(m2 / 9.0 - m * m);
        float var = v.r + v.g + v.b;
        float w = 1.0 / (1.0 + pow(var * 600.0, sharpness * 0.5));
        accum += m * w;
        wsum += w;
    }
    vec3 col = accum / max(wsum, 1e-5);

    // p is in 1080p-sized pixels so the brush looks the same in preview and export
    vec2 p = uv * u_res / max(u_scale, 0.05);
    float h = op_strokes(p, 44.0);
    float hx = dFdx(h) * max(u_scale, 0.05);
    float hy = dFdy(h) * max(u_scale, 0.05);
    // slope per 1080p pixel, lit from the top left
    float lit = clamp((-hx + hy) * 10.0, -1.0, 1.0);
    float weave = sin(p.x * 0.9) * sin(p.y * 0.9);
    float grain = op_noise(p * 0.22) - 0.5;

    col *= 1.0 + lit * brush * 0.16 + (h - 0.5) * brush * 0.10;
    col += (weave * 0.03 + grain * 0.04) * canvas;
    // oil paint is a touch richer than the source
    float l = luma(col);
    col = mix(vec3(l), col, 1.12);
    return vec4(clamp(col, 0.0, 1.0), src.a);
}
