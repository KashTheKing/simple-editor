// @name 3D Ripple
// @desc Classic 2000s: rings of water ripple out from the centre with light and shade on each wave.
// @tags classic 2000s, ripple, water, wave, 3d, retro
// @param amplitude 0.012 0 0.05
// @param frequency 30 5 80
// @param speed 1 0 6
// @param shading 0.5 0 1
vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    vec2 p = (uv - 0.5) * vec2(asp, 1.0);
    float r = length(p);
    float ph = r * frequency - u_time * speed * 6.2831853;
    vec2 dir = r > 0.0001 ? p / r : vec2(0.0);
    float fall = 1.0 / (1.0 + r * 3.0);
    vec2 off = dir * sin(ph) * amplitude * fall / vec2(asp, 1.0);
    vec4 c = texture(tex, uv + off);
    float light = cos(ph) * fall * shading * amplitude * 12.0;
    return vec4(clamp(c.rgb + light, 0.0, 1.0), c.a);
}
