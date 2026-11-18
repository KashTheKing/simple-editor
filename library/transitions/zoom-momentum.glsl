// @name Zoom Momentum
// @desc The outgoing picture rushes into the centre and the incoming one lands out of the same rush, both smeared by a radial blur: a punchy cut for beat edits.
// @tags zoom, momentum, rush, punch, radial blur, beat, edit, velocity
// @param strength 1.4 0.2 4
// @param blur 0.16 0 0.5
vec4 zoomed(vec2 uv, float z, float b, bool from) {
    vec2 c = vec2(0.5);
    vec2 p = (uv - c) / z + c;
    vec4 acc = vec4(0.0);
    for (int i = 0; i < 12; i++) {
        vec2 q = (p - c) * (1.0 - b * float(i) / 11.0) + c;
        acc += from ? getFromColor(q) : getToColor(q);
    }
    return acc / 12.0;
}

vec4 transition(vec2 uv) {
    if (progress < 0.5) {
        float t = progress * 2.0;
        float e = t * t * t;
        return zoomed(uv, 1.0 + strength * e, blur * e, true);
    }
    float t = (progress - 0.5) * 2.0;
    float e = 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t);
    return zoomed(uv, 1.0 + strength * (1.0 - e), blur * (1.0 - e), false);
}
