// @name Beat Sprite
// @desc Beat-synced animation: put it on a sprite-sheet image (a dance loop, cols x rows frames) and the loop steps in time with the music, one full loop every Loop Beats beats with poses landing on the beats, plus a bounce on each hit. Right-click an audio clip ▸ Audio ▸ Add Beat-Synced Clip… makes the sheet from a video or image sequence.
// @tags visualizer, audio, beat, sprite, animation, dance, loop
// @param beat 0 0 1
// @param beats 0 0 100000
// @param level 0.5 0 1
// @param cols 4 1 16
// @param rows 2 1 16
// @param frames 8 1 256
// @param loop_beats 4 1 32
// @param bounce 0.12 0 0.5
// The layer is the whole sheet; this shows one cell of it, fitted (aspect kept) into the layer with
// transparent margins, so the clip's own transform places and sizes the dancer.
vec4 effect(vec4 src, vec2 uv) {
    float c = max(1.0, floor(cols + 0.5)), r = max(1.0, floor(rows + 0.5));
    float n = clamp(floor(frames + 0.5), 1.0, c * r);
    float lb = max(1.0, floor(loop_beats + 0.5));
    // position in the loop, in beats: the beats so far (mod the loop) plus this beat's phase
    float pos = beats < 0.5 ? 0.0 : mod(floor(beats) - 1.0, lb) + beat;
    float f = mod(floor(pos / lb * n), n);
    vec2 cell = vec2(mod(f, c), floor(f / c));
    // fit one cell's aspect inside the layer
    float cellAsp = (u_res.x / c) / (u_res.y / r), asp = u_res.x / u_res.y;
    vec2 q = uv - 0.5;
    if (cellAsp > asp) q.y *= cellAsp / asp; else q.x *= asp / cellAsp;
    // bounce: a quick squash-and-pop on the hit, a little bigger when loud
    float hit = exp(-beat * 8.0) * bounce * (0.6 + 0.4 * level);
    q /= 1.0 + hit;
    q.y -= hit * 0.25;
    vec2 local = q + 0.5;
    if (any(lessThan(local, vec2(0.0))) || any(greaterThan(local, vec2(1.0)))) return vec4(0.0);
    vec2 suv = (cell + local) / vec2(c, r);
    return texture(tex, suv);
}
