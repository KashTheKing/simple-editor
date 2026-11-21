// @name Olympus Storm
// @desc Audio visualizer: a spinning liquid-chrome ball that flows with the music, grows round-edged spikes on every beat and fires needle-sharp chrome bullets on the high screeches, with lightning on the most intense sounds; from the drop it all turns gold; a holy beam behind it that rides the song, pulses on every beat and blooms on held vocals; electric sparks; and a shiny silver waveform arriving at the left edge as it plays. Transparent, so a background shows through.
// @tags visualizer, audio, music, metal, chrome, spikes, lightning, electricity, greek, olympus, hardstyle, waveform, beam
// @credit Visualizer Helper (MIT)
// @param level 0.3 0 1
// @param band1 0.3 0 1
// @param band4 0.3 0 1
// @param kclock 0.5 0 100000
// @param kpow 1 0 1
// @param sclock 0.5 0 100000
// @param vox 0 0 1
// @param wave 0 0 1
// @param lyric 0 0 1
// @param vib 0 -1 1
// @param kick 1.4 0 3
// @param screech 1.2 0 3
// @param fx 1 0 1
// @param beam 0.2 0 3
//
// Keyed by hand (the app's own beat detection misses most hardstyle hits):
// `kclock` one key per beat hit (value = hit number, linear between): fract() is the phase since the
// last hit, floor() the hit count. `kpow` each hit's strength (Hold keys): 1 on the bar's downbeat.
// `sclock` the same clock for the high screeches, ramping over 0.6 s so its phase is time.
// `lyric` (keyed) 1 while a lyric line is on screen: the waveform flattens under it. Ball size is fixed at 0.4.
// `kick` above its normal 1.2 = "go harder" (keyed 2.2 on the second drop): stronger hits, screeches, beam, shake, more spines and electricity.
// `vib` the vocal vibrato (pitch wobble around the held note, 4-8 Hz, -1..1): stretch, ripples, spikes.
// `vox` the held vocal notes (0.15 s rise, 0.6 s fall). `fx` 0 before the drop, 1 from it: lightning,
// bullets, flashes, corona, beam pulses and the gold. `beam` the beam's glow; above 1 it is a white
// flash (keyed up to 3 on the drop).
// The ball: a liquid body flowing with level/band1/band4, round-edged spikes that rise and melt with
// each beat, and bullets fired off on each screech. Motion that is not a hit runs on the clock.

// Visualizer Helper fills these per song (Visualizer Templates / Drop FX): the biggest drop and the shatter
// build into it (clip seconds; 1e6 = none), the colours, the spines per hit, and which layers draw.
const float BIGDROP = 1000000.0;
const float RIFT0 = 999999.0;
const float SUCK0 = 999997.0;
const float SUCK1 = 1000005.0;
const float RIFTHIT0 = 0.0;
const float RIFTHITS = 1.0;
const vec3 METAL0 = vec3(0.62, 0.64, 0.68);
const vec3 METAL1 = vec3(1.0, 0.72, 0.30);
const vec3 ELEC0 = vec3(0.45, 0.75, 1.0);
const vec3 ELEC1 = vec3(1.0, 0.78, 0.35);
const float SPINES = 4.0;
const float SHOW_BALL = 1.0;
const float SHOW_WAVE = 1.0;
const float SHOW_BOLTS = 1.0;
const float SHOW_BEAM = 1.0;

const vec3 AX[16] = vec3[16](
    vec3(0.0, 0.5257, 0.8507), vec3(0.0, 0.5257, -0.8507), vec3(0.5257, 0.8507, 0.0),
    vec3(0.5257, -0.8507, 0.0), vec3(0.8507, 0.0, 0.5257), vec3(-0.8507, 0.0, 0.5257),
    vec3(0.5774, 0.5774, 0.5774), vec3(0.5774, 0.5774, -0.5774), vec3(0.5774, -0.5774, 0.5774),
    vec3(-0.5774, 0.5774, 0.5774), vec3(0.0, 0.3568, 0.9342), vec3(0.0, 0.3568, -0.9342),
    vec3(0.3568, 0.9342, 0.0), vec3(0.3568, -0.9342, 0.0), vec3(0.9342, 0.0, 0.3568),
    vec3(-0.9342, 0.0, 0.3568));

float vn1(float x) {
    float i = floor(x), f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    return mix(hash11(i), hash11(i + 1.0), f);
}

mat3 rotm(float a, float b) {
    float ca = cos(a), sa = sin(a), cb = cos(b), sb = sin(b);
    mat3 ry = mat3(ca, 0.0, -sa, 0.0, 1.0, 0.0, sa, 0.0, ca);
    mat3 rx = mat3(1.0, 0.0, 0.0, 0.0, cb, sb, 0.0, -sb, cb);
    return rx * ry;
}

float g_r;
float g_flow;
float g_t;
vec3 g_ax[32];
float g_len[32];
vec3 g_sd[12];
float g_sp;
float g_maxlen;
float g_vv;
float g_vib;
float g_b1;
float g_b4;
float g_kk;
float g_spd;
float g_hold;
float g_hseed;
float g_bn;
float g_pop;
float g_bph;
float g_cnt;
float g_pamp;
vec2 g_pk[24];
float g_pe[24];
int g_np;
float g_kc;

