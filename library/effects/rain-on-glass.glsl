// @name Rain on Glass
// @desc Looking through a rainy window: drops slide down the fogged glass, leave clear trails and bend the picture behind them.
// @tags rain, window, glass, drops, weather, moody, water
// @param amount 0.7 0 1
// @param drop_size 1.0 0.4 2
// @param speed 1.0 0 3
// @param fog 0.6 0 1
// @param refraction 1.0 0 2

// a sliding drop with its trail in one grid cell; returns (offset.xy, drop mask, wiped mask)
vec4 rg_slider(vec2 p, float t, float seed, vec2 grid, float size) {
    vec2 q = p * grid;
    float col = floor(q.x);
    // stagger columns so drops are not lined up
    q.y += hash11(col * 7.31 + seed) * 5.0;
    vec2 cell = floor(q);
    vec2 f = fract(q) - 0.5;
    float n = hash12(cell + seed * 3.7);
    float n2 = hash12(cell * 1.37 + seed + 4.1);
    float n3 = hash12(cell * 2.11 + seed + 9.3);

    // the drop falls down the cell, sticking and slipping
    float fr = fract(t * (0.10 + 0.12 * n) + n2);
    float s = fr + 0.045 * sin(fr * 18.85);
    float dy = -0.42 + 0.84 * s;
    float dx = (n3 - 0.5) * 0.55 + 0.05 * sin(dy * 9.0 + n * 6.28);
    float alive = step(0.25, n) * smoothstep(0.0, 0.08, fr) * (1.0 - smoothstep(0.88, 1.0, fr));

    // in frame units (height = 1)
    vec2 local = (f - vec2(dx, dy)) / grid;
    float r = 0.024 * size * (0.7 + 0.6 * n2);
    // slightly egg-shaped, wider at the bottom
    vec2 sh = local * vec2(1.0, 0.9 - 0.25 * clamp(local.y / r, -1.0, 1.0));
    float d = length(sh) / r;
    float drop = (1.0 - smoothstep(0.85, 1.0, d)) * alive;

    // trail: small beads left above the drop, and a wiped strip in the fog
    float above = step(f.y, dy);
    float dist = (dy - f.y) / grid.y;
    float tx = abs(f.x - dx + 0.05 * sin(f.y * 9.0 + n * 6.28) - 0.05 * sin(dy * 9.0 + n * 6.28)) / grid.x;
    float wipe = (1.0 - smoothstep(r * 0.5, r * 1.1, tx)) * above * exp(-dist * 6.0) * alive;
    float by = fract(f.y * 7.0 + n * 3.0) - 0.5;
    vec2 bl = vec2(tx, by / (7.0 * grid.y));
    float br = r * 0.35 * (0.5 + 0.5 * exp(-dist * 6.0));
    float bead = (1.0 - smoothstep(0.7, 1.0, length(bl) / br)) * above * step(0.3, hash12(cell + floor(f.y * 7.0 + n * 3.0))) * exp(-dist * 4.0) * alive;

    vec2 off = drop > 0.0 ? local / r : vec2(0.0);
    vec2 boff = bead > 0.0 ? bl / br : vec2(0.0);
    return vec4(off * drop + boff * bead * 0.6, max(drop, bead), max(wipe, max(drop, bead)));
}

// still droplets that bead up and evaporate in place
vec4 rg_static(vec2 p, float t, vec2 grid, float size) {
    vec2 q = p * grid;
    vec2 cell = floor(q);
    vec2 f = fract(q) - 0.5;
    float n = hash12(cell + 31.7);
    vec2 c = vec2(hash12(cell + 2.3), hash12(cell + 5.9)) - 0.5;
    vec2 local = (f - c * 0.7) / grid;
    float life = fract(t * 0.06 + n * 7.0);
    float r = 0.009 * size * (0.4 + n) * smoothstep(0.0, 0.25, life) * (1.0 - smoothstep(0.7, 1.0, life));
    float d = length(local) / max(r, 1e-4);
    float m = (1.0 - smoothstep(0.7, 1.0, d)) * step(0.35, n);
    return vec4(m > 0.0 ? local / max(r, 1e-4) * m : vec2(0.0), m, m);
}

vec4 effect(vec4 src, vec2 uv) {
    float aspect = u_res.x / u_res.y;
    vec2 p = vec2(uv.x * aspect, uv.y);
    float t = u_time * speed;
    float sz = drop_size;

    vec4 a = rg_slider(p, t, 1.0, vec2(4.0, 1.3) / sz, sz);
    vec4 b = rg_slider(p, t * 1.3 + 4.0, 7.0, vec2(7.0, 2.2) / sz, sz * 0.7);
    vec4 c = rg_static(p, t, vec2(14.0, 14.0) / sz, sz);
    a *= smoothstep(0.0, 0.3, amount);
    b *= smoothstep(0.3, 0.6, amount);
    c *= smoothstep(0.1, 0.8, amount);

    vec2 nrm = a.xy + b.xy + c.xy;
    float dropm = clamp(a.z + b.z + c.z, 0.0, 1.0);
    float clear = clamp(a.w + b.w + c.w, 0.0, 1.0);

    // fogged glass: a soft blur of the picture, lifted a little
    vec2 px = 1.0 / u_res;
    float rad = 30.0 * u_scale * fog;
    float rot = hash12(floor(uv * u_res)) * 6.2832;
    vec3 blur = src.rgb;
    for (int i = 0; i < 24; i++) {
        float fi = float(i) + 0.5;
        float ang = fi * 2.39996 + rot;
        blur += texture(tex, uv + vec2(cos(ang), sin(ang)) * rad * sqrt(fi / 24.0) * px).rgb;
    }
    blur /= 25.0;
    vec3 fogged = mix(blur, vec3(luma(blur)) * 0.9 + 0.12, 0.4 * fog);

    // a drop is a tiny lens: it shows a shrunken, flipped view of what is behind it
    vec2 ruv = uv - nrm * vec2(1.0 / aspect, 1.0) * 0.05 * refraction;
    vec3 sharp = sample_clamped(tex, clamp(ruv, 0.0, 1.0)).rgb;
    float rim = smoothstep(0.55, 1.0, length(nrm)) * dropm;
    sharp *= 1.0 - rim * 0.35;
    // a glint on the upper left of every drop
    float glint = smoothstep(0.75, 0.95, dot(normalize(nrm + 1e-5), vec2(-0.6, -0.8))) * smoothstep(0.3, 0.7, length(nrm)) * dropm;
    sharp += glint * 0.35;

    vec3 col = mix(fogged, mix(src.rgb, sharp, dropm), clear);
    return vec4(clamp(col, 0.0, 1.0), src.a);
}
