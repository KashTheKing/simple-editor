// @name RGB Slide
// @desc The red, green and blue channels slide in separately, so the incoming picture assembles from coloured ghosts.
// @tags rgb, slide, ghost, chromatic, channels
// @param distance 0.3 0 0.8
// @param stagger 0.15 0 0.3
float rs_m(float d) {
    float x = clamp((progress - d) / (1.0 - 2.0 * stagger), 0.0, 1.0);
    return x * x * (3.0 - 2.0 * x);
}

vec4 transition(vec2 uv) {
    float mr = rs_m(0.0);
    float mg = rs_m(stagger);
    float mb = rs_m(2.0 * stagger);
    float r = mix(getFromColor(uv + vec2(distance * mr, 0.0)).r, getToColor(uv - vec2(distance * (1.0 - mr), 0.0)).r, mr);
    float g = mix(getFromColor(uv + vec2(distance * mg, 0.0)).g, getToColor(uv - vec2(distance * (1.0 - mg), 0.0)).g, mg);
    float b = mix(getFromColor(uv + vec2(distance * mb, 0.0)).b, getToColor(uv - vec2(distance * (1.0 - mb), 0.0)).b, mb);
    float a = mix(getFromColor(uv).a, getToColor(uv).a, mg);
    return vec4(r, g, b, a);
}
