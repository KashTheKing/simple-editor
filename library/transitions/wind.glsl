// @name Wind
// @desc A ragged left-to-right wipe, as if the image was blown away by the wind.
// @tags wipe, horizontal, noise
// @param size 0.2 0.01 1
// @param reversed 0 0 1
// @credit gre, gl-transitions (MIT)
float rand (vec2 co) {
  return fract(sin(dot(co.xy ,vec2(12.9898,78.233))) * 43758.5453);
}

vec4 transition(vec2 uv) {
  float x = reversed > 0.5 ? 1. - uv.x : uv.x;
  float r = rand(vec2(0, uv.y));
  float m = smoothstep(0.0, -size, x*(1.0-size) + size*r - (progress * (1.0 + size)));
  return mix(
    getFromColor(uv),
    getToColor(uv),
    m
  );
}
