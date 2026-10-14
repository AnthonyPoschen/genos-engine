#version 450
// GI v2 G-buffer (lighting_gpu.rs): the scene's surfaces, one record per pixel, for the
// GI v2 compute passes. Same vertices (gbuffer.vert) and depth prepass as the picture; early
// depth tests so only the surface the pixel shows writes its record.
//
// Record (uvec4): x distance from the eye (float bits; all ones = nothing), y the normal
// (octahedral, snorm 2x16), z albedo rgb and a flags byte (bit 0 two-sided), w
// emitted light (shared-exponent RGB; the light pass moves it out and puts the
// direct irradiance here).
//
// Particle cards, fog cards and unlit quads are not in GI v2 yet: they discard.
#extension GL_GOOGLE_include_directive : require
layout(early_fragment_tests) in;

layout(location = 0) in vec3 v_pos;
layout(location = 1) in vec3 v_albedo;
layout(location = 2) in vec3 v_normal;
layout(location = 3) in float v_shade;
layout(location = 4) in vec2 v_uv;
layout(location = 5) flat in uint v_emit;
layout(location = 6) flat in vec4 v_atlas;
layout(location = 7) flat in vec4 v_normal_atlas;
layout(location = 8) flat in float v_cutoff;
layout(location = 0) out vec4 out_color;

#include "scene_data.glsl"
#include "lighting_pack.glsl"
#include "material_atlas.glsl"

layout(std430, set = 0, binding = 7) readonly buffer MaterialNormal {
    uint width;
    uint height;
    uint pad0;
    uint pad1;
    uint texels[];
} material_normal;

vec3 material_normal_at(vec4 rect, vec2 uv) {
    if (uv.x < 0.0 || rect.z <= 0.5 || material_normal.width == 0u) {
        return vec3(0.5, 0.5, 1.0);
    }
    float u = uv.x - floor(uv.x);
    float v = uv.y - floor(uv.y);
    uint x = uint(rect.x) + uint(min(u * rect.z, rect.z - 1.0));
    uint y = uint(rect.y) + uint(min(v * rect.w, rect.w - 1.0));
    x = min(x, material_normal.width - 1u);
    y = min(y, material_normal.height - 1u);
    uint packed = material_normal.texels[y * material_normal.width + x];
    return vec3(
        float(packed & 255u) / 255.0,
        float((packed >> 8u) & 255u) / 255.0,
        float((packed >> 16u) & 255u) / 255.0);
}

// Screen-space tangent from the position and the uv. No stored tangents.
vec3 material_perturb(vec3 n, vec3 pos, vec2 uv, vec3 mapped) {
    vec3 tn = mapped * 2.0 - 1.0;
    vec3 dp1 = dFdx(pos);
    vec3 dp2 = dFdy(pos);
    vec2 duv1 = dFdx(uv);
    vec2 duv2 = dFdy(uv);
    vec3 t = dp1 * duv2.y - dp2 * duv1.y;
    vec3 b = dp2 * duv1.x - dp1 * duv2.x;
    float lt = length(t);
    float lb = length(b);
    if (lt < 1e-6 || lb < 1e-6) {
        return n;
    }
    return normalize(t / lt * tn.x + b / lb * tn.y + n * tn.z);
}

layout(std430, set = 0, binding = 6) buffer GBuffer {
    uvec4 header;
    uvec4 px[];
} gbuf;

void main() {
    // Particle cards and unlit quads stay out of the G-buffer. A textured
    // surface (uv.x >= 0) used to discard here; it now samples the atlas.
    if (v_shade < 0.5 || v_shade > 1.5) {
        discard;
    }
    vec3 albedo = v_albedo;
    vec3 n = normalize(v_normal);
    if (v_uv.x >= 0.0 && v_atlas.z > 0.5) {
        vec4 texel = material_fetch(v_atlas, v_uv);
        if (v_cutoff > 0.0 && texel.a < v_cutoff) {
            discard;
        }
        albedo *= vec3(material_linear(texel.r), material_linear(texel.g), material_linear(texel.b));
        if (v_normal_atlas.z > 0.5) {
            n = material_perturb(n, v_pos, v_uv, material_normal_at(v_normal_atlas, v_uv));
        }
    }
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
    atomicMin(gbuf.px[i].y, lighting_pack_normal(n));
    atomicMin(gbuf.px[i].z, packUnorm4x8(vec4(clamp(albedo, 0.0, 1.0), two_sided ? 1.0 / 255.0 : 0.0)));
    if (v_emit != 0u) {
        atomicMin(gbuf.px[i].w, v_emit);
    }
    out_color = vec4(albedo, 1.0);
}
