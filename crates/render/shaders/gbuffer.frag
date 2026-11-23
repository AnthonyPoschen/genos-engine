#version 450
// GI G-buffer (lighting_gpu.rs): the scene's surfaces, one record per pixel, for the
// GI compute passes. Same vertices (gbuffer.vert) and depth prepass as the picture; early
// depth tests so only the surface the pixel shows writes its record.
//
// Record (6 uints, gbuffer_layout.glsl): distance, octahedral normal, albedo plus
// a two-sided flag, emission (the light pass overwrites it with direct light),
// roughness/metallic/material flags, then object / bone id.
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
layout(location = 12) flat in uint v_object_id;
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
    // A one-sided surface seen from behind is the inside of a solid: it is
    // drawn black, and its normal is the back's. With the vertex normal, the
    // front's, the back of a wall showed the sun and the lamps that light the
    // other side, and from inside a wall's void (Sponza, behind the gallery
    // vaults) the void was lit. The back's normal alone is not enough: a
    // level field is an unsigned shell, and a ray that starts 2 cm behind a
    // lit wall steps through it to the light. The test is the vertex normal
    // against the face's own normal turned towards the eye, not against the
    // view ray: a smooth normal near a silhouette can point away from the
    // eye on a face that is seen from the front. A two-sided material keeps
    // its normal and its colour; its shade takes the light on either side.
    bool inside_out = false;
    if (!(v_shade > 1.15)) {
        vec3 face = cross(dFdx(v_pos), dFdy(v_pos));
        vec3 to_eye = scene.view_up.w > 0.0 ? -scene.view_forward.xyz : scene.eye.xyz - v_pos;
        if (dot(face, to_eye) < 0.0) {
            face = -face;
        }
        if (dot(n, face) < 0.0) {
            n = -n;
            inside_out = true;
        }
    }
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
    // Along the pixel's ray: from the eye, or in a parallel view
    // (scene.view_up.w > 0) from the plane through the eye.
    float dist = scene.view_up.w > 0.0
        ? max(dot(v_pos - scene.eye.xyz, scene.view_forward.xyz), 1.0e-6)
        : max(length(v_pos - scene.eye.xyz), 1.0e-6);
    uint dist_bits = floatBitsToUint(dist);
    // Elect the nearest fragment (positive floats order as uints). Object 0 and the
    // shading fields take atomicMin; object id must follow the same winner so a
    // farther mover fragment cannot leave its oid on a nearer static surface.
    uint prev_dist = atomicMin(gbuf.words[i * GBUF_STRIDE], dist_bits);
    atomicMin(gbuf.words[i * GBUF_STRIDE + 1u], lighting_pack_normal(n));
    // The base colour is kept in 8 linear bits, so a dark gradient had a few
    // visible steps (a base colour of 0.03 is byte 8). An ordered dither of
    // under half a step, fixed to the pixel, turns the steps into a fine
    // pattern. A colour that is exactly a byte stays that byte.
    const float GBUF_BAYER[16] = float[16](0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    ivec2 cell = ivec2(gl_FragCoord.xy) & 3;
    float dither = ((GBUF_BAYER[cell.y * 4 + cell.x] + 0.5) / 16.0 - 0.5) / 255.0;
    vec3 stored_albedo = clamp(albedo + dither, 0.0, 1.0);
    if (inside_out) {
        stored_albedo = vec3(0.0);
    }
    atomicMin(gbuf.words[i * GBUF_STRIDE + 2u], packUnorm4x8(vec4(stored_albedo, two_sided ? 1.0 / 255.0 : 0.0)));
    uint flags = (v_mat & 1u) != 0u ? 1u : 0u;
    const float gbuf_flag_byte = 255.0;
    atomicMin(gbuf.words[i * GBUF_STRIDE + 4u], packUnorm4x8(vec4(clamp(rough, 0.0, 1.0), clamp(metal, 0.0, 1.0), 0.0, float(flags) / gbuf_flag_byte)));
    if (v_emit != 0u && !inside_out) {
        atomicMin(gbuf.words[i * GBUF_STRIDE + 3u], v_emit);
    }
    // Depth-consistent oid: only a fragment that is at least as near as the
    // elected distance may publish. Ties keep one of the front-most ids.
    if (dist_bits <= prev_dist) {
        gbuf.words[i * GBUF_STRIDE + 5u] = v_object_id;
    }
    out_color = vec4(albedo, 1.0);
}
