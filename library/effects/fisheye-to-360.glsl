// @name Fisheye to 360
// @desc Unwraps circular fisheye footage (one lens, or a dual-lens 360 camera's two circles side by side) into a 360° equirectangular picture.
// @tags 360, vr, fisheye, hemispherical, dual fisheye, equirectangular, unwrap, stitch, insta360
// @param fov 190 90 240
// @param radius 1 0.5 1.5
// @param center_x 0 -0.5 0.5
// @param center_y 0 -0.5 0.5
// @param yaw 0 -180 180
// @param pitch 0 -90 90
// @param roll 0 -180 180
// @param dual 0 0 1
// @param equisolid 0 0 1
// @param blend 5 0 30
// One lens: the circle fills the frame's height (radius 1), and covers fov degrees. Dual (dual > 0.5):
// the left half holds the front lens and the right half the back lens, each circle filling its half's
// height. Equidistant lens model (r grows with the angle), or equisolid (equisolid > 0.5).
const float PI = 3.14159265358979;

vec3 turn_back(vec3 c, vec3 ypr) {
    float cy = cos(ypr.x), sy = sin(ypr.x);
    c = vec3(cy * c.x - sy * c.z, c.y, sy * c.x + cy * c.z);
    float cp = cos(ypr.y), sp = sin(ypr.y);
    c = vec3(c.x, cp * c.y - sp * c.z, sp * c.y + cp * c.z);
    float cr = cos(ypr.z), sr = sin(ypr.z);
    return vec3(cr * c.x + sr * c.y, -sr * c.x + cr * c.y, c.z);
}

vec3 eq_dir(vec2 uv) {
    float lon = (uv.x - 0.5) * 2.0 * PI;
    float lat = (0.5 - uv.y) * PI;
    return vec3(cos(lat) * sin(lon), sin(lat), cos(lat) * cos(lon));
}

// The lens picture's pixel for camera-space direction c (lens looking down +z), with its weight
// (1 inside the circle, fading to 0 over `blend` degrees past the lens' field of view).
vec4 lens(vec3 c, vec2 centre, vec2 half_size) {
    float th = acos(clamp(c.z, -1.0, 1.0));
    float half_fov = radians(fov) * 0.5;
    float rn = equisolid > 0.5 ? sin(th * 0.5) / sin(half_fov * 0.5) : th / half_fov;
    vec2 dir = length(c.xy) > 1e-6 ? normalize(c.xy) : vec2(0.0);
    // the circle's radius in pixels is `radius` x half the picture height
    vec2 px = dir * rn * radius * half_size.y;
    vec2 suv = centre + vec2(px.x, -px.y) / u_res;
    vec2 ht = 0.5 / max(u_res, vec2(1.0));
    vec4 col = textureLod(tex, clamp(suv, ht, 1.0 - ht), 0.0);
    float w = 1.0 - smoothstep(half_fov - radians(max(blend, 0.01)), half_fov, th);
    return vec4(col.rgb, col.a * w);
}

vec4 effect(vec4 src, vec2 uv) {
    vec3 d = turn_back(eq_dir(uv), radians(vec3(yaw, pitch, roll)));
    vec2 off = vec2(center_x, center_y) * u_res;
    if (dual < 0.5) {
        vec2 c = vec2(0.5) + vec2(off.x, -off.y) / u_res;
        return lens(d, c, u_res * 0.5);
    }
    vec2 half_size = vec2(u_res.x * 0.25, u_res.y * 0.5);
    vec2 cf = vec2(0.25, 0.5) + vec2(off.x, -off.y) / u_res;
    vec2 cb = vec2(0.75, 0.5) + vec2(-off.x, -off.y) / u_res;
    vec4 f = lens(d, cf, half_size);
    // the back lens looks down -z: the same picture turned half way round
    vec4 b = lens(vec3(-d.x, d.y, -d.z), cb, half_size);
    float wf = f.a, wb = b.a;
    float sum = max(wf + wb, 1e-4);
    return vec4((f.rgb * wf + b.rgb * wb) / sum, clamp(wf + wb, 0.0, 1.0));
}
