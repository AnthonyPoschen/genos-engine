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