float cone2(vec2 q, float h, float r1, float r2) {
    vec2 k1 = vec2(r2, h);
    vec2 k2 = vec2(r2 - r1, 2.0 * h);
    vec2 ca = vec2(q.x - min(q.x, (q.y < 0.0) ? r1 : r2), abs(q.y) - h);
    vec2 cb = q - k1 + k2 * clamp(dot(k1 - q, k2) / dot(k2, k2), 0.0, 1.0);
    float s = (cb.x < 0.0 && ca.y < 0.0) ? -1.0 : 1.0;
    return s * sqrt(min(dot(ca, ca), dot(cb, cb)));
}
float smin(float a, float b, float k) {
    float h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}
float capsule(vec3 p, vec3 a, vec3 b, float r) {
    vec3 pa = p - a, ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h) - r;
}
// layer 1: the liquid body, flowing on the clock, as deep as the music is full
float body(vec3 p) {
    float st = 0.04 * g_vv + 0.02 * g_vib;
    p *= vec3(1.0 + st, 1.0 - st, 1.0 + 0.5 * st);
    float l = length(p);
    vec3 n = p / max(l, 1e-4);
    float t = g_t;
    float f = sin(3.0 * n.x + t * 1.1 + sin(2.3 * n.y + t * 0.7)) * sin(3.2 * n.y - t * 0.9 + sin(2.1 * n.z + t * 0.5))
            * sin(2.9 * n.z + t * 0.8 + sin(2.7 * n.x - t * 0.6));
    float f2 = sin(7.0 * n.x - t * 1.7) * sin(6.5 * n.y + t * 1.3) * sin(7.3 * n.z + t * 1.5);
    // the voice grows sea-urchin spines over the whole surface: a staggered grid of thin sharp cones,
    // each its own length, trembling a little with the vibrato, sinking back as the voice fades
    float lon = atan(n.z, n.x);
    float lat = asin(clamp(n.y, -1.0, 1.0));
    vec2 gu = vec2(lon / 6.2832 * 30.0, (lat / 3.1416 + 0.5) * 15.0);
    float row = floor(gu.y);
    gu.x += 0.5 * mod(row, 2.0);
    vec2 cid = floor(gu);
    vec2 fu = fract(gu) - 0.5;
    fu.x *= max(cos(lat), 0.25);
    float hs = hash12(cid + 3.7);
    float sl = (0.12 + 0.16 * hs) * (1.0 + 0.25 * g_vib * (hs - 0.5) * 2.0);
    float spine = (0.3 + 0.15 * hs) * pow(max(0.0, 1.0 - length(fu) / 0.47), 3.0);
    // each beat pops 1-4 spines (more when the song is intense) at random spots; each lives four beats:
    // out fast, held, then melting back
    float drive = 0.0;
    for (int i = 0; i < 24; i++) {
        if (i >= g_np) break;
        if (all(equal(g_pk[i], cid))) drive = max(drive, g_pe[i]);
    }
    drive *= g_pamp;
    // frozen bursts: on some beats a different set of spots shoots out long rigid spines that hold still, then retract
    float hsel = hash12(cid + 9.1);
    float frozen = step(0.5, hsel) * step(hsel, 0.75) * step(0.5, hash12(cid + g_hseed * 0.37)) * g_hold;
    float fsp = (0.24 + 0.14 * hs) * pow(max(0.0, 1.0 - length(fu) / 0.47), 3.0) * frozen;
    return l - g_r - g_flow * (0.04 * f + 0.006 * f2) - drive * spine * 1.3 - fsp * 1.3;
}
float ball(vec3 p) {
    // far from the surface, or no spike out: skip the spike search (the cost of this shader)
    float lp = length(p);
    float marg = g_maxlen + 0.2 + 0.6 * g_spd + 0.5 * g_hold + 0.15 * abs(g_vib);
    if (lp > g_r + marg) return (lp - g_r - marg + 0.05) * 0.65;
    float d = body(p);
    float kq = mix(0.65, 0.3, clamp(g_spd, 0.0, 1.0));
    if (g_maxlen < 0.004) return d * kq;
    // layer 2: round-edged spikes on the three nearest of 32 directions while a beat is out
    // (blending three keeps the surface continuous where neighbouring spikes' zones meet)
    float c0 = -2.0, c1 = -2.0, c2 = -2.0;
    int i0 = 0, i1 = 0, i2 = 0;
    for (int i = 0; i < 32; i++) {
        float dd = dot(p, g_ax[i]);
        if (dd > c0) { c2 = c1; i2 = i1; c1 = c0; i1 = i0; c0 = dd; i0 = i; }
        else if (dd > c1) { c2 = c1; i2 = i1; c1 = dd; i1 = i; }
        else if (dd > c2) { c2 = dd; i2 = i; }
    }
    for (int s = 0; s < 3; s++) {
        int bi = s == 0 ? i0 : (s == 1 ? i1 : i2);
        float len = g_len[bi];
        if (len > 0.004) {
            vec3 ax = g_ax[bi];
            float al = dot(p, ax);
            float rr = length(p - ax * al);
            float hb = g_r * 0.75, ht = g_r + len;
            float cn = cone2(vec2(rr, al - 0.5 * (hb + ht)), max(0.5 * (ht - hb), 0.001), g_r * 0.22, 0.0) - 0.03;
            d = smin(d, cn, 0.09 * clamp(len / 0.08, 0.0, 1.0) + 0.005);
        }
    }
    return d * kq;
}

