// @name Rectangle Zoom
// @desc Classic 2000s: the incoming picture starts as a small framed rectangle in the centre and zooms out to fill the frame.
// @tags classic 2000s, rectangle, zoom, box, grow, retro
// @param border 0.006 0 0.03
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    float s = progress;
    vec2 d = abs(uv - 0.5);
    vec2 q = (uv - 0.5) / max(s, 0.0001) + 0.5;
    if (max(d.x, d.y) <= 0.5 * s) return getToColor(q);
    float bw = border * (1.0 - progress);
    if (d.x <= 0.5 * s + bw / ratio && d.y <= 0.5 * s + bw) return vec4(1.0);
    return getFromColor(uv);
}
