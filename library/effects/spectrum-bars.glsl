// @name Spectrum Bars
// @desc Audio visualizer: a mirrored field of thin bars receding to a horizon, in blue and green bands, low notes by the centre. Right-click an audio clip ▸ Audio ▸ Add Audio Visualizer and pick this style; the bars follow the music's spectrum.
// @tags visualizer, audio, music, spectrum, bars, equalizer, retro
// @param level 0.3 0 1
// @param band1 0.5 0 1
// @param band2 0.45 0 1
// @param band3 0.4 0 1
// @param band4 0.35 0 1
// @param band5 0.3 0 1
// @param band6 0.25 0 1
// @param speed 1 0 3
// The look of the Source monitor's original bar field: rows of bars scroll toward the viewer, each a
// thin stalk with a dot at both ends. Heights come from the six baked spectrum bands (60 Hz by the
// centre gap .. 8 kHz outermost); older rows vary a little so the field reads as history.
float bandAt(float x) {
    // x in 0..5 across the six bands, linear in between
    float b[6] = float[6](band1, band2, band3, band4, band5, band6);
    int i = int(clamp(floor(x), 0.0, 4.0));
    return mix(b[i], b[i + 1], clamp(x - float(i), 0.0, 1.0));
}
vec4 effect(vec4 src, vec2 uv) {
    const int ROWS = 28;
    const float HALF = 40.0;
    float asp = u_res.x / u_res.y;
    // layer height = 1; x centred
    vec2 p = vec2((uv.x - 0.5) * asp, uv.y);
    float horizon = 0.45, focal = 0.9, camz = 0.5, gap = 0.06, dx = 0.07, rowd = 0.3, near = 0.8;
    float px = max(1.0 / 500.0, 1.0 / u_res.y);
    float T = u_time * speed * (4.0 + 10.0 * level) * 0.25;
    float scroll = fract(T);
    float gen = floor(T);
    vec3 cols[2] = vec3[2](vec3(30.0, 140.0, 215.0) / 255.0, vec3(35.0, 190.0, 100.0) / 255.0);
    vec3 col = vec3(0.0);
    for (int k = 0; k < ROWS; k++) {
        // k = 0 is the far row; nearer rows paint over farther ones
        float depth = near + (float(ROWS - k) - scroll) * rowd;
        float fade = clamp(1.15 - (depth - near) / (float(ROWS) * rowd), 0.0, 1.0);
        float s = focal / depth;
        float d = abs(p.x) / s;
        float fi = floor((d - gap) / dx + 0.5);
        if (fi < 0.0 || fi >= HALF) continue;
        float xs = (gap + fi * dx) * s;
        // per-row wobble keyed to the row's identity (gen - k), so it travels with the row
        float wob = 0.75 + 0.5 * hash11((gen - float(k)) * 13.1 + fi * 0.37);
        float v = bandAt(fi / (HALF - 1.0) * 5.0) * (k == ROWS - 1 ? 1.0 : wob);
        v = clamp(v, 0.0, 1.2) * (1.0 - 0.55 * fi / HALF);
        float top = horizon - (0.04 + 0.4 * v - camz) * s;
        float bot = horizon - (-0.04 - 0.12 * v - camz) * s;
        vec3 c = cols[int(mod(floor(fi / 6.0), 2.0))] * fade;
        float ex = abs(abs(p.x) - xs);
        if (ex <= px && (abs(p.y - top) <= px || abs(p.y - bot) <= px)) {
            col = c;
        } else if (ex <= px * 0.5 && p.y >= top && p.y <= bot) {
            col = c * 0.55;
        }
    }
    return vec4(col, 1.0);
}
