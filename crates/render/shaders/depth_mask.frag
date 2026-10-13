#version 450
// Depth prepass fragment. Colour writes are masked off. A masked material
// discards here so a hole does not occlude the surface behind it.
#extension GL_GOOGLE_include_directive : require

layout(location = 0) in vec3 v_pos;
layout(location = 1) in vec3 v_albedo;
layout(location = 2) in vec3 v_normal;
layout(location = 3) in float v_shade;
layout(location = 4) in vec2 v_uv;
layout(location = 5) flat in vec4 v_atlas;
layout(location = 6) flat in vec4 v_normal_atlas;
layout(location = 7) flat in float v_cutoff;
layout(location = 0) out vec4 out_color;

#include "material_atlas.glsl"

void main() {
    if (v_cutoff > 0.0 && v_uv.x >= 0.0) {
        vec4 texel = material_fetch(v_atlas, v_uv);
        if (texel.a < v_cutoff) {
            discard;
        }
    }
    out_color = vec4(0.0);
}
