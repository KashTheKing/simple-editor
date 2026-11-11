// @name Crumple
// @desc The outgoing picture crumples into a faceted paper ball that drops away, revealing the incoming one.
// @tags paper, crumple, ball, fold, trash
// @param facets 6 2 14
// @param fold 1 0 2
// @param ball_size 0.24 0.1 0.45
// @param spin 1 -3 3

vec2 cr_h2(vec2 i) {
    return vec2(hash12(i), hash12(i + vec2(17.31, 41.7)));
}

// returns (F1, F2) and the id of the nearest cell
vec2 cr_voronoi(vec2 x, out vec2 cellId, out vec2 cellPos) {
    vec2 n = floor(x);
    vec2 f = fract(x);
    float f1 = 8.0;
    float f2 = 8.0;
    cellId = n;
    cellPos = n;
    for (int j = -1; j <= 1; j++) {
        for (int i = -1; i <= 1; i++) {
            vec2 g = vec2(float(i), float(j));
            vec2 o = 0.15 + 0.7 * cr_h2(n + g);
            vec2 r = g + o - f;
            float d = dot(r, r);
            if (d < f1) {
                f2 = f1;
                f1 = d;
                cellId = n + g;
                cellPos = n + g + o;
            } else if (d < f2) {
                f2 = d;
            }
        }
    }
    return vec2(sqrt(f1), sqrt(f2));
}

float cr_box(vec2 q, vec2 b) {
    vec2 d = abs(q) - b;
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}

vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);

    float px = 1.0 / u_res.y;
    vec2 p = (uv - 0.5) * vec2(ratio, 1.0);
    vec2 hs = vec2(0.5 * ratio, 0.5);

    float a = smoothstep(0.0, 0.5, progress);          // how crumpled
    float ball = smoothstep(0.08, 0.6, progress);       // rectangle -> ball
    float shrink = mix(1.0, ball_size / 0.5, ball);
    float drop = smoothstep(0.45, 1.0, progress);
    vec2 center = vec2(0.08 * drop, -drop * drop * (0.5 + ball_size + 0.25));
    float rot = spin * (ball * 0.6 + drop * 1.6);

    vec2 o = p - center;
    float cs = cos(rot);
    float sn = sin(rot);
    vec2 q = mat2(cs, -sn, sn, cs) * o / shrink;        // paper space

    // silhouette: the frame rectangle morphing into a lumpy ball
    vec2 cid;
    vec2 cpos;
    vec2 vf = cr_voronoi(q * facets, cid, cpos);
    float lump = (hash12(cid) - 0.5) * 0.08 + (vf.x - 0.35) * 0.06;
    float sdR = cr_box(q, hs);
    float sdC = length(q) - 0.5 + lump * ball;
    float sd = mix(sdR, sdC, ball);
    float ppx = px / shrink;
    float mask = 1.0 - smoothstep(-ppx, ppx, sd);

    // incoming picture with the ball's soft shadow under it
    vec4 inc = getToColor(uv);
    float shd = length((o - vec2(0.02, -0.03)) / vec2(1.0, 0.8)) - 0.5 * shrink;
    float shadow = 0.45 * ball * (1.0 - smoothstep(-0.02, 0.12, shd));
    vec4 col = vec4(inc.rgb * (1.0 - shadow), inc.a);
    if (mask <= 0.0) return col;

    // texture: stretch the ball back over the frame, then jumble each facet
    vec2 tq = mix(q, q * hs / 0.5 * 1.15, ball);
    vec2 cc = cpos / facets;
    cc = mix(cc, cc * hs / 0.5 * 1.15, ball);
    vec2 jitter = (cr_h2(cid + 3.7) - 0.5) * 0.18 * fold * a;
    float squash = 1.0 + (hash12(cid + 9.1) - 0.3) * 0.5 * fold * a;
    tq = cc + (tq - cc) * squash + jitter;
    vec2 tuv = clamp(tq / vec2(ratio, 1.0) + 0.5, 0.0, 1.0);
    vec4 f = getFromColor(tuv);

    // per-facet lighting
    vec2 tilt = (cr_h2(cid + 5.3) - 0.5) * 2.2 * fold * a;
    vec2 bulge = q / 0.5 * ball * 0.9;                   // ball-shaped normals
    vec3 n = normalize(vec3(tilt + bulge, 1.0));
    vec3 lightDir = normalize(vec3(-0.45, 0.55, 0.7));
    float lit = dot(n, lightDir) / lightDir.z;
    lit = mix(1.0, lit, a);
    // creases along the facet borders: a dark valley with a light ridge beside it
    float edge = vf.y - vf.x;
    float crease = 1.0 - smoothstep(0.0, 0.025, edge);
    float ridge = smoothstep(0.015, 0.04, edge) * (1.0 - smoothstep(0.04, 0.1, edge));
    float k = a * fold;
    vec3 c = f.rgb * clamp(lit, 0.25, 1.6);
    c *= 1.0 - 0.3 * crease * min(k, 1.0);
    c += 0.1 * ridge * min(k, 1.0);
    // the ball's rim falls into shade
    c *= 1.0 - 0.35 * ball * smoothstep(0.3, 0.5, length(q));

    return mix(col, vec4(c, f.a), mask);
}
