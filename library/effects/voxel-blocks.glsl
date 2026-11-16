// @name Voxel Blocks
// @desc Audio visualizer: a blocky voxel island with pixel-art grass, dirt, stone and ores. Columns rise with the music's spectrum, a block drops in and stacks on every beat while the oldest ones break away, and the camera slowly orbits. Background 0 is transparent for overlays.
// @tags visualizer, audio, music, beat, voxel, blocks, pixel art, 3d, game
// @param level 0.4 0 1
// @param band1 0.6 0 1
// @param band2 0.5 0 1
// @param band3 0.45 0 1
// @param band4 0.4 0 1
// @param band5 0.35 0 1
// @param band6 0.3 0 1
// @param beat 0.5 0 1
// @param beats 6 0 100000
// @param palette 0 0 3
// @param speed 1 0 3
// @param camera 0.5 0 1
// @param background 1 0 2
// Procedural pixel art only: every 16x16 texture below is generated from hashes, no image files.
// palette: 0 meadow (grass, dirt, stone, ores), 1 ember (netherrack-like red rock, glowing ore),
// 2 snow (snow caps, ice), 3 desert (sand, sandstone). background: 0 transparent, 1 sky, 2 black.
// camera: 0 low and close .. 1 high and wide. speed: orbit speed (0 = still).

const int N = 10;         // island is N x N columns
const int HMAX = 16;      // tallest column
const int STACK = 8;      // beats a dropped block stays before it breaks away

float vxh(vec3 p) {
    p = fract(p * vec3(0.1031, 0.1030, 0.0973));
    p += dot(p, p.yxz + 33.33);
    return fract((p.x + p.y) * p.z);
}
float vxh2(vec2 p) { return vxh(vec3(p, 7.13)); }

// column ground height: a small hill plus the spectrum (bass in the middle, highs at the rim)
int baseHeight(ivec2 c) {
    float terr = 1.0 + floor(vxh2(vec2(c) + 3.7) * 2.0);
    vec2 d = (vec2(c) + 0.5 - float(N) * 0.5) / (float(N) * 0.5);
    float r = clamp(length(d) / 1.2, 0.0, 1.0);
    float b[6] = float[6](band1, band2, band3, band4, band5, band6);
    float x = r * 5.0;
    int i = int(min(floor(x), 4.0));
    float v = mix(b[i], b[i + 1], x - float(i));
    float spec = floor(v * (7.0 - 3.0 * r) + 0.35 * level);
    return int(terr + spec);
}

// which column beat `i` drops onto
ivec2 dropCol(float i) {
    float h = vxh(vec3(i, 1.7, 9.1));
    float g = vxh(vec3(i, 5.3, 2.2));
    return ivec2(1 + int(h * float(N - 2)), 1 + int(g * float(N - 2)));
}

// how long into the beat a dropped block lands
const float LAND = 0.35;

// block type at a cell: 0 air; 1 top, 2 sub-surface, 3 rock, 4..7 ores; 8+k dropped block from beat k
int cell(ivec3 q) {
    if (q.x < 0 || q.z < 0 || q.x >= N || q.z >= N || q.y < 0 || q.y >= HMAX) return 0;
    ivec2 c = q.xz;
    int h = baseHeight(c);
    if (q.y < h) {
        if (q.y == h - 1) return 1;
        if (q.y >= h - 3) return 2;
        float o = vxh(vec3(q) + 11.0);
        return o > 0.9 ? 4 + int(vxh(vec3(q) + 2.0) * 4.0) : 3;
    }
    if (q.y >= h + STACK) return 0;
    // stacked drops: the last STACK beats that landed on this column, oldest lowest
    int n = 0;
    float cur = floor(beats);
    for (int k = STACK - 1; k >= 0; k--) {
        float i = cur - float(k);
        if (i < 1.0) continue;
        if (k == 0 && beat < LAND) continue; // still falling
        if (dropCol(i) == c) {
            if (q.y == h + n) return 8 + int(mod(i, 8.0));
            n++;
        }
    }
    return 0;
}

