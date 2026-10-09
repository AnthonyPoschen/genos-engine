// GI v2 packing helpers shared by the G-buffer and the passes.

vec2 gi2_oct_wrap(vec2 v) {
    return (1.0 - abs(v.yx)) * vec2(v.x >= 0.0 ? 1.0 : -1.0, v.y >= 0.0 ? 1.0 : -1.0);
}

uint gi2_pack_normal(vec3 n) {
    n /= abs(n.x) + abs(n.y) + abs(n.z);
    vec2 e = n.z >= 0.0 ? n.xy : gi2_oct_wrap(n.xy);
    return packSnorm2x16(e);
}

vec3 gi2_unpack_normal(uint w) {
    vec2 e = unpackSnorm2x16(w);
    vec3 n = vec3(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        n.xy = gi2_oct_wrap(n.xy);
    }
    return normalize(n);
}

// Shared-exponent RGB (9 bits each, 5-bit exponent): non-negative light in a uint.
uint gi2_pack_rgb9e5(vec3 rgb) {
    const float MAX = 65408.0;
    vec3 c = clamp(rgb, vec3(0.0), vec3(MAX));
    float m = max(c.r, max(c.g, c.b));
    int e = max(-16, int(floor(log2(max(m, 1.0e-30))))) + 1;
    float scale = exp2(float(9 - e));
    if (floor(m * scale + 0.5) >= 512.0) {
        e += 1;
        scale *= 0.5;
    }
    uvec3 q = uvec3(min(floor(c * scale + 0.5), vec3(511.0)));
    return q.r | (q.g << 9u) | (q.b << 18u) | (uint(e + 15) << 27u);
}

vec3 gi2_unpack_rgb9e5(uint w) {
    float scale = exp2(float(int(w >> 27u) - 15 - 9));
    return vec3(float(w & 511u), float((w >> 9u) & 511u), float((w >> 18u) & 511u)) * scale;
}
