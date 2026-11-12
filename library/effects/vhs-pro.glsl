// @name VHS Tape
// @desc Worn tape: colour bleed, wobbling lines, a rolling tracking bar, noise and soft crushed colour.
// @tags vhs, retro, glitch, tape, 90s
// @param bleed 1.0 0 3
// @param jitter 0.6 0 2
// @param tracking 0.6 0 1
// @param noise 0.5 0 1
// @param soften 0.5 0 1
vec4 effect(vec4 src, vec2 uv) {
    float s = u_scale;
    float t = u_time;
    float line = floor(uv.y * u_res.y / max(s * 2.0, 1.0));
    // rolling tracking bar
    float barY = fract(t * 0.13 + 0.2);
    float dBar = uv.y - barY;
    float bar = exp(-pow(dBar / 0.045, 2.0)) * tracking;
    float below = smoothstep(0.0, 0.12, dBar) * (1.0 - smoothstep(0.12, 0.2, dBar));
    // horizontal jitter per line, bursty
    float tt = floor(t * 24.0);
    float burst = step(0.82, hash11(tt * 3.7));
    float lj = (hash12(vec2(line, tt)) - 0.5) * 0.0016 * jitter * (0.3 + burst * 1.5);
    float wob = sin(uv.y * 40.0 + t * 3.0) * 0.0007 * jitter + sin(uv.y * 7.0 - t * 1.3) * 0.0012 * jitter;
    float shift = lj + wob + bar * 0.012 * (hash12(vec2(line, tt)) - 0.3) + bar * 0.01;
    vec2 p = vec2(uv.x + shift, uv.y);
    float px = 1.0 / u_res.x;
    // soft luma: average neighbours horizontally
    float k = soften * 2.5 * s;
    vec3 a = texture(tex, p).rgb;
    vec3 b = 0.25 * (texture(tex, p + vec2(-k * px, 0.0)).rgb + texture(tex, p + vec2(k * px, 0.0)).rgb)
           + 0.25 * (texture(tex, p + vec2(-2.0 * k * px, 0.0)).rgb + texture(tex, p + vec2(2.0 * k * px, 0.0)).rgb);
    vec3 col = mix(a, b, 0.55 * soften + 0.1);
    // chroma bleed: sample colour far to the right (smeared), luma stays
    float bl = bleed * 6.0 * s * px;
    vec3 ch = vec3(0.0);
    for (int i = 0; i < 6; i++) {
        float o = float(i) / 5.0;
        ch += texture(tex, p - vec2(bl * (0.3 + o * 1.7), 0.0)).rgb;
    }
    ch /= 6.0;
    float l = luma(col);
    vec3 chroma = ch - luma(ch);
    col = vec3(l) + mix(col - l, chroma, 0.75 * clamp(bleed, 0.0, 1.0) + 0.1);
    // crushed colours: lift blacks, clip top, desaturate a bit
    float L = luma(col);
    col = mix(vec3(L), col, 0.85);
    col = col * 0.92 + 0.035;
    col = pow(max(col, 0.0), vec3(1.08));
    // noise
    float n = hash12(vec2(floor(uv.x * u_res.x / max(s * 2.0, 1.0)), line + tt * 17.0)) - 0.5;
    col += n * 0.16 * noise;
    // tracking noise band
    float tn = hash12(vec2(floor(uv.x * u_res.x / max(s * 3.0, 1.0)) + tt, line)) ;
    col = mix(col, vec3(0.25 + 0.7 * tn), bar * 0.55 * noise + below * 0.0);
    col += bar * 0.15 * tracking;
    // tape dropout streaks
    float drop = step(0.995, hash12(vec2(line, tt))) * 0.5 * noise;
    col = mix(col, vec3(0.9), drop * step(0.3, hash12(vec2(floor(uv.x * 6.0), line))));
    return vec4(col, src.a);
}
