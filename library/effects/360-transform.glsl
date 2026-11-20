// @name 360 Transform
// @desc Turns a 360° equirectangular video on the sphere: yaw, pitch and roll, keyframeable. 360 Stabilize writes its keys here.
// @tags 360, vr, equirectangular, spherical, rotate, yaw, pitch, roll, horizon, reorient
// @param yaw 45 -180 180
// @param pitch 0 -90 90
// @param roll 0 -180 180
// Equirect convention (every 360 shader and engine::vr360): u = 0.5 looks forward, longitude grows to
// the right; v = 0 is straight up. Direction (x right, y up, z forward). The output pixel's direction is
// turned by Ry(yaw) Rx(pitch) Rz(roll) and the source is read there, so +yaw looks right, +pitch looks up.
const float PI = 3.14159265358979;

vec3 turn(vec3 c, vec3 ypr) {
    float cr = cos(ypr.z), sr = sin(ypr.z);
    c = vec3(cr * c.x - sr * c.y, sr * c.x + cr * c.y, c.z);
    float cp = cos(ypr.y), sp = sin(ypr.y);
    c = vec3(c.x, cp * c.y + sp * c.z, -sp * c.y + cp * c.z);
    float cy = cos(ypr.x), sy = sin(ypr.x);
    return vec3(cy * c.x + sy * c.z, c.y, -sy * c.x + cy * c.z);
}

vec3 eq_dir(vec2 uv) {
    float lon = (uv.x - 0.5) * 2.0 * PI;
    float lat = (0.5 - uv.y) * PI;
    return vec3(cos(lat) * sin(lon), sin(lat), cos(lat) * cos(lon));
}

vec4 eq_sample(vec3 d) {
    float lon = atan(d.x, d.z);
    float lat = asin(clamp(d.y, -1.0, 1.0));
    float hy = 0.5 / max(u_res.y, 1.0);
    // the seam wraps; textureLod keeps the derivative jump at the seam from picking a blurry mip
    return textureLod(tex, vec2(fract(lon / (2.0 * PI) + 0.5), clamp(0.5 - lat / PI, hy, 1.0 - hy)), 0.0);
}

vec4 effect(vec4 src, vec2 uv) {
    vec3 d = turn(eq_dir(uv), radians(vec3(yaw, pitch, roll)));
    return eq_sample(d);
}
