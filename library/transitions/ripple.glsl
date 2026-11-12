// @name Ripple
// @desc Ripples spread from the center while crossfading.
// @tags ripple, wave, blend
// @param amplitude 100 0 300
// @param speed 50 0 150
// @credit gre, gl-transitions (MIT)
vec4 transition(vec2 uv) {
  vec2 dir = uv - vec2(.5);
  float dist = length(dir);
  vec2 offset = dir * (sin(progress * dist * amplitude - progress * speed) + .5) / 30. * progress;
  return mix(
    getFromColor(uv + offset),
    getToColor(uv),
    smoothstep(0.2, 1.0, progress)
  );
}
