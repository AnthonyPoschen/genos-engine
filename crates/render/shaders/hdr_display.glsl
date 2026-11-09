// Bloom add from the finest stored mip, before exposure. Intensity 0 does not
// read the chain. `hdr.extra.w` is the base divisor: 2 is half resolution,
// 4 is quarter resolution (Low). The finest mip starts at texel 0.
layout(std430, set = 0, binding = 16) restrict readonly buffer BloomAdd {
    uint words[];
} bloom_add;

const float BLOOM_TEXEL_CENTER = 0.5;
const float BLOOM_TENT_CENTRE = 0.25;
const float BLOOM_TENT_EDGE = 0.125;
const float BLOOM_TENT_CORNER = 0.0625;
const float BLOOM_PROBE_FLAG = 100.0;
const float BLOOM_ROUND = 0.5;

// Finest mip, RGB16F, two texels in 12 bytes, starting at word 0.
vec3 bloom_load(uint local) {
    uint pair = local >> 1u;
    uint w = pair * 3u;
    vec2 a_rg = unpackHalf2x16(bloom_add.words[w]);
    vec2 mid = unpackHalf2x16(bloom_add.words[w + 1u]);
    vec2 b_gb = unpackHalf2x16(bloom_add.words[w + 2u]);
    if ((local & 1u) == 0u) {
        return vec3(a_rg, mid.x);
    }
    return vec3(mid.y, b_gb);
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
        return BLOOM_TENT_CENTRE;
    }
    if (dx == 0 || dy == 0) {
        return BLOOM_TENT_EDGE;
    }
    return BLOOM_TENT_CORNER;
}

vec3 bloom_bilinear(uint bw, uint bh, float u, float v) {
    if (bw == 0u || bh == 0u) {
        return vec3(0.0);
    }
    float px = u * float(bw) - BLOOM_TEXEL_CENTER;
    float py = v * float(bh) - BLOOM_TEXEL_CENTER;
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
    float u = (float(x) + BLOOM_TEXEL_CENTER) / float(max(full_w, 1u));
    float v = (float(y) + BLOOM_TEXEL_CENTER) / float(max(full_h, 1u));
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

uint bloom_divisor() {
    float mark = hdr.extra.w;
    if (mark >= BLOOM_PROBE_FLAG) {
        mark -= BLOOM_PROBE_FLAG;
    }
    return uint(floor(mark + BLOOM_ROUND));
}

// The tent from the finest mip, before the intensity scale. Zero when bloom is off.
vec3 bloom_added_tent(uint index, uint full_w, uint full_h) {
    if (!(hdr.v.z > 0.0)) {
        return vec3(0.0);
    }
    uint div = bloom_divisor();
    if (div < 2u || full_w == 0u) {
        return vec3(0.0);
    }
    uint bw = max(full_w / div, 1u);
    uint bh = max(full_h / div, 1u);
    uint x = index % full_w;
    uint y = index / full_w;
    return bloom_tent(bw, bh, x, y, full_w, full_h);
}

vec3 add_bloom(vec3 c, uint index, uint full_w, uint full_h) {
    c += bloom_added_tent(index, full_w, full_h) * hdr.v.z;
    return c;
}
