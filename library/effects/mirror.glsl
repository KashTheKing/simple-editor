// @name Mirror
// @desc Mirrors one half of the picture onto the other: left to right, right to left, top to bottom or bottom to top.
// @tags mirror, symmetry, flip, reflection
// @param mode 0 0 3
// @param offset 0 -0.4 0.4
float mirror_fold(float x) {
    float t = mod(x, 2.0);
    return 1.0 - abs(t - 1.0);
}
vec4 effect(vec4 src, vec2 uv) {
    int m = int(mode + 0.5);
    vec2 q = uv;
    float ax = 0.5 + offset;
    if (m == 0) q.x = uv.x > ax ? ax - (uv.x - ax) : uv.x;
    else if (m == 1) q.x = uv.x < ax ? ax + (ax - uv.x) : uv.x;
    else if (m == 2) q.y = uv.y > ax ? ax - (uv.y - ax) : uv.y;
    else q.y = uv.y < ax ? ax + (ax - uv.y) : uv.y;
    q = vec2(mirror_fold(q.x), mirror_fold(q.y));
    return texture(tex, q);
}
