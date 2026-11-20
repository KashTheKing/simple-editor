// @name Flat to 360
// @desc Places a normal (flat) picture onto the 360° sphere as an equirectangular frame, at a direction and size you choose; the rest is transparent.
// @tags 360, vr, rectilinear, equirectangular, flat, place, overlay, sphere
// @param yaw 0 -180 180
// @param pitch 0 -90 90
// @param roll 0 -180 180
// @param hfov 90 5 170
// @param vfov 60 5 170
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

vec4 effect(vec4 src, vec2 uv) {
    vec3 c = turn_back(eq_dir(uv), radians(vec3(yaw, pitch, roll)));
    if (c.z <= 1e-4) {
        return vec4(0.0);
    }
    vec2 p = c.xy / c.z / vec2(tan(radians(hfov) * 0.5), tan(radians(vfov) * 0.5));
    // a one-pixel soft edge so the picture's border is not jagged on the sphere
    vec2 e = 1.0 - smoothstep(vec2(1.0) - 2.0 / u_res, vec2(1.0), abs(p));
    vec2 suv = vec2(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    vec4 col = sample_clamped(tex, suv);
    return vec4(col.rgb, col.a * e.x * e.y);
}
