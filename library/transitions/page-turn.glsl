// @name Page Turn
// @desc The outgoing picture peels from the bottom-right corner around a curl, showing its pale back and casting a soft shadow.
// @tags page, curl, book, paper, peel
// @param radius 0.14 0.04 0.4
// @param angle 30 0 80
// @param shadow 0.7 0 1
// @param back_light 0.75 0 1

const float PT_PI = 3.14159265;

float pg_inside(vec2 q, float px) {
    float e = min(min(q.x, ratio - q.x), min(q.y, 1.0 - q.y));
    return smoothstep(-px, 0.0, e);
}

vec4 pg_from(vec2 q) {
    return getFromColor(q / vec2(ratio, 1.0));
}

vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);

    float px = 1.0 / u_res.y;
    float R = radius;
    float an = radians(angle);
    vec2 dir = vec2(-cos(an), sin(an));     // from the bottom-right corner into the page
    vec2 C = vec2(ratio, 0.0);
    vec2 p = uv * vec2(ratio, 1.0);
    float d = dot(p - C, dir);

    float maxD = max(max(dot(vec2(0.0) - C, dir), dot(vec2(0.0, 1.0) - C, dir)),
                     dot(vec2(ratio, 1.0) - C, dir));
    float L = progress * (maxD + PT_PI * R + 4.0 * px);

    // the floor: incoming picture with the soft shadow of the curl
    vec4 inc = getToColor(uv);
    float gap = L - R - d;                 // distance outside the curl's silhouette
    float grow = smoothstep(0.0, 0.08, L); // shadow fades in with the first lift
    float sh = shadow * 0.6 * grow * exp(-max(gap, 0.0) * 14.0) * smoothstep(-R, 0.0, gap);
    vec4 col = vec4(inc.rgb * (1.0 - sh), inc.a);

    vec3 backTint = vec3(0.96, 0.95, 0.92);

    if (d >= L) {
        // flat, untouched page, darkened a little right next to the curl
        vec4 f = getFromColor(uv);
        float sh2 = shadow * 0.25 * grow * exp(-(d - L) * 30.0);
        col = vec4(f.rgb * (1.0 - sh2), f.a);

        // the curled-over flap lying on top, back side up
        float s3 = L - (PT_PI * R + (d - L));
        vec2 q3 = p + (s3 - d) * dir;
        float m3 = pg_inside(q3, px);
        if (m3 > 0.0) {
            vec4 b = pg_from(q3);
            vec3 back = mix(b.rgb, backTint, back_light);
            // the flap is lit near the fold and slightly shaded toward its far end
            float shade = 0.92 + 0.08 * exp(-(d - L) * 10.0);
            col = mix(col, vec4(back * shade, 1.0), m3);
        }
    } else if (d > L - R) {
        float k = clamp((L - d) / R, 0.0, 1.0);
        float edgeAA = smoothstep(0.0, 1.5 * px, d - (L - R));
        float th1 = asin(k);

        // lower half of the curl: the front side, seen from inside the roll
        float s1 = L - R * th1;
        vec2 q1 = p + (s1 - d) * dir;
        float m1 = pg_inside(q1, px) * edgeAA;
        if (m1 > 0.0) {
            vec4 f = pg_from(q1);
            float shade = mix(1.0, 0.45, k * k);
            col = mix(col, vec4(f.rgb * shade, f.a), m1);
        }

        // upper half: the back side, rolling over
        float th2 = PT_PI - th1;
        float s2 = L - R * th2;
        vec2 q2 = p + (s2 - d) * dir;
        float m2 = pg_inside(q2, px) * edgeAA;
        if (m2 > 0.0) {
            vec4 b = pg_from(q2);
            vec3 back = mix(b.rgb, backTint, back_light);
            float nz = -cos(th2);                    // 0 at the silhouette, 1 on top
            float shade = 0.55 + 0.45 * nz;
            float spec = pow(clamp(1.0 - abs(nz - 0.8) * 3.0, 0.0, 1.0), 2.0) * 0.18;
            col = mix(col, vec4(back * shade + spec, 1.0), m2);
        }
    }
    return col;
}
