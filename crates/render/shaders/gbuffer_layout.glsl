// G-buffer record. header, then 5 uints per pixel:
//   0 distance (float bits; all ones = nothing)
//   1 octahedral normal
//   2 albedo rgb and a flags byte (bit 0 two-sided)
//   3 emission, then the direct pass overwrites it with direct light
//   4 roughness, metallic, unused, material flags (unorm4x8). Flag bit 0:
//     rgb is base colour, and readers scale it by (1 - metallic).
#ifndef GBUFFER_LAYOUT_GLSL
#define GBUFFER_LAYOUT_GLSL

layout(std430, set = 0, binding = 6) buffer GBuffer {
    uvec4 header;
    uint words[];
} gbuf;

uint gbuf_word(uint pixel, uint comp) {
    return gbuf.words[pixel * 5u + comp];
}

uvec4 gbuf_load4(uint pixel) {
    return uvec4(
        gbuf.words[pixel * 5u],
        gbuf.words[pixel * 5u + 1u],
        gbuf.words[pixel * 5u + 2u],
        gbuf.words[pixel * 5u + 3u]);
}

void gbuf_clear_pixel(uint pixel) {
    gbuf.words[pixel * 5u] = ~0u;
    gbuf.words[pixel * 5u + 1u] = ~0u;
    gbuf.words[pixel * 5u + 2u] = ~0u;
    gbuf.words[pixel * 5u + 3u] = ~0u;
    gbuf.words[pixel * 5u + 4u] = ~0u;
}

// base, roughness, metallic. Flag bit 0 off forces metallic to 0: the rgb
// slot is already the diffuse albedo (analytic shapes).
const float GBUF_UNORM_BYTE = 255.0;
const float GBUF_UNORM_ROUND = 0.5;

void gbuf_material(uint pixel, out vec3 base, out float rough, out float metal) {
    vec4 albedo = unpackUnorm4x8(gbuf.words[pixel * 5u + 2u]);
    vec4 mr = unpackUnorm4x8(gbuf.words[pixel * 5u + 4u]);
    base = albedo.rgb;
    rough = mr.r;
    metal = mr.g;
    if ((uint(mr.a * GBUF_UNORM_BYTE + GBUF_UNORM_ROUND) & 1u) == 0u) {
        metal = 0.0;
    }
}

#endif
