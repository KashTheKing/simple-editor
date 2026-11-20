// @name 360 Little Planet
// @desc Wraps a 360° equirectangular video into a tiny planet (stereographic). Pitch 90 turns it inside out into a tunnel.
// @tags 360, vr, equirectangular, stereographic, little planet, tiny planet, tunnel, rabbit hole
// @param yaw 0 -180 180
// @param pitch -90 -90 90
// @param roll 0 -180 180
// @param fov 240 30 330
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
    // fov is the angle seen across the frame's height; a point theta from the centre lands at
    // r = 2 tan(theta / 2) on the plane (stereographic), so the top edge is r = 2 tan(fov / 4)
    float asp = u_res.x / max(u_res.y, 1.0);
    vec2 p = vec2((uv.x - 0.5) * asp, 0.5 - uv.y) * 2.0 * 2.0 * tan(radians(fov) * 0.25);
    float r = length(p);
    float th = 2.0 * atan(r * 0.5);
    vec2 dir = r > 1e-6 ? p / r : vec2(0.0);
    vec3 c = vec3(sin(th) * dir, cos(th));
    return eq_sample(turn(c, radians(vec3(yaw, pitch, roll))));
}
