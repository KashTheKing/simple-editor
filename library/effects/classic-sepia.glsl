// @name Sepia Tone
// @desc Classic 2000s: warm brown old-photo toning.
// @tags classic 2000s, sepia, brown, vintage, photo, retro
// @param amount 1 0 1
vec4 effect(vec4 src, vec2 uv) {
    vec3 c = src.rgb;
    vec3 s = vec3(dot(c, vec3(0.393, 0.769, 0.189)), dot(c, vec3(0.349, 0.686, 0.168)), dot(c, vec3(0.272, 0.534, 0.131)));
    return vec4(mix(c, min(s, vec3(1.0)), amount), src.a);
}
