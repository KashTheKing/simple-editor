// @name Cross Warp
// @desc The outgoing image zooms in while the incoming one zooms out into place, sweeping from right to left.
// @tags warp, zoom, horizontal
// @credit Eke Péter <peterekepeter@gmail.com>, gl-transitions (MIT)
vec4 transition(vec2 p) {
  float x = progress;
  x=smoothstep(.0,1.0,(x*2.0+p.x-1.0));
  return mix(getFromColor((p-.5)*(1.-x)+.5), getToColor((p-.5)*x+.5), x);
}