float nh3(vec3 p) {
    p = fract(p * 0.3183099 + 0.1);
    p *= 17.0;
    return fract(p.x * p.y * p.z * (p.x + p.y + p.z));
}
float vnoise3(vec3 x) {
    vec3 i = floor(x), f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(mix(nh3(i), nh3(i + vec3(1, 0, 0)), f.x), mix(nh3(i + vec3(0, 1, 0)), nh3(i + vec3(1, 1, 0)), f.x), f.y),
               mix(mix(nh3(i + vec3(0, 0, 1)), nh3(i + vec3(1, 0, 1)), f.x), mix(nh3(i + vec3(0, 1, 1)), nh3(i + vec3(1, 1, 1)), f.x), f.y), f.z);
}
// thin bright ridges of noise on the surface: electricity crawling over the metal
float veins(vec3 p, float t) {
    // thin ridged arcs drifting smoothly over the metal
    vec3 o = vec3(t * 0.35, -t * 0.25, t * 0.3);
    float n = 0.6 * vnoise3(p * 3.0 + o + vec3(0.0, t * 0.5, 0.0)) + 0.4 * vnoise3(p * 8.0 - o * 2.0);
    float r1 = pow(1.0 - abs(n * 2.0 - 1.0), 24.0);
    float n2 = vnoise3(p * 14.0 + o * 3.0);
    float r2 = pow(1.0 - abs(n2 * 2.0 - 1.0), 30.0) * step(0.55, n);
    return r1 + 0.35 * r2;
}
vec3 env(vec3 d, float k) {
    float y = d.y;
    vec3 sky = mix(vec3(0.50, 0.58, 0.72), vec3(0.97, 0.98, 1.0), smoothstep(0.0, 0.7, y));
    vec3 gnd = mix(vec3(0.16, 0.17, 0.19), vec3(0.45, 0.46, 0.5), smoothstep(-0.7, 0.0, y));
    vec3 c = mix(gnd, sky, smoothstep(-0.35, 0.45, y));
    c += vec3(3.0) * pow(max(dot(d, normalize(vec3(-0.5, 0.7, 0.6))), 0.0), 50.0);
    c += vec3(2.0) * pow(max(dot(d, normalize(vec3(0.7, 0.2, 0.7))), 0.0), 80.0);
    c += vec3(0.35, 0.65, 1.0) * pow(max(dot(d, normalize(vec3(0.3, -0.4, 0.9))), 0.0), 6.0) * (0.4 + 1.6 * k);
    return c;
}

