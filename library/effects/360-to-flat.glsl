// @name 360 to Flat
// @desc Looks into a 360° equirectangular video through a normal camera: pick the direction and the field of view (keyframe them to pan).
// @tags 360, vr, equirectangular, rectilinear, reframe, flat, camera, pan, fov
// @param yaw 0 -180 180
// @param pitch 0 -90 90
// @param roll 0 -180 180
// @param fov 90 10 170
const float PI = 3.14159265358979;

vec3 turn(vec3 c, vec3 ypr) {
    float cr = cos(ypr.z), sr = sin(ypr.z);
    c = vec3(cr * c.x - sr * c.y, sr * c.x + cr * c.y, c.z);
    float cp = cos(ypr.y), sp = sin(ypr.y);
    c = vec3(c.x, cp * c.y + sp * c.z, -sp * c.y + cp * c.z);
    float cy = cos(ypr.x), sy = sin(ypr.x);
    return vec3(cy * c.x + sy * c.z, c.y, -sy * c.x + cy * c.z);
}

vec4 eq_sample(vec3 d) {
    float lon = atan(d.x, d.z);
    float lat = asin(clamp(d.y, -1.0, 1.0));
    float hy = 0.5 / max(u_res.y, 1.0);
    return textureLod(tex, vec2(fract(lon / (2.0 * PI) + 0.5), clamp(0.5 - lat / PI, hy, 1.0 - hy)), 0.0);
}

vec4 effect(vec4 src, vec2 uv) {
    // fov is horizontal, across the frame's width
    float asp = u_res.x / max(u_res.y, 1.0);
    float t = tan(radians(fov) * 0.5);
    vec3 c = normalize(vec3((uv.x - 0.5) * 2.0 * t, (0.5 - uv.y) * 2.0 * t / asp, 1.0));
    return eq_sample(turn(c, radians(vec3(yaw, pitch, roll))));
}
