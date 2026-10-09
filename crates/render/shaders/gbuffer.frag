#version 450
// GI v2 G-buffer (gi2_gpu.rs): the scene's surfaces, one record per pixel, for the
// GI v2 compute passes. Same vertex shader and depth prepass as the picture; early
// depth tests so only the surface the pixel shows writes its record.
//
// Record (uvec4): x distance from the eye (float bits; all ones = nothing), y the normal
// (octahedral, snorm 2x16), z albedo rgb and a flags byte (bit 0 two-sided), w
// reserved for emission.
//
// Particle cards, fog cards and unlit quads are not in GI v2 yet: they discard.
#extension GL_GOOGLE_include_directive : require
layout(early_fragment_tests) in;

layout(location = 0) in vec3 v_pos;
layout(location = 1) in vec3 v_albedo;
layout(location = 2) in vec3 v_normal;
layout(location = 3) in float v_shade;
layout(location = 4) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

#include "scene_data.glsl"
#include "gi2_pack.glsl"

layout(std430, set = 0, binding = 6) buffer GBuffer {
    uvec4 header;
    uvec4 px[];
} gbuf;

void main() {
    if (v_shade < 0.5 || v_shade > 1.5 || v_uv.x >= 0.0) {
        discard;
    }
    vec3 n = normalize(v_normal);
    bool two_sided = v_shade > 1.15;
    uvec2 p = uvec2(gl_FragCoord.xy);
    if (p.x >= gbuf.header.x || p.y >= gbuf.header.y) {
        return;
    }
    // Only fragments at the depth prepass's final depth get here (early tests), but
    // where faces meet or overlap several do, and plain writes race: a pixel's
    // normal or shading point could flip between frames. Each field takes the
    // minimum over those fragments instead (records are all ones when empty, and
    // positive floats order as uints), the same answer every frame.
    uint i = p.y * gbuf.header.x + p.x;
    atomicMin(gbuf.px[i].x, floatBitsToUint(max(length(v_pos - scene.eye.xyz), 1.0e-6)));
    atomicMin(gbuf.px[i].y, gi2_pack_normal(n));
    atomicMin(gbuf.px[i].z, packUnorm4x8(vec4(clamp(v_albedo, 0.0, 1.0), two_sided ? 1.0 / 255.0 : 0.0)));
    out_color = vec4(v_albedo, 1.0);
}
