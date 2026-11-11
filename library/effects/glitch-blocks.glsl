// @name Glitch Blocks
// @desc Digital glitch: random blocks slide sideways with an RGB split, in jittery bursts.
// @tags glitch, digital, rgb, blocks, distortion
// @param intensity 1 0 2
// @param block_size 1 0.3 3
// @param rate 8 1 24
vec4 effect(vec4 src, vec2 uv) {
    float st = floor(u_time * rate);
    // bursts: a few steps of glitching, then a calmer gap
    float burst = step(hash11(floor(st / 3.0) * 1.7 + 3.1), 0.8);
    vec2 g1 = vec2(5.0, 12.0) / block_size;
    vec2 g2 = vec2(14.0, 40.0) / block_size;
    vec2 id1 = floor(uv * g1);
    vec2 id2 = floor(uv * g2);
    float s1 = step(0.74, hash12(id1 + st * 13.7));
    float s2 = step(0.88, hash12(id2 + st * 7.3 + 50.0));
    float sh1 = (hash12(id1 + st * 3.1 + 9.0) - 0.5) * 0.30 * s1;
    float sh2 = (hash12(id2 + st * 5.9 + 21.0) - 0.5) * 0.16 * s2;
    float sel = max(s1, s2);
    float shift = (sh1 + sh2) * intensity * burst;
    float split = (0.004 + 0.016 * sel) * intensity * burst;
    float x = uv.x + shift;
    float r = texture(tex, vec2(fract(x + split), uv.y)).r;
    vec4 gc = texture(tex, vec2(fract(x), uv.y));
    float b = texture(tex, vec2(fract(x - split), uv.y)).b;
    return vec4(r, gc.g, b, gc.a);
}