// 16x16 pixel-art texel colour for block `t` on a face (`side` = not top/bottom), texel `tx`
vec3 texel(int t, ivec2 tx, bool side, bool bottom, ivec3 q) {
    float n = vxh(vec3(vec2(tx), float(t) * 3.1 + float(q.x * 7 + q.y * 13 + q.z * 17) * 0.01));
    float n2 = vxh(vec3(vec2(tx / 2), float(t) + 4.0));
    int pal = int(floor(palette + 0.5));
    vec3 top, sub, rock, oreBase;
    if (pal == 1) {
        top = vec3(0.50, 0.16, 0.15); sub = vec3(0.42, 0.13, 0.13); rock = vec3(0.24, 0.20, 0.22); oreBase = vec3(1.0, 0.7, 0.2);
    } else if (pal == 2) {
        top = vec3(0.93, 0.96, 1.0); sub = vec3(0.45, 0.32, 0.22); rock = vec3(0.55, 0.70, 0.85); oreBase = vec3(0.3, 0.9, 1.0);
    } else if (pal == 3) {
        top = vec3(0.86, 0.79, 0.55); sub = vec3(0.80, 0.70, 0.46); rock = vec3(0.72, 0.58, 0.38); oreBase = vec3(1.0, 0.85, 0.3);
    } else {
        top = vec3(0.37, 0.62, 0.24); sub = vec3(0.47, 0.33, 0.22); rock = vec3(0.50, 0.50, 0.52); oreBase = vec3(0.3, 0.9, 0.95);
    }
    vec3 c;
    if (t == 1) {
        if (!side && !bottom) {
            c = top * (0.78 + 0.4 * n);
        } else if (bottom) {
            c = sub * (0.75 + 0.4 * n);
        } else {
            // a ragged cap of the top colour over the sub-surface colour
            float lip = 3.0 + floor(vxh(vec3(float(tx.x), 2.0, float(q.x + q.z))) * 3.0);
            c = (float(tx.y) < lip ? top : sub) * (0.75 + 0.4 * n);
        }
    } else if (t == 2) {
        c = sub * (0.72 + 0.45 * n);
    } else if (t == 3) {
        c = rock * (0.75 + 0.35 * n) * (0.9 + 0.2 * n2);
    } else if (t >= 4 && t <= 7) {
        vec3 ores[4] = vec3[4](vec3(0.12), vec3(1.0, 0.82, 0.25), oreBase, vec3(0.95, 0.15, 0.12));
        bool fleck = vxh(vec3(vec2(tx / 3), float(t) * 9.0 + float(q.y))) > 0.62 && n > 0.35;
        c = fleck ? ores[t - 4] * (0.8 + 0.3 * n) : rock * (0.75 + 0.35 * n);
    } else {
        // dropped blocks: planks, bricks, glowing ore, wool... picked by the beat
        int k = t - 8;
        if (k == 0 || k == 4) { // planks
            vec3 w = vec3(0.70, 0.52, 0.30);
            c = w * (0.8 + 0.25 * n) * ((tx.y % 4 == 0) ? 0.75 : 1.0);
        } else if (k == 1 || k == 5) { // bricks
            bool mortar = (tx.y % 4 == 0) || ((tx.x + (tx.y / 4 % 2) * 4) % 8 == 0);
            c = mortar ? vec3(0.62, 0.60, 0.56) : vec3(0.62, 0.27, 0.20) * (0.85 + 0.25 * n);
        } else if (k == 2 || k == 6) { // gold block
            c = vec3(1.0, 0.80, 0.25) * (0.85 + 0.25 * n) + ((tx.x == 1 || tx.y == 1) ? 0.15 : 0.0);
        } else { // gem block
            c = oreBase * (0.75 + 0.3 * n) + ((tx.x + tx.y) % 5 == 0 ? 0.2 : 0.0);
        }
    }
    // darker 1-texel rim reads as block edges at any distance
    if (tx.x == 0 || tx.y == 0 || tx.x == 15 || tx.y == 15) c *= 0.82;
    return c;
}

vec3 sky(vec3 rd) {
    float y = clamp(rd.y * 0.5 + 0.5, 0.0, 1.0);
    int pal = int(floor(palette + 0.5));
    vec3 lo = pal == 1 ? vec3(0.25, 0.06, 0.05) : pal == 2 ? vec3(0.75, 0.82, 0.92) : pal == 3 ? vec3(0.95, 0.80, 0.55) : vec3(0.62, 0.80, 1.0);
    vec3 hi = pal == 1 ? vec3(0.08, 0.02, 0.03) : pal == 2 ? vec3(0.45, 0.60, 0.85) : pal == 3 ? vec3(0.45, 0.62, 0.90) : vec3(0.28, 0.50, 0.92);
    return mix(lo, hi, y);
}

// ray vs axis-aligned box: entry distance (or -1) and the hit normal
float boxHit(vec3 ro, vec3 rd, vec3 bmin, vec3 bmax, out vec3 nrm) {
    vec3 inv = 1.0 / rd;
    vec3 t0 = (bmin - ro) * inv, t1 = (bmax - ro) * inv;
    vec3 tmin = min(t0, t1), tmax = max(t0, t1);
    float tn = max(max(tmin.x, tmin.y), tmin.z);
    float tf = min(min(tmax.x, tmax.y), tmax.z);
    if (tf < max(tn, 0.0)) return -1.0;
    nrm = tn == tmin.x ? vec3(-sign(rd.x), 0, 0) : tn == tmin.y ? vec3(0, -sign(rd.y), 0) : vec3(0, 0, -sign(rd.z));
    return tn;
}

vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    vec2 sp = vec2((uv.x - 0.5) * asp, 0.5 - uv.y);
    // orbiting camera around the island's centre
    float ang = u_time * 0.12 * speed + 0.7;
    float dist = mix(18.0, 27.0, camera);
    float elev = mix(0.38, 0.85, camera);
    vec3 target = vec3(float(N) * 0.5, 2.6, float(N) * 0.5);
    vec3 ro = target + dist * vec3(cos(ang) * cos(elev), sin(elev), sin(ang) * cos(elev));
    vec3 fw = normalize(target - ro);
    vec3 rt = normalize(cross(fw, vec3(0, 1, 0)));
    vec3 up = cross(rt, fw);
    vec3 rd = normalize(fw * 1.6 + rt * sp.x + up * sp.y);

    // DDA through the grid's box
    vec3 nrm0;
    float tEnter = boxHit(ro, rd, vec3(0.0), vec3(float(N), float(HMAX), float(N)), nrm0);
    float tHit = 1e9;
    vec3 hn = vec3(0.0);
    ivec3 hq = ivec3(0);
    int ht = 0;
    if (tEnter >= 0.0) {
        vec3 p = ro + rd * (tEnter + 1e-3);
        ivec3 q = ivec3(floor(p));
        vec3 st = sign(rd);
        vec3 dlt = abs(1.0 / rd);
        vec3 nxt = (vec3(q) + max(st, 0.0) - p) / rd;
        vec3 n = nrm0;
        float t = tEnter;
        for (int i = 0; i < 72; i++) {
            int b = cell(q);
            if (b != 0) { tHit = t; hn = n; hq = q; ht = b; break; }
            if (nxt.x < nxt.y && nxt.x < nxt.z) {
                t = tEnter + nxt.x; nxt.x += dlt.x; q.x += int(st.x); n = vec3(-st.x, 0, 0);
            } else if (nxt.y < nxt.z) {
                t = tEnter + nxt.y; nxt.y += dlt.y; q.y += int(st.y); n = vec3(0, -st.y, 0);
            } else {
                t = tEnter + nxt.z; nxt.z += dlt.z; q.z += int(st.z); n = vec3(0, 0, -st.z);
            }
            if (q.x < -1 || q.z < -1 || q.x > N || q.z > N || q.y < -1 || q.y > HMAX) break;
        }
    }

    // this beat's block, still falling (an ease-in drop onto its column's stack)
    float cur = floor(beats);
    if (cur >= 1.0 && beat < LAND) {
        ivec2 c = dropCol(cur);
        int stack = baseHeight(c);
        for (int k = STACK - 1; k >= 1; k--) {
            float i = cur - float(k);
            if (i >= 1.0 && dropCol(i) == c) stack++;
        }
        float f = beat / LAND;
        float y = float(stack) + (1.0 - f * f) * 9.0;
        vec3 fn;
        vec3 bmin = vec3(float(c.x), y, float(c.y));
        float tb = boxHit(ro, rd, bmin, bmin + 1.0, fn);
        if (tb > 0.0 && tb < tHit) {
            ht = 8 + int(mod(cur, 8.0));
            vec3 hp = ro + rd * tb - bmin;
            ivec2 tx = ivec2(floor((abs(fn.y) > 0.5 ? hp.xz : abs(fn.x) > 0.5 ? vec2(hp.z, 1.0 - hp.y) : vec2(hp.x, 1.0 - hp.y)) * 16.0));
            vec3 col = texel(ht, clamp(tx, 0, 15), abs(fn.y) < 0.5, fn.y < -0.5, ivec3(c.x, 0, c.y));
            float lit = abs(fn.y) > 0.5 ? (fn.y > 0.0 ? 1.0 : 0.5) : (abs(fn.x) > 0.5 ? 0.8 : 0.65);
            return vec4(col * lit * (1.0 + 0.25 * level), 1.0);
        }
    }

    vec3 bg = sky(rd);
    float bga = background < 0.5 ? 0.0 : 1.0;
    if (background >= 1.5) bg = vec3(0.0);
    if (tHit > 1e8) {
        return vec4(bg * bga, bga);
    }
    vec3 hp = ro + rd * tHit - vec3(hq);
    vec2 fuv = abs(hn.y) > 0.5 ? hp.xz : abs(hn.x) > 0.5 ? vec2(hp.z, 1.0 - hp.y) : vec2(hp.x, 1.0 - hp.y);
    ivec2 tx = clamp(ivec2(floor(fuv * 16.0)), 0, 15);
    vec3 col = texel(ht, tx, abs(hn.y) < 0.5, hn.y < -0.5, hq);
    // blocky lighting: sun from above, faces shaded by direction, a soft occlusion from the cell above
    float lit = abs(hn.y) > 0.5 ? (hn.y > 0.0 ? 1.0 : 0.5) : (abs(hn.x) > 0.5 ? 0.8 : 0.65);
    if (hn.y < 0.5 && cell(hq + ivec3(0, 1, 0) + ivec3(hn)) != 0) lit *= 0.7;
    // the block that just landed flashes, and the whole island breathes with the loudness
    bool fresh = ht >= 8 && hq.y >= 0 && cell(hq + ivec3(0, 1, 0)) == 0 && dropCol(floor(beats)) == hq.xz && beat < LAND + 0.25;
    col = col * lit * (0.9 + 0.25 * level) + (fresh ? vec3(0.35) * (1.0 - (beat - LAND) / 0.25) : vec3(0.0));
    // distance haze toward the background (none when transparent)
    float fog = clamp((tHit - dist * 0.6) / (dist * 1.4), 0.0, 1.0) * 0.5 * bga;
    return vec4(mix(col, bg, fog), 1.0);
}
