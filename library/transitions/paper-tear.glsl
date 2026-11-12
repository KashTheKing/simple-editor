// @name Paper Tear
// @desc The outgoing picture is ripped away like a sheet of paper along a ragged, fibrous edge.
// @tags paper, tear, rip, craft, wipe
// @param angle 15 -180 180
// @param roughness 1 0 2
// @param rim_width 1.5 0 3
// @param shadow 0.6 0 1

float pt_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float pt_fbm(vec2 p) {
    float s = 0.0;
    float a = 0.5;
    for (int i = 0; i < 5; i++) {
        s += a * pt_noise(p);
        p = p * 2.07 + vec2(3.1, 7.7);
        a *= 0.5;
    }
    return s / 0.96875;
}

vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);

    float px = 1.0 / u_res.y;
    float an = radians(angle);
    vec2 dir = vec2(cos(an), sin(an));
    vec2 nrm = vec2(-dir.y, dir.x);
    vec2 p = (uv - 0.5) * vec2(ratio, 1.0);

    float d = dot(p, dir);   // across the tear
    float t = dot(p, nrm);   // along the tear

    // big wander + small jaggies of the torn line
    float jag = (pt_fbm(vec2(t * 3.0, 1.3)) - 0.5) * 0.16 * roughness
              + (pt_noise(vec2(t * 38.0, 4.0)) - 0.5) * 0.025 * roughness;
    float rim = 0.022 * rim_width;
    float margin = 0.1 * roughness + rim + 0.08;
    float ext = 0.5 * (abs(dir.x) * ratio + abs(dir.y));
    float e = mix(-ext - margin, ext + margin, progress);

    float x = d - e - jag;   // > 0: the sheet is still there

    // the torn rim: white paper fibres with a ragged, hairy outer boundary
    float fib = pt_fbm(vec2(t * 140.0, d * 9.0));
    float hair = pow(pt_noise(vec2(t * 260.0, 0.5)), 4.0);
    float rimOuter = rim * (0.45 + 0.9 * pt_fbm(vec2(t * 18.0, 9.0)));
    float rx = x + (fib - 0.5) * 0.012 * roughness + hair * 0.012 * roughness;

    vec4 inc = getToColor(uv);
    // soft shadow of the lifted sheet onto the incoming picture
    float sh = shadow * 0.55 * exp(-max(-x, 0.0) * 18.0);
    vec4 col = vec4(inc.rgb * (1.0 - sh), inc.a);

    // paper rim (fibres) on top of the shadow
    float rimMask = smoothstep(-px, px, rx);
    float paperGrain = 0.88 + 0.12 * fib;
    vec3 rimCol = vec3(0.97, 0.96, 0.93) * paperGrain;
    // the fibre band is slightly translucent at its very edge
    float rimA = rimMask * mix(0.75, 1.0, smoothstep(0.0, 0.006, rx));
    col.rgb = mix(col.rgb, rimCol, rimA);
    col.a = mix(col.a, 1.0, rimA);

    // printed picture surface, inset from the fibres
    vec4 outg = getFromColor(uv);
    float printMask = smoothstep(-px, px, x - rimOuter);
    // slight lift: the edge of the sheet catches a little light
    float lift = 1.0 + 0.08 * exp(-max(x - rimOuter, 0.0) * 60.0);
    col = mix(col, vec4(outg.rgb * lift, outg.a), printMask);
    return col;
}