// glass shards: Voronoi cells. x = edge distance, y = the cell's random id, z = distance of its centre from 0
vec3 shardCell(vec2 x) {
    vec2 n = floor(x), f = fract(x);
    vec2 mg = vec2(0.0), mr = vec2(0.0);
    float md = 8.0;
    for (int j = -1; j <= 1; j++) for (int i = -1; i <= 1; i++) {
        vec2 g = vec2(float(i), float(j));
        vec2 o = vec2(hash12(n + g), hash12(n + g + 17.3));
        vec2 r = g + o - f;
        float d = dot(r, r);
        if (d < md) { md = d; mr = r; mg = g; }
    }
    float ed = 8.0;
    for (int j = -1; j <= 1; j++) for (int i = -1; i <= 1; i++) {
        vec2 g = mg + vec2(float(i), float(j));
        vec2 o = vec2(hash12(n + g), hash12(n + g + 17.3));
        vec2 r = g + o - f;
        if (dot(mr - r, mr - r) > 1e-5) ed = min(ed, dot(0.5 * (mr + r), normalize(r - mr)));
    }
    return vec3(ed, hash12(n + mg + 5.5), length(x + mr));
}
vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    float px = 1.0 / u_res.y;
    // beats: a pop on every hit, sized by its strength (the bar's 1 is the boom)
    float beat = fract(kclock);
    float beats = floor(kclock);
    float hit = exp(-beat * 9.0);
    float bst = clamp((kick - 1.2) / 1.0, 0.0, 1.0);
    // the electricity is on from the first drop and rides the hardstyle synths: `screech` is keyed with the
    // synth stem's 1.5-8 kHz loudness (0..2.5), so it goes berserk on the screeches and calms between them
    float elecOn = fx;
    float syn = smoothstep(0.8, 2.1, screech) * fx;
    float k = kick * hit * 1.2 * kpow * fx;
    // screeches
    float sph = fract(sclock);
    float shit = exp(-sph * 4.0) * screech * fx * (1.0 + 0.6 * bst);
    float vs = clamp(vox, 0.0, 1.25) * fx;
    float vv = clamp(vox, 0.0, 1.0);
    // the song's energy: on whenever there is sound, bigger in the hype parts
    float sound = smoothstep(0.02, 0.15, level);
    float hype = smoothstep(0.5, 0.9, level);
    float drop = smoothstep(0.5, 0.75, band1) * hype;
    vec3 elec = mix(ELEC0, ELEC1, fx);
    vec3 holy = vec3(1.0, 0.92, 0.72);
    // the metal: silver before the drop, gold from it
    vec3 metal = mix(METAL0, METAL1, fx);
    vec3 shine = mix(vec3(1.0), vec3(1.0, 0.9, 0.6), fx);
    float flash = clamp((beam - 1.0) / 2.0, 0.0, 1.0);

    vec3 P = vec3(0.0);
    float A = 0.0;

    // a slow drift and sway on screen so it feels like it floats in 3D
    vec2 cc = vec2(0.5 + 0.007 * sin(u_time * 0.53) + 0.003 * sin(u_time * 1.31), 0.44 + 0.006 * sin(u_time * 0.71 + 1.0) + 0.002 * sin(u_time * 1.7));
    // impact shake: hard on the drop flash, a jolt on each bar's boom after it
    float shk = (1.0 + 1.2 * bst) * fx * (step(0.95, kpow) * exp(-beat * 6.0) * 0.012 + kpow * exp(-beat * 7.0) * 0.004 + shit * 0.01 + flash * 0.03);
    float rumble = (1.0 - fx) * pow(smoothstep(0.7, 0.85, beam), 2.0) * 0.014;
    cc += rumble * vec2(vn1(u_time * 25.0) - 0.5, vn1(u_time * 25.0 + 17.3) - 0.5) * 2.0;
    cc += shk * vec2(sin(u_time * 90.0 + beats), cos(u_time * 77.0 + beats * 1.7));
    vec2 p = (uv - cc) * vec2(asp, 1.0);
    p.y = -p.y;
    { float sw = 0.035 * sin(u_time * 0.47); p = mat2(cos(sw), -sin(sw), sin(sw), cos(sw)) * p; }
    float r = length(p);
    float ang = atan(p.y, p.x);
    float vig = smoothstep(0.35, 1.05, length((uv - 0.5) * vec2(1.3, 1.0)) * 1.5);
    // the ball rises with the volume and swells on each beat
    float intro = clamp(beam, 0.0, 1.0);
    float gs = mix(0.5 + 0.5 * intro, 1.0, fx) * (1.0 + 0.25 * smoothstep(0.1, 0.9, level)) + 0.14 * fx * kpow * smoothstep(0.0, 0.16, beat) * pow(1.0 - beat, 1.6);
    gs *= 1.0 + 0.1 * bst + mix(0.6, 0.2, fx) * vv + 0.02 * vib;
    float rb = 0.75 * 0.66 * 0.4 * 0.85 * gs;
    // the build into the biggest drop: everything gets sucked into the ball (beams, light, lightning)
    float suck = smoothstep(SUCK0, SUCK1, u_time) * (1.0 - step(BIGDROP, u_time));
    float rS = r * (1.0 + 2.2 * suck);

    // the beam: always there with sound, grows with the hype, pulses on every beat, blooms on held
    // vocals, blazes on the screeches
    float bm = min(beam, 1.0) * sound * (0.35 + 1.1 * level) + 0.7 * k * sound + vs + 0.9 * shit;
    A = max(A, (0.42 * vig * (1.0 - 0.35 * min(bm, 1.0)) + 0.28 * exp(-r / (rb * 3.0))) * max(SHOW_BEAM, SHOW_BALL));
    float rays = pow(0.5 + 0.5 * sin(ang * 14.0 + u_time * 0.25), 6.0) * (0.4 + 0.6 * vn1(ang * 3.0 + u_time * 0.2));
    rays += 0.5 * pow(0.5 + 0.5 * sin(ang * 23.0 - u_time * 0.17), 10.0);
    bm = min(bm * (1.0 + 0.5 * bst), 2.6);
    // the light, kept aside so it can shine over the waveform too
    vec3 beamL = fx * holy * rays * exp(-rS / (rb * (1.5 + 2.0 * bm))) * 1.25 * bm + fx * holy * exp(-rS / (rb * (1.0 + 0.9 * bm))) * 0.5 * bm
               + mix(vec3(1.0), elec, 0.35) * 0.18 * shit;
    beamL *= SHOW_BEAM;
    P += beamL;
    // light streaming inward along the rays while it is being sucked in
    if (SHOW_BEAM > 0.5 && suck > 0.001) {
        float lane = pow(vn1(ang * 9.0 + 3.0), 3.0) + 0.6 * pow(vn1(ang * 23.0 + 11.0), 5.0);
        float flow = pow(0.5 + 0.5 * sin(r * 34.0 + u_time * 16.0 + lane * 6.0), 10.0);
        vec3 inL = fx * holy * lane * flow * suck * smoothstep(rb * 1.02, rb * 1.5, r) * exp(-r / 0.9) * 1.6;
        P += inL;
        beamL += inL;
    }


    // electric corona just outside the ball: the beat
    // the electric ring hugging the ball, from the drop on: solid, jittering, spiking out on the screeches
    if (max(SHOW_BALL, SHOW_BOLTS) > 0.5 && fx > 0.5 && elecOn > 0.5) {
        float tq = floor(u_time * 30.0);
        float crack = vn1(ang * 11.0 + tq * 1.7) + 0.5 * vn1(ang * 29.0 - tq * 2.3);
        float spk = pow(vn1(ang * 7.0 + floor(sclock) * 9.1), 3.0);
        // the ring bursts outward on an intense beat and eases back in
        float ringR = rb * (1.07 + (0.015 + 0.1 * syn) * crack + (0.35 * shit + 0.3 * syn) * spk + 0.04 * k + 0.28 * kpow * kpow * exp(-beat * 4.5));
        float dr = abs(r - ringR);
        float core = 1.0 - smoothstep(1.2 * px, 2.8 * px, dr);
        P = mix(P, vec3(1.0, 0.97, 0.85), core);
        A = max(A, core);
        P += elec * exp(-dr / (0.006 + 0.01 * shit + 0.012 * syn)) * (0.3 + 0.6 * k + 1.0 * shit + 1.6 * syn);
        P += elec * exp(-max(r - rb, 0.0) / 0.04) * (0.1 + 0.5 * k + 0.5 * shit);
    }

    // lightning: only on the screeches; small arcs on each, huge bolts when the song is loud
    float boom = step(0.95, kpow) * hit * smoothstep(0.55, 0.9, level) * fx;
    float zap = max(max(shit, boom), syn * step(0.45, hash11(floor(u_time * 14.0) + 3.3))) * elecOn;
    float seedBase = (syn > 0.35 ? floor(u_time * 14.0) + 900.0 : (shit > boom ? floor(sclock) : beats + 500.0)) * 13.0;
    float small = max(max(exp(-sph * 3.0) * screech * fx, boom), syn) * elecOn;
    float reach = rb * (zap > 0.01 ? 6.6 : 2.6);
    if (SHOW_BOLTS > 0.5 && zap + small > 0.005 && r < reach) for (int i = 0; i < 9; i++) {
        float fi = float(i);
        bool big = i >= 5;
        if (big && zap < 0.01) continue;
        float seed = seedBase + fi * 7.31;
        float a = hash11(seed) * 6.2832 + (big ? 0.0 : u_time * 0.2);
        vec2 dir = vec2(cos(a), sin(a));
        vec2 nrm = vec2(-dir.y, dir.x);
        float al = dot(p, dir);
        float ac = dot(p, nrm);
        float len = big ? rb * (2.5 + 2.5 * hash11(seed + 3.0)) * (0.8 + 0.5 * drop)
                        : rb * (1.25 + 0.55 * hash11(seed + 3.0)) * (0.9 + 0.5 * small);
        len *= 1.0 - 0.6 * suck;
        float grow = max(al - rb * 0.9, 0.0);
        float amp = big ? 0.10 : 0.045;
        float jag = (vn1(al * (big ? 14.0 : 30.0) + seed * 5.0 + floor(u_time * 24.0) * 1.7) - 0.5) * amp
                  + (vn1(al * 60.0 + seed) - 0.5) * amp * 0.3;
        float dd = abs(ac - jag * min(grow * 8.0, 1.0));
        float span = smoothstep(rb * 0.85, rb * 1.0, al) * (1.0 - smoothstep(len * 0.75, len, al));
        float on = big ? zap * (0.35 + 0.65 * hype) * 1.4 : small * 1.1;
        float core = 1.0 - smoothstep((big ? 1.0 : 0.6) * px, (big ? 2.6 : 1.6) * px, dd);
        P += (vec3(1.0) * core + elec * exp(-dd / (big ? 0.013 : 0.006)) * 0.6) * span * on;
    }

    // sparks drifting upward in two layers, on the clock
    if (SHOW_BOLTS > 0.5 && fx > 0.5) for (int l = 0; l < 2; l++) {
        float fl = float(l);
        float sc = 9.0 + fl * 7.0;
        vec2 q = vec2(uv.x * asp, uv.y + u_time * (0.035 + 0.03 * fl));
        vec2 cell = floor(q * sc);
        vec2 f = fract(q * sc);
        float h = hash12(cell + fl * 31.7);
        vec2 sp = vec2(0.2 + 0.6 * hash12(cell + 7.1), 0.2 + 0.6 * h);
        sp.x += 0.12 * sin(u_time * 0.8 + h * 40.0);
        float d = length(f - sp) / sc;
        float tw = step(0.72, h) * (0.5 + 0.5 * vn1(u_time * 1.5 + h * 50.0));
        float b = tw * (0.15 + 0.35 * level + 0.25 * vs + 0.5 * shit);
        P += (vec3(1.0) * (1.0 - smoothstep(0.0, 1.6 * px, d)) + elec * exp(-d / (3.0 * px))) * b * 0.7;
    }

    // the ball: liquid body, beat spikes that rise and melt, screech shards
    float benv = smoothstep(0.0, 0.16, beat) * pow(1.0 - beat, 1.6);
    float bn = beats, sn = floor(sclock);
    g_t = u_time;
    g_maxlen = 0.0;
    // how hard the singing is pushing: the held note, harder as the mids swell
    g_vv = vv * (0.65 + 0.7 * band4);
    g_vib = vib;
    // the band spines: bass and mids only past a threshold so they rest when the band is quiet
    g_b1 = smoothstep(0.35, 0.95, band1) * sound;
    g_b4 = smoothstep(0.35, 0.95, band4) * sound;
    g_kk = 1.3 * benv * kpow;
    // a burst starts on about one beat in eight after the drop and holds for six beats
    g_hold = 0.0;
    g_hseed = 0.0;
    for (int j = 0; j < 7; j++) {
        float bj = beats - float(j);
        if (fx > 0.5 && hype > 0.2 && hash11(bj * 1.37 + 0.5) < 0.12) {
            float pr = (float(j) + beat) / 6.0;
            if (pr < 1.0) g_hold = smoothstep(0.0, 0.025, pr) * (1.0 - smoothstep(0.85, 1.0, pr));
            g_hseed = bj;
            break;
        }
    }
    g_bn = beats;
    // pop size: the hit's strength after the drop, the voice before it; fuller when the bass or mids are up
    g_bph = beat;
    // the electricity's own clock: it advances with the hits, so it races in busy parts and rests when they stop
    g_kc = kclock * 0.22;
    // how many spines a beat gets follows the song's smooth intensity, and the extra ones fade rather than blink
    float inten = fx > 0.5 ? clamp(hype, 0.0, 1.0) : vv;
    g_cnt = clamp((1.0 + 3.0 * inten + 2.0 * bst) * SPINES / 4.0, 1.0, 6.0);
    g_pamp = (fx > 0.5 ? 1.0 : vv) * sound;
    g_spd = max(g_pamp, g_hold);
    g_np = 0;
    for (int j = 0; j < 4; j++) {
        float bj = g_bn - float(j);
        float prj = (float(j) + g_bph) / 4.0;
        float env = smoothstep(0.0, 0.05, prj) * (1.0 - smoothstep(0.6, 1.0, prj));
        for (int m = 0; m < 6; m++) {
            float wm = clamp(g_cnt - float(m), 0.0, 1.0);
            if (wm <= 0.0) break;
            g_pk[g_np] = floor(vec2(hash11(bj * 3.17 + float(m) * 1.9) * 30.0 - 15.0, hash11(bj * 7.71 + float(m) * 2.3) * 15.0));
            g_pe[g_np] = env * wm;
            g_np++;
        }
    }
    g_flow = smoothstep(0.1, 0.95, 0.45 * level + 0.35 * band1 + 0.35 * band4) * mix(0.45, 1.0, fx) + 0.35 * vv;
    g_r = 0.62 * (1.0 + 0.05 * benv * kpow + 0.03 * vs);
    for (int i = 0; i < 32; i++) {
        float fi = float(i);
        vec3 b0 = i < 16 ? AX[i] : -AX[i - 16];
        vec3 w = vec3(sin(u_time * 0.33 + fi * 1.37), sin(u_time * 0.41 + fi * 2.11), sin(u_time * 0.29 + fi * 0.73));
        g_ax[i] = normalize(b0 + 0.3 * w);
        float hp = hash11(bn * 3.1 + fi * 1.7);
        g_len[i] = 0.34 * kick / 1.2 * mix(0.35 * intro * sound, 1.0, fx) * benv * 1.1 * kpow * step(0.35, hp) * (0.5 + 0.6 * hp);
        g_maxlen = max(g_maxlen, g_len[i]);
    }
    // bullets fire on every screech, and after the drop on each strong beat of a hype part
    bool bulletBeat = fx > 0.5 && kpow > 0.75 && hype > 0.35;
    g_sp = bulletBeat ? beat : ((screech > 0.0 && fx > 0.5) ? sph : 1.0);
    if (bulletBeat) sn = beats + 1000.0;
    for (int i = 0; i < 12; i++) {
        float fi = float(i);
        // each bullet leaves from one of the spines this beat popped, along it, with a little spread
        float mi = mod(fi, max(floor(g_cnt), 1.0));
        vec2 pick = floor(vec2(hash11(beats * 3.17 + mi * 1.9) * 30.0 - 15.0, hash11(beats * 7.71 + mi * 2.3) * 15.0));
        float plat = ((pick.y + 0.5) / 15.0 - 0.5) * 3.1416;
        float plon = (pick.x + 0.5 - 0.5 * mod(pick.y, 2.0)) / 30.0 * 6.2832;
        vec3 sdir = vec3(cos(plat) * cos(plon), sin(plat), cos(plat) * sin(plon));
        g_sd[i] = normalize(sdir + 0.0 * vec3(hash11(sn * 7.0 + fi) - 0.5, hash11(sn * 11.0 + fi) - 0.5, hash11(sn * 13.0 + fi) - 0.5));
    }
    float notch = beats + 1.0 - pow(1.0 - clamp(beat / 0.5, 0.0, 1.0), 2.0);
    float ra = u_time * 0.35;
    mat3 R = rotm(ra, 0.35 + 0.25 * sin(u_time * 0.4));
    // bullets, drawn flat (projected) instead of ray-marched: tip-first chrome needles flying out
    if (SHOW_BALL > 0.5 && g_sp < 1.0) {
        float pr = 1.0 - pow(1.0 - g_sp, 4.0);
        float blen = 0.5 * (1.0 - 0.4 * g_sp);
        float brad = 0.045 * (1.0 - 0.4 * g_sp);
        float sc = 0.85 * 0.4 * gs;
        mat3 Rt = transpose(R);
        for (int i = 0; i < 12; i++) {
            // one bullet per spine popped this beat
            if (float(i) >= floor(g_cnt)) break;
            vec3 dir = Rt * g_sd[i];
            vec3 hd = dir * (0.9 + 4.6 * pr + blen);
            vec3 tl = dir * (0.9 + 4.6 * pr);
            if (hd.z > 2.6 || tl.z > 2.6) continue;
            vec2 h2 = hd.xy * 2.4 / (3.2 - hd.z) * sc;
            vec2 t2 = tl.xy * 2.4 / (3.2 - tl.z) * sc;
            float w2 = brad * 2.4 / (3.2 - tl.z) * sc;
            vec2 ab = h2 - t2;
            float L2 = max(length(ab), 1e-4);
            vec2 ax2 = ab / L2;
            vec2 rel = p - t2;
            float u = dot(rel, ax2) / L2;
            if (u < 0.0 || u > 1.0) continue;
            float v = dot(rel, vec2(-ax2.y, ax2.x));
            float hw = w2 * (1.0 - u) + 0.5 * px;
            float cov = 1.0 - smoothstep(hw - px, hw + px, abs(v));
            if (cov > 0.0) {
                float sv = v / max(hw, 1e-4);
                vec3 bc = metal * (0.55 + 0.45 * cos(sv * 2.5 + 0.8)) + shine * exp(-pow((sv + 0.35) * 5.0, 2.0)) * 0.9;
                P = mix(P, bc, cov);
                A = mix(A, 1.0, cov);
            }
        }
    }
    vec2 q = p / (0.85 * 0.4 * gs);
    vec3 ro = vec3(0.0, 0.0, 3.2);
    vec3 rd = normalize(vec3(q, -2.4));
    float bound = g_r + g_maxlen + 0.2 + 0.6 * g_spd + 0.5 * g_hold + 0.15 * abs(g_vib);
    float bb = dot(ro, rd);
    float disc = bb * bb - dot(ro, ro) + bound * bound;
    if (SHOW_BALL > 0.5 && disc > 0.0) {
        float t = -bb - sqrt(disc);
        float tEnd = -bb + sqrt(disc);
        bool hit3 = false;
        vec3 pp;
        for (int i = 0; i < 48; i++) {
            pp = R * (ro + rd * t);
            float d = ball(pp);
            if (d < 0.002) { hit3 = true; break; }
            t += d;
            if (t > tEnd) break;
        }
        if (hit3) {
            vec2 e = vec2(1.0, -1.0) * 0.002;
            vec3 n = normalize(e.xyy * ball(pp + e.xyy) + e.yyx * ball(pp + e.yyx) + e.yxy * ball(pp + e.yxy) + e.xxx * ball(pp + e.xxx));
            n = transpose(R) * n;
            float fr = pow(1.0 - max(dot(n, -rd), 0.0), 4.0);
            vec3 m = env(reflect(rd, n), k + shit) * mix(metal, shine, fr);
            m += elec * fr * (0.3 + 1.0 * k + 1.2 * shit) + holy * fr * 0.5 * vs;
            // cel shading: the reflections flattened into four bold tones and a dark ink outline;
            // the electricity below glows on top of it
            float lum = dot(m, vec3(0.299, 0.587, 0.114));
            float tone = lum < 0.22 ? 0.3 : (lum < 0.5 ? 0.62 : (lum < 0.95 ? 0.9 : 1.2));
            m = mix(metal, shine, step(1.0, tone) * 0.8) * tone;
            float edge = 1.0 - max(dot(n, -rd), 0.0);
            m *= 1.0 - smoothstep(0.74, 0.8, edge) * 0.88;
            // electricity over the surface: always faintly alive with sound, flaring on kicks and screeches
            // an NCS-style shell of glowing dots over the metal: folded sheets of light that the song's
            // waveform (around the ball) bends and the beats and voice brighten
            float ve = ((0.2 + 0.5 * level + 1.0 * k + 0.8 * shit) * mix(0.5, 1.0, fx) + 0.9 * vv) * sound * (1.0 + 0.7 * bst) + 1.4 * syn;
            vec3 lpn = normalize(pp);
            float lon = atan(lpn.z, lpn.x);
            float lat = asin(clamp(lpn.y, -1.0, 1.0));
            vec2 gd = vec2(lon / 6.2832 * 170.0, (lat / 3.1416 + 0.5) * 85.0);
            float dd = length(fract(gd) - 0.5);
            int wi = clamp(int(float(WAVE_N / 2) + u_wave_off - abs(lon) / 3.1416 * float(WAVE_N / 2 - 2)), 0, WAVE_N - 1);
            float aw = u_wave[wi];
            // wavy bands folding over each other, bent by the waveform and the voice
            float ph = dot(lpn, vec3(0.42, 0.78, -0.46)) * 4.0 + 2.0 * sin(dot(lpn, vec3(-0.6, 0.3, 0.74)) * 3.0 + g_kc * 0.9)
                     + g_kc * 1.5 + aw * 6.0 + vv * 2.5;
            float ph2 = dot(lpn, vec3(-0.7, 0.2, 0.68)) * 3.0 + 1.5 * sin(dot(lpn, vec3(0.5, 0.8, 0.3)) * 2.5 - g_kc * 0.7) - g_kc * 1.1;
            float sheet = max(smoothstep(0.55, 0.95, 0.5 + 0.5 * sin(ph)), 0.7 * smoothstep(0.6, 0.97, 0.5 + 0.5 * sin(ph2)));
            // crackling electric filaments over the metal, brightest inside the folded sheets
            float an = 1.0 - abs(2.0 * vnoise3(lpn * 6.0 + vec3(g_kc * 0.8, -g_kc * 0.6, g_kc * 0.5)) - 1.0);
            float an2 = 1.0 - abs(2.0 * vnoise3(lpn * 13.0 - vec3(g_kc * 1.1, g_kc * 0.7, -g_kc * 0.9)) - 1.0);
            float arcs = pow(an, 30.0) + 0.6 * pow(an2, 40.0);
            float flick = 0.55 + 0.45 * vn1(g_kc * 30.0 + floor(an * 7.0) * 3.1);
            float rimd = pow(1.0 - max(dot(n, -rd), 0.0), 2.5);
            float glow = (arcs * (0.35 + 1.8 * sheet) * flick + rimd * 0.6) * ve;
            m = m * (1.0 - 0.55 * clamp(ve, 0.0, 1.0)) + mix(elec, vec3(1.0), 0.35) * glow * 2.4;
            m = mix(m, vec3(1.0, 0.98, 0.92), clamp(glow * 0.45, 0.0, 1.0));
            P = m;
            A = 1.0;
        }
    }

    // the silver waveform: the sound arrives at the left edge as it plays (the next 1.26 s across)
    float yb = 0.885;
    float pos = float(WAVE_N / 2) + u_wave_off + uv.x * float(WAVE_N / 2 - 2);
    // crisp: the peak right here, minus most of its neighbourhood, so every transient stands up as a spike
    float pi0 = clamp(floor(pos), 0.0, float(WAVE_N - 2));
    int ki = int(pi0);
    float pk = mix(u_wave[ki], u_wave[ki + 1], clamp(pos - pi0, 0.0, 1.0));
    float mean = 0.0;
    for (int j = -6; j <= 6; j++) {
        int kj = clamp(ki + j * 2, 0, WAVE_N - 1);
        mean += u_wave[kj];
    }
    mean /= 13.0;
    float tr = clamp(max(pk - 0.8 * mean, 0.0) / 0.35, 0.0, 1.0);
    // and the whole thing grows with the song's volume
    // height = the song's own 10 ms peak (already 0..1 of its loudest, unsmoothed) with a lift on transients
    // while a lyric line is up, the waveform flattens to a glowing line and the words rise out of it
    float amp = max(0.12 * pow(pk, 1.3) * (0.65 + 0.35 * tr) * (1.0 - 0.9 * lyric), 1.0 * px);
    float dy = uv.y - yb;
    float tt = dy / amp;
    float fill = SHOW_WAVE * (1.0 - smoothstep(amp - px, amp + px, abs(dy)));
    if (fill > 0.0) {
        float chrome = 0.6 + 0.4 * cos(tt * 3.6 + 0.6);
        chrome = chrome * 0.75 + 0.55 * exp(-pow((tt + 0.3) * 10.0, 2.0)) + 0.3 * exp(-pow((tt - 0.6) * 8.0, 2.0)) - 0.25 * exp(-pow(tt * 6.0, 2.0));
        float sweep = exp(-pow((uv.x - (fract(u_time * 0.22) * 1.6 - 0.3)) * 7.0, 2.0));
        vec3 silver = mix(METAL0 * vec3(1.16, 1.17, 1.18), METAL1 * vec3(1.0, 1.08, 1.27), fx) * chrome + shine * sweep * 0.35;
        silver += vec3(1.0) * (1.0 - smoothstep(0.0, 1.5 * px, amp - abs(dy))) * 0.45;
        silver += elec * (0.2 * k + 0.4 * shit);
        P = mix(P, silver, fill) + beamL * fill * 0.9;
        A = mix(A, 1.0, fill);
    }
    float wg = exp(-max(abs(dy) - amp, 0.0) / (0.004 + 0.006 * vs)) * (1.0 - fill);
    P += SHOW_WAVE * mix(vec3(0.8, 0.85, 0.95), elec, 0.6) * wg * (0.06 + 0.25 * k + 0.12 * vs + 0.3 * shit);
    A = max(A, SHOW_WAVE * 0.35 * exp(-abs(dy) / 0.06));

    // the last stretch before the biggest drop: the ball shatters reality around it into glass shards, more and faster on
    // every hit (harder hits jolt them more). The shards are colour-inverted by a separate
    // Difference layer above (olympus-rift-invert.glsl, same shard maths).
    if (u_time > RIFT0 && u_time < BIGDROP) {
        float prog = clamp((beats - RIFTHIT0 + smoothstep(0.0, 0.1, beat)) / RIFTHITS, 0.0, 1.0);
        float jolt = kpow * kpow * exp(-beat * 9.0);
        prog = clamp(prog + 0.06 * jolt, 0.0, 1.0);
        vec2 cp = p;
        float cr = length(cp);
        float r0 = rb * 0.97;
        float ce = max(cr - r0, 0.0);
        float outside = step(r0, cr);
        vec3 sc = shardCell(cp * 7.0);
        float cE = sc.z / 7.0 - r0;
        float h = sc.y;
        // the first shard is the one at the ball's centre, overlapping its edge; the rest of the ball
        // follows, then the shards around it, more and more further out
        vec3 sc0 = shardCell(cp * 4.5);
        float first = (1.0 - step(1e-4, abs(sc0.y - shardCell(vec2(0.0)).y))) * step(0.005, prog);
        float ballR = r0 * 1.1 * smoothstep(0.08, 0.28, prog);
        float inv = max(max(first, step(sc.z / 7.0, ballR)), max(step(cE, 0.03 + 0.12 * prog * prog), step(h, 0.4 + 0.6 * pow(prog, 2.0)) * step(cE, prog * 0.85 + 1.8 * pow(prog, 3.0))) * outside * step(0.14, prog));
        vec3 goldL = vec3(1.0, 0.8, 0.42);
        // the ball's edge burns where reality is giving way
        P += goldL * exp(-ce / (0.01 + 0.02 * prog)) * outside * (0.5 + jolt) * prog;
    }
    // a quick electric flash on the hardest beats
    P += elec * 0.05 * k * k;
    // the drop flash
    P += vec3(1.0, 0.97, 0.9) * flash * 1.5;

    float a = clamp(max(A, max(P.r, max(P.g, P.b))), 0.0, 1.0);
    float dz = (hash12(uv * u_res + fract(u_time) * 17.0) - 0.5) / 255.0;
    a = clamp(a + dz, 0.0, 1.0);
    return vec4(clamp(P / max(a, 1e-3) + dz, 0.0, 1.0), a);
}
