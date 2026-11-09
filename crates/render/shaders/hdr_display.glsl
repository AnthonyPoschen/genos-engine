// Bloom add from the finest stored mip, before exposure. Intensity 0 does not
// read the chain. `hdr.extra.w` is the base divisor: 2 is half resolution,
// 4 is quarter resolution (Low). The finest mip starts at texel 0.
layout(std430, set = 0, binding = 16) restrict readonly buffer BloomAdd {
    uint words[];
} bloom_add;

vec3 bloom_load(uint texel) {
    vec2 rg = unpackHalf2x16(bloom_add.words[texel * 2u]);
    vec2 ba = unpackHalf2x16(bloom_add.words[texel * 2u + 1u]);
    return vec3(rg, ba.x);
}

vec3 bloom_at(uint base_w, uint base_h, int x, int y) {
    if (x < 0 || y < 0) {
        return vec3(0.0);
    }
    if (uint(x) >= base_w || uint(y) >= base_h) {
        return vec3(0.0);
    }
    return bloom_load(uint(y) * base_w + uint(x));
}

float bloom_tent_w(int dx, int dy) {
    if (dx == 0 && dy == 0) {
        return 0.25;
    }
    if (dx == 0 || dy == 0) {
        return 0.125;
    }
    return 0.0625;
}

vec3 bloom_bilinear(uint bw, uint bh, float u, float v) {
    if (bw == 0u || bh == 0u) {
        return vec3(0.0);
    }
    float px = u * float(bw) - 0.5;
    float py = v * float(bh) - 0.5;
    int x0 = int(floor(px));
    int y0 = int(floor(py));
    float tx = px - float(x0);
    float ty = py - float(y0);
    vec3 s00 = bloom_at(bw, bh, x0, y0);
    vec3 s10 = bloom_at(bw, bh, x0 + 1, y0);
    vec3 s01 = bloom_at(bw, bh, x0, y0 + 1);
    vec3 s11 = bloom_at(bw, bh, x0 + 1, y0 + 1);
    return s00 * (1.0 - tx) * (1.0 - ty) + s10 * tx * (1.0 - ty)
        + s01 * (1.0 - tx) * ty + s11 * tx * ty;
}

vec3 bloom_tent(uint bw, uint bh, uint x, uint y, uint full_w, uint full_h) {
    float u = (float(x) + 0.5) / float(max(full_w, 1u));
    float v = (float(y) + 0.5) / float(max(full_h, 1u));
    vec3 acc = vec3(0.0);
    for (int dy = -1; dy <= 1; dy++) {
        for (int dx = -1; dx <= 1; dx++) {
            float su = u + float(dx) / float(max(bw, 1u));
            float sv = v + float(dy) / float(max(bh, 1u));
            acc += bloom_tent_w(dx, dy) * bloom_bilinear(bw, bh, su, sv);
        }
    }
    return acc;
}

vec3 add_bloom(vec3 c, uint index, uint full_w, uint full_h) {
    if (!(hdr.v.z > 0.0)) {
        return c;
    }
    uint div = uint(floor(hdr.extra.w + 0.5));
    if (div < 2u || full_w == 0u) {
        return c;
    }
    uint bw = max(full_w / div, 1u);
    uint bh = max(full_h / div, 1u);
    uint x = index % full_w;
    uint y = index / full_w;
    c += bloom_tent(bw, bh, x, y, full_w, full_h) * hdr.v.z;
    return c;
}
