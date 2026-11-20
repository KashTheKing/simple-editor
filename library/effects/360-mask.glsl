// @name 360 Mask
// @desc Hides part of a 360° equirectangular video by direction (the tripod at the bottom, the sky, a slice behind you) with a soft edge; hidden parts turn transparent.
// @tags 360, vr, equirectangular, mask, nadir, tripod, zenith, hide, crop
// @param yaw 0 -180 180
// @param width 360 0 360
// @param top 90 -90 90
// @param bottom -70 -90 90
// @param feather 5 0 45
// @param invert 0 0 1
// Shows the region `width` degrees wide around `yaw`, between latitudes `bottom` and `top` (degrees,
// up is +); the rest is transparent so a patch or logo on a lower track shows through.
const float PI = 3.14159265358979;

vec4 effect(vec4 src, vec2 uv) {
    float lon = degrees((uv.x - 0.5) * 2.0 * PI);
    float lat = degrees((0.5 - uv.y) * PI);
    float f = max(feather, 0.01);
    // signed longitude distance from the centre, wrapped to -180..180
    float dl = abs(mod(lon - yaw + 180.0, 360.0) - 180.0);
    float h = width >= 360.0 ? 1.0 : 1.0 - smoothstep(width * 0.5 - f, width * 0.5, dl);
    float v = smoothstep(bottom - f, bottom, lat) * (1.0 - smoothstep(top, top + f, lat));
    float keep = h * v;
    if (invert > 0.5) {
        keep = 1.0 - keep;
    }
    return vec4(src.rgb, src.a * keep);
}
