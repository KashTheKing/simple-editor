// @name Directional Warp
// @desc A diagonal wipe with a warped, smeared edge.
// @tags wipe, warp, directional
// @param smoothness 0.1 0.01 0.5
// @credit pschroen, gl-transitions (MIT)
const vec2 direction = vec2(-1.0, 1.0);
const vec2 center = vec2(0.5, 0.5);

vec4 transition(vec2 uv) {
  vec2 v = normalize(direction);
  v /= abs(v.x) + abs(v.y);
  float d = v.x * center.x + v.y * center.y;
  float m = 1.0 - smoothstep(-smoothness, 0.0, v.x * uv.x + v.y * uv.y - (d - 0.5 + progress * (1.0 + smoothness)));
  return mix(getFromColor((uv - 0.5) * (1.0 - m) + 0.5), getToColor((uv - 0.5) * m + 0.5), m);
}
