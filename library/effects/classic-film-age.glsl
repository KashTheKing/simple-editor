// @name Film Age
// @desc Classic 2000s: footage that has sat in a drawer for decades: faded yellowed colour, dark corners, dust and the odd scratch.
// @tags classic 2000s, film, age, old, faded, yellow, dust, vintage, retro
// @param age 0.6 0 1
// @param dust 0.5 0 1
vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    vec3 c = src.rgb;
    float y = luma(c);
    vec3 faded = mix(c, vec3(y), 0.5 * age);
    faded = mix(faded, faded * vec3(1.08, 0.98, 0.72) + vec3(0.06, 0.04, 0.0), age);
    faded = mix(faded, vec3(0.5) + (faded - 0.5) * 0.75, age * 0.6);
    vec2 v = (uv - 0.5) * vec2(asp, 1.0);
    faded *= 1.0 - smoothstep(0.35, 1.05, length(v)) * (0.3 + 0.4 * age);
    float tf = floor(u_time * 8.0);
    vec2 g = uv * u_res / (2.0 * u_scale);
    float speck = step(1.0 - 0.0015 * dust, hash12(floor(g) + tf * 17.0));
    faded = mix(faded, vec3(0.15, 0.12, 0.08), speck);
    float sx = hash11(tf * 3.1);
    float scratch = (hash11(tf + 9.0) < 0.35 * dust) ? 1.0 - smoothstep(0.0, 1.2 * u_scale / u_res.x, abs(uv.x - sx)) : 0.0;
    faded = mix(faded, vec3(0.92, 0.88, 0.75), scratch * 0.6);
    return vec4(clamp(faded, 0.0, 1.0), src.a);
}
