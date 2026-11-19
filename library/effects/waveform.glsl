// @name Waveform
// @desc Audio visualizer: the song's own waveform scrolling right to left past a fixed playhead in the centre, mirrored around the middle line, in the accent blue on black. Right-click an audio clip ▸ Audio ▸ Add Audio Visualizer and pick this style.
// @tags visualizer, audio, music, waveform, oscilloscope, scrolling, minimal
// @param wave 0 0 1
// @param level 0.3 0 1
// @param hue 0.633 0 1
// @param height 0.85 0.1 1
// @param glow 1 0 3
// The waveform is `u_wave` (WAVE_N peaks, 0..1 of the song's loudest, one per 1/100 s, the playhead at
// index WAVE_N/2 + u_wave_off), filled in by the renderer from the baked `wave` keys or the song
// analysis. `level` only brightens the glow; the shape is the real waveform.
vec3 hsv(float h, float s, float v) {
    vec3 k = clamp(abs(fract(h + vec3(0.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0, 0.0, 1.0);
    return v * mix(vec3(1.0), k, s);
}
float peakAt(float pos) {
    float i = clamp(floor(pos), 0.0, float(WAVE_N - 2));
    int k = int(i);
    return mix(u_wave[k], u_wave[k + 1], clamp(pos - i, 0.0, 1.0));
}
vec4 effect(vec4 src, vec2 uv) {
    float px = 1.0 / u_res.y;
    // x = 0.5 is now; the right half is still to come and slides left as the song plays
    float pos = float(WAVE_N / 2) + u_wave_off + (uv.x - 0.5) * float(WAVE_N - 2);
    float amp = max(peakAt(pos) * height * 0.5, 0.75 * px);
    float d = abs(uv.y - 0.5);
    vec3 col = hsv(hue, 0.8, 1.0);
    float fill = 1.0 - smoothstep(amp - px, amp + px, d);
    // the part already played is dimmer
    float played = mix(0.55, 1.0, smoothstep(0.5 - 2.0 * px, 0.5, uv.x));
    vec3 c = col * fill * played;
    // a soft glow around the shape, brighter with the music
    float g = exp(-max(d - amp, 0.0) / (0.02 + 0.03 * level)) * (1.0 - fill);
    c += col * g * glow * (0.12 + 0.35 * level);
    // the centre line and the playhead
    c += col * 0.35 * (1.0 - smoothstep(0.0, px, d));
    float ph = 1.0 - smoothstep(1.0, 2.0, abs(uv.x - 0.5) * u_res.x);
    c = mix(c, vec3(1.0), ph * 0.9);
    return vec4(c, 1.0);
}
