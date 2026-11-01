#version 450
// GI G-buffer (lighting_gpu.rs): the scene's surfaces, one record per pixel, for the
// GI compute passes. Same vertices (gbuffer.vert) and depth prepass as the picture; early
// depth tests so only the surface the pixel shows writes its record.
//
// Record (5 uints, gbuffer_layout.glsl): distance, octahedral normal, albedo plus
// a two-sided flag, emission (the light pass overwrites it with direct light),
// then roughness, metallic, and material flags.
//
// Particle cards, fog cards and unlit quads are not in GI yet: they discard.
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
layout(location = 9) flat in float v_rough;
layout(location = 10) flat in float v_metal;
layout(location = 11) flat in uint v_mat;
layout(location = 0) out vec4 out_color;

#include "scene_data.glsl"
#include "lighting_pack.glsl"
#include "material_atlas.glsl"
#include "gbuffer_layout.glsl"

const float kSnormToSigned = 2.0;
const float kSignedToSnorm = 0.5;

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

void main() {
    // Particle cards and unlit quads stay out of the G-buffer. A textured
    // surface (uv.x >= 0) used to discard here; it now samples the atlas.
    if (v_shade < 0.5 || v_shade > 1.5) {
        discard;
    }
    // v_albedo is the base-colour factor, or the untextured diffuse mean.
    // A textured atlas stores base colour. Flag bit 0 tells readers to scale
    // by (1 − metallic) so the Lambert term stays the diffuse albedo.
    vec3 albedo = v_albedo;
    vec3 n = normalize(v_normal);
    float rough = v_rough;
    float metal = v_metal;
    bool textured = v_uv.x >= 0.0 && v_atlas.z > 0.5;
    if (textured) {
        vec4 texel = material_fetch(v_atlas, v_uv);
        if (v_cutoff > 0.0 && texel.a < v_cutoff) {
            discard;
        }
        vec3 rgb = texel.rgb;
        if (atlas_mode.color_linear == 0u) {
            rgb = vec3(material_linear(texel.r), material_linear(texel.g), material_linear(texel.b));
        }
        albedo *= rgb;
        if ((v_mat & 2u) != 0u) {
            metal = texel.a;
        }
        if (v_normal_atlas.z > 0.5) {
            if ((v_mat & 8u) == 0u) {
                // ADR 0035: Z is rebuilt from RG on the RGBA8 path and the BC5 path.
                vec3 mapped = material_normal_at(v_normal_atlas, v_uv);
                vec2 xy = mapped.xy * kSnormToSigned - 1.0;
                float z = sqrt(max(0.0, 1.0 - dot(xy, xy)));
                mapped = vec3(mapped.xy, z * kSignedToSnorm + kSignedToSnorm);
                n = material_perturb(n, v_pos, v_uv, mapped);
            }
            if ((v_mat & 4u) != 0u) {
                rough = material_normal_alpha(v_normal_atlas, v_uv);
            }
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
    float dist = max(length(v_pos - scene.eye.xyz), 1.0e-6);
    atomicMin(gbuf.words[i * 5u], floatBitsToUint(dist));
    atomicMin(gbuf.words[i * 5u + 1u], lighting_pack_normal(n));
    atomicMin(gbuf.words[i * 5u + 2u], packUnorm4x8(vec4(clamp(albedo, 0.0, 1.0), two_sided ? 1.0 / 255.0 : 0.0)));
    uint flags = (v_mat & 1u) != 0u ? 1u : 0u;
    const float gbuf_flag_byte = 255.0;
    atomicMin(gbuf.words[i * 5u + 4u], packUnorm4x8(vec4(clamp(rough, 0.0, 1.0), clamp(metal, 0.0, 1.0), 0.0, float(flags) / gbuf_flag_byte)));
    if (v_emit != 0u) {
        atomicMin(gbuf.words[i * 5u + 3u], v_emit);
    }
    out_color = vec4(albedo, 1.0);
}
