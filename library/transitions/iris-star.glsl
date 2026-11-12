// @name Iris Star
// @desc A five-pointed star iris grows from the centre, turning slightly, and reveals the incoming picture.
// @tags iris, star, wipe, reveal, shape
// @param softness 0.03 0 0.3
// @param spin 0.12 -0.5 0.5
// @param cx 0.5 0 1
// @param cy 0.5 0 1
vec4 transition(vec2 uv) {
    vec2 c = vec2(cx, cy);
    vec2 pos = (uv - c) * vec2(ratio, 1.0);
    vec2 far = max(c, 1.0 - c) * vec2(ratio, 1.0);
    float maxr = length(far);
    float len = length(pos);
    // angle from 12 o'clock, turning with progress
    float th = atan(pos.x, pos.y) - spin * 6.28318531 * progress;
    float sector = 6.28318531 / 5.0;
    float phi = abs(mod(th + sector * 0.5, sector) - sector * 0.5);
    // star outline: outer tip O at phi = 0, inner notch I at phi = sector / 2
    vec2 O = vec2(1.0, 0.0);
    vec2 I = 0.45 * vec2(cos(sector * 0.5), sin(sector * 0.5));
    vec2 dd = I - O;
    vec2 n = normalize(vec2(-dd.y, dd.x));
    if (dot(n, O) < 0.0) n = -n;
    float cosA = dot(n, vec2(cos(phi), sin(phi)));
    float w = max(softness, 1.5 / u_res.y);
    float e = progress * (maxr + 2.0 * w) - w;
    float sd = len * cosA - e;
    float m = 1.0 - smoothstep(-w, w, sd);
    return mix(getFromColor(uv), getToColor(uv), m);
}
